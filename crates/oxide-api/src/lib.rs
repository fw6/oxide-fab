//! Axum HTTP API: the dashboard's snapshot, incremental event pull, genealogy,
//! SPC, simulation control and the SECS/GEM status.

pub mod dto;
pub mod handlers;

use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::routing::{get, post};
use axum::Router;
use oxide_core::Scenario;
use oxide_secs::SecsHandle;
use oxide_sim::Simulation;
use oxide_store::{GreptimeDb, OutboxTimeline, PgStore};

/// Everything a handler needs. Cheap to clone: all fields are handles.
pub struct AppState {
    pub sim: Arc<Mutex<Simulation>>,
    pub store: PgStore,
    pub greptime: GreptimeDb,
    pub timeline: OutboxTimeline,
    pub scenario: Arc<Scenario>,
    pub secs: Option<Arc<SecsHandle>>,
    pub started_at: Instant,
    /// Serializes operator actions with the simulator's persist step.
    pub write_lock: Arc<tokio::sync::Mutex<()>>,
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
