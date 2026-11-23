//! The 3D scene of oxide-fab, independent of any GPU or UI framework.
//!
//! - [`manifest`]: the contract the offline Blender export writes next to the
//!   GLB (node ↔ equipment/part, picking boxes, explosion directions).
//! - [`state`]: the read-only floor state the scene draws.
//! - [`layout`]: the AMHS ring and where each FOUP stands.
//! - [`graph`]: the draw list and the picker.
//! - [`camera`]: the orbit camera, both projections and the picking ray.
//!
//! `oxide-render` consumes the graph; `oxide-ui` owns the camera and the level.
//! Nothing here allocates GPU resources, so every rule is unit-testable.

pub mod camera;
pub mod graph;
pub mod layout;
pub mod manifest;
pub mod state;

pub use camera::{Camera, Projection, Ray};
pub use graph::{
    Entity, Level, SceneGraph, SceneLine, SceneNode, SceneOptions, SceneTheme, Shape, build,
    defect_class,
};
pub use layout::{AmhsPath, FoupPlacement, TRANSIT_MS, foup_placement, lot_moving};
pub use manifest::{Aabb, BandSpec, FabManifest, ManifestError, PartSpec, StaticSpec, ToolSpec};
pub use state::{FloorState, LotState, StepState, ToolState, WaferState};
