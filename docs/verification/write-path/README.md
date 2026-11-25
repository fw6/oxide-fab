# Model check: the oxide-fab write path

Verified with [everystate](https://github.com/mattzcarey/everystate) 0.4.0
(`npx everystate`), against `bdef8c0`.

`./scripts/verify-models.sh` runs these configs with their expected answer and
is a CI job. A config that used to reproduce a defect must still fail on the
named invariant; `Fixed.cfg` must still pass.

```sh
cd docs/verification/write-path
npx everystate WritePath.tla                              # code as written: fails
npx everystate -config LegalLog.cfg WritePath.tla         # write order
npx everystate -config Recovery.cfg WritePath.tla         # snapshot of uncommitted work
npx everystate -config FailedAppend.cfg WritePath.tla     # failed append, no rollback
npx everystate -config SnapshotRace.cfg WritePath.tla     # capture/read gap
npx everystate -config EngineRace.cfg WritePath.tla       # track-in after hold
npx everystate -config FixedWrites.cfg WritePath.tla      # write fix only: pass
npx everystate -config Fixed.cfg WritePath.tla            # both fixes: pass
```

Exit codes are TLC's: `12` = an invariant is violated, `0` = every reachable
state checked.

## Status

The findings are fixed (2026-11-19):

- Decisions and appends now share `write_lock` for the simulator and every
  operator action, and a failed operator append restores the state copy. This
  is `AtomicCommit = TRUE`.
- `SimState` carries the `lot_event.seq` it reflects; `save_snapshot` copies
  the state and that sequence number in one critical section. This is
  `SnapshotAtomic = TRUE`. `append_events` returns the sequence of an event
  that is already present, so a retried batch cannot make a snapshot skip it.
- The simulator's failed batch is gated: while it is uncommitted, operator
  writes, `/api/snapshot` and what-if answer 503, and no snapshot is taken.
- `write_lock` also serialises `save_snapshot` with `/admin/reset`; a
  `delivery_lock` serialises the publisher with the truncates in reset and
  replay.

`FixedWrites.cfg` and `Fixed.cfg` describe the shipped design. The failing
configs still model the code at `bdef8c0` and remain as the regression record.

One boundary remains: an operator append that returns an error after the
server committed the transaction is treated as not committed (the state copy
is restored). Resolving that case needs a retryable command (an idempotency key
or a single writer queue), which is a larger change than the race fix; the
simulator path already retries the exact batch and uses the duplicate-sequence
lookup.

## The model

One lot aggregate with three transitions: `Hold` (waiting/processing →
on_hold), `Release` (on_hold → waiting), `TrackIn` (waiting → processing).
Three writers: the simulator task and two operator HTTP requests.

The `Decide`/`Commit` split below describes `bdef8c0`, the code the failing
configurations model. The shipped code makes the pair one locked step and
reads the snapshot's sequence number with its state (`FixedWrites.cfg`,
`Fixed.cfg`).

| Model action | Code it stands for |
|---|---|
| `Decide(w)` | The block under `write_lock` + `sim.lock` in `hold_lot`/`release_lot`/`rework_lot`/... (`crates/oxide-api/src/handlers.rs:421-490`), and `sim.tick()` in the simulator task (`bins/oxide-server/src/main.rs:241-250`). The aggregate mutates in memory and returns events. |
| `Commit(w)` | `PgStore::append_events` (`crates/oxide-store/src/pg.rs:245`), called from `finish_action` (`handlers.rs:499`) and from the simulator loop (`main.rs:252`). Separate from Decide; two writers' commits are unordered. |
| `Fail(w)` | `append_events` returns `Err`. `finish_action` returns the error but never undoes the aggregate mutation already made (`handlers.rs:499-500`). |
| `SnapshotCapture` / `SnapshotReadSeq` | `save_snapshot`: state copied under the sim lock (`main.rs:323-326`), then `max_seq()` is a separate query after the lock is released (`main.rs:334`). |
| `Restart` | `restore_simulation` (`main.rs:199-215`): snapshot state, then replay `lot_event` where `seq > last_seq`; `Simulation::apply_event` does not re-check legality. |

Promises:

1. **`LegalLog`** — the committed log is a history the aggregate accepts.
2. **`RecoveryMatches`** — after a restart the engine reflects exactly the
   committed log.
3. **`NoTrackInAfterHold`** — the engine's track-in never lands after a hold it
   raced with (the restart-harmful case of 1).

Bounds: `MaxEvents = 3`, 3 writers, 3 lot states. The searches are exhaustive
at those bounds (151–316 distinct states). Model values are abstract; the
search is over all interleavings, not over time or quantities.

## Findings

### F1 — decisions are serialized, commits are not, so the log can be illegal

`write_lock` is held only while the aggregate decides the transition. The
guard is dropped before `finish_action` appends, and the simulator never takes
`write_lock` at all. Two writers can therefore commit in the opposite order to
their decisions.

`LegalLog.cfg`, depth 4:

```
Decide("op1")   pending.op1 = "Hold",    mem = "on_hold"
Decide("op2")   pending.op2 = "Release", mem = "waiting"
Commit("op2")   log = <<"Release">>          <-- Release from "waiting"
```

The aggregate refuses `Release` from `waiting`. In the decision order the
operators saw `Hold` then `Release`, ending `waiting`; the log says the
opposite and ends `on_hold`. A restart replays the log and re-holds the lot the
operator released.

`EngineRace.cfg`, depth 5, is the same bug with the simulator:

```
Decide("engine")  pending.engine = "TrackIn", mem = "processing"
Decide("op1")     pending.op1 = "Hold",       mem = "on_hold"
Commit("op1")     log = <<"Hold">>
Commit("engine")  log = <<"Hold", "TrackIn">>
```

The live engine has the lot `on_hold`; the durable log has `TrackIn` after the
hold. On restart, `apply_event` sets the lot to `Processing` unconditionally
(`crates/oxide-sim/src/lib.rs:533-548`) while the SQL projection deliberately
keeps it `on_hold` (`crates/oxide-store/src/pg.rs:704-707`, `WHERE state <>
'on_hold'`). The hold is silently lost and the lot can be dispatched again,
with new event ids, so GreptimeDB's key dedupe does not absorb it.

Why the two commits can invert: separate HTTP tasks each `await
append_events`, and PostgreSQL allocates the `seq` of two unrelated inserts in
whichever order they arrive. A process-local `write_lock` would not have fixed
the second half of the problem anyway — the simulator does not take it.

### F2 — the snapshot can persist a decision that was never committed

`save_snapshot` copies `sim.snapshot()` without checking whether the writer
that produced that state has committed. Two reachable paths:

- **Operator.** `hold_lot` mutates the in-memory lot under `sim.lock`, releases
  both locks, and only then appends. `save_snapshot` can copy the state in that
  window. `Recovery.cfg`, depth 6:

  ```
  Decide("op1")      pending.op1 = "Hold", mem = "on_hold"
  SnapshotCapture    snap_state = "on_hold"
  SnapshotReadSeq    snap_last = 0
  SnapshotWrite      snapshot = [state |-> "on_hold", last |-> 0]
  Restart            mem = "on_hold", log = <<>>
  ```

  No `LotHeld` event exists, but after a restart the lot is held. This also
  happens if the append fails and the client gives up: the API returned an
  error, the engine kept the change, and the snapshot made it durable.

- **Engine retry.** If `append_events` fails, the simulator keeps the batch in
  `pending` and retries, but the loop still reaches the snapshot cadence check
  (`main.rs:251-287`). `FailedAppend.cfg`, depth 7: `TrackIn` is decided, the
  append fails, the snapshot captures `processing` with `last = 0`, and the
  restart believes the tool is running with an empty log.

This contradicts the architecture contract that PostgreSQL is the single
source of truth: an effect with no `lot_event` row survives a restart.

### F3 — a failed append leaves the engine ahead of the log, and the retry lies

`finish_action` does `state.store.append_events(...).await?`; the `?` returns
the error, leaving the in-memory mutation in place. The failure is reported as
`503` (`handlers.rs:42-53`), so the client retries. `LotAggregate::hold` on an
already-held lot returns an empty transition (`crates/oxide-core/src/
state_machine.rs:284-286`), so the retry succeeds with `events = 0`:

```
POST /api/lots/LOT-0001/hold  ->  503   (in-memory hold applied, no event)
POST /api/lots/LOT-0001/hold  ->  200   {"events": 0}
```

The operator believes the hold was recorded. It was not. `release` and
`rework` fail differently (400, or a second rework), none of which tells the
client that the first write never committed.

### F4 — the snapshot's state and sequence number are read at different times

`save_snapshot` copies the state, releases the sim lock, serializes, and then
reads `max_seq()`. An operator can decide and commit in that gap. The snapshot
then carries a `last_seq` that covers an event its state does not contain.

`SnapshotRace.cfg` (snapshots only between batches, failed appends off, so the
only possible cause is the gap), depth 7:

```
SnapshotCapture  snap_state = "waiting"      (log empty)
Decide("op1")    pending.op1 = "Hold", mem = "on_hold"
Commit("op1")    log = <<"Hold">>
SnapshotReadSeq  snap_last = 1
SnapshotWrite    snapshot = [state |-> "waiting", last |-> 1]
Restart          mem = "waiting"
```

The committed hold is skipped by recovery: `restore_simulation` starts at
`seq > 1` and never sees it. This breaks the documented RPO 0 for committed
events. With a `TrackIn` instead of a `Hold`, the restart also forgets that a
tool is running: the snapshot's equipment state says idle, `rebuild_heap`
schedules nothing, and the lot is dispatched a second time.

### Observed by reading, not model-checked — reset vs. the publisher

`reset` truncates PostgreSQL and GreptimeDB while the `Publisher` task is
still running (`handlers.rs:702-718`; no shutdown of `publisher.run`). A batch
already claimed by the publisher (`claim_outbox`, `pg.rs:490`) is held in
memory, so this interleaving leaves old points in the fresh demo:

1. Publisher claims a batch from the old log.
2. `reset` truncates GreptimeDB.
3. Publisher inserts the claimed rows into the truncated store.
4. The fresh simulator regenerates the same deterministic event ids with new
   timestamps, which are not deduplicated (ADR 003: the timestamp is part of
   the storage key).

The code explicitly truncates on reset to avoid stale points, so this is a
violation of that intent. It is not in the model; it would need a
publisher/reset spec.

## What the model leaves out

- `rework`, `split`, `merge`, `scrap`, and equipment dispatch. They go through
  the same `Decide`/`Commit` split, but their multi-event transitions and heap
  effects are not modelled.
- The outbox itself (at-least-once + `event_id` dedupe). ADR 003 covers it, and
  `claim_outbox` without a lease is safe for the data because the payload is
  immutable; the dashboard's `last_error` can still be written by a loser.
- Time, quantities, multi-process deployments. The lock is process-local, so a
  second server instance is outside the model (and outside the current
  deployment).
- `greptime insert` failures inside the publisher; the message loss cases the
  skill normally asks for do not change the ordering argument here.

## Fixes and how the model confirms them

1. **One writer at a time, through the append.** Hold `write_lock` across the
   decision *and* `append_events`, and make the simulator take the same lock
   around its tick + append. `AtomicCommit = TRUE` in the model. This makes the
   log order equal the decision order; `FixedWrites.cfg` passes. **Implemented.**
2. **Read the sequence number with the state.** Capture the state and
   `last_seq` in one critical section, store the last committed `seq` in
   `SimState` next to the state, and snapshot that. `SnapshotAtomic = TRUE`;
   `Fixed.cfg` passes both invariants. **Implemented**, together with
   `append_events` returning the sequence of an event that is already present.
3. **Do not keep a state the log does not contain.** The operator path copies
   the state before deciding and restores it if the append fails; the simulator
   keeps its exact batch and is gated while it retries. **Implemented** as
   copy-and-restore rather than compute-on-a-copy.
4. **Make replay validate.** `Simulation::apply_event` still applies an
   illegal order silently. Returning a `Result` and refusing a log the aggregate
   would not accept would turn a silent divergence into a loud failure. **Not
   implemented**; it is a follow-up, useful even after 1–3 as a corruption
   check.

The first three are in `handlers.rs`, `oxide-server`, `oxide-store` and
`oxide-sim`. `cargo test -p oxide-sim` covers the restore/sequence helpers;
`cargo clippy --workspace --all-targets` is clean.

## Files

- `WritePath.tla` — the model.
- `WritePath.cfg` — code as written at `bdef8c0` (all invariants).
- `LegalLog.cfg`, `EngineRace.cfg`, `Recovery.cfg`, `FailedAppend.cfg`,
  `SnapshotRace.cfg` — one finding each.
- `FixedWrites.cfg`, `Fixed.cfg` — the shipped fixes pass.
