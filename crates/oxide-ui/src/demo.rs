//! The guided demo.
//!
//! It drives the same signals the user drives, in a fixed order, and stops the
//! moment the user takes over: every stop request bumps a generation counter
//! the running task checks before each step, so a stopped demo never jumps
//! back in.

use std::time::Duration;

use nsh_core::prelude::*;
use oxide_scene::Entity;

use crate::client;
use crate::scene_glue;
use crate::state::{DemoStage, Tab, Ui};
use crate::viewport;

#[must_use]
pub fn running(ui: Ui) -> bool {
    ui.demo.get() != DemoStage::Idle
}

pub fn stop(ui: Ui) {
    ui.demo_gen.update(|generation| *generation += 1);
    ui.demo.set(DemoStage::Idle);
}

pub fn toggle(ui: Ui) {
    if running(ui) {
        stop(ui);
    } else {
        start(ui);
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "the eight stages are one linear script; each stage is a focus change, a tab and a note"
)]
pub fn start(ui: Ui) {
    let generation = ui.demo_gen.get_untracked() + 1;
    ui.demo_gen.set(generation);
    let alive = move || ui.demo_gen.get_untracked() == generation;
    spawn(async move {
        ui.demo.set(DemoStage::Factory);
        ui.tab.set(Tab::Mes);
        ui.explosion.set(0.0);
        viewport::fit(ui);
        ui.note
            .set(Some("演示：全厂归属 MES；机台灯号与队列同源。".to_owned()));
        timer(Duration::from_millis(2400)).await;
        if !alive() {
            return;
        }

        ui.demo.set(DemoStage::Tool);
        let tool = ui
            .tools
            .get_untracked()
            .into_iter()
            .next()
            .map(|tool| Entity::Tool {
                equipment_id: tool.id,
            });
        if let Some(tool) = tool {
            scene_glue::drill(ui, tool);
            ui.explosion.set(100.0);
            ui.note.set(Some(
                "演示：部件按 manifest 的爆炸方向实时平移，未预烘焙任何图。".to_owned(),
            ));
        }
        timer(Duration::from_millis(2800)).await;
        if !alive() {
            return;
        }

        ui.demo.set(DemoStage::Lot);
        let lot = ui
            .lots
            .get_untracked()
            .into_iter()
            .find(|lot| !lot.wafers.is_empty())
            .or_else(|| ui.lots.get_untracked().into_iter().next());
        if let Some(lot) = lot {
            scene_glue::drill(ui, Entity::Foup { lot_id: lot.id });
        }
        ui.note.set(Some(
            "演示：25 槽网格按 wafer.state 与 defect_info 着色，逐片可下钻。".to_owned(),
        ));
        timer(Duration::from_millis(2800)).await;
        if !alive() {
            return;
        }

        ui.demo.set(DemoStage::Event);
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
            .or_else(|| {
                ui.detail.get_untracked().and_then(|detail| {
                    detail
                        .events
                        .last()
                        .map(|event| event.event.event_id.clone())
                })
            });
        if let Some(event_id) = event_id {
            client::load_trace(ui, event_id);
        } else {
            ui.note.set(Some(
                "演示：尚无告警或 Q-time 事件，L3 由搜索任意 event_id 进入。".to_owned(),
            ));
        }
        ui.tab.set(Tab::Quality);
        timer(Duration::from_millis(2800)).await;
        if !alive() {
            return;
        }

        ui.demo.set(DemoStage::Quality);
        ui.tab.set(Tab::Quality);
        ui.note.set(Some(
            "演示：SPC、告警、配方校验、Q-time 与良率都来自同一份事件日志。".to_owned(),
        ));
        timer(Duration::from_millis(2600)).await;
        if !alive() {
            return;
        }

        ui.demo.set(DemoStage::WhatIf);
        ui.tab.set(Tab::Productivity);
        client::run_what_if(ui);
        ui.note.set(Some(
            "演示：what-if 在隔离副本上重放当前 snapshot，运行实例不受影响。".to_owned(),
        ));
        timer(Duration::from_millis(3200)).await;
        if !alive() {
            return;
        }

        ui.demo.set(DemoStage::Bus);
        ui.tab.set(Tab::Cross);
        ui.note.set(Some(
            "演示：outbox 积压与追平来自 PG；chaos 由 ./scripts/chaos.sh 执行，面板实时反映。"
                .to_owned(),
        ));
        timer(Duration::from_millis(2600)).await;
        if !alive() {
            return;
        }

        ui.demo.set(DemoStage::Kpi);
        ui.tab.set(Tab::Cross);
        ui.note.set(Some(
            "演示：KPI 每个数字都标注口径；RPO/RTO 取演练产物。".to_owned(),
        ));
        timer(Duration::from_millis(2400)).await;
        if alive() {
            ui.demo.set(DemoStage::Done);
        }
    });
}
