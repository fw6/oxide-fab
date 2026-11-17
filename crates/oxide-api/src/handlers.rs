//! Request handlers.
//!
//! Every mutating handler applies the change to the in-memory engine first,
//! then appends the resulting events in one PostgreSQL transaction. If the
//! database is unreachable the request fails with 503 before any state is
//! written, so the dashboard never shows a lot move that the log does not
//! contain.

use std::sync::Arc;

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use oxide_core::events::DomainEvent;
use oxide_sim::Simulation;
use oxide_store::outbox::{ReplayReport, replay as replay_events};
use oxide_store::{DbCounts, OutboxStats};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::dto::{
    ActionResponse, BackupStatus, ConfigResponse, DashboardSnapshot, ErrorResponse, EventsResponse,
    HealthResponse, HoldRequest, LotDetail, MergeRequest, ReplayRequest, RestoreStatus,
    ReworkRequest, ScrapRequest, SpcResponse, SplitRequest, StatsResponse, TimelineResponse,
};
use crate::{AppState, Shared};

#[derive(Debug)]
pub enum ApiError {
    Unavailable(String),
    NotFound(String),
    BadRequest(String),
    Internal(String),
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            ApiError::Unavailable(message) => (StatusCode::SERVICE_UNAVAILABLE, message),
            ApiError::NotFound(message) => (StatusCode::NOT_FOUND, message),
            ApiError::BadRequest(message) => (StatusCode::BAD_REQUEST, message),
            ApiError::Internal(message) => (StatusCode::INTERNAL_SERVER_ERROR, message),
        };
        (status, Json(ErrorResponse { error: message })).into_response()
    }
}

impl From<oxide_store::StoreError> for ApiError {
    fn from(error: oxide_store::StoreError) -> Self {
        if error.is_unavailable() {
            // The chaos contract: while PostgreSQL is away the control plane
            // answers 503, so a client retries instead of treating it as a bug.
            ApiError::Unavailable(error.to_string())
        } else {
            ApiError::Internal(error.to_string())
        }
    }
}

impl From<oxide_sim::SimError> for ApiError {
    fn from(error: oxide_sim::SimError) -> Self {
        let message = error.to_string();
        match error {
            // A lot the operator named that this fab does not have.
            oxide_sim::SimError::UnknownLot(_) => ApiError::NotFound(message),
            // The server's own configuration is unrunnable.
            oxide_sim::SimError::Scenario(_) => ApiError::Internal(message),
            // A legal request for an illegal transition.
            oxide_sim::SimError::Transition(_) | oxide_sim::SimError::MergingIntoSelf => {
                ApiError::BadRequest(message)
            }
        }
    }
}

/// The simulator, or a 500 when a previous holder panicked.
fn lock_sim(state: &AppState) -> Result<std::sync::MutexGuard<'_, Simulation>, ApiError> {
    state
        .sim
        .lock()
        .map_err(|_| ApiError::Internal("simulation lock poisoned".to_owned()))
}

/// Refuse the request with 503 when PostgreSQL is not reachable.
pub async fn require_database(
    State(state): State<Shared>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let alive = tokio::time::timeout(std::time::Duration::from_millis(800), state.store.ping())
        .await
        .unwrap_or(false);
    if !alive {
        return ApiError::Unavailable(
            "database unavailable: the control plane is read-only until PostgreSQL returns"
                .to_owned(),
        )
        .into_response();
    }
    next.run(request).await
}

pub async fn index() -> Json<Value> {
    Json(json!({
        "service": "oxide",
        "display": "氧化层",
        "repo": "fw6/oxide-fab",
        "endpoints": [
            "/healthz", "/api/snapshot", "/api/events?since=0", "/api/lots/{id}",
            "/api/spc", "/api/outbox/timeline", "/api/secs", "/api/backup",
            "/api/restore", "/api/stats", "/admin/replay"
        ],
    }))
}

pub async fn health(State(state): State<Shared>) -> Response {
    let database = tokio::time::timeout(std::time::Duration::from_millis(800), state.store.ping())
        .await
        .unwrap_or(false);
    let greptime =
        tokio::time::timeout(std::time::Duration::from_millis(800), state.greptime.ping())
            .await
            .unwrap_or(false);
    let body = HealthResponse {
        status: if database { "ok" } else { "degraded" }.to_owned(),
        database,
        greptime,
        uptime_ms: state.started_at.elapsed().as_millis(),
    };
    let status = if database {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, Json(body)).into_response()
}

pub async fn snapshot(State(state): State<Shared>) -> Result<Json<DashboardSnapshot>, ApiError> {
    let sim = lock_sim(&state)?.view();
    let db = state.store.counts().await?;
    let outbox = state.store.outbox_stats().await?;
    let last_seq = state.store.max_seq().await?;
    let greptime_ok =
        tokio::time::timeout(std::time::Duration::from_millis(700), state.greptime.ping())
            .await
            .unwrap_or(false);
    let (spc, spc_source) = read_spc(&state).await;
    let secs = state.secs.as_ref().map(|handle| handle.status());
    let backup = backup_from_db(&state).await;
    let restore = restore_from_db(&state).await;
    Ok(Json(DashboardSnapshot {
        sim,
        db,
        outbox,
        last_seq,
        greptime_ok,
        secs,
        backup,
        restore,
        spc,
        spc_source,
        chaos: state.timeline.samples(),
    }))
}

#[derive(Debug, Deserialize)]
pub struct EventsQuery {
    #[serde(default)]
    pub since: i64,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    500
}

pub async fn events_since(
    State(state): State<Shared>,
    Query(query): Query<EventsQuery>,
) -> Result<Json<EventsResponse>, ApiError> {
    let events = state
        .store
        .events_since(query.since, query.limit.clamp(1, 2000))
        .await?;
    let last_seq = state.store.max_seq().await?;
    Ok(Json(EventsResponse { events, last_seq }))
}

pub async fn lot_detail(
    State(state): State<Shared>,
    Path(lot_id): Path<String>,
) -> Result<Json<LotDetail>, ApiError> {
    let detail = state
        .store
        .genealogy(&lot_id)
        .await?
        .ok_or_else(|| ApiError::NotFound(format!("unknown lot {lot_id}")))?;
    Ok(Json(LotDetail {
        lot: detail.lot,
        wafers: detail.wafers,
        genealogy: detail.genealogy,
        events: detail.events,
    }))
}

pub async fn hold_lot(
    State(state): State<Shared>,
    Path(lot_id): Path<String>,
    Json(request): Json<HoldRequest>,
) -> Result<Json<ActionResponse>, ApiError> {
    let events = {
        let _guard = state.write_lock.lock().await;
        lock_sim(&state)?.hold_lot(&lot_id, &request.reason)?
    };
    finish_action(&state, &lot_id, "hold", events).await
}

pub async fn release_lot(
    State(state): State<Shared>,
    Path(lot_id): Path<String>,
) -> Result<Json<ActionResponse>, ApiError> {
    let events = {
        let _guard = state.write_lock.lock().await;
        lock_sim(&state)?.release_lot(&lot_id)?
    };
    finish_action(&state, &lot_id, "release", events).await
}

pub async fn rework_lot(
    State(state): State<Shared>,
    Path(lot_id): Path<String>,
    Json(request): Json<ReworkRequest>,
) -> Result<Json<ActionResponse>, ApiError> {
    let events = {
        let _guard = state.write_lock.lock().await;
        lock_sim(&state)?.rework_lot(&lot_id, request.to_step, &request.reason)?
    };
    finish_action(&state, &lot_id, "rework", events).await
}

pub async fn split_lot(
    State(state): State<Shared>,
    Path(lot_id): Path<String>,
    Json(request): Json<SplitRequest>,
) -> Result<Json<ActionResponse>, ApiError> {
    let events = {
        let _guard = state.write_lock.lock().await;
        lock_sim(&state)?.split_lot(&lot_id, &request.children)?
    };
    finish_action(&state, &lot_id, "split", events).await
}

pub async fn merge_lots(
    State(state): State<Shared>,
    Path(lot_id): Path<String>,
    Json(request): Json<MergeRequest>,
) -> Result<Json<ActionResponse>, ApiError> {
    let events = {
        let _guard = state.write_lock.lock().await;
        lock_sim(&state)?.merge_lots(&lot_id, &request.sources)?
    };
    finish_action(&state, &lot_id, "merge", events).await
}

pub async fn scrap_wafer(
    State(state): State<Shared>,
    Path(lot_id): Path<String>,
    Json(request): Json<ScrapRequest>,
) -> Result<Json<ActionResponse>, ApiError> {
    let events = {
        let _guard = state.write_lock.lock().await;
        lock_sim(&state)?.scrap_wafer(&lot_id, &request.wafer_id, &request.reason)?
    };
    finish_action(&state, &lot_id, "scrap", events).await
}

async fn finish_action(
    state: &Arc<AppState>,
    lot_id: &str,
    action: &str,
    events: Vec<DomainEvent>,
) -> Result<Json<ActionResponse>, ApiError> {
    let count = events.len();
    let last_seq = state.store.append_events(&events, state.route()).await?;
    lock_sim(state)?.mark_persisted(count as u64);
    let detail = json!({ "events": count, "action": action });
    let _ = state.store.audit("operator", action, lot_id, &detail).await;
    Ok(Json(ActionResponse {
        events: count,
        last_seq,
        message: format!("{action} applied to {lot_id}"),
    }))
}

pub async fn play(State(state): State<Shared>) -> Result<Json<Value>, ApiError> {
    lock_sim(&state)?.set_running(true);
    Ok(Json(json!({ "running": true })))
}

pub async fn pause(State(state): State<Shared>) -> Result<Json<Value>, ApiError> {
    lock_sim(&state)?.set_running(false);
    Ok(Json(json!({ "running": false })))
}

pub async fn step(State(state): State<Shared>) -> Result<Json<ActionResponse>, ApiError> {
    let events = {
        let _guard = state.write_lock.lock().await;
        let mut sim = lock_sim(&state)?;
        sim.set_running(false);
        sim.step().events
    };
    let count = events.len();
    let last_seq = state.store.append_events(&events, state.route()).await?;
    lock_sim(&state)?.mark_persisted(count as u64);
    Ok(Json(ActionResponse {
        events: count,
        last_seq,
        message: "one simulation tick".to_owned(),
    }))
}

pub async fn set_speed(
    State(state): State<Shared>,
    Path(speed): Path<f64>,
) -> Result<Json<Value>, ApiError> {
    if !(0.1..=100.0).contains(&speed) {
        return Err(ApiError::BadRequest("speed must be 0.1 .. 100".to_owned()));
    }
    lock_sim(&state)?.set_speed(speed);
    Ok(Json(json!({ "speed": speed })))
}

async fn read_spc(state: &Arc<AppState>) -> (Vec<oxide_store::SpcBucket>, String) {
    if let Ok(buckets) = state.greptime.spc_series(120).await
        && !buckets.is_empty()
    {
        return (buckets, "flow".to_owned());
    }
    match state.greptime.raw_spc_series(120).await {
        Ok(buckets) => (buckets, "raw".to_owned()),
        Err(_) => (vec![], "unavailable".to_owned()),
    }
}

pub async fn spc(State(state): State<Shared>) -> Json<SpcResponse> {
    let (buckets, source) = read_spc(&state).await;
    Json(SpcResponse { source, buckets })
}

pub async fn outbox_timeline(
    State(state): State<Shared>,
) -> Result<Json<TimelineResponse>, ApiError> {
    Ok(Json(TimelineResponse {
        samples: state.timeline.samples(),
        stats: state.store.outbox_stats().await?,
    }))
}

pub async fn secs_status(State(state): State<Shared>) -> Json<Value> {
    match &state.secs {
        Some(handle) => Json(json!(handle.status())),
        None => Json(json!({ "enabled": false })),
    }
}

#[derive(Debug, Deserialize)]
pub struct DropQuery {
    /// How long the host stays away, in milliseconds (default 1500).
    pub hold_ms: Option<i64>,
}

pub async fn secs_drop(
    State(state): State<Shared>,
    Query(query): Query<DropQuery>,
) -> Result<Json<Value>, ApiError> {
    let handle = state
        .secs
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("SECS is not enabled".to_owned()))?;
    let hold_ms = query.hold_ms.unwrap_or(1500);
    handle.drop_link_for(hold_ms);
    Ok(Json(json!({ "dropped": true, "hold_ms": hold_ms })))
}

pub async fn secs_upload_recipe(
    State(state): State<Shared>,
    Path(recipe_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let handle = state
        .secs
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("SECS is not enabled".to_owned()))?;
    let recipe = state
        .scenario
        .recipe(&recipe_id)
        .ok_or_else(|| ApiError::NotFound(format!("unknown recipe {recipe_id}")))?;
    handle.upload_recipe(&recipe.id, &recipe.checksum, &recipe.body);
    Ok(Json(json!({
        "queued": recipe.id,
        "checksum": recipe.checksum,
    })))
}

async fn backup_from_db(state: &Arc<AppState>) -> BackupStatus {
    match state.store.ops_report("backup").await {
        Ok(Some(value)) => serde_json::from_value(value).unwrap_or_default(),
        _ => BackupStatus::default(),
    }
}

async fn restore_from_db(state: &Arc<AppState>) -> Option<RestoreStatus> {
    match state.store.ops_report("restore").await {
        Ok(Some(value)) => serde_json::from_value(value).ok(),
        _ => None,
    }
}

pub async fn backup_status(State(state): State<Shared>) -> Json<BackupStatus> {
    Json(backup_from_db(&state).await)
}

pub async fn restore_status(State(state): State<Shared>) -> Json<Option<RestoreStatus>> {
    Json(restore_from_db(&state).await)
}

/// The scripts POST their report here after a backup or a restore drill.
pub async fn report(
    State(state): State<Shared>,
    Path(kind): Path<String>,
    Json(payload): Json<Value>,
) -> Result<Json<Value>, ApiError> {
    if kind != "backup" && kind != "restore" {
        return Err(ApiError::BadRequest(
            "kind must be backup or restore".to_owned(),
        ));
    }
    state.store.save_ops_report(&kind, &payload).await?;
    Ok(Json(json!({ "stored": kind })))
}

pub async fn stats(State(state): State<Shared>) -> Result<Json<StatsResponse>, ApiError> {
    let db: DbCounts = state.store.counts().await?;
    let outbox: OutboxStats = state.store.outbox_stats().await?;
    let greptime = match state.greptime.table_counts().await {
        Ok(counts) => serde_json::to_value(counts).unwrap_or(Value::Null),
        Err(error) => json!({ "error": error.to_string() }),
    };
    let spc_average = state.greptime.spc_average().await.unwrap_or_default();
    let last_seq = state.store.max_seq().await?;
    Ok(Json(StatsResponse {
        db,
        outbox,
        greptime,
        spc_average,
        last_seq,
    }))
}

pub async fn config(State(state): State<Shared>) -> Json<ConfigResponse> {
    Json(ConfigResponse {
        scenario: (*state.scenario).clone(),
        equipment: state
            .scenario
            .equipment
            .iter()
            .map(|tool| tool.id.clone())
            .collect(),
        lots: state.scenario.lot_count,
        wafers_per_lot: state.scenario.wafers_per_lot,
    })
}

pub async fn replay(
    State(state): State<Shared>,
    Json(request): Json<ReplayRequest>,
) -> Result<Json<ReplayReport>, ApiError> {
    let report = replay_events(
        &state.store,
        &state.greptime,
        state.route(),
        request.truncate,
    )
    .await?;
    Ok(Json(report))
}

pub async fn reset(State(state): State<Shared>) -> Result<Json<Value>, ApiError> {
    // Stop the line and let the simulator's current batch commit or fail
    // before the log is truncated.
    lock_sim(&state)?.set_running(false);
    let _guard = state.write_lock.lock().await;
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    state.store.reset().await?;
    if let Err(error) = state.greptime.truncate_all().await {
        tracing::warn!(%error, "could not truncate GreptimeDB during reset");
    }
    let fresh = Simulation::new((*state.scenario).clone())?;
    let fresh_state = fresh.snapshot();
    *lock_sim(&state)? = fresh;
    let value = serde_json::to_value(&fresh_state)
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    state.store.save_snapshot("fab", &value, 0).await?;
    Ok(Json(json!({ "reset": true })))
}

#[derive(Debug, serde::Deserialize)]
pub struct RedeliverPath {
    event_id: uuid::Uuid,
}

/// Insert one event's derived rows a second time and report the row counts.
///
/// This is the idempotency contract made executable: the same `event_id` must
/// not add a point, because it is part of every table's primary key.
pub async fn redeliver(
    State(state): State<Shared>,
    Path(path): Path<RedeliverPath>,
) -> Result<Json<Value>, ApiError> {
    let events = state.store.all_events().await?;
    let persisted = events
        .into_iter()
        .find(|item| item.event.event_id == path.event_id)
        .ok_or_else(|| ApiError::NotFound(format!("unknown event {}", path.event_id)))?;
    let rows = oxide_core::metrics::derive(&persisted.event, state.route());
    let before = state.greptime.table_counts().await.unwrap_or_default();
    state
        .greptime
        .insert_rows(&rows)
        .await
        .map_err(|error| ApiError::Internal(error.to_string()))?;
    let after = state.greptime.table_counts().await.unwrap_or_default();
    let mut delta = 0_i64;
    for (table, count) in &after {
        let previous = before.get(table).copied().unwrap_or(0);
        delta += (count - previous).abs();
    }
    Ok(Json(json!({
        "event_id": path.event_id,
        "kind": persisted.event.kind.as_str(),
        "rows_redelivered": rows.len(),
        "before": before,
        "after": after,
        "delta": delta,
        "idempotent": delta == 0,
    })))
}
