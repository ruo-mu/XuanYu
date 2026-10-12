#!/usr/bin/env python3
from __future__ import annotations

import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LIB = ROOT / "crates/xuanyu-desktop-host/src/lib.rs"
OUT = ROOT / "crates/xuanyu-desktop-host/src/dispatch_gen.inc"

# Params 无法从 JSON 反序列化的参数类型（命中则跳过生成 dispatch）。
SKIP_PARAM_SUBSTR = ("AppHandle", "Window", "Webview")
# 不生成 dispatch 的函数：
#  - create_app_core: crate 公开 API，非 IPC 命令
#  - init_postgres_install / permission_respond / event_replay_filtered: 手工 dispatch
#  - marketplace_search: 参数已打包为 MarketplaceSearchInput（跨 crate 类型），手工 dispatch
SKIP_NAMES = {
    "create_app_core",
    "init_postgres_install",
    "permission_respond",
    "event_replay_filtered",
    "marketplace_search",
}


def split_params(params_raw: str) -> list[str]:
    parts: list[str] = []
    depth = 0
    current: list[str] = []
    for ch in params_raw:
        if ch in "<([{":
            depth += 1
        elif ch in ">)]}":
            depth = max(0, depth - 1)
        elif ch == "," and depth == 0:
            part = "".join(current).strip()
            if part:
                parts.append(part)
            current = []
            continue
        current.append(ch)
    part = "".join(current).strip()
    if part:
        parts.append(part)
    return parts


def format_result_block(ret_type: str, call: str) -> str:
    """call 形如 `app_init(&core)` 或 `session_create(&core, input.mode, ...).await`。"""
    if ret_type.startswith("Result<") and ret_type.endswith(", String>"):
        return (
            f"    let result: {ret_type} = {call};\n"
            f"    match result {{\n"
            f"        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),\n"
            f"        Err(e) => Err(e),\n"
            f"    }}"
        )
    return (
        f"    let result: {ret_type} = {call};\n"
        f"    serde_json::to_value(result).map_err(|e| e.to_string())"
    )


text = LIB.read_text(encoding="utf-8")
header_pat = re.compile(
    r"(?:pub\(crate\)\s+)?(?P<async>async\s+)?fn\s+(?P<name>\w+)\s*\((?P<params>.*?)\)\s*(?:->\s*(?P<ret>[^\{]+))?\s*\{",
    re.DOTALL,
)

helpers: list[str] = []
arms: list[str] = []

manual_helpers = '''
async fn dispatch_init_postgres_install(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params { skip_download: Option<bool>, install_pgvector: Option<bool> }
    let payload: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result = core.init_postgres_install(
        serde_json::json!({
            "skipDownload": payload.skip_download.unwrap_or(false),
            "installPgvector": payload.install_pgvector.unwrap_or(true),
        }),
        None,
    ).await;
    serde_json::to_value(result).map_err(|e| e.to_string())
}
'''

manual_arms = '''
        "init_postgres_install" => dispatch_init_postgres_install(&managed.0, params).await,
        "app_updater_install" => Err("Electron shell handles updates".to_string()),
        "app_updater_relaunch" => Err("Electron shell handles relaunch".to_string()),
        "launch_setup_app" => Err("Electron shell handles setup".to_string()),
        "permission_respond" => dispatch_permission_respond(&managed.0, params).await,
        "event_replay_filtered" => dispatch_event_replay_filtered(&managed.0, params).await,
        "marketplace_search" => dispatch_marketplace_search(&managed.0, params).await,
'''

manual_helpers += '''
async fn dispatch_permission_respond(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        request_id: String,
        allowed: bool,
        remember: Option<bool>,
        scope: Option<String>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let scope = input.scope.or_else(|| input.remember.unwrap_or(false).then(|| "session".to_string()));
    let result = core.permission_respond_with_scope(input.request_id, input.allowed, scope).await;
    serde_json::to_value(result).map_err(|e| e.to_string())
}

async fn dispatch_event_replay_filtered(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        last_sequence: u64,
        filter: serde_json::Value,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let filter: xuanyu_eventbus::EventFilter =
        serde_json::from_value(input.filter).map_err(|err| err.to_string())?;
    let batch = core.event_replay_filtered(input.last_sequence, filter).await;
    serde_json::to_value(batch).map_err(|err| err.to_string())
}

async fn dispatch_marketplace_search(core: &AppCore, params: Value) -> Result<Value, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Params {
        #[serde(default)]
        query: Option<String>,
        #[serde(default)]
        kind: Option<String>,
        #[serde(default)]
        kinds: Option<serde_json::Value>,
        #[serde(default)]
        request_id: Option<String>,
        #[serde(default)]
        page: Option<u32>,
        #[serde(default)]
        page_size: Option<u32>,
        #[serde(default)]
        category: Option<String>,
        #[serde(default)]
        source_id: Option<String>,
        #[serde(default)]
        summary_only: Option<bool>,
    }
    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;
    let result: Result<Value, String> = marketplace_search(
        core,
        MarketplaceSearchInput {
            query: input.query,
            kind: input.kind,
            kinds: input.kinds,
            request_id: input.request_id,
            page: input.page,
            page_size: input.page_size,
            category: input.category,
            source_id: input.source_id,
            summary_only: input.summary_only,
        },
    )
    .await;
    match result {
        Ok(v) => Ok(serde_json::to_value(v).map_err(|e| e.to_string())?),
        Err(e) => Err(e),
    }
}
'''

for match in header_pat.finditer(text):
    name = match.group("name")
    if name in SKIP_NAMES:
        continue
    is_async = bool(match.group("async"))
    params_raw = match.group("params")
    ret_type = (match.group("ret") or "Result<(), String>").strip()
    helper_name = f"dispatch_{name}"

    args: list[tuple[str, str]] = []
    skip = False
    for part in split_params(params_raw):
        part = part.strip()
        if not part or part.startswith("core:"):
            continue
        if ":" not in part:
            continue
        pname, ptype = part.split(":", 1)
        pname, ptype = pname.strip(), ptype.strip()
        if any(s in ptype for s in SKIP_PARAM_SUBSTR):
            skip = True
            break
        args.append((pname, ptype))
    if skip:
        continue

    # dispatch 直接调用 lib.rs 中的 wrapper fn（wrapper 由此被真实引用，不再是死代码）。
    call = f"{name}(core" + (f", {', '.join(f'input.{p}' for p, _ in args)}" if args else "") + ")"
    if is_async:
        call += ".await"
    result_block = format_result_block(ret_type, call)

    if args:
        fields = "\n        ".join(
            (f"#[serde(default)]\n        {p}: {t}," if "Option<" in t else f"{p}: {t},")
            for p, t in args
        )
        helpers.append(
            f"async fn {helper_name}(core: &AppCore, params: Value) -> Result<Value, String> {{\n"
            f"    #[derive(serde::Deserialize)]\n"
            f"    #[serde(rename_all = \"camelCase\")]\n"
            f"    struct Params {{\n        {fields}\n    }}\n"
            f"    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;\n"
            f"{result_block}\n"
            f"}}"
            if is_async
            else
            f"fn {helper_name}(core: &AppCore, params: Value) -> Result<Value, String> {{\n"
            f"    #[derive(serde::Deserialize)]\n"
            f"    #[serde(rename_all = \"camelCase\")]\n"
            f"    struct Params {{\n        {fields}\n    }}\n"
            f"    let input: Params = serde_json::from_value(params).map_err(|e| e.to_string())?;\n"
            f"{result_block}\n"
            f"}}"
        )
        arms.append(
            f'        "{name}" => {helper_name}(&managed.0, params).await,'
            if is_async
            else f'        "{name}" => {helper_name}(&managed.0, params),'
        )
    else:
        helpers.append(
            f"async fn {helper_name}(core: &AppCore) -> Result<Value, String> {{\n"
            f"{result_block}\n"
            f"}}"
            if is_async
            else
            f"fn {helper_name}(core: &AppCore) -> Result<Value, String> {{\n"
            f"{result_block}\n"
            f"}}"
        )
        arms.append(
            f'        "{name}" => {helper_name}(&managed.0).await,'
            if is_async
            else f'        "{name}" => {helper_name}(&managed.0),'
        )

content = f'''// Auto-generated — python scripts/gen-desktop-dispatch.py

{manual_helpers}

{chr(10).join(helpers)}

pub async fn invoke_command(core: Arc<AppCore>, command: &str, params: Value) -> Result<Value, String> {{
    let managed = CoreState(core);
    match command {{
{manual_arms}
{chr(10).join(arms)}
        other => Err(format!("unknown command: {{other}}")),
    }}
}}
'''
OUT.write_text(content, encoding="utf-8")
print(f"generated {len(arms)} commands -> {OUT}")
