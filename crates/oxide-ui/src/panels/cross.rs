//! Cross-cutting: the event trail, the component monitor, the outbox, KPIs,
//! the drills and the demo's exports.
//!
//! This is the panel that proves a value's provenance: the event id at L3, its
//! outbox row and the `GreptimeDB` rows it produced.

use nsh_core::prelude::*;

use super::{SMALL, TINY, button, button_dyn, card, color, divider, dot, kv, small, small_dyn};
use crate::client;
use crate::data::duration_text;
use crate::demo;
use crate::state::Ui;

#[must_use]
pub fn panel(ui: Ui) -> AnyElement {
    view()
        .gap_3()
        .child(trace(ui))
        .child(monitor(ui))
        .child(outbox(ui))
        .child(kpi(ui))
        .child(drills(ui))
        .child(settings(ui))
        .child(secs(ui))
        .into_element()
}

fn trace(ui: Ui) -> AnyElement {
    let trace = ui.trace;
    card(
        ui,
        "L3 事件 → outbox → 数据点",
        "lot_event.event_id / outbox / GreptimeDB",
        dynamic(move || {
            let Some(trace) = trace.get() else {
                return Some(small(
                    ui,
                    "点一个告警或 Q-time 违规，或在左侧输入 event_id，即可看到这条链。".to_owned(),
                ));
            };
            let id = trace.event_id.clone();
            let event_id_for_copy = id.clone();
            let published = trace
                .outbox
                .as_ref()
                .and_then(|outbox| outbox.published_at.clone());
            let attempts = trace.outbox.as_ref().map_or(0, |outbox| outbox.attempts);
            let tables = trace
                .greptime
                .iter()
                .map(|(table, rows)| format!("{table} {} 行", rows.len()))
                .collect::<Vec<_>>()
                .join(" · ");
            Some(
                view()
                    .gap_1()
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                text(format!(
                                    "{} · {}",
                                    trace.event.event.kind, trace.event.event.lot_id
                                ))
                                .text_size(px(SMALL))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || color(ui, ui.palette().text)),
                            )
                            .child(view().flex_1())
                            .child(button(ui, "复制 event_id", false, move || {
                                set_clipboard(event_id_for_copy.clone());
                            })),
                    )
                    .child(kv(ui, "event_id", id))
                    .child(kv(
                        ui,
                        "outbox",
                        published.map_or_else(
                            || format!("待投递 · 尝试 {attempts} 次"),
                            |at| format!("已投递 {at}"),
                        ),
                    ))
                    .child(kv(
                        ui,
                        "GreptimeDB",
                        if tables.is_empty() {
                            "无派生行（该事件不产指标）".to_owned()
                        } else {
                            tables
                        },
                    ))
                    .child(dynamic(move || {
                        trace
                            .outbox
                            .as_ref()
                            .and_then(|outbox| outbox.last_error.clone())
                            .map(|error| {
                                small(ui, format!("最后一次投递错误：{error}"))
                            })
                    }))
                    .child(small(
                        ui,
                        "追这条链的口径：PG 为事实来源，outbox 至少一次投递，Greptime 行以 event_id 幂等。"
                            .to_owned(),
                    ))
            .into_element())
        }),
    )
}

fn monitor(ui: Ui) -> AnyElement {
    let monitor = ui.monitor;
    card(
        ui,
        "组件巡查 · SmartFactory Monitor",
        "PG / GreptimeDB / 对象存储 / oxide-api / outbox 投递者",
        view()
            .gap_1()
            .child(dynamic(move || {
                let Some(monitor) = monitor.get() else {
                    return Some(small(ui, "读取 monitor…".to_owned()));
                };
                Some(
                    view()
                        .gap_1()
                        .children(monitor.components.iter().map(|component| {
                            let ok = component.status == "up";
                            let warn = component.status == "degraded";
                            view()
                                .flex_row()
                                .items_center()
                                .gap_2()
                                .child(dot(if ok {
                                    ui.palette().good
                                } else if warn {
                                    ui.palette().warn
                                } else {
                                    ui.palette().bad
                                }))
                                .child(
                                    text(component.name.clone())
                                        .text_size(px(SMALL))
                                        .text_color(move || color(ui, ui.palette().text)),
                                )
                                .child(view().flex_1())
                                .child(small(
                                    ui,
                                    format!("{} · {} ms", component.detail, component.latency_ms),
                                ))
                                .into_element()
                        }))
                        .into_element()
                        .into_element(),
                )
            }))
            .child(button(ui, "重新巡查", false, move || {
                spawn(async move {
                    let _ = client::load_monitor(ui).await;
                });
            })),
    )
}

fn outbox(ui: Ui) -> AnyElement {
    let chaos = ui.chaos;
    let pending = ui.outbox_pending;
    let published = ui.outbox_published;
    let greptime_ok = ui.greptime_ok;
    card(
        ui,
        "消息总线 · outbox 积压与追平",
        "outbox WHERE published_at IS NULL → published_event",
        view()
            .gap_2()
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_3()
                    .child(
                        view()
                            .gap_0p5()
                            .child(
                                text(move || format!("{}", pending.get()))
                                    .text_size(px(24.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(move || {
                                        color(
                                            ui,
                                            if pending.get() > 0 {
                                                ui.palette().warn
                                            } else {
                                                ui.palette().good
                                            },
                                        )
                                    }),
                            )
                            .child(small(ui, "待投递".to_owned())),
                    )
                    .child(
                        view()
                            .gap_0p5()
                            .child(
                                text(move || format!("{}", published.get()))
                                    .text_size(px(18.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(move || color(ui, ui.palette().text)),
                            )
                            .child(small(ui, "已投递".to_owned())),
                    )
                    .child(view().flex_1())
                    .child(small(
                        ui,
                        if greptime_ok.get() {
                            "GreptimeDB 在线：追平中会按序下降".to_owned()
                        } else {
                            "GreptimeDB 断流：事件只积压在 PG，恢复后按序追平".to_owned()
                        },
                    )),
            )
            .child(dynamic(move || {
                let samples = chaos.get();
                if samples.is_empty() {
                    return Some(small(
                        ui,
                        if pending.get() == 0 {
                            "无积压：outbox 已追平。运行 ./scripts/chaos.sh 时这里会画出积压曲线"
                                .to_owned()
                        } else {
                            "有积压但尚无采样：投递者正在追平".to_owned()
                        },
                    ));
                }
                let peak = samples.iter().map(|sample| sample.pending).max().unwrap_or(1).max(1);
                let mut peak_at = 0_i64;
                let mut drained: Option<i64> = None;
                let mut started: Option<i64> = None;
                for sample in &samples {
                    if sample.pending > 0 {
                        peak_at = peak_at.max(sample.pending);
                        started.get_or_insert(sample.at_ms);
                    } else if let Some(at) = started.take() {
                        drained = Some(sample.at_ms - at);
                    }
                }
                let items = samples.clone();
                Some(
                    view()
                        .gap_1()
                        .child(
                            view()
                                .h(px(46.0))
                                .flex_row()
                                .items_end()
                                .gap_0p5()
                                .p_1()
                                .rounded(px(7.0))
                                .bg(move || color(ui, ui.palette().surface_alt))
                                .child(each(
                                    move || items.clone(),
                                    |sample| sample.at_ms,
                                    move |sample, _| {
                                        let height = (sample.get().pending as f32 / peak as f32
                                            * 34.0)
                                            .max(1.0);
                                        view()
                                            .flex_1()
                                            .min_w(px(2.0))
                                            .h(px(height))
                                            .bg(move || {
                                                color(
                                                    ui,
                                                    if sample.get().pending > 0 {
                                                        ui.palette().warn
                                                    } else {
                                                        ui.palette().good
                                                    },
                                                )
                                            })
                                    },
                                )),
                        )
                        .child(small(
                            ui,
                            match drained {
                                Some(ms) => format!(
                                    "峰值积压 {peak_at} · 追平 {:.1}s · 零丢失（至少一次 + event_id 幂等）",
                                    ms as f64 / 1000.0
                                ),
                                None => format!("峰值积压 {peak_at} · 尚在追平"),
                            },
                        ))
                        .into_element()
                .into_element())
            })),
    )
}

fn kpi(ui: Ui) -> AnyElement {
    let analysis = ui.analysis;
    card(
        ui,
        "KPI · 口径化指标",
        "lot_event 派生 · 每个数字标注公式",
        dynamic(move || {
            let Some(analysis) = analysis.get() else {
                return Some(small(ui, "读取 analytics…".to_owned()));
            };
            let kpi = &analysis.kpi;
            let downtime: i64 = kpi.tools.iter().map(|tool| tool.down_ms).sum();
            Some(
                view()
                    .gap_1()
                    .child(kv(
                        ui,
                        "平均周期",
                        kpi.mean_cycle_time_ms
                            .map_or_else(|| "—".to_owned(), |ms| duration_text(ms as i64)),
                    ))
                    .child(kv(
                        ui,
                        "产出 / 小时",
                        kpi.throughput_lots_per_hour
                            .map_or_else(|| "—".to_owned(), |value| format!("{value:.2}")),
                    ))
                    .child(kv(
                        ui,
                        "FPY",
                        kpi.first_pass_yield.map_or_else(
                            || "—".to_owned(),
                            |value| format!("{:.1}%", value * 100.0),
                        ),
                    ))
                    .child(kv(ui, "WIP", format!("{} 批", kpi.wip_lots)))
                    .child(kv(ui, "停机合计", duration_text(downtime)))
                    .child(divider(ui))
                    .child(view().gap_0p5().children(analysis.notes.iter().map(|note| {
                        view()
                            .flex_row()
                            .gap_2()
                            .child(
                                text(note.metric.clone())
                                    .text_size(px(TINY))
                                    .text_color(move || color(ui, ui.palette().accent)),
                            )
                            .child(small(ui, format!("{} · {}", note.source, note.formula)))
                            .into_element()
                    })))
                    .into_element(),
            )
        }),
    )
}

fn drills(ui: Ui) -> AnyElement {
    let backup = ui.backup;
    let restore = ui.restore;
    card(
        ui,
        "备份与恢复演练 · RPO / RTO",
        "backup.sh / restore.sh / chaos.sh 产物",
        view()
            .gap_1()
            .child(dynamic(move || {
                let backup = backup.get();
                Some(small(
                    ui,
                    match backup {
                        Some(backup) => format!(
                            "备份 {} · base {} · WAL {} · Greptime {}",
                            backup.taken_at.unwrap_or_default(),
                            backup.pg_full_backup.unwrap_or_default(),
                            backup.wal_continuity.unwrap_or_default(),
                            backup.greptime_snapshot.unwrap_or_default()
                        ),
                        None => "尚无备份：运行 ./scripts/backup.sh".to_owned(),
                    },
                ))
            }))
            .child(dynamic(move || {
                let restore = restore.get();
                Some(small(
                    ui,
                    match restore {
                        Some(restore) => format!(
                            "PITR {} · lots {} · events {} · Greptime {} 行 · RTO {:.1}s · RPO 0（WAL 连续）",
                            restore.target_time.unwrap_or_default(),
                            restore.lots.unwrap_or_default(),
                            restore.events.unwrap_or_default(),
                            restore.greptime_rows.unwrap_or_default(),
                            restore.elapsed_ms.unwrap_or_default() as f64 / 1000.0
                        ),
                        None => "尚无恢复演练：运行 ./scripts/restore.sh".to_owned(),
                    },
                ))
            }))
            .child(small(
                ui,
                "RTO 含容器/进程重启；RPO 由 WAL 连续性与演练校验给出，不在客户端计算。".to_owned(),
            )),
    )
}

fn settings(ui: Ui) -> AnyElement {
    let reduce = ui.reduce_motion;
    let dense = ui.dense;
    card(
        ui,
        "演示、导出与可配置项",
        "写操作均标注 演示触发",
        view()
            .gap_1()
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(small_dyn(ui, move || ui.demo.get().label().to_owned()))
                    .child(view().flex_1())
                    .child(button(
                        ui,
                        if demo::running(ui) {
                            "停止演示"
                        } else {
                            "开始演示"
                        },
                        true,
                        move || demo::toggle(ui),
                    )),
            )
            .child(
                view()
                    .flex_row()
                    .flex_wrap()
                    .gap_1()
                    .child(button(ui, "导出场景快照", false, move || {
                        client::export_scene(ui);
                    }))
                    .child(button(ui, "导出批次追溯", false, move || {
                        client::export_lot(ui);
                    }))
                    .child(button(ui, "导出 chaos 报告", false, move || {
                        client::export_chaos(ui);
                    }))
                    .child(button(
                        ui,
                        "启动本地后端（演示触发）",
                        false,
                        move || {
                            client::start_backend(ui);
                        },
                    )),
            )
            .child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(button_dyn(
                        ui,
                        move || {
                            if reduce.get() {
                                "减少动效 ✓".to_owned()
                            } else {
                                "减少动效".to_owned()
                            }
                        },
                        false,
                        move || reduce.update(|value| *value = !*value),
                    ))
                    .child(button_dyn(
                        ui,
                        move || {
                            if dense.get() {
                                "标签密度：标准".to_owned()
                            } else {
                                "标签密度：精简".to_owned()
                            }
                        },
                        false,
                        move || dense.update(|value| *value = !*value),
                    ))
                    .child(small_dyn(ui, move || format!("后端：{}", ui.backend.get()))),
            )
            .child(button(
                ui,
                "重放派生数据（演示触发）",
                false,
                move || {
                    client::action(
                        ui,
                        "/admin/replay",
                        serde_json::json!({"truncate": true}),
                        "Replay 派生数据",
                    );
                },
            )),
    )
}

fn secs(ui: Ui) -> AnyElement {
    let secs = ui.secs;
    card(
        ui,
        "SECS/GEM 假机台",
        "HSMS-SS · E4/E5/E30/E37",
        dynamic(move || {
            let Some(status) = secs.get() else {
                return Some(small(ui, "SECS/GEM 已关闭".to_owned()));
            };
            Some(
                view()
                    .gap_1()
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(dot(if status.host_connected {
                                ui.palette().good
                            } else {
                                ui.palette().bad
                            }))
                            .child(small(
                                ui,
                                if status.host_connected {
                                    "链路已建立".to_owned()
                                } else {
                                    "链路断开".to_owned()
                                },
                            ))
                            .child(view().flex_1())
                            .child(button(ui, "断链 1.5s", false, move || {
                                client::action(
                                    ui,
                                    "/api/secs/drop",
                                    serde_json::json!({}),
                                    "SECS 断链",
                                );
                            })),
                    )
                    .child(small(
                        ui,
                        format!(
                            "S1F1 {} · S6F11 {} · S5F1 {} · S7F3 {} · S7F6 {}",
                            status.s1f1_ok,
                            status.s6f11_sent,
                            status.s5f1_sent,
                            status.s7f3_uploaded,
                            status.s7f6_verified
                        ),
                    ))
                    .child(small(
                        ui,
                        format!(
                            "补传缓冲 {} · 已补传 {} · 断链 {} 次",
                            status.spool_depth, status.spooled_replayed, status.link_drops
                        ),
                    ))
                    .into_element()
                    .into_element(),
            )
        }),
    )
}
