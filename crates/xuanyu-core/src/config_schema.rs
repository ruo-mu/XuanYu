//! App config gate: reject clearly invalid shapes on save (desktop equivalent of schema validation).

use serde_json::Value;

/// Validate merged app config before persist. Returns human-readable errors.
pub fn validate_app_config(config: &Value) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    if !config.is_object() && !config.is_null() {
        return Err(vec!["config root must be a JSON object".to_string()]);
    }
    if let Some(provider) = config.get("provider") {
        validate_provider(provider, "provider", &mut errors);
    }
    if let Some(routing) = config.get("modelRouting") {
        if let Some(aux) = routing.get("auxiliaryModels").and_then(Value::as_object) {
            for (key, value) in aux {
                if !is_known_aux_task(key) {
                    errors.push(format!(
                        "modelRouting.auxiliaryModels.{key}: unknown task (expected title|summary|memory|intent|keyfacts|planning|implementation|review)"
                    ));
                }
                if value.is_object() {
                    validate_provider_like(value, &format!("modelRouting.auxiliaryModels.{key}"), &mut errors);
                }
            }
        }
    }
    if let Some(memory) = config.get("memory") {
        if let Some(backend) = memory.get("backend").and_then(Value::as_str) {
            let backend = backend.trim().to_ascii_lowercase();
            if !matches!(backend.as_str(), "postgres" | "postgresql" | "") {
                errors.push(format!(
                    "memory.backend '{backend}' is not supported (postgres)"
                ));
            }
        }
    }
    // gateway 段不校验：桌面端固定走 Electron IPC，port 0 即「不开 TCP 监听」。
    if let Some(permissions) = config.get("permissions") {
        if let Some(mode) = permissions.get("mode").and_then(Value::as_str) {
            let mode = mode.trim().to_ascii_lowercase();
            if !matches!(mode.as_str(), "default" | "auto" | "plan" | "") {
                errors.push(format!(
                    "permissions.mode '{mode}' is invalid (default|auto|plan)"
                ));
            }
        }
    }
    if let Some(cm) = config.get("contextManagement") {
        if let Some(threshold) = cm
            .get("autoCompactThreshold")
            .or_else(|| cm.get("auto_compact_threshold"))
        {
            let ok = threshold
                .as_f64()
                .filter(|v| (0.05..=0.99).contains(v))
                .is_some();
            if !ok {
                errors.push(
                    "contextManagement.autoCompactThreshold must be a number in [0.05, 0.99]"
                        .to_string(),
                );
            }
        }
        if let Some(load_mode) = cm.get("loadMode").and_then(Value::as_str) {
            let load_mode = load_mode.trim().to_ascii_lowercase();
            if !matches!(load_mode.as_str(), "lazy" | "balanced" | "eager" | "") {
                errors.push(format!(
                    "contextManagement.loadMode '{load_mode}' is invalid (lazy|balanced|eager)"
                ));
            }
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

fn is_known_aux_task(key: &str) -> bool {
    matches!(
        key.trim().to_ascii_lowercase().as_str(),
        "title"
            | "summary"
            | "memory"
            | "intent"
            | "keyfacts"
            | "key_facts"
            | "planning"
            | "implementation"
            | "review"
    )
}

fn validate_provider(provider: &Value, path: &str, errors: &mut Vec<String>) {
    validate_provider_like(provider, path, errors);
    let model = provider
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let provider_type = provider
        .get("type")
        .or_else(|| provider.get("providerType"))
        .or_else(|| provider.get("provider_type"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if !provider_type.is_empty() && model.is_empty() {
        errors.push(format!("{path}.model is required when provider type is set"));
    }
}

fn validate_provider_like(provider: &Value, path: &str, errors: &mut Vec<String>) {
    if let Some(temp) = provider.get("temperature") {
        if let Some(v) = temp.as_f64() {
            if !(0.0..=2.0).contains(&v) {
                errors.push(format!("{path}.temperature must be in [0, 2]"));
            }
        } else if !temp.is_null() {
            errors.push(format!("{path}.temperature must be a number"));
        }
    }
    if let Some(max_tokens) = provider.get("maxTokens").or_else(|| provider.get("max_tokens")) {
        let ok = max_tokens
            .as_u64()
            .or_else(|| max_tokens.as_i64().map(|v| v as u64))
            .filter(|v| (1..=200_000).contains(v))
            .is_some();
        if !ok && !max_tokens.is_null() {
            errors.push(format!("{path}.maxTokens must be an integer in [1, 200000]"));
        }
    }
}
