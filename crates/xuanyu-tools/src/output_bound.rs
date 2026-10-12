//! Model-facing tool-result bounds.
//!
//! Shell/exec: 64 KiB inline, keep tail, spill full output to a file.
//! MCP / other tools: 64 KiB inline, keep head+tail preview, spill full text.
//! file_read is intentionally excluded (owns its own windowing).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use xuanyu_storage::runtime_data_root;

use crate::ToolOutput;

/// Shell / exec per-stream inline cap (bytes).
pub const EXEC_MAX_OUTPUT_BYTES: usize = 64_000;

/// MCP and other plain-text tools: model-facing inline cap (bytes).
pub const TOOL_INLINE_MAX_BYTES: usize = 64_000;

/// Spill file size ceiling (bytes).
const SPILL_MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy)]
enum BoundMode {
    /// Keep the end of the stream (shell).
    Tail,
    /// Keep head + tail preview (generic / MCP spill policy).
    HeadTail,
}

pub fn apply_model_facing_bound(tool_name: &str, mut output: ToolOutput) -> ToolOutput {
    if should_skip_bound(tool_name, &output) {
        return output;
    }
    let (max_bytes, mode) = if is_shell_like(tool_name) {
        (EXEC_MAX_OUTPUT_BYTES, BoundMode::Tail)
    } else {
        (TOOL_INLINE_MAX_BYTES, BoundMode::HeadTail)
    };
    if output.content.len() <= max_bytes {
        return output;
    }

    let existing_spill = existing_spill_path(tool_name, &output);
    let (bounded, spill_path) = bound_text_with_spill(
        &output.content,
        max_bytes,
        mode,
        tool_name,
        existing_spill.as_deref(),
    );
    output.content = bounded;
    if let Some(path) = spill_path {
        let path_str = path_display(&path);
        if let Some(data) = output.data.as_mut() {
            scrub_oversized_data_fields(data, &path_str, max_bytes);
            if let Some(obj) = data.as_object_mut() {
                obj.insert("spillPath".to_string(), Value::String(path_str));
                obj.insert("truncated".to_string(), Value::Bool(true));
            }
        } else {
            output.data = Some(json!({
                "spillPath": path_display(&path),
                "truncated": true
            }));
        }
    }
    output
}

fn should_skip_bound(tool_name: &str, output: &ToolOutput) -> bool {
    // Own windowing / media payloads must not enter the generic spill loop.
    if tool_name == "file_read" {
        return true;
    }
    // Write tools ship unified diffs for the chat UI (+/− stats + DiffViewer).
    // Truncating/spilling them drops parseable hunks and hides line counts.
    if matches!(
        tool_name,
        "file_write" | "file_edit" | "apply_patch" | "write_file" | "git_patch_apply"
    ) {
        return true;
    }
    if output
        .data
        .as_ref()
        .and_then(|v| v.get("attachment"))
        .is_some()
    {
        return true;
    }
    false
}

fn is_shell_like(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "exec" | "read_process_output" | "start_process" | "list_processes" | "stop_process"
            | "write_process_stdin"
    )
}

fn existing_spill_path(tool_name: &str, output: &ToolOutput) -> Option<PathBuf> {
    if !is_shell_like(tool_name) {
        return None;
    }
    let data = output.data.as_ref()?;
    data.get("logPath")
        .or_else(|| data.pointer("/process/logPath"))
        .or_else(|| data.get("spillPath"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// Drop oversized parallel copies in `data` so session/event payloads stay lean after spill.
fn scrub_oversized_data_fields(data: &mut Value, spill_path: &str, max_bytes: usize) {
    let Some(obj) = data.as_object_mut() else {
        return;
    };
    for key in [
        "diff",
        "output",
        "text",
        "content",
        "stdout",
        "stderr",
        "body",
    ] {
        let too_large = obj
            .get(key)
            .and_then(Value::as_str)
            .is_some_and(|s| s.len() > max_bytes);
        if too_large {
            obj.insert(
                key.to_string(),
                Value::String(format!(
                    "[spilled; full output: {spill_path}]"
                )),
            );
        }
    }
}

fn bound_text_with_spill(
    text: &str,
    max_bytes: usize,
    mode: BoundMode,
    tool_name: &str,
    existing_spill: Option<&Path>,
) -> (String, Option<PathBuf>) {
    if text.len() <= max_bytes {
        return (text.to_string(), None);
    }

    let spill_path = existing_spill
        .map(|p| p.to_path_buf())
        .or_else(|| write_spill(tool_name, text));
    let notice = match spill_path.as_ref() {
        Some(path) => format!(
            "[output truncated; full output: {}]",
            path_display(path)
        ),
        None => "[output truncated; full output: (unavailable)]".to_string(),
    };
    let reserve = notice.len().saturating_add(2);
    let budget = max_bytes.saturating_sub(reserve).max(256);

    let bounded = match mode {
        BoundMode::Tail => {
            let tail = take_utf8_suffix(text, budget);
            format!("{tail}\n\n{notice}")
        }
        BoundMode::HeadTail => {
            let omit_reserve = 48usize;
            let content_budget = budget.saturating_sub(omit_reserve);
            let head_budget = content_budget.div_ceil(2);
            let tail_budget = content_budget / 2;
            let head = take_utf8_prefix(text, head_budget);
            let tail = take_utf8_suffix(text, tail_budget);
            let omitted = text
                .len()
                .saturating_sub(head.len())
                .saturating_sub(tail.len());
            let body = format!("{head}\n\n...[{omitted} bytes omitted]...\n\n{tail}");
            format!("{body}\n\n{notice}")
        }
    };
    (bounded, spill_path)
}

fn write_spill(tool_name: &str, text: &str) -> Option<PathBuf> {
    let root = spill_root();
    std::fs::create_dir_all(&root).ok()?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let safe_name: String = tool_name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(48)
        .collect();
    let path = root.join(format!("{stamp}-{safe_name}.txt"));
    let bytes = text.as_bytes();
    let write_bytes = if bytes.len() > SPILL_MAX_BYTES {
        &bytes[..SPILL_MAX_BYTES]
    } else {
        bytes
    };
    std::fs::write(&path, write_bytes).ok()?;
    Some(path)
}

fn spill_root() -> PathBuf {
    runtime_data_root().join("spills")
}

fn take_utf8_prefix(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn take_utf8_suffix(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut start = s.len().saturating_sub(max_bytes);
    while start < s.len() && !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

fn path_display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
