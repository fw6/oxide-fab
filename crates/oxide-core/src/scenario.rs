//! The scenario file: what the simulated fab is made of.
//!
//! `config/scenario.json` describes products, the route, recipes, the tools
//! and how much material is released. It is data, not code: changing the
//! scenario changes the demo without a rebuild.

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
        let scenario: Scenario = serde_json::from_str(json)?;
        Ok(scenario)
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
    pub fn validate(&self) -> Result<(), String> {
        if self.route.steps.is_empty() {
            return Err("route has no steps".to_owned());
        }
        for step in &self.route.steps {
            if self.recipe(&step.recipe_id).is_none() {
                return Err(format!(
                    "step {} references unknown recipe {}",
                    step.step_no, step.recipe_id
                ));
            }
            if !self
                .equipment
                .iter()
                .any(|e| e.kind == step.equipment_kind && e.qualification.contains(&step.recipe_id))
            {
                return Err(format!(
                    "no tool of kind {} is qualified for {}",
                    step.equipment_kind, step.recipe_id
                ));
            }
        }
        if self.wafers_per_lot <= 0 {
            return Err("wafers_per_lot must be positive".to_owned());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Scenario {
        serde_json::from_str(include_str!("../../../config/scenario.json")).unwrap()
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
    fn an_unknown_recipe_is_rejected() {
        let mut scenario = sample();
        scenario.route.steps[0].recipe_id = "RCP-NOPE".to_owned();
        assert!(scenario.validate().is_err());
    }

    #[test]
    fn a_route_step_needs_a_qualified_tool() {
        let mut scenario = sample();
        for tool in &mut scenario.equipment {
            tool.qualification.clear();
        }
        assert!(scenario.validate().is_err());
    }
}
