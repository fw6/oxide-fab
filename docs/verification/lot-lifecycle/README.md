# Model check: the lot lifecycle

Checked with [everystate](https://github.com/mattzcarey/everystate) 0.4.0
through `scripts/verify-models.sh`, at `a593524`.

```sh
./scripts/verify-models.sh
```

## Status

All three defects are fixed (2026-11-23, `5f3c658`, and the terminal-state
review in `docs/rust-guidelines-review.md`). The model keeps the broken
variants so the guards stay testable.

| config | expected | what it replays |
|---|---|---|
| `AsWritten.cfg` | fails `HoldConsistent` | all three defects at once |
| `HoldLost.cfg` | fails `HoldConsistent` | `track_out` cleared the hold reason |
| `TerminalRevival.cfg` | fails `TerminalAbsorbing` | `track_in` accepted a completed lot |
| `ScrapUnlogged.cfg` | fails `LogAgrees` | the last scrap completed the lot in memory only |
| `Fixed.cfg` | passes | the current state machine |

Exit codes are TLC's: `12` is an invariant violation, `0` is a completed
search.

## What each invariant means

- `HoldConsistent`: `held <=> state = "on_hold"`. A hold is a state, not a flag
  that survives on a queued lot. `HoldLost` reaches `held = TRUE` with
  `state = "queued"` after a track-out.
- `TerminalAbsorbing`: once a lot has been completed, no transition revives it.
  `TerminalRevival` reaches a completed lot back in `processing`.
- `LogAgrees`: a lot the engine calls `completed` also has a `LotCompleted`
  event. `ScrapUnlogged` reaches `state = "completed"` with
  `completed_logged = FALSE`.

## Code this describes

- `LotAggregate::hold`, `release`, `track_in`, `track_out`, `complete`,
  `scrap_wafer` in `crates/oxide-core/src/state_machine.rs`.
- `Simulation::apply_event` and `note_event` in `crates/oxide-sim/src/lib.rs`,
  which replay the same transitions (PR #33).

## Scope and assumptions

One lot, one tool slot, no scheduling. The model checks the lifecycle rules,
not queue order or timing. The 8×11 transition matrix in
`state_machine.rs` remains the exhaustive check of accepted/refused actions;
this model is about invariants that hold across a *sequence* of transitions.
