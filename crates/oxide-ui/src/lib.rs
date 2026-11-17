//! The oxide-fab 3D workbench, written in Rust for nsh.
//!
//! `Ui` is one `Copy` struct of signal handles (~430 bytes). Passing it by
//! value into every panel and every listener is the nsh idiom and avoids
//! lifetime noise in the closures; the size is a handful of pointers, not a
//! buffer.
#![allow(clippy::large_types_passed_by_value)]
//!
//! One window: the factory in a wgpu viewport on the left, the four
//! `SmartFactory` domains and the cross-cutting capabilities in panels on the
//! right. The viewport is a native component (`oxide.view3d`) fed by
//! `oxide-render`; every panel is a read-only projection of the API.
//!
//! ```text
//! PostgreSQL lot_event ──/api/snapshot, /api/events──▶ Ui signals
//!        │                                              │
//!        └──/api/analytics, /api/monitor─────────────────┤
//!                                                       ▼
//!                                     oxide-scene graph ─▶ oxide-render (wgpu)
//!                                                             │ frames
//!                                                             ▼
//!                                                    native component
//! ```

pub mod client;
pub mod data;
pub mod demo;
pub mod engine;
pub mod frames;
pub mod native;
pub mod palette;
pub mod panels;
pub mod scene_glue;
pub mod state;
pub mod viewport;

use nsh_base::input;
use nsh_core::prelude::*;
use oxide_scene::Entity;

use crate::palette::Palette;
use crate::panels::{SMALL, TINY, button, button_dyn, card, color, dot, small, small_dyn, strong};
use crate::state::{Tab, Ui};

const FONT: &str = "WenQuanYi Micro Hei, Noto Sans CJK SC, Source Han Sans SC, PingFang SC, Microsoft YaHei, sans-serif";

/// The native components this crate registers; the development host and the
/// packaged app call these before loading the app.
#[cfg(target_os = "macos")]
#[native_components(macos)]
fn oxide_components_macos(components: &nsh_platform_macos::Components) {
    native::macos::register(components);
}

#[cfg(target_os = "linux")]
#[native_components(linux)]
fn oxide_components_linux(components: &nsh_platform_linux::Components) {
    native::linux::register(components);
}

#[cfg(target_os = "windows")]
#[native_components(windows)]
fn oxide_components_windows(components: &nsh_platform_windows::Components) {
    native::windows::register(components);
}

/// The app's UI set, for a TSX entry or a test.
#[native_module("oxide-ui/native")]
fn native_registry(registry: &mut Registry) {
    registry.register::<App>();
}

/// The application root.
#[component]
pub fn App() -> impl IntoElement {
    let sink = viewport::init();
    let ui = Ui::new(sink);
    client::bootstrap(ui);
    viewport::start_render_loop(ui);
    view()
        .flex_1()
        .font_family(FONT)
        .bg(move || color(ui, ui.palette().background))
        .text_color(move || color(ui, ui.palette().text))
        .child(header(ui))
        .child(
            view()
                .flex_1()
                .flex_row()
                .min_h(px(0.0))
                .gap_2()
                .px_2()
                .pb_2()
                .child(left_rail(ui))
                .child(center(ui))
                .child(inspector(ui)),
        )
        .child(footer(ui))
}

#[expect(
    clippy::too_many_lines,
    reason = "the title bar is one row: brand, tabs, connection, clock, playback, theme and demo"
)]
fn header(ui: Ui) -> impl IntoElement {
    view()
        .w_full()
        .px_3()
        .py_2()
        .flex_row()
        .items_center()
        .gap_3()
        .flex_wrap()
        .bg(move || color(ui, ui.palette().surface))
        .border_b_1()
        .border_color(move || color(ui, ui.palette().border))
        .child(
            view()
                .gap_0p5()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            text("氧化层 · oxide-fab")
                                .text_size(px(17.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(move || color(ui, ui.palette().text)),
                        )
                        .child(
                            text("3D 工作台")
                                .text_size(px(TINY))
                                .text_color(move || color(ui, ui.palette().accent)),
                        ),
                )
                .child(
                    text("仿真产线 · PG 事实来源 · 事件驱动")
                        .text_size(px(TINY))
                        .text_color(move || color(ui, ui.palette().muted)),
                ),
        )
        .child(view().flex_1())
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .children(Tab::ALL.into_iter().map(|tab| tab_button(ui, tab))),
        )
        .child(view().flex_1())
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(view().size(px(8.0)).rounded(px(4.0)).bg(move || {
                    color(
                        ui,
                        if ui.connected.get() {
                            ui.palette().good
                        } else {
                            ui.palette().bad
                        },
                    )
                }))
                .child(
                    text(move || {
                        if ui.connected.get() {
                            "实时"
                        } else {
                            "重连中"
                        }
                    })
                    .text_size(px(TINY))
                    .text_color(move || color(ui, ui.palette().muted)),
                )
                .child(
                    text(move || data::clock_text(ui.now_ms.get()))
                        .text_size(px(TINY))
                        .text_color(move || color(ui, ui.palette().muted)),
                ),
        )
        .child(sim_controls(ui))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(button_dyn(
                    ui,
                    move || if ui.dark.get() { "浅色" } else { "深色" }.to_owned(),
                    false,
                    move || ui.dark.update(|dark| *dark = !*dark),
                ))
                .child(button_dyn(
                    ui,
                    move || {
                        if demo::running(ui) {
                            "停止演示"
                        } else {
                            "演示"
                        }
                        .to_owned()
                    },
                    true,
                    move || demo::toggle(ui),
                )),
        )
}

fn tab_button(ui: Ui, tab: Tab) -> AnyElement {
    view()
        .px_2()
        .py_1()
        .rounded(px(6.0))
        .bg(move || {
            if ui.tab.get() == tab {
                color(ui, ui.palette().surface_alt)
            } else {
                color(ui, ui.palette().surface)
            }
        })
        .border_b_1()
        .border_color(move || {
            if ui.tab.get() == tab {
                color(ui, ui.palette().accent)
            } else {
                color(ui, ui.palette().surface)
            }
        })
        .on_press(move || {
            demo::stop(ui);
            ui.tab.set(tab);
        })
        .child(dynamic(move || {
            let active = ui.tab.get() == tab;
            Some(
                text(tab.label())
                    .text_size(px(SMALL))
                    .font_weight(if active {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(move || {
                        if active {
                            color(ui, ui.palette().text)
                        } else {
                            color(ui, ui.palette().muted)
                        }
                    })
                    .into_element(),
            )
        }))
        .into_element()
}

fn sim_controls(ui: Ui) -> impl IntoElement {
    let running = ui.running;
    let speed = ui.speed;
    view()
        .flex_row()
        .items_center()
        .gap_1()
        .child(button_dyn(
            ui,
            move || if running.get() { "暂停" } else { "运行" }.to_owned(),
            false,
            move || {
                client::sim(
                    ui,
                    if running.get() {
                        "/api/sim/pause"
                    } else {
                        "/api/sim/play"
                    },
                    if running.get() { "暂停" } else { "运行" },
                );
            },
        ))
        .child(button(ui, "单步", false, move || {
            client::sim(ui, "/api/sim/step", "单步");
        }))
        .child(button(ui, "−", false, move || {
            let next = (speed.get() / 2.0).max(0.5);
            client::sim(ui, &format!("/api/sim/speed/{next}"), "降速");
        }))
        .child(
            text(move || format!("×{:.0}", speed.get()))
                .text_size(px(TINY))
                .text_color(move || color(ui, ui.palette().muted)),
        )
        .child(button(ui, "+", false, move || {
            let next = (speed.get() * 2.0).min(100.0);
            client::sim(ui, &format!("/api/sim/speed/{next}"), "提速");
        }))
}

fn left_rail(ui: Ui) -> impl IntoElement {
    view()
        .w(px(244.0))
        .gap_2()
        .child(breadcrumb(ui))
        .child(selection_card(ui))
        .child(search_card(ui))
        .child(legend(ui))
        .child(demo_card(ui))
}

fn breadcrumb(ui: Ui) -> impl IntoElement {
    view()
        .w_full()
        .p_2()
        .gap_1()
        .rounded(px(10.0))
        .bg(move || color(ui, ui.palette().surface))
        .border_1()
        .border_color(move || color(ui, ui.palette().border))
        .child(
            text(move || ui.level_label())
                .text_size(px(SMALL))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || color(ui, ui.palette().text)),
        )
        .child(
            view()
                .flex_row()
                .gap_1()
                .flex_wrap()
                .child(button(ui, "L0", false, move || viewport::fit(ui)))
                .child(button(ui, "L1", false, move || goto_level(ui, 1)))
                .child(button(ui, "L2", false, move || goto_level(ui, 2)))
                .child(button(ui, "L3", false, move || goto_level(ui, 3))),
        )
        .child(
            view()
                .flex_row()
                .gap_1()
                .child(button(ui, "← 退回", false, move || {
                    scene_glue::level_up(ui);
                }))
                .child(button(ui, "复位视角", false, move || {
                    ui.camera.set(oxide_scene::Camera::default());
                })),
        )
}

fn selection_card(ui: Ui) -> impl IntoElement {
    card(
        ui,
        "当前选中",
        "",
        dynamic(move || {
            let Some(entity) = ui.selected.get() else {
                return Some(
                    small(
                        ui,
                        "点场景或点列表：机台 → 部件，批次 → 晶圆，事件 → 数据点".to_owned(),
                    )
                    .into_element(),
                );
            };
            let label = state::entity_label(&entity);
            let detail = match &entity {
                Entity::Tool { equipment_id } | Entity::ToolPart { equipment_id, .. } => ui
                    .tools
                    .get()
                    .into_iter()
                    .find(|tool| tool.id == *equipment_id)
                    .map_or_else(
                        || "机台不在快照中".to_owned(),
                        |tool| {
                            format!(
                                "{} · {} · 在机 {}",
                                tool.kind,
                                tool.state,
                                tool.current_lot.unwrap_or_else(|| "—".to_owned())
                            )
                        },
                    ),
                Entity::Foup { lot_id } | Entity::Wafer { lot_id, .. } => ui
                    .lots
                    .get()
                    .into_iter()
                    .find(|lot| lot.id == *lot_id)
                    .map_or_else(
                        || "批次不在快照中".to_owned(),
                        |lot| {
                            format!(
                                "{} · S{} · {}/{} 片",
                                lot.state, lot.current_step, lot.done_wafers, lot.wafer_count
                            )
                        },
                    ),
                _ => String::new(),
            };
            Some(
                view()
                    .gap_1()
                    .child(strong(ui, label))
                    .child(dynamic(move || {
                        (!detail.is_empty()).then(|| small(ui, detail.clone()))
                    }))
                    .child(
                        view()
                            .flex_row()
                            .gap_1()
                            .child(button(ui, "打开详情", false, move || match &entity {
                                Entity::Tool { equipment_id }
                                | Entity::ToolPart { equipment_id, .. } => {
                                    scene_glue::drill(
                                        ui,
                                        Entity::Tool {
                                            equipment_id: equipment_id.clone(),
                                        },
                                    );
                                }
                                Entity::Foup { lot_id } | Entity::Wafer { lot_id, .. } => {
                                    scene_glue::drill(
                                        ui,
                                        Entity::Foup {
                                            lot_id: lot_id.clone(),
                                        },
                                    );
                                }
                                _ => {}
                            }))
                            .child(button(ui, "清空", false, move || ui.select(None))),
                    )
                    .into_element(),
            )
        }),
    )
}

fn search_card(ui: Ui) -> impl IntoElement {
    card(
        ui,
        "按 ID 定位",
        "lot / wafer / equipment / event_id",
        view()
            .gap_1()
            .child(
                input()
                    .value(move || ui.search.get())
                    .placeholder("输入 ID，如 LOT-0001 或 event UUID")
                    .on_change(move |text: &str| ui.search.set(text.to_owned()))
                    .px_2()
                    .py_1()
                    .rounded(px(6.0))
                    .bg(move || color(ui, ui.palette().surface_alt))
                    .text_color(move || color(ui, ui.palette().text))
                    .text_size(px(SMALL)),
            )
            .child(dynamic(move || {
                let query = ui.search.get().trim().to_owned();
                if query.is_empty() {
                    return None;
                }
                Some(search_results(ui, &query))
            })),
    )
}

fn search_results(ui: Ui, query: &str) -> AnyElement {
    let needle = query.to_ascii_lowercase();
    let mut hits: Vec<(String, Entity)> = Vec::new();
    for lot in ui.lots.get() {
        if lot.id.to_ascii_lowercase().contains(&needle) {
            hits.push((
                lot.id.clone(),
                Entity::Foup {
                    lot_id: lot.id.clone(),
                },
            ));
        }
        for wafer in &lot.wafers {
            if wafer.id.to_ascii_lowercase().contains(&needle) {
                hits.push((
                    wafer.id.clone(),
                    Entity::Wafer {
                        lot_id: lot.id.clone(),
                        wafer_id: wafer.id.clone(),
                    },
                ));
            }
        }
    }
    for tool in ui.tools.get() {
        if tool.id.to_ascii_lowercase().contains(&needle) {
            hits.push((
                tool.id.clone(),
                Entity::Tool {
                    equipment_id: tool.id.clone(),
                },
            ));
        }
    }
    let looks_like_uuid = query.len() > 30 && query.contains('-');
    let event_query = query.to_owned();
    view()
        .gap_1()
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_1()
                .children(hits.into_iter().take(6).map(|(label, entity)| {
                    let entity_for_press = entity.clone();
                    view()
                        .px_1p5()
                        .py_0p5()
                        .rounded(px(5.0))
                        .bg(move || color(ui, ui.palette().surface_alt))
                        .on_press(move || scene_glue::drill(ui, entity_for_press.clone()))
                        .child(
                            text(label)
                                .text_size(px(TINY))
                                .text_color(move || color(ui, ui.palette().info)),
                        )
                        .into_element()
                })),
        )
        .child(dynamic(move || {
            looks_like_uuid.then(|| {
                button(ui, "按 event_id 追踪到 outbox 与数据点", true, {
                    let event_id = event_query.clone();
                    move || client::load_trace(ui, event_id.clone())
                })
            })
        }))
        .into_element()
}

fn legend(ui: Ui) -> impl IntoElement {
    let palette = ui.palette();
    let items = [
        ("运行 / 已完工", palette.good),
        ("维护 / Hold", palette.warn),
        ("停机 / 报废", palette.bad),
        ("处理中", palette.info),
        ("空闲 / 未开工", palette.muted),
    ];
    card(
        ui,
        "图例",
        "同一语义在 3D 与表格里同色",
        view()
            .gap_1()
            .children(items.into_iter().map(|(label, value)| {
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(dot(value))
                    .child(small(ui, label.to_owned()))
                    .into_element()
            })),
    )
}

fn demo_card(ui: Ui) -> impl IntoElement {
    card(
        ui,
        "引导演示",
        "可随时中断，接管后不跳回",
        view()
            .gap_1()
            .child(small_dyn(ui, move || ui.demo.get().label().to_owned()))
            .child(
                view()
                    .flex_row()
                    .gap_1()
                    .child(button_dyn(
                        ui,
                        move || {
                            if demo::running(ui) {
                                "停止"
                            } else {
                                "开始"
                            }
                            .to_owned()
                        },
                        true,
                        move || demo::toggle(ui),
                    ))
                    .child(button(ui, "下一步", false, move || demo::start(ui))),
            ),
    )
}

fn center(ui: Ui) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(420.0))
        .min_h(px(0.0))
        .gap_1()
        .child(viewport_toolbar(ui))
        .child(view().flex_1().min_h(px(0.0)).child(viewport::viewport(ui)))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .px_1()
                .child(small(
                    ui,
                    "拖拽旋转 · Shift+拖拽平移 · 滚轮缩放 · 点击拾取".to_owned(),
                ))
                .child(view().flex_1())
                .child(dynamic(move || {
                    ui.render_error.get().map(|error| {
                        text(error)
                            .text_size(px(TINY))
                            .text_color(move || color(ui, ui.palette().bad))
                            .into_element()
                    })
                }))
                .child(dynamic(move || {
                    let adapter = ui.adapter.get();
                    (!adapter.is_empty()).then(|| {
                        text(format!("wgpu · {adapter}"))
                            .text_size(px(TINY))
                            .text_color(move || color(ui, ui.palette().muted))
                            .into_element()
                    })
                })),
        )
}

fn viewport_toolbar(ui: Ui) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .px_1()
        .py_1()
        .child(button(ui, "正交 / 透视", false, move || {
            let mut camera = ui.camera.get_untracked();
            camera.toggle_projection();
            ui.camera.set(camera);
        }))
        .child(button(ui, "机台聚焦", false, move || goto_level(ui, 1)))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(small_dyn(ui, move || {
                    format!("爆炸 {:.0}%", ui.explosion.get())
                }))
                .child(button(ui, "−", false, move || {
                    ui.explosion
                        .update(|value| *value = (*value - 10.0).max(0.0));
                }))
                .child(button(ui, "+", false, move || {
                    ui.explosion
                        .update(|value| *value = (*value + 10.0).min(100.0));
                })),
        )
        .child(view().flex_1())
        .child(small_dyn(ui, move || ui.level_label()))
}

fn inspector(ui: Ui) -> impl IntoElement {
    view().w(px(430.0)).min_w(px(360.0)).min_h(px(0.0)).child(
        nsh_core::prelude::scroll().flex_1().child(
            view()
                .p_1()
                .gap_2()
                .child(
                    text(move || ui.tab.get().hint())
                        .text_size(px(TINY))
                        .text_color(move || color(ui, ui.palette().muted)),
                )
                .child(dynamic(move || {
                    Some(match ui.tab.get() {
                        Tab::Mes => panels::mes::panel(ui),
                        Tab::Quality => panels::quality::panel(ui),
                        Tab::Productivity => panels::productivity::panel(ui),
                        Tab::Supply => panels::supply::panel(ui),
                        Tab::Cross => panels::cross::panel(ui),
                    })
                })),
        ),
    )
}

fn footer(ui: Ui) -> impl IntoElement {
    view()
        .w_full()
        .px_3()
        .py_1()
        .flex_row()
        .items_center()
        .gap_3()
        .bg(move || color(ui, ui.palette().surface))
        .border_t_1()
        .border_color(move || color(ui, ui.palette().border))
        .child(small_dyn(ui, move || {
            format!(
                "事件 {} · 已投递 {} · outbox 积压 {} · Greptime {}",
                ui.events_total.get(),
                ui.published_events.get(),
                ui.outbox_pending.get(),
                if ui.greptime_ok.get() {
                    "在线"
                } else {
                    "断流（积压在 PG）"
                }
            )
        }))
        .child(small_dyn(ui, move || {
            ui.trace_event
                .get()
                .map_or_else(String::new, |id| format!("L3 event {id}"))
        }))
        .child(view().flex_1())
        .child(dynamic(move || {
            ui.note.get().map(|note| small(ui, note).into_element())
        }))
        .child(dynamic(move || {
            ui.last_action
                .get()
                .map(|action| small(ui, action).into_element())
        }))
}

fn goto_level(ui: Ui, level: u8) {
    demo::stop(ui);
    match level {
        0 => viewport::fit(ui),
        1 => {
            let tool = ui
                .focus
                .get_untracked()
                .and_then(|entity| entity.tool_id().map(str::to_owned))
                .or_else(|| ui.tools.get_untracked().first().map(|tool| tool.id.clone()));
            if let Some(equipment_id) = tool {
                scene_glue::drill(ui, Entity::Tool { equipment_id });
            } else {
                ui.note.set(Some("没有可进入的机台".to_owned()));
            }
        }
        2 => {
            let lot = ui
                .selected_lot
                .get_untracked()
                .or_else(|| ui.lots.get_untracked().first().map(|lot| lot.id.clone()));
            if let Some(lot_id) = lot {
                scene_glue::drill(ui, Entity::Foup { lot_id });
            } else {
                ui.note.set(Some("没有可进入的批次".to_owned()));
            }
        }
        _ => {
            let event_id = ui
                .analysis
                .get_untracked()
                .and_then(|analysis| {
                    analysis
                        .qtime_violations
                        .first()
                        .map(|violation| violation.event_id.clone())
                        .or_else(|| analysis.alarms.first().map(|alarm| alarm.event_id.clone()))
                })
                .or_else(|| ui.trace_event.get_untracked());
            match event_id {
                Some(event_id) => client::load_trace(ui, event_id),
                None => ui
                    .note
                    .set(Some("尚无可追踪事件：在左侧输入一个 event_id".to_owned())),
            }
        }
    }
}

/// Kept so the theme's palette type is used even when a panel is compiled out.
const _: fn() -> Palette = || crate::palette::DARK;
