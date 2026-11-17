//! The scene graph: the single object model the renderer draws and the picker
//! hits.
//!
//! `build` turns the manifest plus the read-only floor state into a flat list
//! of drawable nodes. It is rebuilt whenever data or the camera level changes;
//! it holds no GPU state, so it can be built and asserted in a unit test.

use glam::{Mat4, Vec3};

use crate::camera::{Ray, ray_aabb};
use crate::layout::{AmhsPath, foup_placement};
use crate::manifest::{Aabb, FabManifest};
use crate::state::{FloorState, LotState, WaferState};

/// The three drill-down levels, plus the factory overview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    #[default]
    Factory,
    Tool,
    Lot,
    Event,
}

impl Level {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Level::Factory => "L0 全厂",
            Level::Tool => "L1 机台",
            Level::Lot => "L2 批次",
            Level::Event => "L3 事件",
        }
    }
}

/// What a drawn node *is* in the domain. Picking returns this, and the UI
/// maps it onto its own selection without a second index.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Entity {
    Floor,
    Band { step_no: i32 },
    Rail,
    Tool { equipment_id: String },
    ToolPart { equipment_id: String, part: String },
    Foup { lot_id: String },
    Wafer { lot_id: String, wafer_id: String },
    Exit,
}

impl Entity {
    #[must_use]
    pub fn tool_id(&self) -> Option<&str> {
        match self {
            Entity::Tool { equipment_id } | Entity::ToolPart { equipment_id, .. } => {
                Some(equipment_id)
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn lot_id(&self) -> Option<&str> {
        match self {
            Entity::Foup { lot_id } | Entity::Wafer { lot_id, .. } => Some(lot_id),
            _ => None,
        }
    }

    /// A stable key for the UI's selection signals.
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Entity::Floor => "floor".to_owned(),
            Entity::Band { step_no } => format!("band.{step_no}"),
            Entity::Rail => "rail".to_owned(),
            Entity::Tool { equipment_id } => format!("tool.{equipment_id}"),
            Entity::ToolPart { equipment_id, part } => format!("part.{equipment_id}.{part}"),
            Entity::Foup { lot_id } => format!("foup.{lot_id}"),
            Entity::Wafer { lot_id, wafer_id } => format!("wafer.{lot_id}.{wafer_id}"),
            Entity::Exit => "exit".to_owned(),
        }
    }
}

/// Shapes the renderer draws itself (status lights, beacons, selection marks).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Cube,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneNode {
    pub entity: Entity,
    /// The glTF node whose mesh this draws, with its world transform baked at
    /// load; `None` for shapes the renderer generates.
    pub mesh: Option<String>,
    pub shape: Option<Shape>,
    /// Applied to the baked mesh; the identity for static geometry.
    pub transform: Mat4,
    /// World-space picking box, explosion included.
    pub aabb: Aabb,
    /// sRGB, multiplied into the material's base colour.
    pub color: [f32; 4],
    /// 0 = lit only, 1 = fully self-lit (kept for the shader).
    pub emissive: f32,
    pub pickable: bool,
    pub dimmed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneLine {
    pub a: Vec3,
    pub b: Vec3,
    pub color: [f32; 4],
}

/// Colours the app picks per theme; the scene itself has no palette of its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneTheme {
    pub background: [f32; 4],
    pub accent: [f32; 3],
    pub dim_factor: f32,
    pub grid: [f32; 4],
}

impl Default for SceneTheme {
    fn default() -> Self {
        Self {
            background: [0.055, 0.067, 0.090, 1.0],
            accent: [0.98, 0.80, 0.30],
            dim_factor: 0.34,
            grid: [0.16, 0.20, 0.26, 0.55],
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneOptions {
    pub level: Level,
    /// The tool, lot or wafer in focus; `None` at the factory overview.
    pub focus: Option<Entity>,
    /// 0 = assembled, 1 = fully exploded.
    pub explosion: f32,
    pub selected: Option<Entity>,
    pub reduce_motion: bool,
    /// The derived channel is behind (`GreptimeDB` down, or the outbox has a
    /// backlog): the AMHS ring and a beacon at the entry turn amber.
    pub derive_alert: bool,
    pub theme: SceneTheme,
}

impl Default for SceneOptions {
    fn default() -> Self {
        Self {
            level: Level::Factory,
            focus: None,
            explosion: 0.0,
            selected: None,
            reduce_motion: false,
            derive_alert: false,
            theme: SceneTheme::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SceneGraph {
    pub nodes: Vec<SceneNode>,
    pub lines: Vec<SceneLine>,
    pub background: [f32; 4],
    pub grid: [f32; 4],
}

#[derive(Debug, Clone, PartialEq)]
pub struct PickHit {
    pub node: usize,
    pub entity: Entity,
    pub distance: f32,
    pub point: Vec3,
}

const GREEN: [f32; 3] = [0.13, 0.72, 0.36];
const AMBER: [f32; 3] = [0.96, 0.62, 0.04];
const RED: [f32; 3] = [0.94, 0.27, 0.27];
const BLUE: [f32; 3] = [0.30, 0.60, 1.00];
const SLATE: [f32; 3] = [0.62, 0.68, 0.76];

/// Build the draw list for one frame.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one pass over the scene: static geometry, tools, FOUPs, wafers and the level-3 beacon"
)]
pub fn build(
    manifest: &FabManifest,
    floor: &FloorState,
    options: &SceneOptions,
    path: &AmhsPath,
) -> SceneGraph {
    let mut graph = SceneGraph {
        nodes: Vec::with_capacity(96),
        lines: Vec::new(),
        background: options.theme.background,
        grid: options.theme.grid,
    };
    // A light theme needs a lighter floor: the GLB's materials are tuned for
    // the dark room, and the same tint multiplier works for both.
    let light_scene = options.theme.background[0] > 0.5;
    let static_tint = if light_scene {
        [4.2, 4.2, 4.2, 1.0]
    } else {
        [1.0, 1.0, 1.0, 1.0]
    };
    let deep = options.level != Level::Factory;

    graph.nodes.push(SceneNode {
        entity: Entity::Floor,
        mesh: manifest.static_node("floor").map(str::to_owned),
        shape: None,
        transform: Mat4::IDENTITY,
        aabb: Aabb::new(
            [-manifest.floor.half_x, -0.3, -manifest.floor.half_z],
            [manifest.floor.half_x, 0.0, manifest.floor.half_z],
        ),
        color: static_tint,
        emissive: 0.0,
        pickable: false,
        dimmed: false,
    });
    for band in &manifest.bands {
        graph.nodes.push(SceneNode {
            entity: Entity::Band {
                step_no: band.step_no,
            },
            mesh: manifest.band_node(band.step_no).map(str::to_owned),
            shape: None,
            transform: Mat4::IDENTITY,
            aabb: Aabb::new(
                [-band.half_x, band.y - 0.2, band.z - 1.5],
                [band.half_x, band.y, band.z + 1.5],
            ),
            color: static_tint,
            emissive: 0.0,
            pickable: false,
            dimmed: deep,
        });
    }
    for node in manifest.amhs_nodes() {
        graph.nodes.push(SceneNode {
            entity: Entity::Rail,
            mesh: Some(node.to_owned()),
            shape: None,
            transform: Mat4::IDENTITY,
            aabb: Aabb::ZERO,
            color: static_tint,
            emissive: 0.0,
            pickable: false,
            dimmed: deep,
        });
    }

    // Floor grid and the AMHS centre line: the two pieces of the factory that
    // are geometry of the layout rather than of the model.
    let grid = options.theme.grid;
    let step = 2.0;
    let mut x = -manifest.floor.half_x;
    while x <= manifest.floor.half_x {
        graph.lines.push(SceneLine {
            a: Vec3::new(x, 0.012, -manifest.floor.half_z),
            b: Vec3::new(x, 0.012, manifest.floor.half_z),
            color: grid,
        });
        x += step;
    }
    let mut z = -manifest.floor.half_z;
    while z <= manifest.floor.half_z {
        graph.lines.push(SceneLine {
            a: Vec3::new(-manifest.floor.half_x, 0.012, z),
            b: Vec3::new(manifest.floor.half_x, 0.012, z),
            color: grid,
        });
        z += step;
    }
    let ring = manifest.ring();
    for index in 0..ring.len() {
        let a = ring[index];
        let b = ring[(index + 1) % ring.len()];
        graph.lines.push(SceneLine {
            a,
            b,
            color: if options.derive_alert {
                [0.96, 0.62, 0.04, 0.95]
            } else {
                [0.34, 0.55, 0.70, 0.75]
            },
        });
    }
    for (step_no, [x, z]) in &manifest.amhs.stops {
        if let Some(band) = manifest.band(step_no.parse().unwrap_or_default()) {
            let stop_point = Vec3::new(*x, manifest.amhs.height, *z);
            let spur = Vec3::new(*x, manifest.amhs.height, band.z);
            graph.lines.push(SceneLine {
                a: stop_point,
                b: spur,
                color: [0.34, 0.55, 0.70, 0.55],
            });
            graph.lines.push(SceneLine {
                a: spur,
                b: Vec3::new(*x, 0.05, band.z + 2.35),
                color: [0.34, 0.55, 0.70, 0.35],
            });
        }
    }

    let focused_tool = options.focus.as_ref().and_then(Entity::tool_id);
    let focused_lot = options.focus.as_ref().and_then(Entity::lot_id);
    let all_exploded = options.level == Level::Tool;

    for tool in &manifest.tools {
        let state = floor.tool(&tool.equipment_id);
        let light = state.map_or("green", |state| state.light.as_str());
        let light_color = match light {
            "red" => RED,
            "amber" => AMBER,
            _ => GREEN,
        };
        let is_focus = focused_tool == Some(tool.equipment_id.as_str());
        let dimmed = (all_exploded && !is_focus) || (options.level == Level::Lot);
        let progress = if is_focus {
            options.explosion.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let tint = if state.is_some_and(|state| state.state == "down") {
            mix([1.0, 1.0, 1.0], light_color, 0.55)
        } else if is_focus {
            mix([1.0, 1.0, 1.0], options.theme.accent, 0.22)
        } else {
            [1.0, 1.0, 1.0]
        };
        for part in &tool.parts {
            let offset =
                Vec3::from(part.explode).normalize_or_zero() * manifest.explode_distance * progress;
            let transform = Mat4::from_translation(offset);
            let selected = options
                .selected
                .as_ref()
                .is_some_and(|selected| match selected {
                    Entity::ToolPart {
                        equipment_id,
                        part: selected_part,
                    } => equipment_id == &tool.equipment_id && selected_part == &part.part,
                    Entity::Tool { equipment_id } => equipment_id == &tool.equipment_id,
                    _ => false,
                });
            let exact_part_selected =
                matches!(options.selected, Some(Entity::ToolPart { .. })) && selected;
            let color = if exact_part_selected {
                let accent = options.theme.accent;
                [accent[0], accent[1], accent[2], 1.0]
            } else if selected {
                // The whole tool is chosen: warm it up, but leave the parts'
                // own colours readable so the explosion stays legible.
                let warm = mix(tint, options.theme.accent, 0.22);
                [warm[0], warm[1], warm[2], 1.0]
            } else {
                [tint[0], tint[1], tint[2], 1.0]
            };
            graph.nodes.push(SceneNode {
                entity: Entity::ToolPart {
                    equipment_id: tool.equipment_id.clone(),
                    part: part.part.clone(),
                },
                mesh: Some(part.node.clone()),
                shape: None,
                transform,
                aabb: part.aabb.translated(offset),
                color,
                emissive: if exact_part_selected { 0.45 } else { 0.0 },
                pickable: true,
                dimmed,
            });
        }
        // A small self-lit status cube above the tool: the three-state signal
        // never depends on the body colour alone.
        let top = tool
            .parts
            .iter()
            .map(|part| part.aabb.max[1])
            .fold(0.0_f32, f32::max);
        let anchor = Vec3::from(tool.anchor);
        let light_box = Aabb::new(
            [anchor.x - 0.18, top + 0.30, anchor.z - 0.18],
            [anchor.x + 0.18, top + 0.66, anchor.z + 0.18],
        );
        graph.nodes.push(SceneNode {
            entity: Entity::Tool {
                equipment_id: tool.equipment_id.clone(),
            },
            mesh: None,
            shape: Some(Shape::Cube),
            transform: Mat4::from_translation(light_box.center())
                * Mat4::from_scale(light_box.size() * 0.5),
            aabb: light_box,
            color: [light_color[0], light_color[1], light_color[2], 1.0],
            emissive: 1.0,
            pickable: true,
            dimmed,
        });
        if is_focus {
            graph
                .lines
                .extend(box_lines(light_box.expanded(0.06), options.theme.accent));
        }
    }

    for lot in &floor.lots {
        let placement = foup_placement(
            manifest,
            path,
            floor,
            lot,
            floor.now_ms,
            options.reduce_motion,
        );
        let is_focus = focused_lot == Some(lot.id.as_str());
        let dimmed = match options.level {
            Level::Factory => false,
            Level::Tool => true,
            Level::Lot | Level::Event => !is_focus,
        };
        let tint = lot_color(lot);
        let selected = options
            .selected
            .as_ref()
            .is_some_and(|selected| selected.lot_id() == Some(lot.id.as_str()));
        let color = if selected {
            let warm = mix([tint[0], tint[1], tint[2]], options.theme.accent, 0.5);
            [warm[0], warm[1], warm[2], 1.0]
        } else if is_focus {
            let focused = mix([1.0, 1.0, 1.0], options.theme.accent, 0.35);
            [focused[0], focused[1], focused[2], 1.0]
        } else {
            tint
        };
        let rotation = Mat4::from_rotation_y(placement.yaw);
        let transform = Mat4::from_translation(placement.position) * rotation;
        let mut foup_box = Aabb::ZERO;
        for part in &manifest.templates.foup {
            let aabb = part.aabb.translated(placement.position);
            foup_box = if foup_box == Aabb::ZERO {
                aabb
            } else {
                foup_box.union(aabb)
            };
            graph.nodes.push(SceneNode {
                entity: Entity::Foup {
                    lot_id: lot.id.clone(),
                },
                mesh: Some(part.node.clone()),
                shape: None,
                transform,
                aabb,
                color,
                emissive: if is_focus { 0.35 } else { 0.0 },
                pickable: true,
                dimmed,
            });
        }
        if is_focus {
            graph
                .lines
                .extend(box_lines(foup_box.expanded(0.12), options.theme.accent));
        }

        if options.level == Level::Lot && is_focus {
            for wafer in &lot.wafers {
                // The stack rises out of the open FOUP so every slot is
                // visible: the body is ~1.05 high, slot 1 starts just above it.
                let y = 1.22 + f32::from(i16::try_from(wafer.slot).unwrap_or(1)) * 0.13;
                let position = Vec3::new(placement.position.x, y, placement.position.z);
                let wafer_box = manifest.wafer_body().aabb.translated(position);
                let wafer_color = wafer_color(wafer);
                let selected_wafer = options.selected.as_ref().is_some_and(|selected| {
                    matches!(selected, Entity::Wafer { wafer_id, .. } if wafer_id == &wafer.id)
                });
                let color = if selected_wafer {
                    let accent = options.theme.accent;
                    [accent[0], accent[1], accent[2], 1.0]
                } else {
                    wafer_color
                };
                graph.nodes.push(SceneNode {
                    entity: Entity::Wafer {
                        lot_id: lot.id.clone(),
                        wafer_id: wafer.id.clone(),
                    },
                    mesh: Some(manifest.wafer_body().node.clone()),
                    shape: None,
                    transform: Mat4::from_translation(position),
                    aabb: wafer_box,
                    color,
                    emissive: if selected_wafer { 0.4 } else { 0.08 },
                    pickable: true,
                    dimmed: false,
                });
                if selected_wafer {
                    graph
                        .lines
                        .extend(box_lines(wafer_box.expanded(0.05), options.theme.accent));
                }
            }
        }
    }

    // The derived channel's backlog is shown where the material enters: an
    // amber beacon on the ring, the same colour as the outbox panel.
    if options.derive_alert
        && let Some(entry) = ring.first().copied()
    {
        graph.lines.push(SceneLine {
            a: entry,
            b: Vec3::new(entry.x, entry.y + 6.0, entry.z),
            color: [0.96, 0.62, 0.04, 1.0],
        });
        graph.nodes.push(SceneNode {
            entity: Entity::Rail,
            mesh: None,
            shape: Some(Shape::Cube),
            transform: Mat4::from_translation(Vec3::new(entry.x, entry.y + 6.0, entry.z))
                * Mat4::from_scale(Vec3::splat(0.18)),
            aabb: Aabb::new(
                [entry.x - 0.18, entry.y + 5.82, entry.z - 0.18],
                [entry.x + 0.18, entry.y + 6.18, entry.z + 0.18],
            ),
            color: [0.96, 0.62, 0.04, 1.0],
            emissive: 1.0,
            pickable: false,
            dimmed: false,
        });
    }

    // Level 3: a beacon over the entity the event refers to, so an event's
    // subject is locatable in the 3D scene and not only in the side panel.
    if options.level == Level::Event
        && let Some(focus) = &options.focus
        && let Some(aabb) = graph.bounds_of(focus)
    {
        {
            let center = aabb.center();
            let top = aabb.max[1] + 0.4;
            graph.lines.push(SceneLine {
                a: Vec3::new(center.x, top, center.z),
                b: Vec3::new(center.x, top + 5.0, center.z),
                color: [
                    options.theme.accent[0],
                    options.theme.accent[1],
                    options.theme.accent[2],
                    1.0,
                ],
            });
            graph.nodes.push(SceneNode {
                entity: focus.clone(),
                mesh: None,
                shape: Some(Shape::Cube),
                transform: Mat4::from_translation(Vec3::new(center.x, top + 5.0, center.z))
                    * Mat4::from_scale(Vec3::splat(0.16)),
                aabb: Aabb::new(
                    [center.x - 0.16, top + 4.84, center.z - 0.16],
                    [center.x + 0.16, top + 5.16, center.z + 0.16],
                ),
                color: [
                    options.theme.accent[0],
                    options.theme.accent[1],
                    options.theme.accent[2],
                    1.0,
                ],
                emissive: 1.0,
                pickable: false,
                dimmed: false,
            });
        }
    }

    graph
}

impl SceneGraph {
    /// Nearest pickable node under the ray, in world space.
    #[must_use]
    pub fn pick(&self, ray: &Ray, padding: f32) -> Option<PickHit> {
        let mut best: Option<PickHit> = None;
        for (index, node) in self.nodes.iter().enumerate() {
            if !node.pickable {
                continue;
            }
            if let Some(distance) = ray_aabb(ray, node.aabb.expanded(padding))
                && best.as_ref().is_none_or(|hit| distance < hit.distance)
            {
                best = Some(PickHit {
                    node: index,
                    entity: node.entity.clone(),
                    distance,
                    point: ray.origin + ray.direction * distance,
                });
            }
        }
        best
    }

    /// The union of the boxes of every node for `entity`.
    #[must_use]
    pub fn bounds_of(&self, entity: &Entity) -> Option<Aabb> {
        self.nodes
            .iter()
            .filter(|node| &node.entity == entity)
            .map(|node| node.aabb)
            .reduce(Aabb::union)
    }

    /// The bounds of a tool, all its parts included.
    #[must_use]
    pub fn tool_bounds(&self, equipment_id: &str) -> Option<Aabb> {
        self.nodes
            .iter()
            .filter(|node| node.entity.tool_id() == Some(equipment_id))
            .map(|node| node.aabb)
            .reduce(Aabb::union)
    }

    #[must_use]
    pub fn lot_bounds(&self, lot_id: &str) -> Option<Aabb> {
        self.nodes
            .iter()
            .filter(|node| node.entity.lot_id() == Some(lot_id))
            .map(|node| node.aabb)
            .reduce(Aabb::union)
    }
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] * (1.0 - t) + b[0] * t,
        a[1] * (1.0 - t) + b[1] * t,
        a[2] * (1.0 - t) + b[2] * t,
    ]
}

fn lot_color(lot: &LotState) -> [f32; 4] {
    let rgb = match lot.state.as_str() {
        "completed" => GREEN,
        "processing" => BLUE,
        "on_hold" => AMBER,
        "split" | "merged" => [0.55, 0.55, 0.62],
        _ => SLATE,
    };
    [rgb[0], rgb[1], rgb[2], 1.0]
}

/// The 25-slot wafer grid is coloured by state, and scrapped wafers by the
/// defect class parsed from `wafer.defect_info` — the same string the API
/// returns, with no extra defect database.
fn wafer_color(wafer: &WaferState) -> [f32; 4] {
    if wafer.state == "scrapped" {
        let rgb = match defect_class(wafer.defect_info.as_deref()) {
            Some("scratch") => [0.98, 0.35, 0.20],
            Some("particle") => [0.92, 0.16, 0.55],
            Some("resist") => [0.72, 0.35, 0.95],
            Some("pattern") => [0.95, 0.75, 0.16],
            _ => RED,
        };
        return [rgb[0], rgb[1], rgb[2], 1.0];
    }
    let rgb = match wafer.state.as_str() {
        "done" => GREEN,
        "processing" => BLUE,
        "queued" | "waiting" => SLATE,
        _ => [0.38, 0.42, 0.48],
    };
    if wafer.rework_count > 0 {
        let amber = mix(rgb, AMBER, 0.45);
        return [amber[0], amber[1], amber[2], 1.0];
    }
    [rgb[0], rgb[1], rgb[2], 1.0]
}

/// The domain's defect taxonomy, shared with the analytics so the 3D colours
/// and the quality panel cannot disagree about a wafer.
#[must_use]
pub fn defect_class(info: Option<&str>) -> Option<&'static str> {
    info.map(oxide_core::analytics::defect_class)
}

fn box_lines(box_: Aabb, color: [f32; 3]) -> Vec<SceneLine> {
    let c = box_.corners();
    let edges = [
        (0, 1),
        (1, 2),
        (2, 3),
        (3, 0),
        (4, 5),
        (5, 6),
        (6, 7),
        (7, 4),
        (0, 4),
        (1, 5),
        (2, 6),
        (3, 7),
    ];
    edges
        .iter()
        .map(|(a, b)| SceneLine {
            a: c[*a],
            b: c[*b],
            color: [color[0], color[1], color[2], 0.9],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::MANIFEST;

    fn fixture() -> (FabManifest, FloorState, AmhsPath) {
        let manifest = FabManifest::from_json(MANIFEST).unwrap();
        let path = AmhsPath::new(&manifest);
        let floor = FloorState {
            now_ms: 10_000,
            running: true,
            speed: 10.0,
            steps: Vec::new(),
            tools: vec![
                crate::state::ToolState {
                    id: "CLN-01".to_owned(),
                    kind: "clean".to_owned(),
                    state: "running".to_owned(),
                    light: "green".to_owned(),
                    current_lot: Some("LOT-0001".to_owned()),
                    current_step: Some(1),
                    lots_processed: 0,
                },
                crate::state::ToolState {
                    id: "LIT-01".to_owned(),
                    kind: "litho".to_owned(),
                    state: "down".to_owned(),
                    light: "red".to_owned(),
                    ..crate::state::ToolState::default()
                },
            ],
            lots: vec![LotState {
                id: "LOT-0001".to_owned(),
                state: "processing".to_owned(),
                current_step: 1,
                priority: 5,
                wafer_count: 3,
                done_wafers: 0,
                step_since_ms: 9_000,
                wafers: vec![
                    WaferState {
                        id: "LOT-0001-W01".to_owned(),
                        slot: 1,
                        state: "processing".to_owned(),
                        ..WaferState::default()
                    },
                    WaferState {
                        id: "LOT-0001-W02".to_owned(),
                        slot: 2,
                        state: "scrapped".to_owned(),
                        defect_info: Some("scratch at edge".to_owned()),
                        ..WaferState::default()
                    },
                    WaferState {
                        id: "LOT-0001-W03".to_owned(),
                        slot: 3,
                        state: "queued".to_owned(),
                        ..WaferState::default()
                    },
                ],
                ..LotState::default()
            }],
        };
        (manifest, floor, path)
    }

    #[test]
    fn the_factory_level_draws_six_tools_and_their_parts() {
        let (manifest, floor, path) = fixture();
        let graph = build(&manifest, &floor, &SceneOptions::default(), &path);
        let tools: Vec<&SceneNode> = graph
            .nodes
            .iter()
            .filter(|node| matches!(node.entity, Entity::ToolPart { .. }))
            .collect();
        assert_eq!(tools.len(), manifest.tools.len() * 5);
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| matches!(node.entity, Entity::Tool { .. }))
                .count(),
            manifest.tools.len()
        );
    }

    #[test]
    fn a_down_tool_is_redder_than_a_running_one() {
        let (manifest, floor, path) = fixture();
        let graph = build(&manifest, &floor, &SceneOptions::default(), &path);
        let color_of = |tool: &str| {
            graph
                .nodes
                .iter()
                .find(|node| {
                    matches!(&node.entity, Entity::Tool { equipment_id } if equipment_id == tool)
                })
                .unwrap()
                .color
        };
        let down = color_of("LIT-01");
        let running = color_of("CLN-01");
        assert!(down[0] > down[1] && down[0] > down[2], "{down:?}");
        assert!(running[1] > running[0], "{running:?}");
    }

    #[test]
    fn exploding_a_tool_moves_its_parts_but_not_its_neighbours() {
        let (manifest, floor, path) = fixture();
        let focus = Entity::Tool {
            equipment_id: "CLN-01".to_owned(),
        };
        let assembled = build(
            &manifest,
            &floor,
            &SceneOptions {
                level: Level::Tool,
                focus: Some(focus.clone()),
                ..SceneOptions::default()
            },
            &path,
        );
        let exploded = build(
            &manifest,
            &floor,
            &SceneOptions {
                level: Level::Tool,
                focus: Some(focus.clone()),
                explosion: 1.0,
                ..SceneOptions::default()
            },
            &path,
        );
        let chamber = |graph: &SceneGraph| {
            graph
                .nodes
                .iter()
                .find(|node| {
                    matches!(&node.entity, Entity::ToolPart { equipment_id, part }
                        if equipment_id == "CLN-01" && part == "chamber")
                })
                .unwrap()
                .aabb
        };
        assert!(chamber(&exploded).min[1] > chamber(&assembled).min[1]);
        let neighbour = |graph: &SceneGraph| {
            graph
                .nodes
                .iter()
                .find(|node| {
                    matches!(&node.entity, Entity::ToolPart { equipment_id, part }
                        if equipment_id == "LIT-01" && part == "chamber")
                })
                .unwrap()
                .aabb
        };
        assert_eq!(neighbour(&exploded), neighbour(&assembled));
    }

    #[test]
    fn clicking_a_part_picks_the_tool_and_clicking_the_foup_picks_the_lot() {
        let (manifest, floor, path) = fixture();
        let graph = build(&manifest, &floor, &SceneOptions::default(), &path);
        let body = graph
            .nodes
            .iter()
            .find(|node| {
                matches!(&node.entity, Entity::ToolPart { equipment_id, part }
                    if equipment_id == "CLN-01" && part == "body")
            })
            .unwrap();
        let center = body.aabb.center();
        let ray = Ray {
            origin: center + Vec3::new(0.0, 0.0, 20.0),
            direction: Vec3::new(0.0, 0.0, -1.0),
        };
        let hit = graph.pick(&ray, 0.0).unwrap();
        assert_eq!(
            hit.entity,
            Entity::ToolPart {
                equipment_id: "CLN-01".to_owned(),
                part: "body".to_owned()
            }
        );

        let foup = graph
            .nodes
            .iter()
            .find(|node| matches!(node.entity, Entity::Foup { .. }))
            .unwrap();
        let center = foup.aabb.center();
        let ray = Ray {
            origin: center + Vec3::new(0.0, 10.0, 0.0),
            direction: Vec3::new(0.0, -1.0, 0.0),
        };
        let hit = graph.pick(&ray, 0.0).unwrap();
        assert_eq!(
            hit.entity,
            Entity::Foup {
                lot_id: "LOT-0001".to_owned()
            }
        );
    }

    #[test]
    fn level_two_shows_the_wafer_stack_and_its_defect_colour() {
        let (manifest, floor, path) = fixture();
        let graph = build(
            &manifest,
            &floor,
            &SceneOptions {
                level: Level::Lot,
                focus: Some(Entity::Foup {
                    lot_id: "LOT-0001".to_owned(),
                }),
                ..SceneOptions::default()
            },
            &path,
        );
        let wafers: Vec<&SceneNode> = graph
            .nodes
            .iter()
            .filter(|node| matches!(node.entity, Entity::Wafer { .. }))
            .collect();
        assert_eq!(wafers.len(), 3);
        let scrapped = wafers
            .iter()
            .find(|node| {
                matches!(&node.entity, Entity::Wafer { wafer_id, .. } if wafer_id == "LOT-0001-W02")
            })
            .unwrap();
        // The defect class is "scratch", the orange-red class.
        assert!(scrapped.color[0] > 0.9 && scrapped.color[1] < 0.5);
        assert!(wafers.iter().all(|node| node.pickable));
    }

    #[test]
    fn a_derived_channel_backlog_draws_an_amber_beacon() {
        let (manifest, floor, path) = fixture();
        let graph = build(
            &manifest,
            &floor,
            &SceneOptions {
                derive_alert: true,
                ..SceneOptions::default()
            },
            &path,
        );
        assert!(
            graph.lines.iter().any(|line| line.color[0] > 0.9
                && line.color[1] < 0.7
                && line.b.y - line.a.y > 5.0),
            "an amber beacon marks the entry when the derived channel is behind"
        );
    }

    #[test]
    fn level_three_draws_a_beacon_over_the_event_subject() {
        let (manifest, floor, path) = fixture();
        let graph = build(
            &manifest,
            &floor,
            &SceneOptions {
                level: Level::Event,
                focus: Some(Entity::Tool {
                    equipment_id: "CLN-01".to_owned(),
                }),
                ..SceneOptions::default()
            },
            &path,
        );
        assert_ne!(graph.lines.len(), 0);
        assert!(graph.lines.iter().any(|line| line.b.y - line.a.y > 4.0));
    }

    #[test]
    fn every_pickable_node_has_a_finite_box() {
        let (manifest, floor, path) = fixture();
        let graph = build(&manifest, &floor, &SceneOptions::default(), &path);
        for node in graph.nodes.iter().filter(|node| node.pickable) {
            assert!(
                node.aabb.size().length().is_finite() && node.aabb.size().length() > 0.0,
                "{:?}",
                node.entity
            );
        }
    }

    #[test]
    fn defect_notes_map_to_classes() {
        assert_eq!(defect_class(Some("scratch at edge")), Some("scratch"));
        assert_eq!(defect_class(Some("particle cluster")), Some("particle"));
        assert_eq!(defect_class(Some("CD drift")), Some("pattern"));
        assert_eq!(defect_class(None), None);
    }
}
