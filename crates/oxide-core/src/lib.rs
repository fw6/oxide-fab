//! Pure domain model for the oxide-fab simulation.
//!
//! Everything in this crate is deterministic and free of I/O: the state
//! machine, the event vocabulary, the metric derivation that feeds the time
//! series store, and the scenario description. Persistence lives in
//! `oxide-store`, the discrete-event engine in `oxide-sim`.

pub mod events;
pub mod genealogy;
pub mod metrics;
pub mod model;
pub mod scenario;
pub mod spc;
pub mod state_machine;

pub use events::{DomainEvent, EventKind};
pub use model::{
    Equipment, EquipmentState, Lot, LotState, Recipe, Route, RouteStep, Wafer, WaferState,
};
pub use scenario::Scenario;
pub use state_machine::{LotAggregate, Transition, TransitionError};
