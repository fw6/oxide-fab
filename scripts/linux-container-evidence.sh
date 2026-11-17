#!/usr/bin/env bash
# Run the Linux half of the acceptance inside an Ubuntu container: build,
# test, package, launch under Xvfb and screenshot. Used from a macOS host that
# has Docker; CI does the same steps on a native runner.
set -euo pipefail
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
docker run --rm --platform linux/arm64 \
  -v "$root:/src" -w /src \
  -e CARGO_TARGET_DIR=/tmp/oxide-target \
  -e OXIDE_API_URL \
  ubuntu:24.04-arm64 bash -lc '
    set -euo pipefail
    export DEBIAN_FRONTEND=noninteractive
    apt-get update
    apt-get install -y --no-install-recommends \
      build-essential pkg-config curl ca-certificates git \
      libgtk-4-dev libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
      libpango1.0-dev libvulkan1 mesa-vulkan-drivers xvfb imagemagick \
      fonts-noto-cjk
    curl -fsSL https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.100.0 --profile minimal
    . "$HOME/.cargo/env"
    export NSH_VERSION=v0.5.0
    curl -fsSL https://nsh.dev/install.sh | sh
    export PATH="$HOME/.local/bin:$PATH"
    nsh --version
    for icd in /usr/share/vulkan/icd.d/lvp_icd.aarch64.json /usr/share/vulkan/icd.d/lvp_icd.json; do
      if [ -f "$icd" ]; then export VK_ICD_FILENAMES="$icd"; break; fi
    done
    export OXIDE_RENDER_SOFTWARE=1
    # The .nsh runner is generated with host absolute paths; regenerate
    # it inside the container.
    rm -rf crates/oxide-ui/.nsh
    cargo test --workspace
    ./scripts/validate-assets.sh
    ./scripts/render-evidence.sh artifacts/render-linux
    ./scripts/desktop-package.sh dist
    xvfb-run -a -s "-screen 0 1600x1000x24" bash -c "
      OXIDE_DEMO=1 ${OXIDE_API_URL:+OXIDE_API_URL=}${OXIDE_API_URL:-} dist/linux/oxide-fab/oxidefab &
      pid=\$!
      sleep 22
      import -window root artifacts/desktop-linux.png
      kill \$pid || true
    "
    test -s artifacts/desktop-linux.png
    tar -C dist/linux -czf artifacts/oxide-fab-linux.tar.gz oxide-fab
    echo LINUX-EVIDENCE-DONE
  ' 2>&1 | tee artifacts/linux-evidence.log
