//! Windows host: the frame is drawn with Direct2D into the view's box.
//!
//! The bitmap is rebuilt when the frame version changes; presses and moves go
//! to nsh's own handlers, and the wheel arrives here, where it is forwarded to
//! the app as a native event (nsh reports it to the component first).

use std::cell::{Cell, RefCell};

use nsh_platform_windows::windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_PIXEL_FORMAT,
};
use nsh_platform_windows::windows::Win32::Graphics::Direct2D::{
    D2D1_BITMAP_OPTIONS_NONE, D2D1_BITMAP_PROPERTIES1, D2D1_INTERPOLATION_MODE_LINEAR,
    ID2D1Bitmap1, ID2D1DeviceContext,
};
use nsh_platform_windows::windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use nsh_platform_windows::{ComponentContext, Components, NativeComponent, Wheel};
use serde_json::{Value, json};

use crate::frames;
use crate::native::NAME;

struct ViewportView {
    context: ComponentContext,
    sink: u64,
    version: u64,
    width: u32,
    height: u32,
    /// BGRA, premultiplied (the frames are opaque).
    pixels: Vec<u8>,
    bitmap: RefCell<Option<ID2D1Bitmap1>>,
    drawn: Cell<u64>,
}

impl ViewportView {
    fn new(context: ComponentContext) -> Self {
        Self {
            context,
            sink: 0,
            version: 0,
            width: 0,
            height: 0,
            pixels: Vec::new(),
            bitmap: RefCell::new(None),
            drawn: Cell::new(0),
        }
    }

    fn blit(&mut self) {
        if self.sink == 0 {
            return;
        }
        let Some((version, width, height, pixels)) =
            frames::with_new_frame(self.sink, self.version, |version, width, height, rgba| {
                let mut pixels = Vec::with_capacity(rgba.len());
                for pixel in rgba.as_chunks::<4>().0 {
                    pixels.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
                }
                (version, width, height, pixels)
            })
        else {
            return;
        };
        self.version = version;
        self.width = width;
        self.height = height;
        self.pixels = pixels;
        self.context.redraw();
    }

    fn ensure_bitmap(&self, dc: &ID2D1DeviceContext) {
        if self.drawn.get() == self.version || self.width == 0 || self.height == 0 {
            return;
        }
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        let size = D2D_SIZE_U {
            width: self.width,
            height: self.height,
        };
        // SAFETY: `pixels` holds `height` rows of `width * 4` BGRA bytes.
        let made = unsafe {
            dc.CreateBitmap(
                size,
                Some(self.pixels.as_ptr().cast()),
                self.width * 4,
                &props,
            )
        };
        if let Ok(bitmap) = made {
            *self.bitmap.borrow_mut() = Some(bitmap);
            self.drawn.set(self.version);
        }
    }
}

impl NativeComponent for ViewportView {
    fn draw(&self, dc: &ID2D1DeviceContext, size: (f32, f32)) {
        self.ensure_bitmap(dc);
        let bitmap = self.bitmap.borrow();
        let Some(bitmap) = bitmap.as_ref() else {
            return;
        };
        let destination = D2D_RECT_F {
            left: 0.0,
            top: 0.0,
            right: size.0,
            bottom: size.1,
        };
        // SAFETY: COM calls on the context nsh draws with.
        unsafe {
            dc.DrawBitmap(
                bitmap,
                None,
                1.0,
                D2D1_INTERPOLATION_MODE_LINEAR,
                Some(&destination),
                None,
            );
        }
    }

    fn set_prop(&mut self, name: &str, value: Option<&Value>) {
        match name {
            "sink" => {
                self.sink = value.and_then(Value::as_u64).unwrap_or(0);
                self.version = 0;
                self.blit();
            }
            "frame" => self.blit(),
            _ => {}
        }
    }

    fn wheel(&mut self, event: &Wheel) -> bool {
        self.context.emit(
            "wheel",
            json!({
                "x": event.x,
                "y": event.y,
                "dx": event.dx,
                "dy": event.dy,
                "notches_y": event.notches_y,
                "precise": event.precise,
                "shift": event.shift,
                "ctrl": event.ctrl,
            }),
        );
        true
    }
}

pub fn register(components: &Components) {
    components.register(
        NAME,
        std::rc::Rc::new(|context| -> Box<dyn NativeComponent> {
            Box::new(ViewportView::new(context))
        }),
    );
}
