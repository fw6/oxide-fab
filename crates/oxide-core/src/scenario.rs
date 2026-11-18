//! The scenario file: what the simulated fab is made of.
//!
//! `config/scenario.json` describes products, the route, recipes, the tools
//! and how much material is released. It is data, not code: changing the
//! scenario changes the demo without a rebuild.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::model::{Equipment, Product, Recipe, Route};

/// One tool on the floor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquipmentSpec {
    pub id: String,
    pub kind: String,
    pub chamber: String,
    /// Recipe ids this chamber may run.
    pub qualification: Vec<String>,
}

impl EquipmentSpec {
    #[must_use]
    pub fn to_equipment(&self) -> Equipment {
        Equipment {
            id: self.id.clone(),
            kind: self.kind.clone(),
            state: crate::model::EquipmentState::Idle,
            chamber: self.chamber.clone(),
            qualification: self.qualification.clone(),
        }
    }
}

/// A recipe as written in the scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeSpec {
    pub id: String,
    pub version: i32,
    pub body: String,
}

impl RecipeSpec {
    #[must_use]
    pub fn to_recipe(&self) -> Recipe {
        Recipe::new(&self.id, self.version, &self.body)
    }
}

/// Everything the simulator needs to run the fab.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    pub name: String,
    /// Seed of the deterministic failure RNG.
    pub seed: u64,
    pub product: Product,
    pub route: Route,
    pub recipes: Vec<RecipeSpec>,
    pub equipment: Vec<EquipmentSpec>,
    /// How many lots are released in total.
    pub lot_count: usize,
    /// Wafers per lot.
    pub wafers_per_lot: i32,
    /// Material release interval, in simulated milliseconds.
    pub interarrival_ms: i64,
    /// Simulated milliseconds advanced per engine tick (before `speed`).
    pub tick_ms: i64,
    /// Probability that a tool goes down while loading a lot.
    pub failure_probability: f64,
    /// How long a down tool takes to repair, in simulated milliseconds.
    pub repair_ms: i64,
    /// Store a state snapshot every N persisted events.
    pub snapshot_every_events: u64,
    /// Default playback speed (1.0 = real time).
    pub speed: f64,
}

impl Scenario {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        let mut scenario: Scenario = serde_json::from_str(json)?;
        scenario.fill_qualifications();
        Ok(scenario)
    }

    /// Give every step its qualified tool ids when the file leaves them out.
    ///
    /// `route_step.equipment_qualification` is what the dispatcher checks, so
    /// it must never be empty for a runnable route.
    fn fill_qualifications(&mut self) {
        let equipment = self.equipment.clone();
        for step in &mut self.route.steps {
            if !step.equipment_qualification.is_empty() {
                continue;
            }
            step.equipment_qualification = equipment
                .iter()
                .filter(|tool| {
                    tool.kind == step.equipment_kind
                        && tool
                            .qualification
                            .iter()
                            .any(|recipe| recipe == &step.recipe_id)
                })
                .map(|tool| tool.id.clone())
                .collect();
        }
    }

    pub fn recipes(&self) -> Vec<Recipe> {
        self.recipes.iter().map(RecipeSpec::to_recipe).collect()
    }

    pub fn equipment(&self) -> Vec<Equipment> {
        self.equipment
            .iter()
            .map(EquipmentSpec::to_equipment)
            .collect()
    }

    pub fn recipe(&self, id: &str) -> Option<Recipe> {
        self.recipes
            .iter()
            .find(|r| r.id == id)
            .map(RecipeSpec::to_recipe)
    }

    /// Check the scenario is coherent before the fab starts.
    ///
    /// Returns the first inconsistency found: a route step without a recipe or
    /// a qualified tool, or a non-positive wafer count.
    pub fn validate(&self) -> Result<(), ScenarioError> {
        if self.route.steps.is_empty() {
            return Err(ScenarioError::NoSteps);
        }
        for step in &self.route.steps {
            if self.recipe(&step.recipe_id).is_none() {
                return Err(ScenarioError::UnknownRecipe {
                    step_no: step.step_no,
                    recipe_id: step.recipe_id.clone(),
                });
            }
            if step.equipment_qualification.is_empty() {
                return Err(ScenarioError::NoQualifiedTool {
                    step_no: step.step_no,
                    equipment_kind: step.equipment_kind.clone(),
                    recipe_id: step.recipe_id.clone(),
                });
            }
            for tool_id in &step.equipment_qualification {
                let tool = self
                    .equipment
                    .iter()
                    .find(|tool| &tool.id == tool_id)
                    .ok_or_else(|| ScenarioError::UnknownTool {
                        step_no: step.step_no,
                        tool_id: tool_id.clone(),
                    })?;
                if tool.kind != step.equipment_kind || !tool.qualification.contains(&step.recipe_id)
                {
                    return Err(ScenarioError::UnqualifiedTool {
                        step_no: step.step_no,
                        tool_id: tool_id.clone(),
                    });
                }
            }
        }
        if self.wafers_per_lot <= 0 {
            return Err(ScenarioError::NonPositiveWaferCount {
                wafers_per_lot: self.wafers_per_lot,
            });
        }
        Ok(())
    }
}

/// Why a scenario cannot be run.
///
/// A typed error, not a `String`: the server logs it and the API reports it,
/// and a caller that wants to react to "unknown recipe" should not have to
/// match on message text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScenarioError {
    NoSteps,
    UnknownRecipe {
        step_no: i32,
        recipe_id: String,
    },
    NoQualifiedTool {
        step_no: i32,
        equipment_kind: String,
        recipe_id: String,
    },
    UnknownTool {
        step_no: i32,
        tool_id: String,
    },
    UnqualifiedTool {
        step_no: i32,
        tool_id: String,
    },
    NonPositiveWaferCount {
        wafers_per_lot: i32,
    },
}

impl fmt::Display for ScenarioError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScenarioError::NoSteps => write!(f, "route has no steps"),
            ScenarioError::UnknownRecipe { step_no, recipe_id } => {
                write!(f, "step {step_no} references unknown recipe {recipe_id}")
            }
            ScenarioError::NoQualifiedTool {
                step_no,
                equipment_kind,
                recipe_id,
            } => write!(
                f,
                "no tool of kind {equipment_kind} is qualified for {recipe_id} (step {step_no})"
            ),
            ScenarioError::UnknownTool { step_no, tool_id } => {
                write!(f, "step {step_no} names unknown tool {tool_id}")
            }
            ScenarioError::UnqualifiedTool { step_no, tool_id } => {
                write!(f, "tool {tool_id} is not qualified for step {step_no}")
            }
            ScenarioError::NonPositiveWaferCount { wafers_per_lot } => {
                write!(f, "wafers_per_lot must be positive, is {wafers_per_lot}")
            }
        }
    }
}

impl std::error::Error for ScenarioError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Scenario {
        Scenario::from_json(include_str!("../../../config/scenario.json")).unwrap()
    }

    #[test]
    fn the_shipped_scenario_is_valid_and_has_four_steps() {
        let scenario = sample();
        scenario.validate().unwrap();
        assert_eq!(scenario.route.steps.len(), 4);
        let names: Vec<&str> = scenario
            .route
            .steps
            .iter()
            .map(|s| s.name.as_str())
            .collect();
        assert_eq!(names, ["clean", "litho", "etch", "metrology"]);
    }

    #[test]
    fn every_step_names_the_tools_qualified_for_it() {
        let scenario = sample();
        let steps = &scenario.route.steps;
        assert_eq!(steps[0].equipment_qualification, vec!["CLN-01", "CLN-02"]);
        assert_eq!(steps[1].equipment_qualification, vec!["LIT-01", "LIT-02"]);
        assert_eq!(steps[2].equipment_qualification, vec!["ETC-01"]);
        assert_eq!(steps[3].equipment_qualification, vec!["MET-01"]);
    }

    #[test]
    fn a_step_naming_an_unqualified_tool_is_rejected() {
        let mut scenario = sample();
        scenario.route.steps[0].equipment_qualification = vec!["ETC-01".to_owned()];
        assert_eq!(
            scenario.validate(),
            Err(ScenarioError::UnqualifiedTool {
                step_no: 1,
                tool_id: "ETC-01".to_owned(),
            })
        );
    }

    #[test]
    fn an_unknown_recipe_is_rejected() {
        let mut scenario = sample();
        scenario.route.steps[0].recipe_id = "RCP-NOPE".to_owned();
        assert_eq!(
            scenario.validate(),
            Err(ScenarioError::UnknownRecipe {
                step_no: 1,
                recipe_id: "RCP-NOPE".to_owned(),
            })
        );
    }

    #[test]
    fn a_route_step_needs_a_qualified_tool() {
        let mut scenario = sample();
        for tool in &mut scenario.equipment {
            tool.qualification.clear();
        }
        assert_eq!(
            scenario.validate(),
            Err(ScenarioError::UnqualifiedTool {
                step_no: 1,
                tool_id: "CLN-01".to_owned(),
            })
        );
    }
}
