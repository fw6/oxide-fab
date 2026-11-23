//! The 3D viewport, as TSX drives it.
//!
//! The view itself is a native component (`oxide.view3d`) that blits frames the
//! render thread produces; TSX places it, sends its `sink` and `frame` props,
//! and calls this class for every pointer gesture. Camera and scene live behind
//! one process-wide slot so a TSX hot reload keeps the view where the user left
//! it instead of dropping the GPU and re-framing the factory.

use std::sync::{Arc, Mutex, OnceLock};

use glam::Vec2;
use nsh_core::prelude::*;
use oxide_scene::Camera;

use crate::assets;
use crate::engine::RenderEngine;
use crate::frames;
use crate::scene::{self, EntityInput, SceneInput};

/// What the viewport keeps between frames and between TSX reloads.
struct Shared {
    sink: u64,
    engine: Option<RenderEngine>,
    scene: Option<SceneInput>,
    camera: Camera,
    size: (f64, f64),
}

static SHARED: Mutex<Option<Shared>> = Mutex::new(None);
static SLOT: OnceLock<(u64, Arc<Mutex<frames::Frame>>)> = OnceLock::new();
static ENGINE: OnceLock<RenderEngine> = OnceLock::new();

/// The 3D view: scene in, camera and picks out.
///
/// A module of functions, not a class: the viewport is one per process, and a
/// module can be stood in for in TSX tests (`open`'s `modules`) where a class
/// constructor cannot.
#[module]
#[derive(Debug)]
pub struct Viewport;

/// Runs `f` against the viewport, opening it on first use.
fn with<R>(f: impl FnOnce(&mut Shared) -> R) -> R {
    let mut guard = SHARED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let shared = guard.get_or_insert_with(open);
    f(shared)
}

#[module]
impl Viewport {
    /// The frame slot to give the `oxide.view3d` element.
    #[must_use]
    pub fn sink() -> u64 {
        with(|shared| shared.sink)
    }

    /// Replaces everything the view draws.
    pub fn set_scene(scene: SceneInput) {
        with(|shared| shared.scene = Some(scene));
    }

    /// The view's size in points, as its `onLayout` reports it.
    pub fn set_size(width: f64, height: f64) {
        if width > 0.0 && height > 0.0 {
            with(|shared| shared.size = (width, height));
        }
    }

    /// Renders the current scene and camera; the frame version that is ready.
    ///
    /// TSX calls this about every 50 ms and feeds the result to the element's
    /// `frame` prop, which is what makes the native view blit a new frame.
    #[must_use]
    pub fn tick() -> u64 {
        with(|shared| {
            let Some(engine) = &shared.engine else {
                return 0;
            };
            let (width, height) = shared.size;
            if width < 240.0 || height < 160.0 {
                return engine.finished();
            }
            let Some(scene) = &shared.scene else {
                return engine.finished();
            };
            let Some(graph) = scene::graph(scene) else {
                return engine.finished();
            };
            // Two device pixels per point, capped: crisp on Retina without
            // letting a 4K window turn every frame into a 30 MB readback.
            let pixel_width = (width * 2.0).clamp(320.0, 1600.0) as u32;
            let pixel_height = (height * 2.0).clamp(200.0, 1000.0) as u32;
            engine.submit(graph, shared.camera, pixel_width, pixel_height);
            engine.finished()
        })
    }

    /// The GPU the renderer opened, for the status line.
    #[must_use]
    pub fn adapter() -> String {
        with(|shared| {
            shared
                .engine
                .as_ref()
                .map_or_else(String::new, RenderEngine::adapter)
        })
    }

    /// Why the 3D view is off, when it is.
    #[must_use]
    pub fn error() -> Option<String> {
        with(|shared| shared.engine.as_ref().and_then(RenderEngine::error))
    }

    /// Orbit by a pointer delta in points.
    pub fn orbit(dx: f64, dy: f64) {
        with(|shared| shared.camera.orbit(dx as f32, dy as f32));
    }

    /// Pan by a pointer delta in points.
    pub fn pan(dx: f64, dy: f64) {
        with(|shared| {
            let height = shared.size.1 as f32;
            shared.camera.pan(dx as f32, dy as f32, height);
        });
    }

    /// Zoom by wheel notches; positive zooms in.
    pub fn zoom(notches: f64) {
        with(|shared| shared.camera.zoom(notches as f32));
    }

    /// Switches perspective and orthographic; the projection now shown.
    #[must_use]
    pub fn toggle_projection() -> String {
        with(|shared| {
            shared.camera.toggle_projection();
            match shared.camera.projection {
                oxide_scene::Projection::Perspective => "perspective".to_owned(),
                oxide_scene::Projection::Orthographic => "orthographic".to_owned(),
            }
        })
    }

    /// The factory overview: default camera, nothing framed.
    pub fn reset_camera() {
        with(|shared| shared.camera = Camera::default());
    }

    /// Frames an entity (or the whole floor when `entity` is absent).
    pub fn focus(entity: Option<EntityInput>) {
        with(|shared| {
            let Some(scene) = &shared.scene else {
                return;
            };
            let Some(graph) = scene::graph(scene) else {
                return;
            };
            let entity = entity.and_then(|input| input.to_entity());
            scene::frame(&graph, &mut shared.camera, entity.as_ref());
        });
    }

    /// What is under a point of the view, through the camera it was drawn with.
    #[must_use]
    pub fn pick(x: f64, y: f64) -> Option<EntityInput> {
        with(|shared| {
            let (width, height) = shared.size;
            if width < 1.0 || height < 1.0 {
                return None;
            }
            let scene = shared.scene.as_ref()?;
            let graph = scene::graph(scene)?;
            let ndc = Vec2::new(
                (2.0 * x / width - 1.0) as f32,
                (1.0 - 2.0 * y / height) as f32,
            );
            let ray = shared.camera.ray(ndc, (width / height) as f32);
            graph
                .pick(&ray, 0.08)
                .map(|hit| EntityInput::from_entity(&hit.entity))
        })
    }

    /// The scene as rendered, as JSON: what the export button writes.
    ///
    /// The camera and the draw list live here, so the export is assembled here
    /// rather than from the panels' snapshot.
    #[must_use]
    pub fn describe() -> String {
        with(|shared| {
            let Some(scene) = &shared.scene else {
                return "{}".to_owned();
            };
            let Some(graph) = scene::graph(scene) else {
                return "{}".to_owned();
            };
            let camera = &shared.camera;
            let projection = match camera.projection {
                oxide_scene::Projection::Perspective => "透视",
                oxide_scene::Projection::Orthographic => "正交",
            };
            let nodes: Vec<serde_json::Value> = graph
                .nodes
                .iter()
                .map(|node| {
                    serde_json::json!({
                        "entity": node.entity.key(),
                        "mesh": node.mesh,
                        "aabb": { "min": node.aabb.min, "max": node.aabb.max },
                        "pickable": node.pickable,
                    })
                })
                .collect();
            serde_json::json!({
                "exported_at_ms": scene.now_ms,
                "level": oxide_scene::Level::from(scene.level).label(),
                "focus": scene.focus.as_ref().and_then(EntityInput::to_entity).map(|entity| entity.key()),
                "selected": scene.selected.as_ref().and_then(EntityInput::to_entity).map(|entity| entity.key()),
                "camera": {
                    "target": [camera.target.x, camera.target.y, camera.target.z],
                    "yaw": camera.yaw,
                    "pitch": camera.pitch,
                    "distance": camera.distance,
                    "projection": projection,
                },
                "nodes": nodes,
                "lines": graph.lines.len(),
                "source": "PostgreSQL lot_event 投影 + manifest 场景图；相机为客户端状态",
            })
            .to_string()
        })
    }
}

/// The first time the class is made: one frame slot and one render thread,
/// both process-wide.
fn open() -> Shared {
    let disabled = std::env::var("OXIDE_RENDER_DISABLE").is_ok_and(|value| value == "1");
    if disabled {
        return Shared {
            sink: 0,
            engine: None,
            scene: None,
            camera: Camera::default(),
            size: (0.0, 0.0),
        };
    }
    let (sink, slot) = SLOT.get_or_init(frames::create);
    let (sink, slot) = (*sink, Arc::clone(slot));
    let engine = ENGINE.get_or_init(|| RenderEngine::start(assets::GLB, Arc::clone(&slot)));
    Shared {
        sink,
        engine: Some(engine.clone()),
        scene: None,
        camera: Camera::default(),
        size: (0.0, 0.0),
    }
}
