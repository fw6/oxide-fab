---------------- MODULE LotLifecycle ----------------
\* A model of the lot lifecycle, not of the whole fab.
\*
\* Code this models (as of 40bf781):
\*
\*   Hold      = `LotAggregate::hold` (crates/oxide-core/src/state_machine.rs):
\*               sets OnHold and records the reason.
\*   Release   = `LotAggregate::release`: Processing when material is on a
\*               tool, Queued otherwise.
\*   TrackIn   = `LotAggregate::track_in`: refuses a held or terminal lot.
\*   TrackOut  = `LotAggregate::track_out`: a hold outlives the step it
\*               interrupted (5f3c658).
\*   Complete  = `LotAggregate::complete`: refuses a lot with material on a
\*               tool, clears the hold.
\*   ScrapLast = `LotAggregate::scrap_wafer`: scrapping the last active wafer
\*               writes LotCompleted in the same batch (5f3c658), so the log
\*               and the projection agree.
\*
\* Three past defects are reachable through the three CONSTANTs. Each was a
\* guard that covered some transitions and not others:
\*
\*   PreserveHold = FALSE  track_out cleared the hold reason (5f3c658).
\*   TerminalGuard = FALSE track_in accepted a completed lot (the four
\*                         one-sided guards in docs/rust-guidelines-review.md).
\*   ScrapCompletes = FALSE the last scrap set the lot Completed in memory but
\*                         wrote no LotCompleted event (5f3c658).
\*
\* The model is deliberately small: one lot, one tool slot. The promise is the
\* lifecycle, not scheduling.
EXTENDS Naturals, TLC

CONSTANTS PreserveHold, TerminalGuard, ScrapCompletes

VARIABLES state, held, on_tool, completed_logged, ever_completed

vars == <<state, held, on_tool, completed_logged, ever_completed>>

States == {"queued", "processing", "on_hold", "completed"}

Init ==
    /\ state = "queued"
    /\ held = FALSE
    /\ on_tool = FALSE
    /\ completed_logged = FALSE
    /\ ever_completed = FALSE

\* An operator stops a lot. Material already on a tool stays there.
Hold ==
    /\ state /= "completed"
    /\ ~held
    /\ state' = "on_hold"
    /\ held' = TRUE
    /\ UNCHANGED <<on_tool, completed_logged>>

\* Release returns material to dispatch. If a tool still holds it, the step
\* finishes on its own.
Release ==
    /\ held
    /\ held' = FALSE
    /\ state' = IF on_tool THEN "processing" ELSE "queued"
    /\ UNCHANGED <<on_tool, completed_logged>>

\* A tool loads queued material. A held lot and a finished lot are refused.
TrackIn ==
    /\ ~held
    /\ ~on_tool
    /\ state /= "on_hold"
    /\ (TerminalGuard => state /= "completed")
    /\ on_tool' = TRUE
    /\ state' = "processing"
    /\ UNCHANGED <<held, completed_logged>>

\* The step finishes. The hold is not the step's to clear. A terminal lot
\* (the last wafer was scrapped while the tool ran) refuses the transition;
\* the tool is freed either way.
TrackOut ==
    /\ on_tool
    /\ on_tool' = FALSE
    /\ state' = IF state = "completed" THEN "completed"
                ELSE IF PreserveHold /\ held THEN "on_hold"
                ELSE "queued"
    /\ UNCHANGED <<held, completed_logged>>

\* Every step is done. A lot with material on a tool cannot complete: its
\* track-out would arrive after the refusal and the tool would be stranded.
Complete ==
    /\ state /= "completed"
    /\ ~on_tool
    /\ state' = "completed"
    /\ held' = FALSE
    /\ completed_logged' = TRUE
    /\ UNCHANGED on_tool

\* The last active wafer goes. The lot is finished, and the log records it.
ScrapLast ==
    /\ state /= "completed"
    /\ state' = "completed"
    /\ held' = FALSE
    /\ completed_logged' = IF ScrapCompletes THEN TRUE ELSE completed_logged
    /\ UNCHANGED on_tool

Next ==
    /\ \/ Hold
       \/ Release
       \/ TrackIn
       \/ TrackOut
       \/ Complete
       \/ ScrapLast
    /\ ever_completed' = (ever_completed \/ state' = "completed")

TypeOK ==
    /\ state \in States
    /\ held \in BOOLEAN
    /\ on_tool \in BOOLEAN
    /\ completed_logged \in BOOLEAN
    /\ ever_completed \in BOOLEAN

\* A hold is a state, not a flag that can outlive it.
HoldConsistent == held <=> state = "on_hold"

\* Completed is absorbing: no transition revives a terminal lot.
TerminalAbsorbing == ever_completed => state = "completed"

\* The log and the projection see the same terminal lot.
LogAgrees == state = "completed" => completed_logged

====
