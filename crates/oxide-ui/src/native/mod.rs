//! The `oxide.view3d` native component: the wgpu viewport, hosted natively.
//!
//! The component draws frames that `engine` renders; it owns no camera and no
//! domain state. Input stays at the nsh layer: the view is transparent to the
//! pointer on every platform, so the element's own `on_mouse_*` handlers hear
//! the drag, the wheel and the click, and the app's camera moves.

pub const NAME: &str = "oxide.view3d";

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "windows")]
pub mod windows;
