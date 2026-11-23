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
    /// The last state handed to the render thread, for idle ticks.
    last: Option<LastFrame>,
}

/// The state the last frame was drawn from.
///
/// The view renders on demand: a tick submits only when one of these differs
/// from the last accepted frame, or when a gliding FOUP's clock advances.
struct LastFrame {
    scene: SceneInput,
    camera: Camera,
    width: u32,
    height: u32,
    /// Whether a FOUP was gliding, so the frame that settles it is drawn too.
    animating: bool,
}

impl LastFrame {
    /// Whether the next frame must be drawn.
    ///
    /// A clock tick only changes the picture while a FOUP is in transit; any
    /// other difference — scene, camera, device-pixel size — always does.
    fn needs_frame(
        &self,
        scene: &SceneInput,
        camera: &Camera,
        width: u32,
        height: u32,
        animating: bool,
    ) -> bool {
        if self.width != width
            || self.height != height
            || self.camera != *camera
            || !self.scene.same_view(scene)
        {
            return true;
        }
        // `self.animating` keeps the settle frame: the clock moved past the
        // window, the FOUP stops, and that last position must be drawn.
        (animating || self.animating) && self.scene.now_ms != scene.now_ms
    }
}

/// The offscreen size for a view `width` × `height` points.
///
/// One scale for both axes keeps the picture's aspect: the native views stretch
/// the frame into the element box, so per-axis clamps would distort it and put
/// picking on a different projection from the one drawn. The scale is capped at
/// two device pixels per point and by a 1.6-megapixel readback budget
/// (1600×1000), which also keeps both sides inside the renderer's `max_side`.
fn pixel_size(width: f64, height: f64) -> (u32, u32) {
    let area = (width * height).max(1.0);
    let longest = width.max(height).max(1.0);
    let scale = (1_600_000.0 / area).sqrt().min(1600.0 / longest).min(2.0);
    (
        (width * scale).round().max(1.0) as u32,
        (height * scale).round().max(1.0) as u32,
    )
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
    /// A tick with nothing new to draw submits nothing: an idle view costs no
    /// GPU render and no readback, and a moving FOUP keeps the frame rate.
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
            // Two device pixels per point at most, capped by the readback
            // budget; both axes share the scale so the view keeps its aspect.
            let (pixel_width, pixel_height) = pixel_size(width, height);
            let animating = scene.animating();
            let dirty = shared.last.as_ref().is_none_or(|last| {
                last.needs_frame(scene, &shared.camera, pixel_width, pixel_height, animating)
            });
            if !dirty {
                return engine.finished();
            }
            let Some(graph) = scene::graph(scene) else {
                return engine.finished();
            };
            if engine.submit(graph, shared.camera, pixel_width, pixel_height) {
                shared.last = Some(LastFrame {
                    scene: scene.clone(),
                    camera: shared.camera,
                    width: pixel_width,
                    height: pixel_height,
                    animating,
                });
            }
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
            // The ray must use the aspect the frame was drawn with, not the
            // element's: the image is stretched into the box by the native view.
            let (pixel_width, pixel_height) = pixel_size(width, height);
            let ray = shared
                .camera
                .ray(ndc, pixel_width as f32 / pixel_height as f32);
            // The manifest owns the pick padding; a bare 0.08 here would make
            // editing it a no-op.
            let padding = crate::engine::manifest().map_or(0.08, |manifest| manifest.pick_padding);
            graph
                .pick(&ray, padding)
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
            last: None,
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
        last: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::LevelInput;

    fn scene(now_ms: i64) -> SceneInput {
        SceneInput {
            now_ms,
            running: true,
            speed: 10.0,
            steps: Vec::new(),
            tools: Vec::new(),
            lots: Vec::new(),
            level: LevelInput::Factory,
            focus: None,
            selected: None,
            explosion: 0.0,
            reduce_motion: false,
            alert: false,
            dark: true,
        }
    }

    fn last_frame(scene: SceneInput) -> LastFrame {
        LastFrame {
            scene,
            camera: Camera::default(),
            width: 800,
            height: 600,
            animating: false,
        }
    }

    #[test]
    fn a_clock_tick_with_nothing_in_transit_needs_no_frame() {
        let last = last_frame(scene(1_000));
        assert!(!last.needs_frame(&scene(1_700), &Camera::default(), 800, 600, false));
    }

    #[test]
    fn camera_size_and_scene_each_need_a_frame() {
        let last = last_frame(scene(1_000));
        let camera = Camera::default();
        let mut turned = camera;
        turned.yaw += 0.1;
        assert!(last.needs_frame(&scene(1_000), &turned, 800, 600, false));
        assert!(last.needs_frame(&scene(1_000), &camera, 801, 600, false));
        let mut stopped = scene(1_000);
        stopped.running = false;
        assert!(last.needs_frame(&stopped, &camera, 800, 600, false));
    }

    #[test]
    fn a_gliding_foup_is_drawn_when_the_clock_moves() {
        let last = last_frame(scene(1_000));
        assert!(last.needs_frame(&scene(1_100), &Camera::default(), 800, 600, true));
        // The same clock is the same picture: an idle tick must not re-render.
        assert!(!last.needs_frame(&scene(1_000), &Camera::default(), 800, 600, true));
    }

    #[test]
    fn the_frame_that_settles_a_moving_foup_is_drawn() {
        let mut last = last_frame(scene(1_000));
        last.animating = true;
        assert!(last.needs_frame(&scene(1_700), &Camera::default(), 800, 600, false));
    }

    #[test]
    fn the_offscreen_size_keeps_the_view_aspect_and_the_budget() {
        for (width, height) in [(800.0, 840.0), (600.0, 700.0), (1400.0, 900.0)] {
            let (pixel_width, pixel_height) = pixel_size(width, height);
            let aspect = f64::from(pixel_width) / f64::from(pixel_height);
            assert!(
                (aspect - width / height).abs() < 0.01,
                "{width}x{height} draws at {pixel_width}x{pixel_height}"
            );
            assert!(u64::from(pixel_width) * u64::from(pixel_height) <= 1_700_000);
            assert!(pixel_width <= 1600 && pixel_height <= 1600);
        }
        // Two device pixels per point at most.
        assert_eq!(pixel_size(300.0, 200.0), (600, 400));
    }
}
