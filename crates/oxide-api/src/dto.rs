//! Wire types of the HTTP API.
//!
//! Everything is `serde`-derived so the dashboard and the scripts share one
//! shape. Fields are additive only.

use oxide_core::genealogy::WaferGenealogy;
use oxide_core::model::{Lot, Wafer};
use oxide_secs::SecsStatus;
use oxide_sim::SimSnapshot;
use oxide_store::greptime::SpcBucket;
use oxide_store::outbox::OutboxSample;
use oxide_store::{DbCounts, OutboxStats, PersistedEvent};
use serde::{Deserialize, Serialize};

/// One dashboard refresh.
///
/// The floor (steps, tools, lots) lives only in `sim`; projecting it twice
/// would double the payload and let the two copies disagree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardSnapshot {
    pub sim: SimSnapshot,
    pub db: DbCounts,
    pub outbox: OutboxStats,
    pub last_seq: i64,
    pub greptime_ok: bool,
    pub secs: Option<SecsStatus>,
    pub backup: BackupStatus,
    pub restore: Option<RestoreStatus>,
    pub spc: Vec<SpcBucket>,
    pub spc_source: String,
    pub chaos: Vec<OutboxSample>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventsResponse {
    pub events: Vec<PersistedEvent>,
    pub last_seq: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LotDetail {
    pub lot: Lot,
    pub wafers: Vec<Wafer>,
    pub genealogy: Vec<WaferGenealogy>,
    pub events: Vec<PersistedEvent>,
}

/// Written by `scripts/backup.sh` into `artifacts/backup-latest.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackupStatus {
    pub taken_at: Option<String>,
    pub pg_full_backup: Option<String>,
    pub pg_logical_dump: Option<String>,
    pub greptime_snapshot: Option<String>,
    pub wal_continuity: Option<String>,
    pub ok: bool,
    pub log_path: Option<String>,
}

/// Written by `scripts/restore.sh` into `artifacts/restore-latest.json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RestoreStatus {
    pub drilled_at: Option<String>,
    pub project: Option<String>,
    pub target_time: Option<String>,
    pub recovered_at: Option<String>,
    pub lots: Option<i64>,
    pub events: Option<i64>,
    pub max_created_at: Option<String>,
    pub max_event_at: Option<String>,
    pub greptime_tables: Option<i64>,
    pub greptime_rows: Option<i64>,
    pub elapsed_ms: Option<i64>,
    pub ok: bool,
    pub log_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub database: bool,
    pub greptime: bool,
    pub uptime_ms: u128,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatsResponse {
    pub db: DbCounts,
    pub outbox: OutboxStats,
    pub greptime: serde_json::Value,
    pub spc_average: f64,
    pub last_seq: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpcResponse {
    pub source: String,
    pub buckets: Vec<SpcBucket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineResponse {
    pub samples: Vec<OutboxSample>,
    pub stats: OutboxStats,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeedRequest {
    pub speed: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoldRequest {
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReworkRequest {
    pub to_step: i32,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitRequest {
    pub children: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeRequest {
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScrapRequest {
    pub wafer_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplayRequest {
    #[serde(default = "default_true")]
    pub truncate: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResponse {
    pub events: usize,
    pub last_seq: i64,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigResponse {
    pub scenario: oxide_core::Scenario,
    pub equipment: Vec<String>,
    pub lots: usize,
    pub wafers_per_lot: i32,
}
