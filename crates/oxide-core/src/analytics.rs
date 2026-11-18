//! Read-only projections the workbench shows: recipe verification, Q-time,
//! tool utilisation and the KPI row.
//!
//! Everything is derived from the event log and the scenario. No table is
//! added and no value is synthesised: each field is either counted in
//! `lot_event` or read from the route/recipe the scenario seeded, and
//! [`Analysis::notes`] states the formula the UI must show next to the number.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use crate::events::{DomainEvent, EventKind};
use crate::scenario::Scenario;

/// Every projection the client needs, in one payload.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Analysis {
    pub recipes: Vec<RecipeCheck>,
    pub qtime_limits: Vec<QTimeLimit>,
    pub qtime_violations: Vec<QTimeViolation>,
    pub alarms: Vec<Alarm>,
    pub kpi: Kpi,
    pub notes: Vec<MetricNote>,
}

/// An equipment alarm or its clear, with the event that carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alarm {
    pub event_id: String,
    pub equipment_id: String,
    /// `down` or `up`.
    pub kind: String,
    pub at_ms: i64,
    pub reason: String,
    pub down_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecipeStatus {
    /// The last run's checksum matched the seeded recipe.
    Verified,
    /// A run reported a checksum that is not the seeded one.
    Mismatched,
    /// The recipe exists but no tool has reported running it yet.
    NotRun,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecipeCheck {
    pub recipe_id: String,
    pub version: i32,
    pub checksum: String,
    pub steps: Vec<i32>,
    pub runs: u64,
    pub verified_runs: u64,
    pub mismatched_runs: u64,
    pub last_run_ms: Option<i64>,
    pub status: RecipeStatus,
}

/// The Q-time limit that applies between leaving `from_step` and entering the
/// next step, as the route declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QTimeLimit {
    pub from_step: i32,
    pub to_step: i32,
    pub limit_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QTimeViolation {
    pub lot_id: String,
    pub wafer_ids: Vec<String>,
    pub from_step: i32,
    pub to_step: i32,
    pub elapsed_ms: i64,
    pub limit_ms: i64,
    pub occurred_at_ms: i64,
    /// The `event_id` of the track-in that ended the wait; it is addressable in
    /// the outbox and in `GreptimeDB`.
    pub event_id: String,
    pub equipment_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Kpi {
    pub lots_created: u64,
    pub lots_completed: u64,
    pub wafers_created: u64,
    pub wafers_scrapped: u64,
    pub first_pass_yield: Option<f64>,
    pub mean_cycle_time_ms: Option<f64>,
    pub max_cycle_time_ms: Option<i64>,
    pub throughput_lots_per_hour: Option<f64>,
    pub wip_lots: u64,
    pub window_start_ms: Option<i64>,
    pub window_end_ms: Option<i64>,
    pub tools: Vec<ToolKpi>,
    pub defects: Vec<DefectCount>,
    /// Planned duration against the measured mean, per route step.
    pub step_cycles: Vec<StepCycle>,
    /// Completed lots' cycle times, newest first.
    pub lot_cycles: Vec<LotCycle>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StepCycle {
    pub step_no: i32,
    pub planned_ms: i64,
    pub actual_mean_ms: Option<f64>,
    pub samples: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LotCycle {
    pub lot_id: String,
    pub created_ms: i64,
    pub completed_ms: i64,
    pub cycle_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolKpi {
    pub equipment_id: String,
    pub kind: String,
    pub busy_ms: i64,
    pub down_ms: i64,
    pub track_ins: u64,
    pub track_outs: u64,
    /// Busy time over the event window, in 0..=1.
    pub utilization: f64,
    /// One minus down time over the event window, in 0..=1.
    pub availability: f64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefectCount {
    pub class: String,
    pub count: u64,
}

/// The formula behind a number, shown in the UI next to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricNote {
    pub metric: String,
    pub source: String,
    pub formula: String,
}

/// Classify a defect note into the four classes the wafer grid marks.
#[must_use]
pub fn defect_class(info: &str) -> &'static str {
    let text = info.to_ascii_lowercase();
    for (needle, class) in [
        ("scratch", "scratch"),
        ("particle", "particle"),
        ("resist", "resist"),
        ("pattern", "pattern"),
        ("cd", "pattern"),
        ("defocus", "pattern"),
    ] {
        if text.contains(needle) {
            return class;
        }
    }
    "other"
}

/// Fold the whole event log into the workbench's analytics.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "one pass over the log, one arm per event kind; splitting it would scan the log twice"
)]
pub fn analyse(events: &[DomainEvent], scenario: &Scenario) -> Analysis {
    let mut recipes: BTreeMap<String, RecipeCheck> = scenario
        .recipes()
        .into_iter()
        .map(|recipe| {
            let steps: Vec<i32> = scenario
                .route
                .steps
                .iter()
                .filter(|step| step.recipe_id == recipe.id)
                .map(|step| step.step_no)
                .collect();
            (
                recipe.id.clone(),
                RecipeCheck {
                    recipe_id: recipe.id,
                    version: recipe.version,
                    checksum: recipe.checksum,
                    steps,
                    runs: 0,
                    verified_runs: 0,
                    mismatched_runs: 0,
                    last_run_ms: None,
                    status: RecipeStatus::NotRun,
                },
            )
        })
        .collect();

    let mut violations = Vec::new();
    let mut alarms = Vec::new();
    let mut step_durations: BTreeMap<i32, (i64, u64)> = BTreeMap::new();
    let mut track_out_at: HashMap<(String, i32), (i64, String)> = HashMap::new();
    let mut first_seen: HashMap<String, i64> = HashMap::new();
    let mut completed_at: HashMap<String, i64> = HashMap::new();
    let mut created_lots: BTreeSet<String> = BTreeSet::new();
    let mut terminal_lots: BTreeSet<String> = BTreeSet::new();
    let mut wafers: BTreeSet<String> = BTreeSet::new();
    let mut scrapped: u64 = 0;
    let mut defects: BTreeMap<String, u64> = BTreeMap::new();
    let mut tool_busy: HashMap<String, i64> = HashMap::new();
    let mut tool_down: HashMap<String, i64> = HashMap::new();
    let mut tool_ins: HashMap<String, u64> = HashMap::new();
    let mut tool_outs: HashMap<String, u64> = HashMap::new();
    let mut window_start: Option<i64> = None;
    let mut window_end: Option<i64> = None;

    for event in events {
        let at = event.occurred_at_ms;
        window_start = Some(window_start.map_or(at, |start| start.min(at)));
        window_end = Some(window_end.map_or(at, |end| end.max(at)));
        first_seen
            .entry(event.lot_id.clone())
            .and_modify(|first| *first = (*first).min(at))
            .or_insert(at);

        for wafer in event.wafers() {
            wafers.insert(wafer);
        }
        if let Some(wafer) = &event.wafer_id {
            wafers.insert(wafer.clone());
        }

        match event.kind {
            EventKind::LotCreated => {
                created_lots.insert(event.lot_id.clone());
            }
            EventKind::TrackIn => {
                if let Some(equipment) = event.equipment_id() {
                    *tool_ins.entry(equipment.to_owned()).or_default() += 1;
                }
                let Some(step) = event.step_no() else {
                    continue;
                };
                if step <= 1 {
                    continue;
                }
                let Some((left_at, _)) = track_out_at.get(&(event.lot_id.clone(), step - 1)) else {
                    continue;
                };
                let Some(step_spec) = scenario.route.step(step - 1) else {
                    continue;
                };
                let Some(limit_ms) = step_spec.qtime_ms.filter(|limit| *limit > 0) else {
                    continue;
                };
                let elapsed = at - *left_at;
                if elapsed > limit_ms {
                    violations.push(QTimeViolation {
                        lot_id: event.lot_id.clone(),
                        wafer_ids: event.wafers(),
                        from_step: step - 1,
                        to_step: step,
                        elapsed_ms: elapsed,
                        limit_ms,
                        occurred_at_ms: at,
                        event_id: event.event_id.to_string(),
                        equipment_id: event.equipment_id().unwrap_or_default().to_owned(),
                    });
                }
            }
            EventKind::EquipmentDown => {
                if let Some(equipment) = event.equipment_id() {
                    alarms.push(Alarm {
                        event_id: event.event_id.to_string(),
                        equipment_id: equipment.to_owned(),
                        kind: "down".to_owned(),
                        at_ms: at,
                        reason: event
                            .payload
                            .get("reason")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("equipment down")
                            .to_owned(),
                        down_ms: 0,
                    });
                }
            }
            EventKind::EquipmentUp => {
                if let Some(equipment) = event.equipment_id() {
                    let down_ms = event
                        .payload
                        .get("down_ms")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0);
                    *tool_down.entry(equipment.to_owned()).or_default() += down_ms.max(0);
                    alarms.push(Alarm {
                        event_id: event.event_id.to_string(),
                        equipment_id: equipment.to_owned(),
                        kind: "up".to_owned(),
                        at_ms: at,
                        reason: format!("恢复，停机 {:.1}s", down_ms as f64 / 1000.0),
                        down_ms,
                    });
                }
            }
            EventKind::TrackOut => {
                let Some(equipment) = event.equipment_id() else {
                    continue;
                };
                *tool_outs.entry(equipment.to_owned()).or_default() += 1;
                if let Some(duration) = event
                    .payload
                    .get("duration_ms")
                    .and_then(serde_json::Value::as_i64)
                {
                    *tool_busy.entry(equipment.to_owned()).or_default() += duration.max(0);
                    if let Some(step) = event.step_no() {
                        let entry = step_durations.entry(step).or_default();
                        entry.0 += duration.max(0);
                        entry.1 += 1;
                    }
                }
                if let Some(step) = event.step_no() {
                    track_out_at.insert(
                        (event.lot_id.clone(), step),
                        (at, event.event_id.to_string()),
                    );
                    if let Some(recipe_id) = event.recipe_id()
                        && let Some(check) = recipes.get_mut(recipe_id)
                    {
                        check.runs += 1;
                        check.last_run_ms = Some(at);
                        let checksum = event
                            .payload
                            .get("recipe_checksum")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or_default();
                        if checksum == check.checksum {
                            check.verified_runs += 1;
                        } else {
                            check.mismatched_runs += 1;
                        }
                        check.status = if check.mismatched_runs > 0 {
                            RecipeStatus::Mismatched
                        } else {
                            RecipeStatus::Verified
                        };
                    }
                }
            }
            EventKind::LotCompleted => {
                completed_at.insert(event.lot_id.clone(), at);
                terminal_lots.insert(event.lot_id.clone());
            }
            EventKind::LotSplit | EventKind::LotMerged => {
                terminal_lots.insert(event.lot_id.clone());
            }
            EventKind::WaferScrapped => {
                scrapped += 1;
                let reason = event
                    .payload
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("other");
                *defects.entry(defect_class(reason).to_owned()).or_default() += 1;
            }
            _ => {}
        }
    }

    for check in recipes.values_mut() {
        if check.runs == 0 {
            check.status = RecipeStatus::NotRun;
        }
    }

    let window_ms = match (window_start, window_end) {
        (Some(start), Some(end)) if end > start => end - start,
        _ => 0,
    };
    let tools: Vec<ToolKpi> = scenario
        .equipment
        .iter()
        .map(|tool| {
            let busy_ms = tool_busy.get(&tool.id).copied().unwrap_or(0);
            let down_ms = tool_down.get(&tool.id).copied().unwrap_or(0);
            let (utilization, availability) = if window_ms > 0 {
                (
                    (busy_ms as f64 / window_ms as f64).clamp(0.0, 1.0),
                    (1.0 - down_ms as f64 / window_ms as f64).clamp(0.0, 1.0),
                )
            } else {
                (0.0, 1.0)
            };
            ToolKpi {
                equipment_id: tool.id.clone(),
                kind: tool.kind.clone(),
                busy_ms,
                down_ms,
                track_ins: tool_ins.get(&tool.id).copied().unwrap_or(0),
                track_outs: tool_outs.get(&tool.id).copied().unwrap_or(0),
                utilization,
                availability,
            }
        })
        .collect();

    let cycles: Vec<i64> = completed_at
        .iter()
        .filter_map(|(lot, done)| first_seen.get(lot).map(|first| done - first))
        .filter(|cycle| *cycle > 0)
        .collect();
    let mean_cycle = (!cycles.is_empty())
        .then(|| cycles.iter().map(|v| *v as f64).sum::<f64>() / cycles.len() as f64);
    let max_cycle = cycles.iter().copied().max();
    let throughput = (window_ms > 0 && !completed_at.is_empty())
        .then(|| completed_at.len() as f64 / (window_ms as f64 / 3_600_000.0));
    let wafers_created = wafers.len() as u64;
    let first_pass_yield = (wafers_created > 0)
        .then(|| (wafers_created.saturating_sub(scrapped)) as f64 / wafers_created as f64);
    let qtime_limits: Vec<QTimeLimit> = scenario
        .route
        .steps
        .iter()
        .filter_map(|step| {
            let limit_ms = step.qtime_ms.filter(|limit| *limit > 0)?;
            scenario
                .route
                .next_step(step.step_no)
                .map(|next| QTimeLimit {
                    from_step: step.step_no,
                    to_step: next,
                    limit_ms,
                })
        })
        .collect();

    let step_cycles: Vec<StepCycle> = scenario
        .route
        .steps
        .iter()
        .map(|step| {
            let (sum, samples) = step_durations.get(&step.step_no).copied().unwrap_or((0, 0));
            StepCycle {
                step_no: step.step_no,
                planned_ms: step.duration_ms,
                actual_mean_ms: (samples > 0).then(|| sum as f64 / samples as f64),
                samples,
            }
        })
        .collect();
    let mut lot_cycles: Vec<LotCycle> = completed_at
        .iter()
        .filter_map(|(lot_id, completed)| {
            first_seen.get(lot_id).map(|created| LotCycle {
                lot_id: lot_id.clone(),
                created_ms: *created,
                completed_ms: *completed,
                cycle_ms: completed - created,
            })
        })
        .collect();
    lot_cycles.sort_by_key(|cycle| std::cmp::Reverse(cycle.completed_ms));

    Analysis {
        recipes: recipes.into_values().collect(),
        qtime_limits,
        qtime_violations: violations,
        alarms,
        kpi: Kpi {
            lots_created: created_lots.len() as u64,
            lots_completed: completed_at.len() as u64,
            wafers_created,
            wafers_scrapped: scrapped,
            first_pass_yield,
            mean_cycle_time_ms: mean_cycle,
            max_cycle_time_ms: max_cycle,
            throughput_lots_per_hour: throughput,
            wip_lots: created_lots.difference(&terminal_lots).count() as u64,
            window_start_ms: window_start,
            window_end_ms: window_end,
            tools,
            defects: defects
                .into_iter()
                .map(|(class, count)| DefectCount { class, count })
                .collect(),
            step_cycles,
            lot_cycles,
        },
        notes: notes(),
    }
}

fn notes() -> Vec<MetricNote> {
    let note = |metric: &str, source: &str, formula: &str| MetricNote {
        metric: metric.to_owned(),
        source: source.to_owned(),
        formula: formula.to_owned(),
    };
    vec![
        note(
            "周期时间",
            "lot_event",
            "同批次首条事件到 lot_completed 的 occurred_at 之差，取均值/最大值",
        ),
        note(
            "产出",
            "lot_event",
            "lot_completed 条数 ÷ (窗口末事件时刻 − 首事件时刻) × 3600000",
        ),
        note(
            "利用率",
            "track_out.duration_ms",
            "该机台 track_out 的 duration_ms 之和 ÷ 事件窗口时长",
        ),
        note(
            "可用率",
            "equipment_up.down_ms",
            "1 − 该机台 equipment_up 报告的 down_ms 之和 ÷ 事件窗口时长",
        ),
        note(
            "良率",
            "wafer_scrapped",
            "1 − 报废 wafer 数 ÷ 事件日志中出现的 wafer 总数",
        ),
        note(
            "Q-time",
            "track_out → track_in",
            "同批次上一道 track_out 与下一道 track_in 的 occurred_at 之差，超过 route_step.qtime_ms 记违规",
        ),
        note(
            "配方校验",
            "track_out.recipe_checksum",
            "track_out 上报的 checksum 与 scenario 配方 checksum 逐次比对",
        ),
        note(
            "WIP",
            "lot_created − 终止事件",
            "出现过 lot_created 且尚无 completed/split/merged 的批次数",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{TrackInReport, TrackOutReport, track_in_payload, track_out_payload};
    use crate::model::{Recipe, RouteStep};
    use serde_json::json;
    use uuid::Uuid;

    fn scenario() -> Scenario {
        Scenario::from_json(include_str!("../../../config/scenario.json")).unwrap()
    }

    fn event(kind: EventKind, lot: &str, at: i64, payload: serde_json::Value) -> DomainEvent {
        DomainEvent::new(Uuid::new_v4(), lot, kind, payload, at, "test")
    }

    fn track_in(step: i32, lot: &str, equipment: &str, recipe: &str, at: i64) -> DomainEvent {
        event(
            EventKind::TrackIn,
            lot,
            at,
            track_in_payload(&TrackInReport {
                step_no: step,
                equipment_id: equipment,
                recipe_id: recipe,
                wafers: &[format!("{lot}-W01"), format!("{lot}-W02")],
            }),
        )
    }

    fn track_out(
        step: i32,
        lot: &str,
        equipment: &str,
        recipe: &str,
        at: i64,
        duration: i64,
        checksum: &str,
    ) -> DomainEvent {
        event(
            EventKind::TrackOut,
            lot,
            at,
            track_out_payload(&TrackOutReport {
                step_no: step,
                equipment_id: equipment,
                recipe_id: recipe,
                recipe_checksum: checksum,
                wafers: &[format!("{lot}-W01"), format!("{lot}-W02")],
                duration_ms: duration,
                measured: None,
            }),
        )
    }

    #[test]
    fn a_late_track_in_is_a_qtime_violation_tied_to_its_event() {
        let scenario = scenario();
        let limit = scenario.route.step(1).unwrap().qtime_ms.unwrap();
        let left = track_out(1, "LOT-1", "CLN-01", "RCP-CLEAN", 1_000, 6_000, "sum");
        let late = track_in(2, "LOT-1", "LIT-01", "RCP-LITHO", 1_000 + limit + 1);
        let analysis = analyse(&[left, late.clone()], &scenario);
        assert_eq!(analysis.qtime_violations.len(), 1);
        let violation = &analysis.qtime_violations[0];
        assert_eq!(violation.from_step, 1);
        assert_eq!(violation.to_step, 2);
        assert_eq!(violation.wafer_ids.len(), 2);
        assert_eq!(violation.event_id, late.event_id.to_string());
        assert_eq!(violation.limit_ms, limit);
    }

    #[test]
    fn a_track_in_within_the_limit_is_not_a_violation() {
        let scenario = scenario();
        let left = track_out(1, "LOT-1", "CLN-01", "RCP-CLEAN", 1_000, 6_000, "sum");
        let ok = track_in(2, "LOT-1", "LIT-01", "RCP-LITHO", 2_000);
        let analysis = analyse(&[left, ok], &scenario);
        assert_eq!(analysis.qtime_violations.len(), 0);
    }

    #[test]
    fn a_checksum_mismatch_flags_the_recipe_and_the_run() {
        let scenario = scenario();
        let run = track_out(
            1,
            "LOT-1",
            "CLN-01",
            "RCP-CLEAN",
            5_000,
            6_000,
            "not-the-seed",
        );
        let analysis = analyse(&[run], &scenario);
        let recipe = analysis
            .recipes
            .iter()
            .find(|recipe| recipe.recipe_id == "RCP-CLEAN")
            .unwrap();
        assert_eq!(recipe.mismatched_runs, 1);
        assert_eq!(recipe.status, RecipeStatus::Mismatched);
    }

    #[test]
    fn a_matching_checksum_is_verified() {
        let scenario = scenario();
        let checksum = scenario.recipe("RCP-CLEAN").unwrap().checksum;
        let run = track_out(1, "LOT-1", "CLN-01", "RCP-CLEAN", 5_000, 6_000, &checksum);
        let analysis = analyse(&[run], &scenario);
        let recipe = analysis
            .recipes
            .iter()
            .find(|recipe| recipe.recipe_id == "RCP-CLEAN")
            .unwrap();
        assert_eq!(recipe.verified_runs, 1);
        assert_eq!(recipe.status, RecipeStatus::Verified);
    }

    #[test]
    fn a_finished_lot_contributes_a_cycle_time_and_throughput() {
        let scenario = scenario();
        let events = vec![
            event(
                EventKind::LotCreated,
                "LOT-1",
                0,
                json!({"wafers": ["LOT-1-W01", "LOT-1-W02"], "wafer_count": 2}),
            ),
            track_in(1, "LOT-1", "CLN-01", "RCP-CLEAN", 1_000),
            track_out(1, "LOT-1", "CLN-01", "RCP-CLEAN", 7_000, 6_000, "x"),
            event(EventKind::LotCompleted, "LOT-1", 60_000, json!({})),
        ];
        let analysis = analyse(&events, &scenario);
        assert_eq!(analysis.kpi.lots_completed, 1);
        assert_eq!(analysis.kpi.wafers_created, 2);
        assert_eq!(analysis.kpi.mean_cycle_time_ms, Some(60_000.0));
        assert_eq!(analysis.kpi.wip_lots, 0);
        let throughput = analysis.kpi.throughput_lots_per_hour.unwrap();
        assert!((throughput - 60.0).abs() < 0.001, "{throughput}");
    }

    #[test]
    fn scrapped_wafers_set_the_yield_and_the_defect_classes() {
        let scenario = scenario();
        let events = vec![
            track_in(1, "LOT-1", "CLN-01", "RCP-CLEAN", 1_000),
            event(
                EventKind::WaferScrapped,
                "LOT-1",
                2_000,
                json!({"wafer_id": "LOT-1-W01", "reason": "scratch at edge"}),
            ),
        ];
        let analysis = analyse(&events, &scenario);
        assert_eq!(analysis.kpi.wafers_scrapped, 1);
        assert_eq!(analysis.kpi.first_pass_yield, Some(0.5));
        assert_eq!(
            analysis.kpi.defects,
            vec![DefectCount {
                class: "scratch".to_owned(),
                count: 1
            }]
        );
    }

    #[test]
    fn utilisation_is_busy_time_over_the_event_window() {
        let scenario = scenario();
        let events = vec![
            track_in(1, "LOT-1", "CLN-01", "RCP-CLEAN", 0),
            track_out(1, "LOT-1", "CLN-01", "RCP-CLEAN", 6_000, 6_000, "x"),
            event(EventKind::LotCompleted, "LOT-1", 10_000, json!({})),
        ];
        let analysis = analyse(&events, &scenario);
        let tool = analysis
            .kpi
            .tools
            .iter()
            .find(|tool| tool.equipment_id == "CLN-01")
            .unwrap();
        assert_eq!(tool.busy_ms, 6_000);
        assert!((tool.utilization - 0.6).abs() < 0.001, "{tool:?}");
    }

    #[test]
    fn every_kpi_has_a_note_and_the_route_declares_three_limits() {
        let scenario = scenario();
        let analysis = analyse(&[], &scenario);
        assert_eq!(analysis.notes.len(), 8);
        assert_eq!(analysis.qtime_limits.len(), 3);
        assert_eq!(analysis.qtime_limits[0].from_step, 1);
        assert_eq!(analysis.qtime_limits[2].to_step, 4);
    }

    #[test]
    fn defect_notes_are_classified_without_a_defect_database() {
        assert_eq!(defect_class("particle cluster"), "particle");
        assert_eq!(defect_class("PR scratch"), "scratch");
        assert_eq!(defect_class("CD drift"), "pattern");
        assert_eq!(defect_class("unknown"), "other");
    }

    #[test]
    fn an_unused_recipe_is_not_run() {
        let analysis = analyse(&[], &scenario());
        assert!(
            analysis
                .recipes
                .iter()
                .all(|recipe| recipe.status == RecipeStatus::NotRun)
        );
    }

    #[test]
    fn helper_types_are_constructible() {
        let _ = RouteStep {
            step_no: 1,
            name: "clean".to_owned(),
            equipment_kind: "clean".to_owned(),
            equipment_qualification: vec![],
            recipe_id: "RCP-CLEAN".to_owned(),
            duration_ms: 1,
            target: None,
            ucl: None,
            lcl: None,
            qtime_ms: None,
        };
        let _ = Recipe::new("R", 1, "body");
    }
}
