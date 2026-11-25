#!/usr/bin/env python3
"""Fail on the repository invariants that hand edits keep breaking.

Every check names the file, the conflicting value and the fix. The classes come
from the fix commits: action pins drifted between jobs, the apt cache path was
root-owned twice, the service port lived in nine places, the Rust version in
five, and a sed edit once left comment lines inside a YAML block scalar.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ERRORS: list[str] = []


def fail(message: str) -> None:
    ERRORS.append(message)


def text(path: str) -> str:
    return (ROOT / path).read_text()


def lines(path: str) -> list[str]:
    return text(path).splitlines()


def workflows() -> list[str]:
    return sorted(str(path.relative_to(ROOT)) for path in (ROOT / ".github/workflows").glob("*.yml"))


def actions() -> list[str]:
    return sorted(
        str(path.relative_to(ROOT)) for path in (ROOT / ".github/actions").glob("*/action.yml")
    )


def check_action_pins() -> None:
    uses = re.compile(r"uses:\s*([A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+)@([A-Za-z0-9_.-]+)")
    pins: dict[str, list[tuple[str, int, str]]] = {}
    for path in workflows() + actions():
        for number, line in enumerate(lines(path), 1):
            match = uses.search(line)
            if match:
                pins.setdefault(match.group(1), []).append((path, number, match.group(2)))
    for action, entries in pins.items():
        versioned = [entry for entry in entries if re.fullmatch(r"v\d+", entry[2])]
        if len({entry[2] for entry in versioned}) <= 1:
            continue
        want = max(versioned, key=lambda entry: int(entry[2][1:]))[2]
        for path, number, ref in versioned:
            if ref != want:
                fail(
                    f"{path}:{number}: {action} is @{ref} but the highest pin is @{want}. "
                    f"Use @{want} here; one major per action keeps the jobs on one behaviour."
                )


def check_shared_actions() -> None:
    for path in workflows():
        for number, line in enumerate(lines(path), 1):
            if "Dir::Cache::archives" in line:
                fail(
                    f"{path}:{number}: a workflow configures the apt cache directly. "
                    "Use ./.github/actions/native-deps; the runner-owned path and the "
                    "mirror timeouts live there."
                )
            if "nsh.dev/install" in line:
                fail(
                    f"{path}:{number}: a workflow installs nsh directly. "
                    "Use ./.github/actions/install-nsh; Unix and Windows paths stay in one file."
                )


def check_rust_version() -> None:
    channel = re.search(r'channel\s*=\s*"([^"]+)"', text("rust-toolchain.toml"))
    manifest = re.search(r'rust-version\s*=\s*"([^"]+)"', text("Cargo.toml"))
    if not channel or not manifest:
        fail("rust-toolchain.toml or Cargo.toml no longer declares the Rust version.")
        return
    expected = channel.group(1)
    if manifest.group(1) != expected:
        fail(
            f"Cargo.toml: rust-version is {manifest.group(1)} but rust-toolchain.toml "
            f"pins {expected}. Update both (the MSRV is part of the release contract)."
        )
    for path in workflows():
        for number, line in enumerate(lines(path), 1):
            match = re.search(r'toolchain:\s*"([^"]+)"', line)
            if match and match.group(1) != expected:
                fail(
                    f"{path}:{number}: toolchain is {match.group(1)} but rust-toolchain.toml "
                    f"pins {expected}. Use {expected} (one MSRV, five places)."
                )


def check_app_version() -> None:
    manifest = re.search(
        r'\[workspace\.package\][^\[]*?version\s*=\s*"([^"]+)"', text("Cargo.toml"), re.S
    )
    nsh = json.loads(text("crates/oxide-ui/nsh.json"))
    if not manifest:
        fail("Cargo.toml no longer declares [workspace.package].version.")
        return
    if manifest.group(1) != nsh.get("version"):
        fail(
            f"crates/oxide-ui/nsh.json is {nsh.get('version')} but Cargo.toml is "
            f"{manifest.group(1)}. Release archives are named from both; bump them together."
        )


def check_service_port() -> None:
    match = re.search(r"OXIDE_HTTP_ADDR=0\.0\.0\.0:(\d+)", text(".env.example"))
    if not match:
        fail(".env.example no longer declares OXIDE_HTTP_ADDR; the port has no source of truth.")
        return
    port = match.group(1)
    expectations = {
        "Dockerfile": [f"OXIDE_HTTP_ADDR=0.0.0.0:{port}", f"EXPOSE {port}"],
        "docker-compose.yml": [
            f"OXIDE_HTTP_ADDR: 0.0.0.0:{port}",
            f"127.0.0.1:{port}:{port}",
            f"127.0.0.1:{port}/healthz",
        ],
        "bins/oxide-server/src/main.rs": [f'"0.0.0.0:{port}"'],
        "crates/oxide-ui/store.ts": [f"http://127.0.0.1:{port}"],
        "crates/oxide-ui/App.test.tsx": [f"http://127.0.0.1:{port}"],
    }
    for path, needles in expectations.items():
        body = text(path)
        for needle in needles:
            if needle not in body:
                fail(
                    f"{path}: does not contain {needle!r}. The service port is {port} in "
                    ".env.example; change this file to match (compose, Dockerfile, server, "
                    "UI and its test stubs share the default)."
                )


def check_desktop_layout() -> None:
    # (path, minimum occurrences). The macOS app path appears twice: once to
    # archive the bundle, once to run its executable.
    needed = {
        "linux": ("dist/linux/oxide-fab/oxidefab", 1),
        "macos": ("dist/macos/oxide-fab.app", 2),
        "windows": ("dist/windows/oxide-fab", 1),
    }
    for path in ("ci.yml", "release.yml"):
        body = text(f".github/workflows/{path}").replace("\\", "/")
        for target, (layout, minimum) in needed.items():
            if body.count(layout) < minimum:
                fail(
                    f".github/workflows/{path}: the {target} package layout {layout!r} "
                    f"appears {body.count(layout)} time(s), expected at least {minimum}. "
                    "nsh names the Windows exe oxide-fab.exe and the Unix one oxidefab; "
                    "keep both workflows on the same layout."
                )


def check_block_scalars() -> None:
    """Catch comment lines that a sed edit left inside a data block.

    A `#` line inside a shell `run: |` is a shell comment. Inside
    `workspaces: |` it is data handed to rust-cache, which is how a botched
    edit turned a comment into a cache path.
    """
    for path in workflows() + actions():
        inside = False
        indent = 0
        for number, line in enumerate(lines(path), 1):
            if re.search(r"workspaces:\s*[|>]\s*$", line):
                inside = True
                indent = len(line) - len(line.lstrip())
                continue
            stripped = line.strip()
            if not inside or not stripped:
                continue
            current = len(line) - len(line.lstrip())
            if current <= indent:
                inside = False
                continue
            if stripped.startswith("#"):
                fail(
                    f"{path}:{number}: a comment line sits inside the workspaces block "
                    "scalar, so rust-cache receives it as a path. Move the comment above "
                    "the key."
                )


def main() -> int:
    check_action_pins()
    check_shared_actions()
    check_rust_version()
    check_app_version()
    check_service_port()
    check_desktop_layout()
    check_block_scalars()
    if ERRORS:
        for message in ERRORS:
            print(f"FAIL: {message}", file=sys.stderr)
        return 1
    print("repository invariants: OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
