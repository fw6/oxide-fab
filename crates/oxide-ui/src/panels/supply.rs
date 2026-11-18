//! Supply chain: plan against actual, and the early warning that comes from
//! the same two numbers.
//!
//! Planned durations are `route_step.duration_ms`; actual ones are measured
//! per step from `track_out`. No external solver and no due-date table: the
//! warning is the deviation the log already shows.

use nsh_core::prelude::*;

use super::{SMALL, TINY, badge, bar, card, color, dot, kv, small};
use crate::data::duration_text;
use crate::state::Ui;

#[must_use]
pub fn panel(ui: Ui) -> AnyElement {
    view()
        .gap_3()
        .child(plan(ui))
        .child(delivery(ui))
        .into_element()
}

fn plan(ui: Ui) -> AnyElement {
    let analysis = ui.analysis;
    card(
        ui,
        "计划节拍 vs 实际节拍",
        "route_step.duration_ms ↔ track_out.duration_ms",
        dynamic(move || {
            let Some(analysis) = analysis.get() else {
                return Some(small(ui, "读取 analytics…".to_owned()));
            };
            if analysis.kpi.step_cycles.is_empty() {
                return Some(small(ui, "等待工序样本".to_owned()));
            }
            Some(
                view()
                    .gap_1()
                    .children(analysis.kpi.step_cycles.iter().map(|step| {
                        let actual = step.actual_mean_ms.unwrap_or(0.0);
                        let ratio = if step.planned_ms > 0 {
                            actual / step.planned_ms as f64
                        } else {
                            0.0
                        };
                        let late = ratio > 1.15 && step.samples > 0;
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(dot(if late {
                                ui.palette().warn
                            } else {
                                ui.palette().good
                            }))
                            .child(
                                text(format!("S{}", step.step_no))
                                    .text_size(px(SMALL))
                                    .text_color(move || color(ui, ui.palette().muted)),
                            )
                            .child(small(
                                ui,
                                format!(
                                    "计划 {} → 实际 {}（{} 次）",
                                    duration_text(step.planned_ms),
                                    if step.samples > 0 {
                                        duration_text(actual.round() as i64)
                                    } else {
                                        "—".to_owned()
                                    },
                                    step.samples
                                ),
                            ))
                            .child(view().flex_1())
                            .child(bar(
                                ui,
                                ratio as f32 / 2.0,
                                if late {
                                    ui.palette().warn
                                } else {
                                    ui.palette().good
                                },
                                70.0,
                            ))
                            .child(
                                text(format!("{:.0}%", ratio * 100.0))
                                    .text_size(px(TINY))
                                    .text_color(move || color(ui, ui.palette().text)),
                            )
                            .into_element()
                    }))
                    .into_element()
                    .into_element(),
            )
        }),
    )
}

fn delivery(ui: Ui) -> AnyElement {
    let analysis = ui.analysis;
    card(
        ui,
        "交付偏差 · 早期预警",
        "批次周期 = 首事件 → lot_completed；计划 = 四道工序之和",
        dynamic(move || {
            let Some(analysis) = analysis.get() else {
                return Some(small(ui, "读取 analytics…".to_owned()));
            };
            let planned: i64 = analysis
                .kpi
                .step_cycles
                .iter()
                .map(|step| step.planned_ms)
                .sum();
            if analysis.kpi.lot_cycles.is_empty() {
                return Some(small(
                    ui,
                    format!("尚无完成批次；计划周期 {}", duration_text(planned)),
                ));
            }
            let worst = analysis
                .kpi
                .lot_cycles
                .iter()
                .map(|cycle| cycle.cycle_ms as f64 / planned.max(1) as f64)
                .fold(0.0_f64, f64::max);
            Some(
                view()
                    .gap_1()
                    .child(kv(ui, "计划周期", duration_text(planned)))
                    .child(dynamic(move || {
                        if worst <= 1.15 {
                            Some(small(
                                ui,
                                "无交付偏差预警：实际周期在计划 +15% 以内".to_owned(),
                            ))
                        } else {
                            Some(
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(badge(
                                        ui,
                                        format!("偏差 {:.0}%", (worst - 1.0) * 100.0),
                                        ui.palette().bad,
                                        ui.palette().surface_alt,
                                    ))
                                    .child(small(ui, "存在超计划批次，见下方逐批周期".to_owned()))
                                    .into_element(),
                            )
                        }
                    }))
                    .child(
                        view()
                            .gap_1()
                            .children(analysis.kpi.lot_cycles.iter().take(6).map(|cycle| {
                                let ratio = cycle.cycle_ms as f64 / planned.max(1) as f64;
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        text(cycle.lot_id.clone())
                                            .text_size(px(SMALL))
                                            .text_color(move || color(ui, ui.palette().text)),
                                    )
                                    .child(view().flex_1())
                                    .child(small(ui, duration_text(cycle.cycle_ms)))
                                    .child(
                                        text(format!("{:.0}%", ratio * 100.0))
                                            .text_size(px(TINY))
                                            .text_color(move || {
                                                color(
                                                    ui,
                                                    if ratio > 1.15 {
                                                        ui.palette().warn
                                                    } else {
                                                        ui.palette().good
                                                    },
                                                )
                                            }),
                                    )
                                    .into_element()
                            })),
                    )
                    .into_element(),
            )
        }),
    )
}
