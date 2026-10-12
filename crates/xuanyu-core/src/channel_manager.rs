//! Desktop channel connection manager: config persistence + runtime status for ChannelPanel IPC.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedChannelConfig {
    #[serde(rename = "type")]
    pub channel_type: String,
    pub name: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mention_only: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowlist: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ChannelRuntimeEntry {
    pub config: ManagedChannelConfig,
    pub status: String,
    pub error: Option<String>,
    pub message_count: u64,
    pub active: bool,
}

#[derive(Default)]
pub struct ChannelRuntimeStore {
    pub entries: BTreeMap<String, ChannelRuntimeEntry>,
    pub qr_draft_id: Option<String>,
    /// When set to `true`, in-flight WeChat QR login aborts.
    pub qr_cancel_tx: Option<tokio::sync::watch::Sender<bool>>,
}

#[derive(Debug, Clone)]
pub struct BuiltinChannelAdapter {
    pub channel_type: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub setup_hint: &'static str,
    pub availability: &'static str,
}

pub fn builtin_channel_adapters() -> Vec<BuiltinChannelAdapter> {
    vec![
        BuiltinChannelAdapter {
            channel_type: "wechat",
            label: "WeChat",
            description: "iLink QR bind (desktop engine)",
            setup_hint: "Scan the QR code with WeChat to bind; credentials are stored under channels/wechat.",
            availability: "supported",
        },
        BuiltinChannelAdapter {
            channel_type: "qq",
            label: "QQ",
            description: "Official Bot API v2",
            setup_hint: "Provide App ID and App Secret.",
            availability: "supported",
        },
        BuiltinChannelAdapter {
            channel_type: "telegram",
            label: "Telegram",
            description: "BotFather token via long poll",
            setup_hint: "Paste the bot token from @BotFather.",
            availability: "supported",
        },
        BuiltinChannelAdapter {
            channel_type: "feishu",
            label: "Feishu",
            description: "Enterprise app gateway",
            setup_hint: "Provide App ID and App Secret (token).",
            availability: "supported",
        },
        BuiltinChannelAdapter {
            channel_type: "discord",
            label: "Discord",
            description: "Discord bot gateway",
            setup_hint: "Provide a Discord bot token.",
            availability: "supported",
        },
        BuiltinChannelAdapter {
            channel_type: "whatsapp",
            label: "WhatsApp",
            description: "Meta Cloud API webhook",
            setup_hint: "Provide Phone Number ID, access token, verify token, and app secret.",
            availability: "supported",
        },
        BuiltinChannelAdapter {
            channel_type: "line",
            label: "LINE",
            description: "LINE Messaging API",
            setup_hint: "Provide Channel Access Token and Channel Secret.",
            availability: "supported",
        },
        BuiltinChannelAdapter {
            channel_type: "dingtalk",
            label: "DingTalk",
            description: "Enterprise bot webhook",
            setup_hint: "Provide sign secret and webhook port.",
            availability: "supported",
        },
    ]
}

pub fn channel_adapter_supported(channel_type: &str) -> bool {
    matches!(
        channel_type.to_ascii_lowercase().as_str(),
        "discord"
            | "telegram"
            | "feishu"
            | "lark"
            | "wechat"
            | "qq"
            | "dingtalk"
            | "whatsapp"
            | "line"
    )
}

pub fn channel_has_credentials(config: &ManagedChannelConfig) -> bool {
    let token_ok = config
        .token
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_some();
    let kind = config.channel_type.to_ascii_lowercase();
    let extra = config.extra.as_ref();
    let extra_nonempty = |keys: &[&str]| -> bool {
        extra
            .map(|map| {
                keys.iter().any(|key| {
                    map.get(*key)
                        .map(|value| !value.trim().is_empty())
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    };
    match kind.as_str() {
        "qq" | "feishu" | "lark" => token_ok && extra_nonempty(&["appId", "app_id"]),
        "dingtalk" => token_ok || extra_nonempty(&["secret", "appSecret", "app_secret"]),
        "whatsapp" => {
            token_ok
                && extra_nonempty(&["phoneNumberId", "phone_number_id"])
                && extra_nonempty(&["verifyToken", "verify_token"])
                && extra_nonempty(&["appSecret", "app_secret"])
        }
        "line" | "telegram" | "discord" => token_ok,
        _ => {
            if token_ok {
                return true;
            }
            extra_nonempty(&[
                "appId",
                "app_id",
                "appSecret",
                "app_secret",
                "accountId",
                "botToken",
                "accessToken",
            ])
        }
    }
}

pub fn connection_key(config: &ManagedChannelConfig) -> String {
    config
        .connection_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("{}:{}", config.channel_type, config.name))
}

pub fn parse_managed_channel_config(value: &Value) -> Option<ManagedChannelConfig> {
    let object = value.as_object()?;
    let channel_type = object
        .get("type")
        .or_else(|| object.get("kind"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_ascii_lowercase();
    let name = object
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| channel_type.clone());
    let enabled = object
        .get("enabled")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let token = object
        .get("token")
        .and_then(Value::as_str)
        .map(|value| value.to_string());
    let mention_only = object.get("mentionOnly").and_then(Value::as_bool);
    let allowlist = object
        .get("allowlist")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        });
    let extra = object.get("extra").and_then(Value::as_object).map(|map| {
        map.iter()
            .filter_map(|(key, value)| {
                value
                    .as_str()
                    .map(|text| (key.clone(), text.to_string()))
            })
            .collect::<BTreeMap<_, _>>()
    });
    let tenant_id = object
        .get("tenantId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let connection_id = object
        .get("connectionId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    Some(ManagedChannelConfig {
        channel_type,
        name,
        enabled,
        token,
        mention_only,
        allowlist,
        extra,
        tenant_id,
        connection_id,
    })
}

pub fn managed_channel_to_value(config: &ManagedChannelConfig) -> Value {
    let mut object = Map::new();
    object.insert("type".into(), json!(config.channel_type));
    object.insert("name".into(), json!(config.name));
    object.insert("enabled".into(), json!(config.enabled));
    if let Some(token) = &config.token {
        object.insert("token".into(), json!(token));
    }
    if let Some(mention_only) = config.mention_only {
        object.insert("mentionOnly".into(), json!(mention_only));
    }
    if let Some(allowlist) = &config.allowlist {
        object.insert("allowlist".into(), json!(allowlist));
    }
    if let Some(extra) = &config.extra {
        object.insert("extra".into(), json!(extra));
    }
    if let Some(tenant_id) = &config.tenant_id {
        object.insert("tenantId".into(), json!(tenant_id));
    }
    if let Some(connection_id) = &config.connection_id {
        object.insert("connectionId".into(), json!(connection_id));
    }
    Value::Object(object)
}

pub fn redact_managed_channel_config(config: &ManagedChannelConfig) -> Value {
    let mut value = managed_channel_to_value(config);
    if let Some(object) = value.as_object_mut() {
        if let Some(token) = object.get("token").and_then(Value::as_str) {
            if !token.is_empty() {
                object.insert("token".into(), json!("***"));
            }
        }
    }
    value
}

pub fn load_channel_configs_from_app_config(config: &Value) -> Vec<ManagedChannelConfig> {
    let items = config
        .get("channels")
        .and_then(|value| {
            if value.is_array() {
                value.as_array()
            } else {
                value.get("items").and_then(Value::as_array)
            }
        })
        .cloned()
        .unwrap_or_default();
    items
        .iter()
        .filter_map(parse_managed_channel_config)
        .collect()
}

pub fn write_channel_configs_to_app_config(config: &mut Value, channels: &[ManagedChannelConfig]) {
    let values = channels
        .iter()
        .map(managed_channel_to_value)
        .collect::<Vec<_>>();
    match config.get_mut("channels") {
        Some(Value::Object(object)) => {
            object.insert("items".into(), json!(values));
        }
        Some(Value::Array(_)) | None => {
            config
                .as_object_mut()
                .map(|object| object.insert("channels".into(), json!(values)));
        }
        Some(_) => {
            if let Some(object) = config.as_object_mut() {
                object.insert("channels".into(), json!(values));
            }
        }
    }
}

pub fn merge_managed_channel_config(
    existing: Option<&ManagedChannelConfig>,
    incoming: ManagedChannelConfig,
) -> ManagedChannelConfig {
    let Some(existing) = existing else {
        let connection_id = incoming
            .connection_id
            .clone()
            .unwrap_or_else(|| connection_key(&incoming));
        return ManagedChannelConfig {
            connection_id: Some(connection_id),
            ..incoming
        };
    };
    let mut merged_extra = existing.extra.clone().unwrap_or_default();
    if let Some(extra) = incoming.extra {
        for (key, value) in extra {
            if value.is_empty() {
                merged_extra.remove(&key);
            } else {
                merged_extra.insert(key, value);
            }
        }
    }
    let token = match incoming.token {
        Some(token) => Some(token),
        None => existing.token.clone(),
    };
    let connection_id = incoming
        .connection_id
        .or_else(|| existing.connection_id.clone())
        .or_else(|| Some(connection_key(existing)));
    ManagedChannelConfig {
        channel_type: if incoming.channel_type.is_empty() {
            existing.channel_type.clone()
        } else {
            incoming.channel_type
        },
        name: if incoming.name.is_empty() {
            existing.name.clone()
        } else {
            incoming.name
        },
        enabled: incoming.enabled,
        token,
        mention_only: incoming.mention_only.or(existing.mention_only),
        allowlist: incoming.allowlist.or_else(|| existing.allowlist.clone()),
        extra: if merged_extra.is_empty() {
            None
        } else {
            Some(merged_extra)
        },
        tenant_id: incoming.tenant_id.or_else(|| existing.tenant_id.clone()),
        connection_id,
    }
}

pub fn adapters_list_payload() -> Value {
    json!({
        "adapters": builtin_channel_adapters()
            .into_iter()
            .map(|adapter| json!({
                "type": adapter.channel_type,
                "label": adapter.label,
                "description": adapter.description,
                "setupHint": adapter.setup_hint,
                "availability": adapter.availability,
            }))
            .collect::<Vec<_>>()
    })
}

pub fn connection_runtime_payload(entry: &ChannelRuntimeEntry) -> Value {
    let adapter = builtin_channel_adapters()
        .into_iter()
        .find(|item| item.channel_type == entry.config.channel_type);
    let availability = if channel_adapter_supported(&entry.config.channel_type) {
        "supported"
    } else {
        "unsupported"
    };
    json!({
        "type": entry.config.channel_type,
        "label": adapter.map(|item| item.label).unwrap_or(entry.config.channel_type.as_str()),
        "availability": availability,
        "status": entry.status,
        "error": entry.error,
        "messageCount": entry.message_count,
        "config": redact_managed_channel_config(&entry.config),
    })
}
