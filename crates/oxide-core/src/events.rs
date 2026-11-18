//! The event vocabulary.
//!
//! Every state change in oxide-fab is an immutable `DomainEvent`. PostgreSQL's
//! `lot_event` table is the single write entry point: append the event, project
//! it onto `lot` / `wafer` / `equipment`, and queue it in `outbox` — all inside
//! one transaction. The time series store is fed only from these events, which
//! is what makes it reconstructible.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::model::{LotId, WaferId};
use crate::string_enum::string_enum;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    LotCreated,
    LotQueued,
    LotSplit,
    LotMerged,
    LotHeld,
    LotReleased,
    TrackIn,
    TrackOut,
    TrackOutDuplicate,
    ReworkStarted,
    WaferScrapped,
    LotCompleted,
    EquipmentDown,
    EquipmentUp,
    RecipeVerified,
}

string_enum!(EventKind {
    LotCreated => "lot_created",
    LotQueued => "lot_queued",
    LotSplit => "lot_split",
    LotMerged => "lot_merged",
    LotHeld => "lot_held",
    LotReleased => "lot_released",
    TrackIn => "track_in",
    TrackOut => "track_out",
    TrackOutDuplicate => "track_out_duplicate",
    ReworkStarted => "rework_started",
    WaferScrapped => "wafer_scrapped",
    LotCompleted => "lot_completed",
    EquipmentDown => "equipment_down",
    EquipmentUp => "equipment_up",
    RecipeVerified => "recipe_verified",
});

/// One immutable domain event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DomainEvent {
    pub event_id: Uuid,
    pub lot_id: LotId,
    pub wafer_id: Option<WaferId>,
    pub kind: EventKind,
    pub payload: Value,
    pub occurred_at_ms: i64,
    pub actor: String,
}

impl DomainEvent {
    pub fn new(
        event_id: Uuid,
        lot_id: impl Into<LotId>,
        kind: EventKind,
        payload: Value,
        occurred_at_ms: i64,
        actor: impl Into<String>,
    ) -> Self {
        Self {
            event_id,
            lot_id: lot_id.into(),
            wafer_id: None,
            kind,
            payload,
            occurred_at_ms,
            actor: actor.into(),
        }
    }

    #[must_use]
    pub fn with_wafer(mut self, wafer_id: impl Into<WaferId>) -> Self {
        self.wafer_id = Some(wafer_id.into());
        self
    }

    pub fn wafers(&self) -> Vec<WaferId> {
        self.payload
            .get("wafers")
            .and_then(Value::as_array)
            .map_or_default(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
    }

    pub fn step_no(&self) -> Option<i32> {
        self.payload
            .get("step_no")
            .and_then(Value::as_i64)
            .map(|v| v as i32)
    }

    #[must_use]
    pub fn equipment_id(&self) -> Option<&str> {
        self.payload.get("equipment_id").and_then(Value::as_str)
    }

    /// The process program the step ran.
    #[must_use]
    pub fn recipe_id(&self) -> Option<&str> {
        self.payload.get("recipe_id").and_then(Value::as_str)
    }
}

// Payload constructors keep the wire shape in one place: the projection and
// the metric derivation both read these keys.

#[must_use]
pub fn lot_created_payload(
    product_id: &str,
    route_id: &str,
    priority: i32,
    wafers: &[WaferId],
) -> Value {
    json!({
        "product_id": product_id,
        "route_id": route_id,
        "priority": priority,
        "wafer_count": wafers.len(),
        "wafers": wafers,
    })
}

#[must_use]
pub fn lot_queued_payload(step_no: i32) -> Value {
    json!({ "step_no": step_no })
}

/// What a tool reports when it loads a lot on a step.
///
/// One value instead of four positional parameters: the payload constructor
/// and the state machine take the same struct, so the wire shape cannot drift
/// from what the transition validated.
#[derive(Debug, Clone, Copy)]
pub struct TrackInReport<'a> {
    pub step_no: i32,
    pub equipment_id: &'a str,
    pub recipe_id: &'a str,
    pub wafers: &'a [WaferId],
}

/// What a tool reports when a step finishes.
#[derive(Debug, Clone, Copy)]
pub struct TrackOutReport<'a> {
    pub step_no: i32,
    pub equipment_id: &'a str,
    pub recipe_id: &'a str,
    /// The checksum the tool verified before running the program.
    pub recipe_checksum: &'a str,
    pub wafers: &'a [WaferId],
    pub duration_ms: i64,
    /// The measured value, on metrology steps.
    pub measured: Option<f64>,
}

#[must_use]
pub fn track_in_payload(report: &TrackInReport<'_>) -> Value {
    json!({
        "step_no": report.step_no,
        "equipment_id": report.equipment_id,
        "recipe_id": report.recipe_id,
        "wafers": report.wafers,
    })
}

#[must_use]
pub fn track_out_payload(report: &TrackOutReport<'_>) -> Value {
    json!({
        "step_no": report.step_no,
        "equipment_id": report.equipment_id,
        "recipe_id": report.recipe_id,
        "recipe_checksum": report.recipe_checksum,
        "wafers": report.wafers,
        "duration_ms": report.duration_ms,
        "measured": report.measured,
    })
}

#[must_use]
pub fn equipment_down_payload(equipment_id: &str, kind: &str, reason: &str, alid: u32) -> Value {
    json!({
        "equipment_id": equipment_id,
        "kind": kind,
        "reason": reason,
        "alid": alid,
    })
}

#[must_use]
pub fn equipment_up_payload(equipment_id: &str, kind: &str, down_ms: i64) -> Value {
    json!({
        "equipment_id": equipment_id,
        "kind": kind,
        "down_ms": down_ms,
    })
}

#[must_use]
pub fn split_payload(parent: &str, children: &[(String, Vec<WaferId>)], step_no: i32) -> Value {
    let children: Vec<Value> = children
        .iter()
        .map(|(lot_id, wafers)| json!({ "lot_id": lot_id, "wafers": wafers }))
        .collect();
    json!({ "parent_lot_id": parent, "children": children, "step_no": step_no })
}

#[must_use]
pub fn merged_payload(sources: &[String], target: &str, step_no: i32, wafers: &[WaferId]) -> Value {
    json!({
        "sources": sources,
        "target_lot_id": target,
        "step_no": step_no,
        "wafers": wafers,
    })
}
