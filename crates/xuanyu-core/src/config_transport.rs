//! 配置传输脱敏：下发清空密钥；回传空密钥时保留已有凭据。

use serde_json::{Map, Value};

const SECRET_KEY: &str =
    r"(?:^|[_-])(api[_-]?key|key|token|secret|password|authorization|cookie|connection[_-]?string|private[_-]?key)(?:$|[_-])";

fn secret_key_re() -> regex::Regex {
    regex::Regex::new(SECRET_KEY).expect("secret key regex")
}

fn normalize_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len() + 4);
    let chars: Vec<char> = key.chars().collect();
    for (i, ch) in chars.iter().enumerate() {
        if ch.is_ascii_uppercase() {
            let prev_lower = i > 0 && chars[i - 1].is_ascii_lowercase();
            let next_lower = i + 1 < chars.len() && chars[i + 1].is_ascii_lowercase();
            if prev_lower || next_lower {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(*ch);
        }
    }
    out.to_ascii_lowercase()
}

fn is_secret_key(key: &str) -> bool {
    secret_key_re().is_match(&normalize_key(key))
}

/// 向外发送配置时清空密钥字段，并附带 `credentialHints` 路径列表。
pub fn redact_config_for_transport(config: Value) -> Value {
    let mut hints = Vec::new();
    let mut redacted = redact_value(config, "", &mut hints);
    if let Value::Object(ref mut map) = redacted {
        map.insert(
            "credentialHints".to_string(),
            Value::Array(hints.into_iter().map(Value::String).collect()),
        );
    }
    redacted
}

/// 前端回传空密钥时保留当前已存凭据；非空才轮换。跳过 `credentialHints`。
pub fn preserve_configured_secrets(input: Value, current: &Value) -> Value {
    preserve_value(input, current, "")
}

fn redact_value(value: Value, path: &str, hints: &mut Vec<String>) -> Value {
    match value {
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .enumerate()
                .map(|(index, item)| redact_value(item, &format!("{path}[{index}]"), hints))
                .collect(),
        ),
        Value::Object(map) => {
            let mut out = Map::new();
            for (key, child) in map {
                let child_path = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{path}.{key}")
                };
                if is_secret_key(&key) && child.as_str().is_some() {
                    if child.as_str().map(str::trim).filter(|s| !s.is_empty()).is_some() {
                        hints.push(child_path);
                    }
                    out.insert(key, Value::String(String::new()));
                    continue;
                }
                out.insert(key, redact_value(child, &child_path, hints));
            }
            Value::Object(out)
        }
        other => other,
    }
}

fn preserve_value(input: Value, current: &Value, key: &str) -> Value {
    match input {
        Value::Array(items) => {
            let existing = current.as_array().map(Vec::as_slice).unwrap_or(&[]);
            Value::Array(
                items
                    .into_iter()
                    .enumerate()
                    .map(|(index, item)| {
                        let matched = match_existing_item(&item, existing, index);
                        preserve_value(item, matched, "")
                    })
                    .collect(),
            )
        }
        Value::Object(map) => {
            let existing = current.as_object();
            let mut out = Map::new();
            for (child_key, child) in map {
                if child_key == "credentialHints" {
                    continue;
                }
                let current_child = existing
                    .and_then(|object| object.get(&child_key))
                    .cloned()
                    .unwrap_or(Value::Null);
                out.insert(
                    child_key.clone(),
                    preserve_value(child, &current_child, &child_key),
                );
            }
            Value::Object(out)
        }
        Value::String(text) if is_secret_key(key) && text.trim().is_empty() => current.clone(),
        Value::Null if is_secret_key(key) => current.clone(),
        other => other,
    }
}

fn match_existing_item<'a>(input: &Value, existing: &'a [Value], index: usize) -> &'a Value {
    static NULL: Value = Value::Null;
    let Some(object) = input.as_object() else {
        return existing.get(index).unwrap_or(&NULL);
    };
    for identity_key in ["id", "connectionId", "name"] {
        let Some(identity) = object.get(identity_key).and_then(Value::as_str) else {
            continue;
        };
        if identity.is_empty() {
            continue;
        }
        if let Some(matched) = existing.iter().find(|candidate| {
            let Some(candidate_object) = candidate.as_object() else {
                return false;
            };
            if candidate_object.get(identity_key).and_then(Value::as_str) != Some(identity) {
                return false;
            }
            if identity_key == "name" {
                if let Some(input_type) = object.get("type") {
                    return candidate_object.get("type") == Some(input_type);
                }
            }
            true
        }) {
            return matched;
        }
    }
    existing.get(index).unwrap_or(&NULL)
}
