---------------- MODULE WritePath ----------------
\* A model of oxide-fab's write path, not of the whole system.
\*
\* Code this models (as of bdef8c0):
\*
\*   Decide  = the part inside `state.write_lock` + `state.sim.lock()`:
\*             `Simulation::hold_lot|release_lot|rework_lot|...`
\*             (crates/oxide-api/src/handlers.rs:427-490) and the whole
\*             `sim.tick()` (bins/oxide-server/src/main.rs:241-250).
\*             The aggregate mutates in memory and returns the events.
\*
\*   Commit  = `PgStore::append_events` (crates/oxide-store/src/pg.rs:245).
\*             Called by `finish_action` (handlers.rs:492-500) and by the
\*             simulator task (main.rs:251). This is a separate step from
\*             Decide, and nothing orders two writers' commits.
\*
\*   Fail    = `append_events` returns Err. `finish_action` returns the error
\*             but does NOT roll back the in-memory mutation the aggregate
\*             already made (handlers.rs:499-500), so the effect stays in `mem`.
\*
\*   Snapshot = `save_snapshot` (main.rs:320-339): the state is copied under
\*             the sim lock, then `max_seq()` is read as a separate query.
\*             A writer can decide and commit between those two reads.
\*
\*   Restart = `restore_simulation` (main.rs:197-215): state = snapshot.state,
\*             then replay `lot_event` rows with `seq > snapshot.last_seq`.
\*             `Simulation::apply_event` does not re-check legality.
\*
\* The lot is one aggregate with the three transitions that make the
\* interleavings visible. The promise checked is the one the design claims:
\*
\*   (1) LegalLog        the committed log is a history the aggregate accepts
\*   (2) RecoveryMatches a restart reflects exactly the committed log
\*
\* AtomicCommit = TRUE models the proposed fix: decide and append under one
\* lock, in the same order for every writer.
\* SnapshotAtomic = TRUE models reading the sequence number in the same
\* critical section as the state copy.

EXTENDS Naturals, Sequences, TLC

CONSTANTS MaxEvents, AtomicCommit, SnapshotAtomic, SnapshotQuiet, FailAllowed, RestartAllowed

Writers == {"engine", "op1", "op2"}
Events == {"Hold", "Release", "TrackIn"}
\* What each writer can decide: the engine dispatches, operators hold/release.
WriterEvents(w) == IF w = "engine" THEN {"TrackIn"} ELSE {"Hold", "Release"}
States == {"waiting", "on_hold", "processing"}
Snapshots == {[state |-> s, last |-> n] : s \in States, n \in 0..MaxEvents}
NoSnapshot == [state |-> "none", last |-> 0]

VARIABLES
    mem,            \* the lot state the single process believes
    pending,        \* Writers -> an event decided but not yet committed
    log,            \* committed `lot_event`, in `seq` order
    snapshot,       \* NoSnapshot or the saved [state, last_seq]
    snap_state,     \* state copied by `sim.snapshot()`
    snap_last,      \* sequence number stored with it
    snap_phase,     \* where save_snapshot is between its two reads
    lastWasRestart, \* the state immediately after Restart
    engineFailed,   \* the engine's append failed and its batch is retrying
    moves

Enabled(s, e) ==
    CASE e = "Hold"    -> s \in {"waiting", "processing"}
      [] e = "Release" -> s = "on_hold"
      [] e = "TrackIn" -> s = "waiting"

\* What the aggregate does to `mem`, and what `apply_event` does on replay.
Step(s, e) ==
    CASE e = "Hold"    -> "on_hold"
      [] e = "Release" -> "waiting"
      [] e = "TrackIn" -> "processing"

\* The guarded transition used to decide legality of the committed log.
LegalStep(s, e) ==
    IF s = "illegal" THEN "illegal"
    ELSE IF Enabled(s, e) THEN Step(s, e)
    ELSE "illegal"

RECURSIVE LegalAfter(_, _, _)
LegalAfter(s, l, n) ==
    IF s = "illegal" THEN "illegal"
    ELSE IF n >= Len(l) THEN s
    ELSE LegalAfter(LegalStep(s, l[n + 1]), l, n + 1)

\* The projection a restart performs: no legality check.
RECURSIVE FoldAfter(_, _, _)
FoldAfter(s, l, n) ==
    IF n >= Len(l) THEN s
    ELSE FoldAfter(Step(s, l[n + 1]), l, n + 1)

Init ==
    /\ mem = "waiting"
    /\ pending = [w \in Writers |-> "none"]
    /\ log = <<>>
    /\ snapshot = NoSnapshot
    /\ snap_state = "none"
    /\ snap_last = 0
    /\ snap_phase = "idle"
    /\ lastWasRestart = FALSE
    /\ engineFailed = FALSE
    /\ moves = 0

\* `save_snapshot` runs in the simulator task, so the engine cannot decide a
\* new batch while its own snapshot is between its two reads. Operators can:
\* their handlers are separate tasks.
EngineIdle == snap_phase = "idle"

\* Decide only. The event waits for a separate Commit, as in the code.
Decide(w) ==
    /\ ~AtomicCommit
    /\ (w /= "engine" \/ EngineIdle)
    /\ pending[w] = "none"
    /\ moves < MaxEvents
    /\ \E e \in WriterEvents(w) :
         /\ Enabled(mem, e)
         /\ mem' = Step(mem, e)
         /\ pending' = [pending EXCEPT ![w] = e]
    /\ moves' = moves + 1
    /\ engineFailed' = IF w = "engine" THEN FALSE ELSE engineFailed
    /\ lastWasRestart' = FALSE
    /\ UNCHANGED <<log, snapshot, snap_state, snap_last, snap_phase>>

\* Decide and append in one step: the proposed fix.
AtomicDecide(w) ==
    /\ AtomicCommit
    /\ (w /= "engine" \/ EngineIdle)
    /\ pending[w] = "none"
    /\ moves < MaxEvents
    /\ \E e \in WriterEvents(w) :
         /\ Enabled(mem, e)
         /\ mem' = Step(mem, e)
         /\ log' = Append(log, e)
    /\ moves' = moves + 1
    /\ lastWasRestart' = FALSE
    /\ UNCHANGED <<pending, snapshot, snap_state, snap_last, snap_phase, engineFailed>>

\* Any pending event can commit next: the writers' commits are unordered.
Commit(w) ==
    /\ ~AtomicCommit
    /\ pending[w] /= "none"
    /\ log' = Append(log, pending[w])
    /\ pending' = [pending EXCEPT ![w] = "none"]
    /\ engineFailed' = IF w = "engine" THEN FALSE ELSE engineFailed
    /\ lastWasRestart' = FALSE
    /\ UNCHANGED <<mem, moves, snapshot, snap_state, snap_last, snap_phase>>

\* append_events failed: the event is dropped, the in-memory effect is not.
Fail(w) ==
    /\ FailAllowed
    /\ ~AtomicCommit
    /\ pending[w] /= "none"
    /\ pending' = [pending EXCEPT ![w] = "none"]
    /\ engineFailed' = IF w = "engine" THEN TRUE ELSE engineFailed
    /\ lastWasRestart' = FALSE
    /\ UNCHANGED <<mem, log, moves, snapshot, snap_state, snap_last, snap_phase>>

\* `sim.snapshot()` under the sim lock.
SnapshotCapture ==
    /\ snap_phase = "idle"
    \* SnapshotQuiet: isolate the capture/read gap by only snapshotting
    \* between batches. The code does not enforce this; Recovery.cfg models
    \* the code as written (snapshots can race a failed append).
    /\ (SnapshotQuiet => (\A w \in Writers : pending[w] = "none"))
    \* A snapshot can only see the engine ahead of the log when its append
    \* failed and the batch is being retried; it cannot see a batch the engine
    \* has not yet tried to append (same task).
    /\ (pending["engine"] = "none" \/ engineFailed)
    /\ snap_state' = mem
    /\ snap_last' = IF SnapshotAtomic THEN Len(log) ELSE 0
    /\ snap_phase' = IF SnapshotAtomic THEN "read" ELSE "captured"
    /\ lastWasRestart' = FALSE
    /\ UNCHANGED <<mem, pending, log, snapshot, engineFailed, moves>>

\* `state.store.max_seq().await`, a separate query.
SnapshotReadSeq ==
    /\ ~SnapshotAtomic
    /\ snap_phase = "captured"
    /\ snap_last' = Len(log)
    /\ snap_phase' = "read"
    /\ lastWasRestart' = FALSE
    /\ UNCHANGED <<mem, pending, log, snapshot, snap_state, engineFailed, moves>>

SnapshotWrite ==
    /\ snap_phase = "read"
    /\ snapshot' = [state |-> snap_state, last |-> snap_last]
    /\ snap_phase' = "idle"
    /\ lastWasRestart' = FALSE
    /\ UNCHANGED <<mem, pending, log, snap_state, snap_last, engineFailed, moves>>

\* Crash and restart: snapshot state, then every later log row. A pending
\* (uncommitted) event is lost, which is the documented contract.
Restart ==
    /\ RestartAllowed
    /\ snapshot /= NoSnapshot
    /\ mem' = FoldAfter(snapshot.state, log, snapshot.last)
    /\ pending' = [w \in Writers |-> "none"]
    /\ lastWasRestart' = TRUE
    /\ engineFailed' = FALSE
    /\ UNCHANGED <<log, snapshot, snap_state, snap_last, snap_phase, moves>>

Next ==
    \/ \E w \in Writers : Decide(w)
    \/ \E w \in Writers : AtomicDecide(w)
    \/ \E w \in Writers : Commit(w)
    \/ \E w \in Writers : Fail(w)
    \/ SnapshotCapture
    \/ SnapshotReadSeq
    \/ SnapshotWrite
    \/ Restart

Spec == Init /\ [][Next]_<<mem, pending, log, snapshot, snap_state, snap_last, snap_phase, lastWasRestart, engineFailed, moves>>

TypeOK ==
    /\ mem \in States
    /\ pending \in [Writers -> Events \cup {"none"}]
    /\ log \in Seq(Events)
    /\ snapshot \in Snapshots \cup {NoSnapshot}
    /\ snap_state \in {"none"} \cup States
    /\ snap_last \in 0..MaxEvents
    /\ snap_phase \in {"idle", "captured", "read"}
    /\ lastWasRestart \in BOOLEAN
    /\ engineFailed \in BOOLEAN
    /\ moves \in 0..MaxEvents

\* (1) The committed log never contains a transition the aggregate refuses.
LegalLog == LegalAfter("waiting", log, 0) /= "illegal"

\* (2) After a restart, the engine reflects exactly the committed log.
RecoveryMatches == lastWasRestart => mem = FoldAfter("waiting", log, 0)

\* (1b) The engine's track-in must not land after a hold it raced with.
NoTrackInAfterHold ==
    \A i, j \in 1..Len(log) : (i < j /\ log[i] = "Hold") => log[j] /= "TrackIn"

====
