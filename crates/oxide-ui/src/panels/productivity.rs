//! Productivity (APF semantics): dispatch, WIP, utilisation and the offline
//! what-if.
//!
//! The dispatch list is the simulator's own decision record, not a
//! reconstruction; every row names the rule it hit and the queue it came from.

use nsh_core::prelude::*;
use oxide_scene::Entity;

use super::{SMALL, TINY, badge, bar, button, button_dyn, card, color, dot, small, small_dyn};
use crate::client;
use crate::data::duration_text;
use crate::scene_glue;
use crate::state::Ui;

#[must_use]
pub fn panel(ui: Ui) -> AnyElement {
    view()
        .gap_3()
        .child(what_if(ui))
        .child(dispatch(ui))
        .child(wip(ui))
        .child(utilisation(ui))
        .child(empty_windows(ui))
        .into_element()
}

fn dispatch(ui: Ui) -> AnyElement {
    let dispatch = ui.dispatch;
    card(
        ui,
        "派工决策 · What-Next / Where-Next",
        "oxide-sim 规则 + route_step 资格矩阵",
        dynamic(move || {
            let decisions = dispatch.get();
            if decisions.is_empty() {
                return Some(small(
                    ui,
                    "尚无派工：运行仿真后，每次上料都会带着规则与队列位置出现在这里".to_owned(),
                ));
            }
            Some(
                view()
                    .gap_1()
                    .children(decisions.iter().take(7).map(|decision| {
                        let lot = decision.lot_id.clone();
                        view()
                            .gap_1()
                            .px_1p5()
                            .py_1()
                            .rounded(px(5.0))
                            .bg(move || color(ui, ui.palette().surface_alt))
                            .on_press(move || {
                                scene_glue::drill(
                                    ui,
                                    Entity::Foup {
                                        lot_id: lot.clone(),
                                    },
                                );
                            })
                            .child(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        text(format!(
                                            "{} → {}",
                                            decision.lot_id, decision.equipment_id
                                        ))
                                        .text_size(px(SMALL))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(move || color(ui, ui.palette().text)),
                                    )
                                    .child(view().flex_1())
                                    .child(badge(
                                        ui,
                                        decision.rule.clone(),
                                        ui.palette().accent,
                                        ui.palette().surface,
                                    )),
                            )
                            .child(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(small(
                                        ui,
                                        format!(
                                            "S{} · 优先级 {} · 队列 {}/{}",
                                            decision.step_no,
                                            decision.priority,
                                            decision.queue_position + 1,
                                            decision.queue_len
                                        ),
                                    ))
                                    .child(view().flex_1())
                                    .child(small(
                                        ui,
                                        format!(
                                            "资格 {} 台 · {}",
                                            decision.qualified_tools.len(),
                                            decision.recipe_id
                                        ),
                                    )),
                            )
                            .into_element()
                    }))
                    .into_element()
                    .into_element(),
            )
        }),
    )
}

fn wip(ui: Ui) -> AnyElement {
    let steps = ui.steps;
    card(
        ui,
        "WIP 气泡与瓶颈",
        "route_step 队列长度 · 口径：当前排队批次数",
        dynamic(move || {
            let steps = steps.get();
            if steps.is_empty() {
                return Some(small(ui, "等待快照".to_owned()));
            }
            let peak = steps.iter().map(|step| step.queue_len).max().unwrap_or(0);
            let bottleneck = steps
                .iter()
                .filter(|step| step.queue_len == peak && peak > 0)
                .map(|step| format!("S{} {}", step.step_no, step.name))
                .collect::<Vec<_>>()
                .join(" · ");
            Some(
                view()
                    .gap_2()
                    .child(view().flex_row().items_center().gap_3().children(
                        steps.into_iter().map(|step| {
                            let size = 28.0 + (step.queue_len as f32 * 3.0).min(28.0);
                            view()
                                .items_center()
                                .gap_1()
                                .child(
                                    view()
                                        .size(px(size))
                                        .items_center()
                                        .justify_center()
                                        .rounded(px(size / 2.0))
                                        .bg(move || {
                                            color(
                                                ui,
                                                if step.queue_len == peak && peak > 0 {
                                                    ui.palette().warn
                                                } else {
                                                    ui.palette().info
                                                },
                                            )
                                        })
                                        .child(
                                            text(format!("{}", step.queue_len))
                                                .text_size(px(SMALL))
                                                .font_weight(FontWeight::BOLD)
                                                .text_color(move || {
                                                    color(ui, ui.palette().accent_text)
                                                }),
                                        ),
                                )
                                .child(small(ui, format!("S{}", step.step_no)))
                                .into_element()
                        }),
                    ))
                    .child(small(
                        ui,
                        if bottleneck.is_empty() {
                            "无排队：瓶颈不适用".to_owned()
                        } else {
                            format!("当前瓶颈：{bottleneck}")
                        },
                    ))
                    .into_element(),
            )
        }),
    )
}

fn utilisation(ui: Ui) -> AnyElement {
    let analysis = ui.analysis;
    card(
        ui,
        "设备利用率 · 空窗",
        "track_out.duration_ms / 事件窗口",
        dynamic(move || {
            let Some(analysis) = analysis.get() else {
                return Some(small(ui, "读取 analytics…".to_owned()));
            };
            let window_ms = analysis
                .kpi
                .window_end_ms
                .zip(analysis.kpi.window_start_ms)
                .map_or(0, |(end, start)| end - start);
            Some(
                view()
                    .gap_1()
                    .children(analysis.kpi.tools.iter().map(|tool| {
                        let idle_ms = (window_ms - tool.busy_ms - tool.down_ms).max(0);
                        let idle_ratio = if window_ms > 0 {
                            idle_ms as f32 / window_ms as f32
                        } else {
                            0.0
                        };
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(dot(ui.palette().info))
                            .child(
                                text(tool.equipment_id.clone())
                                    .text_size(px(SMALL))
                                    .text_color(move || color(ui, ui.palette().text)),
                            )
                            .child(view().flex_1())
                            .child(
                                text(format!(
                                    "运行 {:.0}% · 停机 {:.0}% · 空窗 {}",
                                    tool.utilization * 100.0,
                                    (tool.down_ms.max(0) as f64 / window_ms.max(1) as f64) * 100.0,
                                    duration_text(idle_ms)
                                ))
                                .text_size(px(TINY))
                                .text_color(move || color(ui, ui.palette().muted)),
                            )
                            .child(bar(ui, idle_ratio, ui.palette().muted, 54.0))
                            .into_element()
                    }))
                    .into_element()
                    .into_element(),
            )
        }),
    )
}

fn empty_windows(ui: Ui) -> AnyElement {
    let analysis = ui.analysis;
    let running = ui.running;
    card(
        ui,
        "自动 / 单步",
        "FullAuto 与手动单步共用同一组接口",
        view()
            .gap_1()
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(view().size(px(8.0)).rounded(px(4.0)).bg(move || {
                        color(
                            ui,
                            if running.get() {
                                ui.palette().good
                            } else {
                                ui.palette().warn
                            },
                        )
                    }))
                    .child(small_dyn(ui, move || {
                        if running.get() {
                            "事件驱动自动流转中".to_owned()
                        } else {
                            "已暂停：单步推进，用于对比空窗".to_owned()
                        }
                    }))
                    .child(view().flex_1())
                    .child(button(ui, "自动", false, move || {
                        client::sim(ui, "/api/sim/play", "自动流转");
                    }))
                    .child(button(ui, "单步", false, move || {
                        client::sim(ui, "/api/sim/step", "单步");
                    })),
            )
            .child(dynamic(move || {
                let analysis = analysis.get()?;
                let window_ms = analysis
                    .kpi
                    .window_end_ms
                    .zip(analysis.kpi.window_start_ms)
                    .map_or(0, |(end, start)| end - start);
                let idle: i64 = analysis
                    .kpi
                    .tools
                    .iter()
                    .map(|tool| (window_ms - tool.busy_ms - tool.down_ms).max(0))
                    .sum();
                Some(small(
                    ui,
                    format!(
                        "当前口径：全部机台空窗合计 {}（窗口 {}）",
                        duration_text(idle),
                        duration_text(window_ms)
                    ),
                ))
            })),
    )
}

fn what_if(ui: Ui) -> AnyElement {
    let fifo = ui.whatif_fifo;
    let add_tool = ui.whatif_add_tool;
    let result = ui.whatif;
    let busy = ui.whatif_busy;
    card(
        ui,
        "离线 what-if 重放",
        "snapshot 副本重放 · 不打断当前实例",
        view()
            .gap_2()
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(button_dyn(
                        ui,
                        move || {
                            if fifo.get() {
                                "规则 FIFO ✓".to_owned()
                            } else {
                                "规则 优先级→FIFO".to_owned()
                            }
                        },
                        false,
                        move || fifo.update(|value| *value = !*value),
                    ))
                    .child(button_dyn(
                        ui,
                        move || {
                            if add_tool.get() {
                                "加一台同型机 ✓".to_owned()
                            } else {
                                "加一台同型机".to_owned()
                            }
                        },
                        false,
                        move || add_tool.update(|value| *value = !*value),
                    ))
                    .child(view().flex_1())
                    .child(button(
                        ui,
                        if busy.get() {
                            "重放中…"
                        } else {
                            "运行重放"
                        },
                        true,
                        move || client::run_what_if(ui),
                    )),
            )
            .child(dynamic(move || {
                let comparison = result.get()?;
                let baseline = &comparison.baseline;
                let variant = &comparison.variant;
                let delta = &comparison.delta;
                Some(
                    view()
                        .gap_1()
                        .child(row(
                            ui,
                            "完成批次",
                            baseline.completed_lots as f64,
                            variant.completed_lots as f64,
                            format!("{:+.0}", delta.completed_lots as f64),
                        ))
                        .child(row(
                            ui,
                            "平均周期",
                            baseline.mean_cycle_time_ms.unwrap_or(0.0),
                            variant.mean_cycle_time_ms.unwrap_or(0.0),
                            format!("{:+.0}", delta.mean_cycle_time_ms.unwrap_or(0.0)),
                        ))
                        .child(row(
                            ui,
                            "产出/小时",
                            baseline.throughput_lots_per_hour.unwrap_or(0.0),
                            variant.throughput_lots_per_hour.unwrap_or(0.0),
                            format!("{:+.2}", delta.throughput_lots_per_hour.unwrap_or(0.0)),
                        ))
                        .child(row(
                            ui,
                            "WIP",
                            baseline.wip_end as f64,
                            variant.wip_end as f64,
                            format!("{:+}", delta.wip_end),
                        ))
                        .child(
                            view()
                                .flex_row()
                                .gap_1()
                                .child(badge(
                                    ui,
                                    comparison
                                        .variant
                                        .added_tool
                                        .clone()
                                        .unwrap_or_else(|| "未加机台".to_owned()),
                                    ui.palette().accent,
                                    ui.palette().surface_alt,
                                ))
                                .child(dynamic(move || {
                                    comparison.baseline.from_start.then(|| {
                                        badge(
                                            ui,
                                            "从 t0 重放".to_owned(),
                                            ui.palette().warn,
                                            ui.palette().surface_alt,
                                        )
                                    })
                                })),
                        )
                        .child(small(ui, comparison.notes.join(" ")))
                        .into_element()
                        .into_element(),
                )
            })),
    )
}

fn row(ui: Ui, label: &'static str, baseline: f64, variant: f64, delta: String) -> AnyElement {
    view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(
            text(label)
                .text_size(px(SMALL))
                .text_color(move || color(ui, ui.palette().muted)),
        )
        .child(view().flex_1())
        .child(
            text(format!("{baseline:.1} → {variant:.1}"))
                .text_size(px(SMALL))
                .text_color(move || color(ui, ui.palette().text)),
        )
        .child(
            text(delta)
                .text_size(px(SMALL))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || color(ui, ui.palette().accent)),
        )
        .into_element()
}
