#!/usr/bin/env bash
# Reclaim the oxide-ui build caches that grow without bound.
#
#   scripts/clean-ui-cache.sh
#
# nsh writes a complete app into target/debug/build/<pkg>/<hash>/out every time
# a fingerprint changes, so that tree had reached 26 GB — 17 copies of
# nsh-base, 65 of oxide-ui. It is a cache: cargo re-runs the build scripts whose
# output went missing, so the only cost is the next build. deps/ keeps its
# compiled dependencies, and release/, the Windows cross build and .nsh/out are
# left alone.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
target="$root/crates/oxide-ui/target"

reclaim() {
  local dir="$1"
  [ -d "$dir" ] || return 0
  local kb
  kb="$(du -sk "$dir" | cut -f1)"
  rm -rf "$dir"
  echo "$(basename "$dir"): $((kb / 1024)) MB"
}

reclaim "$target/debug/build"
reclaim "$target/debug/incremental"
