# ADR 010 — TSX interface with a Rust viewport

- **Status:** accepted; supersedes the "pure Rust" half of ADR 001
- **Date:** 2026-11-20
- **Decision:** the workbench's interface is TSX (`App.tsx`, `store.ts`,
  `components/`, `panels/`); Rust keeps the wgpu viewport, the scene/camera and
  the two desktop actions, exported through
  `#[native_module("oxide-ui/native")]`. The app is nsh's `mixed` template.

## Context

ADR 001 chose nsh's Rust template: one language, one binary, no QuickJS. That
made every interface change a Rust rebuild and restart, and the debug build of
the UI workspace — nsh's crates plus wgpu, `oxide-scene` and `oxide-render` — is
the slowest and largest part of working on the project. Panels, polling and
formatting change most often; the 3D view and its native view keep needing
Rust.

## Evidence: the documentation read

Read on **2026-11-20** (Asia/Shanghai), all HTTP 200:

| Document | What it settled |
|---|---|
| `https://nsh.dev/llms.txt`, `llms-full.txt` | Index of 264 pages and the full text |
| `docs/reactivity.md` | Components run once; `signal`/`computed`/`effect`; style expressions read signals |
| `docs/rust.md` § Extending a Rust app with TSX | `#[native_module]` + `#[module]`/`#[class]` exports, generated TS declarations, `nsh run App.tsx` hot updates |
| `docs/project-structure.md` | The `mixed` layout: `App.tsx` + `"rust": "."` |
| `docs/dev-server-and-debugging.md` | Hot module replacement: a patch per save, ~120 ms |
| `docs/testing-and-trace.md` | `nsh/test` `open`, `network` routes, `modules` stand-ins |
| `elements/view.md`, `text.md`, `scroll.md`, `input.md` | The TSX primitives the panels use |
| `modules/nsh.md` § nativeComponent | `nativeComponent(name, { props, events })` |

The `mixed` template generated with the then-installed `nsh 0.5.0` was read as
the reference layout. Verified on this machine: `nsh run crates/oxide-ui`
reports `✓ rebuilt crates/oxide-ui/App.tsx (154ms), 1 device` on a saved edit,
with the window, the camera and the 3D frame unchanged across the reload.

The app moved to **nsh 0.6.1** on 2026-11-23 (`nsh update stable`; the CLI,
`nsh-core` and the platform crates move together). 0.6 makes `application` a
capability, and an app that declares `capabilities` must grant it for its main
window: the developer host warns `the application … is not granted` without
`"application": true`, so `nsh.json` now carries it beside `network`,
`statusBar` and `process.env`. Nothing in the Rust API this app uses broke;
`nsh check`, `nsh test`, `cargo clippy -D warnings` and `nsh run` all pass on
0.6.1. 0.6 also made HTTP, WebSocket, SVG and the updater optional, which removed
around thirty transitive crates from the UI workspace.

The repository pins its Cargo registry to rsproxy.cn in `.cargo/config.toml`:
the machine-wide config replaces crates.io with the corporate Artifactory
mirror, which hung on this resolve. A project config is read after
`$CARGO_HOME`'s, so it wins for every workspace here while lock files keep the
canonical crates.io source.

## Boundaries

- **TSX owns** the shell, every panel, the store (signals, 700 ms event poll,
  actions), the guided demo, formatting and the search.
- **Rust owns** the offscreen wgpu renderer, `oxide-scene`, `Camera` and
  picking, the `oxide.view3d` native view (unchanged per platform), the save
  dialog, and `docker compose`. It exports the module `Viewport` (scene in,
  camera/picks/export out) and `Desktop` (`saveJson`, `startBackend`).
- **The boundary is `SceneInput`**: the snapshot projection plus level, focus,
  selection, explosion, alert and theme. It is the only data TSX sends to Rust;
  Rust never fetches.
- Rust state sits behind one process-wide slot, so a TSX hot reload keeps the
  camera and the render thread instead of re-framing the factory.

## Project structure

Checked against [项目结构](https://nsh.dev/zh-CN/docs/project-structure.md)
(2026-11-20). The app directory is the `mixed` layout the page describes:

```text
crates/oxide-ui/
├── App.tsx            the entry module: calls mount()
├── App.test.tsx       its tests (nsh test)
├── nsh.json           name, id, version, icon, theme, capabilities, "rust": "."
├── tsconfig.json      extends ./.nsh/tsconfig.json
├── .gitignore         .nsh/ and target/
├── components/        Shell, Viewport and the small UI kit
├── panels/            one module per domain
├── store.ts           signals, polling and actions
├── model.ts           wire DTOs, event fold, formatting
├── palette.ts         the two themes
├── src/               the Rust half: scene, viewport, engine, native views
├── assets/fab/        oxide-fab.glb, fab-manifest.json
├── Cargo.toml         the crate (its own workspace)
└── icon.png, theme.json
```

Everything on the page is met. Two points are worth recording:

- **The factory ships with the app.** The GLB and its manifest moved from the
  repository root into `crates/oxide-ui/assets/fab/`, beside the code that
  loads them; `src/assets.rs` names the two files and nothing else does.
  `oxide-scene`'s manifest test, `oxide-render`'s GLB tests and
  `oxide-render-evidence` read that same directory, so there is one copy.
  nsh's own `assets/` channel cannot carry them: `asset!()` accepts images,
  SVG and fonts, and the GLB is compiled into the binary instead of shipped as
  a UI resource — which is also why the directory is not declared in
  `#[native_module]`'s `assets!()`.
- **`icon.png` and `theme.json` sit beside `nsh.json`**, as the page's TSX
  example shows; they are named by `"icon"` and `"theme"`, not imported.

## Why a module, not a class

`Viewport` is a module of functions rather than a `#[class]`: `nsh test`'s
`modules` stand-ins replace an export or one of its members one by one, and a
nested object is handed to the app as plain JSON, so a class has no usable
stand-in. A module also matches the fact that there is exactly one viewport per
process.

## Consequences

- Interface edits hot-update; Rust edits rebuild and restart (unchanged from
  nsh's mixed-mode contract).
- Tests moved from `cargo test` (`tests/dashboard.rs`, `src/data.rs`) to
  `App.test.tsx` and `model.test.ts`: the same thirteen behaviors (eight
  interface, five event-fold/clock), a stubbed API, and the viewport/desktop
  exports stood in for. `nsh test` still runs in the `ui` CI job.
- The Windows native view now emits its wheel as `nativewheel`, because
  declaring a `wheel` native event would take `onWheel` away from macOS and
  Linux, where nsh delivers the view event. TSX handles both.
- In headless tests the native view is unregistered, so `nsh test` logs one
  `a child that is not an element …` warning for it; the real app registers it
  (`wgpu · Apple M2` in the status line).
- The app declares `capabilities` for two reasons: `process.env` is the only
  way the documented `OXIDE_*` knobs (`OXIDE_DEMO`, `OXIDE_TAB`, `OXIDE_VIEW`,
  `OXIDE_API_URL`, …) reach a TSX app, and declaring capabilities denies
  everything not listed. `network.hosts` is therefore `["*"]`: the API host is
  configurable and the container evidence runs point it at a LAN address. The
  previous Rust UI had unrestricted network too.
- Trade-off: two languages and one more typed boundary (`SceneInput`). The
  alternative — TSX for everything, including a WebGL/three.js viewport — would
  drop `oxide-render` and its render evidence, and would not match the existing
  native view.
