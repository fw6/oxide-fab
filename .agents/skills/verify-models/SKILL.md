---
name: verify-models
description: "Model-check oxide-fab's write path and lot lifecycle when a change touches commit ordering, snapshots, replay, holds or terminal states. Runs the TLA+ configs whose known-bug variants must still fail."
---

# Verify the models

Two TLA+ models cover the promises unit tests reach least: the write path's
commit/snapshot ordering, and the lot lifecycle across a sequence of
transitions. Each config has a known answer, so the suite is a regression test
for the models themselves as much as for the code.

Run from the repository root:

```sh
./scripts/verify-models.sh
```

Expected: five `caught` lines for the write-path defects, two `pass` lines,
four `caught` lines for the lifecycle defects, and one `pass`. Exit `0` means
every config behaved as documented. Any other result prints the TLC output and
fails.

## When to run it

- `crates/oxide-core/src/state_machine.rs`: any transition, guard, or terminal
  state change.
- `crates/oxide-sim/src/lib.rs`: `apply_event`, `replay`, `note_event`,
  `rebuild_heap`, queue or equipment bookkeeping.
- `crates/oxide-store/src/pg.rs` or `outbox.rs`: append order, projection,
  snapshot `last_seq`.
- The design changes how a lot is held, released, split, merged, scrapped or
  completed.

## How to read a failure

Read each counterexample with the pstack `/statechart` skill. It turns TLC's
shortest path into the code's words (which action, which state, which
invariant). Do not edit the model to make a finding pass; a finding is the
result. See `docs/verification/write-path/README.md` and
`docs/verification/lot-lifecycle/README.md` for what each invariant means.

## Coverage

- Write path: `docs/verification/write-path/WritePath.tla`, models
  `Simulation` decide/commit in `crates/oxide-api/src/handlers.rs`,
  `PgStore::append_events`, `save_snapshot`/`restore_simulation` and
  `Simulation::apply_event`.
- Lot lifecycle: `docs/verification/lot-lifecycle/LotLifecycle.tla`, models
  `LotAggregate::hold|release|track_in|track_out|complete|scrap_wafer`,
  replayed through `Simulation::replay|note_event`.
- Last confirmed against the code at `40bf781` (the replay-through-the-same-
  transitions change). Move this commit forward only after running the suite
  and reading the model against the changed code.

## Not covered

Scheduling and queue order (the 8×11 transition matrix and the simulator tests
cover acceptance), KPI derivation, SECS/GEM, the TSX fold, and SQL projection
correctness. Those have their own tests.

## Updating a model

1. Edit the `.tla` or a `.cfg`, keeping one config per historical defect.
2. Run the suite. If a `caught` line becomes `pass`, the model no longer
   reproduces that defect; fix the model, not the expectation.
3. If `Fixed.cfg` fails, either the code broke the invariant or the model
   drifted from the code. Decide which before changing either.
4. Keep the model's header comment pointing at the functions it describes.
