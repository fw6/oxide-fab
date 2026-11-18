//! The floor state the scene draws, mirroring the API's snapshot.
//!
//! These are deliberately *plain* structs: the UI maps the wire types onto them
//! and `build` never reaches back into HTTP. Everything is read-only.

#[derive(Debug, Clone, PartialEq, Default)]
pub struct FloorState {
    pub now_ms: i64,
    pub running: bool,
    pub speed: f64,
    pub steps: Vec<StepState>,
    pub tools: Vec<ToolState>,
    pub lots: Vec<LotState>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct StepState {
    pub step_no: i32,
    pub name: String,
    pub equipment_kind: String,
    pub queue_len: usize,
    pub busy_tools: usize,
    pub total_tools: usize,
    pub current_lots: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ToolState {
    pub id: String,
    pub kind: String,
    /// `running` / `idle` / `down` / `maintenance`, as the API reports it.
    pub state: String,
    /// `green` / `amber` / `red`.
    pub light: String,
    pub current_lot: Option<String>,
    pub current_step: Option<i32>,
    pub lots_processed: u64,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct LotState {
    pub id: String,
    pub state: String,
    pub current_step: i32,
    pub priority: i32,
    pub wafer_count: usize,
    pub done_wafers: usize,
    pub hold_reason: Option<String>,
    /// When this step was entered, in the demo clock. `0` means the UI has not
    /// seen the transition yet, and the FOUP is placed without interpolation.
    pub step_since_ms: i64,
    pub wafers: Vec<WaferState>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct WaferState {
    pub id: String,
    pub slot: i32,
    pub state: String,
    pub defect_info: Option<String>,
    pub rework_count: u32,
}

impl FloorState {
    #[must_use]
    pub fn tool(&self, equipment_id: &str) -> Option<&ToolState> {
        self.tools.iter().find(|tool| tool.id == equipment_id)
    }

    #[must_use]
    pub fn lot(&self, lot_id: &str) -> Option<&LotState> {
        self.lots.iter().find(|lot| lot.id == lot_id)
    }

    /// The lots queued for a step, in the sim's FIFO order.
    #[must_use]
    pub fn queue(&self, step_no: i32) -> Vec<&LotState> {
        self.lots
            .iter()
            .filter(|lot| lot.current_step == step_no && lot.state == "queued")
            .collect()
    }
}
