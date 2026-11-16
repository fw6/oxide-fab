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
use serde_json::{json, Value};
use uuid::Uuid;

use crate::events::{
    equipment_down_payload, equipment_up_payload, lot_created_payload, lot_queued_payload,
    merged_payload, split_payload, track_in_payload, track_out_payload, DomainEvent, EventKind,
};
use crate::model::{Lot, LotId, LotState, Wafer, WaferId, WaferState};

/// Allocate the event ids of one transition deterministically.
#[derive(Debug, Clone)]
pub struct EventIds {
    base: Uuid,
    offset: u32,
}

impl EventIds {
    pub fn new(base: Uuid) -> Self {
        Self { base, offset: 0 }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Uuid {
        let id = Uuid::from_u128(self.base.as_u128().wrapping_add(self.offset as u128));
        self.offset += 1;
        id
    }

    /// How many ids this set handed out, so the caller can advance its counter
    /// past them (ids stay unique across a restart).
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
    pub fn new(lot: Lot, wafers: Vec<Wafer>) -> Self {
        Self {
            lot,
            wafers,
            completed: HashMap::new(),
            verified_recipes: HashMap::new(),
        }
    }

    /// The highest step every active wafer has finished, if any.
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
            ids.next(),
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
        if matches!(
            self.lot.state,
            LotState::Completed | LotState::Split | LotState::Merged
        ) {
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
                ids.next(),
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
        if self.lot.state == LotState::Completed {
            return Err(TransitionError::AlreadyCompleted);
        }
        if self.lot.state == LotState::OnHold {
            return Ok(Transition::default());
        }
        self.lot.state = LotState::OnHold;
        self.lot.hold_reason = Some(reason.to_owned());
        Ok(Transition {
            events: vec![DomainEvent::new(
                ids.next(),
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
                ids.next(),
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
        match self.lot.state {
            LotState::Processing => {
                return Err(TransitionError::IllegalState {
                    from: self.lot.state,
                    action: "rework",
                })
            }
            LotState::Completed => return Err(TransitionError::AlreadyCompleted),
            _ => {}
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
                ids.next(),
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
    #[allow(clippy::too_many_arguments)]
    pub fn track_in(
        &mut self,
        step_no: i32,
        equipment_id: &str,
        recipe_id: &str,
        wafers: &[WaferId],
        ids: &mut EventIds,
        now_ms: i64,
        actor: &str,
    ) -> Result<Transition, TransitionError> {
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
                ids.next(),
                self.lot.id.clone(),
                EventKind::TrackIn,
                track_in_payload(step_no, equipment_id, recipe_id, wafers),
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
    #[allow(clippy::too_many_arguments)]
    pub fn track_out(
        &mut self,
        step_no: i32,
        equipment_id: &str,
        recipe_id: &str,
        recipe_checksum: &str,
        wafers: &[WaferId],
        duration_ms: i64,
        measured: Option<f64>,
        ids: &mut EventIds,
        now_ms: i64,
        actor: &str,
    ) -> Result<Transition, TransitionError> {
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
            .map(|done| done.iter().collect())
            .unwrap_or_default();

        if selected.iter().all(|id| already.contains(id)) {
            return Ok(Transition {
                events: vec![DomainEvent::new(
                    ids.next(),
                    self.lot.id.clone(),
                    EventKind::TrackOutDuplicate,
                    json!({
                        "step_no": step_no,
                        "equipment_id": equipment_id,
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
                ids.next(),
                self.lot.id.clone(),
                EventKind::TrackOut,
                track_out_payload(
                    step_no,
                    equipment_id,
                    recipe_id,
                    recipe_checksum,
                    wafers,
                    duration_ms,
                    measured,
                ),
                now_ms,
                actor,
            )],
            children: vec![],
        })
    }

    /// `true` when every active wafer finished `step_no`.
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
        if self.lot.state == LotState::Completed {
            return Err(TransitionError::AlreadyCompleted);
        }
        for wafer in &mut self.wafers {
            if wafer.state != WaferState::Scrapped {
                wafer.state = WaferState::Done;
            }
        }
        self.lot.state = LotState::Completed;
        Ok(Transition {
            events: vec![DomainEvent::new(
                ids.next(),
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
                ids.next(),
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
                ids.next(),
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
        if matches!(
            self.lot.state,
            LotState::Completed | LotState::Processing | LotState::Split | LotState::Merged
        ) {
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
                wafer.lot_id = child_id.clone();
                wafer.slot = slot as i32 + 1;
                wafer.state = match self.lot.state {
                    LotState::Queued => WaferState::Queued,
                    _ => WaferState::Waiting,
                };
            }
            let mut child_lot = self.lot.clone();
            child_lot.id = child_id.clone();
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
            ids.next(),
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
        if matches!(self.lot.state, LotState::Processing | LotState::Completed) {
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
            if source.lot.state == LotState::Processing {
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
            for wafer in source.wafers.iter_mut() {
                if wafer.state == WaferState::Scrapped {
                    continue;
                }
                wafer.lot_id = self.lot.id.clone();
                wafer.slot = next_slot;
                wafer.state = WaferState::Waiting;
                next_slot += 1;
                self.wafers.push(wafer.clone());
            }
            source.lot.state = LotState::Merged;
            source.lot.wafer_count = 0;
            events.push(DomainEvent::new(
                ids.next(),
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

    #[test]
    fn split_distributes_wafers_and_ends_the_parent() {
        let mut parent = agg("LOT-0001", 10);
        parent.lot.state = LotState::Waiting;
        let children = ["LOT-0001-A", "LOT-0001-B", "LOT-0001-C"];
        let transition = parent
            .split(&children.map(str::to_owned), &mut ids(), 1000, "operator")
            .unwrap();

        assert_eq!(parent.lot.state, LotState::Split);
        assert!(parent.wafers.is_empty());
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

        lot.track_in(1, "CLN-01", "RCP-CLEAN", &first, &mut ids(), 100, "sim")
            .unwrap();
        lot.track_out(
            1,
            "CLN-01",
            "RCP-CLEAN",
            &checksum(),
            &first,
            6000,
            None,
            &mut ids(),
            6100,
            "sim",
        )
        .unwrap();

        assert_eq!(lot.lot.state, LotState::Waiting);
        assert!(!lot.step_finished(1));

        lot.queue(1, &mut ids(), 6200, "sim").unwrap();
        lot.track_in(1, "CLN-02", "RCP-CLEAN", &second, &mut ids(), 6300, "sim")
            .unwrap();
        lot.track_out(
            1,
            "CLN-02",
            "RCP-CLEAN",
            &checksum(),
            &second,
            6000,
            None,
            &mut ids(),
            12300,
            "sim",
        )
        .unwrap();

        assert!(lot.step_finished(1));
        assert_eq!(lot.active_count(), 10);
    }

    #[test]
    fn a_duplicate_track_out_changes_nothing() {
        let mut lot = agg("LOT-0007", 2);
        verified(&mut lot);
        let all = wafer_ids(&lot);
        lot.track_in(1, "CLN-01", "RCP-CLEAN", &all, &mut ids(), 100, "sim")
            .unwrap();
        lot.track_out(
            1,
            "CLN-01",
            "RCP-CLEAN",
            &checksum(),
            &all,
            6000,
            None,
            &mut ids(),
            6100,
            "sim",
        )
        .unwrap();
        let state_after_first = lot.lot.clone();
        let wafers_after_first = lot.wafers.clone();

        let retry = lot
            .track_out(
                1,
                "CLN-01",
                "RCP-CLEAN",
                &checksum(),
                &all,
                6000,
                None,
                &mut ids(),
                6200,
                "sim",
            )
            .unwrap();

        assert_eq!(retry.events.len(), 1);
        assert_eq!(retry.events[0].kind, EventKind::TrackOutDuplicate);
        assert_eq!(lot.lot, state_after_first);
        assert_eq!(lot.wafers, wafers_after_first);
    }

    #[test]
    fn a_track_out_before_verifying_the_recipe_is_refused() {
        let mut lot = agg("LOT-0008", 2);
        let all = wafer_ids(&lot);
        let error = lot
            .track_in(1, "CLN-01", "RCP-CLEAN", &all, &mut ids(), 1, "sim")
            .unwrap_err();
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
