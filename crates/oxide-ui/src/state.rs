//! Everything the workbench holds: one `Copy` struct of signal handles.
//!
//! A component takes `Ui` by value (every field is `Copy`), reads what it needs
//! and subscribes only to that. The render loop, the poller and the panels all
//! mutate the same signals, so the 3D view and the tables cannot disagree.

use std::collections::HashMap;

use nsh_core::prelude::*;
use oxide_core::analytics::Analysis;
use oxide_scene::{Camera, Entity, Level};

use crate::data::{
    BackupStatus, ChaosSample, EventTrace, LotDetail, LotView, MonitorResponse, RestoreStatus,
    SecsStatus, SpcBucket, StepView, ToolView, WhatIfComparison,
};
use crate::palette::{DARK, LIGHT, Palette};
use crate::scene_glue::StepSince;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Mes,
    Quality,
    Productivity,
    Supply,
    Cross,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Mes,
        Tab::Quality,
        Tab::Productivity,
        Tab::Supply,
        Tab::Cross,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Tab::Mes => "制造执行",
            Tab::Quality => "工艺质量",
            Tab::Productivity => "生产效率",
            Tab::Supply => "供应链",
            Tab::Cross => "横切 · 总线与 KPI",
        }
    }

    #[must_use]
    pub fn hint(self) -> &'static str {
        match self {
            Tab::Mes => "MES 300Works 语义：WIP、genealogy、载具",
            Tab::Quality => "Applied E3 语义：SPC、FD、RMS、Q-time、良率",
            Tab::Productivity => "APF 语义：RTD、派工、WIP、what-if",
            Tab::Supply => "计划与实际节拍、交付偏差预警",
            Tab::Cross => "Monitor、outbox、KPI、演练与导出",
        }
    }
}

/// One drag, tracked so a press that barely moved is a click.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Drag {
    pub active: bool,
    pub start: (f32, f32),
    pub last: (f32, f32),
    pub moved: f32,
}

/// The guided demo's position; stopping is a signal, so a step never jumps
/// back after the user takes over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DemoStage {
    #[default]
    Idle,
    Factory,
    Tool,
    Lot,
    Event,
    Quality,
    WhatIf,
    Bus,
    Kpi,
    Done,
}

impl DemoStage {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            DemoStage::Idle => "演示待启动",
            DemoStage::Factory => "1/8 全厂 · MES",
            DemoStage::Tool => "2/8 机台拆解 · L1",
            DemoStage::Lot => "3/8 批次逐片 · L2",
            DemoStage::Event => "4/8 事件追踪 · L3",
            DemoStage::Quality => "5/8 工艺质量",
            DemoStage::WhatIf => "6/8 离线 what-if",
            DemoStage::Bus => "7/8 消息总线 chaos",
            DemoStage::Kpi => "8/8 KPI 与 RPO/RTO",
            DemoStage::Done => "演示结束：可继续操作",
        }
    }
}

/// The workbench state.
#[derive(Clone, Copy)]
pub struct Ui {
    // The snapshot.
    pub steps: Signal<Vec<StepView>>,
    pub tools: Signal<Vec<ToolView>>,
    pub lots: Signal<Vec<LotView>>,
    pub spc: Signal<Vec<SpcBucket>>,
    pub spc_source: Signal<String>,
    pub chaos: Signal<Vec<ChaosSample>>,
    pub secs: Signal<Option<SecsStatus>>,
    pub backup: Signal<Option<BackupStatus>>,
    pub restore: Signal<Option<RestoreStatus>>,
    pub outbox_pending: Signal<i64>,
    pub outbox_published: Signal<i64>,
    pub events_total: Signal<i64>,
    pub published_events: Signal<i64>,
    pub now_ms: Signal<i64>,
    pub running: Signal<bool>,
    pub speed: Signal<f64>,
    pub last_seq: Signal<i64>,
    pub connected: Signal<bool>,
    pub greptime_ok: Signal<bool>,
    pub note: Signal<Option<String>>,
    pub step_since: Signal<StepSince>,
    pub dispatch: Signal<Vec<oxide_sim::DispatchDecision>>,

    // Selection and the 3D view.
    pub camera: Signal<Camera>,
    pub level: Signal<Level>,
    pub focus: Signal<Option<Entity>>,
    pub selected: Signal<Option<Entity>>,
    pub trail: Signal<Vec<Entity>>,
    pub explosion: Signal<f32>,
    pub reduce_motion: Signal<bool>,
    pub dark: Signal<bool>,
    pub view_size: Signal<(f32, f32)>,
    pub frame: Signal<u64>,
    pub drag: Signal<Drag>,
    pub adapter: Signal<String>,
    pub render_error: Signal<Option<String>>,
    pub sink: u64,

    // Drill-down details.
    pub selected_lot: Signal<Option<String>>,
    pub selected_wafer: Signal<Option<String>>,
    pub detail: Signal<Option<LotDetail>>,
    pub trace: Signal<Option<EventTrace>>,
    pub trace_event: Signal<Option<String>>,

    // Read-only projections.
    pub analysis: Signal<Option<Analysis>>,
    pub monitor: Signal<Option<MonitorResponse>>,
    pub whatif: Signal<Option<WhatIfComparison>>,
    pub whatif_fifo: Signal<bool>,
    pub whatif_add_tool: Signal<bool>,
    pub whatif_busy: Signal<bool>,

    // Chrome.
    pub tab: Signal<Tab>,
    pub search: Signal<String>,
    pub dense: Signal<bool>,
    pub demo: Signal<DemoStage>,
    pub demo_gen: Signal<u64>,
    pub backend: Signal<String>,
    pub last_action: Signal<Option<String>>,
}

impl std::fmt::Debug for Ui {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ui").finish_non_exhaustive()
    }
}

impl Ui {
    #[must_use]
    pub fn new(sink: u64) -> Self {
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
            greptime_ok: signal(false),
            note: signal(None),
            step_since: signal(HashMap::new()),
            dispatch: signal(Vec::new()),
            camera: signal(Camera::default()),
            level: signal(Level::Factory),
            focus: signal(None),
            selected: signal(None),
            trail: signal(Vec::new()),
            explosion: signal(0.0),
            reduce_motion: signal(false),
            dark: signal(true),
            view_size: signal((1100.0, 680.0)),
            frame: signal(0),
            drag: signal(Drag::default()),
            adapter: signal(String::new()),
            render_error: signal(None),
            sink,
            selected_lot: signal(None),
            selected_wafer: signal(None),
            detail: signal(None),
            trace: signal(None),
            trace_event: signal(None),
            analysis: signal(None),
            monitor: signal(None),
            whatif: signal(None),
            whatif_fifo: signal(false),
            whatif_add_tool: signal(true),
            whatif_busy: signal(false),
            tab: signal(Tab::default()),
            search: signal(String::new()),
            dense: signal(true),
            demo: signal(DemoStage::default()),
            demo_gen: signal(0),
            backend: signal(String::from("未检查")),
            last_action: signal(None),
        }
    }

    #[must_use]
    pub fn palette(&self) -> Palette {
        if self.dark.get() { DARK } else { LIGHT }
    }

    /// The API's base URL: the same name the scripts use.
    #[must_use]
    pub fn api_base() -> String {
        std::env::var("OXIDE_API_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_owned())
    }

    #[must_use]
    pub fn level_label(&self) -> String {
        format!(
            "{} · {}",
            self.level.get().label(),
            self.focus
                .get()
                .map_or_else(|| "全厂".to_owned(), |entity| entity_label(&entity))
        )
    }

    pub fn select(&self, entity: Option<Entity>) {
        self.selected.set(entity);
    }

    /// Record which step each lot is on, for the FOUP transit animation.
    pub fn note_steps(&self, previous: &[LotView], next: &[LotView], now_ms: i64) {
        let mut since = self.step_since.get_untracked();
        let mut changed = false;
        for lot in next {
            let old = previous.iter().find(|old| old.id == lot.id);
            if old.is_none_or(|old| old.current_step != lot.current_step) {
                since.insert(lot.id.clone(), (lot.current_step, now_ms));
                changed = true;
            }
        }
        if changed {
            self.step_since.set(since);
        }
    }

    #[must_use]
    pub fn latest_lot(&self) -> Option<LotView> {
        let id = self.selected_lot.get()?;
        self.lots.get().into_iter().find(|lot| lot.id == id)
    }
}

/// A short human label for an entity, used in breadcrumbs, panels and exports.
#[must_use]
pub fn entity_label(entity: &Entity) -> String {
    match entity {
        Entity::Floor => "厂区地面".to_owned(),
        Entity::Band { step_no } => format!("工序带 S{step_no}"),
        Entity::Rail => "AMHS 环线".to_owned(),
        Entity::Tool { equipment_id } => equipment_id.clone(),
        Entity::ToolPart { equipment_id, part } => format!("{equipment_id} · {part}"),
        Entity::Foup { lot_id } => format!("{lot_id} 载具"),
        Entity::Wafer { lot_id, wafer_id } => format!("{lot_id} · {wafer_id}"),
        Entity::Exit => "出片区".to_owned(),
    }
}

impl Default for Ui {
    fn default() -> Self {
        Self::new(0)
    }
}
