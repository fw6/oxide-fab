//! `oxide` — the oxide-fab server.
//!
//! Wires the discrete-event simulator, the PostgreSQL event store, the outbox
//! projector, the simulated SECS/GEM pair and the HTTP API into one process.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use oxide_api::{AppState, Shared};
use oxide_core::Scenario;
use oxide_sim::{SecsTrigger, Simulation};
use oxide_store::outbox::Publisher;
use oxide_store::{GreptimeDb, OutboxTimeline, PgStore};

struct Config {
    http_addr: String,
    database_url: String,
    greptime_url: String,
    greptime_db: String,
    scenario_path: PathBuf,
    migrations_dir: String,
    secs_bind: String,
    secs_enabled: bool,
    migrate_only: bool,
}

impl Config {
    fn from_env() -> Result<Self> {
        let secs_enabled = std::env::var("SECS_ENABLED")
            .map(|value| value != "false" && value != "0")
            .unwrap_or(true);
        Ok(Self {
            http_addr: std::env::var("OXIDE_HTTP_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:8080".to_owned()),
            database_url: std::env::var("DATABASE_URL")
                .unwrap_or_else(|_| "postgres://oxide@127.0.0.1:5432/oxide".to_owned()),
            greptime_url: std::env::var("GREPTIME_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:4000".to_owned()),
            greptime_db: std::env::var("GREPTIME_DB").unwrap_or_else(|_| "public".to_owned()),
            scenario_path: PathBuf::from(
                std::env::var("SCENARIO_PATH")
                    .unwrap_or_else(|_| "config/scenario.json".to_owned()),
            ),
            migrations_dir: std::env::var("OXIDE_MIGRATIONS_DIR")
                .unwrap_or_else(|_| "migrations".to_owned()),
            secs_bind: std::env::var("SECS_BIND").unwrap_or_else(|_| "127.0.0.1:50010".to_owned()),
            secs_enabled,
            migrate_only: std::env::args().any(|arg| arg == "--migrate-only"),
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,oxide=debug".into()),
        )
        .init();

    let config = Config::from_env()?;
    let scenario_text = std::fs::read_to_string(&config.scenario_path)
        .with_context(|| format!("reading {}", config.scenario_path.display()))?;
    let scenario = Scenario::from_json(&scenario_text)
        .with_context(|| format!("parsing {}", config.scenario_path.display()))?;
    scenario.validate().map_err(anyhow::Error::msg)?;
    tracing::info!(line = %scenario.name, lots = scenario.lot_count, "scenario loaded");

    let store = connect_with_retry(&config.database_url).await?;
    store
        .migrate(&config.migrations_dir)
        .await
        .context("running migrations")?;
    store.seed(&scenario).await.context("seeding master data")?;
    let greptime = GreptimeDb::new(&config.greptime_url, &config.greptime_db);
    ensure_greptime(&greptime).await;

    if config.migrate_only {
        tracing::info!("migrations and schema are up to date");
        return Ok(());
    }

    let scenario = Arc::new(scenario);
    let (simulation, restored_from) = restore_simulation(&store, &scenario).await?;
    tracing::info!(from = %restored_from, "simulation ready");
    let simulation = Arc::new(Mutex::new(simulation));

    if store.max_seq().await.unwrap_or(0) == 0 {
        // A fresh log means a fresh demo: do not leave points from a previous
        // volume behind.
        if let Err(error) = greptime.truncate_all().await {
            tracing::warn!(%error, "could not clear GreptimeDB on a fresh log");
        }
    }

    let timeline = OutboxTimeline::new();
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);

    #[cfg(feature = "secs-gem")]
    let secs = if config.secs_enabled {
        let secs_config = oxide_secs::SecsConfig {
            bind: config
                .secs_bind
                .parse()
                .context("SECS_BIND is not an address")?,
            ..Default::default()
        };
        tracing::info!(bind = %config.secs_bind, "starting SECS/GEM pair");
        Some(Arc::new(oxide_secs::start_pair(secs_config)))
    } else {
        None
    };
    #[cfg(not(feature = "secs-gem"))]
    let secs: Option<Arc<oxide_secs::SecsHandle>> = {
        tracing::warn!("built without the secs-gem feature: the SECS/GEM pair is off");
        None
    };

    let state: Shared = Arc::new(AppState {
        sim: Arc::clone(&simulation),
        store: store.clone(),
        greptime: greptime.clone(),
        timeline: timeline.clone(),
        scenario: Arc::clone(&scenario),
        secs,
        started_at: Instant::now(),
        write_lock: Arc::new(tokio::sync::Mutex::new(())),
    });

    spawn_simulator(Arc::clone(&state), shutdown_rx.clone());
    let publisher = Publisher::new(store.clone(), greptime.clone(), timeline);
    tokio::spawn(publisher.run(shutdown_rx.clone()));

    let listener = tokio::net::TcpListener::bind(&config.http_addr)
        .await
        .with_context(|| format!("binding {}", config.http_addr))?;
    tracing::info!(addr = %config.http_addr, "oxide listening");
    let router = oxide_api::router(Arc::clone(&state));

    let (stop_tx, mut stop_rx) = tokio::sync::mpsc::channel::<()>(1);
    tokio::spawn(async move {
        let _ = tokio::signal::ctrl_c().await;
        let _ = stop_tx.send(()).await;
    });

    axum::serve(listener, router)
        .with_graceful_shutdown(async move {
            let _ = stop_rx.recv().await;
            tracing::info!("shutdown requested");
        })
        .await
        .context("serving HTTP")?;

    let _ = shutdown_tx.send(true);
    save_snapshot(&state).await;
    tracing::info!("oxide stopped");
    Ok(())
}

async fn connect_with_retry(url: &str) -> Result<PgStore> {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        match PgStore::connect(url, 16).await {
            Ok(store) => return Ok(store),
            Err(error) => {
                if Instant::now() >= deadline {
                    return Err(anyhow::anyhow!("could not reach PostgreSQL: {error}"));
                }
                tracing::warn!(%error, "waiting for PostgreSQL");
                tokio::time::sleep(Duration::from_millis(750)).await;
            }
        }
    }
}

async fn ensure_greptime(greptime: &GreptimeDb) {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        match greptime.ensure_schema().await {
            Ok(()) => {
                tracing::info!("GreptimeDB schema ready");
                return;
            }
            Err(error) => {
                if Instant::now() >= deadline {
                    tracing::error!(%error, "GreptimeDB schema is not ready; deliveries stay in the outbox");
                    return;
                }
                tracing::warn!(%error, "waiting for GreptimeDB");
                tokio::time::sleep(Duration::from_millis(750)).await;
            }
        }
    }
}

/// Resume from `snapshot + events after it`, or start a new fab.
async fn restore_simulation(store: &PgStore, scenario: &Scenario) -> Result<(Simulation, String)> {
    match store.load_snapshot("fab").await? {
        Some(snapshot) => {
            let state: oxide_sim::SimState = serde_json::from_value(snapshot.state)
                .context("snapshot state does not match SimState")?;
            let mut sim =
                Simulation::from_state(scenario.clone(), state).map_err(anyhow::Error::msg)?;
            let events = store.events_since(snapshot.last_seq, 100_000).await?;
            for persisted in &events {
                sim.apply_event(&persisted.event);
            }
            sim.rebuild_heap();
            Ok((
                sim,
                format!("snapshot@{} + {} events", snapshot.last_seq, events.len()),
            ))
        }
        None => Ok((
            Simulation::new(scenario.clone()).map_err(anyhow::Error::msg)?,
            "fresh".to_owned(),
        )),
    }
}

fn spawn_simulator(state: Shared, shutdown: tokio::sync::watch::Receiver<bool>) {
    tokio::spawn(async move {
        let shutdown = shutdown;
        let mut last_snapshot_events = 0_u64;
        let mut last = Instant::now();
        // Events produced by the engine but not yet committed to the log. If
        // PostgreSQL goes away the batch is kept and retried; the engine is
        // paused meanwhile so it cannot advance past the log.
        let mut pending: Vec<oxide_core::events::DomainEvent> = Vec::new();
        let mut paused_for_database = false;
        loop {
            if *shutdown.borrow() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
            let wall_ms = last.elapsed().as_millis() as i64;
            last = Instant::now();

            let mut secs = Vec::new();
            if pending.is_empty() {
                let output = {
                    let mut sim = match state.sim.lock() {
                        Ok(sim) => sim,
                        Err(_) => return,
                    };
                    let speed = sim.speed();
                    sim.tick(((wall_ms as f64) * speed) as i64)
                };
                pending.extend(output.events);
                secs = output.secs;
            }
            forward_secs(&state, secs);

            if !pending.is_empty() {
                let route = state.route().clone();
                match state.store.append_events(&pending, &route).await {
                    Ok(_) => {
                        let count = pending.len() as u64;
                        pending.clear();
                        if let Ok(mut sim) = state.sim.lock() {
                            sim.mark_persisted(count);
                            if paused_for_database {
                                sim.set_running(true);
                            }
                        }
                        paused_for_database = false;
                    }
                    Err(error) => {
                        tracing::error!(%error, count = pending.len(), "append failed; holding the batch and pausing dispatch");
                        if !paused_for_database {
                            if let Ok(mut sim) = state.sim.lock() {
                                sim.set_running(false);
                            }
                            paused_for_database = true;
                        }
                        tokio::time::sleep(Duration::from_millis(400)).await;
                    }
                }
            }

            let persisted = state
                .sim
                .lock()
                .map(|sim| sim.persisted_events())
                .unwrap_or(0);
            let every = state.scenario.snapshot_every_events.max(1);
            // Modulo, not a running difference: `/admin/reset` restarts the
            // counter, and a difference would then stay negative for a long
            // time and never checkpoint. The equality guard avoids saving
            // twice for the same count.
            if persisted > 0 && persisted.is_multiple_of(every) && persisted != last_snapshot_events
            {
                save_snapshot(&state).await;
                last_snapshot_events = persisted;
            }
        }
    });
}

fn forward_secs(state: &Shared, triggers: Vec<SecsTrigger>) {
    let Some(handle) = &state.secs else { return };
    for trigger in triggers {
        match trigger {
            SecsTrigger::RecipeUpload {
                recipe_id,
                checksum,
                body,
            } => handle.upload_recipe(&recipe_id, &checksum, &body),
            SecsTrigger::ProcessStarted {
                lot_id,
                step_no,
                equipment_id,
            } => handle.process_started(&lot_id, step_no, &equipment_id),
            SecsTrigger::ProcessCompleted {
                lot_id,
                step_no,
                equipment_id,
            } => handle.process_completed(&lot_id, step_no, &equipment_id),
            SecsTrigger::Alarm {
                equipment_id,
                alid,
                text,
            } => handle.alarm(&equipment_id, alid, &text),
        }
    }
}

async fn save_snapshot(state: &Shared) {
    let (snapshot, last_seq) = {
        let sim = match state.sim.lock() {
            Ok(sim) => sim,
            Err(_) => return,
        };
        (sim.snapshot(), sim.persisted_events())
    };
    let value = match serde_json::to_value(&snapshot) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(%error, "could not serialize the snapshot");
            return;
        }
    };
    let last_seq = state.store.max_seq().await.unwrap_or(last_seq as i64);
    if let Err(error) = state.store.save_snapshot("fab", &value, last_seq).await {
        tracing::warn!(%error, "could not write the snapshot");
    }
}
