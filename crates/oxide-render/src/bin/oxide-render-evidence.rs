//! Headless render evidence: draws the four drill-down levels and writes PNGs
//! plus a JSON report naming the adapter that drew them.
//!
//! This is the CI proof that the wgpu path works without a window server, on a
//! software adapter where the platform has one (lavapipe on Linux, WARP on
//! Windows), and it is also the local way to see what the viewport draws.
//!
//! ```sh
//! cargo run -p oxide-render --bin oxide-render-evidence -- \
//!   --out artifacts/render
//! ```

use std::path::PathBuf;
use std::time::Instant;

use oxide_render::{Model, Renderer, RendererOptions};
use oxide_scene::{
    AmhsPath, Camera, Entity, FabManifest, FloorState, Level, LotState, Projection, SceneOptions,
    SceneTheme, ToolState, WaferState, build, foup_placement,
};

/// The fixture the evidence renders: the six tools of the shipped scenario,
/// three lots in flight and one completed, with one scrapped wafer so the
/// defect colours appear in the picture.
fn fixture(manifest: &FabManifest) -> FloorState {
    let mut tools = Vec::new();
    for (index, tool) in manifest.tools.iter().enumerate() {
        let state = match index % 3 {
            0 => "running",
            1 => "idle",
            _ => "down",
        };
        let light = match state {
            "running" => "green",
            "down" => "red",
            _ => "amber",
        };
        tools.push(ToolState {
            id: tool.equipment_id.clone(),
            kind: tool.kind.clone(),
            state: state.to_owned(),
            light: light.to_owned(),
            current_lot: (index % 2 == 0).then(|| format!("LOT-{:04}", index + 1)),
            current_step: Some(tool.band),
            lots_processed: 3 + index as u64,
        });
    }
    let mut lots = Vec::new();
    for serial in 0..6 {
        let step = serial % 4 + 1;
        let on_tool = tools.iter().find(|tool| {
            tool.current_lot.as_deref() == Some(format!("LOT-{:04}", serial + 1).as_str())
        });
        let state = if on_tool.is_some() {
            "processing"
        } else if serial == 5 {
            "completed"
        } else {
            "queued"
        };
        let wafers = (1..=10)
            .map(|slot| WaferState {
                id: format!("LOT-{:04}-W{:02}", serial + 1, slot),
                slot,
                state: if slot == 2 && serial == 0 {
                    "scrapped".to_owned()
                } else if slot <= 4 {
                    "done".to_owned()
                } else if state == "processing" {
                    "processing".to_owned()
                } else {
                    "waiting".to_owned()
                },
                defect_info: (slot == 2 && serial == 0).then(|| "scratch at edge".to_owned()),
                rework_count: u32::from(slot == 3 && serial == 1),
            })
            .collect();
        lots.push(LotState {
            id: format!("LOT-{:04}", serial + 1),
            state: state.to_owned(),
            current_step: step,
            priority: if serial == 1 { 9 } else { 5 },
            wafer_count: 10,
            done_wafers: 4,
            hold_reason: None,
            step_since_ms: 0,
            wafers,
        });
    }
    FloorState {
        now_ms: 120_000,
        running: true,
        speed: 10.0,
        steps: Vec::new(),
        tools,
        lots,
    }
}

struct Options {
    out: PathBuf,
    width: u32,
    height: u32,
    software: bool,
    glb: PathBuf,
    manifest: PathBuf,
    validate_only: bool,
}

fn parse_args() -> Options {
    let mut options = Options {
        out: PathBuf::from("artifacts/render"),
        width: 1440,
        height: 900,
        software: std::env::var("OXIDE_RENDER_SOFTWARE").is_ok_and(|value| value == "1"),
        glb: PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../assets/fab/oxide-fab.glb"),
        manifest: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../assets/fab/fab-manifest.json"),
        validate_only: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => options.out = args.next().map(Into::into).unwrap_or(options.out),
            "--width" => options.width = args.next().and_then(|v| v.parse().ok()).unwrap_or(1440),
            "--height" => options.height = args.next().and_then(|v| v.parse().ok()).unwrap_or(900),
            "--software" => options.software = true,
            "--validate-only" => options.validate_only = true,
            "--glb" => options.glb = args.next().map(Into::into).unwrap_or(options.glb),
            "--manifest" => {
                options.manifest = args.next().map(Into::into).unwrap_or(options.manifest);
            }
            "--help" | "-h" => {
                println!(
                    "oxide-render-evidence [--out DIR] [--width N] [--height N] [--software] \
                     [--validate-only] [--glb FILE] [--manifest FILE]"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }
    options
}

#[expect(
    clippy::too_many_lines,
    reason = "the evidence run: four levels, the ortho shot and the report in one linear script"
)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = parse_args();
    let manifest_text = std::fs::read_to_string(&options.manifest)?;
    let manifest = FabManifest::from_json(&manifest_text)?;
    let steps: Vec<i32> = manifest.bands.iter().map(|band| band.step_no).collect();
    manifest.validate(&steps)?;
    let bytes = std::fs::read(&options.glb)?;
    let model = Model::load(&bytes)?;
    model.validate(&manifest)?;
    let path = AmhsPath::new(&manifest);
    let floor = fixture(&manifest);

    if options.validate_only {
        println!(
            "assets ok: {} manifest nodes, {} GLB meshes, {} vertices, {} tools, {} bands",
            manifest.expected_nodes().len(),
            model.mesh_count(),
            model.vertex_count(),
            manifest.tools.len(),
            manifest.bands.len()
        );
        return Ok(());
    }

    let mut renderer = Renderer::new(
        model,
        RendererOptions {
            prefer_software: options.software,
            max_side: options.width.max(options.height),
        },
    )?;
    renderer.resize(options.width, options.height);
    std::fs::create_dir_all(&options.out)?;
    println!("adapter: {}", renderer.adapter().summary());
    println!("software: {}", renderer.adapter().software);

    let theme = SceneTheme::default();
    let mut shots = Vec::new();
    let camera = Camera::default();

    for (name, level, focus) in [
        ("l0-factory", Level::Factory, None),
        (
            "l1-tool-exploded",
            Level::Tool,
            Some(Entity::Tool {
                equipment_id: "CLN-01".to_owned(),
            }),
        ),
        (
            "l2-lot-wafers",
            Level::Lot,
            Some(Entity::Foup {
                lot_id: "LOT-0001".to_owned(),
            }),
        ),
        (
            "l3-event-beacon",
            Level::Event,
            Some(Entity::Tool {
                equipment_id: "LIT-01".to_owned(),
            }),
        ),
    ] {
        let scene_options = SceneOptions {
            level,
            focus: focus.clone(),
            explosion: if level == Level::Tool { 1.0 } else { 0.0 },
            selected: focus,
            reduce_motion: false,
            derive_alert: false,
            theme,
        };
        let graph = build(&manifest, &floor, &scene_options, &path);
        let bounds = match &scene_options.focus {
            None => None,
            Some(focus) => match focus {
                Entity::Tool { equipment_id } => graph.tool_bounds(equipment_id),
                Entity::Foup { lot_id } | Entity::Wafer { lot_id, .. } => graph.lot_bounds(lot_id),
                other => graph.bounds_of(other),
            },
        };
        let mut shot_camera = if level == Level::Factory {
            Camera::default()
        } else {
            camera
        };
        if let Some(bounds) = bounds {
            shot_camera.focus(bounds);
            let multiplier = if level == Level::Lot { 2.6 } else { 1.25 };
            shot_camera.distance = (shot_camera.distance * multiplier).max(6.0);
        }
        let started = Instant::now();
        let frame = renderer.render(&graph, &shot_camera)?;
        let elapsed_ms = started.elapsed().as_millis();
        let file = options.out.join(format!("{name}.png"));
        frame.to_png(&file)?;
        let bytes = std::fs::metadata(&file)?.len();
        println!(
            "{name}: {}x{} → {} ({} bytes, {} ms, {} nodes, {} lines)",
            frame.width,
            frame.height,
            file.display(),
            bytes,
            elapsed_ms,
            graph.nodes.len(),
            graph.lines.len()
        );
        shots.push(serde_json::json!({
            "name": name,
            "level": level.label(),
            "projection": "perspective",
            "width": frame.width,
            "height": frame.height,
            "nodes": graph.nodes.len(),
            "lines": graph.lines.len(),
            "file": file.display().to_string(),
            "bytes": bytes,
            "render_ms": elapsed_ms,
            "mean_rgb": frame.mean_rgb(),
        }));
    }

    // The orthographic projection is a requirement of its own: prove it draws
    // the same scene with the same graph.
    let mut ortho_camera = camera;
    ortho_camera.toggle_projection();
    let scene_options = SceneOptions {
        level: Level::Factory,
        focus: None,
        explosion: 0.0,
        selected: None,
        reduce_motion: true,
        derive_alert: false,
        theme,
    };
    let graph = build(&manifest, &floor, &scene_options, &path);
    let frame = renderer.render(&graph, &ortho_camera)?;
    let file = options.out.join("l0-factory-ortho.png");
    frame.to_png(&file)?;
    println!(
        "l0-factory-ortho: {}x{} → {} ({} bytes)",
        frame.width,
        frame.height,
        file.display(),
        std::fs::metadata(&file)?.len()
    );
    shots.push(serde_json::json!({
        "name": "l0-factory-ortho",
        "level": Level::Factory.label(),
        "projection": Projection::Orthographic.label(),
        "width": frame.width,
        "height": frame.height,
        "nodes": graph.nodes.len(),
        "lines": graph.lines.len(),
        "file": file.display().to_string(),
        "bytes": std::fs::metadata(&file)?.len(),
        "render_ms": 0,
        "mean_rgb": frame.mean_rgb(),
    }));

    // The queue positions the fixture produced, for the report: the same
    // layout the viewport uses.
    let placements: Vec<serde_json::Value> = floor
        .lots
        .iter()
        .map(|lot| {
            let placement = foup_placement(&manifest, &path, &floor, lot, floor.now_ms, true);
            serde_json::json!({
                "lot_id": lot.id,
                "step": lot.current_step,
                "x": placement.position.x,
                "y": placement.position.y,
                "z": placement.position.z,
            })
        })
        .collect();

    let report = serde_json::json!({
        "adapter": renderer.adapter(),
        "preferred_software": options.software,
        "manifest": options.manifest.display().to_string(),
        "glb": options.glb.display().to_string(),
        "glb_bytes": bytes.len(),
        "manifest_nodes": manifest.expected_nodes().len(),
        "shots": shots,
        "foups": placements,
    });
    let report_path = options.out.join("render-report.json");
    std::fs::write(&report_path, serde_json::to_string_pretty(&report)?)?;
    println!("report: {}", report_path.display());
    Ok(())
}
