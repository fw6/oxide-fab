//! Deriving time series rows from domain events.
//!
//! This is the only place where the shape of `equipment_telemetry`,
//! `gem_event`, `spc_sample` and `e116_perf` is decided. The collector uses it
//! when a transaction commits, and `replay` uses the same function on the same
//! events, so a replay from PostgreSQL reproduces the time series store
//! exactly.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::events::{DomainEvent, EventKind};
use crate::model::Route;

/// Which time series table a row belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricTable {
    EquipmentTelemetry,
    GemEvent,
    SpcSample,
    E116Perf,
}

impl MetricTable {
    /// The table name in the time series store.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            MetricTable::EquipmentTelemetry => "equipment_telemetry",
            MetricTable::GemEvent => "gem_event",
            MetricTable::SpcSample => "spc_sample",
            MetricTable::E116Perf => "e116_perf",
        }
    }

    #[must_use]
    pub fn all() -> [MetricTable; 4] {
        [
            MetricTable::EquipmentTelemetry,
            MetricTable::GemEvent,
            MetricTable::SpcSample,
            MetricTable::E116Perf,
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TelemetryPoint {
    pub ts_ms: i64,
    pub event_id: Uuid,
    pub equipment_id: String,
    pub lot_id: String,
    pub step_id: String,
    pub param: String,
    pub value: f64,
    pub unit: String,
    pub quality: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GemPoint {
    pub ts_ms: i64,
    pub event_id: Uuid,
    pub equipment_id: String,
    pub stream: i32,
    pub function: i32,
    pub ceid: i64,
    pub alid: i64,
    pub payload: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpcPoint {
    pub ts_ms: i64,
    pub event_id: Uuid,
    pub equipment_id: String,
    pub param: String,
    pub value: f64,
    pub ucl: f64,
    pub lcl: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerfPoint {
    pub ts_ms: i64,
    pub event_id: Uuid,
    pub equipment_id: String,
    pub state: String,
    pub duration_ms: i64,
}

/// One row to write into one of the four tables.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "table", rename_all = "snake_case")]
pub enum MetricRow {
    Telemetry(TelemetryPoint),
    Gem(GemPoint),
    Spc(SpcPoint),
    Perf(PerfPoint),
}

impl MetricRow {
    #[must_use]
    pub fn table(&self) -> MetricTable {
        match self {
            MetricRow::Telemetry(_) => MetricTable::EquipmentTelemetry,
            MetricRow::Gem(_) => MetricTable::GemEvent,
            MetricRow::Spc(_) => MetricTable::SpcSample,
            MetricRow::Perf(_) => MetricTable::E116Perf,
        }
    }

    #[must_use]
    pub fn event_id(&self) -> Uuid {
        match self {
            MetricRow::Telemetry(p) => p.event_id,
            MetricRow::Gem(p) => p.event_id,
            MetricRow::Spc(p) => p.event_id,
            MetricRow::Perf(p) => p.event_id,
        }
    }
}

/// GEM collection event ids used by the simulated equipment.
pub const CEID_PROCESS_STARTED: i64 = 1001;
pub const CEID_PROCESS_COMPLETED: i64 = 1002;
pub const CEID_EQUIPMENT_UP: i64 = 1003;
pub const CEID_RECIPE_VERIFIED: i64 = 1004;
pub const ALID_EQUIPMENT_DOWN: i64 = 2001;

/// Derive every time series row the event implies.
///
/// Pure: same event + same route ⇒ same rows, which is what makes replay and
/// the idempotency contract testable. One function per event kind keeps each
/// table's shape next to the code that fills it.
#[must_use]
pub fn derive(event: &DomainEvent, route: &Route) -> Vec<MetricRow> {
    match event.kind {
        EventKind::TrackOut => track_out_rows(event, route),
        EventKind::TrackIn => track_in_rows(event),
        EventKind::EquipmentDown => equipment_down_rows(event),
        EventKind::EquipmentUp => equipment_up_rows(event),
        EventKind::RecipeVerified => recipe_verified_rows(event),
        _ => Vec::new(),
    }
}

fn track_out_rows(event: &DomainEvent, route: &Route) -> Vec<MetricRow> {
    let Some(step_no) = event.step_no() else {
        return Vec::new();
    };
    let Some(step) = route.step(step_no) else {
        return Vec::new();
    };
    let equipment_id = event.equipment_id().unwrap_or_default().to_owned();
    let duration_ms = event
        .payload
        .get("duration_ms")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or_default();
    let measured = event
        .payload
        .get("measured")
        .and_then(serde_json::Value::as_f64);
    let step_id = format!("{}-{}", step.step_no, step.name);
    let mut rows = vec![
        MetricRow::Telemetry(TelemetryPoint {
            ts_ms: event.occurred_at_ms,
            event_id: event.event_id,
            equipment_id: equipment_id.clone(),
            lot_id: event.lot_id.clone(),
            step_id: step_id.clone(),
            param: "process_duration_ms".to_owned(),
            value: duration_ms as f64,
            unit: "ms".to_owned(),
            quality: "good".to_owned(),
        }),
        MetricRow::Gem(GemPoint {
            ts_ms: event.occurred_at_ms,
            event_id: event.event_id,
            equipment_id: equipment_id.clone(),
            stream: 6,
            function: 11,
            ceid: CEID_PROCESS_COMPLETED,
            alid: 0,
            payload: format!("{{\"step\":\"{step_id}\"}}"),
        }),
        MetricRow::Perf(PerfPoint {
            ts_ms: event.occurred_at_ms,
            event_id: event.event_id,
            equipment_id: equipment_id.clone(),
            state: "processing".to_owned(),
            duration_ms,
        }),
    ];
    if let (Some(measured), Some(ucl), Some(lcl)) = (measured, step.ucl, step.lcl) {
        rows.push(MetricRow::Spc(SpcPoint {
            ts_ms: event.occurred_at_ms,
            event_id: event.event_id,
            equipment_id,
            param: format!("{}_cd_nm", step.name),
            value: measured,
            ucl,
            lcl,
        }));
    }
    rows
}

fn track_in_rows(event: &DomainEvent) -> Vec<MetricRow> {
    let (Some(step_no), Some(equipment_id)) = (event.step_no(), event.equipment_id()) else {
        return Vec::new();
    };
    vec![MetricRow::Gem(GemPoint {
        ts_ms: event.occurred_at_ms,
        event_id: event.event_id,
        equipment_id: equipment_id.to_owned(),
        stream: 6,
        function: 11,
        ceid: CEID_PROCESS_STARTED,
        alid: 0,
        payload: format!("{{\"step_no\":{step_no}}}"),
    })]
}

fn equipment_down_rows(event: &DomainEvent) -> Vec<MetricRow> {
    let equipment_id = event.equipment_id().unwrap_or_default().to_owned();
    let alid = event
        .payload
        .get("alid")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(ALID_EQUIPMENT_DOWN);
    let reason = event
        .payload
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    vec![MetricRow::Gem(GemPoint {
        ts_ms: event.occurred_at_ms,
        event_id: event.event_id,
        equipment_id,
        stream: 5,
        function: 1,
        ceid: 0,
        alid,
        payload: format!("{{\"reason\":\"{reason}\"}}"),
    })]
}

fn equipment_up_rows(event: &DomainEvent) -> Vec<MetricRow> {
    let equipment_id = event.equipment_id().unwrap_or_default().to_owned();
    let down_ms = event
        .payload
        .get("down_ms")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or_default();
    vec![
        MetricRow::Gem(GemPoint {
            ts_ms: event.occurred_at_ms,
            event_id: event.event_id,
            equipment_id: equipment_id.clone(),
            stream: 6,
            function: 11,
            ceid: CEID_EQUIPMENT_UP,
            alid: 0,
            payload: "{}".to_owned(),
        }),
        MetricRow::Perf(PerfPoint {
            ts_ms: event.occurred_at_ms,
            event_id: event.event_id,
            equipment_id,
            state: "down".to_owned(),
            duration_ms: down_ms,
        }),
    ]
}

fn recipe_verified_rows(event: &DomainEvent) -> Vec<MetricRow> {
    let recipe_id = event.recipe_id().unwrap_or_default();
    let equipment_id = event.equipment_id().unwrap_or_default().to_owned();
    vec![MetricRow::Gem(GemPoint {
        ts_ms: event.occurred_at_ms,
        event_id: event.event_id,
        equipment_id,
        stream: 7,
        function: 3,
        ceid: CEID_RECIPE_VERIFIED,
        alid: 0,
        payload: format!("{{\"recipe\":\"{recipe_id}\"}}"),
    })]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{TrackOutReport, track_out_payload};
    use crate::model::RouteStep;

    fn route() -> Route {
        Route {
            id: "R".into(),
            name: "r".into(),
            steps: vec![RouteStep {
                step_no: 4,
                name: "metrology".into(),
                equipment_kind: "metrology".into(),
                equipment_qualification: vec!["MET-01".into()],
                recipe_id: "RCP-METRO".into(),
                duration_ms: 5000,
                target: Some(100.0),
                ucl: Some(103.0),
                lcl: Some(97.0),
                qtime_ms: None,
            }],
        }
    }

    fn event(kind: EventKind, payload: serde_json::Value, eid: Uuid) -> DomainEvent {
        DomainEvent::new(eid, "LOT-0001", kind, payload, 1_700_000_000_000, "test")
    }

    #[test]
    fn a_metrology_track_out_yields_four_rows() {
        let eid = Uuid::nil();
        let event = event(
            EventKind::TrackOut,
            track_out_payload(&TrackOutReport {
                step_no: 4,
                equipment_id: "MET-01",
                recipe_id: "RCP-METRO",
                recipe_checksum: "abc",
                wafers: &["W1".into()],
                duration_ms: 5000,
                measured: Some(101.2),
            }),
            eid,
        );
        let rows = derive(&event, &route());
        assert_eq!(rows.len(), 4);
        // The sample is copied through the payload verbatim, so bit equality
        // is the contract here, not an epsilon comparison.
        let measured = rows
            .iter()
            .find_map(|row| match row {
                MetricRow::Spc(point) => Some(point.value),
                _ => None,
            })
            .expect("a metrology track-out yields an SPC sample");
        assert_eq!(measured, 101.2);
        assert!(rows.iter().all(|r| r.event_id() == eid));
    }

    #[test]
    fn every_row_carries_the_event_id_and_the_right_table() {
        let event = event(
            EventKind::EquipmentDown,
            crate::events::equipment_down_payload("ETC-01", "etch", "arc detected", 2001),
            Uuid::nil(),
        );
        let rows = derive(&event, &route());
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].table(), MetricTable::GemEvent);
        match &rows[0] {
            MetricRow::Gem(p) => {
                assert_eq!((p.stream, p.function, p.alid), (5, 1, 2001));
            }
            _ => panic!("expected a gem row"),
        }
    }
}
