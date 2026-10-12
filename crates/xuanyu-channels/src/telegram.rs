use crate::{ChannelConfig, InboundChannelMessage, InboundHandler};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::RwLock;

pub struct TelegramLongPollAdapter {
    config: ChannelConfig,
    running: Arc<AtomicBool>,
    offset: Arc<RwLock<i64>>,
    bot_username: Arc<RwLock<String>>,
}

impl TelegramLongPollAdapter {
    pub fn new(config: ChannelConfig) -> Self {
        Self {
            config,
            running: Arc::new(AtomicBool::new(false)),
            offset: Arc::new(RwLock::new(0)),
            bot_username: Arc::new(RwLock::new(String::new())),
        }
    }
}

#[async_trait::async_trait]
impl super::ChannelAdapter for TelegramLongPollAdapter {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn kind(&self) -> &str {
        &self.config.kind
    }

    async fn start(&self, handler: InboundHandler) -> anyhow::Result<()> {
        let token = self
            .config
            .token
            .clone()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("telegram token required for long poll"))?;
        let proxy_url = resolve_proxy_url(self.config.extra.as_ref());
        let client = build_http_client(proxy_url.as_deref())?;
        let me = client
            .get(format!("https://api.telegram.org/bot{token}/getMe"))
            .send()
            .await?
            .error_for_status()?
            .json::<serde_json::Value>()
            .await?;
        let username = me
            .pointer("/result/username")
            .and_then(|value| value.as_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        *self.bot_username.write().await = username.clone();
        tracing::info!(
            channel = %self.config.id,
            username = %username,
            proxy = %mask_proxy_url(proxy_url.as_deref()),
            "telegram bot connected"
        );

        self.running.store(true, Ordering::SeqCst);
        let running = self.running.clone();
        let offset = self.offset.clone();
        let bot_username = self.bot_username.clone();
        let channel_id = self.config.id.clone();
        let mention_only = mention_only_enabled(self.config.extra.as_ref());
        tokio::spawn(async move {
            while running.load(Ordering::SeqCst) {
                let current_offset = *offset.read().await;
                let url = format!(
                    "https://api.telegram.org/bot{token}/getUpdates?timeout=25&offset={current_offset}"
                );
                match client.get(&url).send().await {
                    Ok(response) if response.status().is_success() => {
                        if let Ok(body) = response.json::<serde_json::Value>().await {
                            if let Some(results) = body.get("result").and_then(|v| v.as_array()) {
                                let username = bot_username.read().await.clone();
                                for update in results {
                                    if let Some(update_id) =
                                        update.get("update_id").and_then(|v| v.as_i64())
                                    {
                                        *offset.write().await = update_id + 1;
                                    }
                                    if let Some(message) =
                                        parse_telegram_update(&channel_id, update, &username)
                                    {
                                        if mention_only && !message.is_mention.unwrap_or(true) {
                                            continue;
                                        }
                                        handler(message);
                                    }
                                }
                            }
                        }
                    }
                    Ok(response) => {
                        tracing::warn!(
                            channel = %channel_id,
                            status = %response.status(),
                            "telegram long poll failed"
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    }
                    Err(err) => {
                        tracing::warn!(channel = %channel_id, "telegram long poll error: {err:#}");
                        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
                    }
                }
            }
        });
        tracing::info!(channel = %self.config.id, "telegram long poll started");
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn send_text(&self, session_id: &str, text: &str) -> anyhow::Result<()> {
        let token = self
            .config
            .token
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("telegram token missing"))?;
        let chat_id = session_id
            .strip_prefix("telegram:")
            .unwrap_or(session_id);
        let proxy_url = resolve_proxy_url(self.config.extra.as_ref());
        let client = build_http_client(proxy_url.as_deref())?;
        let url = format!("https://api.telegram.org/bot{token}/sendMessage");
        for chunk in split_telegram_text(text, 3500) {
            let response = client
                .post(&url)
                .json(&serde_json::json!({
                    "chat_id": chat_id,
                    "text": chunk,
                    "disable_web_page_preview": true
                }))
                .send()
                .await?;
            if !response.status().is_success() {
                anyhow::bail!("telegram sendMessage failed: {}", response.status());
            }
        }
        Ok(())
    }
}

fn mention_only_enabled(extra: Option<&std::collections::BTreeMap<String, String>>) -> bool {
    extra
        .and_then(|map| map.get("mentionOnly").or_else(|| map.get("mention_only")))
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(true)
}

fn resolve_proxy_url(extra: Option<&std::collections::BTreeMap<String, String>>) -> Option<String> {
    if let Some(from_config) = extra
        .and_then(|map| map.get("proxyUrl").or_else(|| map.get("proxy_url")))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return Some(from_config);
    }
    for key in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

fn build_http_client(proxy_url: Option<&str>) -> anyhow::Result<reqwest::Client> {
    let mut builder = reqwest::Client::builder().timeout(std::time::Duration::from_secs(60));
    if let Some(proxy_url) = proxy_url {
        let proxy = reqwest::Proxy::all(proxy_url)
            .map_err(|err| anyhow::anyhow!("invalid telegram proxyUrl: {err}"))?;
        builder = builder.proxy(proxy);
    }
    builder
        .build()
        .map_err(|err| anyhow::anyhow!("failed to build telegram http client: {err}"))
}

fn mask_proxy_url(proxy_url: Option<&str>) -> String {
    let Some(proxy_url) = proxy_url else {
        return "direct".to_string();
    };
    match reqwest::Url::parse(proxy_url) {
        Ok(mut url) => {
            if !url.username().is_empty() {
                let _ = url.set_username("***");
            }
            if url.password().is_some() {
                let _ = url.set_password(Some("***"));
            }
            url.to_string()
        }
        Err(_) => "[invalid-proxy-url]".to_string(),
    }
}

fn split_telegram_text(text: &str, max_chars: usize) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.chars().count() <= max_chars {
        return vec![trimmed.to_string()];
    }
    let mut parts = Vec::new();
    let mut buf = String::new();
    for ch in trimmed.chars() {
        buf.push(ch);
        if buf.chars().count() >= max_chars {
            parts.push(std::mem::take(&mut buf));
        }
    }
    if !buf.is_empty() {
        parts.push(buf);
    }
    parts
}

fn parse_telegram_update(
    channel_id: &str,
    update: &serde_json::Value,
    bot_username: &str,
) -> Option<InboundChannelMessage> {
    let message = update.get("message").or_else(|| update.get("edited_message"))?;
    let text = message
        .get("text")
        .or_else(|| message.get("caption"))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let chat = message.get("chat")?;
    let chat_id = chat.get("id")?;
    let chat_type = chat
        .get("type")
        .and_then(|value| value.as_str())
        .unwrap_or("private");
    let is_group = matches!(chat_type, "group" | "supergroup");
    let mention = if bot_username.is_empty() {
        false
    } else {
        text.to_ascii_lowercase()
            .contains(&format!("@{bot_username}"))
    };
    let is_mention = !is_group || mention || chat_type == "private";
    let external_user_id = message
        .get("from")
        .and_then(|from| from.get("id"))
        .map(|id| id.to_string())
        .unwrap_or_else(|| chat_id.to_string());
    Some(InboundChannelMessage {
        channel_id: channel_id.to_string(),
        external_user_id,
        text,
        session_id: Some(format!("telegram:{chat_id}")),
        is_mention: Some(is_mention),
    })
}
