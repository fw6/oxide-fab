//! The factory assets the app renders.
//!
//! The GLB and its manifest ship with this crate, in `assets/fab/`, beside the
//! code that loads them: `tools/blender/build_fab.py` writes them here and the
//! two constants below are the only place that names them. The repository's
//! other consumers — `oxide-scene`'s manifest test, `oxide-render`'s GLB tests
//! and the `oxide-render-evidence` binary — read this same directory, so there
//! is one copy to keep in sync.
//!
//! nsh's own `assets/` channel cannot carry them: `asset!()` accepts images,
//! SVG and fonts, and the GLB is compiled into the binary rather than shipped
//! as a UI resource. The app ships no images or fonts of its own; `icon.png`
//! and `theme.json` sit beside `nsh.json`, which names them.

/// The demo factory, as Blender exported it.
pub const GLB: &[u8] = include_bytes!("../assets/fab/oxide-fab.glb");

/// The manifest that names every node, part, picking box and explosion
/// direction.
pub const MANIFEST: &str = include_str!("../assets/fab/fab-manifest.json");
