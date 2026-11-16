# ADR 002 — SECS/GEM: `secs4rs` over HSMS-SS, behind the `secs-gem` feature

- **Status:** accepted
- **Date:** 2026-11-16
- **Decision:** implement the host and the simulated equipment on
  [`secs4rs`](https://github.com/Asuka-00/secs4rs) at revision
  `b5a6e707e4214672417364278efc8c398d00e8a8`, with S1F1, S6F11, S5F1 and
  S7F3/S7F4/S7F5/S7F6, all four required behaviours running over a real
  HSMS-SS session on TCP.

## Context

The simulator has to speak SEMI E4/E5/E30/E37, not an internal event bus. The
brief names `secs4rs`; the risk was whether it exists and builds.

## Findings

`secs4rs` is **not on crates.io** (`GET https://crates.io/api/v1/crates/secs4rs`
returns 404, and the sparse index has no entry). The upstream is
`github.com/Asuka-00/secs4rs`, Apache-2.0, described as an idiomatic Rust SECS
stack based on `secs4java8`, with `#![forbid(unsafe_code)]` and **zero runtime
dependencies**. It ships `secs2`, `hsms` / `hsms_ss` / `hsms_gs`, `secs1`,
`gem` (S1/S2/S5/S6/S7/S9/S10/S13) and `sml`.

It builds with the pinned toolchain:

```text
$ cargo build -p secs4rs
   Compiling secs4rs v0.1.0 (/private/tmp/secs4rs/crates/secs4rs)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 2.52s
```

So no substitute implementation was needed: the brief's first choice works.

## Design

- `crates/oxide-secs` depends on the Git revision (pinned, not a floating
  branch) and exposes `start_pair(SecsConfig)`.
- The **equipment is passive** (`open_passive`) and the **host is active**
  (`open_active`), both on `127.0.0.1:50010` by default (`SECS_BIND`).
- The equipment replies to S1F1 with S1F2, to S1F13/S1F17 with the GEM
  acknowledgements, stores S7F3 programs and answers S7F4; it reports S6F11
  collection events and S5F1 alarms, and answers S7F5 with the stored program
  (S7F6) so the host can compare its SHA-256.
- The host runs a **reply thread** of its own: a single-threaded host deadlocks
  when it waits for S7F4 while the equipment waits for S6F12. This was found
  by the integration test, not by inspection.
- Collection events and alarms go through an **equipment spool**; while the
  link is down the spool grows, and the first thing the equipment does after a
  re-select is replay it (the S2F43/S6F23 idea, in simulation).

The four behaviours plus spooling are one integration test:

```text
$ cargo test -p oxide-secs --test four_behaviors
running 1 test
test the_four_gem_behaviours_run_on_a_real_hsms_link ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.67s
```

The test asserts S1F2's MDLN/SOFTREV, the S6F11/S6F12 round trip, the
S5F1/S5F2 alarm, the S7F3/S7F4 download with an S7F5/S7F6 checksum match, then
drops the link, spools one event, and checks it is replayed after the
reconnect (`spooled_replayed == 1`, `s6f11_sent == 2`), and that the traffic
log contains S1F1, S1F2, S1F17, S1F18, S6F11, S6F12, S5F1, S5F2, S7F3, S7F4,
S7F5 and S7F6.

## The feature

`secs-gem` (default on, also on in `oxide-server`) gates the runtime module.
The server enables it through `oxide-server`'s own `secs-gem` feature and
starts the pair unless `SECS_ENABLED=false`. CI builds with `--all-features`,
so the protocol layer is always compiled and tested there.

## Consequences

- Pinning a Git revision means CI needs network access for that repository; it
  is public and Apache-2.0.
- `secs4rs` uses blocking TCP and threads; the pair therefore lives on two
  dedicated threads and talks to the async server through channels and shared
  status, not inside a Tokio task.
- The spool is bounded only by the demo's event rate; a production EAP would
  page on depth (see the known limitations in the README).
