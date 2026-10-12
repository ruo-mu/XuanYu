use crate::{ChannelConfig, InboundChannelMessage, InboundHandler};
use futures_util::{SinkExt, StreamExt};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::time::{interval, MissedTickBehavior};
use tokio_tungstenite::{connect_async, tungstenite::Message};

pub struct FeishuGatewayAdapter {
    config: ChannelConfig,
    running: Arc<AtomicBool>,
}

impl FeishuGatewayAdapter {
    pub fn new(config: ChannelConfig) -> Self {
        Self {
            config,
            running: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[async_trait::async_trait]
impl super::ChannelAdapter for FeishuGatewayAdapter {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn kind(&self) -> &str {
        &self.config.kind
    }

    async fn start(&self, handler: InboundHandler) -> anyhow::Result<()> {
        let app_id = self
            .config
            .app_id
            .clone()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("feishu app_id required"))?;
        let app_secret = self
            .config
            .token
            .clone()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("feishu app_secret required"))?;
        self.running.store(true, Ordering::SeqCst);
        let running = self.running.clone();
        let channel_id = self.config.id.clone();
        tokio::spawn(async move {
            let mut reconnect_attempt = 0u32;
            while running.load(Ordering::SeqCst) {
                match run_feishu_gateway_once(
                    &channel_id,
                    &app_id,
                    &app_secret,
                    handler.clone(),
                    &running,
                )
                .await
                {
                    Ok(()) => {
                        if !running.load(Ordering::SeqCst) {
                            break;
                        }
                        reconnect_attempt = reconnect_attempt.saturating_add(1);
                        tracing::warn!(
                            channel = %channel_id,
                            attempt = reconnect_attempt,
                            "feishu gateway disconnected; reconnecting"
                        );
                        tokio::time::sleep(crate::gateway_reconnect_delay(reconnect_attempt))
                            .await;
                    }
                    Err(err) => {
                        reconnect_attempt = reconnect_attempt.saturating_add(1);
                        tracing::warn!(
                            channel = %channel_id,
                            attempt = reconnect_attempt,
                            "feishu gateway error: {err:#}"
                        );
                        tokio::time::sleep(crate::gateway_reconnect_delay(reconnect_attempt))
                            .await;
                    }
                }
            }
        });
        tracing::info!(channel = %self.config.id, "feishu gateway started");
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn send_text(&self, session_id: &str, text: &str) -> anyhow::Result<()> {
        let app_id = self
            .config
            .app_id
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("feishu app_id missing"))?;
        let app_secret = self
            .config
            .token
            .as_deref()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| anyhow::anyhow!("feishu app_secret missing"))?;
        let token = fetch_tenant_access_token(app_id, app_secret).await?;
        let chat_id = session_id
            .strip_prefix("feishu:")
            .unwrap_or(session_id);
        let client = reqwest::Client::new();
        let response = client
            .post("https://open.feishu.cn/open-apis/im/v1/messages?receive_id_type=chat_id")
            .bearer_auth(token)
            .json(&serde_json::json!({
                "receive_id": chat_id,
                "msg_type": "text",
                "content": serde_json::json!({ "text": text }).to_string()
            }))
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!("feishu send message failed: {}", response.status());
        }
        Ok(())
    }
}

async fn run_feishu_gateway_once(
    channel_id: &str,
    app_id: &str,
    app_secret: &str,
    handler: InboundHandler,
    running: &AtomicBool,
) -> anyhow::Result<()> {
    let _token = fetch_tenant_access_token(app_id, app_secret).await?;
    let url = format!("wss://ws-open-platform.feishu.cn/ws/v2?app_id={app_id}");
    let (mut ws, _) = connect_async(url).await?;
    let mut heartbeat = interval(std::time::Duration::from_secs(30));
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);

    while running.load(Ordering::SeqCst) {
        tokio::select! {
            _ = heartbeat.tick() => {
                let ping = serde_json::json!({ "type": "ping" });
                ws.send(Message::Text(ping.to_string().into())).await?;
            }
            incoming = ws.next() => {
                let Some(incoming) = incoming else { break; };
                let message = incoming?;
                let text = match message {
                    Message::Text(text) => text.to_string(),
                    Message::Ping(payload) => {
                        ws.send(Message::Pong(payload)).await?;
                        continue;
                    }
                    Message::Close(_) => break,
                    _ => continue,
                };
                let payload: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
                if payload.get("type").and_then(|value| value.as_str()) == Some("url_verification") {
                    if let Some(challenge) = payload.get("challenge").and_then(|value| value.as_str()) {
                        let reply = serde_json::json!({ "challenge": challenge });
                        ws.send(Message::Text(reply.to_string().into())).await?;
                    }
                    continue;
                }
                if let Some(message) = parse_feishu_event(channel_id, &payload) {
                    handler(message);
                }
            }
        }
    }
    Ok(())
}

async fn fetch_tenant_access_token(app_id: &str, app_secret: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::new();
    let response = client
        .post("https://open.feishu.cn/open-apis/auth/v3/tenant_access_token/internal")
        .json(&serde_json::json!({
            "app_id": app_id,
            "app_secret": app_secret
        }))
        .send()
        .await?;
    let body: serde_json::Value = response.json().await?;
    body.get("tenant_access_token")
        .and_then(|value| value.as_str())
        .map(str::to_string)
        .ok_or_else(|| anyhow::anyhow!("feishu tenant token missing: {body}"))
}

fn parse_feishu_event(channel_id: &str, payload: &serde_json::Value) -> Option<InboundChannelMessage> {
    let header = payload.get("header")?;
    let event_type = header.get("event_type").and_then(|value| value.as_str())?;
    if event_type != "im.message.receive_v1" {
        return None;
    }
    let event = payload.get("event")?;
    let message = event.get("message")?;
    let text = message
        .get("content")
        .and_then(|value| value.as_str())
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|content| content.get("text").and_then(|value| value.as_str()).map(|s| s.to_string()))
        .or_else(|| {
            message
                .get("content")
                .and_then(|value| value.as_str())
                .map(|s| s.to_string())
        })?
        .trim()
        .to_string();
    if text.is_empty() {
        return None;
    }
    let chat_id = message.get("chat_id").map(|id| id.to_string()).unwrap_or_default();
    let external_user_id = event
        .pointer("/sender/sender_id/open_id")
        .or_else(|| event.pointer("/sender/sender_id/user_id"))
        .map(|id| id.to_string())
        .unwrap_or_else(|| "feishu-user".to_string());
    Some(InboundChannelMessage {
        channel_id: channel_id.to_string(),
        external_user_id,
        text,
        session_id: Some(format!("feishu:{chat_id}")),
        is_mention: Some(true),
    })
}
