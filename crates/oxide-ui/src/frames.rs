//! Frames from the render thread to the native view.
//!
//! A prop carries JSON, so it carries only the slot number and a frame
//! version; the pixels stay in this process, behind a mutex. The component
//! copies the newest frame while it holds the lock, so a 5 MB image never
//! crosses the nsh bridge.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Debug, Default)]
pub struct Frame {
    pub version: u64,
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA8, top row first.
    pub rgba: Vec<u8>,
}

static SLOTS: OnceLock<Mutex<HashMap<u64, Arc<Mutex<Frame>>>>> = OnceLock::new();
static NEXT: AtomicU64 = AtomicU64::new(1);

fn slots() -> &'static Mutex<HashMap<u64, Arc<Mutex<Frame>>>> {
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// A new slot and its id (the number the `sink` prop carries).
#[must_use]
pub fn create() -> (u64, Arc<Mutex<Frame>>) {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let slot = Arc::new(Mutex::new(Frame::default()));
    if let Ok(mut slots) = slots().lock() {
        slots.insert(id, Arc::clone(&slot));
    }
    (id, slot)
}

/// The slot with `id`, if it is still registered.
#[must_use]
pub fn get(id: u64) -> Option<Arc<Mutex<Frame>>> {
    slots().lock().ok()?.get(&id).cloned()
}

/// Run `read` over the frame in `id` when its version is newer than `last`.
/// The closure runs under the lock: copy, do not hold the pixels.
pub fn with_new_frame<R>(
    id: u64,
    last: u64,
    read: impl FnOnce(u64, u32, u32, &[u8]) -> R,
) -> Option<R> {
    let slot = get(id)?;
    let frame = slot.lock().ok()?;
    if frame.version <= last || frame.rgba.is_empty() || frame.width == 0 || frame.height == 0 {
        return None;
    }
    Some(read(frame.version, frame.width, frame.height, &frame.rgba))
}
