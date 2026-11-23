//! The renderer's own contract: it draws something, the four levels look
//! different, and the two projections are genuinely different projections.
//!
//! These tests open a real GPU device. On CI that is a software adapter
//! (lavapipe, WARP), which is the point: the shipping path is what runs.

use oxide_render::{Model, Renderer, RendererOptions};
use oxide_scene::{
    AmhsPath, Camera, Entity, FabManifest, FloorState, Level, LotState, SceneOptions, ToolState,
    WaferState, build,
};

const GLB: &[u8] = include_bytes!("../../../crates/oxide-ui/assets/fab/oxide-fab.glb");
const MANIFEST: &str = include_str!("../../../crates/oxide-ui/assets/fab/fab-manifest.json");

fn setup() -> (FabManifest, FloorState, AmhsPath, Renderer) {
    let manifest = FabManifest::from_json(MANIFEST).unwrap();
    manifest.validate(&[1, 2, 3, 4]).unwrap();
    let model = Model::load(GLB).unwrap();
    model.validate(&manifest).unwrap();
    let mut renderer = Renderer::new(model, RendererOptions::default()).unwrap();
    renderer.resize(480, 300);
    let path = AmhsPath::new(&manifest);
    let mut tools = Vec::new();
    for (index, tool) in manifest.tools.iter().enumerate() {
        tools.push(ToolState {
            id: tool.equipment_id.clone(),
            kind: tool.kind.clone(),
            state: if index % 2 == 0 { "running" } else { "down" }.to_owned(),
            light: if index % 2 == 0 { "green" } else { "red" }.to_owned(),
            current_lot: (index == 0).then(|| "LOT-0001".to_owned()),
            current_step: Some(tool.band),
            lots_processed: 1,
        });
    }
    let floor = FloorState {
        now_ms: 5_000,
        running: true,
        speed: 10.0,
        steps: Vec::new(),
        tools,
        lots: vec![LotState {
            id: "LOT-0001".to_owned(),
            state: "processing".to_owned(),
            current_step: 1,
            priority: 5,
            wafer_count: 4,
            done_wafers: 1,
            step_since_ms: 0,
            wafers: (1..=4)
                .map(|slot| WaferState {
                    id: format!("LOT-0001-W{slot:02}"),
                    slot,
                    state: if slot == 1 { "done" } else { "processing" }.to_owned(),
                    ..WaferState::default()
                })
                .collect(),
            ..LotState::default()
        }],
    };
    (manifest, floor, path, renderer)
}

fn render(level: Level, focus: Option<&Entity>, camera: &Camera) -> (Vec<u8>, u32, u32) {
    let (manifest, floor, path, mut renderer) = setup();
    let graph = build(
        &manifest,
        &floor,
        &SceneOptions {
            level,
            focus: focus.cloned(),
            explosion: if level == Level::Tool { 1.0 } else { 0.0 },
            selected: focus.cloned(),
            reduce_motion: true,
            ..SceneOptions::default()
        },
        &path,
    );
    // Drill-down moves the camera where the app moves it.
    let mut camera = *camera;
    let bounds = match focus {
        None => None,
        Some(entity) => match entity {
            Entity::Tool { equipment_id } => graph.tool_bounds(equipment_id),
            Entity::Foup { lot_id } | Entity::Wafer { lot_id, .. } => graph.lot_bounds(lot_id),
            other => graph.bounds_of(other),
        },
    };
    if let Some(bounds) = bounds {
        camera.focus(bounds);
        camera.distance *= 1.4;
    }
    let frame = renderer.render(&graph, &camera).unwrap();
    (frame.rgba, frame.width, frame.height)
}

#[test]
fn the_factory_renders_a_picture_and_not_a_clear_colour() {
    let (rgba, width, height) = render(Level::Factory, None, &Camera::default());
    assert_eq!(rgba.len(), 480 * 300 * 4);
    assert_eq!((width, height), (480, 300));
    let background = [14.0_f32, 17.0, 23.0];
    let bright = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| f32::from(pixel[0]) > background[0] + 30.0)
        .count();
    assert!(
        bright > 500,
        "only {bright} pixels are brighter than the background"
    );
    // The frame is not one flat colour: some pixels differ from the corner.
    let corner = &rgba[0..4];
    let differing = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel != &corner)
        .count();
    assert!(differing > rgba.len() / 4 / 4);
}

#[test]
fn the_four_levels_are_four_pictures() {
    let camera = Camera::default();
    let frames = [
        render(Level::Factory, None, &camera),
        render(
            Level::Tool,
            Some(&Entity::Tool {
                equipment_id: "CLN-01".to_owned(),
            }),
            &camera,
        ),
        render(
            Level::Lot,
            Some(&Entity::Foup {
                lot_id: "LOT-0001".to_owned(),
            }),
            &camera,
        ),
        render(
            Level::Event,
            Some(&Entity::Tool {
                equipment_id: "LIT-01".to_owned(),
            }),
            &camera,
        ),
    ];
    for i in 0..frames.len() {
        for j in (i + 1)..frames.len() {
            let differing = frames[i]
                .0
                .iter()
                .zip(frames[j].0.iter())
                .filter(|(a, b)| a != b)
                .count();
            assert!(
                differing > frames[i].0.len() / 100,
                "levels {i} and {j} differ in only {differing} bytes"
            );
        }
    }
}

#[test]
fn orthographic_and_perspective_differ() {
    let mut ortho = Camera::default();
    ortho.toggle_projection();
    let perspective = render(Level::Factory, None, &Camera::default());
    let orthographic = render(Level::Factory, None, &ortho);
    assert_ne!(perspective.1, 0);
    let difference = perspective
        .0
        .iter()
        .zip(orthographic.0.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        difference > perspective.0.len() / 20,
        "only {difference} bytes differ between the two projections"
    );
}
