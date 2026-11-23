//! PostgreSQL: the append-only event log, its projections, and the outbox.

use oxide_core::events::{DomainEvent, EventKind};
use oxide_core::genealogy::{WaferGenealogy, genealogy_for};
use oxide_core::metrics::{MetricRow, derive};
use oxide_core::model::{Lot, LotState, Route, Wafer, WaferState};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database: {0}")]
    Db(#[from] sqlx::Error),
    #[error("migration: {0}")]
    Migrate(#[from] sqlx::migrate::MigrateError),
    /// A row holds a value this build does not know. Silently mapping it onto
    /// a default would rewrite history on the next replay.
    #[error("{table}.{column} holds {value:?}, which this build does not know")]
    UnknownValue {
        table: &'static str,
        column: &'static str,
        value: String,
    },
    #[error("time series: {0}")]
    Greptime(#[from] crate::greptime::GreptimeError),
    #[error("{0}")]
    Message(String),
}

pub type Result<T> = std::result::Result<T, StoreError>;

impl StoreError {
    /// `true` when the failure is a connectivity problem rather than a query
    /// or data problem: the HTTP layer turns these into `503` so a client
    /// retries instead of treating the request as a bug.
    #[must_use]
    pub fn is_unavailable(&self) -> bool {
        match self {
            // A new variant defaults to "not a connectivity problem", which is
            // the safe answer: 500, not a retry loop.
            StoreError::Db(
                sqlx::Error::PoolTimedOut | sqlx::Error::PoolClosed | sqlx::Error::Io(_),
            )
            | StoreError::Greptime(crate::greptime::GreptimeError::Http(_)) => true,
            _ => false,
        }
    }
}

/// An event as stored, with its global sequence number.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersistedEvent {
    pub seq: i64,
    pub event: DomainEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub state: Value,
    pub last_seq: i64,
    pub created_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OutboxStats {
    pub pending: i64,
    pub published: i64,
    pub oldest_pending_ms: i64,
    pub max_attempts: i32,
}

/// One outbox row, as the event trace shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutboxTrace {
    pub event_id: Uuid,
    pub aggregate_id: String,
    pub topic: String,
    pub attempts: i32,
    pub created_at: String,
    pub published_at: Option<String>,
    pub last_error: Option<String>,
    pub payload: Value,
}

/// Publisher liveness: how far behind the outbox is, and when it last
/// delivered anything.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboxHealth {
    pub pending: i64,
    pub max_attempts: i32,
    pub oldest_pending_age_ms: i64,
    /// `-1` when nothing has ever been published.
    pub last_published_age_ms: i64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DbCounts {
    pub lots: i64,
    pub wafers: i64,
    pub events: i64,
    pub outbox_pending: i64,
    pub outbox_published: i64,
    pub published_events: i64,
}

/// A lot with its wafers and their step history, as the dashboard shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LotGenealogy {
    pub lot: Lot,
    pub wafers: Vec<Wafer>,
    pub genealogy: Vec<WaferGenealogy>,
    pub events: Vec<PersistedEvent>,
}

#[derive(Clone, Debug)]
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    #[must_use]
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn connect(url: &str, max_connections: u32) -> Result<Self> {
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(max_connections)
            .acquire_timeout(std::time::Duration::from_secs(5))
            .connect(url)
            .await?;
        Ok(Self { pool })
    }

    /// Apply `migrations/*.sql` from disk. Reading them at run time keeps the
    /// build free of `sqlx-macros`, which would drag every SQL driver (and its
    /// advisories) into the dependency graph for a PostgreSQL-only service.
    pub async fn migrate(&self, directory: &str) -> Result<()> {
        let migrator = sqlx::migrate::Migrator::new(std::path::Path::new(directory))
            .await
            .map_err(|error| StoreError::Message(format!("migrations in {directory}: {error}")))?;
        migrator.run(&self.pool).await?;
        Ok(())
    }

    pub async fn ping(&self) -> bool {
        sqlx::query_scalar::<_, i32>("SELECT 1")
            .fetch_one(&self.pool)
            .await
            .is_ok()
    }

    /// Upsert the scenario's static master data. Safe to run on every start.
    pub async fn seed(&self, scenario: &oxide_core::Scenario) -> Result<()> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "INSERT INTO product (id, name, wafer_size_mm) VALUES ($1, $2, $3)
             ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, wafer_size_mm = EXCLUDED.wafer_size_mm",
        )
        .bind(&scenario.product.id)
        .bind(&scenario.product.name)
        .bind(scenario.product.wafer_size_mm)
        .execute(&mut *tx)
        .await?;

        sqlx::query("INSERT INTO route (id, name) VALUES ($1, $2) ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name")
            .bind(&scenario.route.id)
            .bind(&scenario.route.name)
            .execute(&mut *tx)
            .await?;

        for step in &scenario.route.steps {
            sqlx::query(
                "INSERT INTO route_step (route_id, step_no, name, equipment_kind, equipment_qualification,
                                         recipe_id, duration_ms, target, ucl, lcl)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT (route_id, step_no) DO UPDATE SET
                   name = EXCLUDED.name, equipment_kind = EXCLUDED.equipment_kind,
                   equipment_qualification = EXCLUDED.equipment_qualification,
                   recipe_id = EXCLUDED.recipe_id, duration_ms = EXCLUDED.duration_ms,
                   target = EXCLUDED.target, ucl = EXCLUDED.ucl, lcl = EXCLUDED.lcl",
            )
            .bind(&scenario.route.id)
            .bind(step.step_no)
            .bind(&step.name)
            .bind(&step.equipment_kind)
            .bind(
                serde_json::to_value(&step.equipment_qualification)
                    .map_err(|error| StoreError::Message(error.to_string()))?,
            )
            .bind(&step.recipe_id)
            .bind(step.duration_ms)
            .bind(step.target)
            .bind(step.ucl)
            .bind(step.lcl)
            .execute(&mut *tx)
            .await?;
        }

        for recipe in scenario.recipes() {
            sqlx::query(
                "INSERT INTO recipe (id, version, checksum, body) VALUES ($1, $2, $3, $4)
                 ON CONFLICT (id) DO UPDATE SET version = EXCLUDED.version,
                   checksum = EXCLUDED.checksum, body = EXCLUDED.body",
            )
            .bind(&recipe.id)
            .bind(recipe.version)
            .bind(&recipe.checksum)
            .bind(&recipe.body)
            .execute(&mut *tx)
            .await?;
        }

        for equipment in scenario.equipment() {
            let qualification = serde_json::to_value(&equipment.qualification)
                .map_err(|error| StoreError::Message(error.to_string()))?;
            sqlx::query(
                "INSERT INTO equipment (id, kind, state, chamber, qualification_matrix)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (id) DO UPDATE SET kind = EXCLUDED.kind,
                   chamber = EXCLUDED.chamber, qualification_matrix = EXCLUDED.qualification_matrix",
            )
            .bind(&equipment.id)
            .bind(&equipment.kind)
            .bind(equipment.state.as_str())
            .bind(&equipment.chamber)
            .bind(qualification)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Append events, project them and queue them for delivery — one
    /// transaction. An event already in the log (same `event_id`) is skipped
    /// entirely, which makes a retried append idempotent.
    pub async fn append_events(&self, events: &[DomainEvent], route: &Route) -> Result<i64> {
        if events.is_empty() {
            return Ok(0);
        }
        let mut tx = self.pool.begin().await?;
        let mut last_seq = 0_i64;
        for event in events {
            let occurred_at = chrono::DateTime::from_timestamp_millis(event.occurred_at_ms)
                .unwrap_or_else(chrono::Utc::now);
            let seq: Option<i64> = sqlx::query_scalar(
                "INSERT INTO lot_event (event_id, lot_id, wafer_id, kind, payload, occurred_at, occurred_at_ms, actor)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 ON CONFLICT (event_id) DO NOTHING
                 RETURNING seq",
            )
            .bind(event.event_id)
            .bind(&event.lot_id)
            .bind(&event.wafer_id)
            .bind(event.kind.as_str())
            .bind(&event.payload)
            .bind(occurred_at)
            .bind(event.occurred_at_ms)
            .bind(&event.actor)
            .fetch_optional(&mut *tx)
            .await?;
            let Some(seq) = seq else {
                // Already applied: projections and outbox already exist. The
                // caller still needs the row's position, or the next snapshot
                // would claim a sequence that does not include it and replay
                // the event onto a state that already has it.
                let existing: i64 =
                    sqlx::query_scalar("SELECT seq FROM lot_event WHERE event_id = $1")
                        .bind(event.event_id)
                        .fetch_one(&mut *tx)
                        .await?;
                last_seq = last_seq.max(existing);
                continue;
            };
            last_seq = last_seq.max(seq);
            project(&mut tx, event).await?;

            let rows: Vec<MetricRow> = derive(event, route);
            let payload = json!({ "rows": rows });
            sqlx::query(
                "INSERT INTO outbox (event_id, aggregate_id, topic, payload)
                 VALUES ($1, $2, $3, $4) ON CONFLICT (event_id) DO NOTHING",
            )
            .bind(event.event_id)
            .bind(&event.lot_id)
            .bind("metrics")
            .bind(payload)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(last_seq)
    }

    pub async fn events_since(&self, since: i64, limit: i64) -> Result<Vec<PersistedEvent>> {
        let rows = sqlx::query_as::<_, EventColumns>(
            "SELECT seq, event_id, lot_id, wafer_id, kind, payload, occurred_at_ms, actor
             FROM lot_event WHERE seq > $1 ORDER BY seq LIMIT $2",
        )
        .bind(since)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(event_from_row).collect()
    }

    pub async fn events_for_lot(&self, lot_id: &str) -> Result<Vec<PersistedEvent>> {
        let rows = sqlx::query_as::<_, EventColumns>(
            "SELECT seq, event_id, lot_id, wafer_id, kind, payload, occurred_at_ms, actor
             FROM lot_event WHERE lot_id = $1 ORDER BY seq",
        )
        .bind(lot_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(event_from_row).collect()
    }

    /// The events of the lot and of every lot it was split from, in log order.
    ///
    /// A split keeps the wafer ids but starts a new lot, so the steps a child
    /// ran before the split are recorded under the parent's `lot_id`. The
    /// genealogy must include them; the lot's own event list must not, which is
    /// why this is a second query rather than a wider [`Self::events_for_lot`].
    async fn events_for_lineage(&self, lot_id: &str) -> Result<Vec<PersistedEvent>> {
        let rows = sqlx::query_as::<_, EventColumns>(
            "WITH RECURSIVE lineage(id) AS (
                 SELECT $1::text
                 UNION
                 SELECT lot.parent_lot_id
                 FROM lot JOIN lineage ON lot.id = lineage.id
                 WHERE lot.parent_lot_id IS NOT NULL
             )
             SELECT e.seq, e.event_id, e.lot_id, e.wafer_id, e.kind, e.payload,
                    e.occurred_at_ms, e.actor
             FROM lot_event e JOIN lineage ON lineage.id = e.lot_id
             ORDER BY e.seq",
        )
        .bind(lot_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(event_from_row).collect()
    }

    pub async fn all_events(&self) -> Result<Vec<PersistedEvent>> {
        let rows = sqlx::query_as::<_, EventColumns>(
            "SELECT seq, event_id, lot_id, wafer_id, kind, payload, occurred_at_ms, actor
             FROM lot_event ORDER BY seq",
        )
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(event_from_row).collect()
    }

    pub async fn event_by_id(&self, event_id: Uuid) -> Result<Option<PersistedEvent>> {
        let row = sqlx::query_as::<_, EventColumns>(
            "SELECT seq, event_id, lot_id, wafer_id, kind, payload, occurred_at_ms, actor
             FROM lot_event WHERE event_id = $1",
        )
        .bind(event_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(event_from_row).transpose()
    }

    pub async fn outbox_trace(&self, event_id: Uuid) -> Result<Option<OutboxTrace>> {
        let row = sqlx::query_as::<
            _,
            (
                Uuid,
                String,
                String,
                i32,
                chrono::DateTime<chrono::Utc>,
                Option<chrono::DateTime<chrono::Utc>>,
                Option<String>,
                Value,
            ),
        >(
            "SELECT event_id, aggregate_id, topic, attempts, created_at, published_at, last_error, payload
             FROM outbox WHERE event_id = $1",
        )
        .bind(event_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(
            |(
                event_id,
                aggregate_id,
                topic,
                attempts,
                created_at,
                published_at,
                last_error,
                payload,
            )| {
                OutboxTrace {
                    event_id,
                    aggregate_id,
                    topic,
                    attempts,
                    created_at: created_at.to_rfc3339(),
                    published_at: published_at.map(|at| at.to_rfc3339()),
                    last_error,
                    payload,
                }
            },
        ))
    }

    pub async fn outbox_health(&self) -> Result<OutboxHealth> {
        let (pending, max_attempts, oldest_ms, last_published_ms) =
            sqlx::query_as::<_, (i64, i32, i64, i64)>(
                "SELECT
               count(*) FILTER (WHERE published_at IS NULL)::BIGINT,
               coalesce(max(attempts), 0)::INT4,
               coalesce(extract(epoch FROM (now() - min(created_at)
                 FILTER (WHERE published_at IS NULL))) * 1000, 0)::BIGINT,
               coalesce(extract(epoch FROM (now() - max(published_at))) * 1000, -1)::BIGINT
             FROM outbox",
            )
            .fetch_one(&self.pool)
            .await?;
        Ok(OutboxHealth {
            pending,
            max_attempts,
            oldest_pending_age_ms: oldest_ms.max(0),
            last_published_age_ms: last_published_ms,
        })
    }

    pub async fn max_seq(&self) -> Result<i64> {
        let seq = sqlx::query_scalar::<_, Option<i64>>("SELECT max(seq) FROM lot_event")
            .fetch_one(&self.pool)
            .await?;
        Ok(seq.unwrap_or(0))
    }

    pub async fn genealogy(&self, lot_id: &str) -> Result<Option<LotGenealogy>> {
        let lot_row = sqlx::query_as::<_, LotColumns>(
            "SELECT id, product_id, route_id, priority, state, hold_reason, current_step,
                    wafer_count, created_at, parent_lot_id FROM lot WHERE id = $1",
        )
        .bind(lot_id)
        .fetch_optional(&self.pool)
        .await?;
        let Some(lot_row) = lot_row else {
            return Ok(None);
        };
        let wafer_rows = sqlx::query_as::<_, WaferColumns>(
            "SELECT id, lot_id, slot, state, defect_info, rework_count FROM wafer
             WHERE lot_id = $1 ORDER BY slot",
        )
        .bind(lot_id)
        .fetch_all(&self.pool)
        .await?;
        let loaded = wafer_rows
            .into_iter()
            .map(wafer_from_row)
            .collect::<Result<Vec<_>>>()?;
        let events = self.events_for_lot(lot_id).await?;
        let lineage = self.events_for_lineage(lot_id).await?;
        let genealogy = genealogy_for(
            &loaded,
            &lineage
                .iter()
                .map(|event| event.event.clone())
                .collect::<Vec<_>>(),
        );
        Ok(Some(LotGenealogy {
            lot: lot_from_row(lot_row)?,
            wafers: loaded,
            genealogy,
            events,
        }))
    }

    pub async fn counts(&self) -> Result<DbCounts> {
        let row: (i64, i64, i64, i64, i64, i64) = sqlx::query_as(
            "SELECT
               (SELECT count(*) FROM lot),
               (SELECT count(*) FROM wafer),
               (SELECT count(*) FROM lot_event),
               (SELECT count(*) FROM outbox WHERE published_at IS NULL),
               (SELECT count(*) FROM outbox WHERE published_at IS NOT NULL),
               (SELECT count(*) FROM published_event)",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(DbCounts {
            lots: row.0,
            wafers: row.1,
            events: row.2,
            outbox_pending: row.3,
            outbox_published: row.4,
            published_events: row.5,
        })
    }

    pub async fn outbox_stats(&self) -> Result<OutboxStats> {
        let row: (i64, i64, f64, i32) = sqlx::query_as(
            "SELECT
               count(*) FILTER (WHERE published_at IS NULL),
               count(*) FILTER (WHERE published_at IS NOT NULL),
               COALESCE(extract(epoch FROM (now() - min(created_at)
                 FILTER (WHERE published_at IS NULL))) * 1000, 0)::float8,
               COALESCE(max(attempts), 0)::int4
             FROM outbox",
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(OutboxStats {
            pending: row.0,
            published: row.1,
            oldest_pending_ms: row.2 as i64,
            max_attempts: row.3,
        })
    }

    /// Claim a batch with `FOR UPDATE SKIP LOCKED`; the row stays unpublished
    /// until the store has accepted it.
    pub async fn claim_outbox(&self, batch: i64) -> Result<Vec<OutboxRow>> {
        let rows = sqlx::query_as::<_, (Uuid, Value, i32)>(
            "UPDATE outbox SET attempts = attempts + 1
             WHERE event_id IN (
               SELECT event_id FROM outbox WHERE published_at IS NULL
               ORDER BY created_at LIMIT $1 FOR UPDATE SKIP LOCKED
             )
             RETURNING event_id, payload, attempts",
        )
        .bind(batch)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(event_id, payload, attempts)| OutboxRow {
                event_id,
                payload,
                attempts,
            })
            .collect())
    }

    pub async fn mark_published(&self, event_ids: &[Uuid], store: &str) -> Result<()> {
        if event_ids.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "UPDATE outbox SET published_at = now(), last_error = NULL WHERE event_id = ANY($1)",
        )
        .bind(event_ids)
        .execute(&mut *tx)
        .await?;
        for event_id in event_ids {
            sqlx::query(
                "INSERT INTO published_event (event_id, store) VALUES ($1, $2)
                 ON CONFLICT (event_id) DO NOTHING",
            )
            .bind(event_id)
            .bind(store)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn mark_failed(&self, event_id: Uuid, error: &str) -> Result<()> {
        sqlx::query("UPDATE outbox SET last_error = $2 WHERE event_id = $1")
            .bind(event_id)
            .bind(error)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn save_snapshot(
        &self,
        aggregate_id: &str,
        state: &Value,
        last_seq: i64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO snapshot (aggregate_id, state, last_seq) VALUES ($1, $2, $3)
             ON CONFLICT (aggregate_id) DO UPDATE SET state = EXCLUDED.state,
               last_seq = EXCLUDED.last_seq, created_at = now()",
        )
        .bind(aggregate_id)
        .bind(state)
        .bind(last_seq)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn load_snapshot(&self, aggregate_id: &str) -> Result<Option<Snapshot>> {
        let row = sqlx::query_as::<_, (Value, i64, chrono::DateTime<chrono::Utc>)>(
            "SELECT state, last_seq, created_at FROM snapshot WHERE aggregate_id = $1",
        )
        .bind(aggregate_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|(state, last_seq, created_at)| Snapshot {
            state,
            last_seq,
            created_at: created_at.to_rfc3339(),
        }))
    }

    /// Store the latest report of one kind (`backup`, `restore`).
    pub async fn save_ops_report(&self, kind: &str, payload: &Value) -> Result<()> {
        sqlx::query(
            "INSERT INTO ops_report (kind, payload) VALUES ($1, $2)
             ON CONFLICT (kind) DO UPDATE SET payload = EXCLUDED.payload,
               reported_at = now()",
        )
        .bind(kind)
        .bind(payload)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn ops_report(&self, kind: &str) -> Result<Option<Value>> {
        let row: Option<Value> =
            sqlx::query_scalar("SELECT payload FROM ops_report WHERE kind = $1")
                .bind(kind)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row)
    }

    pub async fn audit(&self, who: &str, action: &str, target: &str, detail: &Value) -> Result<()> {
        sqlx::query("INSERT INTO audit_log (who, action, target, after) VALUES ($1, $2, $3, $4)")
            .bind(who)
            .bind(action)
            .bind(target)
            .bind(detail)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Development-only reset used by `scripts/smoke.sh` between runs.
    pub async fn reset(&self) -> Result<()> {
        sqlx::query(
            "TRUNCATE lot_event, outbox, published_event, snapshot, wafer, lot, audit_log RESTART IDENTITY CASCADE",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

/// Project one event onto the tables the dashboard reads.
///
/// One arm per `EventKind`, kept together so the vocabulary and its SQL
/// projection can be reviewed side by side.
#[expect(
    clippy::too_many_lines,
    reason = "one arm per event kind; splitting scatters the projection"
)]
async fn project(tx: &mut Transaction<'_, Postgres>, event: &DomainEvent) -> Result<()> {
    match event.kind {
        EventKind::LotCreated => {
            let wafers: Vec<String> = event
                .payload
                .get("wafers")
                .and_then(Value::as_array)
                .map_or_default(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                });
            let created_at = chrono::DateTime::from_timestamp_millis(event.occurred_at_ms)
                .unwrap_or_else(chrono::Utc::now);
            sqlx::query(
                "INSERT INTO lot (id, product_id, route_id, priority, state, current_step,
                                  wafer_count, created_at, parent_lot_id)
                 VALUES ($1, $2, $3, $4, $5, 0, $6, $7, NULL)
                 ON CONFLICT (id) DO NOTHING",
            )
            .bind(&event.lot_id)
            .bind(event.payload.get("product_id").and_then(Value::as_str))
            .bind(event.payload.get("route_id").and_then(Value::as_str))
            .bind(
                event
                    .payload
                    .get("priority")
                    .and_then(Value::as_i64)
                    .unwrap_or(0) as i32,
            )
            .bind(LotState::Created.as_str())
            .bind(wafers.len() as i32)
            .bind(created_at)
            .execute(&mut **tx)
            .await?;
            for (index, wafer_id) in wafers.iter().enumerate() {
                sqlx::query(
                    "INSERT INTO wafer (id, lot_id, slot, state, rework_count)
                     VALUES ($1, $2, $3, $4, 0) ON CONFLICT (id) DO NOTHING",
                )
                .bind(wafer_id)
                .bind(&event.lot_id)
                .bind(index as i32 + 1)
                .bind(WaferState::Created.as_str())
                .execute(&mut **tx)
                .await?;
            }
        }
        EventKind::LotQueued => {
            let step = event.step_no().unwrap_or(0);
            sqlx::query(
                "UPDATE lot SET state = CASE WHEN state IN ('completed','split','merged','on_hold')
                                             THEN state ELSE $3 END,
                                current_step = $2
                 WHERE id = $1",
            )
            .bind(&event.lot_id)
            .bind(step)
            .bind(LotState::Queued.as_str())
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "UPDATE wafer SET state = 'queued'
                 WHERE lot_id = $1 AND state IN ('created','waiting','queued')
                   AND (SELECT state FROM lot WHERE id = $1) <> 'on_hold'",
            )
            .bind(&event.lot_id)
            .execute(&mut **tx)
            .await?;
        }
        EventKind::RecipeVerified => {}
        EventKind::TrackIn => {
            sqlx::query("UPDATE lot SET state = 'processing' WHERE id = $1 AND state <> 'on_hold'")
                .bind(&event.lot_id)
                .execute(&mut **tx)
                .await?;
            let wafers = event.wafers();
            sqlx::query("UPDATE wafer SET state = 'processing' WHERE id = ANY($1)")
                .bind(&wafers)
                .execute(&mut **tx)
                .await?;
        }
        EventKind::TrackOut | EventKind::TrackOutDuplicate => {
            if event.kind == EventKind::TrackOut {
                let wafers = event.wafers();
                sqlx::query("UPDATE wafer SET state = 'waiting' WHERE id = ANY($1)")
                    .bind(&wafers)
                    .execute(&mut **tx)
                    .await?;
                sqlx::query(
                    "UPDATE lot SET state = CASE
                        WHEN EXISTS (SELECT 1 FROM wafer WHERE lot_id = $1 AND state = 'processing')
                        THEN 'processing' ELSE 'waiting' END
                     WHERE id = $1 AND state NOT IN ('completed','split','merged','on_hold')",
                )
                .bind(&event.lot_id)
                .execute(&mut **tx)
                .await?;
            }
        }
        EventKind::LotCompleted => {
            sqlx::query(
                "UPDATE wafer SET state = 'done' WHERE lot_id = $1 AND state <> 'scrapped'",
            )
            .bind(&event.lot_id)
            .execute(&mut **tx)
            .await?;
            sqlx::query("UPDATE lot SET state = 'completed', hold_reason = NULL WHERE id = $1")
                .bind(&event.lot_id)
                .execute(&mut **tx)
                .await?;
        }
        EventKind::LotHeld => {
            sqlx::query("UPDATE lot SET state = 'on_hold', hold_reason = $2 WHERE id = $1")
                .bind(&event.lot_id)
                .bind(event.payload.get("reason").and_then(Value::as_str))
                .execute(&mut **tx)
                .await?;
        }
        EventKind::LotReleased => {
            sqlx::query(
                "UPDATE lot SET state = CASE
                     WHEN EXISTS (SELECT 1 FROM wafer WHERE lot_id = $1 AND state = 'processing')
                     THEN 'processing' ELSE 'queued' END,
                 hold_reason = NULL
                 WHERE id = $1",
            )
            .bind(&event.lot_id)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "UPDATE wafer SET state = 'queued'
                 WHERE lot_id = $1 AND state IN ('created','waiting')",
            )
            .bind(&event.lot_id)
            .execute(&mut **tx)
            .await?;
        }
        EventKind::ReworkStarted => {
            let to_step = event
                .payload
                .get("to_step")
                .and_then(Value::as_i64)
                .unwrap_or(0) as i32;
            sqlx::query(
                "UPDATE lot SET state = CASE WHEN hold_reason IS NULL THEN 'waiting' ELSE state END,
                                current_step = $2
                 WHERE id = $1",
            )
            .bind(&event.lot_id)
            .bind(to_step)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "UPDATE wafer SET state = 'waiting', rework_count = rework_count + 1
                 WHERE lot_id = $1 AND state <> 'scrapped'",
            )
            .bind(&event.lot_id)
            .execute(&mut **tx)
            .await?;
        }
        EventKind::WaferScrapped => {
            let wafer_id = event
                .payload
                .get("wafer_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            sqlx::query("UPDATE wafer SET state = 'scrapped', defect_info = $2 WHERE id = $1")
                .bind(wafer_id)
                .bind(event.payload.get("reason").and_then(Value::as_str))
                .execute(&mut **tx)
                .await?;
        }
        EventKind::LotSplit => {
            let step = event.step_no().unwrap_or(0);
            let created_at = chrono::DateTime::from_timestamp_millis(event.occurred_at_ms)
                .unwrap_or_else(chrono::Utc::now);
            let children: Vec<Value> = event
                .payload
                .get("children")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for child in children {
                let Some(child_id) = child.get("lot_id").and_then(Value::as_str) else {
                    continue;
                };
                let wafers: Vec<String> = child
                    .get("wafers")
                    .and_then(Value::as_array)
                    .map_or_default(|items| {
                        items
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    });
                sqlx::query(
                    "INSERT INTO lot (id, product_id, route_id, priority, state, current_step,
                                      wafer_count, created_at, parent_lot_id)
                     SELECT $2, product_id, route_id, priority, 'waiting', $3, $4, $5, id
                     FROM lot WHERE id = $1
                     ON CONFLICT (id) DO NOTHING",
                )
                .bind(&event.lot_id)
                .bind(child_id)
                .bind(step)
                .bind(wafers.len() as i32)
                .bind(created_at)
                .execute(&mut **tx)
                .await?;
                for (index, wafer_id) in wafers.iter().enumerate() {
                    sqlx::query(
                        "UPDATE wafer SET lot_id = $2, slot = $3, state = 'waiting' WHERE id = $1",
                    )
                    .bind(wafer_id)
                    .bind(child_id)
                    .bind(index as i32 + 1)
                    .execute(&mut **tx)
                    .await?;
                }
            }
            sqlx::query(
                "UPDATE lot SET state = 'split', wafer_count = 0, hold_reason = NULL WHERE id = $1",
            )
            .bind(&event.lot_id)
            .execute(&mut **tx)
            .await?;
        }
        EventKind::LotMerged => {
            let target = event
                .payload
                .get("target_lot_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let wafers: Vec<String> = event
                .payload
                .get("wafers")
                .and_then(Value::as_array)
                .map_or_default(|items| {
                    items
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                });
            let existing: i64 = sqlx::query_scalar("SELECT count(*) FROM wafer WHERE lot_id = $1")
                .bind(target)
                .fetch_one(&mut **tx)
                .await?;
            for (index, wafer_id) in wafers.iter().enumerate() {
                sqlx::query(
                    "UPDATE wafer SET lot_id = $2, slot = $3, state = 'waiting' WHERE id = $1",
                )
                .bind(wafer_id)
                .bind(target)
                .bind(existing as i32 + index as i32 + 1)
                .execute(&mut **tx)
                .await?;
            }
            sqlx::query(
                "UPDATE lot SET state = 'merged', wafer_count = 0, hold_reason = NULL WHERE id = $1",
            )
            .bind(&event.lot_id)
            .execute(&mut **tx)
            .await?;
            sqlx::query(
                "UPDATE lot SET wafer_count = (SELECT count(*) FROM wafer WHERE lot_id = $1),
                                state = 'waiting', hold_reason = NULL WHERE id = $1",
            )
            .bind(target)
            .execute(&mut **tx)
            .await?;
        }
        EventKind::EquipmentDown => {
            if let Some(equipment_id) = event.equipment_id() {
                sqlx::query("UPDATE equipment SET state = 'down' WHERE id = $1")
                    .bind(equipment_id)
                    .execute(&mut **tx)
                    .await?;
            }
        }
        EventKind::EquipmentUp => {
            if let Some(equipment_id) = event.equipment_id() {
                sqlx::query("UPDATE equipment SET state = 'idle' WHERE id = $1")
                    .bind(equipment_id)
                    .execute(&mut **tx)
                    .await?;
            }
        }
    }
    Ok(())
}

/// The columns of `lot_event`, as `query_as` reads them without a derive
/// macro (the build has no `sqlx-macros`).
type EventColumns = (
    i64,
    Uuid,
    String,
    Option<String>,
    String,
    Value,
    i64,
    String,
);

/// Rebuild a `DomainEvent` from its columns.
///
/// An unknown `kind` is an error, not a default: a replay that silently
/// reclassified a row would corrupt the projection it is rebuilding.
fn event_from_row(row: EventColumns) -> Result<PersistedEvent> {
    let (seq, event_id, lot_id, wafer_id, kind, payload, occurred_at_ms, actor) = row;
    Ok(PersistedEvent {
        seq,
        event: DomainEvent {
            event_id,
            lot_id,
            wafer_id,
            kind: EventKind::parse(&kind).ok_or_else(|| StoreError::UnknownValue {
                table: "lot_event",
                column: "kind",
                value: kind,
            })?,
            payload,
            occurred_at_ms,
            actor,
        },
    })
}

type LotColumns = (
    String,
    String,
    String,
    i32,
    String,
    Option<String>,
    i32,
    i32,
    chrono::DateTime<chrono::Utc>,
    Option<String>,
);

fn lot_from_row(row: LotColumns) -> Result<Lot> {
    let (
        id,
        product_id,
        route_id,
        priority,
        state,
        hold_reason,
        current_step,
        wafer_count,
        created_at,
        parent_lot_id,
    ) = row;
    Ok(Lot {
        id,
        product_id,
        route_id,
        priority,
        state: LotState::parse(&state).ok_or_else(|| StoreError::UnknownValue {
            table: "lot",
            column: "state",
            value: state,
        })?,
        hold_reason,
        current_step,
        wafer_count,
        created_at_ms: created_at.timestamp_millis(),
        parent_lot_id,
    })
}

type WaferColumns = (String, String, i32, String, Option<String>, i32);

fn wafer_from_row(row: WaferColumns) -> Result<Wafer> {
    let (id, lot_id, slot, state, defect_info, rework_count) = row;
    Ok(Wafer {
        id,
        lot_id,
        slot,
        state: WaferState::parse(&state).ok_or_else(|| StoreError::UnknownValue {
            table: "wafer",
            column: "state",
            value: state,
        })?,
        defect_info,
        rework_count: rework_count.max(0) as u32,
    })
}

#[derive(Debug, Clone)]
pub struct OutboxRow {
    pub event_id: Uuid,
    pub payload: Value,
    pub attempts: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row written by a newer build must stop the replay, not be silently
    /// reclassified as `lot_created`.
    #[test]
    fn an_unknown_event_kind_is_an_error() {
        let row: EventColumns = (
            1,
            Uuid::nil(),
            "LOT-0001".to_owned(),
            None,
            "lot_exploded".to_owned(),
            Value::Null,
            0,
            "test".to_owned(),
        );
        let error = event_from_row(row).unwrap_err();
        assert!(matches!(
            error,
            StoreError::UnknownValue {
                column: "kind",
                value,
                ..
            } if value == "lot_exploded"
        ));
    }

    #[test]
    fn an_unknown_lot_state_is_an_error() {
        let row: LotColumns = (
            "LOT-0001".to_owned(),
            "PRD-OXIDE-300".to_owned(),
            "RTE-LOGIC-4".to_owned(),
            5,
            "haunted".to_owned(),
            None,
            1,
            2,
            chrono::Utc::now(),
            None,
        );
        let error = lot_from_row(row).unwrap_err();
        assert!(matches!(
            error,
            StoreError::UnknownValue {
                table: "lot",
                column: "state",
                ..
            }
        ));
    }
}
