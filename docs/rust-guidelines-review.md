# Rust guidelines review

Reviewed on 2026-11-17 against the
[Pragmatic Rust Guidelines](https://microsoft.github.io/rust-guidelines/agents/all.txt)
and the [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/checklist.html).

Scope: the six workspace crates (`oxide-core`, `oxide-sim`, `oxide-store`,
`oxide-api`, `oxide-secs`, `oxide-server`) and the nsh dashboard
(`oxide-ui`), roughly 10 000 lines of Rust.

The review had two outcomes: a lint gate that makes the guidelines mechanical,
and a set of fixes the gate then found. One structural item is left for a
decision — see [Stage 2](#stage-2-typed-event-payloads).

## 1. The gate

`Cargo.toml` now carries a `[workspace.lints]` table with the guideline's
recommended rust lints and the `cargo`, `complexity`, `correctness`, `perf` and
`pedantic` clippy groups; every crate opts in with `[lints] workspace = true`,
and the nsh workspace has the same table in its own manifest (it is a separate
workspace, so it cannot inherit ours). CI already ran
`cargo clippy --all-targets --all-features -- -D warnings`; until now that
command only enforced clippy's default set. It is now a real gate.

Six opt-outs are recorded next to the policy with their reasons — the numeric
cast family (the domain models step numbers as `i32` and counts as `usize`, so
the conversions are deliberate), `missing_errors_doc` / `missing_panics_doc`
(failure modes are documented on the error types; the lint would fire on every
axum handler), `cargo_common_metadata` (private workspace), and
`multiple_crate_versions` (a git dependency pins an older `syn`).

Lint overrides inside the code use `#[expect]` rather than `#[allow]`
(M-LINT-OVERRIDE-EXPECT), so an override that stops firing fails the build:

- `EventIds::next` was renamed to `next_id`; the `should_implement_trait`
  suppression is gone, and the name no longer imitates `Iterator::next`.
- Two `#[allow(clippy::too_many_arguments)]` covers were **stale** — the
  functions they sat on did not trip the lint. `#[expect]` would have caught
  that immediately.
- The third one was real (`track_out` took ten positional parameters) and is
  fixed below.
- `apply_event` (sim), `project` (store) and the UI's fold are long by design:
  one arm per `EventKind`, kept together so the vocabulary and its projections
  can be reviewed side by side. Each carries
  `#[expect(clippy::too_many_lines, reason = "…")]`.

## 2. Correctness found by the review

These are not style issues; they are behaviour that the guidelines' discipline
exposed.

### Unknown enum values silently became defaults

`pg.rs` mapped rows with
`EventKind::parse(&kind).unwrap_or(EventKind::LotCreated)` (and the same for
`LotState` / `WaferState` → `Created`). A row written by a newer build, or by a
hand-edited migration, would be **replayed as a lot-created event**: the
projection and the time series are then wrong, silently, and `replay` would
propagate the corruption. Unknown values are now
`StoreError::UnknownValue { table, column, value }` and the row mappers return
`Result`.

### `merge_lots` could strand lots

The source lots were removed from the map one by one; an unknown id in the
middle returned early, leaving the already-removed lots out of the simulator
while the API answered 400. Now every id is checked before anything is removed,
and the merge is all-or-nothing.

### Every operator error was a 400

`Simulation::hold_lot` and friends returned `Result<_, String>`, so
`ApiError::BadRequest` was the only possible mapping — including for a lot that
does not exist. They now return `SimError` (`UnknownLot`, `MergingIntoSelf`,
`Transition`, `Scenario`), and the API maps `UnknownLot` to **404**, illegal
transitions and self-merges to 400, and an unrunnable scenario to 500
(M-ERRORS-CANONICAL-STRUCTS: a `String` cannot carry the distinction).

`Scenario::validate` likewise returns `ScenarioError` with one variant per
inconsistency, and the tests now assert the variant rather than "is an error".

### A panic on runtime data in the SECS/GEM pair

`send_event` did `Secs2::ascii(text).expect("text")` on text built from lot ids
and alarm text, then handled `s6f11_report` errors properly one line later. A
SECS encoder refusal would have panicked the equipment thread — the pair goes
dark and the dashboard stops updating. The encode step now takes the same error
path as the rest of the message.

### GreptimeDB outages looked like "no samples"

`spc_series`, `raw_spc_series` and `table_counts` did
`self.execute(sql).await.unwrap_or_default()`. A dead time series store
therefore rendered as an empty chart with source `raw` instead of `unavailable`.
All three propagate now; `handlers::read_spc` already had the fallback chain
that consumes the error.

### Terminal lots could be revived

Writing the lifecycle matrix (§"Considered and rejected") turned up four
one-sided guards. `LotState::Completed`, `Split` and `Merged` are documented as
the end of a lot's life, but only some actions checked for all three:

- `hold`, `rework` and `complete` only refused `Completed`, so holding a split
  lot flipped it back to `on_hold`;
- `track_in` and `track_out` had no terminal check at all, so a completed lot
  could be loaded again (state back to `processing`) or un-completed by a late
  track-out;
- `merge` refused a `Processing`/`Completed` target but not `Split`/`Merged`,
  and accepted terminal sources, whose `done` wafers would move as `waiting`.

One predicate, `LotState::is_terminal()`, now backs every guard, and
`complete` additionally refuses a lot with material still on a tool — the
track-out would otherwise be refused after the lot was declared finished.

## 3. Taste and structure

### One table per fact (`string_enum!`)

Each state enum carried three parallel tables: the `serde(rename_all)` attribute,
`as_str`, and `parse`. Adding a variant meant three edits, and a missing `parse`
arm was silent because the row mapper had a default. `oxide-core/src/string_enum.rs`
generates `as_str` and `parse` from one table in the same screen as the enum
declaration, so drift is visible on review. Four enums, about seventy lines
removed.

### Ten positional parameters became a struct

`LotAggregate::track_out(step_no, equipment_id, recipe_id, recipe_checksum,
wafers, duration_ms, measured, ids, now_ms, actor)` — with `recipe_id` and
`recipe_checksum` adjacent and both `&str`, a transposition type-checks.
`TrackInReport` / `TrackOutReport` (in `events.rs`, next to the payload
constructors) make the call sites named, and the payload constructor takes the
same value the state machine validated, so the wire shape cannot drift from the
checked one.

### Errors are types

- `SimError` and `ScenarioError` replace `Result<_, String>` in `oxide-sim`
  and `oxide-core::scenario`.
- `oxide_store::outbox::publish_once` returned `Result<_, String>` while the
  crate already had `StoreError`; it returns `StoreError` now, and
  `StoreError::Greptime` replaced three `Message(e.to_string())` wrappers that
  threw the source away.
- `StoreError::is_unavailable` names the connectivity failures the API turns
  into 503. This keeps the documented chaos contract (the API answers 503
  during a PostgreSQL outage) without changing it.

### The simulator's invariants are protected

`Simulation`'s fields (`scenario`, `route`, `state`) are `pub(crate)`. The heap
must mirror the busy tools and every lot must sit in exactly one queue; any
other crate mutating `state` could break that and the tests were the only thing
relying on the exposure. `Simulation` also no longer derives `Clone` — a fat
clone of the whole engine is a footgun, and nothing used it
(cf. M-SERVICES-CLONE's advice to keep `Clone` cheap).

### Dispatch reads as a loop over outcomes

`dispatch` was a hundred-line `while` loop with `break` and `continue` doing
three different jobs. It is now `dispatch` (pick a lot, load it) plus `load_lot`
returning `ControlFlow<()>`: `Continue` = look at the queue again, `Break` =
this step cannot dispatch. `complete_step`'s tail used a closure that wrote into
a captured `events` local and then threw the result away with `.is_ok()`;
`finish_lot` returns the transition and logs a refusal.

### Smaller items

- `metrics::derive` is five functions, one per event kind, instead of a
  143-line match.
- `GreptimeDb::spc_series` and `raw_spc_series` shared their row mapping
  (`bucket_from_row` / `buckets_from_rows`).
- `MetricTable::name` → `as_str`, matching the other three enums (C-CONV).
- `/api/snapshot` carried the floor twice — top-level `steps`/`equipment`/`lots`
  *and* the same vectors inside `sim`. The payload has one copy now
  (`snapshot.sim.*`), and the UI reads it from there.
- `AppState::sim` was `Arc<Mutex<Simulation>>` inside an `Arc<AppState>`; the
  inner `Arc` is gone.
- `Lot::id()` and `Transition::events()` duplicated public fields and had no
  callers; removed. `step` handler's `let _ = count;` was dead; removed.
- Repeated `state.sim.lock().map_err(|_| ApiError::Internal(…))` became
  `lock_sim(&state)`.
- `route.clone()` before every `append_events` was unnecessary — both borrows
  are immutable.
- `Recipe::checksum_of` and `GreptimeDb::urlencode` built one `String` per byte
  with `format!`; they push nibbles into a preallocated buffer.

## 4. Deliberately not changed

- **`DomainEvent::payload: serde_json::Value`.** The largest remaining item;
  see Stage 2. Incremental accessors (`recipe_id()` was added to join
  `step_no()` / `equipment_id()`) are the interim measure.
- **`oxide-secs`' `Result<_, String>` across the thread boundary.** These are
  private functions; their only consumer is `SecsStatus::last_error`, which is a
  display string. A typed error would be stringified at the same place.
- **`require_database` pings PostgreSQL on every `/api/**` request.** It looks
  like check-then-act, but it is what keeps the documented invariant "a request
  that cannot be persisted never moves the in-memory floor". `scripts/chaos.sh`
  asserts the resulting 503. Its cost is one `SELECT 1` per request.
- **Numeric casts.** Opted out at the workspace level; see §1.
- **`spc_source: String`** on the wire (`"flow"` / `"raw"` / `"unavailable"`).
  An enum would be better; it is a documented, displayed field and changing it
  touches the UI for no behavioural gain.

## Considered and rejected: `rust-fsm`

`rust-fsm` (0.8, MIT, maintained) models `(state, input) → (state, output)`:
its DSL gives every pair exactly one target state, and outputs are declared
enum variants. The lot lifecycle is not that shape:

- edges are conditional on data — `queue` keeps `processing` while a wafer is
  on a tool — and the DSL cannot express a guarded edge;
- one transition emits several events carrying runtime payloads, which an
  output enum cannot hold;
- split and merge create or consume aggregates, which is coordination between
  machines rather than one machine's transition;
- a repeated track-out is *accepted* without changing state, which the crate's
  "transition impossible" error cannot represent;
- the aggregate is snapshotted with its state, so a machine object would either
  duplicate it or be rebuilt from it.

Manual `StateMachineImpl` impls can branch, but then the crate is a wrapper
around the `match` we already have, plus a state that must be kept in sync.

The lifecycle is pinned instead by an exhaustive matrix:
`the_transition_matrix_is_the_lifecycle_contract` in
`crates/oxide-core/src/state_machine.rs` asserts all 8 states × 11 actions
(accepted / accepted-but-unchanged / refused), and both dimensions are
compile-checked — a new `LotState` fails to compile until it gets a column, a
new action until it gets a row. That guard also closed the terminal-state
holes listed in §2.

## Stage 2: typed event payloads

If there is one structural change worth making next, it is this. The event
vocabulary is the system's single source of truth, and it is the only place
where the type system stops at the boundary:

```rust
pub struct DomainEvent {
    pub kind: EventKind,
    pub payload: Value,   // the shape lives in prose and in five readers
}
```

The same keys are re-derived by hand in five places — `Simulation::apply_event`,
`pg::project`, `metrics::derive`, `genealogy_for`, and the UI's fold — with
`.and_then(Value::as_str).unwrap_or_default()` at each site. That has three
costs:

1. A renamed or mistyped key is a silent default, not a compile error. (The
   row-mapping half of this class is fixed; the projection half is not.)
2. `EventKind` and the payload are independent: nothing stops a `track_out`
   from being constructed with a `lot_created` payload.
3. Every accessor is `Option`, so "this event kind always has a step number" is
   not expressible.

Proposal: keep JSON on the wire (the `lot_event.payload` column and the API are
unchanged), but make the payload an enum inside `oxide-core`:

```rust
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Payload {
    LotCreated { product_id: String, route_id: String, priority: i32,
                 wafer_count: i32, wafers: Vec<WaferId> },
    TrackIn(TrackInReportOwned),
    // …one variant per EventKind
}

impl DomainEvent {
    pub fn payload(&self) -> Result<&Payload, PayloadError>; // or store it typed
}
```

Two ways to land it:

- **Minimum diff**: keep `payload: Value`, add `DomainEvent::typed() ->
  Result<Payload, PayloadError>` and switch the five readers to it. Each reader
  then gets a compile error when a variant changes, and the `unwrap_or_default`
  defaults become `match` arms.
- **Clean**: store `Payload` in `DomainEvent` and serialise it into the column
  at the store boundary. One conversion instead of five parsers, but every
  payload constructor and the snapshot format move.

The first is a day's work and stops the drift; the second is the shape the
domain wants. Both need `serde` compatibility with the rows already in
PostgreSQL (they are field-for-field compatible if variant fields keep the
current key names), and both want a round-trip test that pins the wire shape.

Not purely mechanical: the UI crate folds the same events, so its payload
accessors move with the domain. Worth its own change, with the integration
suite green.

## How to verify

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace

cd crates/oxide-ui
cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --lib
```

The integration scripts (`scripts/smoke.sh`, `scripts/chaos.sh`,
`scripts/replay.sh`, `scripts/idempotency.sh`) exercise the behaviour this
review changed: the 503 contract, the snapshot shape and the 404/400 split.
