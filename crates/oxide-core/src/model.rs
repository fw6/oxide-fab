//! Domain entities: routes, recipes, equipment, lots and wafers.
//!
//! The PostgreSQL tables in `migrations/` mirror these types one to one; the
//! types themselves are what the state machine and the simulator reason about.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

impl LotState {
    pub fn as_str(self) -> &'static str {
        match self {
            LotState::Created => "created",
            LotState::Queued => "queued",
            LotState::Processing => "processing",
            LotState::Waiting => "waiting",
            LotState::OnHold => "on_hold",
            LotState::Completed => "completed",
            LotState::Split => "split",
            LotState::Merged => "merged",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "created" => LotState::Created,
            "queued" => LotState::Queued,
            "processing" => LotState::Processing,
            "waiting" => LotState::Waiting,
            "on_hold" => LotState::OnHold,
            "completed" => LotState::Completed,
            "split" => LotState::Split,
            "merged" => LotState::Merged,
            _ => return None,
        })
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

impl WaferState {
    pub fn as_str(self) -> &'static str {
        match self {
            WaferState::Created => "created",
            WaferState::Queued => "queued",
            WaferState::Processing => "processing",
            WaferState::Waiting => "waiting",
            WaferState::Done => "done",
            WaferState::Scrapped => "scrapped",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "created" => WaferState::Created,
            "queued" => WaferState::Queued,
            "processing" => WaferState::Processing,
            "waiting" => WaferState::Waiting,
            "done" => WaferState::Done,
            "scrapped" => WaferState::Scrapped,
            _ => return None,
        })
    }
}

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
    pub fn step(&self, step_no: i32) -> Option<&RouteStep> {
        self.steps.iter().find(|s| s.step_no == step_no)
    }

    pub fn first_step(&self) -> i32 {
        self.steps.first().map(|s| s.step_no).unwrap_or(1)
    }

    pub fn next_step(&self, step_no: i32) -> Option<i32> {
        self.steps
            .iter()
            .find(|s| s.step_no > step_no)
            .map(|s| s.step_no)
    }

    pub fn last_step(&self) -> i32 {
        self.steps.last().map(|s| s.step_no).unwrap_or(1)
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
    pub fn checksum_of(body: &str) -> String {
        use sha2::{Digest, Sha256};
        let digest = Sha256::digest(body.as_bytes());
        digest.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// `true` when `checksum` matches the body. A mismatch must abort the
    /// download: an unverified recipe never reaches a tool.
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

impl EquipmentState {
    pub fn as_str(self) -> &'static str {
        match self {
            EquipmentState::Idle => "idle",
            EquipmentState::Running => "running",
            EquipmentState::Down => "down",
            EquipmentState::Maintenance => "maintenance",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "idle" => EquipmentState::Idle,
            "running" => EquipmentState::Running,
            "down" => EquipmentState::Down,
            "maintenance" => EquipmentState::Maintenance,
            _ => return None,
        })
    }
}

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
    pub fn is_available(&self) -> bool {
        matches!(self.state, EquipmentState::Idle)
    }

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
    #[allow(clippy::too_many_arguments)]
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

    pub fn id(&self) -> &str {
        &self.id
    }
}

/// A deterministic lot id generator: `LOT-0001`.
pub fn lot_id_for(serial: u64) -> String {
    format!("LOT-{serial:04}")
}

/// A deterministic event id from the simulator clock, the lot and a counter.
///
/// Events keep their id across replays so the time series store can dedupe
/// them; they are UUID v4-shaped but derived, never random.
pub fn deterministic_event_id(seed: u64, counter: u64) -> Uuid {
    let mut bytes = [0u8; 16];
    bytes[0..8].copy_from_slice(&seed.to_be_bytes());
    bytes[8..16].copy_from_slice(&counter.to_be_bytes());
    // Set the version (4) and variant (RFC 4122) bits.
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}
