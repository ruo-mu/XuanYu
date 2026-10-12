mod dingtalk;
mod discord_gateway;
mod feishu_gateway;
mod line;
mod qq;
mod telegram;
mod webhook;
mod wechat;
mod whatsapp;
pub mod wechat_ilink;

pub use webhook::{send_outbound_webhook, WebhookChannelServer};
pub use wechat_ilink::{login_wechat_via_qr, WechatCredentials};

use async_trait::async_trait;
use dingtalk::DingtalkWebhookAdapter;
use discord_gateway::DiscordGatewayAdapter;
use feishu_gateway::FeishuGatewayAdapter;
use line::LineWebhookAdapter;
use qq::QqGatewayAdapter;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use telegram::TelegramLongPollAdapter;
use tokio::sync::RwLock;
use wechat::WechatLongPollAdapter;
use whatsapp::WhatsappWebhookAdapter;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelConfig {
    pub id: String,
    pub kind: String,
    pub enabled: bool,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub webhook_url: Option<String>,
    #[serde(default)]
    pub transport: Option<String>,
    #[serde(default)]
    pub app_id: Option<String>,
    #[serde(default)]
    pub extra: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InboundChannelMessage {
    pub channel_id: String,
    pub external_user_id: String,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_mention: Option<bool>,
}

pub type InboundHandler = Arc<dyn Fn(InboundChannelMessage) + Send + Sync>;

#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    fn id(&self) -> &str;
    fn kind(&self) -> &str;
    async fn start(&self, handler: InboundHandler) -> anyhow::Result<()>;
    async fn stop(&self) -> anyhow::Result<()>;
    async fn send_text(&self, session_id: &str, text: &str) -> anyhow::Result<()>;
    /// Optional typing indicator. `active=true` starts/keeps typing; `false` cancels.
    async fn set_typing(&self, _user_id: &str, _active: bool) -> anyhow::Result<()> {
        Ok(())
    }
}

struct WebhookChannelAdapter {
    config: ChannelConfig,
    running: RwLock<bool>,
}

impl WebhookChannelAdapter {
    fn new(config: ChannelConfig) -> Self {
        Self {
            config,
            running: RwLock::new(false),
        }
    }
}

#[async_trait]
impl ChannelAdapter for WebhookChannelAdapter {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn kind(&self) -> &str {
        &self.config.kind
    }

    async fn start(&self, _handler: InboundHandler) -> anyhow::Result<()> {
        *self.running.write().await = true;
        tracing::info!(channel = %self.config.id, kind = %self.config.kind, "channel webhook adapter ready");
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        *self.running.write().await = false;
        Ok(())
    }

    async fn send_text(&self, session_id: &str, text: &str) -> anyhow::Result<()> {
        send_outbound_webhook(&self.config, session_id, text).await
    }
}

pub struct ChannelHub {
    adapters: Vec<Arc<dyn ChannelAdapter>>,
    configs: Vec<ChannelConfig>,
    webhook_server: Arc<RwLock<Option<Arc<WebhookChannelServer>>>>,
    webhook_port: u16,
}

impl ChannelHub {
    pub fn from_configs(configs: &[ChannelConfig], webhook_port: u16) -> Self {
        Self::from_configs_with_data_dir(configs, webhook_port, PathBuf::from("."))
    }

    pub fn from_configs_with_data_dir(
        configs: &[ChannelConfig],
        webhook_port: u16,
        data_dir: PathBuf,
    ) -> Self {
        let enabled_configs = configs
            .iter()
            .filter(|config| config.enabled)
            .cloned()
            .collect::<Vec<_>>();
        let adapters = enabled_configs
            .iter()
            .map(|config| build_adapter(config, &data_dir) as Arc<dyn ChannelAdapter>)
            .collect();
        Self {
            adapters,
            configs: enabled_configs,
            webhook_server: Arc::new(RwLock::new(None)),
            webhook_port,
        }
    }

    pub fn adapters(&self) -> &[Arc<dyn ChannelAdapter>] {
        &self.adapters
    }

    pub fn webhook_port(&self) -> u16 {
        self.webhook_port
    }

    pub async fn start_all(&self, handler: InboundHandler) -> anyhow::Result<()> {
        for adapter in &self.adapters {
            adapter.start(handler.clone()).await?;
        }
        if self
            .configs
            .iter()
            .any(channel_uses_http_webhook)
        {
            let webhook_configs = self
                .configs
                .iter()
                .filter(|config| channel_uses_http_webhook(config))
                .cloned()
                .collect::<Vec<_>>();
            let server = Arc::new(WebhookChannelServer::new(
                self.webhook_port,
                webhook_configs,
                handler,
            ));
            let server_for_task = server.clone();
            tokio::spawn(async move {
                if let Err(err) = server_for_task.start().await {
                    tracing::warn!("channel webhook server stopped: {err:#}");
                }
            });
            *self.webhook_server.write().await = Some(server);
        }
        Ok(())
    }

    pub async fn stop_all(&self) -> anyhow::Result<()> {
        if let Some(server) = self.webhook_server.write().await.take() {
            server.stop().await;
        }
        for adapter in &self.adapters {
            adapter.stop().await?;
        }
        Ok(())
    }
}

fn build_adapter(config: &ChannelConfig, data_dir: &Path) -> Arc<dyn ChannelAdapter> {
    let transport = resolved_transport(config);
    let kind = config.kind.to_ascii_lowercase();
    match (transport.as_str(), kind.as_str()) {
        ("long_poll", "telegram") => Arc::new(TelegramLongPollAdapter::new(config.clone())),
        ("long_poll", "wechat") => {
            Arc::new(WechatLongPollAdapter::new(config.clone(), data_dir.to_path_buf()))
        }
        ("gateway", "discord") => Arc::new(DiscordGatewayAdapter::new(config.clone())),
        ("gateway", "feishu" | "lark") => Arc::new(FeishuGatewayAdapter::new(config.clone())),
        ("gateway", "qq") => Arc::new(QqGatewayAdapter::new(config.clone())),
        (_, "dingtalk") => Arc::new(DingtalkWebhookAdapter::new(config.clone())),
        (_, "whatsapp") => Arc::new(WhatsappWebhookAdapter::new(config.clone())),
        (_, "line") => Arc::new(LineWebhookAdapter::new(config.clone())),
        _ => Arc::new(WebhookChannelAdapter::new(config.clone())),
    }
}

fn resolved_transport(config: &ChannelConfig) -> String {
    if let Some(transport) = config
        .transport
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return transport.to_ascii_lowercase();
    }
    match config.kind.to_ascii_lowercase().as_str() {
        "telegram" if config
            .webhook_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none() =>
        {
            "long_poll".to_string()
        }
        "wechat" if config
            .token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some() =>
        {
            "long_poll".to_string()
        }
        "discord" if config
            .token
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some() =>
        {
            "gateway".to_string()
        }
        "feishu" | "lark" if config
            .app_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
            && config
                .token
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some() =>
        {
            "gateway".to_string()
        }
        "qq" if config
            .app_id
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
            && config
                .token
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_some() =>
        {
            "gateway".to_string()
        }
        "dingtalk" | "whatsapp" | "line" => "webhook".to_string(),
        _ => "webhook".to_string(),
    }
}

fn channel_uses_http_webhook(config: &ChannelConfig) -> bool {
    if !config.enabled {
        return false;
    }
    // Dedicated adapters bind their own HTTP listeners.
    if matches!(
        config.kind.to_ascii_lowercase().as_str(),
        "dingtalk" | "whatsapp" | "line" | "qq"
    ) {
        return false;
    }
    resolved_transport(config) == "webhook"
        && matches!(
            config.kind.to_ascii_lowercase().as_str(),
            "discord"
                | "telegram"
                | "feishu"
                | "lark"
                | "webhook"
                | "wechat"
                | "custom"
        )
}

pub fn default_channel_configs() -> Vec<ChannelConfig> {
    vec![
        ChannelConfig {
            id: "discord-main".to_string(),
            kind: "discord".to_string(),
            enabled: false,
            token: None,
            webhook_url: None,
            transport: Some("gateway".to_string()),
            app_id: None,
            extra: None,
        },
        ChannelConfig {
            id: "telegram-main".to_string(),
            kind: "telegram".to_string(),
            enabled: false,
            token: None,
            webhook_url: None,
            transport: Some("long_poll".to_string()),
            app_id: None,
            extra: None,
        },
        ChannelConfig {
            id: "feishu-main".to_string(),
            kind: "feishu".to_string(),
            enabled: false,
            token: None,
            webhook_url: None,
            transport: Some("gateway".to_string()),
            app_id: None,
            extra: None,
        },
        ChannelConfig {
            id: "wechat-main".to_string(),
            kind: "wechat".to_string(),
            enabled: false,
            token: None,
            webhook_url: None,
            transport: Some("long_poll".to_string()),
            app_id: None,
            extra: None,
        },
    ]
}

pub fn channel_webhook_port_from_config(config: &serde_json::Value) -> u16 {
    config
        .get("channels")
        .and_then(|value| value.get("webhookPort"))
        .or_else(|| config.get("channels").and_then(|value| value.get("webhook_port")))
        .and_then(serde_json::Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .unwrap_or(8787)
}

pub fn channel_route_map(configs: &[ChannelConfig], webhook_port: u16) -> HashMap<String, String> {
    configs
        .iter()
        .map(|config| {
            (
                config.id.clone(),
                format!(
                    "http://127.0.0.1:{webhook_port}/channels/webhook/{}",
                    config.id
                ),
            )
        })
        .collect()
}

pub fn gateway_reconnect_delay(attempt: u32) -> std::time::Duration {
    let base = 5u64;
    let shift = attempt.saturating_sub(1).min(4);
    std::time::Duration::from_secs(base.saturating_mul(1u64 << shift).min(60))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ChannelCredentialStatus {
    pub id: String,
    pub kind: String,
    pub transport: String,
    pub enabled: bool,
    pub ready: bool,
    pub status: String,
    pub missing: Vec<String>,
    pub detail: String,
}

/// Production credential readiness for Discord / Feishu / Telegram.
pub fn channel_credential_status(config: &ChannelConfig) -> ChannelCredentialStatus {
    let transport = resolved_transport(config);
    let kind = config.kind.to_ascii_lowercase();
    let mut missing = Vec::new();
    match kind.as_str() {
        "discord" if transport == "gateway" => {
            if config
                .token
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                missing.push("token (Discord Bot Token)".to_string());
            }
        }
        "feishu" | "lark" if transport == "gateway" => {
            if config
                .app_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                missing.push("appId".to_string());
            }
            if config
                .token
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                missing.push("token (app_secret / tenant secret)".to_string());
            }
        }
        "telegram" if transport == "long_poll" => {
            if config
                .token
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            {
                missing.push("token (Telegram Bot Token)".to_string());
            }
        }
        "discord" | "telegram" | "feishu" | "lark" | "webhook" if transport == "webhook"
            // webhook adapter can start without outbound token; outbound send needs webhookUrl
            && config
                .webhook_url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .is_none()
            => {
                missing.push("webhookUrl (outbound)".to_string());
            }
        _ => {}
    }
    let ready = config.enabled && missing.is_empty();
    let status = if !config.enabled {
        "disabled".to_string()
    } else if missing.is_empty() {
        "ready".to_string()
    } else {
        "needs_config".to_string()
    };
    let detail = if missing.is_empty() {
        format!("{kind}/{transport} credentials present")
    } else {
        format!("missing: {}", missing.join(", "))
    };
    ChannelCredentialStatus {
        id: config.id.clone(),
        kind: config.kind.clone(),
        transport,
        enabled: config.enabled,
        ready,
        status,
        missing,
        detail,
    }
}
