//! The HTTP client and the poll loop.
//!
//! The snapshot seeds every signal; `/api/events?since=` folds increments into
//! the lots and tools; the projections (`analytics`, `monitor`, the lot detail)
//! are pulled less often and after every action. A dropped connection keeps
//! the last scene on screen and reconnects with the increment it missed — the
//! phase-1 rule, unchanged.

use std::path::PathBuf;
use std::time::Duration;

use nsh_core::file_dialog::SaveDialog;
use nsh_core::prelude::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::data::{
    Analysis, EventTrace, EventsResponse, LotDetail, MonitorResponse, Snapshot, TimelineResponse,
    WhatIfComparison,
};
use crate::scene_glue;
use crate::state::Ui;

async fn get_json<T: DeserializeOwned>(path: &str) -> Result<T, String> {
    let url = format!("{}{}", Ui::api_base(), path);
    let response = fetch(url).await?;
    if !response.ok() {
        return Err(format!("HTTP {} {}", response.status(), path));
    }
    serde_json::from_str(&response.text()).map_err(|error| error.to_string())
}

async fn post_json<T: DeserializeOwned>(path: &str, body: &Value) -> Result<T, String> {
    let url = format!("{}{}", Ui::api_base(), path);
    let request =
        HttpRequest::post(url, body.to_string()).header("Content-Type", "application/json");
    let response = fetch(request).await?;
    if !response.ok() {
        return Err(format!("HTTP {} {}", response.status(), path));
    }
    serde_json::from_str(&response.text()).map_err(|error| error.to_string())
}

/// The initial load and the poll loop.
pub fn bootstrap(ui: Ui) {
    spawn(async move {
        let _ = load_snapshot(ui).await;
        let _ = load_analytics(ui).await;
        let _ = load_monitor(ui).await;
        if std::env::var("OXIDE_DEMO").is_ok_and(|value| value == "1") {
            timer(Duration::from_millis(800)).await;
            crate::demo::start(ui);
        }
        let mut ticks: u64 = 0;
        loop {
            timer(Duration::from_millis(700)).await;
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
            ticks += 1;
            if ticks.is_multiple_of(6) {
                let _ = load_snapshot(ui).await;
            }
            if ticks.is_multiple_of(9) {
                let _ = load_analytics(ui).await;
                let _ = load_monitor(ui).await;
            }
        }
    });
}

/// `OXIDE_VIEW=factory|tool:<id>|lot:<id>|event:<uuid>` opens the workbench at
/// a level, for screenshots and for CI without a pointer. Applied once, after
/// the first snapshot that actually carries the floor.
fn apply_startup_view(ui: Ui) {
    static APPLIED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if ui.tools.get_untracked().is_empty() {
        return;
    }
    if APPLIED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    if std::env::var("OXIDE_THEME").is_ok_and(|value| value == "light") {
        ui.dark.set(false);
    }
    if let Ok(explode) = std::env::var("OXIDE_EXPLODE")
        && let Ok(value) = explode.parse::<f32>()
    {
        ui.explosion.set(value.clamp(0.0, 100.0));
    }
    if let Ok(tab) = std::env::var("OXIDE_TAB") {
        ui.tab.set(match tab.as_str() {
            "quality" => crate::state::Tab::Quality,
            "productivity" => crate::state::Tab::Productivity,
            "supply" => crate::state::Tab::Supply,
            "cross" => crate::state::Tab::Cross,
            _ => crate::state::Tab::Mes,
        });
    }
    if std::env::var("OXIDE_WHATIF").is_ok_and(|value| value == "1") {
        run_what_if(ui);
    }
    let Ok(view) = std::env::var("OXIDE_VIEW") else {
        return;
    };
    let (kind, value) = view.split_once(':').unwrap_or((view.as_str(), ""));
    match kind.trim() {
        "factory" => crate::viewport::fit(ui),
        "tool" if !value.is_empty() => scene_glue::drill(
            ui,
            oxide_scene::Entity::Tool {
                equipment_id: value.trim().to_owned(),
            },
        ),
        "lot" if !value.is_empty() => scene_glue::drill(
            ui,
            oxide_scene::Entity::Foup {
                lot_id: value.trim().to_owned(),
            },
        ),
        "event" if !value.is_empty() => load_trace(ui, value.trim().to_owned()),
        _ => {}
    }
}

pub async fn load_snapshot(ui: Ui) -> Result<(), String> {
    let snapshot: Snapshot = get_json("/api/snapshot").await?;
    let previous = ui.lots.get_untracked();
    ui.note_steps(&previous, &snapshot.sim.lots, snapshot.sim.now_ms);
    ui.steps.set(snapshot.sim.steps);
    ui.tools.set(snapshot.sim.equipment);
    ui.lots.set(snapshot.sim.lots);
    ui.dispatch.set(snapshot.sim.dispatch);
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
    apply_startup_view(ui);
    Ok(())
}

async fn pull_events(ui: Ui) -> Result<bool, String> {
    let since = ui.last_seq.get_untracked();
    let page: EventsResponse = get_json(&format!("/api/events?since={since}&limit=500")).await?;
    ui.last_seq.set(page.last_seq);
    if page.events.is_empty() {
        return Ok(true);
    }
    let previous = ui.lots.get_untracked();
    let mut lots = previous.clone();
    let mut tools = ui.tools.get_untracked();
    for persisted in &page.events {
        if !crate::data::apply_event(&mut lots, &mut tools, &persisted.event) {
            return Ok(false);
        }
    }
    let now = page.events.last().map_or_else(
        || ui.now_ms.get_untracked(),
        |event| event.event.occurred_at_ms,
    );
    ui.note_steps(&previous, &lots, now);
    ui.lots.set(lots);
    ui.tools.set(tools);
    Ok(true)
}

pub fn load_lot(ui: Ui, lot_id: String) {
    ui.selected_lot.set(Some(lot_id.clone()));
    ui.selected_wafer.set(None);
    spawn(async move {
        match get_json::<LotDetail>(&format!("/api/lots/{lot_id}")).await {
            Ok(detail) => ui.detail.set(Some(detail)),
            Err(error) => ui.note.set(Some(error)),
        }
    });
}

pub async fn load_analytics(ui: Ui) -> Result<(), String> {
    let analysis: Analysis = get_json("/api/analytics").await?;
    ui.analysis.set(Some(analysis));
    Ok(())
}

pub async fn load_monitor(ui: Ui) -> Result<(), String> {
    let monitor: MonitorResponse = get_json("/api/monitor").await?;
    ui.monitor.set(Some(monitor));
    Ok(())
}

/// Fetch one event's whole trail and put the scene at level 3.
pub fn load_trace(ui: Ui, event_id: String) {
    ui.trace_event.set(Some(event_id.clone()));
    spawn(async move {
        match get_json::<EventTrace>(&format!("/api/events/{event_id}")).await {
            Ok(trace) => {
                let focus = trace
                    .event
                    .event
                    .payload
                    .get("equipment_id")
                    .and_then(Value::as_str)
                    .map(|equipment_id| oxide_scene::Entity::Tool {
                        equipment_id: equipment_id.to_owned(),
                    })
                    .or_else(|| {
                        (!trace.event.event.lot_id.is_empty()).then(|| oxide_scene::Entity::Foup {
                            lot_id: trace.event.event.lot_id.clone(),
                        })
                    });
                ui.trace.set(Some(trace));
                ui.level.set(oxide_scene::Level::Event);
                ui.tab.set(crate::state::Tab::Quality);
                if let Some(entity) = focus {
                    ui.focus.set(Some(entity.clone()));
                    ui.selected.set(Some(entity.clone()));
                    scene_glue::focus_camera(ui, Some(&entity));
                }
                load_analytics(ui).await.ok();
            }
            Err(error) => ui.note.set(Some(format!("事件 {event_id}: {error}"))),
        }
    });
}

/// Run the offline what-if on a copy of the server's current state.
pub fn run_what_if(ui: Ui) {
    let fifo = ui.whatif_fifo.get_untracked();
    let add_tool = ui.whatif_add_tool.get_untracked();
    let kind = ui
        .focus
        .get_untracked()
        .and_then(|entity| match entity {
            oxide_scene::Entity::Tool { equipment_id }
            | oxide_scene::Entity::ToolPart { equipment_id, .. } => ui
                .tools
                .get_untracked()
                .into_iter()
                .find(|tool| tool.id == equipment_id)
                .map(|tool| tool.kind),
            _ => None,
        })
        .unwrap_or_else(|| "etch".to_owned());
    ui.whatif_busy.set(true);
    spawn(async move {
        let body = json!({
            "rule": if fifo { "fifo" } else { "priority" },
            "add_tool_kind": add_tool.then_some(kind),
            "horizon_ms": 7_200_000,
        });
        match post_json::<WhatIfComparison>("/api/what-if", &body).await {
            Ok(comparison) => {
                ui.whatif.set(Some(comparison));
                ui.last_action
                    .set(Some("what-if 完成：当前运行实例未被改写".to_owned()));
            }
            Err(error) => ui.note.set(Some(format!("what-if: {error}"))),
        }
        ui.whatif_busy.set(false);
    });
}

/// A write operation, always labelled 演示触发 in the UI and always through the
/// API's event log.
pub fn action(ui: Ui, path: &str, body: Value, label: &str) {
    if !ui.connected.get_untracked() {
        ui.note
            .set(Some("后端未就绪：写操作已禁用，只读引导仍可用".to_owned()));
        return;
    }
    let path = path.to_owned();
    let label = label.to_owned();
    spawn(async move {
        match post_json::<Value>(&path, &body).await {
            Ok(_) => {
                ui.last_action
                    .set(Some(format!("{label} · 已写入 lot_event")));
                let _ = pull_events(ui).await;
                let _ = load_snapshot(ui).await;
                let _ = load_analytics(ui).await;
                if let Some(lot_id) = ui.selected_lot.get_untracked() {
                    load_lot(ui, lot_id);
                }
            }
            Err(error) => ui.note.set(Some(format!("{label}: {error}"))),
        }
    });
}

pub fn sim(ui: Ui, path: &str, label: &str) {
    action(ui, path, json!({}), label);
}

pub fn hold(ui: Ui) {
    if let Some(lot) = ui.selected_lot.get_untracked() {
        action(
            ui,
            &format!("/api/lots/{lot}/hold"),
            json!({"reason": "workbench hold"}),
            "Hold",
        );
    }
}

pub fn release(ui: Ui) {
    if let Some(lot) = ui.selected_lot.get_untracked() {
        action(
            ui,
            &format!("/api/lots/{lot}/release"),
            json!({}),
            "Release",
        );
    }
}

pub fn rework(ui: Ui) {
    if let Some(lot) = ui.latest_lot() {
        let to_step = (lot.current_step - 1).max(1);
        action(
            ui,
            &format!("/api/lots/{}/rework", lot.id),
            json!({"to_step": to_step, "reason": "workbench rework"}),
            "Rework",
        );
    }
}

pub fn split(ui: Ui) {
    if let Some(lot) = ui.selected_lot.get_untracked() {
        action(
            ui,
            &format!("/api/lots/{lot}/split"),
            json!({"children": [format!("{lot}-A"), format!("{lot}-B")]}),
            "Split",
        );
    }
}

pub fn merge(ui: Ui) {
    let Some(lot) = ui.latest_lot() else { return };
    let source = ui
        .lots
        .get_untracked()
        .into_iter()
        .find(|other| {
            other.id != lot.id
                && other.current_step == lot.current_step
                && !matches!(other.state.as_str(), "completed" | "split" | "merged")
        })
        .map(|other| other.id);
    let Some(source) = source else {
        ui.note.set(Some("没有同工序可合并的批次".to_owned()));
        return;
    };
    action(
        ui,
        &format!("/api/lots/{}/merge", lot.id),
        json!({"sources": [source]}),
        "Merge",
    );
}

pub fn scrap(ui: Ui) {
    if let Some(detail) = ui.detail.get_untracked()
        && let Some(wafer) = detail.wafers.iter().find(|wafer| wafer.state != "scrapped")
    {
        action(
            ui,
            &format!("/api/lots/{}/scrap", detail.lot.id),
            json!({"wafer_id": wafer.id, "reason": "workbench scrap"}),
            "Scrap",
        );
    }
}

/// Start the local compose project (the packaged app needs a backend URL).
pub fn start_backend(ui: Ui) {
    ui.backend.set("启动中…".to_owned());
    spawn(async move {
        let root = std::env::var("OXIDE_REPO_ROOT").unwrap_or_else(|_| ".".to_owned());
        let result = blocking(move || {
            std::process::Command::new("docker")
                .args(["compose", "up", "-d", "--wait"])
                .current_dir(&root)
                .output()
        })
        .await;
        match result {
            Ok(Ok(output)) if output.status.success() => {
                ui.backend.set("后端已就绪".to_owned());
                let _ = load_snapshot(ui).await;
                let _ = load_analytics(ui).await;
                let _ = load_monitor(ui).await;
            }
            Ok(Ok(output)) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                ui.backend.set(format!(
                    "compose 失败：{}",
                    stderr.lines().last().unwrap_or_default()
                ));
            }
            Ok(Err(error)) => ui.backend.set(format!("无法运行 docker：{error}")),
            Err(error) => ui.backend.set(format!("启动任务失败：{error}")),
        }
    });
}

/// Save a JSON document through the system dialog.
fn save(ui: Ui, suggested: &str, value: Value) {
    let suggested = suggested.to_owned();
    spawn(async move {
        let dialog = SaveDialog::new()
            .file_name(suggested)
            .title("导出 oxide-fab 演示数据");
        match pick_save_path(dialog).await {
            Ok(Some(path)) => match write_json(&path, &value) {
                Ok(()) => ui
                    .last_action
                    .set(Some(format!("已导出 {}", path.display()))),
                Err(error) => ui.note.set(Some(format!("写入失败：{error}"))),
            },
            Ok(None) => {}
            Err(error) => ui.note.set(Some(format!("保存对话框不可用：{error}"))),
        }
    });
}

fn write_json(path: &PathBuf, value: &Value) -> Result<(), String> {
    let text = serde_json::to_string_pretty(value).map_err(|error| error.to_string())?;
    std::fs::write(path, text).map_err(|error| error.to_string())
}

pub fn export_scene(ui: Ui) {
    let graph = scene_glue::graph(ui);
    let value = json!({
        "exported_at_ms": ui.now_ms.get_untracked(),
        "level": ui.level.get_untracked().label(),
        "focus": ui.focus.get_untracked().map(|entity| entity.key()),
        "selected": ui.selected.get_untracked().map(|entity| entity.key()),
        "camera": camera_json(&ui.camera.get_untracked()),
        "nodes": graph.as_ref().map(|graph| graph.nodes.iter().map(|node| json!({
            "entity": node.entity.key(),
            "mesh": node.mesh,
            "aabb": {"min": node.aabb.min, "max": node.aabb.max},
            "pickable": node.pickable,
        })).collect::<Vec<_>>()),
        "lines": graph.as_ref().map(|graph| graph.lines.len()),
        "source": "PostgreSQL lot_event 投影 + manifest 场景图；相机为客户端状态",
    });
    save(ui, "oxide-scene.json", value);
}

fn camera_json(camera: &oxide_scene::Camera) -> Value {
    json!({
        "target": [camera.target.x, camera.target.y, camera.target.z],
        "yaw": camera.yaw,
        "pitch": camera.pitch,
        "distance": camera.distance,
        "projection": camera.projection.label(),
    })
}

pub fn export_lot(ui: Ui) {
    let Some(detail) = ui.detail.get_untracked() else {
        ui.note.set(Some("先选中一个批次".to_owned()));
        return;
    };
    let value = serde_json::to_value(LotTraceExport::from(&detail)).unwrap_or(Value::Null);
    save(ui, &format!("{}.json", detail.lot.id), value);
}

#[derive(Serialize)]
struct LotTraceExport {
    lot_id: String,
    state: String,
    current_step: i32,
    wafers: Vec<serde_json::Value>,
    genealogy: Vec<serde_json::Value>,
    events: Vec<serde_json::Value>,
    source: &'static str,
}

impl From<&LotDetail> for LotTraceExport {
    fn from(detail: &LotDetail) -> Self {
        Self {
            lot_id: detail.lot.id.clone(),
            state: detail.lot.state.clone(),
            current_step: detail.lot.current_step,
            wafers: detail
                .wafers
                .iter()
                .map(|wafer| {
                    json!({
                        "id": wafer.id,
                        "slot": wafer.slot,
                        "state": wafer.state,
                        "defect_info": wafer.defect_info,
                        "rework_count": wafer.rework_count,
                    })
                })
                .collect(),
            genealogy: detail
                .genealogy
                .iter()
                .map(|entry| serde_json::to_value(entry).unwrap_or(Value::Null))
                .collect(),
            events: detail
                .events
                .iter()
                .map(|event| serde_json::to_value(event).unwrap_or(Value::Null))
                .collect(),
            source: "lot_event（按 seq 完整）、wafer 投影、genealogy 推导",
        }
    }
}

/// The chaos report: outbox timeline, backup/restore drill and the analytics
/// that depend on them. Fetched fresh so the file matches what was shown.
pub fn export_chaos(ui: Ui) {
    spawn(async move {
        let timeline: Result<TimelineResponse, String> = get_json("/api/outbox/timeline").await;
        let analysis: Result<Analysis, String> = get_json("/api/analytics").await;
        let restore: Result<Value, String> = get_json("/api/restore").await;
        let backup: Result<Value, String> = get_json("/api/backup").await;
        let value = json!({
            "exported_at_ms": ui.now_ms.get_untracked(),
            "timeline": timeline.ok(),
            "analytics": analysis.ok(),
            "backup": backup.ok(),
            "restore": restore.ok(),
            "rpo_rto_note": "RPO/RTO 来自 scripts/backup.sh、restore.sh、chaos.sh 的产物；UI 只读取并展示。",
        });
        save(ui, "oxide-chaos-report.json", value);
    });
}
