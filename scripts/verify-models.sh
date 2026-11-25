#!/usr/bin/env bash
# Every model config has a known answer.
#
# A config that replays a past defect must fail on the invariant that defect
# broke, with TLC's exit code 12. A config that describes the current code must
# pass. A model edit that quietly weakens a check fails here before it can be
# trusted.
set -euo pipefail

cd "$(dirname "$0")/.."

readonly MODEL_TOOL=(npx --yes everystate@0.4.0)

fail_on() {
  local model=$1 config=$2 invariant=$3
  local output code
  set +e
  output=$("${MODEL_TOOL[@]}" -config "$config" "$model" 2>&1)
  code=$?
  set -e
  if [ "$code" -ne 12 ] || ! grep -q "Invariant ${invariant} is violated" <<<"$output"; then
    printf 'FAIL %s -config %s: expected %s to be violated (exit 12), got %s\n' \
      "$model" "$config" "$invariant" "$code"
    printf '%s\n' "$output"
    exit 1
  fi
  printf 'caught %-22s %s\n' "$config" "$invariant"
}

passes() {
  local model=$1 config=$2
  local output code
  set +e
  output=$("${MODEL_TOOL[@]}" -config "$config" "$model" 2>&1)
  code=$?
  set -e
  if [ "$code" -ne 0 ]; then
    printf 'FAIL %s -config %s: expected a pass, got %s\n' "$model" "$config" "$code"
    printf '%s\n' "$output"
    exit 1
  fi
  printf 'pass   %-22s\n' "$config"
}

write_path=docs/verification/write-path/WritePath.tla
fail_on "$write_path" LegalLog.cfg LegalLog
fail_on "$write_path" Recovery.cfg RecoveryMatches
fail_on "$write_path" FailedAppend.cfg RecoveryMatches
fail_on "$write_path" SnapshotRace.cfg RecoveryMatches
fail_on "$write_path" EngineRace.cfg NoTrackInAfterHold
passes "$write_path" FixedWrites.cfg
passes "$write_path" Fixed.cfg

lifecycle=docs/verification/lot-lifecycle/LotLifecycle.tla
fail_on "$lifecycle" AsWritten.cfg LogAgrees
fail_on "$lifecycle" HoldLost.cfg HoldConsistent
fail_on "$lifecycle" TerminalRevival.cfg TerminalAbsorbing
fail_on "$lifecycle" ScrapUnlogged.cfg LogAgrees
passes "$lifecycle" Fixed.cfg
