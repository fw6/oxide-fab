# ADR 005 — Phase 2 scope: a desktop 3D workbench, three native packages

- **Status:** accepted
- **Date:** 2026-11-17
- **Decision:** phase 2 replaces the read-only dashboard with a **desktop
  application** for macOS, Windows and Linux. The 3D viewport is a wgpu scene
  hosted by an nsh **native component registered from the app's own Rust
  crate**. The web and TUI packages are dropped; the backend containers,
  `docker-compose.yml`, migrations, the outbox contract, the ops scripts and the
  GHCR chain are untouched.

This changes the UI **deliverable** (two web/TUI packages → three desktop
packages). It does not lower the bar: every phase-1 acceptance item still runs,
except that the web/TUI artefact rows are replaced by the three desktop ones
(below).

## Evidence: the nsh documentation actually read

Fetched on **2026-11-17** with `curl -sSL https://nsh.dev/<path>`; every file
returned HTTP 200.

| Document | URL | Bytes |
|---|---|---|
| Documentation index | `https://nsh.dev/llms.txt` | 63,456 |
| Native components (app-crate registration, props, events, commands) | `https://nsh.dev/docs/native-components.md` | 28,853 |
| Packaging (targets, outputs, icons) | `https://nsh.dev/docs/packaging.md` | 28,092 |
| Building with Rust | `https://nsh.dev/docs/rust.md` | 33,043 |
| Project structure (`nsh.json`, `window`, `icon`) | `https://nsh.dev/docs/project-structure.md` | 24,758 |
| macOS (AppKit, packaging) | `https://nsh.dev/docs/macos.md` | 21,729 |
| Windows (Direct2D, packaging) | `https://nsh.dev/docs/windows.md` | 14,366 |
| Linux (GTK 4, packaging) | `https://nsh.dev/docs/linux.md` | 13,502 |
| Images and assets | `https://nsh.dev/docs/images-and-assets.md` | 25,449 |
| CLI | `https://nsh.dev/docs/cli.md` | 34,686 |
| Networking (fetch, capabilities) | `https://nsh.dev/docs/networking.md` | 17,007 |
| Installation | `https://nsh.dev/docs/installation.md` | 15,707 |
| Shader element | `https://nsh.dev/elements/shader.md` | 17,414 |

Installed toolchain: `nsh 0.5.0 (f4d68f7)`; CI installs the same `v0.5.0` so
the app-crate component ABI and the CLI agree.

## What the documentation fixes for this project

### Registration is `#[native_components(<platform>)]`, not `#[native_module]`

`native-components.md` ("Registering from the app's crate") is explicit:

> A function with `#[native_components(<platform>)]` at the crate's root
> (`lib.rs`) registers them on that platform's `Components`, and the host calls
> it for every view it makes.

The platform is `windows`, `macos`, `linux` or `ios`, and the function must be
`fn(&nsh_platform_<platform>::Components)`. The macro expansion (read in
`nsh-macros-0.5.1/src/native_components.rs`) generates
`__nsh_native_components_<platform>` at the crate root and a compile-time check
that the function is not inside a module. `#[native_module("...")]`, by
contrast, only exports components/classes/modules to a **TSX** entry, which this
app does not have. The brief's `#[native_module]` wording therefore names the
wrong attribute; the implementation uses the documented one and this section is
the correction of record.

### The component contract, per platform

From `native-components.md` and the 0.5.0 sources of the platform crates:

| Platform | View | Trait | Input |
|---|---|---|---|
| macOS | `NSView` | `view() -> Retained<NSView>`, `set_prop`, `command` | the component's view fills the container; `hitTest:` returning `nil` lets nsh's own handlers receive the pointer (the documented `NshLentClip` pattern in `nsh-platform-macos/src/native.rs`) |
| Linux | `gtk::Widget` | `widget() -> gtk::Widget` | a plain `GtkPicture` takes no presses; events bubble to nsh's container |
| Windows | Direct2D `draw` or child `HWND` | `draw(&ID2D1DeviceContext, (f32, f32))`, `set_prop`, `wheel` | presses/moves go to nsh's view handlers (documented); the wheel reaches the component |

The viewport uses the **drawn** Windows variant (a Direct2D bitmap in the
component's own box), not a child window: it keeps one rendering path and the
same frame data on all three platforms.

### Packaging output is exactly the three installables

`packaging.md`:

| Target | What `nsh package` writes |
|---|---|
| `macos` | `.nsh/out/<Name>.app`, ad hoc signed |
| `linux` | `.nsh/out/<Name>/` with the executable, `nsh/assets/…`, `.desktop`, icon |
| `windows` | `.nsh\out\<Name>\` plus `<Name>.iss` and, when Inno Setup 6 is installed, `<Name>-setup.exe` |

The release therefore attaches a macOS `.zip` of the `.app`, a Linux
`.tar.gz` of the directory, and a Windows `.zip` (and `-setup.exe` when the
packager has Inno Setup). The packages are **unsigned**, which the release notes
state.

### A GLB is not an asset the nsh runtime loads

`images-and-assets.md` covers images and fonts only. The GLB, its manifest and
the WGSL are embedded with `include_bytes!`/`include_str!` in the app crate, so
they travel inside the executable the packaged runner builds; nothing depends on
`asset:/` resolution at run time.

## The Shader element cannot replace wgpu (checked, not assumed)

`elements/shader.md` defines a shader view as a **full-screen fragment
function**:

- one entry point, `fn shade(uv: vec2f) -> vec4f`, over the view's box;
- no vertex/index buffers, no depth buffer, no instancing, no camera matrices;
- inputs only from the built-ins (`nsh.size`, `nsh.time`, `nsh.scale`), a
  `struct Uniforms` of at most **4096 bytes**, and `sample_content`,
  `sample_backdrop`, `sample_from`, `sample_to` — pictures **nsh itself drew**
  (children, what is behind, a transition's sides), not arbitrary textures;
- source at most 64 KiB, frame at most 4096 px a side.

The implementation confirms the shape: `nsh-core/src/shader.rs` wraps `shade`
into `nsh_vertex`/`nsh_fragment` with bindings for built-ins (`@binding(0)`) and
uniforms (`@binding(1)`); `nsh-engine/src/shader.rs` renders that quad **with
wgpu** into a texture, reads it back and the backend draws the pixels (with an
IOSurface fast path on Apple platforms).

That is a 2D painting primitive. Loading a GLB, drawing depth-tested parts,
moving each part with its own transform matrix, switching ortho/perspective and
picking the part under a pixel are outside its interface. A raymarched SDF inside
`shade` would mean abandoning the GLB and hardcoding geometry into a uniform
block — against the requirement that the scene is constructed from data. The
workbench therefore keeps its own wgpu renderer; the Shader element is not used.

## Phase-1 acceptance item 2, re-scoped

| Phase 1 | Phase 2 |
|---|---|
| `nsh test` + web + TUI builds and runs | `nsh test` (unchanged) + **macOS, Windows and Linux packages built, run and screenshotted on their own runners** |
| `docs/screenshots/dashboard-web.png` | `docs/screenshots/workbench-*.png` and `artifacts/render/l*.png` |
| web package in the release | macOS `.zip`, Linux `.tar.gz`, Windows `.zip`/`setup.exe` |

Everything else in `docs/acceptance.md` is re-run unchanged.

## Consequences

- New crates: `oxide-scene` (manifest, layout, picking, camera) and
  `oxide-render` (GLB, wgpu, headless frames), both members of the root
  workspace so `cargo test/clippy` cover them.
- `crates/oxide-ui` stays its own Cargo workspace (nsh builds its runners) and
  gains a path dependency on both crates.
- The app crate's root `lib.rs` holds one `#[native_components(...)]` function
  per platform; nothing is registered in a module.
- CI gains a three-OS matrix that packages, runs and screenshots the app; the
  web/TUI jobs are removed.
- `nsh.json` gains the window size and the icon; `appearance` stays `"dark"`
  because the app owns its two palettes (ADR 008).
