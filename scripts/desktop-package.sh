#!/usr/bin/env bash
# Build the desktop package for the machine this runs on.
#
#   scripts/desktop-package.sh [output-directory]
#
# macOS:   .nsh/out is copied to <out>/macos/oxide-fab.app
# Linux:   <out>/linux/oxide-fab (directory with the executable + nsh assets)
# Windows: <out>/windows/oxide-fab (directory, plus <Name>-setup.exe when Inno
#          Setup is installed)
#
# The version comes from crates/oxide-ui/nsh.json; the build number from
# $BUILD_NUMBER (CI) or 1.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"
out="${1:-dist}"

case "$(uname -s)" in
  Darwin) target=macos ;;
  Linux) target=linux ;;
  MINGW* | MSYS* | CYGWIN*) target=windows ;;
  *)
    echo "unsupported host: $(uname -s)" >&2
    exit 2
    ;;
esac

version="$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' crates/oxide-ui/nsh.json | head -1)"
build="${BUILD_NUMBER:-1}"
mkdir -p "$out/$target"

nsh package crates/oxide-ui \
  --target "$target" \
  --name oxide-fab \
  --app-version "$version" \
  --build "$build" \
  -o "$out/$target"

case "$target" in
  macos)
    test -d "$out/macos/oxide-fab.app"
    echo "$out/macos/oxide-fab.app"
    ;;
  linux)
    # nsh names the executable after the app id's last segment (`oxidefab`).
    exe="$(find "$out/linux/oxide-fab" -maxdepth 1 -type f -perm -u+x | head -1)"
    test -n "$exe"
    echo "$exe"
    ;;
  windows)
    exe="$(find "$out/windows/oxide-fab" -maxdepth 1 -type f -name '*.exe' | head -1)"
    test -n "$exe"
    echo "$exe"
    ;;
esac
