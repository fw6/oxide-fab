//! Where things stand: the AMHS ring, queue anchors and FOUP positions.
//!
//! Every position is derived from the manifest and the read-only floor state.
//! The only interpolation is presentation: a FOUP that has just changed step
//! glides along the ring to its new queue slot, and stops gliding when the
//! user asks for reduced motion.

use std::collections::BTreeMap;

use glam::Vec3;

use crate::manifest::FabManifest;
use crate::state::{FloorState, LotState};

/// How long a FOUP takes to travel between two stops, in demo-clock ms.
pub const TRANSIT_MS: i64 = 1_400;
/// The share of the transit spent descending from the rail into the queue.
const APPROACH: f32 = 0.28;

/// The AMHS ring with cumulative lengths and per-step stops.
#[derive(Debug, Clone)]
pub struct AmhsPath {
    points: Vec<Vec3>,
    cumulative: Vec<f32>,
    total: f32,
    height: f32,
    stops: BTreeMap<i32, Vec3>,
}

impl AmhsPath {
    #[must_use]
    pub fn new(manifest: &FabManifest) -> Self {
        let points = manifest.ring();
        let mut cumulative = Vec::with_capacity(points.len() + 1);
        let mut total = 0.0;
        cumulative.push(0.0);
        for pair in points.windows(2) {
            total += pair[0].distance(pair[1]);
            cumulative.push(total);
        }
        if manifest.amhs.r#loop
            && let (Some(first), Some(last)) = (points.first(), points.last())
        {
            total += last.distance(*first);
            cumulative.push(total);
        }
        let stops = manifest
            .amhs
            .stops
            .iter()
            .filter_map(|(step, [x, z])| {
                step.parse::<i32>()
                    .ok()
                    .map(|step| (step, Vec3::new(*x, manifest.amhs.height, *z)))
            })
            .collect();
        Self {
            points,
            cumulative,
            total,
            height: manifest.amhs.height,
            stops,
        }
    }

    /// The ring points in order, for drawing the rail.
    #[must_use]
    pub fn points(&self) -> &[Vec3] {
        &self.points
    }

    #[must_use]
    pub fn height(&self) -> f32 {
        self.height
    }

    #[must_use]
    pub fn stop(&self, step_no: i32) -> Option<Vec3> {
        self.stops.get(&step_no).copied()
    }

    /// Distance from the start of the ring to the point nearest `target`.
    fn distance_of(&self, target: Vec3) -> f32 {
        let mut best = (f32::INFINITY, 0.0);
        for (index, pair) in self.points.windows(2).enumerate() {
            let (a, b) = (pair[0], pair[1]);
            let segment = b - a;
            let length = segment.length();
            if length <= f32::EPSILON {
                continue;
            }
            let t = ((target - a).dot(segment) / (length * length)).clamp(0.0, 1.0);
            let closest = a + segment * t;
            let distance = closest.distance_squared(target);
            if distance < best.0 {
                best = (distance, self.cumulative[index] + t * length);
            }
        }
        best.1
    }

    /// The point `distance` around the ring, walking forward.
    #[must_use]
    pub fn at_distance(&self, distance: f32) -> Vec3 {
        if self.points.is_empty() {
            return Vec3::ZERO;
        }
        if self.total <= f32::EPSILON {
            return self.points[0];
        }
        let distance = distance.rem_euclid(self.total);
        for (index, pair) in self.points.windows(2).enumerate() {
            let start = self.cumulative[index];
            let end = self.cumulative[index + 1];
            if distance <= end {
                let t = ((distance - start) / (end - start).max(1e-6)).clamp(0.0, 1.0);
                return pair[0].lerp(pair[1], t);
            }
        }
        if self.points.len() > 1 {
            let last = *self.points.last().expect("non-empty");
            let first = self.points[0];
            let start = self.cumulative[self.points.len() - 1];
            let t = ((distance - start) / (self.total - start).max(1e-6)).clamp(0.0, 1.0);
            return last.lerp(first, t);
        }
        self.points[0]
    }

    /// Walk from `from` to `to` along the ring (forward) by `t` in `0..=1`.
    #[must_use]
    pub fn between(&self, from: i32, to: i32, t: f32) -> Vec3 {
        let (Some(start), Some(end)) = (self.stop(from), self.stop(to)) else {
            return self.stop(to).unwrap_or(self.points[0]);
        };
        if (start - end).length() < 0.01 {
            return end;
        }
        let a = self.distance_of(start);
        let b = self.distance_of(end);
        let mut span = b - a;
        if span < 0.0 {
            span += self.total;
        }
        self.at_distance(a + span * t.clamp(0.0, 1.0))
    }
}

/// Where a lot's FOUP stands and how far it has descended from the rail.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FoupPlacement {
    pub position: Vec3,
    /// Yaw in radians; 0 faces +z (toward the viewer/aisle).
    pub yaw: f32,
}

/// The queue anchor for `index` lots waiting at a band.
#[must_use]
pub fn queue_anchor(manifest: &FabManifest, step_no: i32, index: usize) -> Vec3 {
    let Some(band) = manifest.band(step_no) else {
        return Vec3::ZERO;
    };
    // In front of the band, ordered from the west end: a visible queue.
    let first = -band.half_x + 2.4;
    Vec3::new(first + index as f32 * 1.35, 0.0, band.z + 2.35)
}

/// The load-port position in front of a tool.
#[must_use]
pub fn load_port(manifest: &FabManifest, equipment_id: &str) -> Vec3 {
    let Some(tool) = manifest.tool(equipment_id) else {
        return Vec3::ZERO;
    };
    if let Some(part) = tool.parts.iter().find(|part| part.part == "loadport") {
        let center = part.aabb.center();
        Vec3::new(center.x, 0.0, part.aabb.max[2] + 0.62)
    } else {
        Vec3::new(tool.anchor[0], 0.0, tool.anchor[2] + 2.2)
    }
}

/// The exit aisle, east of the floor.
#[must_use]
pub fn exit_anchor(manifest: &FabManifest) -> Vec3 {
    Vec3::new(manifest.floor.half_x - 2.0, 0.0, 0.0)
}

/// The tool a lot is loaded on, if any.
fn on_tool<'a>(floor: &'a FloorState, lot: &LotState) -> Option<&'a str> {
    floor
        .tools
        .iter()
        .find(|tool| tool.current_lot.as_deref() == Some(lot.id.as_str()))
        .map(|tool| tool.id.as_str())
}

/// Whether the lot ended its life: completed, split or merged.
fn lot_terminal(lot: &LotState) -> bool {
    matches!(lot.state.as_str(), "completed" | "split" | "merged")
}

/// Whether `lot`'s FOUP moves with the clock at `now_ms`.
///
/// The step-change window is the scene's only clock-driven motion: a lot on a
/// tool, a finished lot and reduced motion all stand still. `foup_placement`
/// places the FOUP accordingly, and the viewport's idle check reads the same
/// predicate, so a frame is never skipped while a FOUP is gliding.
#[must_use]
pub fn lot_moving(floor: &FloorState, lot: &LotState, now_ms: i64, reduce_motion: bool) -> bool {
    if reduce_motion || lot.step_since_ms <= 0 || lot_terminal(lot) {
        return false;
    }
    if !(0..TRANSIT_MS).contains(&(now_ms - lot.step_since_ms)) {
        return false;
    }
    on_tool(floor, lot).is_none()
}

/// One lot's FOUP placement at `now_ms`.
///
/// - on a tool: at its load port;
/// - queued/waiting: the queue slot in front of the band;
/// - in transit (recent step change, motion allowed): on the AMHS ring, then
///   descending into the queue;
/// - completed/split/merged: the exit aisle.
#[must_use]
pub fn foup_placement(
    manifest: &FabManifest,
    path: &AmhsPath,
    floor: &FloorState,
    lot: &LotState,
    now_ms: i64,
    reduce_motion: bool,
) -> FoupPlacement {
    if let Some(tool) = on_tool(floor, lot) {
        return FoupPlacement {
            position: load_port(manifest, tool),
            yaw: std::f32::consts::PI,
        };
    }
    if lot_terminal(lot) {
        return FoupPlacement {
            position: exit_anchor(manifest),
            yaw: -std::f32::consts::FRAC_PI_2,
        };
    }
    let index = floor
        .queue(lot.current_step)
        .iter()
        .position(|queued| queued.id == lot.id)
        .unwrap_or(0);
    let anchor = queue_anchor(manifest, lot.current_step, index);
    if lot_moving(floor, lot, now_ms, reduce_motion) {
        let elapsed = now_ms - lot.step_since_ms;
        let t = (elapsed as f32 / TRANSIT_MS as f32).clamp(0.0, 1.0);
        let from = lot.current_step.saturating_sub(1).max(1);
        let ring = path.between(from, lot.current_step, t);
        if t >= 1.0 - APPROACH {
            // The last stretch descends from the rail into the queue slot.
            let k = ((t - (1.0 - APPROACH)) / APPROACH).clamp(0.0, 1.0);
            let eased = k * k * (3.0 - 2.0 * k);
            return FoupPlacement {
                position: ring.lerp(anchor, eased),
                yaw: 0.0,
            };
        }
        return FoupPlacement {
            position: ring,
            yaw: 0.0,
        };
    }
    FoupPlacement {
        position: anchor,
        yaw: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::MANIFEST;

    fn manifest() -> FabManifest {
        FabManifest::from_json(MANIFEST).unwrap()
    }

    fn queued_lot(id: &str, step: i32) -> LotState {
        LotState {
            id: id.to_owned(),
            state: "queued".to_owned(),
            current_step: step,
            wafer_count: 10,
            ..LotState::default()
        }
    }

    #[test]
    fn the_ring_is_a_closed_loop_with_the_four_stops_on_it() {
        let manifest = manifest();
        let path = AmhsPath::new(&manifest);
        assert_eq!(path.points().len(), 4);
        for step in 1..=4 {
            let stop = path.stop(step).unwrap();
            let distance = path.distance_of(stop);
            assert!((path.at_distance(distance) - stop).length() < 0.01);
        }
    }

    #[test]
    fn walking_between_stops_lands_on_the_destination_at_one() {
        let manifest = manifest();
        let path = AmhsPath::new(&manifest);
        let end = path.between(1, 3, 1.0);
        assert!((end - path.stop(3).unwrap()).length() < 0.01);
        let start = path.between(1, 3, 0.0);
        assert!((start - path.stop(1).unwrap()).length() < 0.01);
    }

    #[test]
    fn queue_slots_are_ordered_and_in_front_of_the_band() {
        let manifest = manifest();
        let first = queue_anchor(&manifest, 2, 0);
        let second = queue_anchor(&manifest, 2, 1);
        assert!(second.x > first.x);
        assert_eq!(first.z, manifest.band(2).unwrap().z + 2.35);
    }

    #[test]
    fn a_lot_on_a_tool_sits_at_its_load_port() {
        let manifest = manifest();
        let path = AmhsPath::new(&manifest);
        let lot = LotState {
            id: "LOT-1".to_owned(),
            state: "processing".to_owned(),
            current_step: 1,
            ..LotState::default()
        };
        let floor = FloorState {
            tools: vec![crate::state::ToolState {
                id: "CLN-01".to_owned(),
                current_lot: Some("LOT-1".to_owned()),
                ..crate::state::ToolState::default()
            }],
            ..FloorState::default()
        };
        let placement = foup_placement(&manifest, &path, &floor, &lot, 0, false);
        assert_eq!(placement.position, load_port(&manifest, "CLN-01"));
        assert!(placement.position.z > manifest.band(1).unwrap().z);
    }

    #[test]
    fn a_recent_step_change_puts_the_foup_on_the_rail() {
        let manifest = manifest();
        let path = AmhsPath::new(&manifest);
        let mut lot = queued_lot("LOT-1", 3);
        lot.step_since_ms = 10_000;
        let floor = FloorState {
            lots: vec![lot.clone()],
            ..FloorState::default()
        };
        let placement = foup_placement(&manifest, &path, &floor, &lot, 10_200, false);
        assert!((placement.position.y - path.height()).abs() < 0.3);
    }

    #[test]
    fn reduced_motion_skips_the_transit() {
        let manifest = manifest();
        let path = AmhsPath::new(&manifest);
        let mut lot = queued_lot("LOT-1", 3);
        lot.step_since_ms = 10_000;
        let floor = FloorState {
            lots: vec![lot.clone()],
            ..FloorState::default()
        };
        let placement = foup_placement(&manifest, &path, &floor, &lot, 10_200, true);
        assert_eq!(placement.position.y, 0.0);
    }

    #[test]
    fn a_lot_moves_only_inside_the_transit_window_and_off_a_tool() {
        let mut lot = queued_lot("LOT-1", 3);
        lot.step_since_ms = 10_000;
        let floor = FloorState {
            lots: vec![lot.clone()],
            ..FloorState::default()
        };
        assert!(lot_moving(&floor, &lot, 10_200, false));
        assert!(!lot_moving(&floor, &lot, 10_000 + TRANSIT_MS, false));
        assert!(!lot_moving(&floor, &lot, 10_200, true));

        let on_tool = FloorState {
            tools: vec![crate::state::ToolState {
                id: "CLN-01".to_owned(),
                current_lot: Some("LOT-1".to_owned()),
                ..crate::state::ToolState::default()
            }],
            lots: vec![lot.clone()],
            ..FloorState::default()
        };
        assert!(!lot_moving(&on_tool, &lot, 10_200, false));
    }
}
