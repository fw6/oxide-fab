//! The lot state machine.
//!
//! A lot moves through `created → queued → processing → waiting → … →
//! completed`. Every transition is a method that returns the events describing
//! it, or an error explaining why it is illegal. There is no other way to
//! mutate a lot: the simulator, the API and the tests all go through here, so
//! an illegal move cannot reach the event log.

use std::collections::{HashMap, HashSet};
use std::fmt;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::events::{
    DomainEvent, EventKind, TrackInReport, TrackOutReport, equipment_down_payload,
    equipment_up_payload, lot_created_payload, lot_queued_payload, merged_payload, split_payload,
    track_in_payload, track_out_payload,
};
use crate::model::{Lot, LotId, LotState, Wafer, WaferId, WaferState};

/// Allocate the event ids of one transition deterministically.
#[derive(Debug, Clone)]
pub struct EventIds {
    base: Uuid,
    offset: u32,
}

impl EventIds {
    #[must_use]
    pub fn new(base: Uuid) -> Self {
        Self { base, offset: 0 }
    }

    /// The next id of this set.
    ///
    /// Named `next_id`, not `next`: the id stream never ends, so it is not an
    /// `Iterator`.
    pub fn next_id(&mut self) -> Uuid {
        let id = Uuid::from_u128(self.base.as_u128().wrapping_add(self.offset as u128));
        self.offset += 1;
        id
    }

    /// How many ids this set handed out, so the caller can advance its counter
    /// past them (ids stay unique across a restart).
    #[must_use]
    pub fn used(&self) -> u32 {
        self.offset
    }
}

/// Why a transition was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    IllegalState {
        from: LotState,
        action: &'static str,
    },
    StepMismatch {
        expected: i32,
        actual: i32,
    },
    UnknownWafer(String),
    EmptyWaferSet,
    HoldWithoutReason,
    NotOnHold,
    /// The lot's life has ended: it is completed, split or merged
    /// ([`LotState::is_terminal`]).
    AlreadyCompleted,
    DifferentRoute {
        other: String,
    },
    DifferentStep {
        other: i32,
    },
    ReworkTargetNotEarlier {
        from: i32,
        to: i32,
    },
    NotEnoughWafers {
        needed: usize,
        available: usize,
    },
    RecipeNotVerified(String),
}

impl fmt::Display for TransitionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransitionError::IllegalState { from, action } => {
                write!(f, "cannot {action} a lot in state {}", from.as_str())
            }
            TransitionError::StepMismatch { expected, actual } => {
                write!(f, "lot is at step {actual}, not {expected}")
            }
            TransitionError::UnknownWafer(id) => write!(f, "lot has no wafer {id}"),
            TransitionError::EmptyWaferSet => write!(f, "no wafers selected"),
            TransitionError::HoldWithoutReason => write!(f, "a hold needs a reason"),
            TransitionError::NotOnHold => write!(f, "lot is not on hold"),
            TransitionError::AlreadyCompleted => write!(f, "lot is already completed"),
            TransitionError::DifferentRoute { other } => {
                write!(f, "lot follows route {other}, not the same route")
            }
            TransitionError::DifferentStep { other } => {
                write!(f, "lot is at step {other}, not the same step")
            }
            TransitionError::ReworkTargetNotEarlier { from, to } => {
                write!(f, "rework target {to} is not earlier than step {from}")
            }
            TransitionError::NotEnoughWafers { needed, available } => {
                write!(f, "needs {needed} wafers, has {available}")
            }
            TransitionError::RecipeNotVerified(id) => {
                write!(f, "recipe {id} checksum was not verified")
            }
        }
    }
}

impl std::error::Error for TransitionError {}

/// What a transition produced.
#[derive(Debug, Clone, Default)]
pub struct Transition {
    pub events: Vec<DomainEvent>,
    /// Lots created by this transition (a split, or the lot itself on create).
    pub children: Vec<LotAggregate>,
}

impl Transition {
    #[must_use]
    pub fn events(&self) -> &[DomainEvent] {
        &self.events
    }
}

/// A lot plus its wafers: the consistency boundary of the state machine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LotAggregate {
    pub lot: Lot,
    pub wafers: Vec<Wafer>,
    /// Wafers that finished each step. A track-out whose wafers are all in
    /// this set is a retry from the tool, and changes nothing.
    pub completed: HashMap<i32, Vec<WaferId>>,
    /// Recipe checksums verified for this lot (`recipe_id → checksum`).
    pub verified_recipes: HashMap<String, String>,
}

impl LotAggregate {
    #[must_use]
    pub fn new(lot: Lot, wafers: Vec<Wafer>) -> Self {
        Self {
            lot,
            wafers,
            completed: HashMap::new(),
            verified_recipes: HashMap::new(),
        }
    }

    /// The highest step every active wafer has finished, if any.
    #[must_use]
    pub fn last_completed_step(&self) -> i32 {
        self.completed.keys().copied().max().unwrap_or(0)
    }

    /// Create a lot and its wafers. The created aggregate is returned in
    /// `children`, so callers can insert it into their state.
    pub fn create(
        lot: Lot,
        wafer_ids: Vec<WaferId>,
        ids: &mut EventIds,
        now_ms: i64,
        actor: &str,
    ) -> Result<Transition, TransitionError> {
        if wafer_ids.is_empty() {
            return Err(TransitionError::EmptyWaferSet);
        }
        let wafers: Vec<Wafer> = wafer_ids
            .into_iter()
            .enumerate()
            .map(|(index, id)| Wafer {
                id,
                lot_id: lot.id.clone(),
                slot: index as i32 + 1,
                state: WaferState::Created,
                defect_info: None,
                rework_count: 0,
            })
            .collect();
        let event = DomainEvent::new(
            ids.next_id(),
            lot.id.clone(),
            EventKind::LotCreated,
            lot_created_payload(
                &lot.product_id,
                &lot.route_id,
                lot.priority,
                &wafers.iter().map(|w| w.id.clone()).collect::<Vec<_>>(),
            ),
            now_ms,
            actor,
        );
        let aggregate = LotAggregate::new(lot, wafers);
        Ok(Transition {
            events: vec![event],
            children: vec![aggregate],
        })
    }

    pub fn active_wafers(&self) -> impl Iterator<Item = &Wafer> {
        self.wafers
            .iter()
            .filter(|w| w.state != WaferState::Scrapped)
    }

    #[must_use]
    pub fn active_count(&self) -> usize {
        self.active_wafers().count()
    }

    /// Put the lot in the queue of `step_no`.
    pub fn queue(
        &mut self,
        step_no: i32,
        ids: &mut EventIds,
        now_ms: i64,
        actor: &str,
    ) -> Result<Transition, TransitionError> {
        if self.lot.state == LotState::OnHold {
            return Err(TransitionError::IllegalState {
                from: self.lot.state,
                action: "queue",
            });
        }
        if self.lot.state.is_terminal() {
            return Err(TransitionError::AlreadyCompleted);
        }
        let busy = self
            .wafers
            .iter()
            .any(|w| w.state == WaferState::Processing);
        self.lot.current_step = step_no;
        if !busy {
            self.lot.state = LotState::Queued;
        }
        for wafer in &mut self.wafers {
            if matches!(
                wafer.state,
                WaferState::Created | WaferState::Waiting | WaferState::Queued
            ) {
                wafer.state = WaferState::Queued;
            }
        }
        Ok(Transition {
            events: vec![DomainEvent::new(
                ids.next_id(),
                self.lot.id.clone(),
                EventKind::LotQueued,
                lot_queued_payload(step_no),
                now_ms,
                actor,
            )],
            children: vec![],
        })
    }

    /// Stop the lot: no dispatch, no track-in, until released.
    pub fn hold(
        &mut self,
        reason: &str,
        ids: &mut EventIds,
        now_ms: i64,
        actor: &str,
    ) -> Result<Transition, TransitionError> {
        if reason.trim().is_empty() {
            return Err(TransitionError::HoldWithoutReason);
        }
        if self.lot.state.is_terminal() {
            return Err(TransitionError::AlreadyCompleted);
        }
        if self.lot.state == LotState::OnHold {
            return Ok(Transition::default());
        }
        self.lot.state = LotState::OnHold;
        self.lot.hold_reason = Some(reason.to_owned());
        Ok(Transition {
            events: vec![DomainEvent::new(
                ids.next_id(),
                self.lot.id.clone(),
                EventKind::LotHeld,
                json!({ "reason": reason, "step_no": self.lot.current_step }),
                now_ms,
                actor,
            )],
            children: vec![],
        })
    }

    /// Release a held lot back into its current step.
    pub fn release(
        &mut self,
        ids: &mut EventIds,
        now_ms: i64,
        actor: &str,
    ) -> Result<Transition, TransitionError> {
        if self.lot.state != LotState::OnHold {
            return Err(TransitionError::NotOnHold);
        }
        let reason = self.lot.hold_reason.take();
        self.lot.state = LotState::Waiting;
        for wafer in &mut self.wafers {
            if wafer.state == WaferState::Queued || wafer.state == WaferState::Waiting {
                wafer.state = WaferState::Waiting;
            }
        }
        Ok(Transition {
            events: vec![DomainEvent::new(
                ids.next_id(),
                self.lot.id.clone(),
                EventKind::LotReleased,
                json!({ "step_no": self.lot.current_step, "reason": reason }),
                now_ms,
                actor,
            )],
            children: vec![],
        })
    }

    /// Send the lot back to an earlier step (a rework).
    pub fn rework(
        &mut self,
        to_step: i32,
        reason: &str,
        ids: &mut EventIds,
        now_ms: i64,
        actor: &str,
    ) -> Result<Transition, TransitionError> {
        if self.lot.state == LotState::Processing {
            return Err(TransitionError::IllegalState {
                from: self.lot.state,
                action: "rework",
            });
        }
        if self.lot.state.is_terminal() {
            return Err(TransitionError::AlreadyCompleted);
        }
        if to_step <= 0 || to_step >= self.lot.current_step {
            return Err(TransitionError::ReworkTargetNotEarlier {
                from: self.lot.current_step,
                to: to_step,
            });
        }
        let from_step = self.lot.current_step;
        for wafer in &mut self.wafers {
            if wafer.state != WaferState::Scrapped {
                wafer.state = WaferState::Waiting;
                wafer.rework_count += 1;
            }
        }
        // Everything from the rework target onwards has to be redone.
        self.completed.retain(|step, _| *step < to_step);
        self.lot.state = LotState::Waiting;
        self.lot.current_step = to_step;
        Ok(Transition {
            events: vec![DomainEvent::new(
                ids.next_id(),
                self.lot.id.clone(),
                EventKind::ReworkStarted,
                json!({ "from_step": from_step, "to_step": to_step, "reason": reason }),
                now_ms,
                actor,
            )],
            children: vec![],
        })
    }

    /// Load `wafers` of this lot on a tool. A subset is legal: that is how a
    /// partial lot is processed while its siblings wait.
    pub fn track_in(
        &mut self,
        report: TrackInReport<'_>,
        ids: &mut EventIds,
        now_ms: i64,
        actor: &str,
    ) -> Result<Transition, TransitionError> {
        let TrackInReport {
            step_no,
            recipe_id,
            wafers,
            ..
        } = report;
        if self.lot.state.is_terminal() {
            return Err(TransitionError::AlreadyCompleted);
        }
        if self.lot.state == LotState::OnHold {
            return Err(TransitionError::IllegalState {
                from: self.lot.state,
                action: "track in",
            });
        }
        if self.lot.current_step != step_no {
            return Err(TransitionError::StepMismatch {
                expected: self.lot.current_step,
                actual: step_no,
            });
        }
        if !self.verified_recipes.contains_key(recipe_id) {
            return Err(TransitionError::RecipeNotVerified(recipe_id.to_owned()));
        }
        let selected = self.select_wafers(wafers)?;
        for wafer in &mut self.wafers {
            if selected.contains(&wafer.id) {
                wafer.state = WaferState::Processing;
            }
        }
        self.lot.state = LotState::Processing;
        Ok(Transition {
            events: vec![DomainEvent::new(
                ids.next_id(),
                self.lot.id.clone(),
                EventKind::TrackIn,
                track_in_payload(&report),
                now_ms,
                actor,
            )],
            children: vec![],
        })
    }

    /// A tool reports a finished (or partially finished) step.
    ///
    /// A repeated submission for wafers that already finished this step is
    /// idempotent: it records an audit event and changes nothing. Tools retry;
    /// the state machine must not advance material twice.
    pub fn track_out(
        &mut self,
        report: TrackOutReport<'_>,
        ids: &mut EventIds,
        now_ms: i64,
        actor: &str,
    ) -> Result<Transition, TransitionError> {
        let TrackOutReport {
            step_no,
            recipe_id,
            recipe_checksum,
            wafers,
            ..
        } = report;
        if self.lot.state.is_terminal() {
            return Err(TransitionError::AlreadyCompleted);
        }
        if self.lot.current_step != step_no {
            return Err(TransitionError::StepMismatch {
                expected: self.lot.current_step,
                actual: step_no,
            });
        }
        let verified = self
            .verified_recipes
            .get(recipe_id)
            .ok_or_else(|| TransitionError::RecipeNotVerified(recipe_id.to_owned()))?;
        if verified != recipe_checksum {
            return Err(TransitionError::RecipeNotVerified(recipe_id.to_owned()));
        }
        let selected = self.select_wafers(wafers)?;
        let already: HashSet<&WaferId> = self
            .completed
            .get(&step_no)
            .map_or_default(|done| done.iter().collect());

        if selected.iter().all(|id| already.contains(id)) {
            return Ok(Transition {
                events: vec![DomainEvent::new(
                    ids.next_id(),
                    self.lot.id.clone(),
                    EventKind::TrackOutDuplicate,
                    json!({
                        "step_no": step_no,
                        "equipment_id": report.equipment_id,
                        "wafers": wafers,
                        "idempotent": true,
                    }),
                    now_ms,
                    actor,
                )],
                children: vec![],
            });
        }

        let selected_owned: HashSet<WaferId> = selected;
        for wafer in &mut self.wafers {
            if selected_owned.contains(&wafer.id) {
                wafer.state = WaferState::Waiting;
            }
        }
        let entry = self.completed.entry(step_no).or_default();
        for id in &selected_owned {
            if !entry.contains(id) {
                entry.push(id.clone());
            }
        }
        let any_processing = self
            .wafers
            .iter()
            .any(|w| w.state == WaferState::Processing);
        self.lot.state = if any_processing {
            LotState::Processing
        } else {
            LotState::Waiting
        };
        Ok(Transition {
            events: vec![DomainEvent::new(
                ids.next_id(),
                self.lot.id.clone(),
                EventKind::TrackOut,
                track_out_payload(&report),
                now_ms,
                actor,
            )],
            children: vec![],
        })
    }

    /// `true` when every active wafer finished `step_no`.
    #[must_use]
    pub fn step_finished(&self, step_no: i32) -> bool {
        let Some(done) = self.completed.get(&step_no) else {
            return false;
        };
        self.active_wafers().all(|w| done.contains(&w.id))
    }

    /// Finish the route: every active wafer is done and the lot completes.
    pub fn complete(
        &mut self,
        step_no: i32,
        ids: &mut EventIds,
        now_ms: i64,
        actor: &str,
    ) -> Result<Transition, TransitionError> {
        if self.lot.state.is_terminal() {
            return Err(TransitionError::AlreadyCompleted);
        }
        // Material is on a tool: its track-out would arrive after the lot was
        // declared finished, and the tool's report would then be refused.
        if self.lot.state == LotState::Processing {
            return Err(TransitionError::IllegalState {
                from: self.lot.state,
                action: "complete",
            });
        }
        for wafer in &mut self.wafers {
            if wafer.state != WaferState::Scrapped {
                wafer.state = WaferState::Done;
            }
        }
        self.lot.state = LotState::Completed;
        Ok(Transition {
            events: vec![DomainEvent::new(
                ids.next_id(),
                self.lot.id.clone(),
                EventKind::LotCompleted,
                json!({ "step_no": step_no }),
                now_ms,
                actor,
            )],
            children: vec![],
        })
    }

    /// Scrap one wafer: the rest of the lot carries on (a partial lot).
    pub fn scrap_wafer(
        &mut self,
        wafer_id: &str,
        reason: &str,
        ids: &mut EventIds,
        now_ms: i64,
        actor: &str,
    ) -> Result<Transition, TransitionError> {
        let wafer = self
            .wafers
            .iter_mut()
            .find(|w| w.id == wafer_id)
            .ok_or_else(|| TransitionError::UnknownWafer(wafer_id.to_owned()))?;
        if wafer.state == WaferState::Scrapped {
            return Ok(Transition::default());
        }
        wafer.state = WaferState::Scrapped;
        wafer.defect_info = Some(reason.to_owned());
        let remaining = self
            .wafers
            .iter()
            .filter(|w| w.state != WaferState::Scrapped)
            .count();
        if remaining == 0 {
            self.lot.state = LotState::Completed;
        }
        Ok(Transition {
            events: vec![DomainEvent::new(
                ids.next_id(),
                self.lot.id.clone(),
                EventKind::WaferScrapped,
                json!({ "wafer_id": wafer_id, "reason": reason, "step_no": self.lot.current_step }),
                now_ms,
                actor,
            )
            .with_wafer(wafer_id)],
            children: vec![],
        })
    }

    /// Record that a recipe download was checksum-verified for this lot.
    pub fn verify_recipe(
        &mut self,
        recipe_id: &str,
        checksum: &str,
        ids: &mut EventIds,
        now_ms: i64,
        actor: &str,
    ) -> Transition {
        self.verified_recipes
            .insert(recipe_id.to_owned(), checksum.to_owned());
        Transition {
            events: vec![DomainEvent::new(
                ids.next_id(),
                self.lot.id.clone(),
                EventKind::RecipeVerified,
                json!({ "recipe_id": recipe_id, "checksum": checksum }),
                now_ms,
                actor,
            )],
            children: vec![],
        }
    }

    /// Split the lot into `child_ids`, distributing its wafers in order.
    ///
    /// The parent keeps no wafers and ends in the `split` terminal state. Each
    /// child continues at the parent's current step with the same route,
    /// priority and product.
    pub fn split(
        &mut self,
        child_ids: &[LotId],
        ids: &mut EventIds,
        now_ms: i64,
        actor: &str,
    ) -> Result<Transition, TransitionError> {
        if self.lot.state.is_terminal() || self.lot.state == LotState::Processing {
            return Err(TransitionError::IllegalState {
                from: self.lot.state,
                action: "split",
            });
        }
        let active: Vec<Wafer> = self
            .wafers
            .iter()
            .filter(|w| w.state != WaferState::Scrapped)
            .cloned()
            .collect();
        if child_ids.len() < 2 || active.len() < child_ids.len() {
            return Err(TransitionError::NotEnoughWafers {
                needed: child_ids.len(),
                available: active.len(),
            });
        }
        let mut buckets: Vec<Vec<Wafer>> = vec![Vec::new(); child_ids.len()];
        for (index, wafer) in active.into_iter().enumerate() {
            buckets[index % child_ids.len()].push(wafer);
        }
        let mut children = Vec::with_capacity(child_ids.len());
        let mut described: Vec<(String, Vec<WaferId>)> = Vec::new();
        for (child_id, mut wafers) in child_ids.iter().cloned().zip(buckets) {
            for (slot, wafer) in wafers.iter_mut().enumerate() {
                wafer.lot_id.clone_from(&child_id);
                wafer.slot = slot as i32 + 1;
                wafer.state = match self.lot.state {
                    LotState::Queued => WaferState::Queued,
                    _ => WaferState::Waiting,
                };
            }
            let mut child_lot = self.lot.clone();
            child_lot.id.clone_from(&child_id);
            child_lot.parent_lot_id = Some(self.lot.id.clone());
            child_lot.wafer_count = wafers.len() as i32;
            child_lot.state = match self.lot.state {
                LotState::Queued => LotState::Queued,
                _ => LotState::Waiting,
            };
            described.push((
                child_id.clone(),
                wafers.iter().map(|w| w.id.clone()).collect(),
            ));
            children.push(LotAggregate {
                lot: child_lot,
                wafers,
                completed: self.completed.clone(),
                verified_recipes: self.verified_recipes.clone(),
            });
        }
        let event = DomainEvent::new(
            ids.next_id(),
            self.lot.id.clone(),
            EventKind::LotSplit,
            split_payload(&self.lot.id, &described, self.lot.current_step),
            now_ms,
            actor,
        );
        self.wafers.clear();
        self.lot.wafer_count = 0;
        self.lot.state = LotState::Split;
        Ok(Transition {
            events: vec![event],
            children,
        })
    }

    /// Merge `sources` into this lot. All sources must be at the same route and
    /// step and none may be processing.
    pub fn merge(
        &mut self,
        sources: &mut [LotAggregate],
        ids: &mut EventIds,
        now_ms: i64,
        actor: &str,
    ) -> Result<Transition, TransitionError> {
        if sources.is_empty() {
            return Err(TransitionError::EmptyWaferSet);
        }
        if self.lot.state.is_terminal() || self.lot.state == LotState::Processing {
            return Err(TransitionError::IllegalState {
                from: self.lot.state,
                action: "merge",
            });
        }
        for source in sources.iter() {
            if source.lot.route_id != self.lot.route_id {
                return Err(TransitionError::DifferentRoute {
                    other: source.lot.route_id.clone(),
                });
            }
            if source.lot.current_step != self.lot.current_step {
                return Err(TransitionError::DifferentStep {
                    other: source.lot.current_step,
                });
            }
            if source.lot.state.is_terminal() || source.lot.state == LotState::Processing {
                return Err(TransitionError::IllegalState {
                    from: source.lot.state,
                    action: "merge",
                });
            }
        }
        let mut events = Vec::new();
        let mut next_slot = self.wafers.len() as i32 + 1;
        for source in sources.iter_mut() {
            let moved: Vec<WaferId> = source
                .wafers
                .iter()
                .filter(|w| w.state != WaferState::Scrapped)
                .map(|w| w.id.clone())
                .collect();
            for wafer in &mut source.wafers {
                if wafer.state == WaferState::Scrapped {
                    continue;
                }
                wafer.lot_id.clone_from(&self.lot.id);
                wafer.slot = next_slot;
                wafer.state = WaferState::Waiting;
                next_slot += 1;
                self.wafers.push(wafer.clone());
            }
            source.lot.state = LotState::Merged;
            source.lot.wafer_count = 0;
            events.push(DomainEvent::new(
                ids.next_id(),
                source.lot.id.clone(),
                EventKind::LotMerged,
                merged_payload(
                    std::slice::from_ref(&source.lot.id),
                    &self.lot.id,
                    self.lot.current_step,
                    &moved,
                ),
                now_ms,
                actor,
            ));
        }
        self.lot.wafer_count = self.wafers.len() as i32;
        self.lot.state = LotState::Waiting;
        Ok(Transition {
            events,
            children: vec![],
        })
    }

    fn select_wafers(&self, wafers: &[WaferId]) -> Result<HashSet<WaferId>, TransitionError> {
        if wafers.is_empty() {
            return Err(TransitionError::EmptyWaferSet);
        }
        let mut selected = HashSet::new();
        for id in wafers {
            let found = self
                .wafers
                .iter()
                .any(|w| &w.id == id && w.state != WaferState::Scrapped);
            if !found {
                return Err(TransitionError::UnknownWafer(id.clone()));
            }
            selected.insert(id.clone());
        }
        Ok(selected)
    }
}

/// Equipment transitions are small enough to live beside the lot machine.
#[must_use]
pub fn equipment_down(
    equipment_id: &str,
    kind: &str,
    reason: &str,
    alid: u32,
    event_id: Uuid,
    now_ms: i64,
    actor: &str,
) -> DomainEvent {
    DomainEvent::new(
        event_id,
        "",
        EventKind::EquipmentDown,
        equipment_down_payload(equipment_id, kind, reason, alid),
        now_ms,
        actor,
    )
}

#[must_use]
pub fn equipment_up(
    equipment_id: &str,
    kind: &str,
    down_ms: i64,
    event_id: Uuid,
    now_ms: i64,
    actor: &str,
) -> DomainEvent {
    DomainEvent::new(
        event_id,
        "",
        EventKind::EquipmentUp,
        equipment_up_payload(equipment_id, kind, down_ms),
        now_ms,
        actor,
    )
}

/// Payload of a track-out, for projections that need the measurement.
pub fn track_out_measurement(event: &DomainEvent) -> Option<f64> {
    event.payload.get("measured").and_then(Value::as_f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Recipe;

    fn agg(prefix: &str, wafers: i32) -> LotAggregate {
        let mut lot = Lot::new(prefix, "PRD-OXIDE-300", "RTE-LOGIC-4", 5, wafers, 0);
        lot.current_step = 1;
        lot.state = LotState::Queued;
        let wafers = (1..=wafers)
            .map(|slot| Wafer {
                id: format!("{prefix}-W{slot:02}"),
                lot_id: prefix.to_owned(),
                slot,
                state: WaferState::Queued,
                defect_info: None,
                rework_count: 0,
            })
            .collect();
        LotAggregate::new(lot, wafers)
    }

    fn ids() -> EventIds {
        EventIds::new(Uuid::from_u128(0x0f_ab))
    }

    fn checksum() -> String {
        Recipe::checksum_of("CLEAN;SC-1")
    }

    fn verified(aggregate: &mut LotAggregate) {
        aggregate.verify_recipe("RCP-CLEAN", &checksum(), &mut ids(), 0, "test");
    }

    fn wafer_ids(aggregate: &LotAggregate) -> Vec<WaferId> {
        aggregate.wafers.iter().map(|w| w.id.clone()).collect()
    }

    /// Load `wafers` on `equipment` at the lot's current step.
    fn track_in(
        lot: &mut LotAggregate,
        equipment: &str,
        wafers: &[WaferId],
        now_ms: i64,
    ) -> Result<Transition, TransitionError> {
        lot.track_in(
            TrackInReport {
                step_no: lot.lot.current_step,
                equipment_id: equipment,
                recipe_id: "RCP-CLEAN",
                wafers,
            },
            &mut ids(),
            now_ms,
            "sim",
        )
    }

    /// Finish the lot's current step on `equipment`.
    fn track_out(
        lot: &mut LotAggregate,
        equipment: &str,
        wafers: &[WaferId],
        now_ms: i64,
    ) -> Result<Transition, TransitionError> {
        lot.track_out(
            TrackOutReport {
                step_no: lot.lot.current_step,
                equipment_id: equipment,
                recipe_id: "RCP-CLEAN",
                recipe_checksum: &checksum(),
                wafers,
                duration_ms: 6000,
                measured: None,
            },
            &mut ids(),
            now_ms,
            "sim",
        )
    }

    // ------------------------------------------------------------------
    // the lifecycle contract
    // ------------------------------------------------------------------

    /// What a `(state, action)` pair did.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Outcome {
        /// The aggregate changed.
        Accepted,
        /// Accepted, but nothing about the lot or its wafers changed: a retry.
        Idempotent,
        /// Refused for a lifecycle reason: this state does not allow it.
        Refused,
    }

    /// Every action a tool or an operator can ask the aggregate to perform.
    #[derive(Debug, Clone, Copy)]
    enum Action {
        Queue,
        Hold,
        Release,
        Rework,
        TrackIn,
        TrackOut,
        Complete,
        ScrapWafer,
        VerifyRecipe,
        Split,
        Merge,
    }

    const STATES: [LotState; 8] = [
        LotState::Created,
        LotState::Queued,
        LotState::Processing,
        LotState::Waiting,
        LotState::OnHold,
        LotState::Completed,
        LotState::Split,
        LotState::Merged,
    ];

    /// The column `state` occupies in every row of [`LIFECYCLE`].
    ///
    /// A new `LotState` fails to compile here; that is the point, because the
    /// matrix must not silently gain a state nobody classified.
    const fn column(state: LotState) -> usize {
        match state {
            LotState::Created => 0,
            LotState::Queued => 1,
            LotState::Processing => 2,
            LotState::Waiting => 3,
            LotState::OnHold => 4,
            LotState::Completed => 5,
            LotState::Split => 6,
            LotState::Merged => 7,
        }
    }

    use Outcome::{Accepted as A, Idempotent as I, Refused as R};

    /// The lot lifecycle: one row per action, one cell per `STATES` column.
    ///
    /// `A` accepted (the aggregate changed), `I` accepted with no change (a
    /// retry), `R` refused for a lifecycle reason. A new `Action` fails to
    /// compile in [`observe`], so no action can go unclassified either.
    ///
    /// The aggregate under test is synthetic — four queued wafers, a verified
    /// recipe, the same current step — so a difference between two cells can
    /// only come from the lot's state. A few cells (a split lot that still has
    /// wafers) are unreachable in a running fab; they pin the guard, not a
    /// scenario. `scrap_wafer` and `verify_recipe` are lifecycle-independent on
    /// purpose: a quality disposition and a download record do not move a lot.
    #[rustfmt::skip]
    const LIFECYCLE: [(Action, [Outcome; 8]); 11] = [
        //                      C  Q  P  W  H  D  S  M
        (Action::Queue,        [A, I, A, A, R, R, R, R]),
        (Action::Hold,         [A, A, A, A, I, R, R, R]),
        (Action::Release,      [R, R, R, R, A, R, R, R]),
        (Action::Rework,       [A, A, R, A, A, R, R, R]),
        (Action::TrackIn,      [A, A, A, A, R, R, R, R]),
        (Action::TrackOut,     [A, A, A, A, A, R, R, R]),
        (Action::Complete,     [A, A, R, A, A, R, R, R]),
        (Action::ScrapWafer,   [A, A, A, A, A, A, A, A]),
        (Action::VerifyRecipe, [I, I, I, I, I, I, I, I]),
        (Action::Split,        [A, A, R, A, A, R, R, R]),
        (Action::Merge,        [A, A, R, A, A, R, R, R]),
    ];

    const STEP: i32 = 2;
    const NOW: i64 = 5_000;
    const ACTOR: &str = "operator";

    /// A lot in `state` whose data preconditions are all satisfied: four
    /// queued wafers, a verified recipe, the same current step.
    fn fixture(state: LotState) -> LotAggregate {
        let mut lot = agg("LOT-0100", 4);
        lot.lot.current_step = STEP;
        lot.lot.state = state;
        if state == LotState::OnHold {
            lot.lot.hold_reason = Some("engineer review".to_owned());
        }
        verified(&mut lot);
        lot
    }

    /// `true` when the refusal is about the lot's state rather than about the
    /// data in the request.
    fn is_lifecycle_refusal(error: &TransitionError) -> bool {
        matches!(
            error,
            TransitionError::IllegalState { .. }
                | TransitionError::AlreadyCompleted
                | TransitionError::NotOnHold
        )
    }

    /// Run `action` against a lot in `state` and classify what happened.
    ///
    /// Panics on a *data* refusal: the fixture satisfies every data
    /// precondition, so one means the fixture or the table is wrong, not the
    /// guard under test.
    fn observe(state: LotState, action: Action) -> Outcome {
        let mut lot = fixture(state);
        let before = lot.clone();
        let two = wafer_ids(&lot)[..2].to_vec();
        let outcome: Result<(), TransitionError> = match action {
            Action::Queue => lot.queue(STEP, &mut ids(), NOW, ACTOR).map(|_| ()),
            Action::Hold => lot
                .hold("engineer review", &mut ids(), NOW, ACTOR)
                .map(|_| ()),
            Action::Release => lot.release(&mut ids(), NOW, ACTOR).map(|_| ()),
            Action::Rework => lot
                .rework(STEP - 1, "scanner drift", &mut ids(), NOW, ACTOR)
                .map(|_| ()),
            Action::TrackIn => track_in(&mut lot, "CLN-01", &two, NOW).map(|_| ()),
            Action::TrackOut => track_out(&mut lot, "CLN-01", &two, NOW).map(|_| ()),
            Action::Complete => lot.complete(STEP, &mut ids(), NOW, ACTOR).map(|_| ()),
            Action::ScrapWafer => lot
                .scrap_wafer("LOT-0100-W01", "particle", &mut ids(), NOW, ACTOR)
                .map(|_| ()),
            Action::VerifyRecipe => {
                lot.verify_recipe("RCP-CLEAN", &checksum(), &mut ids(), NOW, ACTOR);
                Ok(())
            }
            Action::Split => {
                let children = ["LOT-0100-A".to_owned(), "LOT-0100-B".to_owned()];
                lot.split(&children, &mut ids(), NOW, ACTOR).map(|_| ())
            }
            Action::Merge => {
                let mut source = agg("LOT-0200", 2);
                source.lot.current_step = STEP;
                source.lot.state = LotState::Waiting;
                lot.merge(std::slice::from_mut(&mut source), &mut ids(), NOW, ACTOR)
                    .map(|_| ())
            }
        };
        match outcome {
            Err(error) if is_lifecycle_refusal(&error) => Outcome::Refused,
            Err(error) => panic!(
                "({state:?}, {action:?}) was refused for a data reason ({error}); \
                 the fixture must satisfy every data precondition"
            ),
            Ok(()) if lot == before => Outcome::Idempotent,
            Ok(()) => Outcome::Accepted,
        }
    }

    #[test]
    fn the_transition_matrix_is_the_lifecycle_contract() {
        for (action, row) in LIFECYCLE {
            for state in STATES {
                assert_eq!(
                    observe(state, action),
                    row[column(state)],
                    "({state:?}, {action:?})"
                );
            }
        }
    }

    #[test]
    fn split_distributes_wafers_and_ends_the_parent() {
        let mut parent = agg("LOT-0001", 10);
        parent.lot.state = LotState::Waiting;
        let children = ["LOT-0001-A", "LOT-0001-B", "LOT-0001-C"];
        let transition = parent
            .split(&children.map(str::to_owned), &mut ids(), 1000, "operator")
            .unwrap();

        assert_eq!(parent.lot.state, LotState::Split);
        assert!(
            parent.wafers.is_empty(),
            "the parent keeps no wafers, still holds {:?}",
            parent.wafers
        );
        assert_eq!(transition.children.len(), 3);
        let sizes: Vec<i32> = transition
            .children
            .iter()
            .map(|c| c.lot.wafer_count)
            .collect();
        assert_eq!(sizes, vec![4, 3, 3]);
        for child in &transition.children {
            assert_eq!(child.lot.parent_lot_id.as_deref(), Some("LOT-0001"));
            assert!(child.wafers.iter().all(|w| w.lot_id == child.lot.id));
        }
        assert_eq!(transition.events[0].kind, EventKind::LotSplit);
    }

    #[test]
    fn merge_combines_two_lots_into_one() {
        let mut target = agg("LOT-0002", 5);
        target.lot.current_step = 2;
        target.lot.state = LotState::Waiting;
        let mut source = agg("LOT-0003", 5);
        source.lot.current_step = 2;
        source.lot.state = LotState::Waiting;

        let transition = target
            .merge(
                std::slice::from_mut(&mut source),
                &mut ids(),
                2000,
                "operator",
            )
            .unwrap();

        assert_eq!(source.lot.state, LotState::Merged);
        assert_eq!(target.lot.wafer_count, 10);
        assert_eq!(target.wafers.len(), 10);
        let slots: Vec<i32> = target.wafers.iter().map(|w| w.slot).collect();
        assert_eq!(slots, (1..=10).collect::<Vec<_>>());
        assert!(target.wafers.iter().all(|w| w.lot_id == "LOT-0002"));
        assert_eq!(transition.events[0].kind, EventKind::LotMerged);
    }

    #[test]
    fn merge_refuses_lots_at_different_steps() {
        let mut target = agg("LOT-0002", 5);
        target.lot.current_step = 2;
        target.lot.state = LotState::Waiting;
        let mut source = agg("LOT-0009", 5);
        source.lot.current_step = 3;
        source.lot.state = LotState::Waiting;
        let error = target
            .merge(std::slice::from_mut(&mut source), &mut ids(), 1, "operator")
            .unwrap_err();
        assert_eq!(error, TransitionError::DifferentStep { other: 3 });
    }

    #[test]
    fn rework_moves_the_lot_back_and_clears_completed_steps() {
        let mut lot = agg("LOT-0004", 4);
        lot.lot.current_step = 3;
        lot.lot.state = LotState::Waiting;
        lot.completed.insert(1, wafer_ids(&lot));
        lot.completed.insert(2, wafer_ids(&lot));
        lot.completed.insert(3, wafer_ids(&lot));

        let transition = lot
            .rework(2, "scanner drift", &mut ids(), 3000, "engineer")
            .unwrap();

        assert_eq!(lot.lot.current_step, 2);
        assert_eq!(lot.lot.state, LotState::Waiting);
        assert!(lot.wafers.iter().all(|w| w.rework_count == 1));
        assert_eq!(lot.completed.keys().copied().collect::<Vec<_>>(), vec![1]);
        assert_eq!(transition.events[0].kind, EventKind::ReworkStarted);
        assert!(matches!(
            lot.rework(3, "forward", &mut ids(), 1, "x"),
            Err(TransitionError::ReworkTargetNotEarlier { .. })
        ));
    }

    #[test]
    fn hold_blocks_dispatch_until_released() {
        let mut lot = agg("LOT-0005", 2);
        let held = lot
            .hold("metal contamination", &mut ids(), 10, "operator")
            .unwrap();
        assert_eq!(held.events[0].kind, EventKind::LotHeld);
        assert_eq!(lot.lot.state, LotState::OnHold);
        assert_eq!(lot.lot.hold_reason.as_deref(), Some("metal contamination"));
        assert!(matches!(
            lot.queue(1, &mut ids(), 11, "dispatcher"),
            Err(TransitionError::IllegalState { .. })
        ));

        let released = lot.release(&mut ids(), 12, "operator").unwrap();
        assert_eq!(released.events[0].kind, EventKind::LotReleased);
        assert_eq!(lot.lot.state, LotState::Waiting);
        assert!(lot.lot.hold_reason.is_none());
        lot.queue(1, &mut ids(), 13, "dispatcher").unwrap();
        assert_eq!(lot.lot.state, LotState::Queued);
    }

    #[test]
    fn a_partial_lot_finishes_its_step_in_two_loads() {
        let mut lot = agg("LOT-0006", 10);
        verified(&mut lot);
        let first = wafer_ids(&lot)[..5].to_vec();
        let second = wafer_ids(&lot)[5..].to_vec();

        track_in(&mut lot, "CLN-01", &first, 100).unwrap();
        track_out(&mut lot, "CLN-01", &first, 6100).unwrap();

        assert_eq!(lot.lot.state, LotState::Waiting);
        assert!(!lot.step_finished(1));

        lot.queue(1, &mut ids(), 6200, "sim").unwrap();
        track_in(&mut lot, "CLN-02", &second, 6300).unwrap();
        track_out(&mut lot, "CLN-02", &second, 12300).unwrap();

        assert!(lot.step_finished(1));
        assert_eq!(lot.active_count(), 10);
    }

    #[test]
    fn a_duplicate_track_out_changes_nothing() {
        let mut lot = agg("LOT-0007", 2);
        verified(&mut lot);
        let all = wafer_ids(&lot);
        track_in(&mut lot, "CLN-01", &all, 100).unwrap();
        track_out(&mut lot, "CLN-01", &all, 6100).unwrap();
        let state_after_first = lot.lot.clone();
        let wafers_after_first = lot.wafers.clone();

        let retry = track_out(&mut lot, "CLN-01", &all, 6200).unwrap();

        assert_eq!(retry.events.len(), 1);
        assert_eq!(retry.events[0].kind, EventKind::TrackOutDuplicate);
        assert_eq!(lot.lot, state_after_first);
        assert_eq!(lot.wafers, wafers_after_first);
    }

    #[test]
    fn a_track_out_before_verifying_the_recipe_is_refused() {
        let mut lot = agg("LOT-0008", 2);
        let all = wafer_ids(&lot);
        let error = track_in(&mut lot, "CLN-01", &all, 1).unwrap_err();
        assert_eq!(
            error,
            TransitionError::RecipeNotVerified("RCP-CLEAN".into())
        );
    }

    #[test]
    fn scrapping_one_wafer_keeps_the_rest_of_the_lot_alive() {
        let mut lot = agg("LOT-0010", 3);
        lot.lot.current_step = 4;
        lot.lot.state = LotState::Waiting;
        lot.scrap_wafer("LOT-0010-W02", "particle", &mut ids(), 1, "engineer")
            .unwrap();
        assert_eq!(lot.active_count(), 2);
        assert_eq!(lot.lot.state, LotState::Waiting);
        assert_eq!(
            lot.wafers
                .iter()
                .find(|w| w.id == "LOT-0010-W02")
                .unwrap()
                .defect_info
                .as_deref(),
            Some("particle")
        );
    }
}
