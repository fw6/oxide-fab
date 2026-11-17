//! The scene manifest that the offline Blender export writes next to the GLB.
//!
//! The manifest is the contract between the model and the code: it names every
//! mesh node, says which equipment or part it belongs to, carries the picking
//! box in glTF space, and gives each part its explosion direction. Nothing in
//! the application guesses a node name or a position that is not in here.
//!
//! `validate` is the guard: a GLB whose nodes, kinds or boxes disagree with the
//! manifest must fail at startup, not render a subtly wrong factory.

use std::collections::BTreeMap;

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// One axis-aligned box in glTF space (Y up, metres).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Aabb {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl Aabb {
    pub const ZERO: Self = Self {
        min: [0.0; 3],
        max: [0.0; 3],
    };

    #[must_use]
    pub fn new(min: [f32; 3], max: [f32; 3]) -> Self {
        Self { min, max }
    }

    #[must_use]
    pub fn center(self) -> Vec3 {
        (Vec3::from(self.min) + Vec3::from(self.max)) * 0.5
    }

    #[must_use]
    pub fn size(self) -> Vec3 {
        Vec3::from(self.max) - Vec3::from(self.min)
    }

    #[must_use]
    pub fn radius(self) -> f32 {
        self.size().length() * 0.5
    }

    #[must_use]
    pub fn expanded(self, by: f32) -> Self {
        Self {
            min: self.min.map(|v| v - by),
            max: self.max.map(|v| v + by),
        }
    }

    #[must_use]
    pub fn translated(self, offset: Vec3) -> Self {
        let [x, y, z] = offset.to_array();
        Self {
            min: [self.min[0] + x, self.min[1] + y, self.min[2] + z],
            max: [self.max[0] + x, self.max[1] + y, self.max[2] + z],
        }
    }

    #[must_use]
    pub fn union(self, other: Self) -> Self {
        Self {
            min: [
                self.min[0].min(other.min[0]),
                self.min[1].min(other.min[1]),
                self.min[2].min(other.min[2]),
            ],
            max: [
                self.max[0].max(other.max[0]),
                self.max[1].max(other.max[1]),
                self.max[2].max(other.max[2]),
            ],
        }
    }

    #[must_use]
    pub fn contains(self, point: Vec3) -> bool {
        let [x, y, z] = point.to_array();
        (self.min[0]..=self.max[0]).contains(&x)
            && (self.min[1]..=self.max[1]).contains(&y)
            && (self.min[2]..=self.max[2]).contains(&z)
    }

    /// Every corner, for drawing the box.
    #[must_use]
    pub fn corners(self) -> [Vec3; 8] {
        let [x0, y0, z0] = self.min;
        let [x1, y1, z1] = self.max;
        [
            Vec3::new(x0, y0, z0),
            Vec3::new(x1, y0, z0),
            Vec3::new(x1, y1, z0),
            Vec3::new(x0, y1, z0),
            Vec3::new(x0, y0, z1),
            Vec3::new(x1, y0, z1),
            Vec3::new(x1, y1, z1),
            Vec3::new(x0, y1, z1),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FloorSpec {
    pub half_x: f32,
    pub half_z: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BandSpec {
    pub step_no: i32,
    pub name: String,
    /// glTF z of the band centre.
    pub z: f32,
    /// Top of the band slab (the tools stand on it).
    pub y: f32,
    pub half_x: f32,
}

/// A non-tool mesh of the factory: the floor, a process band, an AMHS rail.
/// The names live here so no code formats a glTF node name itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StaticSpec {
    pub node: String,
    pub role: String,
    #[serde(default)]
    pub step_no: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartSpec {
    /// The glTF node name (`tool.CLN-01.chamber`).
    pub node: String,
    /// The semantic part (`body`, `chamber`, `loadport`, `panel`, `exhaust`).
    pub part: String,
    /// Unit direction; the renderer scales it by the manifest's distance.
    pub explode: [f32; 3],
    pub aabb: Aabb,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub equipment_id: String,
    pub kind: String,
    /// The empty that parents this tool's parts.
    pub root: String,
    pub band: i32,
    /// The root's world position (the bottom of the body).
    pub anchor: [f32; 3],
    pub parts: Vec<PartSpec>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TemplatePart {
    pub node: String,
    pub role: String,
    pub aabb: Aabb,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Templates {
    pub foup: Vec<TemplatePart>,
    pub wafer: Vec<TemplatePart>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AmhsSpec {
    /// The ring, (x, z) in glTF space; the last point joins the first.
    pub points: Vec<[f32; 2]>,
    pub r#loop: bool,
    /// Rail height, where FOUPs travel.
    pub height: f32,
    /// Per step: the ring point that feeds that step's band.
    pub stops: BTreeMap<String, [f32; 2]>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FabManifest {
    pub schema: u32,
    pub model: String,
    pub coordinate_system: String,
    pub units: String,
    pub floor: FloorSpec,
    pub bands: Vec<BandSpec>,
    pub r#static: Vec<StaticSpec>,
    pub tools: Vec<ToolSpec>,
    pub templates: Templates,
    pub amhs: AmhsSpec,
    pub explode_distance: f32,
    pub pick_padding: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest schema {0} is not supported (expected 1)")]
    Schema(u32),
    #[error("manifest coordinates are {0}, expected gltf-y-up")]
    Coordinates(String),
    #[error("no tools in the manifest")]
    NoTools,
    #[error("tool {0} has no parts")]
    ToolWithoutParts(String),
    #[error("tool {0} names the same part twice")]
    DuplicatePart(String),
    #[error("part {0} is not on a known band")]
    UnknownBand(String),
    #[error("part {0} has an inverted bounding box")]
    InvertedBox(String),
    #[error("step {0} has no band")]
    MissingBand(i32),
    #[error("step {0} has no AMHS stop")]
    MissingStop(i32),
    #[error("the AMHS ring needs at least three points")]
    ShortRing,
    #[error("template {0} has no body part")]
    TemplateWithoutBody(String),
    #[error("the manifest has no {role} static node")]
    MissingStatic { role: &'static str },
    #[error("step {0} names no process band node")]
    MissingBandNode(i32),
}

impl FabManifest {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// The manifest must describe exactly the four-step line the scenario runs.
    pub fn validate(&self, step_numbers: &[i32]) -> Result<(), ManifestError> {
        if self.schema != 1 {
            return Err(ManifestError::Schema(self.schema));
        }
        if self.coordinate_system != "gltf-y-up" {
            return Err(ManifestError::Coordinates(self.coordinate_system.clone()));
        }
        if self.parts().next().is_none() {
            return Err(ManifestError::NoTools);
        }
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for tool in &self.tools {
            if tool.parts.is_empty() {
                return Err(ManifestError::ToolWithoutParts(tool.equipment_id.clone()));
            }
            if self.band(tool.band).is_none() {
                return Err(ManifestError::UnknownBand(tool.equipment_id.clone()));
            }
            for part in &tool.parts {
                if !seen.insert(part.node.as_str()) {
                    return Err(ManifestError::DuplicatePart(part.node.clone()));
                }
                if part.aabb.min[0] > part.aabb.max[0]
                    || part.aabb.min[1] > part.aabb.max[1]
                    || part.aabb.min[2] > part.aabb.max[2]
                {
                    return Err(ManifestError::InvertedBox(part.node.clone()));
                }
            }
        }
        for step in step_numbers {
            if self.band(*step).is_none() {
                return Err(ManifestError::MissingBand(*step));
            }
            if !self.amhs.stops.contains_key(&step.to_string()) {
                return Err(ManifestError::MissingStop(*step));
            }
        }
        if self.amhs.points.len() < 3 {
            return Err(ManifestError::ShortRing);
        }
        if self.templates.foup.iter().all(|part| part.role != "body") {
            return Err(ManifestError::TemplateWithoutBody("foup".to_owned()));
        }
        if self.templates.wafer.is_empty() {
            return Err(ManifestError::TemplateWithoutBody("wafer".to_owned()));
        }
        if !self.r#static.iter().any(|node| node.role == "floor") {
            return Err(ManifestError::MissingStatic { role: "floor" });
        }
        if !self.r#static.iter().any(|node| node.role == "amhs_rail") {
            return Err(ManifestError::MissingStatic { role: "amhs_rail" });
        }
        for step in step_numbers {
            if self.band_node(*step).is_none() {
                return Err(ManifestError::MissingBandNode(*step));
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn band(&self, step_no: i32) -> Option<&BandSpec> {
        self.bands.iter().find(|band| band.step_no == step_no)
    }

    #[must_use]
    pub fn static_node(&self, role: &str) -> Option<&str> {
        self.r#static
            .iter()
            .find(|node| node.role == role)
            .map(|node| node.node.as_str())
    }

    #[must_use]
    pub fn band_node(&self, step_no: i32) -> Option<&str> {
        self.r#static
            .iter()
            .find(|node| node.role == "process_band" && node.step_no == Some(step_no))
            .map(|node| node.node.as_str())
    }

    /// Every AMHS mesh (rails and their posts).
    pub fn amhs_nodes(&self) -> impl Iterator<Item = &str> {
        self.r#static
            .iter()
            .filter(|node| node.role.starts_with("amhs_"))
            .map(|node| node.node.as_str())
    }

    #[must_use]
    pub fn tool(&self, equipment_id: &str) -> Option<&ToolSpec> {
        self.tools
            .iter()
            .find(|tool| tool.equipment_id == equipment_id)
    }

    /// Every part of every tool.
    pub fn parts(&self) -> impl Iterator<Item = (&ToolSpec, &PartSpec)> {
        self.tools
            .iter()
            .flat_map(|tool| tool.parts.iter().map(move |part| (tool, part)))
    }

    #[must_use]
    pub fn part(&self, node: &str) -> Option<(&ToolSpec, &PartSpec)> {
        self.parts().find(|(_, part)| part.node == node)
    }

    /// The node names the GLB must contain, in manifest order.
    #[must_use]
    pub fn expected_nodes(&self) -> Vec<&str> {
        let mut nodes: Vec<&str> = self
            .r#static
            .iter()
            .map(|node| node.node.as_str())
            .collect();
        nodes.extend(
            self.tools
                .iter()
                .flat_map(|tool| tool.parts.iter().map(|part| part.node.as_str())),
        );
        nodes.extend(self.templates.foup.iter().map(|part| part.node.as_str()));
        nodes.extend(self.templates.wafer.iter().map(|part| part.node.as_str()));
        nodes
    }

    #[must_use]
    pub fn foup_body(&self) -> &TemplatePart {
        self.templates
            .foup
            .iter()
            .find(|part| part.role == "body")
            .expect("validate() rejects a FOUP template without a body")
    }

    #[must_use]
    pub fn wafer_body(&self) -> &TemplatePart {
        &self.templates.wafer[0]
    }

    /// The interpolation ring as world points at rail height.
    #[must_use]
    pub fn ring(&self) -> Vec<Vec3> {
        self.amhs
            .points
            .iter()
            .map(|[x, z]| Vec3::new(*x, self.amhs.height, *z))
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const MANIFEST: &str = include_str!("../../../assets/fab/fab-manifest.json");

    fn manifest() -> FabManifest {
        FabManifest::from_json(MANIFEST).expect("the committed manifest parses")
    }

    #[test]
    fn the_committed_manifest_validates_for_the_four_step_route() {
        manifest().validate(&[1, 2, 3, 4]).unwrap();
    }

    #[test]
    fn every_tool_has_the_five_named_parts() {
        let manifest = manifest();
        assert_eq!(manifest.tools.len(), 6);
        for tool in &manifest.tools {
            let names: Vec<&str> = tool.parts.iter().map(|part| part.part.as_str()).collect();
            assert_eq!(
                names,
                ["body", "chamber", "loadport", "panel", "exhaust"],
                "{}",
                tool.equipment_id
            );
        }
    }

    #[test]
    fn a_part_box_contains_the_tool_anchor_and_sits_above_the_band() {
        let manifest = manifest();
        let tool = manifest.tool("CLN-01").unwrap();
        let band = manifest.band(tool.band).unwrap();
        let body = &tool.parts[0];
        assert!(body.aabb.min[1] >= band.y - 0.01);
        assert!(
            body.aabb
                .contains(Vec3::from(tool.anchor) + Vec3::new(0.0, 0.4, 0.0))
        );
    }

    #[test]
    fn an_inverted_box_is_rejected() {
        let mut manifest = manifest();
        manifest.tools[0].parts[0].aabb.min[1] = 99.0;
        assert_eq!(
            manifest.validate(&[1, 2, 3, 4]),
            Err(ManifestError::InvertedBox(
                manifest.tools[0].parts[0].node.clone()
            ))
        );
    }

    #[test]
    fn a_step_without_a_stop_is_rejected() {
        let mut manifest = manifest();
        manifest.amhs.stops.remove("3");
        assert_eq!(
            manifest.validate(&[1, 2, 3, 4]),
            Err(ManifestError::MissingStop(3))
        );
    }

    #[test]
    fn boxes_union_translate_and_expand() {
        let a = Aabb::new([0.0; 3], [1.0; 3]);
        let b = Aabb::new([2.0, 0.0, 0.0], [3.0, 1.0, 1.0]);
        assert_eq!(a.union(b), Aabb::new([0.0; 3], [3.0, 1.0, 1.0]));
        assert_eq!(
            a.translated(Vec3::new(0.0, 2.0, 0.0)),
            Aabb::new([0.0, 2.0, 0.0], [1.0, 3.0, 1.0])
        );
        assert_eq!(a.expanded(0.5), Aabb::new([-0.5; 3], [1.5; 3]));
    }
}
