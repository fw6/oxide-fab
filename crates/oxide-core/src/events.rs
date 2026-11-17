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

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::LotCreated => "lot_created",
            EventKind::LotQueued => "lot_queued",
            EventKind::LotSplit => "lot_split",
            EventKind::LotMerged => "lot_merged",
            EventKind::LotHeld => "lot_held",
            EventKind::LotReleased => "lot_released",
            EventKind::TrackIn => "track_in",
            EventKind::TrackOut => "track_out",
            EventKind::TrackOutDuplicate => "track_out_duplicate",
            EventKind::ReworkStarted => "rework_started",
            EventKind::WaferScrapped => "wafer_scrapped",
            EventKind::LotCompleted => "lot_completed",
            EventKind::EquipmentDown => "equipment_down",
            EventKind::EquipmentUp => "equipment_up",
            EventKind::RecipeVerified => "recipe_verified",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "lot_created" => EventKind::LotCreated,
            "lot_queued" => EventKind::LotQueued,
            "lot_split" => EventKind::LotSplit,
            "lot_merged" => EventKind::LotMerged,
            "lot_held" => EventKind::LotHeld,
            "lot_released" => EventKind::LotReleased,
            "track_in" => EventKind::TrackIn,
            "track_out" => EventKind::TrackOut,
            "track_out_duplicate" => EventKind::TrackOutDuplicate,
            "rework_started" => EventKind::ReworkStarted,
            "wafer_scrapped" => EventKind::WaferScrapped,
            "lot_completed" => EventKind::LotCompleted,
            "equipment_down" => EventKind::EquipmentDown,
            "equipment_up" => EventKind::EquipmentUp,
            "recipe_verified" => EventKind::RecipeVerified,
            _ => return None,
        })
    }
}

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

    pub fn with_wafer(mut self, wafer_id: impl Into<WaferId>) -> Self {
        self.wafer_id = Some(wafer_id.into());
        self
    }

    pub fn wafers(&self) -> Vec<WaferId> {
        self.payload
            .get("wafers")
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn step_no(&self) -> Option<i32> {
        self.payload
            .get("step_no")
            .and_then(Value::as_i64)
            .map(|v| v as i32)
    }

    pub fn equipment_id(&self) -> Option<&str> {
        self.payload.get("equipment_id").and_then(Value::as_str)
    }
}

// Payload constructors keep the wire shape in one place: the projection and
// the metric derivation both read these keys.

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

pub fn lot_queued_payload(step_no: i32) -> Value {
    json!({ "step_no": step_no })
}

pub fn track_in_payload(
    step_no: i32,
    equipment_id: &str,
    recipe_id: &str,
    wafers: &[WaferId],
) -> Value {
    json!({
        "step_no": step_no,
        "equipment_id": equipment_id,
        "recipe_id": recipe_id,
        "wafers": wafers,
    })
}

pub fn track_out_payload(
    step_no: i32,
    equipment_id: &str,
    recipe_id: &str,
    recipe_checksum: &str,
    wafers: &[WaferId],
    duration_ms: i64,
    measured: Option<f64>,
) -> Value {
    json!({
        "step_no": step_no,
        "equipment_id": equipment_id,
        "recipe_id": recipe_id,
        "recipe_checksum": recipe_checksum,
        "wafers": wafers,
        "duration_ms": duration_ms,
        "measured": measured,
    })
}

pub fn equipment_down_payload(equipment_id: &str, kind: &str, reason: &str, alid: u32) -> Value {
    json!({
        "equipment_id": equipment_id,
        "kind": kind,
        "reason": reason,
        "alid": alid,
    })
}

pub fn equipment_up_payload(equipment_id: &str, kind: &str, down_ms: i64) -> Value {
    json!({
        "equipment_id": equipment_id,
        "kind": kind,
        "down_ms": down_ms,
    })
}

pub fn split_payload(parent: &str, children: &[(String, Vec<WaferId>)], step_no: i32) -> Value {
    let children: Vec<Value> = children
        .iter()
        .map(|(lot_id, wafers)| json!({ "lot_id": lot_id, "wafers": wafers }))
        .collect();
    json!({ "parent_lot_id": parent, "children": children, "step_no": step_no })
}

pub fn merged_payload(sources: &[String], target: &str, step_no: i32, wafers: &[WaferId]) -> Value {
    json!({
        "sources": sources,
        "target_lot_id": target,
        "step_no": step_no,
        "wafers": wafers,
    })
}
