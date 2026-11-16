//! The oxide-fab dashboard, written entirely in Rust for nsh.
//!
//! One screen shows the four process steps, the tool lights, the lots and
//! their wafer genealogy, the SPC chart from GreptimeDB's downsampled series,
//! the backup/restore drill results and the outbox backlog during a chaos run.
//!
//! Data flow: an initial `/api/snapshot` seeds every signal; afterwards the
//! dashboard pulls `/api/events?since=<seq>` every 600 ms and folds the new
//! events into the lots and tools it already holds. A tool light changes only
//! when an event about that tool arrives, never because a frame was drawn. A
//! full snapshot every eight polls reconciles the aggregates (queue lengths,
//! SPC, backup status), and a dropped connection reconnects and pulls the
//! increment it missed.

pub mod data;

use std::time::Duration;

use nsh_base::{button, theme};
use nsh_core::prelude::*;
use serde::de::DeserializeOwned;

use data::{
    BackupStatus, ChaosSample, LotDetail, LotView, RestoreStatus, SecsStatus, Snapshot, SpcBucket,
    StepView, ToolView, clock_text, duration_text,
};

const FONT: &str = "WenQuanYi Micro Hei, Noto Sans CJK SC, Source Han Sans SC, PingFang SC, Microsoft YaHei, sans-serif";
const GREEN: u32 = 0x22c55e;
const AMBER: u32 = 0xf59e0b;
const RED: u32 = 0xef4444;
const BLUE: u32 = 0x60a5fa;

/// Everything the panels read and write. Every field is a `Copy` signal
/// handle, so the struct itself is copied into each panel.
#[derive(Clone, Copy)]
pub struct Ui {
    steps: Signal<Vec<StepView>>,
    tools: Signal<Vec<ToolView>>,
    lots: Signal<Vec<LotView>>,
    spc: Signal<Vec<SpcBucket>>,
    spc_source: Signal<String>,
    chaos: Signal<Vec<ChaosSample>>,
    secs: Signal<Option<SecsStatus>>,
    backup: Signal<Option<BackupStatus>>,
    restore: Signal<Option<RestoreStatus>>,
    outbox_pending: Signal<i64>,
    outbox_published: Signal<i64>,
    events_total: Signal<i64>,
    published_events: Signal<i64>,
    now_ms: Signal<i64>,
    running: Signal<bool>,
    speed: Signal<f64>,
    last_seq: Signal<i64>,
    connected: Signal<bool>,
    note: Signal<Option<String>>,
    selected_lot: Signal<Option<String>>,
    detail: Signal<Option<LotDetail>>,
    selected_wafer: Signal<Option<String>>,
    greptime_ok: Signal<bool>,
}

impl Ui {
    fn new() -> Self {
        Self {
            steps: signal(Vec::new()),
            tools: signal(Vec::new()),
            lots: signal(Vec::new()),
            spc: signal(Vec::new()),
            spc_source: signal(String::from("raw")),
            chaos: signal(Vec::new()),
            secs: signal(None),
            backup: signal(None),
            restore: signal(None),
            outbox_pending: signal(0),
            outbox_published: signal(0),
            events_total: signal(0),
            published_events: signal(0),
            now_ms: signal(0),
            running: signal(false),
            speed: signal(10.0),
            last_seq: signal(0),
            connected: signal(false),
            note: signal(None),
            selected_lot: signal(None),
            detail: signal(None),
            selected_wafer: signal(None),
            greptime_ok: signal(false),
        }
    }
}

/// The app's UI set (what a TSX entry may import from `oxide-ui/native`).
#[native_module("oxide-ui/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

/// The application's root component.
#[component]
pub fn App() -> impl IntoElement {
    let ui = Ui::new();
    bootstrap(ui);
    view()
        .flex_1()
        .font_family(FONT)
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .child(header(ui))
        .child(
            view()
                .flex_1()
                .p_3()
                .gap_3()
                .flex_row()
                .flex_wrap()
                .items_start()
                .child(left_column(ui))
                .child(right_column(ui)),
        )
        .child(footer(ui))
}

// ----------------------------------------------------------------------
// data
// ----------------------------------------------------------------------

fn api_base() -> String {
    std::env::var("OXIDE_API_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_owned())
}

async fn get_json<T: DeserializeOwned>(path: &str) -> Result<T, String> {
    let response = fetch(format!("{}{}", api_base(), path)).await?;
    if !response.ok() {
        return Err(format!("HTTP {} {}", response.status(), path));
    }
    serde_json::from_str(&response.text()).map_err(|error| error.to_string())
}

fn bootstrap(ui: Ui) {
    spawn(async move {
        let _ = load_snapshot(ui).await;
        let mut ticks: u64 = 0;
        loop {
            timer(Duration::from_millis(600)).await;
            match pull_events(ui).await {
                Ok(true) => {
                    ui.connected.set(true);
                }
                Ok(false) => {
                    let _ = load_snapshot(ui).await;
                }
                Err(error) => {
                    ui.connected.set(false);
                    ui.note.set(Some(format!("连接中断，正在重连：{error}")));
                }
            }
            // Aggregates (queues, SPC, backups) are reconciled less often than
            // the per-tool event stream.
            ticks += 1;
            if ticks % 8 == 0 {
                let _ = load_snapshot(ui).await;
            }
        }
    });
}

async fn load_snapshot(ui: Ui) -> Result<(), String> {
    let snapshot: Snapshot = get_json("/api/snapshot").await?;
    ui.steps.set(snapshot.steps);
    ui.tools.set(snapshot.equipment);
    ui.lots.set(snapshot.lots);
    ui.spc.set(snapshot.spc);
    ui.spc_source.set(snapshot.spc_source);
    ui.chaos.set(snapshot.chaos);
    ui.secs.set(snapshot.secs);
    ui.backup.set(if snapshot.backup.taken_at.is_some() {
        Some(snapshot.backup)
    } else {
        None
    });
    ui.restore.set(snapshot.restore);
    ui.outbox_pending.set(snapshot.db.outbox_pending);
    ui.outbox_published.set(snapshot.db.outbox_published);
    ui.events_total.set(snapshot.db.events);
    ui.published_events.set(snapshot.db.published_events);
    ui.now_ms.set(snapshot.sim.now_ms);
    ui.running.set(snapshot.sim.running);
    ui.speed.set(snapshot.sim.speed);
    ui.last_seq.set(snapshot.last_seq);
    ui.greptime_ok.set(snapshot.greptime_ok);
    ui.connected.set(true);
    Ok(())
}

async fn pull_events(ui: Ui) -> Result<bool, String> {
    let since = ui.last_seq.get_untracked();
    let page: data::EventsResponse =
        get_json(&format!("/api/events?since={since}&limit=500")).await?;
    ui.last_seq.set(page.last_seq);
    if page.events.is_empty() {
        return Ok(true);
    }
    let mut lots = ui.lots.get_untracked();
    let mut tools = ui.tools.get_untracked();
    for persisted in &page.events {
        if !data::apply_event(&mut lots, &mut tools, &persisted.event) {
            return Ok(false);
        }
    }
    ui.lots.set(lots);
    ui.tools.set(tools);
    Ok(true)
}

fn post(ui: Ui, path: &str, body: &str) {
    let url = format!("{}{}", api_base(), path);
    let body = body.to_owned();
    spawn(async move {
        let request = HttpRequest::post(url, body).header("Content-Type", "application/json");
        match fetch(request).await {
            Ok(response) if response.ok() => {
                ui.note.set(Some("指令已下发".to_owned()));
                let _ = pull_events(ui).await;
                let _ = load_snapshot(ui).await;
            }
            Ok(response) => ui.note.set(Some(format!("HTTP {}", response.status()))),
            Err(error) => ui.note.set(Some(error)),
        }
    });
}

fn load_lot(ui: Ui, lot_id: String) {
    ui.selected_lot.set(Some(lot_id.clone()));
    ui.selected_wafer.set(None);
    spawn(async move {
        match get_json::<LotDetail>(&format!("/api/lots/{lot_id}")).await {
            Ok(detail) => ui.detail.set(Some(detail)),
            Err(error) => ui.note.set(Some(error)),
        }
    });
}

// ----------------------------------------------------------------------
// chrome
// ----------------------------------------------------------------------

fn header(ui: Ui) -> impl IntoElement {
    let connected = ui.connected;
    let running = ui.running;
    let speed = ui.speed;
    view()
        .w_full()
        .px_4()
        .py_3()
        .flex_row()
        .items_center()
        .gap_3()
        .flex_wrap()
        .bg(move || theme().card)
        .border_b_1()
        .border_color(move || theme().border)
        .child(
            view()
                .gap_0p5()
                .child(
                    text("氧化层 · oxide-fab")
                        .text_size(px(20.0))
                        .font_weight(FontWeight::BOLD),
                )
                .child(
                    text("Rust 仿真产线 · PG 事实来源 · Greptime 可重建")
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(view().flex_1())
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1p5()
                .child(view().size(px(9.0)).rounded(px(5.0)).bg(move || {
                    if connected.get() {
                        rgb(GREEN)
                    } else {
                        rgb(RED)
                    }
                }))
                .child(
                    text(move || {
                        if connected.get() {
                            "实时连接".to_owned()
                        } else {
                            "重连中".to_owned()
                        }
                    })
                    .text_size(px(12.0)),
                ),
        )
        .child(
            text(move || clock_text(ui.now_ms.get()))
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_1()
                .child(
                    button()
                        .px(px(10.0))
                        .py(px(6.0))
                        .rounded(px(8.0))
                        .bg(move || theme().primary)
                        .active(|style| style.opacity(0.7))
                        .on_press(move || {
                            post(
                                ui,
                                if running.get() {
                                    "/api/sim/pause"
                                } else {
                                    "/api/sim/play"
                                },
                                "",
                            )
                        })
                        .child(
                            text(move || {
                                if running.get() {
                                    "暂停".to_owned()
                                } else {
                                    "运行".to_owned()
                                }
                            })
                            .text_size(px(12.0))
                            .text_color(move || theme().primary_foreground),
                        ),
                )
                .child(
                    button()
                        .px(px(10.0))
                        .py(px(6.0))
                        .rounded(px(8.0))
                        .bg(move || theme().muted)
                        .active(|style| style.opacity(0.7))
                        .on_press(move || post(ui, "/api/sim/step", ""))
                        .child(text("单步").text_size(px(12.0))),
                )
                .child(
                    button()
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(8.0))
                        .bg(move || theme().muted)
                        .active(|style| style.opacity(0.7))
                        .on_press(move || {
                            let next = (speed.get() / 2.0).max(0.5);
                            post(ui, &format!("/api/sim/speed/{next}"), "")
                        })
                        .child(text("−").text_size(px(12.0))),
                )
                .child(
                    text(move || format!("×{:.0}", speed.get()))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    button()
                        .px(px(8.0))
                        .py(px(6.0))
                        .rounded(px(8.0))
                        .bg(move || theme().muted)
                        .active(|style| style.opacity(0.7))
                        .on_press(move || {
                            let next = (speed.get() * 2.0).min(100.0);
                            post(ui, &format!("/api/sim/speed/{next}"), "")
                        })
                        .child(text("+").text_size(px(12.0))),
                ),
        )
}

fn footer(ui: Ui) -> impl IntoElement {
    let note = ui.note;
    view()
        .w_full()
        .px_4()
        .py_2()
        .flex_row()
        .items_center()
        .gap_3()
        .bg(move || theme().card)
        .border_t_1()
        .border_color(move || theme().border)
        .child(
            text(move || {
                format!(
                    "事件 {} · 已投递 {} · outbox 积压 {} · Greptime {}",
                    ui.events_total.get(),
                    ui.published_events.get(),
                    ui.outbox_pending.get(),
                    if ui.greptime_ok.get() {
                        "在线"
                    } else {
                        "不可达（积压在 PG）"
                    }
                )
            })
            .text_size(px(11.0))
            .text_color(move || theme().muted_foreground),
        )
        .child(
            text(move || note.get().unwrap_or_default())
                .text_size(px(11.0))
                .text_color(move || theme().muted_foreground),
        )
}

fn left_column(ui: Ui) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(420.0))
        .gap_3()
        .child(steps_panel(ui))
        .child(tools_panel(ui))
        .child(lots_panel(ui))
}

fn right_column(ui: Ui) -> impl IntoElement {
    view()
        .flex_1()
        .min_w(px(380.0))
        .gap_3()
        .child(spc_panel(ui))
        .child(recovery_panel(ui))
        .child(ops_panel(ui))
        .child(secs_panel(ui))
}

fn card(title: &'static str, badge: Option<String>, content: impl IntoElement) -> impl IntoElement {
    view()
        .w_full()
        .p_3()
        .gap_2()
        .rounded(px(12.0))
        .bg(move || theme().card)
        .border_1()
        .border_color(move || theme().border)
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(title)
                        .text_size(px(14.0))
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(view().flex_1())
                .child(
                    text(badge.unwrap_or_default())
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(content)
}

fn dot(color: u32) -> impl IntoElement {
    view().size(px(10.0)).rounded(px(5.0)).bg(rgb(color))
}

fn light_color(light: &str) -> u32 {
    match light {
        "red" => RED,
        "amber" => AMBER,
        _ => GREEN,
    }
}

fn state_color(state: &str) -> u32 {
    match state {
        "completed" => GREEN,
        "processing" => BLUE,
        "on_hold" => AMBER,
        "split" | "merged" => AMBER,
        _ => 0x94a3b8,
    }
}

// ----------------------------------------------------------------------
// panels
// ----------------------------------------------------------------------

fn steps_panel(ui: Ui) -> impl IntoElement {
    let steps = ui.steps;
    card(
        "四条工序队列",
        Some("clean → litho → etch → metrology".to_owned()),
        view().flex_row().flex_wrap().gap_2().child(each(
            steps,
            |step| step.step_no,
            |step, _| {
                view()
                    .flex_1()
                    .min_w(px(150.0))
                    .p_2()
                    .gap_1()
                    .rounded(px(10.0))
                    .bg(move || theme().muted)
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_1()
                            .child(
                                text(move || {
                                    format!("{}. {}", step.get().step_no, step.get().name)
                                })
                                .text_size(px(13.0))
                                .font_weight(FontWeight::SEMIBOLD),
                            )
                            .child(view().flex_1())
                            .child(dot(BLUE)),
                    )
                    .child(
                        text(move || format!("排队 {}", step.get().queue_len))
                            .text_size(px(20.0))
                            .font_weight(FontWeight::BOLD),
                    )
                    .child(
                        text(move || {
                            format!(
                                "机台 {}/{} 忙 · {}",
                                step.get().busy_tools,
                                step.get().total_tools,
                                step.get()
                                    .current_lots
                                    .iter()
                                    .take(2)
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            )
                        })
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                    )
            },
        )),
    )
}

fn tools_panel(ui: Ui) -> impl IntoElement {
    let tools = ui.tools;
    card(
        "机台灯号",
        Some("绿=运行 黄=维护 红=停机".to_owned()),
        view().flex_row().flex_wrap().gap_2().child(each(
            tools,
            |tool| tool.id.clone(),
            |tool, _| {
                view()
                    .flex_row()
                    .items_center()
                    .gap_1p5()
                    .px_2()
                    .py_1()
                    .rounded(px(8.0))
                    .bg(move || theme().muted)
                    .child(dot(light_color(&tool.get().light)))
                    .child(
                        text(move || tool.get().id.clone())
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD),
                    )
                    .child(
                        text(move || {
                            let tool = tool.get();
                            match tool.current_lot {
                                Some(lot) => format!("{} · {}", tool.state, lot),
                                None => tool.state,
                            }
                        })
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                    )
            },
        )),
    )
}

fn lots_panel(ui: Ui) -> impl IntoElement {
    let lots = ui.lots;
    let detail = ui.detail;
    let selected = ui.selected_lot;
    card(
        "批次与逐片追溯",
        Some("点击批次 → 晶圆 → 工序时间线".to_owned()),
        view().gap_1().child(each(
            lots,
            |lot| lot.id.clone(),
            move |lot, _| {
                let id = lot.get().id.clone();
                let id_for_press = id.clone();
                view()
                    .gap_1()
                    .p_2()
                    .rounded(px(8.0))
                    .bg(move || {
                        if selected.get().as_deref() == Some(lot.get().id.as_str()) {
                            theme().accent
                        } else {
                            theme().muted
                        }
                    })
                    .on_press(move || load_lot(ui, id_for_press.clone()))
                    .child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(
                                text(move || lot.get().id.clone())
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD),
                            )
                            .child(dot(state_color(&lot.get().state)))
                            .child(
                                text(move || {
                                    let lot = lot.get();
                                    let step = if lot.current_step > 0 {
                                        format!("S{}", lot.current_step)
                                    } else {
                                        "—".to_owned()
                                    };
                                    format!(
                                        "{} · {} · {}/{} 片 · 优先级 {}",
                                        lot.state,
                                        step,
                                        lot.done_wafers,
                                        lot.wafer_count,
                                        lot.priority
                                    )
                                })
                                .text_size(px(11.0))
                                .text_color(move || theme().muted_foreground),
                            )
                            .child(view().flex_1())
                            .child(
                                text(move || {
                                    lot.get()
                                        .hold_reason
                                        .clone()
                                        .map(|reason| format!("HOLD: {reason}"))
                                        .unwrap_or_default()
                                })
                                .text_size(px(11.0))
                                .text_color(rgb(AMBER)),
                            ),
                    )
                    // The selected lot expands into its wafers, and the
                    // selected wafer into its per-step genealogy.
                    .child(dynamic(move || {
                        if selected.get().as_deref() != Some(lot.get().id.as_str()) {
                            return None;
                        }
                        let Some(detail) = detail.get() else {
                            return Some(
                                text("正在读取追溯…")
                                    .text_size(px(11.0))
                                    .text_color(move || theme().muted_foreground)
                                    .into_element(),
                            );
                        };
                        Some(wafer_grid(ui, detail).into_element())
                    }))
            },
        )),
    )
}

fn wafer_grid(ui: Ui, detail: LotDetail) -> impl IntoElement {
    let selected_wafer = ui.selected_wafer;
    let wafers = detail.wafers.clone();
    view()
        .gap_1()
        .pt_1()
        .child(view().flex_row().flex_wrap().gap_1().child(each(
            move || wafers.clone(),
            |wafer| wafer.id.clone(),
            move |wafer, _| {
                let wafer_id = wafer.get().id.clone();
                view()
                    .px_1p5()
                    .py_1()
                    .rounded(px(6.0))
                    .bg(move || {
                        let wafer = wafer.get();
                        if selected_wafer.get().as_deref() == Some(wafer.id.as_str()) {
                            theme().primary
                        } else if wafer.state == "done" {
                            AdaptiveColor::from(rgb(0x134e4a))
                        } else if wafer.state == "scrapped" {
                            AdaptiveColor::from(rgb(0x7f1d1d))
                        } else {
                            theme().card
                        }
                    })
                    .border_1()
                    .border_color(move || {
                        if wafer.get().state == "done" {
                            AdaptiveColor::from(rgb(GREEN))
                        } else {
                            theme().border
                        }
                    })
                    .on_press(move || ui.selected_wafer.set(Some(wafer_id.clone())))
                    .child(
                        text(move || format!("W{:02}", wafer.get().slot))
                            .text_size(px(10.0))
                            .text_color(move || theme().foreground),
                    )
            },
        )))
        .child(dynamic(move || {
            let Some(wafer_id) = selected_wafer.get() else {
                return None;
            };
            let entry = detail
                .genealogy
                .iter()
                .find(|entry| entry.wafer_id == wafer_id)?;
            let mut rows: Vec<AnyElement> = Vec::new();
            rows.push(
                text(format!(
                    "{}  {} · 返工 {} 次{}",
                    entry.wafer_id,
                    entry.state,
                    entry.rework_count,
                    entry
                        .defect_info
                        .as_ref()
                        .map(|info| format!(" · {info}"))
                        .unwrap_or_default()
                ))
                .text_size(px(11.0))
                .text_color(move || theme().muted_foreground)
                .into_element(),
            );
            for step in &entry.steps {
                rows.push(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(
                            text(format!("S{}", step.step_no))
                                .text_size(px(11.0))
                                .font_weight(FontWeight::SEMIBOLD),
                        )
                        .child(
                            text(step.recipe_id.clone())
                                .text_size(px(11.0))
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(text(step.equipment_id.clone()).text_size(px(11.0)))
                        .child(
                            text(if step.track_out_ms > 0 {
                                duration_text(step.duration_ms)
                            } else {
                                "进行中".to_owned()
                            })
                            .text_size(px(11.0))
                            .text_color(move || theme().muted_foreground),
                        )
                        .child(
                            text(
                                step.measured
                                    .map(|value| format!("{value:.2} nm"))
                                    .unwrap_or_default(),
                            )
                            .text_size(px(11.0))
                            .text_color(rgb(BLUE)),
                        )
                        .into_element(),
                );
            }
            Some(view().gap_1().pt_1().child(rows).into_element())
        }))
}

fn spc_panel(ui: Ui) -> impl IntoElement {
    let spc = ui.spc;
    let source = ui.spc_source;
    card(
        "SPC · 量测控制图",
        Some("读 GreptimeDB 降采样".to_owned()),
        view()
            .gap_2()
            .child(dynamic(move || {
                let buckets = spc.get();
                if buckets.is_empty() {
                    return text("等待量测样本（工序 4 完成后出现）")
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground)
                        .into_element();
                }
                spc_chart(buckets).into_element()
            }))
            .child(
                text(move || {
                    format!(
                        "来源 {} · {} 个时间桶 · 规格 97–103 nm",
                        source.get(),
                        spc.get().len()
                    )
                })
                .text_size(px(11.0))
                .text_color(move || theme().muted_foreground),
            ),
    )
}

fn spc_chart(buckets: Vec<SpcBucket>) -> impl IntoElement {
    let ucl = buckets.last().map(|bucket| bucket.ucl).unwrap_or(103.0);
    let lcl = buckets.last().map(|bucket| bucket.lcl).unwrap_or(97.0);
    let high = buckets
        .iter()
        .map(|bucket| bucket.max_value.max(bucket.ucl))
        .fold(ucl, f64::max)
        + 0.4;
    let low = buckets
        .iter()
        .map(|bucket| bucket.min_value.min(bucket.lcl))
        .fold(lcl, f64::min)
        - 0.4;
    let span = (high - low).max(0.1);
    let height = move |value: f64| ((value - low) / span * 92.0).clamp(2.0, 92.0) as f32;

    view()
        .gap_1()
        .child(
            view()
                .h(px(100.0))
                .flex_row()
                .items_end()
                .gap_1()
                .p_1()
                .rounded(px(8.0))
                .bg(move || theme().muted)
                .child(each(
                    {
                        let items = buckets.clone();
                        move || items.clone()
                    },
                    |bucket| bucket.ts.clone(),
                    move |bucket, _| {
                        let value = bucket.get().avg_value;
                        let out = value > ucl || value < lcl;
                        view()
                            .flex_1()
                            .min_w(px(3.0))
                            .h(px(height(value)))
                            .rounded(px(2.0))
                            .bg(move || if out { rgb(RED) } else { rgb(GREEN) })
                    },
                )),
        )
        .child(
            view()
                .flex_row()
                .justify_between()
                .child(
                    text(format!("UCL {ucl:.1}"))
                        .text_size(px(10.0))
                        .text_color(rgb(RED)),
                )
                .child(
                    text(move || {
                        format!(
                            "最近 {:.2} nm",
                            buckets.last().map(|bucket| bucket.avg_value).unwrap_or(0.0)
                        )
                    })
                    .text_size(px(10.0)),
                )
                .child(
                    text(format!("LCL {lcl:.1}"))
                        .text_size(px(10.0))
                        .text_color(rgb(AMBER)),
                ),
        )
}

fn recovery_panel(ui: Ui) -> impl IntoElement {
    let chaos = ui.chaos;
    let pending = ui.outbox_pending;
    card(
        "一致性 · outbox 积压与追平",
        Some("PG → 投递者 → GreptimeDB".to_owned()),
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
                                    .text_size(px(26.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(move || {
                                        if pending.get() > 0 {
                                            rgb(AMBER)
                                        } else {
                                            rgb(GREEN)
                                        }
                                    }),
                            )
                            .child(
                                text("待投递事件")
                                    .text_size(px(10.0))
                                    .text_color(move || theme().muted_foreground),
                            ),
                    )
                    .child(
                        view()
                            .gap_0p5()
                            .child(
                                text(move || format!("{}", ui.outbox_published.get()))
                                    .text_size(px(18.0))
                                    .font_weight(FontWeight::SEMIBOLD),
                            )
                            .child(
                                text("已投递")
                                    .text_size(px(10.0))
                                    .text_color(move || theme().muted_foreground),
                            ),
                    )
                    .child(view().flex_1())
                    .child(
                        text(move || {
                            let (peak, drained) = backlog_summary(&chaos.get());
                            match drained {
                                Some(ms) => {
                                    format!("峰值积压 {peak} · 追平 {:.1}s", ms as f64 / 1000.0)
                                }
                                None => format!("峰值积压 {peak}"),
                            }
                        })
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                    ),
            )
            .child(dynamic(move || backlog_chart(chaos.get()).into_element())),
    )
}

fn backlog_summary(samples: &[ChaosSample]) -> (i64, Option<i64>) {
    let peak = samples
        .iter()
        .map(|sample| sample.pending)
        .max()
        .unwrap_or(0);
    let mut started: Option<(i64, i64)> = None;
    let mut drained = None;
    for sample in samples {
        if sample.pending > 0 {
            if started.is_none() {
                started = Some((sample.at_ms, sample.pending));
            }
        } else if let Some((at, _)) = started.take() {
            drained = Some(sample.at_ms - at);
        }
    }
    (peak, drained)
}

fn backlog_chart(samples: Vec<ChaosSample>) -> impl IntoElement {
    let peak = samples
        .iter()
        .map(|sample| sample.pending)
        .max()
        .unwrap_or(1)
        .max(1);
    view()
        .h(px(48.0))
        .flex_row()
        .items_end()
        .gap_0p5()
        .p_1()
        .rounded(px(8.0))
        .bg(move || theme().muted)
        .child(each(
            {
                let items = samples.clone();
                move || items.clone()
            },
            |sample| sample.at_ms,
            move |sample, _| {
                let height = (sample.get().pending as f32 / peak as f32 * 36.0).max(1.0);
                view().flex_1().min_w(px(2.0)).h(px(height)).bg(move || {
                    if sample.get().pending > 0 {
                        rgb(AMBER)
                    } else {
                        rgb(GREEN)
                    }
                })
            },
        ))
}

fn ops_panel(ui: Ui) -> impl IntoElement {
    card(
        "备份与恢复演练",
        Some("PG PITR + Greptime 快照".to_owned()),
        view()
            .gap_1()
            .child(
                text(move || match ui.backup.get() {
                    Some(backup) => format!(
                        "备份 {} · base {} · WAL {} · Greptime {}",
                        backup.taken_at.unwrap_or_default(),
                        backup.pg_full_backup.unwrap_or_default(),
                        backup.wal_continuity.unwrap_or_default(),
                        backup.greptime_snapshot.unwrap_or_default()
                    ),
                    None => "尚无备份：运行 ./scripts/backup.sh".to_owned(),
                })
                .text_size(px(11.0))
                .text_color(move || theme().muted_foreground),
            )
            .child(
                text(move || match ui.restore.get() {
                    Some(restore) => format!(
                        "恢复演练 {} · lots {} · events {} · Greptime {} 行 · {:.1}s · {}",
                        restore.drilled_at.unwrap_or_default(),
                        restore.lots.unwrap_or_default(),
                        restore.events.unwrap_or_default(),
                        restore.greptime_rows.unwrap_or_default(),
                        restore.elapsed_ms.unwrap_or_default() as f64 / 1000.0,
                        if restore.ok { "通过" } else { "失败" }
                    ),
                    None => "尚无恢复演练：运行 ./scripts/restore.sh".to_owned(),
                })
                .text_size(px(11.0))
                .text_color(move || theme().muted_foreground),
            ),
    )
}

fn secs_panel(ui: Ui) -> impl IntoElement {
    let secs = ui.secs;
    card(
        "SECS/GEM 假机台",
        Some("HSMS-SS · E4/E5/E30/E37".to_owned()),
        dynamic(move || {
            let Some(status) = secs.get() else {
                return text("SECS/GEM 已关闭")
                    .text_size(px(12.0))
                    .text_color(move || theme().muted_foreground)
                    .into_element();
            };
            view()
                .gap_1()
                .child(
                    view()
                        .flex_row()
                        .items_center()
                        .gap_2()
                        .child(dot(if status.host_connected { GREEN } else { RED }))
                        .child(
                            text(if status.host_connected {
                                "链路已建立"
                            } else {
                                "链路断开"
                            })
                            .text_size(px(12.0)),
                        )
                        .child(view().flex_1())
                        .child(
                            button()
                                .px(px(8.0))
                                .py(px(4.0))
                                .rounded(px(6.0))
                                .bg(move || theme().muted)
                                .active(|style| style.opacity(0.7))
                                .on_press(move || post(ui, "/api/secs/drop", ""))
                                .child(text("断链 1.5s").text_size(px(11.0))),
                        ),
                )
                .child(
                    text(format!(
                        "S1F1 {} · S6F11 {} · S5F1 {} · S7F3 {} · S7F6 校验 {}",
                        status.s1f1_ok,
                        status.s6f11_sent,
                        status.s5f1_sent,
                        status.s7f3_uploaded,
                        status.s7f6_verified
                    ))
                    .text_size(px(11.0))
                    .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(format!(
                        "补传缓冲 {} 条 · 已补传 {} · 断链 {} 次{}",
                        status.spool_depth,
                        status.spooled_replayed,
                        status.link_drops,
                        status
                            .last_error
                            .as_ref()
                            .map(|error| format!(" · {error}"))
                            .unwrap_or_default()
                    ))
                    .text_size(px(11.0))
                    .text_color(move || theme().muted_foreground),
                )
                .child(
                    view()
                        .gap_0p5()
                        .children(status.traffic.iter().rev().take(3).map(|entry| {
                            text(format!(
                                "{} {} — {}",
                                entry.from, entry.message, entry.detail
                            ))
                            .text_size(px(10.0))
                            .text_color(move || theme().muted_foreground)
                        })),
                )
                .into_element()
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backlog_summary_reports_the_peak_and_the_drain() {
        let samples = vec![
            ChaosSample {
                at_ms: 0,
                pending: 0,
                ..Default::default()
            },
            ChaosSample {
                at_ms: 1000,
                pending: 40,
                ..Default::default()
            },
            ChaosSample {
                at_ms: 3000,
                pending: 12,
                ..Default::default()
            },
            ChaosSample {
                at_ms: 5000,
                pending: 0,
                ..Default::default()
            },
        ];
        let (peak, drained) = backlog_summary(&samples);
        assert_eq!(peak, 40);
        assert_eq!(drained, Some(4000));
    }
}
