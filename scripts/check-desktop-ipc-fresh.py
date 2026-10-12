#!/usr/bin/env python3
"""Fail if desktop IPC generated artifacts are out of date vs host dispatch.

Usage:
  python scripts/check-desktop-ipc-fresh.py
"""

from __future__ import annotations

import json
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GEN = ROOT / "apps" / "desktop-ui" / "src" / "ipc" / "ipc-command-map.json"


def main() -> int:
    if not GEN.exists():
        print("missing apps/desktop-ui/src/ipc/ipc-command-map.json — run: pnpm gen:desktop-ipc", file=sys.stderr)
        return 1
    before = GEN.read_text(encoding="utf-8")
    proc = subprocess.run(
        [sys.executable, str(ROOT / "scripts" / "gen-ipc-command-map.py")],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        print(proc.stdout)
        print(proc.stderr, file=sys.stderr)
        return proc.returncode
    after = GEN.read_text(encoding="utf-8")
    # Restore original if changed so the check is non-mutating for dirty trees
    # that only want verification — but we still fail if drift would occur.
    if before != after:
        GEN.write_text(before, encoding="utf-8")
        print(
            "ipc-command-map.json is stale. Run `pnpm gen:desktop-ipc` and commit.",
            file=sys.stderr,
        )
        return 1
    # Also ensure dispatch_gen.inc / desktopIpc.gen exist
    for rel in (
        "crates/xuanyu-desktop-host/src/dispatch_gen.inc",
        "apps/desktop-ui/src/ipc/desktopIpc.gen.ts",
    ):
        path = ROOT / rel
        if not path.exists():
            print(f"missing generated file: {rel}", file=sys.stderr)
            return 1
    data = json.loads(before)
    print(
        f"desktop IPC fresh: host={data.get('host_command_count')} covered={data.get('covered_count')} missing={data.get('missing_count')}"
    )
    if int(data.get("missing_count") or 0) > 0:
        print("ipc-command-map reports missing WS→host mappings", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
