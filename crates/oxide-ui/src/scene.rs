//! The scene description TSX sends, and its bridge to `oxide-scene`.
//!
//! TSX owns the snapshot, the level and the selection; Rust owns the geometry.
//! `SceneInput` is that boundary: one value per rendered state, built from the
//! same fields the panels show, with entities as plain objects so a pick can
//! come back the same way.

use nsh_core::prelude::*;
use oxide_scene::{
    AmhsPath, Camera, Entity, FloorState, Level, LotState, SceneGraph, SceneOptions, StepState,
    ToolState, WaferState, build,
};

use crate::engine::manifest;
use crate::palette;

/// Which drill level the view shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PropEnum)]
pub enum LevelInput {
    /// The whole factory.
    Factory,
    /// One tool, its parts exploded.
    Tool,
    /// One lot and its wafers.
    Lot,
    /// One event, marked in the scene.
    Event,
}

impl From<LevelInput> for Level {
    fn from(level: LevelInput) -> Self {
        match level {
            LevelInput::Factory => Level::Factory,
            LevelInput::Tool => Level::Tool,
            LevelInput::Lot => Level::Lot,
            LevelInput::Event => Level::Event,
        }
    }
}

/// What kind of scene entity a selection is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PropEnum)]
pub enum EntityKind {
    /// The factory floor.
    #[default]
    Floor,
    /// One route step's band.
    Band,
    /// The AMHS ring.
    Rail,
    /// A whole tool.
    Tool,
    /// One part of a tool.
    ToolPart,
    /// A FOUP.
    Foup,
    /// One wafer in a lot.
    Wafer,
    /// The exit area.
    Exit,
}

/// A scene entity, as TSX names it and a pick returns it.
#[derive(Debug, Clone, Default, JsObject)]
pub struct EntityInput {
    /// Which kind this is; the other fields are read by kind.
    pub kind: EntityKind,
    /// The route step, for `band`.
    pub step_no: Option<i32>,
    /// The tool, for `tool`, `tool_part` and `wafer` (through the lot).
    pub equipment_id: Option<String>,
    /// The part, for `tool_part`.
    pub part: Option<String>,
    /// The lot, for `foup` and `wafer`.
    pub lot_id: Option<String>,
    /// The wafer, for `wafer`.
    pub wafer_id: Option<String>,
}

impl EntityInput {
    /// The entity, when the fields this kind needs are there.
    #[must_use]
    pub fn to_entity(&self) -> Option<Entity> {
        match self.kind {
            EntityKind::Floor => Some(Entity::Floor),
            EntityKind::Band => Some(Entity::Band {
                step_no: self.step_no?,
            }),
            EntityKind::Rail => Some(Entity::Rail),
            EntityKind::Tool => Some(Entity::Tool {
                equipment_id: self.equipment_id.clone()?,
            }),
            EntityKind::ToolPart => Some(Entity::ToolPart {
                equipment_id: self.equipment_id.clone()?,
                part: self.part.clone()?,
            }),
            EntityKind::Foup => Some(Entity::Foup {
                lot_id: self.lot_id.clone()?,
            }),
            EntityKind::Wafer => Some(Entity::Wafer {
                lot_id: self.lot_id.clone()?,
                wafer_id: self.wafer_id.clone()?,
            }),
            EntityKind::Exit => Some(Entity::Exit),
        }
    }

    /// The entity as TSX names it.
    #[must_use]
    pub fn from_entity(entity: &Entity) -> Self {
        match entity {
            Entity::Floor => Self {
                kind: EntityKind::Floor,
                ..Self::default()
            },
            Entity::Band { step_no } => Self {
                kind: EntityKind::Band,
                step_no: Some(*step_no),
                ..Self::default()
            },
            Entity::Rail => Self {
                kind: EntityKind::Rail,
                ..Self::default()
            },
            Entity::Tool { equipment_id } => Self {
                kind: EntityKind::Tool,
                equipment_id: Some(equipment_id.clone()),
                ..Self::default()
            },
            Entity::ToolPart { equipment_id, part } => Self {
                kind: EntityKind::ToolPart,
                equipment_id: Some(equipment_id.clone()),
                part: Some(part.clone()),
                ..Self::default()
            },
            Entity::Foup { lot_id } => Self {
                kind: EntityKind::Foup,
                lot_id: Some(lot_id.clone()),
                ..Self::default()
            },
            Entity::Wafer { lot_id, wafer_id } => Self {
                kind: EntityKind::Wafer,
                lot_id: Some(lot_id.clone()),
                wafer_id: Some(wafer_id.clone()),
                ..Self::default()
            },
            Entity::Exit => Self {
                kind: EntityKind::Exit,
                ..Self::default()
            },
        }
    }
}

/// One route step, as the panels and the scene use it.
#[derive(Debug, Clone, JsObject)]
pub struct StepInput {
    /// The step number.
    pub step_no: i32,
    /// Its name.
    pub name: String,
    /// The equipment kind it runs on.
    pub equipment_kind: String,
    /// How many lots wait for it.
    pub queue_len: usize,
    /// How many of its tools are busy.
    pub busy_tools: usize,
    /// How many tools it has.
    pub total_tools: usize,
    /// The lots waiting now.
    pub current_lots: Vec<String>,
}

/// One tool.
#[derive(Debug, Clone, JsObject)]
pub struct ToolInput {
    /// The equipment id.
    pub id: String,
    /// Its kind.
    pub kind: String,
    /// Its E116 state.
    pub state: String,
    /// Its light: `green`, `amber` or `red`.
    pub light: String,
    /// The lot on it, if any.
    pub current_lot: Option<String>,
    /// The step it is running.
    pub current_step: Option<i32>,
    /// How many lots it has processed.
    pub lots_processed: u64,
}

/// One wafer.
#[derive(Debug, Clone, JsObject)]
pub struct WaferInput {
    /// The wafer id.
    pub id: String,
    /// Its FOUP slot, 1-based.
    pub slot: i32,
    /// Its state.
    pub state: String,
    /// Its defect class, when scrapped.
    pub defect_info: Option<String>,
    /// How often it was reworked.
    pub rework_count: u32,
}

/// One lot.
#[derive(Debug, Clone, JsObject)]
pub struct LotInput {
    /// The lot id.
    pub id: String,
    /// Its state.
    pub state: String,
    /// The step it is at.
    pub current_step: i32,
    /// Its dispatch priority.
    pub priority: i32,
    /// How many wafers it holds.
    pub wafer_count: usize,
    /// How many are done.
    pub done_wafers: usize,
    /// Why it is on hold, when it is.
    pub hold_reason: Option<String>,
    /// When it entered the step, for the FOUP transit; 0 when unknown.
    pub step_since_ms: i64,
    /// Its wafers.
    pub wafers: Vec<WaferInput>,
}

/// Everything the 3D view draws, as TSX sends it.
#[expect(
    clippy::struct_excessive_bools,
    reason = "independent scene inputs: running, reduce_motion, alert and dark are not one state"
)]
#[derive(Debug, Clone, JsObject)]
pub struct SceneInput {
    /// The simulation clock.
    pub now_ms: i64,
    /// Whether the line runs.
    pub running: bool,
    /// The simulation speed.
    pub speed: f64,
    /// The route steps.
    pub steps: Vec<StepInput>,
    /// The tools.
    pub tools: Vec<ToolInput>,
    /// The lots.
    pub lots: Vec<LotInput>,
    /// The drill level.
    pub level: LevelInput,
    /// What the camera frames.
    pub focus: Option<EntityInput>,
    /// What is picked.
    pub selected: Option<EntityInput>,
    /// How far the focused tool's parts are apart, 0–100.
    pub explosion: f64,
    /// Whether the FOUP transit is skipped.
    pub reduce_motion: bool,
    /// Whether the derived channel is behind (amber AMHS ring).
    pub alert: bool,
    /// Whether the dark scene palette applies.
    pub dark: bool,
}

impl SceneInput {
    fn floor(&self) -> FloorState {
        FloorState {
            now_ms: self.now_ms,
            running: self.running,
            speed: self.speed,
            steps: self
                .steps
                .iter()
                .map(|step| StepState {
                    step_no: step.step_no,
                    name: step.name.clone(),
                    equipment_kind: step.equipment_kind.clone(),
                    queue_len: step.queue_len,
                    busy_tools: step.busy_tools,
                    total_tools: step.total_tools,
                    current_lots: step.current_lots.clone(),
                })
                .collect(),
            tools: self
                .tools
                .iter()
                .map(|tool| ToolState {
                    id: tool.id.clone(),
                    kind: tool.kind.clone(),
                    state: tool.state.clone(),
                    light: tool.light.clone(),
                    current_lot: tool.current_lot.clone(),
                    current_step: tool.current_step,
                    lots_processed: tool.lots_processed,
                })
                .collect(),
            lots: self
                .lots
                .iter()
                .map(|lot| LotState {
                    id: lot.id.clone(),
                    state: lot.state.clone(),
                    current_step: lot.current_step,
                    priority: lot.priority,
                    wafer_count: lot.wafer_count,
                    done_wafers: lot.done_wafers,
                    hold_reason: lot.hold_reason.clone(),
                    step_since_ms: lot.step_since_ms,
                    wafers: lot
                        .wafers
                        .iter()
                        .map(|wafer| WaferState {
                            id: wafer.id.clone(),
                            slot: wafer.slot,
                            state: wafer.state.clone(),
                            defect_info: wafer.defect_info.clone(),
                            rework_count: wafer.rework_count,
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    fn options(&self) -> SceneOptions {
        SceneOptions {
            level: self.level.into(),
            focus: self.focus.as_ref().and_then(EntityInput::to_entity),
            explosion: (self.explosion / 100.0).clamp(0.0, 1.0) as f32,
            selected: self.selected.as_ref().and_then(EntityInput::to_entity),
            reduce_motion: self.reduce_motion,
            derive_alert: self.alert,
            theme: palette::scene_theme(self.dark),
        }
    }
}

/// The graph for a scene, or `None` when the shipped manifest cannot be read.
#[must_use]
pub fn graph(scene: &SceneInput) -> Option<SceneGraph> {
    let manifest = manifest()?;
    let path = AmhsPath::new(manifest);
    Some(build(manifest, &scene.floor(), &scene.options(), &path))
}

/// The bounds a level frames: the tool, the lot, or the whole floor.
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

/// Frame `entity` in `camera` against `graph`.
pub fn frame(graph: &SceneGraph, camera: &mut Camera, entity: Option<&Entity>) {
    if let Some(bounds) = bounds_for(graph, entity) {
        camera.focus(bounds);
        if let Some(entity) = entity {
            // A lot's box includes the wafer stack rising out of the FOUP;
            // framing it needs more room than a tool does.
            camera.distance *= if entity.lot_id().is_some() { 1.7 } else { 0.5 };
        }
    }
}
