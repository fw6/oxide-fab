//! Process quality (Applied E3 semantics): SPC, fault detection, recipes,
//! run-to-run, Q-time and yield.
//!
//! Every number is a projection of `spc_sample` or `lot_event`; R2R is
//! explicitly read-only and says so.

use nsh_core::prelude::*;
use oxide_scene::Entity;

use super::{SMALL, TINY, button, card, color, dot, kv, small, strong};
use crate::client;
use crate::data::SpcBucket;
use crate::scene_glue;
use crate::state::Ui;

#[must_use]
pub fn panel(ui: Ui) -> AnyElement {
    view()
        .gap_3()
        .child(trace(ui))
        .child(spc(ui))
        .child(alarms(ui))
        .child(qtime(ui))
        .child(recipes(ui))
        .child(r2r(ui))
        .child(yield_card(ui))
        .into_element()
}

fn trace(ui: Ui) -> AnyElement {
    let trace = ui.trace;
    card(
        ui,
        "L3 事件 → outbox → 数据点",
        "event_id 可复制 · 三处可核对",
        dynamic(move || {
            let Some(trace) = trace.get() else {
                return Some(small(
                    ui,
                    "告警、Q-time 违规或 L3 定位后，这条链会出现在这里。".to_owned(),
                ));
            };
            let id = trace.event_id.clone();
            let copy = id.clone();
            let outbox = trace.outbox.as_ref().map_or_else(
                || "无 outbox 行".to_owned(),
                |outbox| {
                    outbox.published_at.clone().map_or_else(
                        || format!("待投递 · 尝试 {} 次", outbox.attempts),
                        |at| format!("已投递 {at}"),
                    )
                },
            );
            let tables = trace
                .greptime
                .iter()
                .map(|(table, rows)| format!("{table} {} 行", rows.len()))
                .collect::<Vec<_>>()
                .join(" · ");
            Some(
                view()
                    .gap_1()
                    .child(kv(ui, "event_id", id))
                    .child(kv(ui, "outbox", outbox))
                    .child(kv(
                        ui,
                        "GreptimeDB",
                        if tables.is_empty() {
                            "该事件不产指标行".to_owned()
                        } else {
                            tables
                        },
                    ))
                    .child(button(ui, "复制 event_id", false, move || {
                        set_clipboard(copy.clone());
                    }))
                    .into_element(),
            )
        }),
    )
}

fn spc(ui: Ui) -> AnyElement {
    let spc = ui.spc;
    let source = ui.spc_source;
    card(
        ui,
        "SPC · 控制图",
        "spc_sample 降采样 · 超限关联机台",
        view()
            .gap_2()
            .child(dynamic(move || {
                let buckets = spc.get();
                if buckets.is_empty() {
                    return Some(small(ui, "等待量测样本（S4 完成后出现）".to_owned()));
                }
                Some(chart(ui, &buckets))
            }))
            .child(dynamic(move || {
                let out: Vec<SpcBucket> = spc
                    .get()
                    .into_iter()
                    .filter(|bucket| bucket.avg_value > bucket.ucl || bucket.avg_value < bucket.lcl)
                    .rev()
                    .take(5)
                    .collect();
                if out.is_empty() {
                    return None;
                }
                Some(
                    view()
                        .gap_1()
                        .children(out.into_iter().map(|bucket| {
                            let equipment = bucket.equipment_id.clone();
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .px_1p5()
                                .py_1()
                                .rounded(px(5.0))
                                .bg(move || color(ui, ui.palette().surface_alt))
                                .on_press(move || {
                                    scene_glue::drill(
                                        ui,
                                        Entity::Tool {
                                            equipment_id: equipment.clone(),
                                        },
                                    );
                                })
                                .child(dot(ui.palette().bad))
                                .child(small(
                                    ui,
                                    format!("{} · {}", bucket.equipment_id, bucket.ts),
                                ))
                                .child(view().flex_1())
                                .child(small(
                                    ui,
                                    format!(
                                        "{:.2} nm · {:.1}–{:.1}",
                                        bucket.avg_value, bucket.lcl, bucket.ucl
                                    ),
                                ))
                                .into_element()
                        }))
                        .into_element()
                        .into_element(),
                )
            }))
            .child(
                text(move || {
                    format!(
                        "来源 {} · {} 个时间桶 · 规格 {:.0}–{:.0} nm",
                        source.get(),
                        spc.get().len(),
                        spc.get().last().map_or(0.0, |bucket| bucket.lcl),
                        spc.get().last().map_or(0.0, |bucket| bucket.ucl)
                    )
                })
                .text_size(px(TINY))
                .text_color(move || color(ui, ui.palette().muted)),
            ),
    )
}

fn chart(ui: Ui, buckets: &[SpcBucket]) -> AnyElement {
    let ucl = buckets.last().map_or(103.0, |bucket| bucket.ucl);
    let lcl = buckets.last().map_or(97.0, |bucket| bucket.lcl);
    let high = buckets
        .iter()
        .map(|bucket| bucket.max_value.max(bucket.ucl))
        .fold(ucl, f64::max)
        + 0.3;
    let low = buckets
        .iter()
        .map(|bucket| bucket.min_value.min(bucket.lcl))
        .fold(lcl, f64::min)
        - 0.3;
    let span = (high - low).max(0.1);
    let height = move |value: f64| ((value - low) / span * 86.0).clamp(2.0, 86.0) as f32;
    let items = buckets.to_vec();
    let last = buckets.last().map_or(0.0, |bucket| bucket.avg_value);
    view()
        .gap_1()
        .child(
            view()
                .h(px(96.0))
                .flex_row()
                .items_end()
                .gap_0p5()
                .p_1()
                .rounded(px(8.0))
                .bg(move || color(ui, ui.palette().surface_alt))
                .child(each(
                    move || items.clone(),
                    |bucket| bucket.ts.clone(),
                    move |bucket, _| {
                        let value = bucket.get().avg_value;
                        let out = value > ucl || value < lcl;
                        view()
                            .flex_1()
                            .min_w(px(2.0))
                            .h(px(height(value)))
                            .rounded(px(1.5))
                            .bg(move || {
                                color(
                                    ui,
                                    if out {
                                        ui.palette().bad
                                    } else {
                                        ui.palette().good
                                    },
                                )
                            })
                    },
                )),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(small(ui, format!("UCL {ucl:.1}")))
                .child(view().flex_1())
                .child(small(ui, format!("最近 {last:.2} nm")))
                .child(view().flex_1())
                .child(small(ui, format!("LCL {lcl:.1}"))),
        )
        .into_element()
}

fn alarms(ui: Ui) -> AnyElement {
    let analysis = ui.analysis;
    card(
        ui,
        "告警与 OCAP 路径",
        "equipment_down/up · 可追到 event_id",
        dynamic(move || {
            let Some(analysis) = analysis.get() else {
                return Some(small(ui, "读取 analytics…".to_owned()));
            };
            if analysis.alarms.is_empty() {
                return Some(small(
                    ui,
                    "无设备告警：equipment_down 会带着 event_id 出现在这里".to_owned(),
                ));
            }
            Some(
                view()
                    .gap_1()
                    .children(analysis.alarms.iter().rev().take(6).map(|alarm| {
                        let event_id = alarm.event_id.clone();
                        let tool = alarm.equipment_id.clone();
                        let down = alarm.kind == "down";
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .px_1p5()
                            .py_1()
                            .rounded(px(5.0))
                            .bg(move || color(ui, ui.palette().surface_alt))
                            .on_press(move || client::load_trace(ui, event_id.clone()))
                            .child(dot(if down {
                                ui.palette().bad
                            } else {
                                ui.palette().good
                            }))
                            .child(
                                text(format!("{} · {}", alarm.equipment_id, alarm.reason))
                                    .text_size(px(SMALL))
                                    .text_color(move || color(ui, ui.palette().text)),
                            )
                            .child(view().flex_1())
                            .child(button(ui, "机台", false, move || {
                                scene_glue::drill(
                                    ui,
                                    Entity::Tool {
                                        equipment_id: tool.clone(),
                                    },
                                );
                            }))
                            .into_element()
                    }))
                    .into_element()
                    .into_element(),
            )
        }),
    )
}

fn qtime(ui: Ui) -> AnyElement {
    let analysis = ui.analysis;
    card(
        ui,
        "Q-time · 连续工序上限",
        "lot_event.occurred_at + route_step.qtime_ms",
        dynamic(move || {
            let Some(analysis) = analysis.get() else {
                return Some(small(ui, "读取 analytics…".to_owned()));
            };
            let limits = analysis
                .qtime_limits
                .iter()
                .map(|limit| {
                    format!(
                        "S{}→S{} ≤ {:.0}s",
                        limit.from_step,
                        limit.to_step,
                        limit.limit_ms as f64 / 1000.0
                    )
                })
                .collect::<Vec<_>>()
                .join(" · ");
            let mut patterns: std::collections::BTreeMap<String, usize> =
                std::collections::BTreeMap::new();
            for violation in &analysis.qtime_violations {
                *patterns
                    .entry(format!("S{}→S{}", violation.from_step, violation.to_step))
                    .or_default() += 1;
            }
            let pattern_text = patterns
                .iter()
                .map(|(pair, count)| format!("{pair} × {count}"))
                .collect::<Vec<_>>()
                .join(" · ");
            let no_violations = analysis.qtime_violations.is_empty();
            Some(
                view()
                    .gap_1()
                    .child(small(ui, format!("上限 {limits}")))
                    .child(if no_violations {
                        small(ui, "无违规：所有跨工序等待都在上限内".to_owned())
                    } else {
                        view()
                            .gap_1()
                            .children(analysis.qtime_violations.iter().take(5).map(|violation| {
                                let event_id = violation.event_id.clone();
                                let tool = violation.equipment_id.clone();
                                view()
                                    .flex_row()
                                    .items_center()
                                    .gap_2()
                                    .px_1p5()
                                    .py_1()
                                    .rounded(px(5.0))
                                    .bg(move || color(ui, ui.palette().surface_alt))
                                    .on_press(move || client::load_trace(ui, event_id.clone()))
                                    .child(dot(ui.palette().warn))
                                    .child(
                                        text(format!(
                                            "{} · S{}→S{} · {:.0}s > {:.0}s · {} 片",
                                            violation.lot_id,
                                            violation.from_step,
                                            violation.to_step,
                                            violation.elapsed_ms as f64 / 1000.0,
                                            violation.limit_ms as f64 / 1000.0,
                                            violation.wafer_ids.len()
                                        ))
                                        .text_size(px(SMALL))
                                        .text_color(move || color(ui, ui.palette().text)),
                                    )
                                    .child(view().flex_1())
                                    .child(button(ui, "机台", false, move || {
                                        scene_glue::drill(
                                            ui,
                                            Entity::Tool {
                                                equipment_id: tool.clone(),
                                            },
                                        );
                                    }))
                                    .into_element()
                            }))
                            .into_element()
                    })
                    .child(if pattern_text.is_empty() {
                        small(ui, String::new())
                    } else {
                        small(ui, format!("跨班次重复模式：{pattern_text}"))
                    })
                    .into_element(),
            )
        }),
    )
}

fn recipes(ui: Ui) -> AnyElement {
    let analysis = ui.analysis;
    card(
        ui,
        "配方 · RMS 校验",
        "recipe.version/checksum ↔ track_out",
        dynamic(move || {
            let Some(analysis) = analysis.get() else {
                return Some(small(ui, "读取 analytics…".to_owned()));
            };
            Some(
                view()
                    .gap_1()
                    .children(analysis.recipes.iter().map(|recipe| {
                        let mismatch = recipe.mismatched_runs > 0;
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .px_1p5()
                            .py_1()
                            .rounded(px(5.0))
                            .bg(move || color(ui, ui.palette().surface_alt))
                            .child(dot(if mismatch {
                                ui.palette().bad
                            } else if recipe.runs > 0 {
                                ui.palette().good
                            } else {
                                ui.palette().muted
                            }))
                            .child(
                                text(format!(
                                    "{} v{} · {}",
                                    recipe.recipe_id,
                                    recipe.version,
                                    recipe.checksum.chars().take(10).collect::<String>()
                                ))
                                .text_size(px(SMALL))
                                .text_color(move || color(ui, ui.palette().text)),
                            )
                            .child(view().flex_1())
                            .child(small(
                                ui,
                                if mismatch {
                                    format!("{} 次不一致", recipe.mismatched_runs)
                                } else if recipe.runs > 0 {
                                    format!("{} 次执行均一致", recipe.runs)
                                } else {
                                    "尚未执行".to_owned()
                                },
                            ))
                            .into_element()
                    }))
                    .into_element()
                    .into_element(),
            )
        }),
    )
}

fn r2r(ui: Ui) -> AnyElement {
    let spc = ui.spc;
    card(
        ui,
        "Run-to-Run · 只读建议",
        "spc_sample 均值与规格中心 · 不写回设备",
        dynamic(move || {
            let buckets = spc.get();
            let last = buckets.last()?;
            let target = f64::midpoint(last.ucl, last.lcl);
            let prior = buckets
                .iter()
                .rev()
                .nth(1)
                .map_or(last.avg_value, |bucket| bucket.avg_value);
            let suggestion = target - last.avg_value;
            Some(
                view()
                    .gap_1()
                    .child(kv(ui, "规格中心", format!("{target:.2} nm")))
                    .child(kv(ui, "上次均值", format!("{prior:.2} nm")))
                    .child(kv(ui, "本次均值", format!("{:.2} nm", last.avg_value)))
                    .child(kv(
                        ui,
                        "建议调整",
                        format!("{suggestion:+.2} nm（只读展示）"),
                    ))
                    .child(small(
                        ui,
                        "R2R 只给出调整量，不闭环写回设备；输入全部来自 spc_sample。".to_owned(),
                    ))
                    .into_element(),
            )
        }),
    )
}

fn yield_card(ui: Ui) -> AnyElement {
    let analysis = ui.analysis;
    let selected_wafer = ui.selected_wafer;
    let detail = ui.detail;
    card(
        ui,
        "缺陷与良率",
        "wafer.defect_info / wafer_scrapped",
        dynamic(move || {
            let Some(analysis) = analysis.get() else {
                return Some(small(ui, "读取 analytics…".to_owned()));
            };
            let defects = analysis
                .kpi
                .defects
                .iter()
                .map(|defect| format!("{} {}", defect.class, defect.count))
                .collect::<Vec<_>>()
                .join(" · ");
            let source = selected_wafer.get().and_then(|wafer_id| {
                detail.get().and_then(|detail| {
                    detail
                        .genealogy
                        .iter()
                        .find(|entry| entry.wafer_id == wafer_id)
                        .and_then(|entry| entry.steps.last().map(|step| step.equipment_id.clone()))
                })
            });
            Some(
                view()
                    .gap_1()
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(strong(
                                ui,
                                format!(
                                    "FPY {:.1}%",
                                    analysis.kpi.first_pass_yield.unwrap_or(0.0) * 100.0
                                ),
                            ))
                            .child(small(
                                ui,
                                format!(
                                    "报废 {} / 出现 {} 片",
                                    analysis.kpi.wafers_scrapped, analysis.kpi.wafers_created
                                ),
                            )),
                    )
                    .child(small(ui, format!("缺陷分类：{defects}")))
                    .child(dynamic(move || {
                        source.clone().map(|tool| {
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(small(ui, "缺陷来源层".to_owned()))
                                .child(button(ui, "定位来源机台", false, move || {
                                    scene_glue::drill(
                                        ui,
                                        Entity::Tool {
                                            equipment_id: tool.clone(),
                                        },
                                    );
                                }))
                        })
                    }))
                    .into_element(),
            )
        }),
    )
}
