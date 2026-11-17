//! Discrete-event simulation engine.
//!
//! The engine keeps a `BinaryHeap` of scheduled events, one queue per route
//! step and a bucket of tools per equipment kind. Every tick advances the
//! simulated clock, drains the events that became due and dispatches material
//! with hand-written rules: FIFO within a priority class, a qualification
//! matrix, and a verified recipe. No solver, no per-item thread.

use std::cmp::Reverse;
use std::collections::{BTreeMap, BinaryHeap, VecDeque};

use oxide_core::events::{DomainEvent, EventKind};
use oxide_core::model::{
    Equipment, EquipmentState, Lot, LotState, Route, Wafer, WaferId, WaferState,
    deterministic_event_id, lot_id_for,
};
use oxide_core::scenario::Scenario;
use oxide_core::state_machine::{EventIds, LotAggregate, equipment_down, equipment_up};
use serde::{Deserialize, Serialize};

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
}

/// The simulator.
#[derive(Debug, Clone)]
pub struct Simulation {
    pub scenario: Scenario,
    pub route: Route,
    pub state: SimState,
    heap: BinaryHeap<Reverse<Scheduled>>,
    schedule_seq: u64,
}

impl Simulation {
    pub fn new(scenario: Scenario) -> Result<Self, String> {
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
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
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
        };
        let mut sim = Self {
            scenario,
            route,
            state,
            heap: BinaryHeap::new(),
            schedule_seq: 0,
        };
        sim.schedule_release();
        Ok(sim)
    }

    /// Resume from a snapshot. The caller applies any newer events with
    /// [`Simulation::apply_event`] and then calls [`Simulation::rebuild_heap`].
    pub fn from_state(scenario: Scenario, state: SimState) -> Result<Self, String> {
        scenario.validate()?;
        let route = scenario.route.clone();
        Ok(Self {
            scenario,
            route,
            state,
            heap: BinaryHeap::new(),
            schedule_seq: 0,
        })
    }

    pub fn snapshot(&self) -> SimState {
        self.state.clone()
    }

    pub fn is_running(&self) -> bool {
        self.state.running
    }

    pub fn set_running(&mut self, running: bool) {
        self.state.running = running;
    }

    pub fn set_speed(&mut self, speed: f64) {
        self.state.speed = speed.clamp(0.1, 100.0);
    }

    pub fn speed(&self) -> f64 {
        self.state.speed
    }

    pub fn now_ms(&self) -> i64 {
        self.state.now_ms
    }

    pub fn mark_persisted(&mut self, count: u64) {
        self.state.persisted_events += count;
    }

    pub fn persisted_events(&self) -> u64 {
        self.state.persisted_events
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
    /// This is the projection half of the simulator: it never schedules
    /// anything, so the caller decides when to rebuild the heap.
    pub fn apply_event(&mut self, event: &DomainEvent) {
        self.state.now_ms = self.state.now_ms.max(event.occurred_at_ms);
        self.state.event_counter = self.state.event_counter.saturating_add(1);
        match event.kind {
            EventKind::LotCreated => {
                let wafers: Vec<String> = event
                    .payload
                    .get("wafers")
                    .and_then(serde_json::Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                let mut lot = Lot::new(
                    event.lot_id.clone(),
                    event
                        .payload
                        .get("product_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default(),
                    event
                        .payload
                        .get("route_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default(),
                    event
                        .payload
                        .get("priority")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0) as i32,
                    wafers.len() as i32,
                    event.occurred_at_ms,
                );
                lot.state = LotState::Created;
                let wafers = wafers
                    .into_iter()
                    .enumerate()
                    .map(|(index, id)| Wafer {
                        id,
                        lot_id: event.lot_id.clone(),
                        slot: index as i32 + 1,
                        state: WaferState::Created,
                        defect_info: None,
                        rework_count: 0,
                    })
                    .collect();
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
                self.state
                    .lots
                    .entry(event.lot_id.clone())
                    .or_insert_with(|| LotAggregate::new(lot, wafers));
            }
            EventKind::LotQueued => {
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id) {
                    if let Some(step) = event.step_no() {
                        aggregate.lot.current_step = step;
                    }
                    if !aggregate
                        .wafers
                        .iter()
                        .any(|w| w.state == WaferState::Processing)
                    {
                        aggregate.lot.state = LotState::Queued;
                    }
                    for wafer in &mut aggregate.wafers {
                        if matches!(
                            wafer.state,
                            WaferState::Created | WaferState::Waiting | WaferState::Queued
                        ) {
                            wafer.state = WaferState::Queued;
                        }
                    }
                }
            }
            EventKind::RecipeVerified => {
                let recipe_id = event
                    .payload
                    .get("recipe_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                let checksum = event
                    .payload
                    .get("checksum")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default();
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id) {
                    aggregate
                        .verified_recipes
                        .insert(recipe_id.to_owned(), checksum.to_owned());
                }
            }
            EventKind::TrackIn => {
                let equipment_id = event.equipment_id().unwrap_or_default().to_owned();
                let wafers = event.wafers();
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id) {
                    for wafer in &mut aggregate.wafers {
                        if wafers.contains(&wafer.id) {
                            wafer.state = WaferState::Processing;
                        }
                    }
                    aggregate.lot.state = LotState::Processing;
                }
                if let Some(runtime) = self.state.equipment.get_mut(&equipment_id) {
                    let step_no = event.step_no().unwrap_or(0);
                    let duration = self
                        .route
                        .step(step_no)
                        .map(|step| step.duration_ms)
                        .unwrap_or(0);
                    runtime.equipment.state = EquipmentState::Running;
                    runtime.busy_lot = Some(event.lot_id.clone());
                    runtime.busy_step = step_no;
                    runtime.busy_recipe = event
                        .payload
                        .get("recipe_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    runtime.busy_wafers = wafers;
                    runtime.busy_until_ms = event.occurred_at_ms + duration;
                }
            }
            EventKind::TrackOut => {
                let equipment_id = event.equipment_id().unwrap_or_default().to_owned();
                let wafers = event.wafers();
                let step_no = event.step_no().unwrap_or(0);
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id) {
                    for wafer in &mut aggregate.wafers {
                        if wafers.contains(&wafer.id) {
                            wafer.state = WaferState::Waiting;
                        }
                    }
                    let done = aggregate.completed.entry(step_no).or_default();
                    for id in wafers {
                        if !done.contains(&id) {
                            done.push(id);
                        }
                    }
                    aggregate.lot.state = if aggregate
                        .wafers
                        .iter()
                        .any(|w| w.state == WaferState::Processing)
                    {
                        LotState::Processing
                    } else {
                        LotState::Waiting
                    };
                }
                if let Some(runtime) = self.state.equipment.get_mut(&equipment_id) {
                    runtime.equipment.state = EquipmentState::Idle;
                    runtime.busy_lot = None;
                    runtime.busy_wafers.clear();
                    runtime.lots_processed += 1;
                }
            }
            EventKind::LotCompleted => {
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id) {
                    for wafer in &mut aggregate.wafers {
                        if wafer.state != WaferState::Scrapped {
                            wafer.state = WaferState::Done;
                        }
                    }
                    aggregate.lot.state = LotState::Completed;
                }
            }
            EventKind::LotHeld => {
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id) {
                    aggregate.lot.state = LotState::OnHold;
                    aggregate.lot.hold_reason = event
                        .payload
                        .get("reason")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                }
            }
            EventKind::LotReleased => {
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id) {
                    aggregate.lot.state = LotState::Waiting;
                    aggregate.lot.hold_reason = None;
                }
            }
            EventKind::ReworkStarted => {
                let to_step = event
                    .payload
                    .get("to_step")
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(0) as i32;
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id) {
                    aggregate.lot.current_step = to_step;
                    aggregate.lot.state = LotState::Waiting;
                    aggregate.completed.retain(|step, _| *step < to_step);
                    for wafer in &mut aggregate.wafers {
                        if wafer.state != WaferState::Scrapped {
                            wafer.state = WaferState::Waiting;
                            wafer.rework_count += 1;
                        }
                    }
                }
            }
            EventKind::WaferScrapped => {
                let wafer_id = event
                    .payload
                    .get("wafer_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                if let Some(aggregate) = self.state.lots.get_mut(&event.lot_id)
                    && let Some(wafer) = aggregate.wafers.iter_mut().find(|w| w.id == wafer_id)
                {
                    wafer.state = WaferState::Scrapped;
                    wafer.defect_info = event
                        .payload
                        .get("reason")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned);
                }
            }
            EventKind::LotSplit => {
                let Some(children) = event
                    .payload
                    .get("children")
                    .and_then(serde_json::Value::as_array)
                else {
                    return;
                };
                let parent_wafers: Vec<Wafer> = self
                    .state
                    .lots
                    .get(&event.lot_id)
                    .map(|a| a.wafers.clone())
                    .unwrap_or_default();
                let parent_lot = self
                    .state
                    .lots
                    .get(&event.lot_id)
                    .map(|a| a.lot.clone())
                    .unwrap_or_else(|| {
                        Lot::new(
                            &event.lot_id,
                            "",
                            &self.route.id,
                            0,
                            0,
                            event.occurred_at_ms,
                        )
                    });
                if let Some(parent) = self.state.lots.get_mut(&event.lot_id) {
                    parent.wafers.clear();
                    parent.lot.wafer_count = 0;
                    parent.lot.state = LotState::Split;
                }
                for child in children {
                    let child_id = child
                        .get("lot_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    let ids: Vec<String> = child
                        .get("wafers")
                        .and_then(serde_json::Value::as_array)
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(serde_json::Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default();
                    let mut lot = parent_lot.clone();
                    lot.id = child_id.clone();
                    lot.parent_lot_id = Some(event.lot_id.clone());
                    lot.wafer_count = ids.len() as i32;
                    lot.state = LotState::Waiting;
                    let mut wafers: Vec<Wafer> = parent_wafers
                        .iter()
                        .filter(|w| ids.contains(&w.id))
                        .cloned()
                        .collect();
                    for (index, wafer) in wafers.iter_mut().enumerate() {
                        wafer.lot_id = child_id.clone();
                        wafer.slot = index as i32 + 1;
                        wafer.state = WaferState::Waiting;
                    }
                    self.state
                        .lots
                        .insert(child_id, LotAggregate::new(lot, wafers));
                }
            }
            EventKind::LotMerged => {
                let target_id = event
                    .payload
                    .get("target_lot_id")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                let moved: Vec<String> = event
                    .payload
                    .get("wafers")
                    .and_then(serde_json::Value::as_array)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(serde_json::Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                let mut taken: Vec<Wafer> = Vec::new();
                if let Some(source) = self.state.lots.get_mut(&event.lot_id) {
                    taken = source
                        .wafers
                        .iter()
                        .filter(|w| moved.contains(&w.id))
                        .cloned()
                        .collect();
                    source.wafers.retain(|w| !moved.contains(&w.id));
                    source.lot.state = LotState::Merged;
                    source.lot.wafer_count = 0;
                }
                if let Some(target) = self.state.lots.get_mut(&target_id) {
                    for mut wafer in taken {
                        wafer.lot_id = target_id.clone();
                        wafer.slot = target.wafers.len() as i32 + 1;
                        wafer.state = WaferState::Waiting;
                        target.wafers.push(wafer);
                    }
                    target.lot.wafer_count = target.wafers.len() as i32;
                }
            }
            EventKind::EquipmentDown => {
                let equipment_id = event.equipment_id().unwrap_or_default().to_owned();
                if let Some(runtime) = self.state.equipment.get_mut(&equipment_id) {
                    runtime.equipment.state = EquipmentState::Down;
                    runtime.down_since_ms = event.occurred_at_ms;
                    runtime.down_until_ms = event.occurred_at_ms + self.scenario.repair_ms;
                }
            }
            EventKind::EquipmentUp => {
                let equipment_id = event.equipment_id().unwrap_or_default().to_owned();
                if let Some(runtime) = self.state.equipment.get_mut(&equipment_id) {
                    runtime.equipment.state = EquipmentState::Idle;
                    runtime.down_since_ms = 0;
                    runtime.down_until_ms = 0;
                }
            }
            EventKind::TrackOutDuplicate => {}
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
                    output.events.extend(queued_events);
                    self.state
                        .queues
                        .entry(first)
                        .or_default()
                        .push_back(lot_id.clone());
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
        let mut ids = self.ids_base();
        let Some(aggregate) = self.state.lots.get_mut(lot_id) else {
            return;
        };
        let transition = aggregate.track_out(
            step_no,
            equipment_id,
            &step.recipe_id,
            &checksum,
            wafers,
            duration_ms,
            measured,
            &mut ids,
            self.state.now_ms,
            "equipment",
        );
        match transition {
            Ok(transition) => {
                self.advance_counter(ids.used());
                output.events.extend(transition.events.clone());
                if let Some(event) = transition.events.first()
                    && event.kind == EventKind::TrackOut
                {
                    output.secs.push(SecsTrigger::ProcessCompleted {
                        lot_id: lot_id.to_owned(),
                        step_no,
                        equipment_id: equipment_id.to_owned(),
                    });
                }
            }
            Err(error) => {
                tracing::warn!(%error, lot = lot_id, step = step_no, "track-out refused");
                return;
            }
        }
        if let Some(runtime) = self.state.equipment.get_mut(equipment_id) {
            runtime.equipment.state = EquipmentState::Idle;
            runtime.busy_lot = None;
            runtime.busy_wafers.clear();
            runtime.busy_until_ms = 0;
            runtime.lots_processed += 1;
        }
        let all_done = self
            .state
            .lots
            .get(lot_id)
            .map(|aggregate| aggregate.step_finished(step_no))
            .unwrap_or(false);
        if !all_done {
            self.enqueue_at(lot_id, step_no, output);
            return;
        }
        match self.route.next_step(step_no) {
            Some(next) => self.enqueue_at(lot_id, next, output),
            None => {
                let mut ids = self.ids_base();
                let mut events = Vec::new();
                let completed = {
                    let Some(aggregate) = self.state.lots.get_mut(lot_id) else {
                        return;
                    };
                    aggregate
                        .complete(step_no, &mut ids, self.state.now_ms, "mes")
                        .map(|transition| {
                            events = transition.events;
                        })
                        .is_ok()
                };
                if completed {
                    self.advance_counter(ids.used());
                    output.events.extend(events);
                }
            }
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
            output.events.extend(events);
            let queue = self.state.queues.entry(step_no).or_default();
            if !queue.iter().any(|id| id == lot_id) {
                queue.push_back(lot_id.to_owned());
            }
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
        runtime.equipment.state = EquipmentState::Idle;
        runtime.down_since_ms = 0;
        runtime.down_until_ms = 0;
        output.events.push(equipment_up(
            equipment_id,
            &kind,
            down_ms,
            event_id,
            now_ms,
            "maintenance",
        ));
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
                let Some(equipment_id) = self.pick_equipment(&step) else {
                    break;
                };
                // A tool that fails while loading goes down before any material
                // moves; the lot stays queued for the next tool.
                if self.roll_failure() {
                    self.take_tool_down(&equipment_id, output);
                    continue;
                }
                if !self.ensure_recipe(&lot_id, &step, output) {
                    break;
                }
                let wanted: Vec<WaferId> = self
                    .state
                    .lots
                    .get(&lot_id)
                    .map(|aggregate| {
                        aggregate
                            .wafers
                            .iter()
                            .filter(|w| w.state == WaferState::Queued)
                            .map(|w| w.id.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                if wanted.is_empty() {
                    self.dequeue(step_no, &lot_id);
                    continue;
                }
                let recipe_checksum = self
                    .scenario
                    .recipe(&step.recipe_id)
                    .map(|r| r.checksum)
                    .unwrap_or_default();
                let mut ids = self.ids_base();
                let mut refused = None;
                let mut tracked = false;
                {
                    let Some(aggregate) = self.state.lots.get_mut(&lot_id) else {
                        break;
                    };
                    aggregate
                        .verified_recipes
                        .insert(step.recipe_id.clone(), recipe_checksum);
                    let transition = aggregate.track_in(
                        step_no,
                        &equipment_id,
                        &step.recipe_id,
                        &wanted,
                        &mut ids,
                        self.state.now_ms,
                        "dispatcher",
                    );
                    match transition {
                        Ok(transition) => {
                            output.events.extend(transition.events);
                            tracked = true;
                        }
                        Err(error) => refused = Some(error),
                    }
                }
                match refused {
                    Some(error) => {
                        tracing::warn!(%error, lot = %lot_id, step = step_no, "track-in refused");
                        self.dequeue(step_no, &lot_id);
                        continue;
                    }
                    None => {
                        if tracked {
                            self.advance_counter(ids.used());
                        }
                    }
                }
                output.secs.push(SecsTrigger::ProcessStarted {
                    lot_id: lot_id.clone(),
                    step_no,
                    equipment_id: equipment_id.clone(),
                });
                if let Some(runtime) = self.state.equipment.get_mut(&equipment_id) {
                    runtime.equipment.state = EquipmentState::Running;
                    runtime.busy_lot = Some(lot_id.clone());
                    runtime.busy_step = step_no;
                    runtime.busy_recipe = step.recipe_id.clone();
                    runtime.busy_wafers = wanted.clone();
                    runtime.busy_until_ms = self.state.now_ms + step.duration_ms;
                }
                self.dequeue(step_no, &lot_id);
                self.schedule(
                    self.state.now_ms + step.duration_ms,
                    ScheduledKind::StepComplete {
                        equipment_id: equipment_id.clone(),
                        lot_id,
                        step_no,
                        wafers: wanted,
                    },
                );
            }
        }
    }

    /// Highest priority first, FIFO within a priority, skipping held, busy and
    /// fully processed lots.
    fn pick_lot(&self, step_no: i32) -> Option<String> {
        let queue = self.state.queues.get(&step_no)?;
        let mut best: Option<(i32, &String)> = None;
        for lot_id in queue {
            let Some(aggregate) = self.state.lots.get(lot_id) else {
                continue;
            };
            if aggregate.lot.state != LotState::Queued && aggregate.lot.state != LotState::Waiting {
                continue;
            }
            if aggregate.lot.current_step != step_no {
                continue;
            }
            if aggregate
                .wafers
                .iter()
                .any(|w| w.state == WaferState::Processing)
            {
                continue;
            }
            if aggregate.lot.hold_reason.is_some() {
                continue;
            }
            if !aggregate
                .wafers
                .iter()
                .any(|w| w.state == WaferState::Queued)
            {
                continue;
            }
            // Strictly greater: the first lot of the highest priority wins,
            // which keeps FIFO order within a priority class.
            if best
                .as_ref()
                .is_none_or(|(priority, _)| aggregate.lot.priority > *priority)
            {
                best = Some((aggregate.lot.priority, lot_id));
            }
        }
        best.map(|(_, lot_id)| lot_id.clone())
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
            .map(|aggregate| aggregate.verified_recipes.contains_key(&step.recipe_id))
            .unwrap_or(false);
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
        runtime.equipment.state = EquipmentState::Down;
        runtime.down_since_ms = now_ms;
        runtime.down_until_ms = now_ms + repair_ms;
        output.events.push(equipment_down(
            equipment_id,
            &kind,
            "simulated chamber fault",
            2001,
            event_id,
            now_ms,
            "equipment",
        ));
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
}

impl Simulation {
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
                    queue_len: queue.map(VecDeque::len).unwrap_or(0),
                    busy_tools,
                    total_tools,
                    current_lots: queue
                        .map(|q| q.iter().cloned().collect())
                        .unwrap_or_default(),
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
        }
    }

    /// Operator actions, used by the API and the dashboard.
    pub fn hold_lot(&mut self, lot_id: &str, reason: &str) -> Result<Vec<DomainEvent>, String> {
        let mut ids = self.ids_base();
        let events;
        {
            let aggregate = self
                .state
                .lots
                .get_mut(lot_id)
                .ok_or_else(|| format!("unknown lot {lot_id}"))?;
            let transition = aggregate
                .hold(reason, &mut ids, self.state.now_ms, "operator")
                .map_err(|e| e.to_string())?;
            events = transition.events;
        }
        self.advance_counter(ids.used());
        self.remove_from_queues(lot_id);
        Ok(events)
    }

    pub fn release_lot(&mut self, lot_id: &str) -> Result<Vec<DomainEvent>, String> {
        let mut ids = self.ids_base();
        let events;
        let step;
        {
            let aggregate = self
                .state
                .lots
                .get_mut(lot_id)
                .ok_or_else(|| format!("unknown lot {lot_id}"))?;
            let transition = aggregate
                .release(&mut ids, self.state.now_ms, "operator")
                .map_err(|e| e.to_string())?;
            events = transition.events;
            step = aggregate.lot.current_step;
        }
        self.advance_counter(ids.used());
        self.state
            .queues
            .entry(step)
            .or_default()
            .push_back(lot_id.to_owned());
        Ok(events)
    }

    pub fn rework_lot(
        &mut self,
        lot_id: &str,
        to_step: i32,
        reason: &str,
    ) -> Result<Vec<DomainEvent>, String> {
        let mut ids = self.ids_base();
        let events;
        {
            let aggregate = self
                .state
                .lots
                .get_mut(lot_id)
                .ok_or_else(|| format!("unknown lot {lot_id}"))?;
            let transition = aggregate
                .rework(to_step, reason, &mut ids, self.state.now_ms, "engineer")
                .map_err(|e| e.to_string())?;
            events = transition.events;
        }
        self.advance_counter(ids.used());
        self.remove_from_queues(lot_id);
        self.state
            .queues
            .entry(to_step)
            .or_default()
            .push_back(lot_id.to_owned());
        Ok(events)
    }

    pub fn scrap_wafer(
        &mut self,
        lot_id: &str,
        wafer_id: &str,
        reason: &str,
    ) -> Result<Vec<DomainEvent>, String> {
        let mut ids = self.ids_base();
        let events;
        {
            let aggregate = self
                .state
                .lots
                .get_mut(lot_id)
                .ok_or_else(|| format!("unknown lot {lot_id}"))?;
            let transition = aggregate
                .scrap_wafer(wafer_id, reason, &mut ids, self.state.now_ms, "engineer")
                .map_err(|e| e.to_string())?;
            events = transition.events;
        }
        self.advance_counter(ids.used());
        Ok(events)
    }

    pub fn split_lot(
        &mut self,
        lot_id: &str,
        child_ids: &[String],
    ) -> Result<Vec<DomainEvent>, String> {
        let mut ids = self.ids_base();
        let children;
        let events;
        {
            let aggregate = self
                .state
                .lots
                .get_mut(lot_id)
                .ok_or_else(|| format!("unknown lot {lot_id}"))?;
            let transition = aggregate
                .split(child_ids, &mut ids, self.state.now_ms, "operator")
                .map_err(|e| e.to_string())?;
            children = transition.children;
            events = transition.events;
        }
        self.advance_counter(ids.used());
        self.remove_from_queues(lot_id);
        for child in children {
            let step = child.lot.current_step;
            self.state
                .queues
                .entry(step)
                .or_default()
                .push_back(child.lot.id.clone());
            self.state.lots.insert(child.lot.id.clone(), child);
        }
        Ok(events)
    }

    pub fn merge_lots(
        &mut self,
        target_lot: &str,
        source_lots: &[String],
    ) -> Result<Vec<DomainEvent>, String> {
        if source_lots.iter().any(|id| id == target_lot) {
            return Err("a lot cannot be merged into itself".to_owned());
        }
        let mut sources = Vec::new();
        for id in source_lots {
            let source = self
                .state
                .lots
                .remove(id)
                .ok_or_else(|| format!("unknown lot {id}"))?;
            sources.push(source);
        }
        let mut ids = self.ids_base();
        let mut events = None;
        let mut step = 0;
        let mut failed = None;
        {
            let Some(aggregate) = self.state.lots.get_mut(target_lot) else {
                for source in sources {
                    self.state.lots.insert(source.lot.id.clone(), source);
                }
                return Err(format!("unknown lot {target_lot}"));
            };
            match aggregate.merge(&mut sources, &mut ids, self.state.now_ms, "operator") {
                Ok(transition) => {
                    events = Some(transition.events);
                    step = aggregate.lot.current_step;
                }
                Err(error) => failed = Some(error),
            }
        }
        self.advance_counter(ids.used());
        match failed {
            Some(error) => {
                for source in sources {
                    self.state.lots.insert(source.lot.id.clone(), source);
                }
                Err(error.to_string())
            }
            None => {
                for source in sources {
                    self.remove_from_queues(&source.lot.id);
                    self.state.lots.insert(source.lot.id.clone(), source);
                }
                self.remove_from_queues(target_lot);
                self.state
                    .queues
                    .entry(step)
                    .or_default()
                    .push_back(target_lot.to_owned());
                Ok(events.unwrap_or_default())
            }
        }
    }

    fn remove_from_queues(&mut self, lot_id: &str) {
        for queue in self.state.queues.values_mut() {
            if let Some(position) = queue.iter().position(|id| id == lot_id) {
                queue.remove(position);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxide_core::model::EquipmentState;

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
            .filter(|e| e.kind == EventKind::TrackOut)
            .count();
        assert!(
            track_outs >= 8,
            "expected 4 track-outs per lot, got {track_outs}"
        );
    }

    #[test]
    fn dispatch_respects_priority() {
        let mut scenario = scenario();
        scenario.lot_count = 3;
        let mut sim = Simulation::new(scenario).unwrap();
        // Make all three lots due at once.
        sim.state.next_release_ms = 0;
        sim.state.released_lots = 3;
        sim.state.lots.clear();
        for (serial, priority) in [(1u64, 1), (2, 9), (3, 5)] {
            let lot = Lot::new(lot_id_for(serial), "PRD", &sim.route.id, priority, 2, 0);
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
            .find(|e| e.kind == EventKind::TrackIn)
            .expect("a lot was loaded");
        assert_eq!(
            loaded.lot_id, "LOT-0002",
            "the highest priority lot goes first"
        );
        assert!(running.contains(&"LOT-0002".to_owned()));
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
                .any(|e| e.kind == EventKind::EquipmentDown)
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
}
