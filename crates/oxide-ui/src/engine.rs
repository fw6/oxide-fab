//! The render thread: wgpu lives here, the UI thread never blocks on a GPU
//! readback.
//!
//! The UI submits a scene graph and a camera at its own pace; the thread keeps
//! the newest job, renders it, and publishes the frame into the slot the native
//! view reads. Dropping a job when the thread is busy is a skipped frame, never
//! a stall.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;

use oxide_render::{Model, RenderError, Renderer, RendererOptions};
use oxide_scene::{Camera, FabManifest, SceneGraph};

use crate::assets;
use crate::frames::Frame;

#[derive(Debug)]
struct Job {
    graph: SceneGraph,
    camera: Camera,
    width: u32,
    height: u32,
    version: u64,
}

#[derive(Debug, Clone)]
pub struct RenderEngine {
    tx: SyncSender<Job>,
    submitted: Arc<AtomicU64>,
    finished: Arc<AtomicU64>,
    adapter: Arc<Mutex<String>>,
    error: Arc<Mutex<Option<String>>>,
}

impl RenderEngine {
    /// Start the thread. The GPU is opened on it, so a slow driver never
    /// delays the first frame of the UI.
    #[must_use]
    pub fn start(glb: &'static [u8], slot: Arc<Mutex<Frame>>) -> Self {
        let (tx, rx) = sync_channel::<Job>(1);
        let submitted = Arc::new(AtomicU64::new(0));
        let finished = Arc::new(AtomicU64::new(0));
        let adapter = Arc::new(Mutex::new("正在打开 GPU…".to_owned()));
        let error = Arc::new(Mutex::new(None));
        let thread_adapter = Arc::clone(&adapter);
        let thread_error = Arc::clone(&error);
        let thread_finished = Arc::clone(&finished);
        let spawned = thread::Builder::new()
            .name("oxide-render".to_owned())
            .spawn(move || {
                let model = match Model::load(glb) {
                    Ok(model) => model,
                    Err(error) => {
                        *thread_error
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            Some(error.to_string());
                        return;
                    }
                };
                let mut renderer = match Renderer::new(model, RendererOptions::default()) {
                    Ok(renderer) => renderer,
                    Err(error) => {
                        *thread_error
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner) =
                            Some(glb_error(&error));
                        return;
                    }
                };
                *thread_adapter
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    renderer.adapter().summary();
                while let Ok(job) = rx.recv() {
                    renderer.resize(job.width, job.height);
                    match renderer.render(&job.graph, &job.camera) {
                        Ok(frame) => {
                            if let Ok(mut slot) = slot.lock() {
                                slot.version = job.version;
                                slot.width = frame.width;
                                slot.height = frame.height;
                                slot.rgba = frame.rgba;
                            }
                            thread_finished.store(job.version, Ordering::Release);
                        }
                        Err(error) => {
                            *thread_error
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                                Some(error.to_string());
                        }
                    }
                }
            });
        if let Err(spawn_error) = spawned {
            *error
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                Some(format!("cannot start the render thread: {spawn_error}"));
        }
        Self {
            tx,
            submitted,
            finished,
            adapter,
            error,
        }
    }

    /// Ask for a frame. A busy thread drops this one and keeps the newer.
    pub fn submit(&self, graph: SceneGraph, camera: Camera, width: u32, height: u32) {
        let version = self.submitted.fetch_add(1, Ordering::Relaxed) + 1;
        let job = Job {
            graph,
            camera,
            width,
            height,
            version,
        };
        // A full channel is a skipped frame; disconnected means the render
        // thread stopped, and `error` already says why.
        let _ = self.tx.try_send(job);
    }

    #[must_use]
    pub fn finished(&self) -> u64 {
        self.finished.load(Ordering::Acquire)
    }

    #[must_use]
    pub fn adapter(&self) -> String {
        self.adapter
            .lock()
            .map_or_else(|_| String::new(), |s| s.clone())
    }

    #[must_use]
    pub fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|error| error.clone())
    }
}

/// A GPU that cannot open is a state the workbench shows, not a crash: the
/// rest of the screen still reads the backend.
fn glb_error(error: &RenderError) -> String {
    match error {
        RenderError::Adapter(_) | RenderError::Device(_) => format!(
            "GPU 不可用：{error}。工作台仍可读取数据，3D 视图停用（软件渲染可设 OXIDE_RENDER_SOFTWARE=1）。"
        ),
        _ => format!("3D 视图初始化失败：{error}"),
    }
}

/// The manifest the app ships with; parsed once, used by the scene builder and
/// the renderer's validation.
pub fn manifest() -> Option<&'static FabManifest> {
    static MANIFEST: std::sync::OnceLock<Option<FabManifest>> = std::sync::OnceLock::new();
    MANIFEST
        .get_or_init(|| {
            let manifest = FabManifest::from_json(assets::MANIFEST).ok()?;
            manifest.validate(&[1, 2, 3, 4]).ok()?;
            Some(manifest)
        })
        .as_ref()
}
