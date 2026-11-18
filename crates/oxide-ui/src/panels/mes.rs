//! MES: WIP, tool lights and genealogy.
//!
//! The panels are projections of the snapshot; every write is a labelled
//! 演示触发 button that goes through `/api/lots/...` and lands in `lot_event`.

use nsh_core::prelude::*;
use oxide_scene::Entity;

use super::{BODY, SMALL, TINY, badge, bar, button, card, color, divider, dot, small, strong};
use crate::client;
use crate::data::{LotView, WaferView, duration_text};
use crate::scene_glue;
use crate::state::Ui;

#[must_use]
pub fn panel(ui: Ui) -> AnyElement {
    view()
        .gap_3()
        .child(steps(ui))
        .child(tools(ui))
        .child(lots(ui))
        .into_element()
}

fn state_color(ui: Ui, state: &str) -> u32 {
    let palette = ui.palette();
    match state {
        "completed" => palette.good,
        "processing" => palette.info,
        "on_hold" => palette.warn,
        "split" | "merged" => palette.muted,
        _ => palette.border,
    }
}

fn light_color(ui: Ui, light: &str) -> u32 {
    let palette = ui.palette();
    match light {
        "red" => palette.bad,
        "amber" => palette.warn,
        _ => palette.good,
    }
}

fn steps(ui: Ui) -> AnyElement {
    let steps = ui.steps;
    let tools = ui.tools;
    card(
        ui,
        "工序队列 · WIP",
        "route_step + lot · 队列只影响显示，不排序仿真",
        view().gap_1p5().child(each(
            steps,
            |step| step.step_no,
            move |step, _| {
                let current = step.get().current_lots.clone();
                view()
                    .gap_1()
                    .p_2()
                    .rounded(px(8.0))
                    .bg(move || color(ui, ui.palette().surface_alt))
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                text(move || {
                                    format!("S{} · {}", step.get().step_no, step.get().name)
                                })
                                .text_size(px(BODY))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || color(ui, ui.palette().text)),
                            )
                            .child(view().flex_1())
                            .child(
                                text(move || {
                                    format!(
                                        "排队 {} · 机台 {}/{}",
                                        step.get().queue_len,
                                        step.get().busy_tools,
                                        step.get().total_tools
                                    )
                                })
                                .text_size(px(TINY))
                                .text_color(move || color(ui, ui.palette().muted)),
                            ),
                    )
                    .child(view().flex_row().flex_wrap().gap_1().children(
                        current.into_iter().take(6).map(|lot_id| {
                            let lot = lot_id.clone();
                            view()
                                .px_1p5()
                                .py_0p5()
                                .rounded(px(5.0))
                                .bg(move || color(ui, ui.palette().surface))
                                .border_1()
                                .border_color(move || color(ui, ui.palette().border))
                                .on_press(move || {
                                    scene_glue::drill(
                                        ui,
                                        Entity::Foup {
                                            lot_id: lot.clone(),
                                        },
                                    );
                                })
                                .child(
                                    text(lot_id)
                                        .text_size(px(TINY))
                                        .text_color(move || color(ui, ui.palette().info)),
                                )
                                .into_element()
                        }),
                    ))
                    .child(
                        text(move || {
                            let tools = tools.get();
                            let busy: Vec<String> = tools
                                .iter()
                                .filter(|tool| tool.current_step == Some(step.get().step_no))
                                .map(|tool| tool.id.clone())
                                .collect();
                            if busy.is_empty() {
                                "本工序无在机批次".to_owned()
                            } else {
                                format!("在机 {}", busy.join(" · "))
                            }
                        })
                        .text_size(px(TINY))
                        .text_color(move || color(ui, ui.palette().muted)),
                    )
                    .into_element()
            },
        )),
    )
}

fn tools(ui: Ui) -> AnyElement {
    let tools = ui.tools;
    let analysis = ui.analysis;
    card(
        ui,
        "机台三态 · E116 时长",
        "equipment.state / e116_perf 聚合 · 绿运行 黄维护 红停机",
        view().gap_1().child(each(
            tools,
            |tool| tool.id.clone(),
            move |tool, _| {
                let id = tool.get().id.clone();
                let id_for_press = id.clone();
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1p5()
                    .rounded(px(7.0))
                    .bg(move || color(ui, ui.palette().surface_alt))
                    .on_press(move || {
                        scene_glue::drill(
                            ui,
                            Entity::Tool {
                                equipment_id: id_for_press.clone(),
                            },
                        );
                    })
                    .child(dot(light_color(ui, &tool.get().light)))
                    .child(
                        text(move || tool.get().id.clone())
                            .text_size(px(BODY))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || color(ui, ui.palette().text)),
                    )
                    .child(
                        text(move || {
                            let tool = tool.get();
                            tool.current_lot.as_ref().map_or_else(
                                || tool.state.clone(),
                                |lot| format!("{} · {lot}", tool.state),
                            )
                        })
                        .text_size(px(TINY))
                        .text_color(move || color(ui, ui.palette().muted)),
                    )
                    .child(view().flex_1())
                    .child(dynamic(move || {
                        let e116 = analysis.get().and_then(|analysis| {
                            analysis
                                .kpi
                                .tools
                                .iter()
                                .find(|entry| entry.equipment_id == id)
                                .map(|entry| {
                                    (
                                        entry.utilization,
                                        entry.busy_ms,
                                        entry.down_ms,
                                        entry.track_outs,
                                    )
                                })
                        });
                        Some(match e116 {
                            Some((utilization, busy, down, outs)) => view()
                                .flex_row()
                                .items_center()
                                .gap_1()
                                .child(bar(ui, utilization as f32, ui.palette().good, 70.0))
                                .child(
                                    text(format!(
                                        "效率 {:.0}% · {} · 停 {} · {} 次",
                                        utilization * 100.0,
                                        duration_text(busy),
                                        duration_text(down),
                                        outs
                                    ))
                                    .text_size(px(TINY))
                                    .text_color(move || color(ui, ui.palette().muted)),
                                )
                                .into_element(),
                            None => small(ui, "等待 e116".to_owned()),
                        })
                    }))
                    .into_element()
            },
        )),
    )
}

fn lots(ui: Ui) -> AnyElement {
    let lots = ui.lots;
    let selected = ui.selected_lot;
    card(
        ui,
        "批次与逐片追溯",
        "lot / wafer / lot_event · 点击批次进入 L2",
        view().gap_1().child(each(
            lots,
            |lot| lot.id.clone(),
            move |lot, _| {
                let id = lot.get().id.clone();
                let id_for_press = id.clone();
                let is_selected = move || selected.get().as_deref() == Some(lot.get().id.as_str());
                view()
                    .gap_1()
                    .p_2()
                    .rounded(px(8.0))
                    .bg(move || {
                        if is_selected() {
                            color(ui, ui.palette().surface_alt)
                        } else {
                            color(ui, ui.palette().surface)
                        }
                    })
                    .border_1()
                    .border_color(move || {
                        if is_selected() {
                            color(ui, ui.palette().accent)
                        } else {
                            color(ui, ui.palette().border)
                        }
                    })
                    .on_press(move || {
                        scene_glue::drill(
                            ui,
                            Entity::Foup {
                                lot_id: id_for_press.clone(),
                            },
                        );
                    })
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(dot(state_color(ui, &lot.get().state)))
                            .child(
                                text(move || lot.get().id.clone())
                                    .text_size(px(BODY))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || color(ui, ui.palette().text)),
                            )
                            .child(
                                text(move || lot.get().state.clone())
                                    .text_size(px(TINY))
                                    .text_color(move || {
                                        color(ui, state_color(ui, &lot.get().state))
                                    }),
                            )
                            .child(view().flex_1())
                            .child(
                                text(move || {
                                    let lot = lot.get();
                                    format!(
                                        "S{} · {}/{} 片 · P{}",
                                        lot.current_step,
                                        lot.done_wafers,
                                        lot.wafer_count,
                                        lot.priority
                                    )
                                })
                                .text_size(px(TINY))
                                .text_color(move || color(ui, ui.palette().muted)),
                            ),
                    )
                    .child(dynamic(move || {
                        if !is_selected() {
                            return None;
                        }
                        let detail = ui.detail.get()?;
                        Some(
                            detail_view(ui, &detail.lot, &detail.wafers, &detail.genealogy)
                                .into_element(),
                        )
                    }))
                    .into_element()
            },
        )),
    )
}

fn detail_view(
    ui: Ui,
    lot: &LotView,
    wafers: &[WaferView],
    genealogy: &[crate::data::WaferGenealogy],
) -> impl IntoElement {
    let selected_wafer = ui.selected_wafer;
    view()
        .gap_2()
        .pt_1()
        .child(divider(ui))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(
                    text(format!("{} · 批次操作", lot.id))
                        .text_size(px(SMALL))
                        .text_color(move || color(ui, ui.palette().muted)),
                )
                .child(view().flex_1())
                .child(badge(
                    ui,
                    "演示触发".to_owned(),
                    ui.palette().warn,
                    ui.palette().surface_alt,
                )),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_1()
                .child(button(ui, "Hold", false, move || client::hold(ui)))
                .child(button(ui, "Release", false, move || client::release(ui)))
                .child(button(ui, "返工", false, move || client::rework(ui)))
                .child(button(ui, "拆分", false, move || client::split(ui)))
                .child(button(ui, "合并", false, move || client::merge(ui)))
                .child(button(ui, "报废首片", false, move || client::scrap(ui))),
        )
        .child(
            view()
                .flex_row()
                .flex_wrap()
                .gap_1()
                .children((1..=25).map(|slot| {
                    let wafer = wafers.iter().find(|wafer| wafer.slot == slot);
                    wafer_cell(ui, lot, slot, wafer)
                })),
        )
        .child(match selected_wafer.get() {
            Some(wafer_id) => genealogy
                .iter()
                .find(|entry| entry.wafer_id == wafer_id)
                .map_or_else(
                    || small(ui, "读取逐片履历…".to_owned()),
                    |entry| genealogy_view(ui, entry),
                ),
            None => small(ui, "点一个槽位看逐片履历；红色为缺陷分类".to_owned()),
        })
}

fn wafer_cell(ui: Ui, lot: &LotView, slot: i32, wafer: Option<&WaferView>) -> AnyElement {
    let palette = ui.palette();
    let (fill, border, label) = match wafer {
        Some(wafer) if wafer.state == "scrapped" => {
            let class = oxide_scene::defect_class(wafer.defect_info.as_deref()).unwrap_or("other");
            let fill = match class {
                "scratch" => 0xd9572b,
                "particle" => 0xeb2f8c,
                "resist" => 0xb45cf2,
                "pattern" => 0xf2bf29,
                _ => palette.bad,
            };
            (fill, palette.bad, format!("W{slot:02}"))
        }
        Some(wafer) if wafer.state == "done" => (palette.good, palette.good, format!("W{slot:02}")),
        Some(wafer) if wafer.state == "processing" => {
            (palette.info, palette.info, format!("W{slot:02}"))
        }
        Some(_) => (palette.surface_alt, palette.border, format!("W{slot:02}")),
        None => (palette.background, palette.border, format!("{slot:02}")),
    };
    let wafer_id = wafer.map(|wafer| wafer.id.clone());
    let lot_id = lot.id.clone();
    view()
        .size(px(26.0))
        .items_center()
        .justify_center()
        .rounded(px(5.0))
        .bg(move || color(ui, fill))
        .border_1()
        .border_color(move || color(ui, border))
        .on_press(move || {
            if let Some(wafer_id) = &wafer_id {
                scene_glue::drill(
                    ui,
                    Entity::Wafer {
                        lot_id: lot_id.clone(),
                        wafer_id: wafer_id.clone(),
                    },
                );
            }
        })
        .child(
            text(label)
                .text_size(px(8.5))
                .text_color(move || color(ui, palette.text)),
        )
        .into_element()
}

fn genealogy_view(ui: Ui, entry: &crate::data::WaferGenealogy) -> AnyElement {
    let last_tool = entry.steps.last().map(|step| step.equipment_id.clone());
    let rows: Vec<AnyElement> = entry
        .steps
        .iter()
        .map(|step| {
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(format!("S{}", step.step_no))
                        .text_size(px(TINY))
                        .text_color(move || color(ui, ui.palette().muted)),
                )
                .child(
                    text(step.equipment_id.clone())
                        .text_size(px(TINY))
                        .text_color(move || color(ui, ui.palette().info)),
                )
                .child(
                    text(step.recipe_id.clone())
                        .text_size(px(TINY))
                        .text_color(move || color(ui, ui.palette().muted)),
                )
                .child(view().flex_1())
                .child(
                    text(if step.track_out_ms > 0 {
                        duration_text(step.duration_ms)
                    } else {
                        "进行中".to_owned()
                    })
                    .text_size(px(TINY))
                    .text_color(move || color(ui, ui.palette().text)),
                )
                .child(
                    text(
                        step.measured
                            .map_or_else(String::new, |value| format!("{value:.2} nm")),
                    )
                    .text_size(px(TINY))
                    .text_color(move || color(ui, ui.palette().warn)),
                )
                .into_element()
        })
        .collect();
    view()
        .gap_1()
        .p_2()
        .rounded(px(7.0))
        .bg(move || color(ui, ui.palette().surface_alt))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(strong(ui, entry.wafer_id.clone()))
                .child(small(
                    ui,
                    format!(
                        "{} · 返工 {} 次{}{}",
                        entry.state,
                        entry.rework_count,
                        entry
                            .defect_info
                            .as_ref()
                            .map_or_else(String::new, |info| format!(" · 缺陷 {info}")),
                        ""
                    ),
                ))
                .child(view().flex_1())
                .child(dynamic(move || {
                    last_tool.clone().map(|tool| {
                        button(ui, "来源机台", false, move || {
                            scene_glue::drill(
                                ui,
                                Entity::Tool {
                                    equipment_id: tool.clone(),
                                },
                            );
                        })
                    })
                })),
        )
        .child(view().gap_0p5().child(rows))
        .into_element()
}
