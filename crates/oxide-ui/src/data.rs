//! Wire types and the incremental update rules of the dashboard.
//!
//! The UI never recomputes the floor from scratch while it polls: the initial
//! snapshot seeds the state, and each `/api/events?since=` page is folded into
//! the existing lots and tools. Only the aggregate panels (queue lengths, SPC,
//! backups) are refreshed wholesale, and only every few seconds.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Snapshot {
    #[serde(default)]
    pub sim: SimInfo,
    #[serde(default)]
    pub db: DbCounts,
    #[serde(default)]
    pub outbox: OutboxStats,
    #[serde(default)]
    pub last_seq: i64,
    #[serde(default)]
    pub greptime_ok: bool,
    #[serde(default)]
    pub secs: Option<SecsStatus>,
    #[serde(default)]
    pub backup: BackupStatus,
    #[serde(default)]
    pub restore: Option<RestoreStatus>,
    #[serde(default)]
    pub spc: Vec<SpcBucket>,
    #[serde(default)]
    pub spc_source: String,
    #[serde(default)]
    pub chaos: Vec<ChaosSample>,
}

/// The floor and the engine's clock, as `/api/snapshot` reports them under
/// `sim`. The API projects steps, tools and lots exactly once, here.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SimInfo {
    #[serde(default)]
    pub now_ms: i64,
    #[serde(default)]
    pub running: bool,
    #[serde(default)]
    pub speed: f64,
    #[serde(default)]
    pub released_lots: usize,
    #[serde(default)]
    pub total_lots: usize,
    #[serde(default)]
    pub persisted_events: u64,
    #[serde(default)]
    pub steps: Vec<StepView>,
    #[serde(default)]
    pub equipment: Vec<ToolView>,
    #[serde(default)]
    pub lots: Vec<LotView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StepView {
    #[serde(default)]
    pub step_no: i32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub equipment_kind: String,
    #[serde(default)]
    pub queue_len: usize,
    #[serde(default)]
    pub busy_tools: usize,
    #[serde(default)]
    pub total_tools: usize,
    #[serde(default)]
    pub current_lots: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ToolView {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub chamber: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub light: String,
    #[serde(default)]
    pub current_lot: Option<String>,
    #[serde(default)]
    pub current_step: Option<i32>,
    #[serde(default)]
    pub lots_processed: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WaferView {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub slot: i32,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub defect_info: Option<String>,
    #[serde(default)]
    pub rework_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LotView {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub product_id: String,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub current_step: i32,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub wafer_count: usize,
    #[serde(default)]
    pub done_wafers: usize,
    #[serde(default)]
    pub hold_reason: Option<String>,
    #[serde(default)]
    pub parent_lot_id: Option<String>,
    #[serde(default)]
    pub wafers: Vec<WaferView>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DbCounts {
    #[serde(default)]
    pub lots: i64,
    #[serde(default)]
    pub wafers: i64,
    #[serde(default)]
    pub events: i64,
    #[serde(default)]
    pub outbox_pending: i64,
    #[serde(default)]
    pub outbox_published: i64,
    #[serde(default)]
    pub published_events: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct OutboxStats {
    #[serde(default)]
    pub pending: i64,
    #[serde(default)]
    pub published: i64,
    #[serde(default)]
    pub oldest_pending_ms: i64,
    #[serde(default)]
    pub max_attempts: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpcBucket {
    pub ts: String,
    pub equipment_id: String,
    pub param: String,
    pub avg_value: f64,
    pub min_value: f64,
    pub max_value: f64,
    pub ucl: f64,
    pub lcl: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ChaosSample {
    #[serde(default)]
    pub at_ms: i64,
    #[serde(default)]
    pub pending: i64,
    #[serde(default)]
    pub published: i64,
    #[serde(default)]
    pub oldest_pending_ms: i64,
    #[serde(default)]
    pub delivered_rows: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct TrafficEntry {
    #[serde(default)]
    pub at_ms: i64,
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SecsStatus {
    #[serde(default)]
    pub host_connected: bool,
    #[serde(default)]
    pub equipment_online: bool,
    #[serde(default)]
    pub s1f1_ok: u64,
    #[serde(default)]
    pub s6f11_sent: u64,
    #[serde(default)]
    pub s5f1_sent: u64,
    #[serde(default)]
    pub s7f3_uploaded: u64,
    #[serde(default)]
    pub s7f4_ack: u64,
    #[serde(default)]
    pub s7f6_verified: u64,
    #[serde(default)]
    pub spool_depth: u64,
    #[serde(default)]
    pub spooled_total: u64,
    #[serde(default)]
    pub spooled_replayed: u64,
    #[serde(default)]
    pub link_drops: u64,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub traffic: Vec<TrafficEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct BackupStatus {
    #[serde(default)]
    pub taken_at: Option<String>,
    #[serde(default)]
    pub pg_full_backup: Option<String>,
    #[serde(default)]
    pub pg_logical_dump: Option<String>,
    #[serde(default)]
    pub greptime_snapshot: Option<String>,
    #[serde(default)]
    pub wal_continuity: Option<String>,
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub log_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct RestoreStatus {
    #[serde(default)]
    pub drilled_at: Option<String>,
    #[serde(default)]
    pub target_time: Option<String>,
    #[serde(default)]
    pub lots: Option<i64>,
    #[serde(default)]
    pub events: Option<i64>,
    #[serde(default)]
    pub max_created_at: Option<String>,
    #[serde(default)]
    pub greptime_rows: Option<i64>,
    #[serde(default)]
    pub elapsed_ms: Option<i64>,
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub log_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventsResponse {
    #[serde(default)]
    pub events: Vec<PersistedEvent>,
    #[serde(default)]
    pub last_seq: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedEvent {
    #[serde(default)]
    pub seq: i64,
    pub event: Event,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Event {
    #[serde(default)]
    pub event_id: String,
    #[serde(default)]
    pub lot_id: String,
    #[serde(default)]
    pub wafer_id: Option<String>,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub payload: Value,
    #[serde(default)]
    pub occurred_at_ms: i64,
    #[serde(default)]
    pub actor: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct LotDetail {
    #[serde(default)]
    pub lot: LotView,
    #[serde(default)]
    pub wafers: Vec<WaferView>,
    #[serde(default)]
    pub genealogy: Vec<WaferGenealogy>,
    #[serde(default)]
    pub events: Vec<PersistedEvent>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct WaferGenealogy {
    #[serde(default)]
    pub wafer_id: String,
    #[serde(default)]
    pub lot_id: String,
    #[serde(default)]
    pub slot: i32,
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub defect_info: Option<String>,
    #[serde(default)]
    pub rework_count: u32,
    #[serde(default)]
    pub steps: Vec<StepRecord>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StepRecord {
    #[serde(default)]
    pub step_no: i32,
    #[serde(default)]
    pub equipment_id: String,
    #[serde(default)]
    pub recipe_id: String,
    #[serde(default)]
    pub track_in_ms: i64,
    #[serde(default)]
    pub track_out_ms: i64,
    #[serde(default)]
    pub duration_ms: i64,
    #[serde(default)]
    pub measured: Option<f64>,
}

/// Fold one event into the lots and tools. Returns `false` when the event
/// mentions something this view does not know, which means a full snapshot is
/// needed.
///
/// One arm per event kind, kept together so the vocabulary and the fold can be
/// reviewed side by side.
#[expect(
    clippy::too_many_lines,
    reason = "one arm per event kind; splitting scatters the fold"
)]
pub fn apply_event(lots: &mut [LotView], tools: &mut [ToolView], event: &Event) -> bool {
    let step_no = event
        .payload
        .get("step_no")
        .and_then(Value::as_i64)
        .map(|v| v as i32);
    let equipment_id = event
        .payload
        .get("equipment_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
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

    match event.kind.as_str() {
        "lot_created" => return false,
        "track_in"
        | "track_out"
        | "lot_queued"
        | "lot_completed"
        | "lot_held"
        | "lot_released"
        | "rework_started"
        | "wafer_scrapped"
        | "track_out_duplicate" => {
            let Some(lot) = lots.iter_mut().find(|lot| lot.id == event.lot_id) else {
                return false;
            };
            match event.kind.as_str() {
                "track_in" => {
                    lot.state = "processing".to_owned();
                    if let Some(step) = step_no {
                        lot.current_step = step;
                    }
                    for wafer in &mut lot.wafers {
                        if wafers.contains(&wafer.id) {
                            wafer.state = "processing".to_owned();
                        }
                    }
                    if let Some(id) = &equipment_id
                        && let Some(tool) = tools.iter_mut().find(|tool| &tool.id == id)
                    {
                        tool.state = "running".to_owned();
                        tool.light = "green".to_owned();
                        tool.current_lot = Some(lot.id.clone());
                        tool.current_step = step_no;
                    }
                }
                "track_out" => {
                    for wafer in &mut lot.wafers {
                        if wafers.contains(&wafer.id) {
                            wafer.state = "waiting".to_owned();
                        }
                    }
                    lot.state = "waiting".to_owned();
                    if let Some(id) = &equipment_id
                        && let Some(tool) = tools.iter_mut().find(|tool| &tool.id == id)
                    {
                        tool.state = "idle".to_owned();
                        tool.light = "green".to_owned();
                        tool.current_lot = None;
                        tool.current_step = None;
                        tool.lots_processed += 1;
                    }
                }
                "lot_queued" => {
                    lot.state = "queued".to_owned();
                    if let Some(step) = step_no {
                        lot.current_step = step;
                    }
                }
                "lot_completed" => {
                    lot.state = "completed".to_owned();
                    lot.done_wafers = lot.wafer_count;
                    for wafer in &mut lot.wafers {
                        if wafer.state != "scrapped" {
                            wafer.state = "done".to_owned();
                        }
                    }
                }
                "lot_held" => {
                    lot.state = "on_hold".to_owned();
                    lot.hold_reason = event
                        .payload
                        .get("reason")
                        .and_then(Value::as_str)
                        .map(str::to_owned);
                }
                "lot_released" => {
                    lot.state = "waiting".to_owned();
                    lot.hold_reason = None;
                }
                "rework_started" => {
                    lot.state = "waiting".to_owned();
                    if let Some(step) = event.payload.get("to_step").and_then(Value::as_i64) {
                        lot.current_step = step as i32;
                    }
                    for wafer in &mut lot.wafers {
                        if wafer.state != "scrapped" {
                            wafer.state = "waiting".to_owned();
                            wafer.rework_count += 1;
                        }
                    }
                }
                "wafer_scrapped" => {
                    if let Some(wafer_id) = event.payload.get("wafer_id").and_then(Value::as_str)
                        && let Some(wafer) = lot.wafers.iter_mut().find(|w| w.id == wafer_id)
                    {
                        wafer.state = "scrapped".to_owned();
                        wafer.defect_info = event
                            .payload
                            .get("reason")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                    }
                }
                _ => {}
            }
        }
        "equipment_down" | "equipment_up" => {
            let Some(id) = equipment_id else { return false };
            let Some(tool) = tools.iter_mut().find(|tool| tool.id == id) else {
                return false;
            };
            if event.kind == "equipment_down" {
                tool.state = "down".to_owned();
                tool.light = "red".to_owned();
            } else {
                tool.state = "idle".to_owned();
                tool.light = "green".to_owned();
            }
        }
        _ => {}
    }
    true
}

/// The demo clock: simulated milliseconds rendered as `mm:ss` since the line
/// started (the simulator starts at wall time, so show the time of day).
#[must_use]
pub fn clock_text(now_ms: i64) -> String {
    let total_seconds = now_ms.div_euclid(1000).rem_euclid(86_400);
    let hours = total_seconds / 3600;
    let minutes = (total_seconds % 3600) / 60;
    let seconds = total_seconds % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02} UTC")
}

/// `mm:ss` for a duration in milliseconds.
#[must_use]
pub fn duration_text(ms: i64) -> String {
    let total = (ms.max(0) as f64 / 1000.0).round() as i64;
    format!("{:02}:{:02}", total / 60, total % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn lot() -> LotView {
        LotView {
            id: "LOT-0001".to_owned(),
            state: "queued".to_owned(),
            current_step: 1,
            wafer_count: 2,
            wafers: vec![
                WaferView {
                    id: "LOT-0001-W01".to_owned(),
                    slot: 1,
                    state: "queued".to_owned(),
                    ..Default::default()
                },
                WaferView {
                    id: "LOT-0001-W02".to_owned(),
                    slot: 2,
                    state: "queued".to_owned(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    fn tool() -> ToolView {
        ToolView {
            id: "CLN-01".to_owned(),
            state: "idle".to_owned(),
            light: "green".to_owned(),
            ..Default::default()
        }
    }

    fn event(kind: &str, payload: Value) -> Event {
        Event {
            lot_id: "LOT-0001".to_owned(),
            kind: kind.to_owned(),
            payload,
            ..Default::default()
        }
    }

    #[test]
    fn a_track_in_turns_one_tool_light_and_the_lot_processing() {
        let mut lots = vec![lot()];
        let mut tools = vec![tool()];
        let ok = apply_event(
            &mut lots,
            &mut tools,
            &event(
                "track_in",
                json!({"step_no": 1, "equipment_id": "CLN-01", "wafers": ["LOT-0001-W01", "LOT-0001-W02"]}),
            ),
        );
        assert!(ok);
        assert_eq!(lots[0].state, "processing");
        assert_eq!(tools[0].state, "running");
        assert_eq!(tools[0].current_lot.as_deref(), Some("LOT-0001"));
    }

    #[test]
    fn a_track_out_frees_the_tool_and_only_touches_its_wafers() {
        let mut lots = vec![lot()];
        let mut tools = vec![tool()];
        apply_event(
            &mut lots,
            &mut tools,
            &event(
                "track_in",
                json!({"step_no": 1, "equipment_id": "CLN-01", "wafers": ["LOT-0001-W01"]}),
            ),
        );
        apply_event(
            &mut lots,
            &mut tools,
            &event(
                "track_out",
                json!({"step_no": 1, "equipment_id": "CLN-01", "wafers": ["LOT-0001-W01"], "duration_ms": 6000}),
            ),
        );
        assert_eq!(tools[0].state, "idle");
        assert_eq!(tools[0].lots_processed, 1);
        assert_eq!(lots[0].wafers[0].state, "waiting");
        assert_eq!(lots[0].wafers[1].state, "queued");
    }

    #[test]
    fn completion_marks_every_live_wafer_done() {
        let mut lots = vec![lot()];
        let mut tools = vec![tool()];
        apply_event(
            &mut lots,
            &mut tools,
            &event("lot_completed", json!({"step_no": 4})),
        );
        assert_eq!(lots[0].state, "completed");
        assert_eq!(lots[0].done_wafers, 2);
        assert!(lots[0].wafers.iter().all(|w| w.state == "done"));
    }

    #[test]
    fn an_unknown_lot_asks_for_a_full_snapshot() {
        let mut lots = vec![];
        let mut tools = vec![tool()];
        let ok = apply_event(&mut lots, &mut tools, &event("lot_completed", json!({})));
        assert!(!ok);
    }

    #[test]
    fn the_clock_is_utc_time_of_day() {
        assert_eq!(clock_text(0), "00:00:00 UTC");
        assert_eq!(clock_text(3_723_000), "01:02:03 UTC");
    }
}
