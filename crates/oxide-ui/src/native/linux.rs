//! Linux host: a `GtkPicture` painted from a `GdkMemoryTexture`.
//!
//! GTK re-uploads the texture nsh's own shader path uses; the picture takes no
//! presses, so nsh's container keeps the drag and the wheel.

use nsh_platform_linux::gtk4 as gtk;
use nsh_platform_linux::{ComponentContext, Components, NativeComponent};
use serde_json::Value;

use crate::frames;
use crate::native::NAME;

struct ViewportView {
    picture: gtk::Picture,
    sink: u64,
    version: u64,
}

impl ViewportView {
    fn new(_context: ComponentContext) -> Self {
        let picture = gtk::Picture::new();
        picture.set_can_shrink(true);
        picture.set_content_fit(gtk::ContentFit::Fill);
        Self {
            picture,
            sink: 0,
            version: 0,
        }
    }

    fn blit(&mut self) {
        if self.sink == 0 {
            return;
        }
        let picture = &self.picture;
        let Some(version) =
            frames::with_new_frame(self.sink, self.version, |version, width, height, rgba| {
                // The renderer writes opaque RGBA; premultiplied and straight agree.
                let bytes = gtk::glib::Bytes::from(rgba);
                let texture = gtk::gdk::MemoryTexture::new(
                    width as i32,
                    height as i32,
                    gtk::gdk::MemoryFormat::R8g8b8a8Premultiplied,
                    &bytes,
                    width as usize * 4,
                );
                picture.set_paintable(Some(&texture));
                version
            })
        else {
            return;
        };
        self.version = version;
    }
}

impl NativeComponent for ViewportView {
    fn widget(&self) -> gtk::Widget {
        use gtk::prelude::Cast;
        self.picture.clone().upcast()
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
