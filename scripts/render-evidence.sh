#!/usr/bin/env bash
# Render the four drill-down levels headlessly and write the report.
#
# On Linux and Windows the software adapter is requested (lavapipe / WARP); on
# macOS Metal has no software implementation, so the runner's GPU is used and
# the report says so.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
out="${1:-artifacts/render}"

args=(--out "$out")
case "$(uname -s)" in
  Linux)
    # Ubuntu's Mesa ships lavapipe as an ordinary Vulkan ICD; pointing at it
    # keeps the run off any real GPU the runner may expose.
    for icd in \
      /usr/share/vulkan/icd.d/lvp_icd.x86_64.json \
      /usr/share/vulkan/icd.d/lvp_icd.aarch64.json \
      /usr/share/vulkan/icd.d/lvp_icd.json; do
      if [ -f "$icd" ]; then
        export VK_ICD_FILENAMES="$icd"
        echo "Vulkan ICD: $icd"
        break
      fi
    done
    args+=(--software)
    ;;
  MINGW* | MSYS* | CYGWIN*)
    args+=(--software)
    ;;
esac

cargo run --locked -p oxide-render --bin oxide-render-evidence -- "${args[@]}"
