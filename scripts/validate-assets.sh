#!/usr/bin/env bash
# Validate the committed GLB against its manifest without opening a GPU, and
# print the checksums the release records.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
cargo run --locked -q -p oxide-render --bin oxide-render-evidence -- --validate-only
sha256sum assets/fab/oxide-fab.glb assets/fab/fab-manifest.json 2>/dev/null \
  || shasum -a 256 assets/fab/oxide-fab.glb assets/fab/fab-manifest.json
