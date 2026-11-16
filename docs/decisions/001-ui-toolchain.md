# ADR 001 — UI toolchain: nsh (Native Shell), pure Rust mode

- **Status:** accepted
- **Date:** 2026-11-16
- **Decision:** build `oxide-ui` as an nsh app in its **Rust** template
  (`nsh init --template rust`), with no TSX entry and no QuickJS in the app.

## Context

The dashboard must be Rust end to end and must run both as a web page and in a
terminal. nsh is the only candidate that renders Rust components natively on
web, desktop and a TUI with one code base.

## Evidence: the documentation actually read

Fetched on **2026-11-16 between 19:22 and 19:23 Asia/Shanghai (11:22–11:23
UTC)**, with `curl -sSL https://nsh.dev/<path>`; every file below returned
HTTP 200 and was read in full before any UI code was written.

| Document | URL | Bytes |
|---|---|---|
| Documentation index (the authoritative entry point) | `https://nsh.dev/llms.txt` | 63,456 |
| Full documentation | `https://nsh.dev/llms-full.txt` | 2,792,218 |
| Building with Rust | `https://nsh.dev/docs/rust.md` | 33,043 |
| Native components | `https://nsh.dev/docs/native-components.md` | 28,853 |
| Reactivity | `https://nsh.dev/docs/reactivity.md` | 21,099 |
| Layout | `https://nsh.dev/docs/layout.md` | 16,879 |
| Testing and trace | `https://nsh.dev/docs/testing-and-trace.md` | 64,493 |
| Networking | `https://nsh.dev/docs/networking.md` | 17,007 |
| CLI | `https://nsh.dev/docs/cli.md` | 34,686 |
| TUI | `https://nsh.dev/docs/tui.md` | 22,630 |
| Web | `https://nsh.dev/docs/web.md` | 43,297 |
| Fonts | `https://nsh.dev/docs/fonts.md` | 11,458 |
| Getting started | `https://nsh.dev/docs/getting-started.md` | 21,747 |
| Project structure | `https://nsh.dev/docs/project-structure.md` | 24,758 |
| Styling | `https://nsh.dev/docs/styling.md` | 33,342 |
| Text | `https://nsh.dev/docs/text.md` | 15,788 |
| Packaging | `https://nsh.dev/docs/packaging.md` | 28,092 |

`llms.txt` is itself the index: it lists 264 `https://nsh.dev/...` pages, and
the rows above are the subset this project needed (Rust, components,
reactivity, layout, testing, networking, CLI, TUI, web, fonts, packaging).

The installed toolchain is `nsh 0.5.0 (f4d68f7)`; CI installs `v0.5.1`
(`nsh 0.5.1 (224d9a7)`), and both were used while building this app.

## What the documentation fixes for this project

From `rust.md` ("An app written in Rust"):

```sh
nsh init my-app --template rust
cd my-app
nsh run                        # --target ios, android, web, macos, linux or windows
nsh package --target web       # or ios, ios-simulator, android, macos, linux, windows
```

and "A Rust app's `"root"` is a component of the first crate listed", with
`nsh.json` holding `"rust": "."` and `"root": "App"`.

From `tui.md` ("Running an app", "Testing"):

```sh
nsh run --target tui
nsh test --target tui     # in-memory terminal, needs no terminal of its own, runs in CI
```

From `cli.md` ("nsh test"):

```sh
nsh test [PATH] [--filter <NAME>] [--trace] [--fps] [--junit <FILE>] [--target <TARGET>] ...
```

with the sentence that decides our CI call: *"Given a directory of an app with
Rust crates of its own (`nsh.json` `"rust"`: a Rust app or a TSX app with a
crate), it first runs `cargo test` for each crate in the directory, in the
crate's own directory, and shows and counts each Rust test with the TypeScript
tests… a Rust app with only Rust tests is tested this way too."*

From `testing-and-trace.md` ("Tests in Rust with cargo test"):

> `nsh test` runs these tests too: given the directory of an app with Rust
> crates (`nsh.json` `"rust"`), it runs `cargo test` for each crate in it
> before the test files, and shows and counts the Rust tests with the
> TypeScript ones (`✓ src/lib.rs › tests::adds`), in its summary and its
> `--junit` report. So `nsh test` is the one command for a Rust app.

That is why the UI's tests are **Rust tests** (`crates/oxide-ui/src/data.rs`,
`crates/oxide-ui/tests/dashboard.rs`) and CI runs
`nsh test crates/oxide-ui --junit artifacts/nsh-tests.xml` — `nsh test`
executes the crate's `cargo test` and writes the JUnit report CI displays.

From `networking.md` ("From Rust components"): `fetch` is available in Rust
components, takes a URL or an `HttpRequest`, runs in a task, and is cancelled
when the component goes away. The dashboard uses it for `/api/snapshot`,
`/api/events` and the operator commands.

From `reactivity.md`: components run once; signals notify only their readers;
`each(items, key, row)` keys rows; only synchronous reads are tracked. That is
the model the incremental dashboard is built on (see `oxide-ui/src/lib.rs`).

From `fonts.md` and `rust.md` (Theme): the app's font comes from the theme's
`font-sans`, so `crates/oxide-ui/theme.json` sets
`WenQuanYi Micro Hei, Noto Sans CJK SC, …` and every text inherits it. The TUI
backend converts CJK characters to two Cells, which keeps the columns aligned.

## The TUI is not an nsh package (checked, not assumed)

The brief asks for an nsh package for `--target tui`. nsh cannot produce one:

```text
$ nsh package crates/oxide-ui --target tui      # 0.5.0 and 0.5.1 alike
error: invalid value 'tui' for '--target <TARGET>'
  [possible values: ios, ios-simulator, android, linux, macos, windows, web]
```

`tui` is a built-in *run/test* target, so the CLI validates and rejects it
before the documented external-target mechanism (`nsh-platform-<name>`) is
consulted; a `nsh-platform-tui` program on `PATH` is never reached. The latest
CLI documentation (`https://nsh.dev/docs/cli.md`, read 2026-11-17) still lists
the same seven package targets, and the nightly release of 2026-11-16 ships the
same CLI. There is therefore no published nsh version that supports
`nsh package --target tui`.

What the release ships instead, and how the TUI is verified:

- `oxide-ui-<version>-tui.tar.gz` contains the app description (`app.json`)
  and the crate the terminal host mounts; `cd app && nsh run --target tui`
  runs it with an installed nsh.
- `nsh test crates/oxide-ui --target tui --junit artifacts/nsh-tui-tests.xml`
  builds the terminal development host and runs the app through the terminal
  renderer in an in-memory terminal (the CI-safe path documented in
  `testing-and-trace.md`), which is the TUI target being built and run.

## Evidence produced by this repository

| What | Command | Evidence |
|---|---|---|
| Rust + TSX UI tests (10 tests: 9 Rust, 1 TSX) | `nsh test crates/oxide-ui --junit artifacts/nsh-tests.xml` | `artifacts/nsh-tests.xml` |
| Web target built and run in headless Chrome | `nsh test crates/oxide-ui --target web --junit artifacts/nsh-web-tests.xml` | `artifacts/nsh-web-tests.xml`, `artifacts/ui-web.log` |
| Web dev server serves the app | `nsh run --target web --port 8123` → `GET /` returns 200 | `artifacts/ui-web-run.log` |
| Web package (what the release ships) | `nsh package crates/oxide-ui --target web -o /tmp/oxide-web` | `artifacts/ui-web-package.log`, `docs/screenshots/dashboard-web.png` |
| TUI target built and run in an in-memory terminal | `nsh test crates/oxide-ui --target tui --junit artifacts/nsh-tui-tests.xml` | `artifacts/nsh-tui-tests.xml`, `artifacts/ui-tui.log` |
| TUI development host launches and connects | `nsh run --target tui` | `artifacts/ui-tui-host.log` |

`nsh run --target tui` was started under a PTY; the host builds and its dev
session connects, but this automation environment has no user-facing terminal,
so the interactive screen itself was not captured. The TUI renderer is
verified by the `--target tui` run above, which draws the same `App` through
ratatui in an in-memory terminal and asserts the panel texts.

## Consequences

- `crates/oxide-ui` is its own Cargo workspace (`[workspace]`), as the
  `--template rust` scaffold writes it; the root workspace excludes it so
  `cargo test --workspace` does not try to build nsh's runners.
- `nsh run --target web` and `nsh run --target tui` both mount the same
  `App` component; evidence paths are in `docs/screenshots/` and
  `artifacts/ui-*.log`.
- `nsh test` is a CI gate of its own; `cargo check -p oxide-ui` is not a
  substitute.
