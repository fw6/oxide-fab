//! The bridge from the wire snapshot to `oxide-scene`.
//!
//! One function builds the floor state, one builds the options, one the graph;
//! every call reads signals, so the 3D view redraws when the data does.

use std::collections::HashMap;

use oxide_scene::{
    AmhsPath, Camera, Entity, FabManifest, FloorState, Level, LotState, SceneGraph, SceneOptions,
    StepState, ToolState, WaferState, build,
};

use crate::engine::manifest;
use crate::state::Ui;

/// When each lot entered its current step, for the FOUP transit.
pub type StepSince = HashMap<String, (i32, i64)>;

#[must_use]
pub fn floor_state(ui: Ui) -> FloorState {
    let since = ui.step_since.get();
    let steps = ui
        .steps
        .get()
        .into_iter()
        .map(|step| StepState {
            step_no: step.step_no,
            name: step.name,
            equipment_kind: step.equipment_kind,
            queue_len: step.queue_len,
            busy_tools: step.busy_tools,
            total_tools: step.total_tools,
            current_lots: step.current_lots,
        })
        .collect();
    let tools = ui
        .tools
        .get()
        .into_iter()
        .map(|tool| ToolState {
            id: tool.id,
            kind: tool.kind,
            state: tool.state,
            light: tool.light,
            current_lot: tool.current_lot,
            current_step: tool.current_step,
            lots_processed: tool.lots_processed,
        })
        .collect();
    let lots = ui
        .lots
        .get()
        .into_iter()
        .map(|lot| {
            let step_since_ms = since
                .get(&lot.id)
                .filter(|(step, _)| *step == lot.current_step)
                .map_or(0, |(_, at)| *at);
            LotState {
                id: lot.id,
                state: lot.state,
                current_step: lot.current_step,
                priority: lot.priority,
                wafer_count: lot.wafer_count,
                done_wafers: lot.done_wafers,
                hold_reason: lot.hold_reason,
                step_since_ms,
                wafers: lot
                    .wafers
                    .into_iter()
                    .map(|wafer| WaferState {
                        id: wafer.id,
                        slot: wafer.slot,
                        state: wafer.state,
                        defect_info: wafer.defect_info,
                        rework_count: wafer.rework_count,
                    })
                    .collect(),
            }
        })
        .collect();
    FloorState {
        now_ms: ui.now_ms.get(),
        running: ui.running.get(),
        speed: ui.speed.get(),
        steps,
        tools,
        lots,
    }
}

#[must_use]
pub fn options(ui: Ui) -> SceneOptions {
    let palette = ui.palette();
    SceneOptions {
        level: ui.level.get(),
        focus: ui.focus.get(),
        explosion: f64::from(ui.explosion.get()) as f32 / 100.0,
        selected: ui.selected.get(),
        reduce_motion: ui.reduce_motion.get(),
        derive_alert: !ui.greptime_ok.get() || ui.outbox_pending.get() > 0,
        theme: palette.scene_theme(),
    }
}

#[must_use]
pub fn graph(ui: Ui) -> Option<SceneGraph> {
    let manifest = manifest()?;
    let path = AmhsPath::new(manifest);
    Some(build(manifest, &floor_state(ui), &options(ui), &path))
}

/// The bounds a level should frame: the tool, the lot, or the whole floor.
#[must_use]
pub fn bounds_for(graph: &SceneGraph, entity: Option<&Entity>) -> Option<oxide_scene::Aabb> {
    match entity {
        None => graph.nodes.first().map(|node| node.aabb),
        Some(Entity::Tool { equipment_id } | Entity::ToolPart { equipment_id, .. }) => {
            graph.tool_bounds(equipment_id)
        }
        Some(Entity::Foup { lot_id } | Entity::Wafer { lot_id, .. }) => graph.lot_bounds(lot_id),
        Some(other) => graph.bounds_of(other),
    }
}

/// Move the camera so `entity` fills the view.
pub fn focus_camera(ui: Ui, entity: Option<&Entity>) {
    let mut camera = ui.camera.get_untracked();
    if let Some(graph) = graph(ui)
        && let Some(bounds) = bounds_for(&graph, entity)
    {
        camera.focus(bounds);
        if let Some(entity) = entity {
            // A lot's box includes the wafer stack rising out of the FOUP;
            // framing it needs more room than a tool does.
            camera.distance *= if entity.lot_id().is_some() { 1.7 } else { 0.5 };
        }
    }
    ui.camera.set(camera);
}

/// Drill into an entity: pick the level, remember where we came from, frame it,
/// and load the lot's genealogy when the entity is a lot or a wafer.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the entity is stored, cloned into a focus, and pushed on the trail; taking it by value reads as the move it is"
)]
pub fn drill(ui: Ui, entity: Entity) {
    ui.selected.set(Some(entity.clone()));
    match &entity {
        Entity::Tool { equipment_id } | Entity::ToolPart { equipment_id, .. } => {
            ui.level.set(Level::Tool);
            ui.focus.set(Some(Entity::Tool {
                equipment_id: equipment_id.clone(),
            }));
            ui.trail.update(|trail| trail.push(entity.clone()));
            focus_camera(
                ui,
                Some(&Entity::Tool {
                    equipment_id: equipment_id.clone(),
                }),
            );
        }
        Entity::Foup { lot_id } => {
            ui.level.set(Level::Lot);
            ui.focus.set(Some(Entity::Foup {
                lot_id: lot_id.clone(),
            }));
            ui.trail.update(|trail| trail.push(entity.clone()));
            crate::client::load_lot(ui, lot_id.clone());
            focus_camera(
                ui,
                Some(&Entity::Foup {
                    lot_id: lot_id.clone(),
                }),
            );
        }
        Entity::Wafer { lot_id, wafer_id } => {
            ui.level.set(Level::Lot);
            ui.focus.set(Some(Entity::Foup {
                lot_id: lot_id.clone(),
            }));
            ui.trail.update(|trail| trail.push(entity.clone()));
            // `load_lot` clears the wafer selection for a new lot; set it after.
            crate::client::load_lot(ui, lot_id.clone());
            ui.selected_wafer.set(Some(wafer_id.clone()));
            focus_camera(
                ui,
                Some(&Entity::Foup {
                    lot_id: lot_id.clone(),
                }),
            );
        }
        Entity::Floor | Entity::Band { .. } | Entity::Rail | Entity::Exit => {
            ui.level.set(Level::Factory);
            ui.focus.set(None);
            focus_camera(ui, None);
        }
    }
}

/// Go up one level, keeping the selection.
pub fn level_up(ui: Ui) {
    let trail = ui.trail.get_untracked();
    let previous = trail.len().checked_sub(1).and_then(|_| {
        let mut trail = trail;
        let previous = trail.pop();
        ui.trail.set(trail);
        previous
    });
    if let Some(entity) = previous {
        match entity {
            Entity::Tool { .. } | Entity::ToolPart { .. } => {
                ui.level.set(Level::Tool);
                ui.focus.set(Some(entity.clone()));
                focus_camera(ui, Some(&entity));
            }
            Entity::Foup { .. } | Entity::Wafer { .. } => {
                ui.level.set(Level::Lot);
                ui.focus.set(Some(entity.clone()));
                focus_camera(ui, Some(&entity));
            }
            _ => {
                ui.level.set(Level::Factory);
                ui.focus.set(None);
                focus_camera(ui, None);
            }
        }
    } else {
        ui.level.set(Level::Factory);
        ui.focus.set(None);
        focus_camera(ui, None);
    }
}

#[must_use]
pub fn camera_reset() -> Camera {
    Camera::default()
}

/// The manifest, for panels that list the tools and bands.
#[must_use]
pub fn fab() -> Option<&'static FabManifest> {
    manifest()
}
