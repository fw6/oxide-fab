//! The wgpu renderer of the oxide-fab 3D workbench.
//!
//! - [`glb`] loads the committed GLB, bakes node transforms and validates it
//!   against the manifest.
//! - [`gpu`] creates the device, preferring a software adapter when asked.
//! - [`renderer`] draws a [`oxide_scene::SceneGraph`] with an orbit camera and
//!   reads the frame back, so the same call paints CI's evidence and the app's
//!   viewport.
//!
//! The renderer has no windowing code: presentation belongs to the native
//! component in `oxide-ui`, and the headless CLI uses the frames directly.

pub mod glb;
pub mod gpu;
pub mod renderer;

pub use glb::{Mesh, Model, Vertex};
pub use gpu::{AdapterReport, Gpu};
pub use renderer::{Frame, Renderer, RendererOptions};

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("glTF: {0}")]
    Glb(String),
    #[error("the GLB lacks manifest nodes: {0}")]
    MissingNodes(String),
    #[error("no GPU adapter: {0}")]
    Adapter(#[from] wgpu::RequestAdapterError),
    #[error("GPU device: {0}")]
    Device(String),
    #[error("image: {0}")]
    Image(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

impl From<wgpu::RequestDeviceError> for RenderError {
    fn from(error: wgpu::RequestDeviceError) -> Self {
        RenderError::Device(error.to_string())
    }
}
