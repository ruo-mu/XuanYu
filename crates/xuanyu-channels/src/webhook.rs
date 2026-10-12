use crate::{ChannelConfig, InboundChannelMessage, InboundHandler};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Router,
};
use serde_json::Value;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Clone)]
pub struct WebhookChannelState {
    channels: Arc<HashMap<String, ChannelConfig>>,
    handler: InboundHandler,
}

pub struct WebhookChannelServer {
    port: u16,
    state: WebhookChannelState,
    shutdown: Arc<RwLock<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl WebhookChannelServer {
    pub fn new(port: u16, channels: Vec<ChannelConfig>, handler: InboundHandler) -> Self {
        let map = channels
            .into_iter()
            .map(|config| (config.id.clone(), config))
            .collect();
        Self {
            port,
            state: WebhookChannelState {
                channels: Arc::new(map),
                handler,
            },
            shutdown: Arc::new(RwLock::new(None)),
        }
    }

    pub async fn start(&self) -> anyhow::Result<()> {
        let app = Router::new()
            .route("/channels/webhook/{channel_id}", post(handle_webhook))
            .route("/healthz", get(healthz))
            .with_state(self.state.clone());
        let addr = SocketAddr::from(([127, 0, 0, 1], self.port));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        *self.shutdown.write().await = Some(tx);
        tracing::info!(%addr, "channel webhook server listening");
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await?;
        Ok(())
    }

    pub async fn stop(&self) {
        if let Some(tx) = self.shutdown.write().await.take() {
            let _ = tx.send(());
        }
    }
}

async fn healthz() -> StatusCode {
    StatusCode::OK
}

async fn handle_webhook(
    State(state): State<WebhookChannelState>,
    Path(channel_id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let Some(config) = state.channels.get(&channel_id) else {
        return StatusCode::NOT_FOUND;
    };
    if !config.enabled {
        return StatusCode::FORBIDDEN;
    }
    if let Some(token) = config.token.as_deref().filter(|value| !value.is_empty()) {
        let authorized = headers
            .get("x-channel-token")
            .and_then(|value| value.to_str().ok())
            .map(|value| value == token)
            .unwrap_or(false)
            || headers
                .get("authorization")
                .and_then(|value| value.to_str().ok())
                .map(|value| value == format!("Bearer {token}"))
                .unwrap_or(false);
        if !authorized {
            return StatusCode::UNAUTHORIZED;
        }
    }
    let payload: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let Some(message) = parse_inbound_message(&channel_id, &config.kind, &payload) else {
        return StatusCode::ACCEPTED;
    };
    (state.handler)(message);
    StatusCode::OK
}

fn parse_inbound_message(
    channel_id: &str,
    kind: &str,
    payload: &Value,
) -> Option<InboundChannelMessage> {
    match kind.to_ascii_lowercase().as_str() {
        "telegram" => parse_telegram_payload(channel_id, payload),
        "discord" => parse_discord_payload(channel_id, payload),
        "feishu" | "lark" => parse_feishu_payload(channel_id, payload),
        _ => parse_generic_payload(channel_id, payload),
    }
}

fn parse_telegram_payload(channel_id: &str, payload: &Value) -> Option<InboundChannelMessage> {
    let message = payload.get("message").or_else(|| payload.get("edited_message"))?;
    let text = message
        .get("text")
        .or_else(|| message.get("caption"))
        .and_then(Value::as_str)?
        .trim()
        .to_string();
    if text.is_empty() {
        return None;
    }
    let external_user_id = message
        .get("from")
        .and_then(|from| from.get("id"))
        .map(|id| id.to_string())
        .unwrap_or_else(|| "telegram-user".to_string());
    Some(InboundChannelMessage {
        channel_id: channel_id.to_string(),
        external_user_id,
        text,
        session_id: None,
        is_mention: Some(true),
    })
}

fn parse_discord_payload(channel_id: &str, payload: &Value) -> Option<InboundChannelMessage> {
    if payload.get("type").and_then(Value::as_u64) == Some(1) {
        return None;
    }
    let text = payload
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let external_user_id = payload
        .get("author")
        .and_then(|author| author.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("discord-user")
        .to_string();
    Some(InboundChannelMessage {
        channel_id: channel_id.to_string(),
        external_user_id,
        text,
        session_id: payload
            .get("sessionId")
            .or_else(|| payload.get("session_id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        is_mention: Some(true),
    })
}

fn parse_feishu_payload(channel_id: &str, payload: &Value) -> Option<InboundChannelMessage> {
    if payload
        .pointer("/header/event_type")
        .and_then(Value::as_str)
        .is_some_and(|value| value.contains("url_verification"))
    {
        return None;
    }
    let text = payload
        .pointer("/event/message/content")
        .and_then(Value::as_str)
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|content| content.get("text").and_then(Value::as_str).map(str::to_string))
        .or_else(|| {
            payload
                .pointer("/event/message/content")
                .and_then(Value::as_str)
                .map(str::to_string)
        })?
        .trim()
        .to_string();
    if text.is_empty() {
        return None;
    }
    let external_user_id = payload
        .pointer("/event/sender/sender_id/open_id")
        .or_else(|| payload.pointer("/event/sender/sender_id/user_id"))
        .map(|id| id.to_string())
        .unwrap_or_else(|| "feishu-user".to_string());
    Some(InboundChannelMessage {
        channel_id: channel_id.to_string(),
        external_user_id,
        text,
        session_id: None,
        is_mention: Some(true),
    })
}

fn parse_generic_payload(channel_id: &str, payload: &Value) -> Option<InboundChannelMessage> {
    let text = payload
        .get("text")
        .or_else(|| payload.get("message"))
        .or_else(|| payload.get("content"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?
        .to_string();
    let external_user_id = payload
        .get("userId")
        .or_else(|| payload.get("user_id"))
        .or_else(|| payload.get("externalUserId"))
        .map(|id| id.to_string())
        .unwrap_or_else(|| "channel-user".to_string());
    let is_mention = payload
        .get("isMention")
        .or_else(|| payload.get("is_mention"))
        .and_then(Value::as_bool);
    Some(InboundChannelMessage {
        channel_id: channel_id.to_string(),
        external_user_id,
        text,
        session_id: payload
            .get("sessionId")
            .or_else(|| payload.get("session_id"))
            .and_then(Value::as_str)
            .map(str::to_string),
        is_mention,
    })
}

pub async fn send_outbound_webhook(
    config: &ChannelConfig,
    session_id: &str,
    text: &str,
) -> anyhow::Result<()> {
    let Some(url) = config
        .webhook_url
        .as_deref()
        .filter(|value| !value.is_empty())
    else {
        tracing::debug!(channel = %config.id, "no outbound webhook url configured");
        return Ok(());
    };
    let client = reqwest::Client::new();
    let body = match config.kind.to_ascii_lowercase().as_str() {
        "discord" => serde_json::json!({ "content": text, "sessionId": session_id }),
        "telegram" => serde_json::json!({ "text": text, "sessionId": session_id }),
        "feishu" | "lark" => serde_json::json!({
            "msg_type": "text",
            "content": { "text": text },
            "sessionId": session_id
        }),
        _ => serde_json::json!({ "text": text, "sessionId": session_id }),
    };
    let mut request = client.post(url).json(&body);
    if let Some(token) = config.token.as_deref().filter(|value| !value.is_empty()) {
        request = request.header("Authorization", format!("Bearer {token}"));
    }
    let response = request.send().await?;
    if !response.status().is_success() {
        anyhow::bail!("outbound webhook failed: {}", response.status());
    }
    Ok(())
}
