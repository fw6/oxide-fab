//! Shared look of the panels: cards, rows, badges and bars.
//!
//! One place decides type sizes, spacing and colour usage, so the five domain
//! panels read as one product.

use nsh_core::prelude::*;

use crate::state::Ui;

pub const TINY: f32 = 10.5;
pub const SMALL: f32 = 11.5;
pub const BODY: f32 = 12.5;
pub const TITLE: f32 = 13.5;

#[must_use]
pub fn color(ui: Ui, value: u32) -> AdaptiveColor {
    let _ = ui;
    AdaptiveColor::from(rgb(value))
}

fn ui_source(ui: Ui, source: &'static str) -> String {
    let _ = ui;
    source.to_owned()
}

#[must_use]
pub fn card(
    ui: Ui,
    title: &'static str,
    source: &'static str,
    content: impl IntoElement,
) -> AnyElement {
    view()
        .w_full()
        .p_3()
        .gap_2()
        .rounded(px(10.0))
        .bg(move || color(ui, ui.palette().surface))
        .border_1()
        .border_color(move || color(ui, ui.palette().border))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(title)
                        .text_size(px(TITLE))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || color(ui, ui.palette().text)),
                )
                .child(view().flex_1())
                .child(dynamic(move || {
                    (ui.dense.get() && !source.is_empty()).then(|| {
                        text(ui_source(ui, source))
                            .text_size(px(TINY))
                            .text_color(move || color(ui, ui.palette().muted))
                            .into_element()
                    })
                })),
        )
        .child(content)
        .into_element()
}

#[must_use]
pub fn small(ui: Ui, value: String) -> AnyElement {
    text(value)
        .text_size(px(SMALL))
        .text_color(move || color(ui, ui.palette().muted))
        .into_element()
}

/// Text whose value is read from signals on every change.
#[must_use]
pub fn small_dyn(ui: Ui, value: impl Fn() -> String + 'static) -> AnyElement {
    text(value)
        .text_size(px(SMALL))
        .text_color(move || color(ui, ui.palette().muted))
        .into_element()
}

/// A button whose label is read from signals on every change.
#[must_use]
pub fn button_dyn(
    ui: Ui,
    label: impl Fn() -> String + 'static,
    accent: bool,
    press: impl Fn() + 'static,
) -> AnyElement {
    view()
        .px_2()
        .py_1()
        .rounded(px(6.0))
        .bg(move || {
            color(
                ui,
                if accent {
                    ui.palette().accent
                } else {
                    ui.palette().surface_alt
                },
            )
        })
        .border_1()
        .border_color(move || color(ui, ui.palette().border))
        .on_press(press)
        .child(text(label).text_size(px(SMALL)).text_color(move || {
            color(
                ui,
                if accent {
                    ui.palette().accent_text
                } else {
                    ui.palette().text
                },
            )
        }))
        .into_element()
}

#[must_use]
pub fn body(ui: Ui, value: String) -> AnyElement {
    text(value)
        .text_size(px(BODY))
        .text_color(move || color(ui, ui.palette().text))
        .into_element()
}

#[must_use]
pub fn strong(ui: Ui, value: String) -> AnyElement {
    text(value)
        .text_size(px(BODY))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || color(ui, ui.palette().text))
        .into_element()
}

#[must_use]
pub fn dot(value: u32) -> AnyElement {
    view()
        .size(px(8.0))
        .rounded(px(4.0))
        .bg(move || AdaptiveColor::from(rgb(value)))
        .into_element()
}

#[must_use]
pub fn badge(ui: Ui, label: String, fg: u32, bg: u32) -> AnyElement {
    view()
        .px_1p5()
        .py_0p5()
        .rounded(px(5.0))
        .bg(move || color(ui, bg))
        .child(
            text(label)
                .text_size(px(TINY))
                .text_color(move || color(ui, fg)),
        )
        .into_element()
}

#[must_use]
pub fn kv(ui: Ui, key: &'static str, value: String) -> AnyElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(
            text(key)
                .text_size(px(SMALL))
                .text_color(move || color(ui, ui.palette().muted)),
        )
        .child(view().flex_1())
        .child(
            text(value)
                .text_size(px(SMALL))
                .text_color(move || color(ui, ui.palette().text)),
        )
        .into_element()
}

/// A thin bar: `ratio` in 0..=1.
#[must_use]
pub fn bar(ui: Ui, ratio: f32, value: u32, width: f32) -> AnyElement {
    view()
        .h(px(6.0))
        .w(px(width))
        .rounded(px(3.0))
        .bg(move || color(ui, ui.palette().surface_alt))
        .child(
            view()
                .h(px(6.0))
                .w(px(width * ratio.clamp(0.0, 1.0)))
                .rounded(px(3.0))
                .bg(move || color(ui, value)),
        )
        .into_element()
}

#[must_use]
pub fn button(ui: Ui, label: &'static str, accent: bool, press: impl Fn() + 'static) -> AnyElement {
    view()
        .px_2()
        .py_1()
        .rounded(px(6.0))
        .bg(move || {
            color(
                ui,
                if accent {
                    ui.palette().accent
                } else {
                    ui.palette().surface_alt
                },
            )
        })
        .border_1()
        .border_color(move || color(ui, ui.palette().border))
        .on_press(press)
        .child(text(label).text_size(px(SMALL)).text_color(move || {
            color(
                ui,
                if accent {
                    ui.palette().accent_text
                } else {
                    ui.palette().text
                },
            )
        }))
        .into_element()
}

#[must_use]
pub fn section_label(ui: Ui, label: &'static str) -> AnyElement {
    text(label)
        .text_size(px(TINY))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || color(ui, ui.palette().muted))
        .into_element()
}

#[must_use]
pub fn divider(ui: Ui) -> AnyElement {
    view()
        .h(px(1.0))
        .w_full()
        .bg(move || color(ui, ui.palette().border))
        .into_element()
}
