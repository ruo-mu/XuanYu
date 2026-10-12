//! Best-effort parse of partial tool-call JSON while the model streams write args.
//! Drives UI +N/−N previews; does not write to disk.

use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct StreamWritePreview {
    pub path: Option<String>,
    pub content: Option<String>,
    pub old_text: Option<String>,
    pub new_text: Option<String>,
    pub add: usize,
    pub del: usize,
    pub args: BTreeMap<String, Value>,
    /// Existing file content when known; empty string = new file; None = not resolved yet.
    pub existing_before: Option<String>,
}

pub fn is_streamable_write_tool(name: &str) -> bool {
    matches!(name, "file_write" | "file_edit")
}

pub fn is_mid_turn_write_tool(name: &str) -> bool {
    matches!(name, "file_write" | "file_edit" | "apply_patch")
}

/// Preview streamed write-tool arguments. `existing_before` improves file_write add/del stats.
pub fn preview_stream_write_args(
    tool_name: &str,
    raw_args: &str,
    existing_before: Option<&str>,
) -> Option<StreamWritePreview> {
    if raw_args.is_empty() || !is_streamable_write_tool(tool_name) {
        return None;
    }
    if let Ok(parsed) = serde_json::from_str::<BTreeMap<String, Value>>(raw_args) {
        return Some(from_complete_args(tool_name, &parsed, existing_before));
    }
    match tool_name {
        "file_write" => {
            let path = extract_json_string_field(raw_args, "path", false)
                .or_else(|| extract_json_string_field(raw_args, "file_path", false));
            let content = extract_json_string_field(raw_args, "content", true);
            if path.is_none() && content.is_none() {
                return None;
            }
            let (add, del) = write_stats(content.as_deref(), existing_before);
            let mut args = BTreeMap::new();
            if let Some(path) = path.as_ref() {
                args.insert("path".to_string(), Value::String(path.clone()));
            }
            if let Some(content) = content.as_ref() {
                args.insert("content".to_string(), Value::String(content.clone()));
            }
            Some(StreamWritePreview {
                path,
                content,
                add,
                del,
                args,
                existing_before: existing_before.map(str::to_string),
                ..Default::default()
            })
        }
        "file_edit" => {
            let path = extract_json_string_field(raw_args, "path", false)
                .or_else(|| extract_json_string_field(raw_args, "file_path", false));
            let old_text = extract_json_string_field(raw_args, "old_text", true);
            let new_text = extract_json_string_field(raw_args, "new_text", true);
            if path.is_none() && old_text.is_none() && new_text.is_none() {
                return None;
            }
            let del = old_text.as_ref().map(|t| count_lines(t)).unwrap_or(0);
            let add = new_text.as_ref().map(|t| count_lines(t)).unwrap_or(0);
            let mut args = BTreeMap::new();
            if let Some(path) = path.as_ref() {
                args.insert("path".to_string(), Value::String(path.clone()));
            }
            if let Some(old_text) = old_text.as_ref() {
                args.insert("old_text".to_string(), Value::String(old_text.clone()));
            }
            if let Some(new_text) = new_text.as_ref() {
                args.insert("new_text".to_string(), Value::String(new_text.clone()));
            }
            Some(StreamWritePreview {
                path,
                old_text,
                new_text,
                add,
                del,
                args,
                ..Default::default()
            })
        }
        _ => None,
    }
}

fn from_complete_args(
    tool_name: &str,
    parsed: &BTreeMap<String, Value>,
    existing_before: Option<&str>,
) -> StreamWritePreview {
    if tool_name == "file_write" {
        let path = string_field(parsed, "path").or_else(|| string_field(parsed, "file_path"));
        let content = string_field(parsed, "content").unwrap_or_default();
        let (add, del) = write_stats(Some(&content), existing_before);
        let mut args = BTreeMap::new();
        if let Some(path) = path.as_ref() {
            args.insert("path".to_string(), Value::String(path.clone()));
        }
        args.insert("content".to_string(), Value::String(content.clone()));
        return StreamWritePreview {
            path,
            content: Some(content),
            add,
            del,
            args,
            existing_before: existing_before.map(str::to_string),
            ..Default::default()
        };
    }
    if tool_name == "file_edit" {
        let path = string_field(parsed, "path").or_else(|| string_field(parsed, "file_path"));
        let old_text = string_field(parsed, "old_text").unwrap_or_default();
        let new_text = string_field(parsed, "new_text").unwrap_or_default();
        let mut args = BTreeMap::new();
        if let Some(path) = path.as_ref() {
            args.insert("path".to_string(), Value::String(path.clone()));
        }
        args.insert("old_text".to_string(), Value::String(old_text.clone()));
        args.insert("new_text".to_string(), Value::String(new_text.clone()));
        return StreamWritePreview {
            path,
            old_text: Some(old_text.clone()),
            new_text: Some(new_text.clone()),
            add: count_lines(&new_text),
            del: count_lines(&old_text),
            args,
            ..Default::default()
        };
    }
    StreamWritePreview {
        args: parsed.clone(),
        ..Default::default()
    }
}

fn write_stats(content: Option<&str>, existing_before: Option<&str>) -> (usize, usize) {
    let Some(content) = content else {
        return (0, 0);
    };
    match existing_before {
        // Unknown whether file exists — do not invent overwrite stats.
        None => {
            // Still stream a provisional +N for create-shaped args (path not yet parsed).
            // Overwrite is corrected as soon as path resolves and existing_before is known.
            (count_lines(content), 0)
        }
        Some("") => (count_lines(content), 0),
        Some(before) => count_line_diff_stats(before, content),
    }
}

fn count_line_diff_stats(before: &str, after: &str) -> (usize, usize) {
    let before_lines: Vec<&str> = before.lines().collect();
    let after_lines: Vec<&str> = after.lines().collect();
    let mut add = 0usize;
    let mut del = 0usize;
    let mut i = 0usize;
    let mut j = 0usize;
    while i < before_lines.len() || j < after_lines.len() {
        if i < before_lines.len() && j < after_lines.len() && before_lines[i] == after_lines[j] {
            i += 1;
            j += 1;
            continue;
        }
        if j < after_lines.len()
            && (i >= before_lines.len()
                || before_lines[i..].iter().any(|line| *line == after_lines[j]))
        {
            add += 1;
            j += 1;
            continue;
        }
        if i < before_lines.len() {
            del += 1;
            i += 1;
            continue;
        }
        add += 1;
        j += 1;
    }
    (add, del)
}

pub fn count_lines(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    text.chars().filter(|ch| *ch == '\n').count() + 1
}

fn string_field(obj: &BTreeMap<String, Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(Value::as_str).map(str::to_string)
}

fn extract_json_string_field(raw: &str, key: &str, allow_unclosed: bool) -> Option<String> {
    let needle = format!("\"{key}\"");
    let mut search_from = 0usize;
    while let Some(rel) = raw[search_from..].find(&needle) {
        let key_start = search_from + rel;
        let after_key = key_start + needle.len();
        let rest = raw[after_key..].trim_start();
        if !rest.starts_with(':') {
            search_from = after_key;
            continue;
        }
        let after_colon = &rest[1..];
        let trimmed = after_colon.trim_start();
        if !trimmed.starts_with('"') {
            search_from = after_key;
            continue;
        }
        let value_start = raw.len() - trimmed.len() + 1;
        return read_json_string(raw, value_start, allow_unclosed);
    }
    None
}

fn read_json_string(raw: &str, start: usize, allow_unclosed: bool) -> Option<String> {
    let bytes = raw.as_bytes();
    let mut out = String::new();
    let mut i = start;
    while i < bytes.len() {
        let ch = bytes[i] as char;
        if ch == '"' {
            return Some(out);
        }
        if ch == '\\' {
            if i + 1 >= bytes.len() {
                return if allow_unclosed { Some(out) } else { None };
            }
            let n = bytes[i + 1] as char;
            match n {
                'n' => out.push('\n'),
                'r' => out.push('\r'),
                't' => out.push('\t'),
                '"' | '\\' | '/' => out.push(n),
                'u' => {
                    if i + 6 > bytes.len() {
                        return if allow_unclosed { Some(out) } else { None };
                    }
                    let hex = &raw[i + 2..i + 6];
                    let Ok(code) = u32::from_str_radix(hex, 16) else {
                        return if allow_unclosed { Some(out) } else { None };
                    };
                    if let Some(c) = char::from_u32(code) {
                        out.push(c);
                    }
                    i += 6;
                    continue;
                }
                other => out.push(other),
            }
            i += 2;
            continue;
        }
        out.push(ch);
        i += 1;
    }
    if allow_unclosed {
        Some(out)
    } else {
        None
    }
}
