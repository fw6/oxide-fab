//! macOS host: an `NSImageView` inside a pass-through `NSView`.
//!
//! `hitTest:` returning nil is the documented nsh pattern for a view that must
//! let the pointer through (`NshLentClip` does the same for lent web views), so
//! nsh's own mouse, wheel and press handlers keep receiving events.

use nsh_platform_macos::objc2::rc::Retained;
use nsh_platform_macos::objc2::{
    AnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send,
};
use nsh_platform_macos::objc2_app_kit::{
    NSAutoresizingMaskOptions, NSBitmapImageRep, NSCalibratedRGBColorSpace, NSImage,
    NSImageScaling, NSImageView, NSResponder, NSView,
};
use nsh_platform_macos::objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize};
use nsh_platform_macos::{ComponentContext, Components, NativeComponent};
use serde_json::Value;

use crate::frames;
use crate::native::NAME;

define_class!(
    /// Hosts the viewport image and lets every event through to nsh.
    #[unsafe(super(NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OxideViewportView"]
    struct PassThrough;

    unsafe impl NSObjectProtocol for PassThrough {}

    impl PassThrough {
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, _point: NSPoint) -> Option<Retained<NSView>> {
            None
        }
    }
);

struct ViewportView {
    host: Retained<PassThrough>,
    image: Retained<NSImageView>,
    sink: u64,
    version: u64,
}

impl ViewportView {
    fn new(_context: ComponentContext) -> Self {
        let mtm = MainThreadMarker::new().expect("a native component is made on the main thread");
        // SAFETY: NSView's designated initializer, on the main thread.
        let host: Retained<PassThrough> =
            unsafe { msg_send![PassThrough::alloc(mtm), initWithFrame: NSRect::ZERO] };
        let image = NSImageView::new(mtm);
        image.setImageScaling(NSImageScaling::ScaleAxesIndependently);
        image.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        image.setFrame(host.bounds());
        host.addSubview(&image);
        Self {
            host,
            image,
            sink: 0,
            version: 0,
        }
    }

    fn blit(&mut self) {
        if self.sink == 0 {
            return;
        }
        let image = &self.image;
        let Some(version) = frames::with_new_frame(
            self.sink,
            self.version,
            |version, width, height, rgba| {
                let width = width as isize;
                let height = height as isize;
                // SAFETY: AppKit allocates the bitmap planes (null planes) and the
                // pointer it returns holds `height` rows of `bytesPerRow` bytes.
                unsafe {
                    let rep = NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
                    <NSBitmapImageRep as AnyThread>::alloc(),
                    std::ptr::null_mut(),
                    width,
                    height,
                    8,
                    4,
                    true,
                    false,
                    NSCalibratedRGBColorSpace,
                    width * 4,
                    32,
                );
                    let Some(rep) = rep else {
                        return version;
                    };
                    let row = rep.bytesPerRow() as usize;
                    let destination = rep.bitmapData();
                    if destination.is_null() {
                        return version;
                    }
                    let source_row = width as usize * 4;
                    for row_index in 0..height as usize {
                        std::ptr::copy_nonoverlapping(
                            rgba.as_ptr().add(row_index * source_row),
                            destination.add(row_index * row),
                            source_row,
                        );
                    }
                    let frame = NSImage::initWithSize(
                        NSImage::alloc(),
                        NSSize::new(width as f64, height as f64),
                    );
                    frame.addRepresentation(&rep);
                    image.setImage(Some(&frame));
                    version
                }
            },
        ) else {
            return;
        };
        self.version = version;
    }
}

impl NativeComponent for ViewportView {
    fn view(&self) -> Retained<NSView> {
        Retained::into_super(self.host.clone())
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
}

pub fn register(components: &Components) {
    components.register(
        NAME,
        std::rc::Rc::new(|context| -> Box<dyn NativeComponent> {
            Box::new(ViewportView::new(context))
        }),
    );
}
