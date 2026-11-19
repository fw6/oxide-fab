//! Axum HTTP API: the dashboard's snapshot, incremental event pull, genealogy,
//! SPC, simulation control and the SECS/GEM status.

pub mod dto;
pub mod handlers;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::Router;
use axum::routing::{get, post};
use oxide_core::Scenario;
use oxide_secs::SecsHandle;
use oxide_sim::Simulation;
use oxide_store::{GreptimeDb, OutboxTimeline, PgStore};

/// Coordinates the simulator task with operator writes and snapshots.
///
/// `catching_up` is set while the engine holds a batch `append_events` could
/// not commit. Its in-memory state is then ahead of the log, so no operator
/// decision may be based on it and no snapshot may capture it. `epoch` is
/// bumped by `/admin/reset` so the simulator drops a batch that belongs to the
/// previous demo instead of retrying it into the truncated log.
#[derive(Debug, Default)]
pub struct WriteGate {
    catching_up: AtomicBool,
    epoch: AtomicU64,
}

impl WriteGate {
    #[must_use]
    pub fn catching_up(&self) -> bool {
        self.catching_up.load(Ordering::Acquire)
    }

    pub fn set_catching_up(&self, value: bool) {
        self.catching_up.store(value, Ordering::Release);
    }

    #[must_use]
    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    /// Start over after `/admin/reset`: clear the gate and return the new
    /// epoch.
    pub fn reset(&self) -> u64 {
        self.catching_up.store(false, Ordering::Release);
        self.epoch.fetch_add(1, Ordering::AcqRel) + 1
    }
}

/// Everything a handler needs. Cheap to clone: all fields are handles.
#[derive(Debug)]
pub struct AppState {
    /// The engine. Not behind its own `Arc`: `AppState` is already shared.
    pub sim: Mutex<Simulation>,
    pub store: PgStore,
    pub greptime: GreptimeDb,
    pub timeline: OutboxTimeline,
    pub scenario: Arc<Scenario>,
    pub secs: Option<Arc<SecsHandle>>,
    pub started_at: Instant,
    /// Serializes every decision with its append, so `lot_event` order is the
    /// order the aggregate accepted.
    pub write_lock: Arc<tokio::sync::Mutex<()>>,
    pub write_gate: WriteGate,
    /// Serializes outbox delivery with the truncates performed by reset and
    /// replay.
    pub delivery_lock: Arc<tokio::sync::Mutex<()>>,
}

pub type Shared = Arc<AppState>;

impl AppState {
    pub fn route(&self) -> &oxide_core::Route {
        &self.scenario.route
    }
}

pub fn router(state: Shared) -> Router {
    let api = Router::new()
        .route("/snapshot", get(handlers::snapshot))
        .route("/events", get(handlers::events_since))
        .route("/events/{event_id}", get(handlers::event_trace))
        .route("/lots/{lot_id}", get(handlers::lot_detail))
        .route("/lots/{lot_id}/hold", post(handlers::hold_lot))
        .route("/lots/{lot_id}/release", post(handlers::release_lot))
        .route("/lots/{lot_id}/rework", post(handlers::rework_lot))
        .route("/lots/{lot_id}/split", post(handlers::split_lot))
        .route("/lots/{lot_id}/merge", post(handlers::merge_lots))
        .route("/lots/{lot_id}/scrap", post(handlers::scrap_wafer))
        .route("/sim/play", post(handlers::play))
        .route("/sim/pause", post(handlers::pause))
        .route("/sim/step", post(handlers::step))
        .route("/sim/speed/{speed}", post(handlers::set_speed))
        .route("/spc", get(handlers::spc))
        .route("/outbox/timeline", get(handlers::outbox_timeline))
        .route("/secs", get(handlers::secs_status))
        .route("/secs/drop", post(handlers::secs_drop))
        .route(
            "/secs/recipe/{recipe_id}",
            post(handlers::secs_upload_recipe),
        )
        .route("/backup", get(handlers::backup_status))
        .route("/restore", get(handlers::restore_status))
        .route("/stats", get(handlers::stats))
        .route("/config", get(handlers::config))
        .route("/analytics", get(handlers::analytics))
        .route("/monitor", get(handlers::monitor))
        .route("/what-if", post(handlers::what_if))
        .route_layer(axum::middleware::from_fn_with_state(
            Arc::clone(&state),
            handlers::require_database,
        ));

    Router::new()
        .route("/healthz", get(handlers::health))
        .route("/api", get(handlers::index))
        .nest("/api", api)
        .route("/admin/replay", post(handlers::replay))
        .route("/admin/reset", post(handlers::reset))
        .route("/admin/report/{kind}", post(handlers::report))
        .route("/admin/redeliver/{event_id}", post(handlers::redeliver))
        // Applied last so every route above, and the nested API, gets it.
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::WriteGate;

    #[test]
    fn reset_clears_the_gate_and_moves_the_epoch() {
        let gate = WriteGate::default();
        let epoch = gate.epoch();
        gate.set_catching_up(true);

        let next = gate.reset();

        assert!(!gate.catching_up());
        assert_eq!(gate.epoch(), next);
        assert!(next > epoch);
    }
}
