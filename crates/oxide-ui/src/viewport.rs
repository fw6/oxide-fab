//! The 3D viewport: the native element, its input and the render pump.
//!
//! Input is nsh's own: the native view lets the pointer through, so the element
//! hears drags, the wheel and presses. Picking is CPU-side with the same camera
//! the renderer uses, so a click lands on the part that is actually under the
//! pixel.

use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use glam::Vec2;
use nsh_core::prelude::*;
use oxide_scene::Camera;

use crate::engine::RenderEngine;
use crate::frames;
use crate::native::NAME;
use crate::scene_glue;
use crate::state::{Drag, Ui};

const GLB: &[u8] = include_bytes!("../../../assets/fab/oxide-fab.glb");

static SLOT: OnceLock<(u64, Arc<Mutex<frames::Frame>>)> = OnceLock::new();
static ENGINE: OnceLock<RenderEngine> = OnceLock::new();

/// The frame slot's id, created once per app.
pub fn init() -> u64 {
    if std::env::var("OXIDE_RENDER_DISABLE").is_ok_and(|value| value == "1") {
        return 0;
    }
    let (id, slot) = SLOT.get_or_init(|| {
        let (id, slot) = frames::create();
        (id, slot)
    });
    ENGINE.get_or_init(|| RenderEngine::start(GLB, Arc::clone(slot)));
    *id
}

fn engine() -> Option<&'static RenderEngine> {
    ENGINE.get()
}

/// The pump: rebuild the graph, submit it, publish the finished frame.
pub fn start_render_loop(ui: Ui) {
    spawn(async move {
        loop {
            timer(Duration::from_millis(50)).await;
            let Some(engine) = engine() else { continue };
            if let Some(error) = engine.error() {
                ui.render_error.set(Some(error));
            }
            let adapter = engine.adapter();
            if !adapter.is_empty() {
                ui.adapter.set(adapter);
            }
            let (width, height) = ui.view_size.get_untracked();
            if width >= 240.0
                && height >= 160.0
                && let Some(graph) = scene_glue::graph(ui)
            {
                // Two device pixels per point, capped: crisp on Retina without
                // letting a 4K window turn every frame into a 30 MB readback.
                let pixel_width = (width * 2.0).clamp(320.0, 1600.0) as u32;
                let pixel_height = (height * 2.0).clamp(200.0, 1000.0) as u32;
                engine.submit(graph, ui.camera.get_untracked(), pixel_width, pixel_height);
            }
            let finished = engine.finished();
            if finished != ui.frame.get_untracked() {
                ui.frame.set(finished);
            }
        }
    });
}

/// The viewport element.
#[must_use]
pub fn viewport(ui: Ui) -> impl IntoElement {
    let frame = ui.frame;
    let sink = ui.sink;
    view()
        .flex_1()
        .relative()
        .rounded(px(10.0))
        .overflow_hidden()
        .border_1()
        .border_color(move || crate::palette::Palette::color(ui.palette().border))
        .bg(move || crate::palette::Palette::color(ui.palette().scene))
        .child(
            native(NAME)
                .prop("sink", serde_json::json!(sink))
                .prop("frame", move || serde_json::json!(frame.get()))
                .on_layout(move |bounds: &Bounds<Points>| {
                    let size = (bounds.size.width.0, bounds.size.height.0);
                    if size.0 > 0.0 && size.1 > 0.0 && ui.view_size.get_untracked() != size {
                        ui.view_size.set(size);
                    }
                })
                .on_mouse_move(move |event: &MouseEvent| drag(ui, event))
                .on_mouse_up(move |event: &MouseEvent| release(ui, event))
                .on_wheel(move |event: &WheelEvent| {
                    let delta = event.delta.points(Points(16.0));
                    zoom(ui, -f64::from(delta.y.0));
                })
                .on_native_event(move |event: &NativeEvent| {
                    if &*event.name == "wheel" {
                        let dy = event.data["dy"].as_f64().unwrap_or(0.0);
                        let notches = event.data["notches_y"].as_f64().unwrap_or(0.0);
                        zoom(ui, -(dy / 40.0) - notches * 1.5);
                    }
                })
                .flex_1(),
        )
}

fn drag(ui: Ui, event: &MouseEvent) {
    let x = event.position.x.0;
    let y = event.position.y.0;
    let mut drag = ui.drag.get_untracked();
    if event.pressed_button.is_some() {
        if drag.active {
            let dx = x - drag.last.0;
            let dy = y - drag.last.1;
            drag.moved += (dx * dx + dy * dy).sqrt();
            drag.last = (x, y);
            if dx.abs() + dy.abs() > 0.0 {
                let mut camera = ui.camera.get_untracked();
                if event.modifiers.shift {
                    camera.pan(dx, dy, ui.view_size.get_untracked().1);
                } else {
                    camera.orbit(dx, dy);
                }
                ui.camera.set(camera);
            }
        } else {
            drag = Drag {
                active: true,
                start: (x, y),
                last: (x, y),
                moved: 0.0,
            };
        }
    } else {
        drag.active = false;
    }
    if ui.drag.get_untracked() != drag {
        ui.drag.set(drag);
    }
}

fn release(ui: Ui, event: &MouseEvent) {
    let mut drag = ui.drag.get_untracked();
    let was_click = drag.active && drag.moved < 5.0 && event.button == Some(MouseButton::Left);
    drag.active = false;
    ui.drag.set(drag);
    if was_click {
        pick(ui, event.position.x.0, event.position.y.0);
    }
}

fn zoom(ui: Ui, notches: f64) {
    let mut camera = ui.camera.get_untracked();
    camera.zoom(notches as f32);
    ui.camera.set(camera);
}

/// A click at a point in the view: the same ray the renderer drew with.
fn pick(ui: Ui, x: f32, y: f32) {
    let (width, height) = ui.view_size.get_untracked();
    if width < 1.0 || height < 1.0 {
        return;
    }
    let ndc = Vec2::new(2.0 * x / width - 1.0, 1.0 - 2.0 * y / height);
    let camera = ui.camera.get_untracked();
    let ray = camera.ray(ndc, width / height);
    let Some(graph) = scene_glue::graph(ui) else {
        return;
    };
    match graph.pick(&ray, 0.08) {
        Some(hit) => scene_glue::drill(ui, hit.entity),
        None => ui.select(None),
    }
}

/// Frame the whole factory.
pub fn fit(ui: Ui) {
    ui.level.set(oxide_scene::Level::Factory);
    ui.focus.set(None);
    ui.trail.set(Vec::new());
    let camera = Camera::default();
    ui.camera.set(camera);
}

/// Whether the viewport has a renderer at all (the panels show the reason).
#[must_use]
pub fn available() -> bool {
    engine().is_some_and(|engine| engine.error().is_none())
}
