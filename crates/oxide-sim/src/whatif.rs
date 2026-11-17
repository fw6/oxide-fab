//! Offline what-if: replay the *current* state forward twice, once unchanged
//! and once with a dispatch rule or an extra tool, and compare.
//!
//! The live simulation is never touched: the caller hands in a snapshot of
//! [`SimState`], both runs clone it, and everything they emit is discarded.
//! That is what lets the workbench show a comparison while the line keeps
//! running.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{DispatchRule, EquipmentRuntime, SimError, SimState, Simulation};
use oxide_core::events::EventKind;
use oxide_core::scenario::{EquipmentSpec, Scenario};

/// What the user asks the offline replay to change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhatIfOptions {
    /// The rule the variant runs with.
    pub rule: DispatchRule,
    /// Add one tool of this kind before the variant starts.
    #[serde(default)]
    pub add_tool_kind: Option<String>,
    /// How far the replay may run in simulated milliseconds.
    #[serde(default = "default_horizon")]
    pub horizon_ms: i64,
}

fn default_horizon() -> i64 {
    7_200_000
}

impl Default for WhatIfOptions {
    fn default() -> Self {
        Self {
            rule: DispatchRule::PriorityThenFifo,
            add_tool_kind: None,
            horizon_ms: default_horizon(),
        }
    }
}

/// One run's outcome, in the units the comparison shows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhatIfRun {
    pub label: String,
    pub rule: DispatchRule,
    pub added_tool: Option<String>,
    pub completed_lots: u64,
    pub mean_cycle_time_ms: Option<f64>,
    pub throughput_lots_per_hour: Option<f64>,
    pub wip_end: u64,
    pub simulated_ms: i64,
    pub timed_out: bool,
    /// The baseline started at t0 because the snapshot had no material in
    /// flight (everything was terminal).
    pub from_start: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhatIfDelta {
    pub completed_lots: i64,
    pub mean_cycle_time_ms: Option<f64>,
    pub throughput_lots_per_hour: Option<f64>,
    pub wip_end: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WhatIfComparison {
    pub baseline: WhatIfRun,
    pub variant: WhatIfRun,
    pub delta: WhatIfDelta,
    /// The formula behind each compared number.
    pub notes: Vec<String>,
}

impl WhatIfComparison {
    #[must_use]
    pub fn new(baseline: WhatIfRun, variant: WhatIfRun) -> Self {
        let delta = WhatIfDelta {
            completed_lots: variant.completed_lots as i64 - baseline.completed_lots as i64,
            mean_cycle_time_ms: optional_delta(
                variant.mean_cycle_time_ms,
                baseline.mean_cycle_time_ms,
            ),
            throughput_lots_per_hour: optional_delta(
                variant.throughput_lots_per_hour,
                baseline.throughput_lots_per_hour,
            ),
            wip_end: variant.wip_end as i64 - baseline.wip_end as i64,
        };
        Self {
            baseline,
            variant,
            delta,
            notes: vec![
                "两次重放都从同一份 snapshot 起步，互不影响，也不改写当前运行实例。".to_owned(),
                "周期时间 = 本次重放内 lot_completed 的 occurred_at − 该批次 created_at，取均值。"
                    .to_owned(),
                "产出 = 本次重放完成的批次数 ÷ 重放时长（小时）。".to_owned(),
                "WIP = 重放结束时未进入 completed/split/merged 的批次数。".to_owned(),
            ],
        }
    }
}

/// Run the baseline and the variant from the same snapshot.
pub fn compare(
    scenario: &Scenario,
    state: &SimState,
    options: &WhatIfOptions,
) -> Result<WhatIfComparison, SimError> {
    let from_start = state.lots.values().all(|lot| lot.lot.state.is_terminal());
    let baseline_scenario = scenario.clone();
    let baseline = if from_start {
        run_from_start(
            "baseline",
            baseline_scenario,
            DispatchRule::PriorityThenFifo,
            None,
            options.horizon_ms,
        )?
    } else {
        run(
            "baseline",
            baseline_scenario,
            state.clone(),
            DispatchRule::PriorityThenFifo,
            None,
            options.horizon_ms,
        )?
    };
    let (variant_scenario, added_tool) =
        variant_scenario(scenario, options.add_tool_kind.as_deref())?;
    let mut variant_state = state.clone();
    if let Some(id) = &added_tool {
        let spec = variant_scenario
            .equipment
            .iter()
            .find(|spec| &spec.id == id)
            .expect("the variant scenario carries the added tool");
        variant_state
            .equipment
            .insert(id.clone(), EquipmentRuntime::new(spec.to_equipment()));
    }
    let variant = if from_start {
        run_from_start(
            "variant",
            variant_scenario,
            options.rule,
            added_tool,
            options.horizon_ms,
        )?
    } else {
        run(
            "variant",
            variant_scenario,
            variant_state,
            options.rule,
            added_tool,
            options.horizon_ms,
        )?
    };
    let mut comparison = WhatIfComparison::new(baseline, variant);
    if from_start {
        comparison
            .notes
            .push("当前 snapshot 无在制批次：两次重放都从 t0 开始（from_start=true）。".to_owned());
    }
    Ok(comparison)
}

fn variant_scenario(
    scenario: &Scenario,
    add_kind: Option<&str>,
) -> Result<(Scenario, Option<String>), SimError> {
    let Some(kind) = add_kind else {
        return Ok((scenario.clone(), None));
    };
    let recipes: BTreeSet<String> = scenario
        .route
        .steps
        .iter()
        .filter(|step| step.equipment_kind == kind)
        .map(|step| step.recipe_id.clone())
        .collect();
    if recipes.is_empty() {
        return Err(SimError::UnknownKind(kind.to_owned()));
    }
    let mut variant = scenario.clone();
    let id = format!("{}-X1", kind.to_ascii_uppercase());
    if variant.equipment.iter().any(|tool| tool.id == id) {
        return Err(SimError::DuplicateTool(id));
    }
    variant.equipment.push(EquipmentSpec {
        id: id.clone(),
        kind: kind.to_owned(),
        chamber: "WHATIF".to_owned(),
        qualification: recipes.iter().cloned().collect(),
    });
    for step in &mut variant.route.steps {
        if step.equipment_kind == kind && !step.equipment_qualification.contains(&id) {
            step.equipment_qualification.push(id.clone());
        }
    }
    variant.validate()?;
    Ok((variant, Some(id)))
}

fn run(
    label: &str,
    scenario: Scenario,
    mut state: SimState,
    rule: DispatchRule,
    added_tool: Option<String>,
    horizon_ms: i64,
) -> Result<WhatIfRun, SimError> {
    state.running = true;
    state.dispatch.clear();
    let start_ms = state.now_ms;
    let total_lots = scenario.lot_count;
    let mut sim = Simulation::from_state(scenario, state)?.with_rule(rule);
    sim.rebuild_heap();
    let tick_ms = sim.scenario().tick_ms.max(1);
    let horizon = horizon_ms.max(tick_ms);
    let mut completed: Vec<(String, i64)> = Vec::new();
    let mut timed_out = false;
    loop {
        let elapsed = sim.now_ms() - start_ms;
        if elapsed >= horizon {
            timed_out = true;
            break;
        }
        let output = sim.tick(tick_ms);
        for event in &output.events {
            if event.kind == EventKind::LotCompleted {
                completed.push((event.lot_id.clone(), event.occurred_at_ms));
            }
        }
        let snapshot = sim.snapshot();
        let all_done = snapshot
            .lots
            .values()
            .all(|lot| lot.lot.state.is_terminal());
        if all_done && snapshot.released_lots >= total_lots {
            break;
        }
    }
    let end_ms = sim.now_ms();
    let snapshot = sim.snapshot();
    let simulated_ms = (end_ms - start_ms).max(0);
    let cycles: Vec<f64> = completed
        .iter()
        .filter_map(|(lot_id, at)| {
            snapshot
                .lots
                .get(lot_id)
                .map(|aggregate| (at - aggregate.lot.created_at_ms) as f64)
        })
        .filter(|cycle| *cycle > 0.0)
        .collect();
    let mean_cycle = (!cycles.is_empty()).then(|| cycles.iter().sum::<f64>() / cycles.len() as f64);
    let throughput = (simulated_ms > 0 && !completed.is_empty())
        .then(|| completed.len() as f64 / (simulated_ms as f64 / 3_600_000.0));
    let wip_end = snapshot
        .lots
        .values()
        .filter(|lot| !lot.lot.state.is_terminal())
        .count() as u64;
    Ok(WhatIfRun {
        label: label.to_owned(),
        rule,
        added_tool,
        completed_lots: completed.len() as u64,
        mean_cycle_time_ms: mean_cycle,
        throughput_lots_per_hour: throughput,
        wip_end,
        simulated_ms,
        timed_out,
        from_start: false,
    })
}

/// Replay the scenario from t0 (used when the snapshot holds no WIP).
fn run_from_start(
    label: &str,
    scenario: Scenario,
    rule: DispatchRule,
    added_tool: Option<String>,
    horizon_ms: i64,
) -> Result<WhatIfRun, SimError> {
    let total_lots = scenario.lot_count;
    let mut sim = Simulation::new(scenario)?.with_rule(rule);
    let start_ms = sim.now_ms();
    let mut completed: Vec<(String, i64)> = Vec::new();
    let mut timed_out = false;
    loop {
        if sim.now_ms() - start_ms >= horizon_ms {
            timed_out = true;
            break;
        }
        let output = sim.tick(sim.scenario().tick_ms.max(1));
        for event in &output.events {
            if event.kind == EventKind::LotCompleted {
                completed.push((event.lot_id.clone(), event.occurred_at_ms));
            }
        }
        let snapshot = sim.snapshot();
        if snapshot
            .lots
            .values()
            .all(|lot| lot.lot.state.is_terminal())
            && snapshot.released_lots >= total_lots
        {
            break;
        }
    }
    let snapshot = sim.snapshot();
    let simulated_ms = (sim.now_ms() - start_ms).max(0);
    let cycles: Vec<f64> = completed
        .iter()
        .filter_map(|(lot_id, at)| {
            snapshot
                .lots
                .get(lot_id)
                .map(|aggregate| (at - aggregate.lot.created_at_ms) as f64)
        })
        .filter(|cycle| *cycle > 0.0)
        .collect();
    let mean_cycle = (!cycles.is_empty()).then(|| cycles.iter().sum::<f64>() / cycles.len() as f64);
    let throughput = (simulated_ms > 0 && !completed.is_empty())
        .then(|| completed.len() as f64 / (simulated_ms as f64 / 3_600_000.0));
    let wip_end = snapshot
        .lots
        .values()
        .filter(|lot| !lot.lot.state.is_terminal())
        .count() as u64;
    Ok(WhatIfRun {
        label: label.to_owned(),
        rule,
        added_tool,
        completed_lots: completed.len() as u64,
        mean_cycle_time_ms: mean_cycle,
        throughput_lots_per_hour: throughput,
        wip_end,
        simulated_ms,
        timed_out,
        from_start: true,
    })
}

fn optional_delta(variant: Option<f64>, baseline: Option<f64>) -> Option<f64> {
    match (variant, baseline) {
        (Some(variant), Some(baseline)) => Some(variant - baseline),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario() -> Scenario {
        let mut scenario =
            Scenario::from_json(include_str!("../../../config/scenario.json")).unwrap();
        scenario.failure_probability = 0.0;
        scenario.lot_count = 3;
        scenario.interarrival_ms = 0;
        scenario
    }

    fn warmed(scenario: &Scenario, steps: usize) -> (Simulation, SimState) {
        let mut sim = Simulation::new(scenario.clone()).unwrap();
        for _ in 0..steps {
            let output = sim.step();
            sim.mark_persisted(output.events.len() as u64);
        }
        let state = sim.snapshot();
        (sim, state)
    }

    #[test]
    fn the_first_decision_names_its_rule_priority_and_qualified_tools() {
        let scenario = scenario();
        let (sim, _) = warmed(&scenario, 1);
        let view = sim.view();
        let decision = view.dispatch.last().expect("a dispatch on the first tick");
        assert_eq!(decision.rule, DispatchRule::PriorityThenFifo.label());
        assert_eq!(decision.step_no, 1);
        assert!(
            decision
                .qualified_tools
                .iter()
                .any(|tool| tool == &decision.equipment_id)
        );
        assert!(decision.queue_len >= 1);
        assert!(decision.priority > 0);
    }

    #[test]
    fn what_if_runs_from_the_snapshot_without_touching_the_live_sim() {
        let scenario = scenario();
        let (sim, state) = warmed(&scenario, 40);
        let before = sim.snapshot();
        let comparison = compare(
            &scenario,
            &state,
            &WhatIfOptions {
                rule: DispatchRule::Fifo,
                add_tool_kind: Some("clean".to_owned()),
                horizon_ms: 600_000,
            },
        )
        .unwrap();
        assert_eq!(sim.snapshot(), before, "the live state changed");
        assert_eq!(comparison.variant.added_tool.as_deref(), Some("CLEAN-X1"));
        assert!(comparison.baseline.completed_lots > 0);
        assert!(comparison.variant.completed_lots > 0);
        assert!(
            !comparison.notes.is_empty(),
            "the comparison explains its numbers"
        );
    }

    #[test]
    fn what_if_runs_even_when_the_live_line_is_paused() {
        let scenario = scenario();
        let (mut sim, _) = warmed(&scenario, 10);
        sim.set_running(false);
        let state = sim.snapshot();
        let comparison = compare(&scenario, &state, &WhatIfOptions::default()).unwrap();
        assert!(comparison.baseline.completed_lots > 0);
    }

    #[test]
    fn a_finished_line_replays_from_t0_and_says_so() {
        let scenario = scenario();
        let (sim, _) = warmed(&scenario, 5000);
        assert!(
            sim.snapshot()
                .lots
                .values()
                .all(|lot| lot.lot.state.is_terminal())
        );
        let comparison = compare(&scenario, &sim.snapshot(), &WhatIfOptions::default()).unwrap();
        assert!(comparison.baseline.from_start);
        assert!(comparison.baseline.completed_lots > 0);
        assert!(comparison.notes.iter().any(|note| note.contains("t0")));
    }

    #[test]
    fn adding_a_tool_of_an_unknown_kind_is_refused() {
        let scenario = scenario();
        let state = Simulation::new(scenario.clone()).unwrap().snapshot();
        let error = compare(
            &scenario,
            &state,
            &WhatIfOptions {
                add_tool_kind: Some("plating".to_owned()),
                ..WhatIfOptions::default()
            },
        )
        .unwrap_err();
        assert!(matches!(error, SimError::UnknownKind(_)));
    }

    #[test]
    fn the_delta_is_variant_minus_baseline() {
        let comparison = WhatIfComparison::new(
            WhatIfRun {
                label: "baseline".to_owned(),
                rule: DispatchRule::PriorityThenFifo,
                added_tool: None,
                completed_lots: 2,
                mean_cycle_time_ms: Some(100.0),
                throughput_lots_per_hour: Some(4.0),
                wip_end: 3,
                simulated_ms: 1_000,
                timed_out: false,
                from_start: false,
            },
            WhatIfRun {
                label: "variant".to_owned(),
                rule: DispatchRule::Fifo,
                added_tool: None,
                completed_lots: 4,
                mean_cycle_time_ms: Some(80.0),
                throughput_lots_per_hour: Some(8.0),
                wip_end: 1,
                simulated_ms: 1_000,
                timed_out: false,
                from_start: false,
            },
        );
        assert_eq!(comparison.delta.completed_lots, 2);
        assert_eq!(comparison.delta.mean_cycle_time_ms, Some(-20.0));
        assert_eq!(comparison.delta.wip_end, -2);
    }
}
