//! Discrete-event simulation engine.
//!
//! The engine keeps a `BinaryHeap` of scheduled events, one queue per route
//! step and a bucket of tools per equipment kind. Every tick advances the
//! simulated clock, drains the events that became due and dispatches material
//! with hand-written rules: FIFO within a priority class, a qualification
//! matrix, and a verified recipe. No solver, no per-item thread.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, VecDeque};
use std::ops::ControlFlow;

use oxide_core::events::{DomainEvent, EventData, EventKind, TrackInReport, TrackOutReport};
use oxide_core::model::{
    Equipment, EquipmentState, Lot, LotId, LotState, Route, WaferId, WaferState,
    deterministic_event_id, lot_id_for,
};
use oxide_core::scenario::Scenario;
use oxide_core::state_machine::{
    EventIds, LotAggregate, TransitionError, equipment_down, equipment_up,
};
use serde::{Deserialize, Serialize};

/// Why the simulator refused an operation.
///
/// The API maps these onto status codes: an unknown lot is a 404, an illegal
/// transition or a self-merge is a 400, and a bad scenario is the server's own
/// fault. A `String` could not carry that distinction.
#[derive(Debug, thiserror::Error)]
pub enum SimError {
    #[error("unknown lot {0}")]
    UnknownLot(String),
    #[error("a lot cannot be merged into itself")]
    MergingIntoSelf,
    #[error(transparent)]
    Transition(#[from] TransitionError),
    #[error("invalid scenario: {0}")]
    Scenario(#[from] oxide_core::ScenarioError),
    #[error("no process step runs on tool kind {0}")]
    UnknownKind(String),
    #[error("the what-if tool {0} already exists")]
    DuplicateTool(String),
}

/// A message the simulated equipment sends to the SECS/GEM host.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SecsTrigger {
    /// Host uploads a process program (S7F3), the equipment verifies it.
    RecipeUpload {
        recipe_id: String,
        checksum: String,
        body: String,
    },
    /// Process started on a tool (S6F11, CEID 1001).
    ProcessStarted {
        lot_id: String,
        step_no: i32,
        equipment_id: String,
    },
    /// Process completed on a tool (S6F11, CEID 1002).
    ProcessCompleted {
        lot_id: String,
        step_no: i32,
        equipment_id: String,
    },
    /// Tool alarm (S5F1).
    Alarm {
        equipment_id: String,
        alid: u32,
        text: String,
    },
}

/// What one tick produced: domain events to persist, SECS messages to send.
#[derive(Debug, Default, Clone)]
pub struct TickOutput {
    pub events: Vec<DomainEvent>,
    pub secs: Vec<SecsTrigger>,
}

/// How the dispatcher chooses among eligible lots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DispatchRule {
    /// Highest priority first, FIFO within a priority (the shipped rule).
    #[default]
    PriorityThenFifo,
    /// Strict FIFO: the head of the queue goes first, whatever its priority.
    Fifo,
}

impl DispatchRule {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            DispatchRule::PriorityThenFifo => "优先级 → FIFO",
            DispatchRule::Fifo => "FIFO",
        }
    }
}

/// One dispatch decision, with the inputs that produced it.
///
/// This is the read model behind "every dispatch shows the rule it hit": the
/// priority, the queue it came from and the tool that was qualified for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchDecision {
    pub at_ms: i64,
    pub lot_id: String,
    pub step_no: i32,
    pub equipment_id: String,
    pub priority: i32,
    /// Zero-based position in the step's queue when it was picked.
    pub queue_position: usize,
    pub queue_len: usize,
    /// Tools the route step declares qualified for its recipe.
    pub qualified_tools: Vec<String>,
    pub recipe_id: String,
    pub rule: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ScheduledKind {
    ReleaseLot,
    StepComplete {
        equipment_id: String,
        lot_id: String,
        step_no: i32,
        wafers: Vec<WaferId>,
    },
    Repair {
        equipment_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scheduled {
    pub at_ms: i64,
    pub seq: u64,
    pub kind: ScheduledKind,
}

impl Ord for Scheduled {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (self.at_ms, self.seq).cmp(&(other.at_ms, other.seq))
    }
}

impl PartialOrd for Scheduled {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// A tool on the simulated floor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EquipmentRuntime {
    pub equipment: Equipment,
    pub busy_lot: Option<String>,
    pub busy_step: i32,
    pub busy_recipe: String,
    pub busy_wafers: Vec<WaferId>,
    pub busy_until_ms: i64,
    pub down_since_ms: i64,
    pub down_until_ms: i64,
    pub lots_processed: u64,
}

impl EquipmentRuntime {
    fn new(equipment: Equipment) -> Self {
        Self {
            equipment,
            busy_lot: None,
            busy_step: 0,
            busy_recipe: String::new(),
            busy_wafers: vec![],
            busy_until_ms: 0,
            down_since_ms: 0,
            down_until_ms: 0,
            lots_processed: 0,
        }
    }

    /// Green when producing, amber when held for maintenance, red when down.
    #[must_use]
    pub fn light(&self) -> &'static str {
        match self.equipment.state {
            EquipmentState::Down => "red",
            EquipmentState::Maintenance => "amber",
            _ => "green",
        }
    }
}

/// Everything needed to resume a simulation: no heap, no handles.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SimState {
    pub now_ms: i64,
    pub seed: u64,
    pub rng: u64,
    pub event_counter: u64,
    pub lots: BTreeMap<String, LotAggregate>,
    pub equipment: BTreeMap<String, EquipmentRuntime>,
    pub queues: BTreeMap<i32, VecDeque<String>>,
    pub released_lots: usize,
    pub next_lot_serial: u64,
    pub next_release_ms: i64,
    pub running: bool,
    pub speed: f64,
    pub persisted_events: u64,
    /// The `lot_event.seq` this state corresponds to. It is written next to
    /// the state so a snapshot cannot claim a log position the state does not
    /// contain.
    #[serde(default)]
    pub last_seq: i64,
    /// The most recent dispatch decisions, newest last. In-memory only: it is
    /// a view of the log, not a second source of truth, and snapshots carry it
    /// as a convenience.
    #[serde(default)]
    pub dispatch: Vec<DispatchDecision>,
}

/// The simulator.
///
/// The fields are crate-private: the engine's invariants (the heap mirroring
/// the busy tools, every lot in exactly one queue) only hold if nothing else
/// mutates them. Callers read through [`Simulation::view`] and
/// [`Simulation::snapshot`].
#[derive(Debug)]
pub struct Simulation {
    pub(crate) scenario: Scenario,
    pub(crate) route: Route,
    pub(crate) state: SimState,
    heap: BinaryHeap<Reverse<Scheduled>>,
    schedule_seq: u64,
    rule: DispatchRule,
}

impl Simulation {
    /// Start a new fab from `scenario`.
    ///
    /// # Errors
    /// [`SimError::Scenario`] when the scenario is not runnable; see
    /// [`Scenario::validate`].
    pub fn new(scenario: Scenario) -> Result<Self, SimError> {
        scenario.validate()?;
        let route = scenario.route.clone();
        let equipment: BTreeMap<String, EquipmentRuntime> = scenario
            .equipment
            .iter()
            .map(|spec| {
                let runtime = EquipmentRuntime::new(spec.to_equipment());
                (runtime.equipment.id.clone(), runtime)
            })
            .collect();
        // Start at wall-clock time so the derived time series carries real
        // timestamps; the schedule itself stays relative.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as i64);
        let state = SimState {
            now_ms,
            seed: scenario.seed,
            rng: scenario.seed.max(1),
            event_counter: 0,
            lots: BTreeMap::new(),
            equipment,
            queues: BTreeMap::new(),
            released_lots: 0,
            next_lot_serial: 1,
            next_release_ms: now_ms,
            running: true,
            speed: scenario.speed,
            persisted_events: 0,
            last_seq: 0,
            dispatch: Vec::new(),
        };
        let mut sim = Self {
            scenario,
            route,
            state,
            heap: BinaryHeap::new(),
            schedule_seq: 0,
            rule: DispatchRule::default(),
        };
        sim.schedule_release();
        Ok(sim)
    }

    /// Resume from a snapshot. The caller applies any newer events with
    /// [`Simulation::apply_event`] and then calls [`Simulation::rebuild_heap`].
    pub fn from_state(scenario: Scenario, state: SimState) -> Result<Self, SimError> {
        scenario.validate()?;
        let route = scenario.route.clone();
        Ok(Self {
            scenario,
            route,
            state,
            heap: BinaryHeap::new(),
            schedule_seq: 0,
            rule: DispatchRule::default(),
        })
    }

    /// Choose a different dispatch rule (used by the offline what-if).
    #[must_use]
    pub fn with_rule(mut self, rule: DispatchRule) -> Self {
        self.rule = rule;
        self
    }

    #[must_use]
    pub fn rule(&self) -> DispatchRule {
        self.rule
    }

    #[must_use]
    pub fn scenario(&self) -> &Scenario {
        &self.scenario
    }

    #[must_use]
    pub fn snapshot(&self) -> SimState {
        self.state.clone()
    }

    #[must_use]
    pub fn is_running(&self) -> bool {
        self.state.running
    }

    pub fn set_running(&mut self, running: bool) {
        self.state.running = running;
    }

    pub fn set_speed(&mut self, speed: f64) {
        self.state.speed = speed.clamp(0.1, 100.0);
    }

    #[must_use]
    pub fn speed(&self) -> f64 {
        self.state.speed
    }

    #[must_use]
    pub fn now_ms(&self) -> i64 {
        self.state.now_ms
    }

    pub fn mark_persisted(&mut self, count: u64) {
        self.state.persisted_events += count;
    }

    #[must_use]
    pub fn persisted_events(&self) -> u64 {
        self.state.persisted_events
    }

    /// The log position this state reflects.
    #[must_use]
    pub fn last_seq(&self) -> i64 {
        self.state.last_seq
    }

    /// Move the recorded log position forward. Never moves it back.
    pub fn advance_seq(&mut self, seq: i64) {
        self.state.last_seq = self.state.last_seq.max(seq);
    }

    /// Put back a state copied before a decision that did not commit.
    pub fn restore(&mut self, state: SimState) {
        self.state = state;
        self.rebuild_heap();
    }

    /// Advance the clock by `dt_ms` and process everything that became due.
    pub fn tick(&mut self, dt_ms: i64) -> TickOutput {
        let mut output = TickOutput::default();
        if !self.state.running {
            return output;
        }
        self.state.now_ms += dt_ms.max(0);
        self.release_due_lots(&mut output);
        loop {
            let due = match self.heap.peek() {
                Some(Reverse(next)) if next.at_ms <= self.state.now_ms => {
                    self.heap.pop().map(|Reverse(scheduled)| scheduled)
                }
                _ => None,
            };
            let Some(scheduled) = due else { break };
            self.handle_scheduled(scheduled, &mut output);
        }
        self.dispatch(&mut output);
        output
    }

    /// Run one tick at the scenario's base tick length.
    pub fn step(&mut self) -> TickOutput {
        let tick = self.scenario.tick_ms.max(1);
        self.tick(tick)
    }

    /// One tick while paused: the operator's single step.
    ///
    /// `tick` does nothing unless the line runs, so this releases the guard for
    /// the duration of one tick and leaves the line paused afterwards.
    pub fn force_step(&mut self) -> TickOutput {
        self.state.running = true;
        let output = self.step();
        self.state.running = false;
        output
    }

    /// Put back on the heap whatever the restored state implies.
    pub fn rebuild_heap(&mut self) {
        self.heap.clear();
        let mut pending: Vec<(i64, ScheduledKind)> = Vec::new();
        for runtime in self.state.equipment.values() {
            if runtime.equipment.state == EquipmentState::Running {
                if let Some(lot_id) = &runtime.busy_lot {
                    pending.push((
                        runtime.busy_until_ms.max(self.state.now_ms),
                        ScheduledKind::StepComplete {
                            equipment_id: runtime.equipment.id.clone(),
                            lot_id: lot_id.clone(),
                            step_no: runtime.busy_step,
                            wafers: runtime.busy_wafers.clone(),
                        },
                    ));
                }
            } else if runtime.equipment.state == EquipmentState::Down {
                pending.push((
                    runtime.down_until_ms.max(self.state.now_ms),
                    ScheduledKind::Repair {
                        equipment_id: runtime.equipment.id.clone(),
                    },
                ));
            }
        }
        if self.state.released_lots < self.scenario.lot_count {
            // A snapshot that lagged the log must not make the next release
            // happen in the past all at once.
            if self.state.next_release_ms <= self.state.now_ms {
                self.state.next_release_ms = self.state.now_ms + self.scenario.interarrival_ms;
            }
            pending.push((self.state.next_release_ms, ScheduledKind::ReleaseLot));
        }
        for (at_ms, kind) in pending {
            self.schedule(at_ms, kind);
        }
    }

    /// Re-apply one persisted event to the state (used after a snapshot).
    ///
    /// The projection is the state machine itself: the command is rebuilt from
    /// the payload and run through the transition the live engine ran.
    /// `note_event` then applies the runtime consequences (queues, tool state)
    /// that live commits also go through, so a replay cannot drift by
    /// re-implementing a rule the live path already has.
    pub fn apply_event(&mut self, event: &DomainEvent) {
        self.state.now_ms = self.state.now_ms.max(event.occurred_at_ms);
        self.state.event_counter = self.state.event_counter.saturating_add(1);
        self.replay(event);
        self.note_event(event);
    }

    /// Run the transition a logged event records.
    ///
    /// The returned events are discarded: the log is already the output. What
    /// matters is the mutation the transition makes, which is the same one the
    /// live engine made before it committed the event.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per event kind; each rebuilds the command that produced it"
    )]
    fn replay(&mut self, event: &DomainEvent) {
        let now = event.occurred_at_ms;
        let actor = event.actor.as_str();
        let mut ids = EventIds::new(uuid::Uuid::nil());
        match &event.data {
            EventData::LotCreated {
                product_id,
                route_id,
                priority,
                wafers,
                ..
            } => {
                let lot = Lot::new(
                    event.lot_id.clone(),
                    product_id,
                    route_id,
                    *priority,
                    wafers.len() as i32,
                    now,
                );
                match LotAggregate::create(lot, wafers.clone(), &mut ids, now, actor) {
                    Ok(transition) => {
                        if let Some(aggregate) = transition.children.into_iter().next() {
                            self.state
                                .lots
                                .entry(event.lot_id.clone())
                                .or_insert(aggregate);
                        }
                    }
                    Err(error) => {
                        tracing::warn!(%error, lot = %event.lot_id, "replay could not create lot");
                    }
                }
                // A replay must also restore the release schedule, or the
                // engine would re-release the lots it just rebuilt.
                if let Some(serial) = event
                    .lot_id
                    .strip_prefix("LOT-")
                    .and_then(|rest| rest.parse::<u64>().ok())
                {
                    self.state.next_lot_serial = self.state.next_lot_serial.max(serial + 1);
                    self.state.released_lots = self.state.released_lots.max(serial as usize);
                }
            }
            EventData::LotQueued { step_no } => {
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id) {
                    let result = if aggregate.lot.hold_reason.is_some() {
                        aggregate.advance_while_held(*step_no, &mut ids, now, actor)
                    } else {
                        aggregate.queue(*step_no, &mut ids, now, actor)
                    };
                    if let Err(error) = result {
                        tracing::warn!(%error, lot = %event.lot_id, "replay could not queue lot");
                    }
                }
            }
            EventData::RecipeVerified {
                recipe_id,
                checksum,
            } => {
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id) {
                    aggregate.verify_recipe(recipe_id, checksum, &mut ids, now, actor);
                }
            }
            EventData::TrackIn {
                step_no,
                equipment_id,
                recipe_id,
                wafers,
            } => {
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id) {
                    let result = aggregate.track_in(
                        TrackInReport {
                            step_no: *step_no,
                            equipment_id,
                            recipe_id,
                            wafers,
                        },
                        &mut ids,
                        now,
                        actor,
                    );
                    if let Err(error) = result {
                        tracing::warn!(%error, lot = %event.lot_id, "replay could not track in");
                    }
                }
            }
            EventData::TrackOut {
                step_no,
                equipment_id,
                recipe_id,
                recipe_checksum,
                wafers,
                duration_ms,
                measured,
            } => {
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id) {
                    let result = aggregate.track_out(
                        TrackOutReport {
                            step_no: *step_no,
                            equipment_id,
                            recipe_id,
                            recipe_checksum,
                            wafers,
                            duration_ms: *duration_ms,
                            measured: *measured,
                        },
                        &mut ids,
                        now,
                        actor,
                    );
                    if let Err(error) = result {
                        tracing::warn!(%error, lot = %event.lot_id, "replay could not track out");
                    }
                }
            }
            EventData::LotCompleted { step_no } => {
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id)
                    && !aggregate.lot.state.is_terminal()
                {
                    // A scrap of the last wafer already completed the lot in
                    // the same batch; the log carries both events.
                    if let Err(error) = aggregate.complete(*step_no, &mut ids, now, actor) {
                        tracing::warn!(%error, lot = %event.lot_id, "replay could not complete lot");
                    }
                }
            }
            EventData::LotHeld { reason, .. } => {
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id)
                    && let Err(error) = aggregate.hold(reason, &mut ids, now, actor)
                {
                    tracing::warn!(%error, lot = %event.lot_id, "replay could not hold lot");
                }
            }
            EventData::LotReleased { .. } => {
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id)
                    && let Err(error) = aggregate.release(&mut ids, now, actor)
                {
                    tracing::warn!(%error, lot = %event.lot_id, "replay could not release lot");
                }
            }
            EventData::ReworkStarted {
                to_step, reason, ..
            } => {
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id)
                    && let Err(error) = aggregate.rework(*to_step, reason, &mut ids, now, actor)
                {
                    tracing::warn!(%error, lot = %event.lot_id, "replay could not rework lot");
                }
            }
            EventData::WaferScrapped {
                wafer_id, reason, ..
            } => {
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id)
                    && let Err(error) =
                        aggregate.scrap_wafer(wafer_id, reason, &mut ids, now, actor)
                {
                    tracing::warn!(%error, lot = %event.lot_id, "replay could not scrap wafer");
                }
            }
            EventData::LotSplit { children, .. } => {
                let child_ids: Vec<LotId> =
                    children.iter().map(|child| child.lot_id.clone()).collect();
                let split = self
                    .state
                    .lots
                    .get_mut(&event.lot_id)
                    .map(|aggregate| aggregate.split(&child_ids, &mut ids, now, actor));
                match split {
                    Some(Ok(transition)) => {
                        for child in transition.children {
                            self.state.lots.insert(child.lot.id.clone(), child);
                        }
                    }
                    Some(Err(error)) => {
                        tracing::warn!(%error, lot = %event.lot_id, "replay could not split lot");
                    }
                    None => {}
                }
            }
            EventData::LotMerged { target_lot_id, .. } => {
                let Some(mut source) = self.state.lots.remove(&event.lot_id) else {
                    return;
                };
                let merged = self.state.lots.get_mut(target_lot_id).map(|target| {
                    target.merge(std::slice::from_mut(&mut source), &mut ids, now, actor)
                });
                match merged {
                    Some(Ok(_)) => {}
                    Some(Err(error)) => {
                        tracing::warn!(%error, source = %event.lot_id, "replay could not merge lot");
                    }
                    None => {
                        tracing::warn!(target = %target_lot_id, "replay merge target is missing");
                    }
                }
                self.state.lots.insert(source.lot.id.clone(), source);
            }
            // A retry changes nothing by definition, and tool state is
            // runtime: neither has a command to replay.
            EventData::TrackOutDuplicate { .. }
            | EventData::EquipmentDown { .. }
            | EventData::EquipmentUp { .. } => {}
        }
    }

    /// Apply the runtime consequences of a committed event.
    ///
    /// Live commits and replays both call this, so queues and tool state have
    /// one implementation. Scheduling and the dispatch log stay with the live
    /// path: they are views of a run, not state a snapshot restores.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per event kind; splitting scatters the runtime projection"
    )]
    fn note_event(&mut self, event: &DomainEvent) {
        match &event.data {
            EventData::LotQueued { step_no } => {
                let held = self
                    .state
                    .lots
                    .get(&event.lot_id)
                    .is_some_and(|aggregate| aggregate.lot.hold_reason.is_some());
                if !held {
                    self.enqueue_lot(&event.lot_id, *step_no);
                }
            }
            EventData::TrackIn {
                step_no,
                equipment_id,
                recipe_id,
                wafers,
            } => {
                self.remove_from_queues(&event.lot_id);
                if let Some(runtime) = self.state.equipment.get_mut(equipment_id) {
                    let duration = self.route.step(*step_no).map_or(0, |step| step.duration_ms);
                    runtime.equipment.state = EquipmentState::Running;
                    runtime.busy_lot = Some(event.lot_id.clone());
                    runtime.busy_step = *step_no;
                    runtime.busy_recipe.clone_from(recipe_id);
                    runtime.busy_wafers.clone_from(wafers);
                    runtime.busy_until_ms = event.occurred_at_ms + duration;
                }
            }
            EventData::TrackOut { equipment_id, .. } => {
                if let Some(runtime) = self.state.equipment.get_mut(equipment_id) {
                    runtime.equipment.state = EquipmentState::Idle;
                    runtime.busy_lot = None;
                    runtime.busy_wafers.clear();
                    runtime.busy_until_ms = 0;
                    runtime.lots_processed += 1;
                }
            }
            EventData::LotHeld { .. } | EventData::LotCompleted { .. } => {
                self.remove_from_queues(&event.lot_id);
            }
            EventData::LotReleased { .. } => {
                let Some(aggregate) = self.state.lots.get(&event.lot_id) else {
                    return;
                };
                if !aggregate
                    .wafers
                    .iter()
                    .any(|wafer| wafer.state == WaferState::Processing)
                {
                    self.enqueue_lot(&event.lot_id, aggregate.lot.current_step);
                }
            }
            EventData::ReworkStarted { to_step, .. } => {
                self.remove_from_queues(&event.lot_id);
                let held = self
                    .state
                    .lots
                    .get(&event.lot_id)
                    .is_some_and(|aggregate| aggregate.lot.hold_reason.is_some());
                if !held {
                    self.enqueue_lot(&event.lot_id, *to_step);
                }
            }
            EventData::LotSplit { .. } => {
                self.remove_from_queues(&event.lot_id);
                let children: Vec<(String, i32)> = self
                    .state
                    .lots
                    .values()
                    .filter(|aggregate| {
                        aggregate.lot.parent_lot_id.as_deref() == Some(event.lot_id.as_str())
                    })
                    .map(|aggregate| (aggregate.lot.id.clone(), aggregate.lot.current_step))
                    .collect();
                for (child_id, step_no) in children {
                    self.enqueue_lot(&child_id, step_no);
                }
            }
            EventData::LotMerged { target_lot_id, .. } => {
                self.remove_from_queues(&event.lot_id);
                if let Some(step_no) = self
                    .state
                    .lots
                    .get(target_lot_id)
                    .map(|aggregate| aggregate.lot.current_step)
                {
                    self.remove_from_queues(target_lot_id);
                    self.enqueue_lot(target_lot_id, step_no);
                }
            }
            EventData::EquipmentDown { equipment_id, .. } => {
                if let Some(runtime) = self.state.equipment.get_mut(equipment_id) {
                    runtime.equipment.state = EquipmentState::Down;
                    runtime.down_since_ms = event.occurred_at_ms;
                    runtime.down_until_ms = event.occurred_at_ms + self.scenario.repair_ms;
                }
            }
            EventData::EquipmentUp { equipment_id, .. } => {
                if let Some(runtime) = self.state.equipment.get_mut(equipment_id) {
                    runtime.equipment.state = EquipmentState::Idle;
                    runtime.down_since_ms = 0;
                    runtime.down_until_ms = 0;
                }
            }
            _ => {}
        }
    }

    fn enqueue_lot(&mut self, lot_id: &str, step_no: i32) {
        let queue = self.state.queues.entry(step_no).or_default();
        if !queue.iter().any(|id| id == lot_id) {
            queue.push_back(lot_id.to_owned());
        }
    }

    // ------------------------------------------------------------------
    // internals
    // ------------------------------------------------------------------

    fn release_due_lots(&mut self, output: &mut TickOutput) {
        while self.state.released_lots < self.scenario.lot_count
            && self.state.next_release_ms <= self.state.now_ms
        {
            let serial = self.state.next_lot_serial;
            self.state.next_lot_serial += 1;
            self.state.released_lots += 1;
            let lot_id = lot_id_for(serial);
            let wafers: Vec<WaferId> = (1..=self.scenario.wafers_per_lot)
                .map(|slot| format!("{lot_id}-W{slot:02}"))
                .collect();
            let priority = if serial.is_multiple_of(3) { 9 } else { 5 };
            let lot = Lot::new(
                &lot_id,
                &self.scenario.product.id,
                &self.route.id,
                priority,
                wafers.len() as i32,
                self.state.now_ms,
            );
            let mut ids = self.ids_base();
            let transition = LotAggregate::create(lot, wafers, &mut ids, self.state.now_ms, "mes");
            self.advance_counter(ids.used());
            match transition {
                Ok(transition) => {
                    output.events.extend(transition.events);
                    if let Some(aggregate) = transition.children.into_iter().next() {
                        self.state.lots.insert(lot_id.clone(), aggregate);
                    }
                    let first = self.route.first_step();
                    let mut ids = self.ids_base();
                    let mut queued_events = Vec::new();
                    {
                        if let Some(aggregate) = self.state.lots.get_mut(&lot_id)
                            && let Ok(queued) =
                                aggregate.queue(first, &mut ids, self.state.now_ms, "mes")
                        {
                            queued_events = queued.events;
                        }
                    }
                    self.advance_counter(ids.used());
                    for event in &queued_events {
                        self.note_event(event);
                    }
                    output.events.extend(queued_events);
                }
                Err(error) => {
                    tracing::warn!(%error, "could not create lot");
                }
            }
            self.state.next_release_ms += self.scenario.interarrival_ms;
        }
    }

    fn schedule_release(&mut self) {
        if self.state.released_lots < self.scenario.lot_count {
            self.schedule(self.state.next_release_ms, ScheduledKind::ReleaseLot);
        }
    }

    fn handle_scheduled(&mut self, scheduled: Scheduled, output: &mut TickOutput) {
        match scheduled.kind {
            ScheduledKind::ReleaseLot => {
                // Releases are handled by `release_due_lots` at tick start.
            }
            ScheduledKind::StepComplete {
                equipment_id,
                lot_id,
                step_no,
                wafers,
            } => {
                self.complete_step(&equipment_id, &lot_id, step_no, &wafers, output);
            }
            ScheduledKind::Repair { equipment_id } => {
                self.repair(&equipment_id, output);
            }
        }
    }

    fn complete_step(
        &mut self,
        equipment_id: &str,
        lot_id: &str,
        step_no: i32,
        wafers: &[WaferId],
        output: &mut TickOutput,
    ) {
        // The physical step finished. Free the tool first: a refused transition
        // must not leave a busy tool whose timer has already expired.
        self.release_equipment(equipment_id);
        let Some(step) = self.route.step(step_no).cloned() else {
            return;
        };
        let recipe = self.scenario.recipe(&step.recipe_id);
        let (checksum, duration_ms) = match &recipe {
            Some(recipe) => (recipe.checksum.clone(), step.duration_ms),
            None => (String::new(), step.duration_ms),
        };
        let measured = if step.is_metrology() {
            let noise = (self.next_f64() - 0.5) * 2.0;
            Some(step.target.unwrap_or(100.0) + noise * 1.4)
        } else {
            None
        };
        // A wafer scrapped while the tool ran is not part of the report: it
        // left the lot, and naming it would make the whole report invalid.
        let active: Vec<WaferId> = match self.state.lots.get(lot_id) {
            Some(aggregate) => wafers
                .iter()
                .filter(|id| aggregate.has_active_wafer(id))
                .cloned()
                .collect(),
            None => return,
        };
        if active.is_empty() {
            return;
        }
        let mut ids = self.ids_base();
        let Some(aggregate) = self.state.lots.get_mut(lot_id) else {
            return;
        };
        let transition = aggregate.track_out(
            TrackOutReport {
                step_no,
                equipment_id,
                recipe_id: &step.recipe_id,
                recipe_checksum: &checksum,
                wafers: &active,
                duration_ms,
                measured,
            },
            &mut ids,
            self.state.now_ms,
            "equipment",
        );
        match transition {
            Ok(transition) => {
                self.advance_counter(ids.used());
                if let Some(event) = transition.events.first()
                    && event.kind() == EventKind::TrackOut
                {
                    output.secs.push(SecsTrigger::ProcessCompleted {
                        lot_id: lot_id.to_owned(),
                        step_no,
                        equipment_id: equipment_id.to_owned(),
                    });
                }
                for event in &transition.events {
                    self.note_event(event);
                }
                output.events.extend(transition.events);
            }
            Err(error) => {
                tracing::warn!(%error, lot = lot_id, step = step_no, "track-out refused");
                return;
            }
        }
        let held = self
            .state
            .lots
            .get(lot_id)
            .is_some_and(|aggregate| aggregate.lot.hold_reason.is_some());
        let all_done = self
            .state
            .lots
            .get(lot_id)
            .is_some_and(|aggregate| aggregate.step_finished(step_no));
        if held {
            // The hold survives the step: record where the material now stands
            // and leave the lot out of the queues until an operator releases it.
            if all_done {
                match self.route.next_step(step_no) {
                    Some(next) => self.advance_while_held(lot_id, next, output),
                    None => self.finish_lot(lot_id, step_no, output),
                }
            }
            return;
        }
        if !all_done {
            self.enqueue_at(lot_id, step_no, output);
            return;
        }
        match self.route.next_step(step_no) {
            Some(next) => self.enqueue_at(lot_id, next, output),
            None => self.finish_lot(lot_id, step_no, output),
        }
    }

    /// A tool whose step finished is idle again, whatever the log does with it.
    fn release_equipment(&mut self, equipment_id: &str) {
        if let Some(runtime) = self.state.equipment.get_mut(equipment_id) {
            runtime.equipment.state = EquipmentState::Idle;
            runtime.busy_lot = None;
            runtime.busy_wafers.clear();
            runtime.busy_until_ms = 0;
        }
    }

    /// Record the next step of a held lot without queuing it.
    fn advance_while_held(&mut self, lot_id: &str, step_no: i32, output: &mut TickOutput) {
        let now_ms = self.state.now_ms;
        let mut ids = self.ids_base();
        let advanced = self
            .state
            .lots
            .get_mut(lot_id)
            .map(|aggregate| aggregate.advance_while_held(step_no, &mut ids, now_ms, "mes"));
        match advanced {
            Some(Ok(transition)) => {
                self.advance_counter(ids.used());
                for event in &transition.events {
                    self.note_event(event);
                }
                output.events.extend(transition.events);
            }
            Some(Err(error)) => {
                tracing::warn!(%error, lot = lot_id, step = step_no, "held advance refused");
            }
            None => {}
        }
    }

    /// Every step is done: complete the lot.
    fn finish_lot(&mut self, lot_id: &str, step_no: i32, output: &mut TickOutput) {
        let mut ids = self.ids_base();
        let completed = {
            let Some(aggregate) = self.state.lots.get_mut(lot_id) else {
                return;
            };
            match aggregate.complete(step_no, &mut ids, self.state.now_ms, "mes") {
                Ok(transition) => Some(transition),
                Err(error) => {
                    tracing::warn!(%error, lot = lot_id, "completion refused");
                    None
                }
            }
        };
        if let Some(transition) = completed {
            self.advance_counter(ids.used());
            for event in &transition.events {
                self.note_event(event);
            }
            output.events.extend(transition.events);
        }
    }

    fn enqueue_at(&mut self, lot_id: &str, step_no: i32, output: &mut TickOutput) {
        let mut ids = self.ids_base();
        let mut events = Vec::new();
        let mut queued = false;
        {
            let Some(aggregate) = self.state.lots.get_mut(lot_id) else {
                return;
            };
            match aggregate.queue(step_no, &mut ids, self.state.now_ms, "mes") {
                Ok(transition) => {
                    events = transition.events;
                    queued = true;
                }
                Err(error) => {
                    tracing::warn!(%error, lot = lot_id, step = step_no, "queue refused");
                }
            }
        }
        if queued {
            self.advance_counter(ids.used());
            for event in &events {
                self.note_event(event);
            }
            output.events.extend(events);
        }
    }

    fn repair(&mut self, equipment_id: &str, output: &mut TickOutput) {
        let event_id = self.take_id();
        let now_ms = self.state.now_ms;
        let Some(runtime) = self.state.equipment.get_mut(equipment_id) else {
            return;
        };
        if runtime.equipment.state != EquipmentState::Down {
            return;
        }
        let down_ms = (now_ms - runtime.down_since_ms).max(0);
        let kind = runtime.equipment.kind.clone();
        let event = equipment_up(
            equipment_id,
            &kind,
            down_ms,
            event_id,
            now_ms,
            "maintenance",
        );
        self.note_event(&event);
        output.events.push(event);
    }

    /// Hand-written dispatch: priority first, then FIFO, then a qualified
    /// idle tool; the recipe must be checksum-verified before loading.
    fn dispatch(&mut self, output: &mut TickOutput) {
        let steps: Vec<i32> = self.route.steps.iter().map(|s| s.step_no).collect();
        for step_no in steps {
            let Some(step) = self.route.step(step_no).cloned() else {
                continue;
            };
            while let Some(lot_id) = self.pick_lot(step_no) {
                if self.load_lot(&step, step_no, &lot_id, output).is_break() {
                    break;
                }
            }
        }
    }

    /// Try to load the next queued lot on a tool.
    ///
    /// `Continue` means the step's queue should be looked at again: either the
    /// lot was loaded, or it was skipped for a reason another lot may not hit.
    /// `Break` means this step cannot dispatch anything right now — no idle
    /// qualified tool, or a recipe the fab cannot verify.
    #[expect(
        clippy::too_many_lines,
        reason = "one load: pick a tool, verify the recipe, track in, record the decision, schedule the finish"
    )]
    fn load_lot(
        &mut self,
        step: &oxide_core::model::RouteStep,
        step_no: i32,
        lot_id: &str,
        output: &mut TickOutput,
    ) -> ControlFlow<()> {
        let Some(equipment_id) = self.pick_equipment(step) else {
            return ControlFlow::Break(());
        };
        // A tool that fails while loading goes down before any material
        // moves; the lot stays queued for the next tool.
        if self.roll_failure() {
            self.take_tool_down(&equipment_id, output);
            return ControlFlow::Continue(());
        }
        if !self.ensure_recipe(lot_id, step, output) {
            return ControlFlow::Break(());
        }
        let wanted: Vec<WaferId> = self.state.lots.get(lot_id).map_or_default(|aggregate| {
            aggregate
                .wafers
                .iter()
                .filter(|w| w.state == WaferState::Queued)
                .map(|w| w.id.clone())
                .collect()
        });
        if wanted.is_empty() {
            self.dequeue(step_no, lot_id);
            return ControlFlow::Continue(());
        }
        let recipe_checksum = self
            .scenario
            .recipe(&step.recipe_id)
            .map_or_default(|recipe| recipe.checksum);
        let mut ids = self.ids_base();
        let tracked = {
            let Some(aggregate) = self.state.lots.get_mut(lot_id) else {
                return ControlFlow::Break(());
            };
            aggregate
                .verified_recipes
                .insert(step.recipe_id.clone(), recipe_checksum);
            aggregate.track_in(
                TrackInReport {
                    step_no,
                    equipment_id: &equipment_id,
                    recipe_id: &step.recipe_id,
                    wafers: &wanted,
                },
                &mut ids,
                self.state.now_ms,
                "dispatcher",
            )
        };
        let transition = match tracked {
            Ok(transition) => transition,
            Err(error) => {
                tracing::warn!(%error, lot = lot_id, step = step_no, "track-in refused");
                self.dequeue(step_no, lot_id);
                return ControlFlow::Continue(());
            }
        };
        self.advance_counter(ids.used());
        output.secs.push(SecsTrigger::ProcessStarted {
            lot_id: lot_id.to_owned(),
            step_no,
            equipment_id: equipment_id.clone(),
        });
        let (queue_position, queue_len) = self.state.queues.get(&step_no).map_or((0, 0), |queue| {
            (
                queue
                    .iter()
                    .position(|queued| queued == lot_id)
                    .unwrap_or(0),
                queue.len(),
            )
        });
        self.state.dispatch.push(DispatchDecision {
            at_ms: self.state.now_ms,
            lot_id: lot_id.to_owned(),
            step_no,
            equipment_id: equipment_id.clone(),
            priority: self
                .state
                .lots
                .get(lot_id)
                .map_or(0, |aggregate| aggregate.lot.priority),
            queue_position,
            queue_len,
            qualified_tools: step.equipment_qualification.clone(),
            recipe_id: step.recipe_id.clone(),
            rule: self.rule.label().to_owned(),
        });
        if self.state.dispatch.len() > 64 {
            self.state.dispatch.remove(0);
        }
        for event in &transition.events {
            self.note_event(event);
        }
        output.events.extend(transition.events);
        self.schedule(
            self.state.now_ms + step.duration_ms,
            ScheduledKind::StepComplete {
                equipment_id,
                lot_id: lot_id.to_owned(),
                step_no,
                wafers: wanted,
            },
        );
        ControlFlow::Continue(())
    }

    /// Highest priority first, FIFO within a priority, skipping held, busy and
    /// fully processed lots. Under [`DispatchRule::Fifo`] the head of the
    /// queue wins whatever its priority.
    fn pick_lot(&self, step_no: i32) -> Option<String> {
        let queue = self.state.queues.get(&step_no)?;
        let eligible = |lot_id: &String| {
            let aggregate = self.state.lots.get(lot_id)?;
            if aggregate.lot.state != LotState::Queued && aggregate.lot.state != LotState::Waiting {
                return None;
            }
            if aggregate.lot.current_step != step_no {
                return None;
            }
            if aggregate
                .wafers
                .iter()
                .any(|w| w.state == WaferState::Processing)
            {
                return None;
            }
            if aggregate.lot.hold_reason.is_some() {
                return None;
            }
            if !aggregate
                .wafers
                .iter()
                .any(|w| w.state == WaferState::Queued)
            {
                return None;
            }
            Some(aggregate.lot.priority)
        };
        match self.rule {
            DispatchRule::Fifo => queue
                .iter()
                .find(|lot_id| eligible(lot_id).is_some())
                .cloned(),
            DispatchRule::PriorityThenFifo => {
                let mut best: Option<(i32, &String)> = None;
                for lot_id in queue {
                    let Some(priority) = eligible(lot_id) else {
                        continue;
                    };
                    // Strictly greater: the first lot of the highest priority
                    // wins, which keeps FIFO order within a priority class.
                    if best.as_ref().is_none_or(|(best, _)| priority > *best) {
                        best = Some((priority, lot_id));
                    }
                }
                best.map(|(_, lot_id)| lot_id.clone())
            }
        }
    }

    fn pick_equipment(&self, step: &oxide_core::model::RouteStep) -> Option<String> {
        self.state
            .equipment
            .values()
            .filter(|runtime| runtime.equipment.kind == step.equipment_kind)
            .filter(|runtime| runtime.equipment.is_available())
            .filter(|runtime| runtime.equipment.qualified_for(&step.recipe_id))
            .filter(|runtime| {
                step.equipment_qualification
                    .iter()
                    .any(|id| id == &runtime.equipment.id)
            })
            .map(|runtime| runtime.equipment.id.clone())
            .min()
    }

    fn ensure_recipe(
        &mut self,
        lot_id: &str,
        step: &oxide_core::model::RouteStep,
        output: &mut TickOutput,
    ) -> bool {
        let Some(recipe) = self.scenario.recipe(&step.recipe_id) else {
            return false;
        };
        let already = self
            .state
            .lots
            .get(lot_id)
            .is_some_and(|aggregate| aggregate.verified_recipes.contains_key(&step.recipe_id));
        if already {
            return true;
        }
        let mut ids = self.ids_base();
        let events;
        {
            let Some(aggregate) = self.state.lots.get_mut(lot_id) else {
                return false;
            };
            let transition = aggregate.verify_recipe(
                &recipe.id,
                &recipe.checksum,
                &mut ids,
                self.state.now_ms,
                "eap",
            );
            events = transition.events;
        }
        self.advance_counter(ids.used());
        output.events.extend(events);
        output.secs.push(SecsTrigger::RecipeUpload {
            recipe_id: recipe.id.clone(),
            checksum: recipe.checksum.clone(),
            body: recipe.body.clone(),
        });
        true
    }

    fn take_tool_down(&mut self, equipment_id: &str, output: &mut TickOutput) {
        let event_id = self.take_id();
        let now_ms = self.state.now_ms;
        let repair_ms = self.scenario.repair_ms;
        let Some(runtime) = self.state.equipment.get_mut(equipment_id) else {
            return;
        };
        let kind = runtime.equipment.kind.clone();
        let event = equipment_down(
            equipment_id,
            &kind,
            "simulated chamber fault",
            2001,
            event_id,
            now_ms,
            "equipment",
        );
        self.note_event(&event);
        output.events.push(event);
        output.secs.push(SecsTrigger::Alarm {
            equipment_id: equipment_id.to_owned(),
            alid: 2001,
            text: format!("simulated chamber fault on {kind}"),
        });
        self.schedule(
            now_ms + repair_ms,
            ScheduledKind::Repair {
                equipment_id: equipment_id.to_owned(),
            },
        );
    }

    fn dequeue(&mut self, step_no: i32, lot_id: &str) {
        if let Some(queue) = self.state.queues.get_mut(&step_no)
            && let Some(position) = queue.iter().position(|id| id == lot_id)
        {
            queue.remove(position);
        }
    }

    fn schedule(&mut self, at_ms: i64, kind: ScheduledKind) {
        self.schedule_seq += 1;
        self.heap.push(Reverse(Scheduled {
            at_ms,
            seq: self.schedule_seq,
            kind,
        }));
    }

    /// Event ids for one transition, without borrowing the state.
    ///
    /// The counter is shared with [`EventIds::used`]: a transition that hands
    /// out `n` ids advances the counter by `n`, so ids never repeat.
    fn ids_base(&self) -> EventIds {
        EventIds::new(deterministic_event_id(
            self.state.seed,
            self.state.event_counter,
        ))
    }

    fn advance_counter(&mut self, used: u32) {
        self.state.event_counter += used as u64;
    }

    fn take_id(&mut self) -> uuid::Uuid {
        let id = deterministic_event_id(self.state.seed, self.state.event_counter);
        self.state.event_counter += 1;
        id
    }

    /// xorshift64*: deterministic, dependency-free.
    fn next_u64(&mut self) -> u64 {
        let mut x = self.state.rng.max(1);
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.state.rng = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn roll_failure(&mut self) -> bool {
        if self.scenario.failure_probability <= 0.0 {
            return false;
        }
        self.next_f64() < self.scenario.failure_probability
    }
}

/// One step as the dashboard shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepView {
    pub step_no: i32,
    pub name: String,
    pub equipment_kind: String,
    pub queue_len: usize,
    pub busy_tools: usize,
    pub total_tools: usize,
    pub current_lots: Vec<String>,
}

/// One tool as the dashboard shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EquipmentView {
    pub id: String,
    pub kind: String,
    pub chamber: String,
    pub state: String,
    pub light: String,
    pub current_lot: Option<String>,
    pub current_step: Option<i32>,
    pub busy_until_ms: i64,
    pub lots_processed: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaferView {
    pub id: String,
    pub slot: i32,
    pub state: String,
    pub defect_info: Option<String>,
    pub rework_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LotView {
    pub id: String,
    pub product_id: String,
    pub state: String,
    pub current_step: i32,
    pub priority: i32,
    pub wafer_count: usize,
    pub done_wafers: usize,
    pub hold_reason: Option<String>,
    pub parent_lot_id: Option<String>,
    pub wafers: Vec<WaferView>,
}

/// The whole floor, for the dashboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimSnapshot {
    pub now_ms: i64,
    pub running: bool,
    pub speed: f64,
    pub released_lots: usize,
    pub total_lots: usize,
    pub steps: Vec<StepView>,
    pub equipment: Vec<EquipmentView>,
    pub lots: Vec<LotView>,
    pub persisted_events: u64,
    /// Newest first, at most 64.
    pub dispatch: Vec<DispatchDecision>,
}

impl Simulation {
    #[must_use]
    pub fn view(&self) -> SimSnapshot {
        let steps = self
            .route
            .steps
            .iter()
            .map(|step| {
                let queue = self.state.queues.get(&step.step_no);
                let total_tools = self
                    .state
                    .equipment
                    .values()
                    .filter(|r| r.equipment.kind == step.equipment_kind)
                    .count();
                let busy_tools = self
                    .state
                    .equipment
                    .values()
                    .filter(|r| {
                        r.equipment.kind == step.equipment_kind
                            && r.equipment.state == EquipmentState::Running
                    })
                    .count();
                StepView {
                    step_no: step.step_no,
                    name: step.name.clone(),
                    equipment_kind: step.equipment_kind.clone(),
                    queue_len: queue.map_or(0, VecDeque::len),
                    busy_tools,
                    total_tools,
                    current_lots: queue.map_or_default(|q| q.iter().cloned().collect()),
                }
            })
            .collect();
        let equipment = self
            .state
            .equipment
            .values()
            .map(|runtime| EquipmentView {
                id: runtime.equipment.id.clone(),
                kind: runtime.equipment.kind.clone(),
                chamber: runtime.equipment.chamber.clone(),
                state: runtime.equipment.state.as_str().to_owned(),
                light: runtime.light().to_owned(),
                current_lot: runtime.busy_lot.clone(),
                current_step: if runtime.equipment.state == EquipmentState::Running {
                    Some(runtime.busy_step)
                } else {
                    None
                },
                busy_until_ms: runtime.busy_until_ms,
                lots_processed: runtime.lots_processed,
            })
            .collect();
        let lots = self
            .state
            .lots
            .values()
            .map(|aggregate| LotView {
                id: aggregate.lot.id.clone(),
                product_id: aggregate.lot.product_id.clone(),
                state: aggregate.lot.state.as_str().to_owned(),
                current_step: aggregate.lot.current_step,
                priority: aggregate.lot.priority,
                wafer_count: aggregate.wafers.len(),
                done_wafers: aggregate
                    .wafers
                    .iter()
                    .filter(|w| w.state == WaferState::Done)
                    .count(),
                hold_reason: aggregate.lot.hold_reason.clone(),
                parent_lot_id: aggregate.lot.parent_lot_id.clone(),
                wafers: aggregate
                    .wafers
                    .iter()
                    .map(|wafer| WaferView {
                        id: wafer.id.clone(),
                        slot: wafer.slot,
                        state: wafer.state.as_str().to_owned(),
                        defect_info: wafer.defect_info.clone(),
                        rework_count: wafer.rework_count,
                    })
                    .collect(),
            })
            .collect();
        SimSnapshot {
            now_ms: self.state.now_ms,
            running: self.state.running,
            speed: self.state.speed,
            released_lots: self.state.released_lots,
            total_lots: self.scenario.lot_count,
            steps,
            equipment,
            lots,
            persisted_events: self.state.persisted_events,
            dispatch: self.state.dispatch.iter().rev().cloned().collect(),
        }
    }

    /// Operator actions, used by the API and the dashboard.
    pub fn hold_lot(&mut self, lot_id: &str, reason: &str) -> Result<Vec<DomainEvent>, SimError> {
        let mut ids = self.ids_base();
        let now_ms = self.state.now_ms;
        let events = self
            .aggregate_mut(lot_id)?
            .hold(reason, &mut ids, now_ms, "operator")?
            .events;
        self.advance_counter(ids.used());
        for event in &events {
            self.note_event(event);
        }
        Ok(events)
    }

    pub fn release_lot(&mut self, lot_id: &str) -> Result<Vec<DomainEvent>, SimError> {
        let mut ids = self.ids_base();
        let now_ms = self.state.now_ms;
        let events = self
            .aggregate_mut(lot_id)?
            .release(&mut ids, now_ms, "operator")?
            .events;
        self.advance_counter(ids.used());
        for event in &events {
            self.note_event(event);
        }
        Ok(events)
    }

    pub fn rework_lot(
        &mut self,
        lot_id: &str,
        to_step: i32,
        reason: &str,
    ) -> Result<Vec<DomainEvent>, SimError> {
        let mut ids = self.ids_base();
        let now_ms = self.state.now_ms;
        let events = self
            .aggregate_mut(lot_id)?
            .rework(to_step, reason, &mut ids, now_ms, "engineer")?
            .events;
        self.advance_counter(ids.used());
        for event in &events {
            self.note_event(event);
        }
        Ok(events)
    }

    pub fn scrap_wafer(
        &mut self,
        lot_id: &str,
        wafer_id: &str,
        reason: &str,
    ) -> Result<Vec<DomainEvent>, SimError> {
        let mut ids = self.ids_base();
        let now_ms = self.state.now_ms;
        let events = self
            .aggregate_mut(lot_id)?
            .scrap_wafer(wafer_id, reason, &mut ids, now_ms, "engineer")?
            .events;
        self.advance_counter(ids.used());
        for event in &events {
            self.note_event(event);
        }
        Ok(events)
    }

    pub fn split_lot(
        &mut self,
        lot_id: &str,
        child_ids: &[String],
    ) -> Result<Vec<DomainEvent>, SimError> {
        let mut ids = self.ids_base();
        let now_ms = self.state.now_ms;
        let (events, children) = {
            let aggregate = self.aggregate_mut(lot_id)?;
            let transition = aggregate.split(child_ids, &mut ids, now_ms, "operator")?;
            (transition.events, transition.children)
        };
        self.advance_counter(ids.used());
        for child in children {
            self.state.lots.insert(child.lot.id.clone(), child);
        }
        for event in &events {
            self.note_event(event);
        }
        Ok(events)
    }

    pub fn merge_lots(
        &mut self,
        target_lot: &str,
        source_lots: &[String],
    ) -> Result<Vec<DomainEvent>, SimError> {
        if source_lots.iter().any(|id| id == target_lot) {
            return Err(SimError::MergingIntoSelf);
        }
        // Check every source before removing any, so a typo in the last id
        // cannot strand the lots that were already taken out of the map.
        if let Some(missing) = source_lots
            .iter()
            .find(|id| !self.state.lots.contains_key(*id))
        {
            return Err(SimError::UnknownLot(missing.clone()));
        }
        let mut sources: Vec<LotAggregate> = source_lots
            .iter()
            .filter_map(|id| self.state.lots.remove(id))
            .collect();

        let mut ids = self.ids_base();
        let now_ms = self.state.now_ms;
        let merged = {
            let Some(target) = self.state.lots.get_mut(target_lot) else {
                for source in sources {
                    self.state.lots.insert(source.lot.id.clone(), source);
                }
                return Err(SimError::UnknownLot(target_lot.to_owned()));
            };
            target.merge(&mut sources, &mut ids, now_ms, "operator")
        };
        self.advance_counter(ids.used());
        match merged {
            Err(error) => {
                // `merge` validates before it mutates, so the sources are
                // intact and can go back into their queues.
                for source in sources {
                    self.state.lots.insert(source.lot.id.clone(), source);
                }
                Err(SimError::Transition(error))
            }
            Ok(transition) => {
                for source in sources {
                    self.state.lots.insert(source.lot.id.clone(), source);
                }
                for event in &transition.events {
                    self.note_event(event);
                }
                Ok(transition.events)
            }
        }
    }

    /// The lot's aggregate, or the error the API reports as 404.
    fn aggregate_mut(&mut self, lot_id: &str) -> Result<&mut LotAggregate, SimError> {
        self.state
            .lots
            .get_mut(lot_id)
            .ok_or_else(|| SimError::UnknownLot(lot_id.to_owned()))
    }

    fn remove_from_queues(&mut self, lot_id: &str) {
        for queue in self.state.queues.values_mut() {
            if let Some(position) = queue.iter().position(|id| id == lot_id) {
                queue.remove(position);
            }
        }
    }
}

pub mod whatif;

#[cfg(test)]
mod tests {
    use super::*;

    fn scenario() -> Scenario {
        let mut scenario =
            Scenario::from_json(include_str!("../../../config/scenario.json")).unwrap();
        // Keep tests short and deterministic: no random failures.
        scenario.failure_probability = 0.0;
        scenario.lot_count = 2;
        scenario.wafers_per_lot = 4;
        scenario
    }

    #[test]
    fn a_lot_walks_all_four_steps_and_completes() {
        let mut sim = Simulation::new(scenario()).unwrap();
        let mut events = Vec::new();
        for _ in 0..5000 {
            let output = sim.tick(100);
            events.extend(output.events);
            if sim
                .state
                .lots
                .values()
                .all(|aggregate| aggregate.lot.state == LotState::Completed)
                && sim.state.lots.len() >= 2
            {
                break;
            }
        }
        assert!(sim.state.lots.len() >= 2, "both lots were released");
        let first = sim.state.lots.values().next().unwrap();
        assert_eq!(first.lot.state, LotState::Completed);
        assert_eq!(first.active_count(), 4);
        assert!(first.wafers.iter().all(|w| w.state == WaferState::Done));
        // Four steps: one track-in and one track-out each, per lot.
        let track_outs = events
            .iter()
            .filter(|e| e.kind() == EventKind::TrackOut)
            .count();
        assert!(
            track_outs >= 8,
            "expected 4 track-outs per lot, got {track_outs}"
        );
    }

    fn queued_sim(priorities: &[(u64, i32)]) -> Simulation {
        let mut scenario = scenario();
        scenario.lot_count = priorities.len();
        let mut sim = Simulation::new(scenario).unwrap();
        // Make all lots due at once.
        sim.state.next_release_ms = 0;
        sim.state.released_lots = priorities.len();
        sim.state.lots.clear();
        for (serial, priority) in priorities {
            let lot = Lot::new(lot_id_for(*serial), "PRD", &sim.route.id, *priority, 2, 0);
            let wafers: Vec<String> = (1..=2)
                .map(|slot| format!("LOT-{serial:04}-W{slot:02}"))
                .collect();
            let mut ids = sim.ids_base();
            let transition = LotAggregate::create(lot, wafers, &mut ids, 0, "test").unwrap();
            sim.state.event_counter += ids.used() as u64;
            let id = transition.children[0].lot.id.clone();
            sim.state
                .lots
                .insert(id.clone(), transition.children.into_iter().next().unwrap());
            let mut ids = sim.ids_base();
            sim.state
                .lots
                .get_mut(&id)
                .unwrap()
                .queue(1, &mut ids, 0, "test")
                .unwrap();
            sim.state.event_counter += ids.used() as u64;
            sim.state.queues.entry(1).or_default().push_back(id);
        }
        sim
    }

    #[test]
    fn dispatch_respects_priority() {
        let mut sim = queued_sim(&[(1, 1), (2, 9), (3, 5)]);
        let output = sim.tick(0);
        let running: Vec<String> = sim
            .state
            .equipment
            .values()
            .filter(|r| r.equipment.state == EquipmentState::Running)
            .filter_map(|r| r.busy_lot.clone())
            .collect();
        let loaded = output
            .events
            .iter()
            .find(|e| e.kind() == EventKind::TrackIn)
            .expect("a lot was loaded");
        assert_eq!(
            loaded.lot_id, "LOT-0002",
            "the highest priority lot goes first"
        );
        assert!(running.contains(&"LOT-0002".to_owned()));
        let decision = sim.state.dispatch.first().expect("a decision was recorded");
        assert_eq!(decision.lot_id, "LOT-0002");
        assert_eq!(decision.rule, DispatchRule::PriorityThenFifo.label());
        assert_eq!(decision.queue_len, 3);
        assert_eq!(decision.queue_position, 1);
    }

    #[test]
    fn the_fifo_rule_takes_the_queue_head_whatever_its_priority() {
        let mut sim = queued_sim(&[(1, 1), (2, 9), (3, 5)]).with_rule(DispatchRule::Fifo);
        let output = sim.tick(0);
        let loaded = output
            .events
            .iter()
            .find(|e| e.kind() == EventKind::TrackIn)
            .expect("a lot was loaded");
        assert_eq!(loaded.lot_id, "LOT-0001");
        assert_eq!(sim.state.dispatch[0].rule, DispatchRule::Fifo.label());
    }

    #[test]
    fn a_snapshot_plus_the_log_resumes_the_same_state() {
        let mut sim = Simulation::new(scenario()).unwrap();
        for _ in 0..40 {
            sim.tick(100);
        }
        let state = sim.snapshot();
        let mut restored = Simulation::from_state(scenario(), state.clone()).unwrap();
        restored.rebuild_heap();
        assert_eq!(restored.state.now_ms, state.now_ms);
        assert_eq!(restored.state.lots, state.lots);
        // The resumed engine keeps processing: no scheduled event was lost.
        for _ in 0..2000 {
            restored.tick(100);
        }
        assert!(
            restored
                .state
                .lots
                .values()
                .any(|aggregate| aggregate.lot.state == LotState::Completed)
        );
    }

    #[test]
    fn a_failed_append_can_put_the_state_back_without_moving_the_log_position() {
        let mut sim = Simulation::new(scenario()).unwrap();
        for _ in 0..20 {
            sim.tick(100);
        }
        sim.advance_seq(41);
        let committed = sim.snapshot();

        sim.tick(100);
        assert_ne!(sim.snapshot(), committed);

        sim.restore(committed.clone());
        assert_eq!(sim.snapshot(), committed);
        assert_eq!(sim.last_seq(), 41);
    }

    #[test]
    fn the_log_position_never_moves_back() {
        let mut sim = Simulation::new(scenario()).unwrap();
        sim.advance_seq(40);
        sim.advance_seq(12);
        assert_eq!(sim.last_seq(), 40);
    }

    #[test]
    fn a_disabled_tool_is_repaired_and_returns_to_idle() {
        let mut scenario = scenario();
        scenario.failure_probability = 1.0;
        scenario.repair_ms = 500;
        let mut sim = Simulation::new(scenario).unwrap();
        let output = sim.tick(100);
        assert!(
            output
                .events
                .iter()
                .any(|e| e.kind() == EventKind::EquipmentDown)
        );
        assert!(
            sim.state
                .equipment
                .values()
                .any(|r| r.equipment.state == EquipmentState::Down)
        );
        for _ in 0..20 {
            sim.tick(100);
        }
        assert!(
            sim.state
                .equipment
                .values()
                .any(|r| r.equipment.state == EquipmentState::Idle)
        );
    }

    #[test]
    fn replaying_the_log_restores_the_release_schedule() {
        let mut scenario = scenario();
        scenario.lot_count = 3;
        let mut sim = Simulation::new(scenario.clone()).unwrap();
        let mut events = Vec::new();
        for _ in 0..120 {
            events.extend(sim.tick(100).events);
        }
        let released = sim.state.released_lots;
        assert!(released >= 2, "several lots were released: {released}");

        // Rebuild from the log alone (as a restart does when the snapshot is
        // older than the events).
        let mut fresh = Simulation::new(scenario).unwrap();
        fresh.state.now_ms = sim.state.now_ms;
        for event in &events {
            fresh.apply_event(event);
        }
        assert_eq!(fresh.state.released_lots, released);
        assert_eq!(fresh.state.next_lot_serial, sim.state.next_lot_serial);
        fresh.rebuild_heap();
        let lots = fresh.state.lots.len();
        fresh.tick(100);
        assert_eq!(
            fresh.state.lots.len(),
            lots,
            "a replay must not release the same lots again"
        );
    }

    /// The live engine and a replay must rebuild identical lot aggregates and
    /// queues. This is the regression the split used to fail: the live path
    /// copied `verified_recipes` and `completed` into the children, the replay
    /// projection built them empty.
    #[test]
    fn replaying_the_log_after_a_split_rebuilds_the_same_lots() {
        let scenario = scenario();
        let mut sim = Simulation::new(scenario.clone()).unwrap();
        let mut ids = sim.ids_base();
        let lot = Lot::new("LOT-0001", "PRD-OXIDE-300", &sim.route.id, 5, 4, 0);
        let wafers: Vec<WaferId> = (1..=4).map(|slot| format!("LOT-0001-W{slot:02}")).collect();
        let created = LotAggregate::create(lot, wafers, &mut ids, 0, "mes").unwrap();
        let mut events = created.events;
        let mut aggregate = created.children.into_iter().next().unwrap();
        events.push(
            aggregate
                .verify_recipe("RCP-CLEAN", "checksum", &mut ids, 0, "mes")
                .events[0]
                .clone(),
        );
        events.extend(aggregate.queue(1, &mut ids, 0, "mes").unwrap().events);
        sim.state.lots.insert("LOT-0001".to_owned(), aggregate);
        let children = ["LOT-0001-A".to_owned(), "LOT-0001-B".to_owned()];
        events.extend(sim.split_lot("LOT-0001", &children).unwrap());

        let mut fresh = Simulation::new(scenario).unwrap();
        fresh.state.now_ms = sim.state.now_ms;
        for event in &events {
            fresh.apply_event(event);
        }
        assert_eq!(fresh.state.lots.len(), sim.state.lots.len());
        for (id, live) in &sim.state.lots {
            let replayed = fresh
                .state
                .lots
                .get(id)
                .unwrap_or_else(|| panic!("replay has no lot {id}"));
            assert_eq!(replayed.lot, live.lot, "lot {id}");
            assert_eq!(replayed.wafers, live.wafers, "lot {id}");
            assert_eq!(
                replayed.verified_recipes, live.verified_recipes,
                "verified recipes of lot {id}"
            );
            assert_eq!(
                replayed.completed, live.completed,
                "completed steps of lot {id}"
            );
        }
        assert_eq!(fresh.state.queues, sim.state.queues, "queues");
    }

    /// A whole run and its replay must agree on every piece of runtime state
    /// the log claims to describe.
    #[test]
    fn a_live_run_and_its_replay_agree_on_lots_queues_and_tools() {
        let mut scenario = scenario();
        scenario.lot_count = 2;
        let mut sim = Simulation::new(scenario.clone()).unwrap();
        let mut events = Vec::new();
        for _ in 0..160 {
            events.extend(sim.tick(100).events);
        }
        assert!(!events.is_empty(), "the run produced events");

        let mut fresh = Simulation::new(scenario).unwrap();
        fresh.state.now_ms = sim.state.now_ms;
        for event in &events {
            fresh.apply_event(event);
        }
        assert_eq!(fresh.state.lots, sim.state.lots, "lots");
        assert_eq!(fresh.state.queues, sim.state.queues, "queues");
        assert_eq!(fresh.state.equipment, sim.state.equipment, "equipment");
    }

    /// An unknown source id must not consume the sources that were already
    /// taken out of the map.
    #[test]
    fn merging_with_an_unknown_source_leaves_every_lot_in_place() {
        let mut sim = Simulation::new(scenario()).unwrap();
        for _ in 0..200 {
            sim.tick(100);
        }
        let ids: Vec<String> = sim.state.lots.keys().cloned().collect();
        assert!(ids.len() >= 2, "several lots were released: {ids:?}");
        let before = sim.state.lots.len();

        let error = sim
            .merge_lots(&ids[0], &[ids[1].clone(), "LOT-9999".to_owned()])
            .unwrap_err();

        assert!(matches!(error, SimError::UnknownLot(id) if id == "LOT-9999"));
        assert_eq!(sim.state.lots.len(), before, "no source was removed");
    }

    #[test]
    fn operator_hold_stops_dispatch_until_release() {
        let mut sim = Simulation::new(scenario()).unwrap();
        sim.tick(100);
        let lot_id = sim.state.lots.keys().next().unwrap().clone();
        sim.hold_lot(&lot_id, "engineer review").unwrap();
        assert_eq!(sim.state.lots[&lot_id].lot.state, LotState::OnHold);
        for _ in 0..40 {
            sim.tick(100);
        }
        assert_eq!(sim.state.lots[&lot_id].lot.state, LotState::OnHold);
        sim.release_lot(&lot_id).unwrap();
        for _ in 0..400 {
            sim.tick(100);
        }
        assert!(matches!(
            sim.state.lots[&lot_id].lot.state,
            LotState::Completed | LotState::Processing | LotState::Waiting | LotState::Queued
        ));
    }

    /// A tool that is mid-step, with the lot and one of its wafers.
    fn find_running(sim: &Simulation) -> Option<(String, String, i64, String)> {
        sim.state.equipment.iter().find_map(|(id, runtime)| {
            if runtime.equipment.state != EquipmentState::Running {
                return None;
            }
            let lot = runtime.busy_lot.clone()?;
            let wafer = runtime.busy_wafers.first().cloned()?;
            Some((id.clone(), lot, runtime.busy_until_ms, wafer))
        })
    }

    #[test]
    fn scrapping_a_wafer_mid_step_does_not_freeze_the_tool() {
        let mut sim = Simulation::new(scenario()).unwrap();
        let mut events = Vec::new();
        let mut busy = None;
        for _ in 0..200 {
            events.extend(sim.tick(100).events);
            if let Some(found) = find_running(&sim) {
                busy = Some(found);
                break;
            }
        }
        let (equipment_id, lot_id, busy_until, wafer) = busy.expect("a tool loaded a lot");
        let step = sim.state.lots[&lot_id].lot.current_step;
        sim.scrap_wafer(&lot_id, &wafer, "test scrap").unwrap();
        while sim.state.now_ms <= busy_until {
            events.extend(sim.tick(100).events);
        }

        let runtime = &sim.state.equipment[&equipment_id];
        if runtime.equipment.state == EquipmentState::Running {
            assert!(
                runtime.busy_until_ms > sim.state.now_ms,
                "{equipment_id} kept an expired step"
            );
        }
        assert!(runtime.lots_processed >= 1, "the finished step was counted");
        assert!(
            events
                .iter()
                .any(|event| event.kind() == EventKind::TrackOut && event.lot_id == lot_id),
            "the step still tracked out"
        );
        let aggregate = &sim.state.lots[&lot_id];
        assert!(
            aggregate.lot.current_step > step || aggregate.lot.state == LotState::Completed,
            "the lot moved on: {:?} at step {}",
            aggregate.lot.state,
            aggregate.lot.current_step
        );
    }

    #[test]
    fn a_single_step_advances_the_paused_line() {
        let mut sim = Simulation::new(scenario()).unwrap();
        sim.set_running(false);
        let before = sim.state.now_ms;
        assert!(sim.step().events.is_empty(), "a paused tick does nothing");
        let output = sim.force_step();
        assert!(sim.state.now_ms > before, "the clock advanced");
        assert!(!sim.state.running, "the line is still paused");
        assert!(
            !output.events.is_empty(),
            "the released lot was written to the log"
        );
    }

    #[test]
    fn a_hold_during_processing_survives_the_step_and_release_dispatches_it() {
        let mut sim = Simulation::new(scenario()).unwrap();
        let mut busy = None;
        for _ in 0..200 {
            sim.tick(100);
            if let Some(found) = find_running(&sim) {
                busy = Some(found);
                break;
            }
        }
        let (_equipment, lot_id, busy_until, _wafer) = busy.expect("a tool loaded a lot");
        let step = sim.state.lots[&lot_id].lot.current_step;
        sim.hold_lot(&lot_id, "engineer review").unwrap();
        assert!(
            !sim.state
                .queues
                .values()
                .any(|queue| queue.iter().any(|id| *id == lot_id))
        );

        while sim.state.now_ms <= busy_until {
            sim.tick(100);
        }
        let aggregate = &sim.state.lots[&lot_id];
        assert_eq!(
            aggregate.lot.state,
            LotState::OnHold,
            "the hold survives the step"
        );
        assert!(
            aggregate.lot.current_step > step,
            "the log moved the lot to the next step"
        );
        assert!(
            !sim.state
                .queues
                .values()
                .any(|queue| queue.iter().any(|id| *id == lot_id))
        );

        sim.release_lot(&lot_id).unwrap();
        let step = sim.state.lots[&lot_id].lot.current_step;
        assert!(
            sim.state
                .queues
                .get(&step)
                .is_some_and(|queue| queue.iter().any(|id| *id == lot_id)),
            "release puts the lot back in the queue"
        );
        let mut loaded = false;
        for _ in 0..200 {
            let output = sim.tick(100);
            if output
                .events
                .iter()
                .any(|event| event.kind() == EventKind::TrackIn && event.lot_id == lot_id)
            {
                loaded = true;
                break;
            }
        }
        assert!(loaded, "a released lot is dispatched again");
    }

    #[test]
    fn scrapping_every_wafer_completes_the_lot_and_removes_it_from_the_queue() {
        let mut sim = queued_sim(&[(1, 5)]);
        let lot_id = "LOT-0001".to_owned();
        let wafers: Vec<String> = sim.state.lots[&lot_id]
            .wafers
            .iter()
            .map(|wafer| wafer.id.clone())
            .collect();
        let mut events = Vec::new();
        for wafer in &wafers {
            events.extend(
                sim.scrap_wafer(&lot_id, wafer, "quality disposition")
                    .unwrap(),
            );
        }
        assert_eq!(sim.state.lots[&lot_id].lot.state, LotState::Completed);
        assert!(
            events
                .iter()
                .any(|event| event.kind() == EventKind::LotCompleted)
        );
        assert!(
            !sim.state
                .queues
                .values()
                .any(|queue| queue.iter().any(|id| id == &lot_id))
        );
    }
}
