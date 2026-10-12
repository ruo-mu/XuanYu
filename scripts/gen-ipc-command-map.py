#!/usr/bin/env python3
"""Generate WS type ↔ host IPC command coverage report from dispatch_gen.inc + protocol.

Does not rewrite useWebSocket.ts yet; produces apps/desktop-ui/src/ipc/ipc-command-map.json
for the next codegen pass.
"""

from __future__ import annotations

import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DISPATCH = ROOT / "crates/xuanyu-desktop-host/src/dispatch_gen.inc"
OUT = ROOT / "apps/desktop-ui/src/ipc/ipc-command-map.json"
REF_PROTOCOL = Path(r"D:/develop/projects/xuanyu-agent/src/gateway/web/src/protocol.ts")
APP_PROTOCOL = ROOT / "apps/desktop-ui/src/protocol.ts"


def host_commands() -> list[str]:
    text = DISPATCH.read_text(encoding="utf-8")
    return sorted(set(re.findall(r'"([a-z0-9_]+)"\s*=>', text)))


def client_ws_types(path: Path) -> list[str]:
    text = path.read_text(encoding="utf-8")
    # ClientMessageType union members like 'session.list'
    block = re.search(r"export type ClientMessageType\s*=([\s\S]*?);", text)
    if not block:
        return []
    return sorted(set(re.findall(r"'([a-z0-9.]+)'", block.group(1))))


def ws_to_snake(ws: str) -> str:
    return ws.replace(".", "_")


# Known intentional remaps (WS protocol → host command)
ALIASES = {
    "chat.send": "session_send_message",
    "chat.abort": "session_abort",
    "context.reset": "session_reset_context",
    "config.save": "config_update",
    "tools.catalog.get": "tool_catalog",
    "tools.catalog": "tool_catalog",
    "replay.request": "event_replay",
    "stats.get": "stats_get",
    "task.cancel": "task_control",
    "task.pause": "task_control",
    "task.resume": "task_control",
    "task.retry": "task_control",
    "session.restore": "session_history",  # UI restores via history + local state
    "providers.catalog": None,  # renderer-local getProviderCatalog()
    "session.pin": "session_pin",
    "workspace.pickFolder": None,  # Electron IPC only
    "workspace.reveal": None,  # Electron IPC only
}


def main() -> None:
    commands = host_commands()
    cmd_set = set(commands)
    # Prefer app protocol (includes channel.*); fall back to reference if missing.
    ref_ws = client_ws_types(APP_PROTOCOL)
    if not ref_ws and REF_PROTOCOL.exists():
        ref_ws = client_ws_types(REF_PROTOCOL)
    covered = []
    missing = []
    electron_only = []
    for ws in ref_ws:
        if ws in ALIASES:
            target = ALIASES[ws]
            if target is None:
                electron_only.append(ws)
                continue
            if target in cmd_set:
                covered.append({"ws": ws, "host": target, "via": "alias"})
            else:
                missing.append({"ws": ws, "expected": target})
            continue
        snake = ws_to_snake(ws)
        if snake in cmd_set:
            covered.append({"ws": ws, "host": snake, "via": "direct"})
        else:
            missing.append({"ws": ws, "expected": snake})

    payload = {
        "host_command_count": len(commands),
        "ref_ws_count": len(ref_ws),
        "covered_count": len(covered),
        "missing_count": len(missing),
        "electron_only_count": len(electron_only),
        "host_commands": commands,
        "covered": covered,
        "missing": missing,
        "electron_only": electron_only,
        "aliases": ALIASES,
    }
    OUT.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(
        f"wrote {OUT} covered={len(covered)} missing={len(missing)} electron_only={len(electron_only)}"
    )


if __name__ == "__main__":
    main()
