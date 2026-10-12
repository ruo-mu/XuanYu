use crate::{ChannelConfig, InboundChannelMessage, InboundHandler};
use futures_util::{SinkExt, StreamExt};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::time::{interval, MissedTickBehavior};
use tokio_tungstenite::{connect_async, tungstenite::Message};

pub struct DiscordGatewayAdapter {
    config: ChannelConfig,
    running: Arc<AtomicBool>,
}

impl DiscordGatewayAdapter {
    pub fn new(config: ChannelConfig) -> Self {
        Self {
            config,
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[async_trait::async_trait]
impl super::ChannelAdapter for DiscordGatewayAdapter {
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
            .ok_or_else(|| anyhow::anyhow!("discord token required for gateway"))?;
        self.running.store(true, Ordering::SeqCst);
        let running = self.running.clone();
        let channel_id = self.config.id.clone();
        tokio::spawn(async move {
            let mut reconnect_attempt = 0u32;
            while running.load(Ordering::SeqCst) {
                match run_discord_gateway_once(&channel_id, &token, handler.clone(), &running).await
                {
                    Ok(()) => {
                        if !running.load(Ordering::SeqCst) {
                            break;
                        }
                        reconnect_attempt = reconnect_attempt.saturating_add(1);
                        tracing::warn!(
                            channel = %channel_id,
                            attempt = reconnect_attempt,
                            "discord gateway disconnected; reconnecting"
                        );
                        tokio::time::sleep(crate::gateway_reconnect_delay(reconnect_attempt))
                            .await;
                    }
                    Err(err) => {
                        reconnect_attempt = reconnect_attempt.saturating_add(1);
                        tracing::warn!(
                            channel = %channel_id,
                            attempt = reconnect_attempt,
                            "discord gateway error: {err:#}"
                        );
                        tokio::time::sleep(crate::gateway_reconnect_delay(reconnect_attempt))
                            .await;
                    }
                }
            }
        });
        tracing::info!(channel = %self.config.id, "discord gateway started");
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
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("discord token required for send_text"))?;
        let channel_id = session_id
            .strip_prefix("discord:")
            .unwrap_or(session_id)
            .trim();
        if channel_id.is_empty() {
            anyhow::bail!("discord send_text requires channel id");
        }
        let client = reqwest::Client::new();
        let url = format!("https://discord.com/api/v10/channels/{channel_id}/messages");
        for chunk in split_discord_text(text, 1900) {
            let response = client
                .post(&url)
                .header("Authorization", format!("Bot {token}"))
                .header("Content-Type", "application/json")
                .json(&serde_json::json!({ "content": chunk }))
                .send()
                .await?;
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                anyhow::bail!(
                    "Discord send failed HTTP {status}: {}",
                    body.chars().take(200).collect::<String>()
                );
            }
        }
        Ok(())
    }
}

fn split_discord_text(text: &str, max_chars: usize) -> Vec<String> {
    if text.chars().count() <= max_chars {
        return vec![text.to_string()];
    }
    let mut out = Vec::new();
    let mut buf = String::new();
    for ch in text.chars() {
        if buf.chars().count() >= max_chars {
            out.push(std::mem::take(&mut buf));
        }
        buf.push(ch);
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

async fn run_discord_gateway_once(
    channel_id: &str,
    token: &str,
    handler: InboundHandler,
    running: &AtomicBool,
) -> anyhow::Result<()> {
    let (mut ws, _) = connect_async("wss://gateway.discord.gg/?v=10&encoding=json").await?;
    let mut heartbeat = interval(std::time::Duration::from_secs(45));
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut heartbeat_enabled = false;
    let mut last_sequence: Option<u64> = None;

    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }
        tokio::select! {
            _ = heartbeat.tick(), if heartbeat_enabled => {
                let payload = serde_json::json!({ "op": 1, "d": last_sequence });
                ws.send(Message::Text(payload.to_string().into())).await?;
            }
            incoming = ws.next() => {
                let Some(incoming) = incoming else {
                    break;
                };
                let message = incoming?;
                let text = match message {
                    Message::Text(text) => text,
                    Message::Ping(payload) => {
                        ws.send(Message::Pong(payload)).await?;
                        continue;
                    }
                    Message::Close(_) => break,
                    _ => continue,
                };
                let payload: serde_json::Value = serde_json::from_str(&text)?;
                if let Some(sequence) = payload.get("s").and_then(|value| value.as_u64()) {
                    last_sequence = Some(sequence);
                }
                let op = payload.get("op").and_then(|value| value.as_u64()).unwrap_or(999);
                match op {
                    10 => {
                        let heartbeat_ms = payload
                            .pointer("/d/heartbeat_interval")
                            .and_then(|value| value.as_u64())
                            .unwrap_or(45_000)
                            .max(1000);
                        heartbeat = interval(std::time::Duration::from_millis(heartbeat_ms));
                        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
                        heartbeat_enabled = true;
                        let identify = serde_json::json!({
                            "op": 2,
                            "d": {
                                "token": token,
                                "intents": 37377,
                                "properties": {
                                    "os": "linux",
                                    "browser": "xuanyu-agent",
                                    "device": "xuanyu-agent"
                                }
                            }
                        });
                        ws.send(Message::Text(identify.to_string().into())).await?;
                    }
                    0 => {
                        let event_type = payload
                            .get("t")
                            .and_then(|value| value.as_str())
                            .unwrap_or_default();
                        if event_type == "MESSAGE_CREATE" {
                            if let Some(message) = parse_discord_message(channel_id, payload.get("d")) {
                                handler(message);
                            }
                        }
                    }
                    7 | 9 => break,
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn parse_discord_message(
    channel_id: &str,
    payload: Option<&serde_json::Value>,
) -> Option<InboundChannelMessage> {
    let payload = payload?;
    if payload
        .get("author")
        .and_then(|author| author.get("bot"))
        .and_then(|value| value.as_bool())
        == Some(true)
    {
        return None;
    }
    let text = payload
        .get("content")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let external_user_id = payload
        .pointer("/author/id")
        .and_then(|value| value.as_str())
        .unwrap_or("discord-user")
        .to_string();
    let dm_channel = payload
        .get("channel_id")
        .and_then(|value| value.as_str())
        .map(|id| format!("discord:{id}"));
    Some(InboundChannelMessage {
        channel_id: channel_id.to_string(),
        external_user_id,
        text,
        session_id: dm_channel,
        is_mention: Some(true),
    })
}
