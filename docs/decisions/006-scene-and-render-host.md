# ADR 006 — The 3D scene, the renderer and the native host

- **Status:** accepted
- **Date:** 2026-11-17
- **Decision:** the workbench draws the factory with **wgpu** into an offscreen
  target, reads the frame back, and shows it in a native view registered by the
  app's own crate. The scene is built from a Blender-exported **GLB plus a
  manifest**; picking is CPU-side against the manifest's boxes with the same
  camera the renderer uses.

## The model is authored offline

`tools/blender/build_fab.py` builds the four tool types part by part, the
floor, the four process bands, the AMHS ring and two templates (FOUP, wafer),
then writes:

```
crates/oxide-ui/assets/fab/oxide-fab.glb
crates/oxide-ui/assets/fab/fab-manifest.json
artifacts/blender/fx-overview.png         (review render, not shipped)
artifacts/blender/fx-tool-closeup.png
```

Blender 5.3 is an **offline** tool. Neither `bpy` nor a `.blend` is in the
runtime, the packages or CI; the committed GLB and manifest are. They live
beside the UI crate that loads them (ADR 010 moved them there from the
repository root).

### Naming and the manifest contract

```
tool.<equipment-id>              empty: the tool instance
tool.<equipment-id>.<part>       body | chamber | loadport | panel | exhaust
factory.floor
factory.band.<step-no>
factory.amhs.<n> / .post.<n>
foup.template / foup.template.handle
wafer.template
```

`fab-manifest.json` carries, in glTF space (Y up, metres). It is a **3D node
contract**, not the 2.5D layer/atlas manifest the brief rules out: it names
meshes, parts, boxes and explosion vectors, and nothing renders from PNG
layers.

- `static[]`: every non-tool mesh with its role (`floor`, `process_band` with
  `step_no`, `amhs_rail`, `amhs_post`). **No code formats a glTF node name.**
- `tools[]`: `equipment_id`, `kind`, `band`, `anchor`, and per part the node
  name, semantic part, unit explosion direction and **picking AABB**.
- `templates`: the FOUP body/handle and the wafer, with their boxes.
- `amhs`: the ring points, height and the per-step stop.
- `explode_distance`, `pick_padding`.

`oxide-scene::manifest::validate` refuses a manifest whose boxes are inverted,
whose step has no band or stop, or whose schema/coordinates are not the ones the
code reads. `oxide-render::Model::validate` then refuses a GLB that lacks any
node the manifest promises — the check that catches a re-export that renamed a
part. The committed pair is exactly 46 manifest nodes = 46 GLB meshes
(`scripts/validate-assets.sh`).

### Explosion

Each part moves by `unit_direction × explode_distance × progress` along its own
axis (chamber and exhaust up, loadport forward, panel back). The transform is
applied every frame; nothing is pre-baked into layered images.

## The renderer

`oxide-render` is one synchronous render call:

1. The scene graph (CPU) is a flat draw list of nodes: entity, glTF mesh name,
   transform, world AABB, sRGB colour, emissive, dimmed, pickable.
2. The renderer writes one camera uniform and one object uniform per node into
   a dynamic-offset buffer, then a mesh pipeline (opaque, depth `Depth24Plus`)
   and a line pipeline (grid, AMHS centre line, selection boxes, beacon).
3. A `Rgba8UnormSrgb` texture is copied into a padded read-back buffer and
   mapped; the bytes are returned as a top-down RGBA frame.

The target is capped at 1600 px a side and 1.6 megapixels; both axes share one
scale, so the picture keeps the view's aspect and picking uses the projection it
was drawn with. The frame pump ticks at 20 Hz but submits only when the scene,
camera or device-pixel size changed, or while a FOUP is in transit. While the
render thread is still on the previous job it refuses the new one, and the
viewport retries with its newest state on the next tick: a slow GPU skips frames
instead of stalling the UI, and an idle view costs no GPU work.

### Why offscreen instead of three surfaces

- **One path, three platforms.** A `CAMetalLayer`, a GTK `GdkSurface` and a
  child `HWND` are three different surface lifetimes, resize protocols and
  threading rules. The offscreen path is the same code everywhere; only the blit
  differs (below).
- **The CI screenshot is the app frame.** `oxide-render-evidence` calls the same
  `render`, so the PNG CI proves is the picture the viewport shows.
- **Input belongs to nsh.** The native view is transparent to the pointer, so
  the element's own `on_mouse_move/on_press/on_wheel` handlers drive the camera;
  no per-platform event plumbing.
- Precedent: nsh's own Shader element renders with wgpu into a texture, reads it
  back and has the backend draw the pixels (ADR 005), so this is the framework's
  own presentation technique.

The cost is a GPU→CPU copy per frame. At the cap (1600×1000, 6.4 MB) and 20 Hz
that is ~128 MB/s of memcpy, measured at 5–48 ms per frame on an Apple M2
(`artifacts/render/render-report.json`).

### The blit, per platform

| Platform | The component shows the frame as |
|---|---|
| macOS | `NSImageView` inside a pass-through `NSView` (`hitTest:` → `nil`); each frame becomes an `NSBitmapImageRep` → `NSImage` |
| Linux | `GtkPicture` painted from a `GdkMemoryTexture` (`R8g8b8a8Premultiplied`, the same format nsh's shader path uses) |
| Windows | a Direct2D bitmap created from BGRA premultiplied pixels and drawn with `DrawBitmap` into the component's box |

Frames reach the component through a **slot registry**: the prop carries only
the slot id and a frame version; the pixels stay behind a mutex in the same
process. A prop update never carries megabytes of JSON.

## Picking and the camera

`oxide-scene::Camera` is the single camera: orbit (yaw, pitch, distance),
ground-plane pan, zoom, ortho/perspective toggle, `focus(aabb)`, and a ray
through a normalised device coordinate. The renderer draws with
`view_projection(aspect)`; a click builds the ray with the **same** matrix and
tests the nodes' world AABBs (`ray_aabb`, slab method, expanded by
`pick_padding`). The hit's `Entity` is the same value the panels, the breadcrumb
and the search index use, so a click cannot select one object while the panel
shows another.

## Levels are one graph, four views

`Level::Factory | Tool | Lot | Event` changes the graph's visibility, tinting and
camera focus — not the asset, not the scene graph identity:

- L0: all tools, status cubes, FOUPs, grid, AMHS.
- L1: the focused tool's parts exploded; other tools dimmed.
- L2: the focused lot's FOUP highlighted, its 25-slot wafer stack drawn above
  the open FOUP, coloured by `wafer.state` and the defect class.
- L3: the event's subject gets a vertical beacon (the event id is the anchor in
  the side panels).

## Software rendering, honestly

- **Linux CI:** `lavapipe` via `VK_ICD_FILENAMES`; the evidence report records
  the adapter (`software: true`).
- **Windows CI:** `--software` asks for a fallback adapter, then for a
  `DeviceType::Cpu` adapter — WARP. The report records it.
- **macOS:** there is no software Metal implementation. The runner's Metal
  device is used and the report says `software: false` with the adapter name.
  Claiming otherwise would be false; the report and the release notes say so.

## Consequences

- `oxide-scene` has no GPU dependency and is unit-tested (46 node manifest,
  layout, explosion, picking, level visibility).
- `oxide-render`'s tests open a real adapter; CI installs lavapipe.
- The app embeds the GLB and manifest with `include_bytes!`/`include_str!`, so
  the packaged runner carries the assets.
- If a future nsh release lets an app feed a texture into a view, the native
  modules shrink to a thin wrapper; the scene and renderer stay as they are.
