//! The Rust side of the oxide-fab workbench.
//!
//! The interface is TSX (`App.tsx`): panels, polling and interaction hot-reload
//! in roughly 120 ms, which is why they moved out of Rust. What stays here is
//! what needs native code: the offscreen wgpu renderer, the scene graph and
//! camera, the `oxide.view3d` native view the frames land in, and the two
//! desktop actions.
//!
//! ```text
//! PostgreSQL ──/api/snapshot, /api/events──▶ App.tsx (signals)
//!                                                │
//!                          level, focus, selection, snapshot
//!                                                ▼
//!                                     Viewport (this crate)
//!                                                │
//!                          oxide-scene ─▶ oxide-render (wgpu)
//!                                                │ frames
//!                                                ▼
//!                                     oxide.view3d native view
//! ```

pub mod assets;
pub mod desktop;
pub mod engine;
pub mod frames;
pub mod native;
pub mod palette;
pub mod scene;
pub mod viewport;

use nsh_core::prelude::*;

/// The native components this crate registers; the development host and the
/// packaged app call these before loading the app.
#[cfg(target_os = "macos")]
#[native_components(macos)]
fn oxide_components_macos(components: &nsh_platform_macos::Components) {
    native::macos::register(components);
}

#[cfg(target_os = "linux")]
#[native_components(linux)]
fn oxide_components_linux(components: &nsh_platform_linux::Components) {
    native::linux::register(components);
}

#[cfg(target_os = "windows")]
#[native_components(windows)]
fn oxide_components_windows(components: &nsh_platform_windows::Components) {
    native::windows::register(components);
}

/// What TSX imports from `oxide-ui/native`: the viewport and the desktop
/// actions it cannot reach itself.
#[native_module("oxide-ui/native")]
fn native_registry(registry: &mut Registry) {
    registry
        .module::<viewport::Viewport>()
        .module::<desktop::Desktop>();
}
