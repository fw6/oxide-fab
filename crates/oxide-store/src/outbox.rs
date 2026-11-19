//! The outbox projector: at-least-once delivery from PostgreSQL to `GreptimeDB`.
//!
//! A transaction commits `lot_event` + projections + `outbox` together. This
//! projector then pushes each outbox row to the time series store and marks it
//! published in the same transaction that records the delivery in
//! `published_event`. Nothing is lost while `GreptimeDB` is down: rows simply
//! stay pending, and the backlog is what the dashboard's chaos timeline plots.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use oxide_core::metrics::MetricRow;
use oxide_core::model::Route;
use serde::{Deserialize, Serialize};
use tokio::time::{Duration, sleep};

use crate::greptime::GreptimeDb;
use crate::pg::{PgStore, Result as StoreResult, StoreError};

/// One sample of the delivery backlog, for the UI's recovery timeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutboxSample {
    pub at_ms: i64,
    pub pending: i64,
    pub published: i64,
    pub oldest_pending_ms: i64,
    pub delivered_rows: u64,
}

/// A bounded ring of the backlog history.
#[derive(Debug, Clone, Default)]
pub struct OutboxTimeline {
    inner: Arc<Mutex<VecDeque<OutboxSample>>>,
}

impl OutboxTimeline {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&self, sample: OutboxSample) {
        let mut guard = self.inner.lock().expect("timeline lock");
        guard.push_back(sample);
        while guard.len() > 600 {
            guard.pop_front();
        }
    }

    #[must_use]
    pub fn samples(&self) -> Vec<OutboxSample> {
        self.inner
            .lock()
            .expect("timeline lock")
            .iter()
            .cloned()
            .collect()
    }

    #[must_use]
    pub fn latest(&self) -> Option<OutboxSample> {
        self.inner.lock().expect("timeline lock").back().cloned()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishReport {
    pub delivered_events: usize,
    pub delivered_rows: usize,
}

/// Deliver one batch. Returns how much was accepted by the store.
///
/// The delivery lock keeps a batch out of a `TRUNCATE` performed by reset or
/// replay: a publisher that had already claimed rows must finish, and its rows
/// must be in the store before the truncate, not after it.
pub async fn publish_once(
    store: &PgStore,
    greptime: &GreptimeDb,
    delivery_lock: &tokio::sync::Mutex<()>,
    batch: i64,
) -> StoreResult<PublishReport> {
    let _guard = delivery_lock.lock().await;
    let claimed = store.claim_outbox(batch).await?;
    if claimed.is_empty() {
        return Ok(PublishReport {
            delivered_events: 0,
            delivered_rows: 0,
        });
    }
    let mut rows = Vec::new();
    for row in &claimed {
        let parsed = row
            .payload
            .get("rows")
            .cloned()
            .map(serde_json::from_value::<Vec<MetricRow>>)
            .transpose()
            .map_err(|error| {
                StoreError::Message(format!("outbox payload is not metric rows: {error}"))
            })?;
        rows.extend(parsed.unwrap_or_default());
    }
    if let Err(error) = greptime.insert_rows(&rows).await {
        // Record why on every claimed row before handing the error back, so
        // the dashboard's outbox view explains the backlog.
        let message = error.to_string();
        for row in &claimed {
            let _ = store.mark_failed(row.event_id, &message).await;
        }
        return Err(error.into());
    }
    let ids: Vec<uuid::Uuid> = claimed.iter().map(|row| row.event_id).collect();
    store.mark_published(&ids, "greptimedb").await?;
    Ok(PublishReport {
        delivered_events: claimed.len(),
        delivered_rows: rows.len(),
    })
}

/// The long-running projector.
#[derive(Debug)]
pub struct Publisher {
    pub store: PgStore,
    pub greptime: GreptimeDb,
    pub timeline: OutboxTimeline,
    pub delivery_lock: Arc<tokio::sync::Mutex<()>>,
    pub poll_interval: Duration,
    pub batch: i64,
}

impl Publisher {
    #[must_use]
    pub fn new(
        store: PgStore,
        greptime: GreptimeDb,
        timeline: OutboxTimeline,
        delivery_lock: Arc<tokio::sync::Mutex<()>>,
    ) -> Self {
        Self {
            store,
            greptime,
            timeline,
            delivery_lock,
            poll_interval: Duration::from_millis(200),
            batch: 64,
        }
    }

    pub async fn run(self, shutdown: tokio::sync::watch::Receiver<bool>) {
        let mut delivered_rows = 0_u64;
        let mut shutdown = shutdown;
        loop {
            if *shutdown.borrow() {
                return;
            }
            match publish_once(&self.store, &self.greptime, &self.delivery_lock, self.batch).await {
                Ok(report) => delivered_rows += report.delivered_rows as u64,
                Err(error) => {
                    tracing::warn!(%error, "outbox delivery failed; rows stay pending");
                }
            }
            if let Ok(stats) = self.store.outbox_stats().await {
                self.timeline.push(OutboxSample {
                    at_ms: chrono::Utc::now().timestamp_millis(),
                    pending: stats.pending,
                    published: stats.published,
                    oldest_pending_ms: stats.oldest_pending_ms,
                    delivered_rows,
                });
            }
            tokio::select! {
                () = sleep(self.poll_interval) => {}
                _ = shutdown.changed() => {
                    if *shutdown.borrow() {
                        return;
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReplayReport {
    pub events: usize,
    pub rows: usize,
    pub elapsed_ms: i64,
}

/// Rebuild the time series store from scratch out of `lot_event`.
///
/// `truncate` first drops every derived table's contents; then each event's
/// rows are derived with the same pure function the collector used.
pub async fn replay(
    store: &PgStore,
    greptime: &GreptimeDb,
    delivery_lock: &tokio::sync::Mutex<()>,
    route: &Route,
    truncate: bool,
) -> StoreResult<ReplayReport> {
    let _guard = delivery_lock.lock().await;
    let started = std::time::Instant::now();
    if truncate {
        greptime.truncate_all().await?;
    }
    let events = store.all_events().await?;
    let mut batch = Vec::new();
    let mut rows = 0;
    let mut count = 0;
    for persisted in &events {
        batch.extend(oxide_core::metrics::derive(&persisted.event, route));
        count += 1;
        if batch.len() >= 500 {
            rows += greptime.insert_rows(&batch).await?;
            batch.clear();
        }
    }
    if !batch.is_empty() {
        rows += greptime.insert_rows(&batch).await?;
    }
    Ok(ReplayReport {
        events: count,
        rows,
        elapsed_ms: started.elapsed().as_millis() as i64,
    })
}
