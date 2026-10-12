//! LINE Messaging API：webhook 验签，replyToken 优先，过期后 push。

use crate::{ChannelConfig, InboundChannelMessage, InboundHandler};
use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Router,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tokio::sync::RwLock;

type HmacSha256 = Hmac<Sha256>;
const LINE_API: &str = "https://api.line.me";

pub struct LineWebhookAdapter {
    config: ChannelConfig,
    running: Arc<AtomicBool>,
    reply_tokens: Arc<Mutex<HashMap<String, String>>>,
    shutdown: Arc<RwLock<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl LineWebhookAdapter {
    pub fn new(config: ChannelConfig) -> Self {
        Self {
            config,
            running: Arc::new(AtomicBool::new(false)),
            reply_tokens: Arc::new(Mutex::new(HashMap::new())),
            shutdown: Arc::new(RwLock::new(None)),
        }
    }

    fn credentials(&self) -> anyhow::Result<(String, Option<String>)> {
        let token = self
            .config
            .token
            .clone()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("LINE adapter requires channel access token"))?;
        let channel_secret = self
            .config
            .extra
            .as_ref()
            .and_then(|e| e.get("channelSecret").or_else(|| e.get("channel_secret")))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty());
        Ok((token, channel_secret))
    }

    fn listen_addr(&self) -> (String, u16, String) {
        let extra = self.config.extra.as_ref();
        let port = extra
            .and_then(|e| e.get("webhookPort").or_else(|| e.get("webhook_port")))
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(18914);
        let host = extra
            .and_then(|e| e.get("bindHost").or_else(|| e.get("bind_host")))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "0.0.0.0".to_string());
        let path = normalize_path(
            extra
                .and_then(|e| e.get("webhookPath").or_else(|| e.get("webhook_path")))
                .map(String::as_str)
                .unwrap_or("/line/webhook"),
        );
        (host, port, path)
    }
}

#[async_trait::async_trait]
impl super::ChannelAdapter for LineWebhookAdapter {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn kind(&self) -> &str {
        &self.config.kind
    }

    async fn start(&self, handler: InboundHandler) -> anyhow::Result<()> {
        let (token, channel_secret) = self.credentials()?;
        let (host, port, path) = self.listen_addr();
        self.running.store(true, Ordering::SeqCst);
        let state = LineState {
            channel_id: self.config.id.clone(),
            channel_secret,
            handler,
            reply_tokens: self.reply_tokens.clone(),
        };
        let _ = token;
        let app = Router::new()
            .route(&path, post(handle_line))
            .with_state(state);
        let addr: SocketAddr = format!("{host}:{port}")
            .parse()
            .map_err(|err| anyhow::anyhow!("invalid LINE bind address: {err}"))?;
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        *self.shutdown.write().await = Some(tx);
        let running = self.running.clone();
        tokio::spawn(async move {
            let serve = axum::serve(listener, app).with_graceful_shutdown(async {
                let _ = rx.await;
            });
            if let Err(err) = serve.await {
                tracing::warn!("line webhook server stopped: {err:#}");
            }
            running.store(false, Ordering::SeqCst);
        });
        tracing::info!(channel = %self.config.id, %addr, %path, "line webhook listening");
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        self.running.store(false, Ordering::SeqCst);
        if let Some(tx) = self.shutdown.write().await.take() {
            let _ = tx.send(());
        }
        Ok(())
    }

    async fn send_text(&self, session_id: &str, text: &str) -> anyhow::Result<()> {
        let (token, _) = self.credentials()?;
        let chunks = split_text(text, 4500);
        let reply_token = {
            let mut map = self
                .reply_tokens
                .lock()
                .map_err(|_| anyhow::anyhow!("line reply_tokens poisoned"))?;
            map.remove(session_id)
        };
        let client = reqwest::Client::new();
        if let Some(reply_token) = reply_token {
            let messages: Vec<Value> = chunks
                .iter()
                .map(|chunk| json!({ "type": "text", "text": chunk }))
                .collect();
            let response = client
                .post(format!("{LINE_API}/v2/bot/message/reply"))
                .bearer_auth(&token)
                .json(&json!({ "replyToken": reply_token, "messages": messages }))
                .send()
                .await?;
            if response.status().is_success() {
                return Ok(());
            }
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            tracing::warn!(
                status = %status,
                body = %body.chars().take(120).collect::<String>(),
                "LINE reply failed; falling back to push"
            );
        }
        for chunk in chunks {
            let response = client
                .post(format!("{LINE_API}/v2/bot/message/push"))
                .bearer_auth(&token)
                .json(&json!({
                    "to": session_id,
                    "messages": [{ "type": "text", "text": chunk }],
                }))
                .send()
                .await?;
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                anyhow::bail!(
                    "LINE push failed HTTP {status}: {}",
                    body.chars().take(200).collect::<String>()
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct LineState {
    channel_id: String,
    channel_secret: Option<String>,
    handler: InboundHandler,
    reply_tokens: Arc<Mutex<HashMap<String, String>>>,
}

async fn handle_line(
    State(state): State<LineState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    if let Some(secret) = state.channel_secret.as_deref() {
        let signature = headers
            .get("x-line-signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        if !verify_line_signature(secret, &body, signature) {
            return StatusCode::FORBIDDEN;
        }
    }
    let payload: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let events = payload
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for event in events {
        if event.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let message = event.get("message").cloned().unwrap_or(Value::Null);
        if message.get("type").and_then(Value::as_str) != Some("text") {
            continue;
        }
        let text = message
            .get("text")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let Some(text) = text else { continue };
        let source = event.get("source").cloned().unwrap_or(Value::Null);
        let channel_target = source
            .get("userId")
            .or_else(|| source.get("groupId"))
            .or_else(|| source.get("roomId"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if channel_target.is_empty() {
            continue;
        }
        if let Some(reply_token) = event.get("replyToken").and_then(Value::as_str) {
            if let Ok(mut map) = state.reply_tokens.lock() {
                map.insert(channel_target.clone(), reply_token.to_string());
            }
        }
        let source_type = source.get("type").and_then(Value::as_str).unwrap_or("");
        let mentioned = message
            .get("mention")
            .map(|v| !v.is_null())
            .unwrap_or(false);
        (state.handler)(InboundChannelMessage {
            channel_id: state.channel_id.clone(),
            external_user_id: channel_target,
            text: text.to_string(),
            session_id: None,
            is_mention: Some(source_type == "user" || mentioned),
        });
    }
    StatusCode::OK
}

fn verify_line_signature(secret: &str, body: &[u8], signature: &str) -> bool {
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    let expected = B64.encode(mac.finalize().into_bytes());
    expected.as_bytes() == signature.as_bytes()
}

fn normalize_path(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "/line/webhook".to_string();
    }
    if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    }
}

fn split_text(text: &str, max_chars: usize) -> Vec<String> {
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
