//! Domain entities: routes, recipes, equipment, lots and wafers.
//!
//! The PostgreSQL tables in `migrations/` mirror these types one to one; the
//! types themselves are what the state machine and the simulator reason about.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::string_enum::string_enum;

pub type LotId = String;
pub type WaferId = String;

/// Where a lot is in its lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LotState {
    /// Created, not yet queued for its first step.
    Created,
    /// Waiting in a step queue for a qualified tool.
    Queued,
    /// Loaded on a tool, being processed.
    Processing,
    /// Finished a step, waiting to be dispatched to the next one.
    Waiting,
    /// Stopped by an operator; no dispatch until released.
    OnHold,
    /// Every wafer finished every step.
    Completed,
    /// Ended by splitting into child lots.
    Split,
    /// Ended by being merged into another lot.
    Merged,
}

string_enum!(LotState {
    Created => "created",
    Queued => "queued",
    Processing => "processing",
    Waiting => "waiting",
    OnHold => "on_hold",
    Completed => "completed",
    Split => "split",
    Merged => "merged",
});

impl LotState {
    /// `true` once the lot's life has ended for good: completed, split into
    /// children, or merged into another lot.
    ///
    /// No process action may move a lot out of these states; the state
    /// machine's guards all consult this one predicate, and
    /// `tests::the_transition_matrix_is_the_lifecycle_contract` pins it.
    #[must_use]
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            LotState::Completed | LotState::Split | LotState::Merged
        )
    }
}

/// Where one wafer is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaferState {
    Created,
    Queued,
    Processing,
    Waiting,
    Done,
    Scrapped,
}

string_enum!(WaferState {
    Created => "created",
    Queued => "queued",
    Processing => "processing",
    Waiting => "waiting",
    Done => "done",
    Scrapped => "scrapped",
});

/// One wafer of a lot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Wafer {
    pub id: WaferId,
    pub lot_id: LotId,
    pub slot: i32,
    pub state: WaferState,
    pub defect_info: Option<String>,
    pub rework_count: u32,
}

impl Wafer {
    pub fn new(lot_id: impl Into<LotId>, slot: i32) -> Self {
        let lot_id = lot_id.into();
        Self {
            id: format!("{lot_id}-W{slot:02}"),
            lot_id,
            slot,
            state: WaferState::Created,
            defect_info: None,
            rework_count: 0,
        }
    }
}

/// The material a lot is built from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Product {
    pub id: String,
    pub name: String,
    pub wafer_size_mm: i32,
}

/// One step of a route.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RouteStep {
    pub step_no: i32,
    pub name: String,
    /// The kind of tool this step needs (`clean`, `litho`, ...).
    pub equipment_kind: String,
    /// The tools of that kind that are qualified to run the step, by id.
    ///
    /// Filled from the scenario's equipment when the file leaves it out, so
    /// the route step carries the qualification the dispatcher checks.
    #[serde(default)]
    pub equipment_qualification: Vec<String>,
    pub recipe_id: String,
    pub duration_ms: i64,
    /// Nominal measured value, for metrology steps.
    pub target: Option<f64>,
    /// Upper control limit of the measured value, for metrology steps.
    pub ucl: Option<f64>,
    /// Lower control limit of the measured value, for metrology steps.
    pub lcl: Option<f64>,
}

impl RouteStep {
    #[must_use]
    pub fn is_metrology(&self) -> bool {
        self.ucl.is_some() && self.lcl.is_some()
    }
}

/// An ordered set of steps a lot follows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Route {
    pub id: String,
    pub name: String,
    pub steps: Vec<RouteStep>,
}

impl Route {
    #[must_use]
    pub fn step(&self, step_no: i32) -> Option<&RouteStep> {
        self.steps.iter().find(|s| s.step_no == step_no)
    }

    #[must_use]
    pub fn first_step(&self) -> i32 {
        self.steps.first().map_or(1, |s| s.step_no)
    }

    #[must_use]
    pub fn next_step(&self, step_no: i32) -> Option<i32> {
        self.steps
            .iter()
            .find(|s| s.step_no > step_no)
            .map(|s| s.step_no)
    }

    #[must_use]
    pub fn last_step(&self) -> i32 {
        self.steps.last().map_or(1, |s| s.step_no)
    }
}

/// A process program. Its checksum is verified before every download.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe {
    pub id: String,
    pub version: i32,
    pub checksum: String,
    pub body: String,
}

impl Recipe {
    pub fn new(id: impl Into<String>, version: i32, body: impl Into<String>) -> Self {
        let body = body.into();
        Self {
            id: id.into(),
            version,
            checksum: Self::checksum_of(&body),
            body,
        }
    }

    /// SHA-256 of the program body, lowercase hex.
    #[must_use]
    pub fn checksum_of(body: &str) -> String {
        use sha2::{Digest, Sha256};
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let digest = Sha256::digest(body.as_bytes());
        let mut checksum = String::with_capacity(digest.len() * 2);
        for byte in digest {
            checksum.push(char::from(HEX[usize::from(byte >> 4)]));
            checksum.push(char::from(HEX[usize::from(byte & 0x0f)]));
        }
        checksum
    }

    /// `true` when `checksum` matches the body. A mismatch must abort the
    /// download: an unverified recipe never reaches a tool.
    #[must_use]
    pub fn verify(&self, checksum: &str) -> bool {
        self.checksum == checksum && Self::checksum_of(&self.body) == checksum
    }
}

/// What a tool is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EquipmentState {
    Idle,
    Running,
    Down,
    Maintenance,
}

string_enum!(EquipmentState {
    Idle => "idle",
    Running => "running",
    Down => "down",
    Maintenance => "maintenance",
});

/// One simulated tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Equipment {
    pub id: String,
    pub kind: String,
    pub state: EquipmentState,
    pub chamber: String,
    /// Recipe ids this chamber is qualified to run (the qualification matrix).
    pub qualification: Vec<String>,
}

impl Equipment {
    #[must_use]
    pub fn is_available(&self) -> bool {
        matches!(self.state, EquipmentState::Idle)
    }

    #[must_use]
    pub fn qualified_for(&self, recipe_id: &str) -> bool {
        self.qualification.iter().any(|q| q == recipe_id)
            || self.qualification.iter().any(|q| q == "*")
    }
}

/// A production lot: a carrier of wafers that travels the route together.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lot {
    pub id: LotId,
    pub product_id: String,
    pub route_id: String,
    pub priority: i32,
    pub state: LotState,
    pub hold_reason: Option<String>,
    pub current_step: i32,
    pub wafer_count: i32,
    pub created_at_ms: i64,
    /// Set when the lot was produced by splitting another lot.
    pub parent_lot_id: Option<LotId>,
}

impl Lot {
    pub fn new(
        id: impl Into<LotId>,
        product_id: impl Into<String>,
        route_id: impl Into<String>,
        priority: i32,
        wafer_count: i32,
        created_at_ms: i64,
    ) -> Self {
        Self {
            id: id.into(),
            product_id: product_id.into(),
            route_id: route_id.into(),
            priority,
            state: LotState::Created,
            hold_reason: None,
            current_step: 0,
            wafer_count,
            created_at_ms,
            parent_lot_id: None,
        }
    }
}

/// A deterministic lot id generator: `LOT-0001`.
#[must_use]
pub fn lot_id_for(serial: u64) -> String {
    format!("LOT-{serial:04}")
}

/// A deterministic event id from the simulator clock, the lot and a counter.
///
/// Events keep their id across replays so the time series store can dedupe
/// them; they are UUID v4-shaped but derived, never random.
#[must_use]
pub fn deterministic_event_id(seed: u64, counter: u64) -> Uuid {
    let mut bytes = [0u8; 16];
    bytes[0..8].copy_from_slice(&seed.to_be_bytes());
    bytes[8..16].copy_from_slice(&counter.to_be_bytes());
    // Set the version (4) and variant (RFC 4122) bits.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}
