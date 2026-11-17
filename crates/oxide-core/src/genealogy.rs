//! Per-wafer genealogy: which tool ran which step, when, with which recipe.
//!
//! Genealogy is derived, never stored twice: the store folds `lot_event` into
//! these records, so the wafer history always matches the event log.

use serde::{Deserialize, Serialize};

use crate::events::{DomainEvent, EventKind};
use crate::model::{Wafer, WaferId, WaferState};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaferStepRecord {
    pub step_no: i32,
    pub equipment_id: String,
    pub recipe_id: String,
    pub track_in_ms: i64,
    pub track_out_ms: i64,
    pub duration_ms: i64,
    /// The measured value, for metrology steps.
    pub measured: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaferGenealogy {
    pub wafer_id: WaferId,
    pub lot_id: String,
    pub slot: i32,
    pub state: WaferState,
    pub defect_info: Option<String>,
    pub rework_count: u32,
    pub steps: Vec<WaferStepRecord>,
}

/// Fold the track-in / track-out events of a lot into per-wafer timelines.
///
/// Only events that name wafers are used; a track-out closes the most recent
/// open track-in for the same step and tool.
pub fn genealogy_for(wafers: &[Wafer], events: &[DomainEvent]) -> Vec<WaferGenealogy> {
    let mut out: Vec<WaferGenealogy> = wafers
        .iter()
        .map(|wafer| WaferGenealogy {
            wafer_id: wafer.id.clone(),
            lot_id: wafer.lot_id.clone(),
            slot: wafer.slot,
            state: wafer.state,
            defect_info: wafer.defect_info.clone(),
            rework_count: wafer.rework_count,
            steps: vec![],
        })
        .collect();

    for event in events {
        match event.kind {
            EventKind::TrackIn => {
                let Some(step_no) = event.step_no() else {
                    continue;
                };
                let equipment_id = event.equipment_id().unwrap_or_default().to_owned();
                let recipe_id = event.recipe_id().unwrap_or_default().to_owned();
                for wafer_id in event.wafers() {
                    if let Some(record) = out.iter_mut().find(|g| g.wafer_id == wafer_id) {
                        record.steps.push(WaferStepRecord {
                            step_no,
                            equipment_id: equipment_id.clone(),
                            recipe_id: recipe_id.clone(),
                            track_in_ms: event.occurred_at_ms,
                            track_out_ms: 0,
                            duration_ms: 0,
                            measured: None,
                        });
                    }
                }
            }
            EventKind::TrackOut => {
                let Some(step_no) = event.step_no() else {
                    continue;
                };
                let duration_ms = event
                    .payload
                    .get("duration_ms")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or_default();
                let measured = event
                    .payload
                    .get("measured")
                    .and_then(serde_json::Value::as_f64);
                for wafer_id in event.wafers() {
                    if let Some(record) = out.iter_mut().find(|g| g.wafer_id == wafer_id)
                        && let Some(step) = record
                            .steps
                            .iter_mut()
                            .rev()
                            .find(|s| s.step_no == step_no && s.track_out_ms == 0)
                    {
                        step.track_out_ms = event.occurred_at_ms;
                        step.duration_ms = duration_ms;
                        step.measured = measured;
                    }
                }
            }
            _ => {}
        }
    }
    for record in &mut out {
        record.steps.sort_by_key(|s| s.track_in_ms);
    }
    out
}
