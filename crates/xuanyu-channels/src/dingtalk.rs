//! 钉钉机器人：HTTP 回调验签，sessionWebhook 出站。

use crate::{ChannelConfig, InboundChannelMessage, InboundHandler};
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Router,
};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use hmac::{Hmac, Mac};
use serde::Deserialize;
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

pub struct DingtalkWebhookAdapter {
    config: ChannelConfig,
    running: Arc<AtomicBool>,
    session_webhooks: Arc<Mutex<HashMap<String, String>>>,
    shutdown: Arc<RwLock<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl DingtalkWebhookAdapter {
    pub fn new(config: ChannelConfig) -> Self {
        Self {
            config,
            running: Arc::new(AtomicBool::new(false)),
            session_webhooks: Arc::new(Mutex::new(HashMap::new())),
            shutdown: Arc::new(RwLock::new(None)),
        }
    }

    fn secret(&self) -> anyhow::Result<String> {
        self.config
            .token
            .clone()
            .or_else(|| {
                self.config
                    .extra
                    .as_ref()
                    .and_then(|e| e.get("secret").or_else(|| e.get("appSecret")).cloned())
            })
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("DingTalk adapter requires token (app secret) for signature verification")
            })
    }

    fn listen_addr(&self) -> (String, u16, String) {
        let extra = self.config.extra.as_ref();
        let port = extra
            .and_then(|e| e.get("webhookPort").or_else(|| e.get("webhook_port")))
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(18912);
        let host = extra
            .and_then(|e| e.get("bindHost").or_else(|| e.get("bind_host")))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let path = normalize_path(
            extra
                .and_then(|e| e.get("webhookPath").or_else(|| e.get("webhook_path")))
                .map(String::as_str)
                .unwrap_or("/dingtalk/webhook"),
        );
        (host, port, path)
    }
}

#[async_trait::async_trait]
impl super::ChannelAdapter for DingtalkWebhookAdapter {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn kind(&self) -> &str {
        &self.config.kind
    }

    async fn start(&self, handler: InboundHandler) -> anyhow::Result<()> {
        let secret = self.secret()?;
        let (host, port, path) = self.listen_addr();
        self.running.store(true, Ordering::SeqCst);
        let state = DingtalkState {
            channel_id: self.config.id.clone(),
            secret,
            handler,
            session_webhooks: self.session_webhooks.clone(),
        };
        let app = Router::new()
            .route(&path, post(handle_dingtalk))
            .with_state(state);
        let addr: SocketAddr = format!("{host}:{port}")
            .parse()
            .map_err(|err| anyhow::anyhow!("invalid DingTalk bind address: {err}"))?;
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        *self.shutdown.write().await = Some(tx);
        let running = self.running.clone();
        tokio::spawn(async move {
            let serve = axum::serve(listener, app).with_graceful_shutdown(async {
                let _ = rx.await;
            });
            if let Err(err) = serve.await {
                tracing::warn!("dingtalk webhook server stopped: {err:#}");
            }
            running.store(false, Ordering::SeqCst);
        });
        tracing::info!(channel = %self.config.id, %addr, %path, "dingtalk webhook listening");
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
        let webhook = self
            .session_webhooks
            .lock()
            .map_err(|_| anyhow::anyhow!("dingtalk webhook map poisoned"))?
            .get(session_id)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No sessionWebhook for DingTalk conversation {session_id}; wait for an inbound message first"
                )
            })?;
        let client = reqwest::Client::new();
        for chunk in split_text(text, 1800) {
            let response = client
                .post(&webhook)
                .json(&json!({
                    "msgtype": "text",
                    "text": { "content": chunk },
                }))
                .send()
                .await?;
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                anyhow::bail!(
                    "DingTalk reply failed HTTP {status}: {}",
                    body.chars().take(200).collect::<String>()
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct DingtalkState {
    channel_id: String,
    secret: String,
    handler: InboundHandler,
    session_webhooks: Arc<Mutex<HashMap<String, String>>>,
}

#[derive(Debug, Deserialize)]
struct SignQuery {
    timestamp: Option<String>,
    sign: Option<String>,
}

async fn handle_dingtalk(
    State(state): State<DingtalkState>,
    Query(query): Query<SignQuery>,
    headers: HeaderMap,
    body: Bytes,
) -> (StatusCode, axum::Json<Value>) {
    let timestamp = headers
        .get("timestamp")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or(query.timestamp)
        .unwrap_or_default();
    let sign = headers
        .get("sign")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or(query.sign)
        .unwrap_or_default();
    if !verify_dingtalk_sign(&timestamp, &sign, &state.secret) {
        return (StatusCode::FORBIDDEN, axum::Json(json!({ "error": "invalid signature" })));
    }
    let payload: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let text = extract_text(&payload);
    let sender = payload
        .get("senderStaffId")
        .or_else(|| payload.get("senderId"))
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    let conversation_id = payload
        .get("conversationId")
        .or_else(|| payload.get("chatbotCorpId"))
        .and_then(Value::as_str)
        .unwrap_or(sender.as_str())
        .to_string();
    if let Some(webhook) = payload.get("sessionWebhook").and_then(Value::as_str) {
        if is_allowed_dingtalk_webhook(webhook) {
            if let Ok(mut map) = state.session_webhooks.lock() {
                map.insert(conversation_id.clone(), webhook.to_string());
            }
        } else {
            tracing::warn!(webhook = %webhook.chars().take(120).collect::<String>(), "dingtalk sessionWebhook rejected");
        }
    }
    if !text.is_empty() {
        let conversation_type = payload
            .get("conversationType")
            .and_then(Value::as_str)
            .unwrap_or("");
        let is_at = payload
            .get("isInAtList")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        (state.handler)(InboundChannelMessage {
            channel_id: state.channel_id.clone(),
            external_user_id: conversation_id,
            text,
            session_id: None,
            is_mention: Some(conversation_type == "1" || is_at),
        });
    }
    (StatusCode::OK, axum::Json(json!({ "msgtype": "empty" })))
}

fn verify_dingtalk_sign(timestamp: &str, sign: &str, secret: &str) -> bool {
    let timestamp = timestamp.trim();
    let sign = sign.trim();
    if timestamp.is_empty() || sign.is_empty() {
        return false;
    }
    let Ok(ts) = timestamp.parse::<i64>() else {
        return false;
    };
    let now = chrono::Utc::now().timestamp_millis();
    if (now - ts).abs() > 5 * 60_000 {
        return false;
    }
    let string_to_sign = format!("{timestamp}\n{secret}");
    let Ok(mut mac) = HmacSha256::new_from_slice(secret.as_bytes()) else {
        return false;
    };
    mac.update(string_to_sign.as_bytes());
    let expected = B64.encode(mac.finalize().into_bytes());
    expected.as_bytes() == sign.as_bytes()
}

fn is_allowed_dingtalk_webhook(webhook: &str) -> bool {
    let Ok(parsed) = url::Url::parse(webhook) else {
        return false;
    };
    if parsed.scheme() != "https" {
        return false;
    }
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    host == "oapi.dingtalk.com"
        || host.ends_with(".oapi.dingtalk.com")
        || host == "api.dingtalk.com"
        || host.ends_with(".dingtalk.com")
}

fn extract_text(payload: &Value) -> String {
    if let Some(text) = payload
        .pointer("/text/content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return text.to_string();
    }
    payload
        .get("content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("")
        .to_string()
}

fn normalize_path(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "/dingtalk/webhook".to_string();
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
