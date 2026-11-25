//! The event vocabulary.
//!
//! Every state change in oxide-fab is an immutable `DomainEvent`. PostgreSQL's
//! `lot_event` table is the single write entry point: append the event, project
//! it onto `lot` / `wafer` / `equipment`, and queue it in `outbox` — all inside
//! one transaction. The time series store is fed only from these events, which
//! is what makes it reconstructible.
//!
//! The payload is typed: `EventData` has one variant per `EventKind`, so a
//! writer cannot pair a `track_out` with a `lot_created` payload.

use std::fmt;

use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
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

/// The payload of one event, tagged by kind on the wire.
///
/// Serialises as `{"kind": "...", "payload": {...}}`, which is the shape the
/// `lot_event` column, the API and the TSX fold already use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum EventData {
    LotCreated {
        product_id: String,
        route_id: String,
        priority: i32,
        wafer_count: i32,
        wafers: Vec<WaferId>,
    },
    LotQueued {
        step_no: i32,
    },
    LotSplit {
        parent_lot_id: LotId,
        children: Vec<SplitChild>,
        step_no: i32,
    },
    LotMerged {
        sources: Vec<LotId>,
        target_lot_id: LotId,
        step_no: i32,
        wafers: Vec<WaferId>,
    },
    LotHeld {
        reason: String,
        step_no: i32,
    },
    LotReleased {
        step_no: i32,
        #[serde(default)]
        reason: Option<String>,
    },
    TrackIn {
        step_no: i32,
        equipment_id: String,
        recipe_id: String,
        wafers: Vec<WaferId>,
    },
    TrackOut {
        step_no: i32,
        equipment_id: String,
        recipe_id: String,
        recipe_checksum: String,
        wafers: Vec<WaferId>,
        duration_ms: i64,
        #[serde(default)]
        measured: Option<f64>,
    },
    TrackOutDuplicate {
        step_no: i32,
        equipment_id: String,
        wafers: Vec<WaferId>,
        idempotent: bool,
    },
    ReworkStarted {
        from_step: i32,
        to_step: i32,
        reason: String,
    },
    WaferScrapped {
        wafer_id: WaferId,
        reason: String,
        step_no: i32,
    },
    LotCompleted {
        step_no: i32,
    },
    EquipmentDown {
        equipment_id: String,
        kind: String,
        reason: String,
        alid: u32,
    },
    EquipmentUp {
        equipment_id: String,
        kind: String,
        down_ms: i64,
    },
    RecipeVerified {
        recipe_id: String,
        checksum: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SplitChild {
    pub lot_id: LotId,
    pub wafers: Vec<WaferId>,
}

#[derive(Debug)]
pub struct PayloadError {
    kind: EventKind,
    source: serde_json::Error,
}

impl fmt::Display for PayloadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} payload does not match its kind: {}",
            self.kind.as_str(),
            self.source
        )
    }
}

impl std::error::Error for PayloadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

impl EventData {
    /// Parse the tagged `{kind, payload}` object a row or a request carries.
    ///
    /// # Errors
    /// [`PayloadError`] when a field is missing or has the wrong JSON type.
    pub fn from_parts(kind: EventKind, payload: &Value) -> Result<Self, PayloadError> {
        let tagged = json!({ "kind": kind.as_str(), "payload": payload });
        serde_json::from_value(tagged).map_err(|source| PayloadError { kind, source })
    }

    #[must_use]
    pub fn kind(&self) -> EventKind {
        match self {
            EventData::LotCreated { .. } => EventKind::LotCreated,
            EventData::LotQueued { .. } => EventKind::LotQueued,
            EventData::LotSplit { .. } => EventKind::LotSplit,
            EventData::LotMerged { .. } => EventKind::LotMerged,
            EventData::LotHeld { .. } => EventKind::LotHeld,
            EventData::LotReleased { .. } => EventKind::LotReleased,
            EventData::TrackIn { .. } => EventKind::TrackIn,
            EventData::TrackOut { .. } => EventKind::TrackOut,
            EventData::TrackOutDuplicate { .. } => EventKind::TrackOutDuplicate,
            EventData::ReworkStarted { .. } => EventKind::ReworkStarted,
            EventData::WaferScrapped { .. } => EventKind::WaferScrapped,
            EventData::LotCompleted { .. } => EventKind::LotCompleted,
            EventData::EquipmentDown { .. } => EventKind::EquipmentDown,
            EventData::EquipmentUp { .. } => EventKind::EquipmentUp,
            EventData::RecipeVerified { .. } => EventKind::RecipeVerified,
        }
    }

    #[must_use]
    pub fn step_no(&self) -> Option<i32> {
        match self {
            EventData::LotQueued { step_no }
            | EventData::LotSplit { step_no, .. }
            | EventData::LotMerged { step_no, .. }
            | EventData::LotHeld { step_no, .. }
            | EventData::LotReleased { step_no, .. }
            | EventData::TrackIn { step_no, .. }
            | EventData::TrackOut { step_no, .. }
            | EventData::TrackOutDuplicate { step_no, .. }
            | EventData::WaferScrapped { step_no, .. }
            | EventData::LotCompleted { step_no } => Some(*step_no),
            _ => None,
        }
    }

    #[must_use]
    pub fn equipment_id(&self) -> Option<&str> {
        match self {
            EventData::TrackIn { equipment_id, .. }
            | EventData::TrackOut { equipment_id, .. }
            | EventData::TrackOutDuplicate { equipment_id, .. }
            | EventData::EquipmentDown { equipment_id, .. }
            | EventData::EquipmentUp { equipment_id, .. } => Some(equipment_id),
            _ => None,
        }
    }

    #[must_use]
    pub fn recipe_id(&self) -> Option<&str> {
        match self {
            EventData::TrackIn { recipe_id, .. }
            | EventData::TrackOut { recipe_id, .. }
            | EventData::RecipeVerified { recipe_id, .. } => Some(recipe_id),
            _ => None,
        }
    }

    #[must_use]
    pub fn wafers(&self) -> &[WaferId] {
        match self {
            EventData::LotCreated { wafers, .. }
            | EventData::TrackIn { wafers, .. }
            | EventData::TrackOut { wafers, .. }
            | EventData::TrackOutDuplicate { wafers, .. }
            | EventData::LotMerged { wafers, .. } => wafers,
            _ => &[],
        }
    }

    #[must_use]
    pub fn children(&self) -> &[SplitChild] {
        match self {
            EventData::LotSplit { children, .. } => children,
            _ => &[],
        }
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        match self {
            EventData::LotHeld { reason, .. }
            | EventData::WaferScrapped { reason, .. }
            | EventData::EquipmentDown { reason, .. } => Some(reason),
            _ => None,
        }
    }

    #[must_use]
    pub fn alid(&self) -> Option<u32> {
        match self {
            EventData::EquipmentDown { alid, .. } => Some(*alid),
            _ => None,
        }
    }

    #[must_use]
    pub fn down_ms(&self) -> Option<i64> {
        match self {
            EventData::EquipmentUp { down_ms, .. } => Some(*down_ms),
            _ => None,
        }
    }

    #[must_use]
    pub fn duration_ms(&self) -> Option<i64> {
        match self {
            EventData::TrackOut { duration_ms, .. } => Some(*duration_ms),
            _ => None,
        }
    }

    #[must_use]
    pub fn measured(&self) -> Option<f64> {
        match self {
            EventData::TrackOut { measured, .. } => *measured,
            _ => None,
        }
    }

    #[must_use]
    pub fn recipe_checksum(&self) -> Option<&str> {
        match self {
            EventData::TrackOut {
                recipe_checksum, ..
            } => Some(recipe_checksum),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_step(&self) -> Option<i32> {
        match self {
            EventData::ReworkStarted { to_step, .. } => Some(*to_step),
            _ => None,
        }
    }

    #[must_use]
    pub fn target_lot_id(&self) -> Option<&str> {
        match self {
            EventData::LotMerged { target_lot_id, .. } => Some(target_lot_id),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct DomainEvent {
    pub event_id: Uuid,
    pub lot_id: LotId,
    pub wafer_id: Option<WaferId>,
    pub data: EventData,
    pub occurred_at_ms: i64,
    pub actor: String,
}

impl DomainEvent {
    pub fn new(
        event_id: Uuid,
        lot_id: impl Into<LotId>,
        data: EventData,
        occurred_at_ms: i64,
        actor: impl Into<String>,
    ) -> Self {
        Self {
            event_id,
            lot_id: lot_id.into(),
            wafer_id: None,
            data,
            occurred_at_ms,
            actor: actor.into(),
        }
    }

    /// Rebuild an event from its columns.
    ///
    /// # Errors
    /// [`PayloadError`] when the kind and the payload disagree, or a required
    /// key is missing. A row written by a newer schema must fail here rather
    /// than be projected as a default.
    pub fn from_parts(
        event_id: Uuid,
        lot_id: LotId,
        wafer_id: Option<WaferId>,
        kind: EventKind,
        payload: &Value,
        occurred_at_ms: i64,
        actor: String,
    ) -> Result<Self, PayloadError> {
        Ok(Self {
            event_id,
            lot_id,
            wafer_id,
            data: EventData::from_parts(kind, payload)?,
            occurred_at_ms,
            actor,
        })
    }

    #[must_use]
    pub fn with_wafer(mut self, wafer_id: impl Into<WaferId>) -> Self {
        self.wafer_id = Some(wafer_id.into());
        self
    }

    #[must_use]
    pub fn kind(&self) -> EventKind {
        self.data.kind()
    }

    /// The payload as it is stored: the inner object, without the kind tag.
    #[must_use]
    pub fn payload_json(&self) -> Value {
        serde_json::to_value(&self.data)
            .ok()
            .and_then(|mut tagged| tagged.get_mut("payload").map(Value::take))
            .unwrap_or(Value::Null)
    }

    #[must_use]
    pub fn wafers(&self) -> &[WaferId] {
        self.data.wafers()
    }

    #[must_use]
    pub fn step_no(&self) -> Option<i32> {
        self.data.step_no()
    }

    #[must_use]
    pub fn equipment_id(&self) -> Option<&str> {
        self.data.equipment_id()
    }

    #[must_use]
    pub fn recipe_id(&self) -> Option<&str> {
        self.data.recipe_id()
    }

    #[must_use]
    pub fn children(&self) -> &[SplitChild] {
        self.data.children()
    }

    #[must_use]
    pub fn reason(&self) -> Option<&str> {
        self.data.reason()
    }

    #[must_use]
    pub fn alid(&self) -> Option<u32> {
        self.data.alid()
    }

    #[must_use]
    pub fn down_ms(&self) -> Option<i64> {
        self.data.down_ms()
    }

    #[must_use]
    pub fn duration_ms(&self) -> Option<i64> {
        self.data.duration_ms()
    }

    #[must_use]
    pub fn measured(&self) -> Option<f64> {
        self.data.measured()
    }

    #[must_use]
    pub fn recipe_checksum(&self) -> Option<&str> {
        self.data.recipe_checksum()
    }

    #[must_use]
    pub fn to_step(&self) -> Option<i32> {
        self.data.to_step()
    }

    #[must_use]
    pub fn target_lot_id(&self) -> Option<&str> {
        self.data.target_lot_id()
    }
}

impl Serialize for DomainEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let tagged = serde_json::to_value(&self.data).map_err(serde::ser::Error::custom)?;
        let Some(tagged) = tagged.as_object() else {
            return Err(serde::ser::Error::custom("event data is not an object"));
        };
        let mut map = serializer.serialize_map(Some(4 + tagged.len()))?;
        map.serialize_entry("event_id", &self.event_id)?;
        map.serialize_entry("lot_id", &self.lot_id)?;
        map.serialize_entry("wafer_id", &self.wafer_id)?;
        for (key, value) in tagged {
            map.serialize_entry(key, value)?;
        }
        map.serialize_entry("occurred_at_ms", &self.occurred_at_ms)?;
        map.serialize_entry("actor", &self.actor)?;
        map.end()
    }
}

#[derive(Deserialize)]
struct WireEvent {
    event_id: Uuid,
    lot_id: LotId,
    #[serde(default)]
    wafer_id: Option<WaferId>,
    kind: EventKind,
    payload: Value,
    occurred_at_ms: i64,
    #[serde(default)]
    actor: String,
}

impl<'de> Deserialize<'de> for DomainEvent {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = WireEvent::deserialize(deserializer)?;
        let data =
            EventData::from_parts(wire.kind, &wire.payload).map_err(serde::de::Error::custom)?;
        Ok(Self {
            event_id: wire.event_id,
            lot_id: wire.lot_id,
            wafer_id: wire.wafer_id,
            data,
            occurred_at_ms: wire.occurred_at_ms,
            actor: wire.actor,
        })
    }
}

#[must_use]
pub fn lot_created_payload(
    product_id: &str,
    route_id: &str,
    priority: i32,
    wafers: &[WaferId],
) -> EventData {
    EventData::LotCreated {
        product_id: product_id.to_owned(),
        route_id: route_id.to_owned(),
        priority,
        wafer_count: wafers.len() as i32,
        wafers: wafers.to_vec(),
    }
}

#[must_use]
pub fn lot_queued_payload(step_no: i32) -> EventData {
    EventData::LotQueued { step_no }
}

/// What a tool reports when it loads a lot on a step.
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
pub fn track_in_payload(report: &TrackInReport<'_>) -> EventData {
    EventData::TrackIn {
        step_no: report.step_no,
        equipment_id: report.equipment_id.to_owned(),
        recipe_id: report.recipe_id.to_owned(),
        wafers: report.wafers.to_vec(),
    }
}

#[must_use]
pub fn track_out_payload(report: &TrackOutReport<'_>) -> EventData {
    EventData::TrackOut {
        step_no: report.step_no,
        equipment_id: report.equipment_id.to_owned(),
        recipe_id: report.recipe_id.to_owned(),
        recipe_checksum: report.recipe_checksum.to_owned(),
        wafers: report.wafers.to_vec(),
        duration_ms: report.duration_ms,
        measured: report.measured,
    }
}

#[must_use]
pub fn equipment_down_payload(
    equipment_id: &str,
    kind: &str,
    reason: &str,
    alid: u32,
) -> EventData {
    EventData::EquipmentDown {
        equipment_id: equipment_id.to_owned(),
        kind: kind.to_owned(),
        reason: reason.to_owned(),
        alid,
    }
}

#[must_use]
pub fn equipment_up_payload(equipment_id: &str, kind: &str, down_ms: i64) -> EventData {
    EventData::EquipmentUp {
        equipment_id: equipment_id.to_owned(),
        kind: kind.to_owned(),
        down_ms,
    }
}

#[must_use]
pub fn split_payload(parent: &str, children: &[(LotId, Vec<WaferId>)], step_no: i32) -> EventData {
    EventData::LotSplit {
        parent_lot_id: parent.to_owned(),
        children: children
            .iter()
            .map(|(lot_id, wafers)| SplitChild {
                lot_id: lot_id.clone(),
                wafers: wafers.clone(),
            })
            .collect(),
        step_no,
    }
}

#[must_use]
pub fn merged_payload(
    sources: &[LotId],
    target: &str,
    step_no: i32,
    wafers: &[WaferId],
) -> EventData {
    EventData::LotMerged {
        sources: sources.to_vec(),
        target_lot_id: target.to_owned(),
        step_no,
        wafers: wafers.to_vec(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn every_kind() -> Vec<EventData> {
        vec![
            lot_created_payload("P", "R", 5, &["LOT-0001-W01".to_owned()]),
            lot_queued_payload(1),
            split_payload(
                "LOT-0001",
                &[("LOT-0001-A".to_owned(), vec!["LOT-0001-W01".to_owned()])],
                1,
            ),
            merged_payload(
                &["LOT-0002".to_owned()],
                "LOT-0001",
                1,
                &["LOT-0002-W01".to_owned()],
            ),
            EventData::LotHeld {
                reason: "engineer review".to_owned(),
                step_no: 2,
            },
            EventData::LotReleased {
                step_no: 2,
                reason: Some("engineer review".to_owned()),
            },
            EventData::TrackIn {
                step_no: 1,
                equipment_id: "CLN-01".to_owned(),
                recipe_id: "RCP-CLEAN".to_owned(),
                wafers: vec!["LOT-0001-W01".to_owned()],
            },
            EventData::TrackOut {
                step_no: 1,
                equipment_id: "CLN-01".to_owned(),
                recipe_id: "RCP-CLEAN".to_owned(),
                recipe_checksum: "abc".to_owned(),
                wafers: vec!["LOT-0001-W01".to_owned()],
                duration_ms: 6000,
                measured: Some(100.5),
            },
            EventData::TrackOutDuplicate {
                step_no: 1,
                equipment_id: "CLN-01".to_owned(),
                wafers: vec!["LOT-0001-W01".to_owned()],
                idempotent: true,
            },
            EventData::ReworkStarted {
                from_step: 3,
                to_step: 1,
                reason: "scanner drift".to_owned(),
            },
            EventData::WaferScrapped {
                wafer_id: "LOT-0001-W01".to_owned(),
                reason: "particle".to_owned(),
                step_no: 2,
            },
            EventData::LotCompleted { step_no: 4 },
            equipment_down_payload("CLN-01", "clean", "alarm", 1001),
            equipment_up_payload("CLN-01", "clean", 500),
            EventData::RecipeVerified {
                recipe_id: "RCP-CLEAN".to_owned(),
                checksum: "abc".to_owned(),
            },
        ]
    }

    #[test]
    fn every_event_serialises_to_its_published_shape() {
        for data in every_kind() {
            let event = DomainEvent::new(Uuid::nil(), "LOT-0001", data.clone(), 1, "test");
            let json = serde_json::to_value(&event).unwrap();
            assert_eq!(json["kind"], data.kind().as_str());
            assert!(json["payload"].is_object(), "{json}");
            let back: DomainEvent = serde_json::from_value(json).unwrap();
            assert_eq!(back.data, data);
            assert_eq!(back.payload_json(), event.payload_json());
        }
    }

    #[test]
    fn a_missing_required_key_is_an_error_not_a_default() {
        let error = EventData::from_parts(
            EventKind::TrackOut,
            &json!({
                "step_no": 1,
                "equipment_id": "CLN-01",
                "recipe_id": "RCP-CLEAN",
                "wafers": [],
                "duration_ms": 1
            }),
        )
        .unwrap_err();
        assert!(error.to_string().contains("recipe_checksum"), "{error}");
    }

    #[test]
    fn a_payload_for_another_kind_is_an_error() {
        let error = EventData::from_parts(
            EventKind::TrackOut,
            &json!({ "product_id": "P", "route_id": "R", "priority": 1, "wafers": [] }),
        )
        .unwrap_err();
        assert!(error.to_string().contains("track_out"), "{error}");
    }
}
