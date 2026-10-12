//! WhatsApp Cloud API：webhook 验签 + Graph 发信。

use crate::{ChannelConfig, InboundChannelMessage, InboundHandler};
use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get},
    Router,
};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::Sha256;
use std::net::SocketAddr;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::RwLock;

type HmacSha256 = Hmac<Sha256>;
const GRAPH: &str = "https://graph.facebook.com/v19.0";

pub struct WhatsappWebhookAdapter {
    config: ChannelConfig,
    running: Arc<AtomicBool>,
    shutdown: Arc<RwLock<Option<tokio::sync::oneshot::Sender<()>>>>,
}

impl WhatsappWebhookAdapter {
    pub fn new(config: ChannelConfig) -> Self {
        Self {
            config,
            running: Arc::new(AtomicBool::new(false)),
            shutdown: Arc::new(RwLock::new(None)),
        }
    }

    fn credentials(&self) -> anyhow::Result<(String, String, String, String)> {
        let extra = self.config.extra.as_ref();
        let token = self
            .config
            .token
            .clone()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("WhatsApp adapter requires token"))?;
        let phone_number_id = extra
            .and_then(|e| e.get("phoneNumberId").or_else(|| e.get("phone_number_id")))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("WhatsApp adapter requires extra.phoneNumberId"))?;
        let verify_token = extra
            .and_then(|e| e.get("verifyToken").or_else(|| e.get("verify_token")))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("WhatsApp adapter requires extra.verifyToken for webhook subscription")
            })?;
        let app_secret = extra
            .and_then(|e| e.get("appSecret").or_else(|| e.get("app_secret")))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                anyhow::anyhow!("WhatsApp adapter requires extra.appSecret for X-Hub-Signature-256")
            })?;
        Ok((token, phone_number_id, verify_token, app_secret))
    }

    fn listen_addr(&self) -> (String, u16, String) {
        let extra = self.config.extra.as_ref();
        let port = extra
            .and_then(|e| e.get("webhookPort").or_else(|| e.get("webhook_port")))
            .and_then(|v| v.parse::<u16>().ok())
            .unwrap_or(18913);
        let host = extra
            .and_then(|e| e.get("bindHost").or_else(|| e.get("bind_host")))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "127.0.0.1".to_string());
        let path = normalize_path(
            extra
                .and_then(|e| e.get("webhookPath").or_else(|| e.get("webhook_path")))
                .map(String::as_str)
                .unwrap_or("/whatsapp/webhook"),
        );
        (host, port, path)
    }
}

#[async_trait::async_trait]
impl super::ChannelAdapter for WhatsappWebhookAdapter {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn kind(&self) -> &str {
        &self.config.kind
    }

    async fn start(&self, handler: InboundHandler) -> anyhow::Result<()> {
        let (token, phone_number_id, verify_token, app_secret) = self.credentials()?;
        let (host, port, path) = self.listen_addr();
        self.running.store(true, Ordering::SeqCst);
        let state = WhatsappState {
            channel_id: self.config.id.clone(),
            verify_token,
            app_secret,
            handler,
        };
        let _ = (token, phone_number_id);
        let app = Router::new()
            .route(&path, get(handle_whatsapp_verify).post(handle_whatsapp_event))
            .with_state(state);
        let addr: SocketAddr = format!("{host}:{port}")
            .parse()
            .map_err(|err| anyhow::anyhow!("invalid WhatsApp bind address: {err}"))?;
        let listener = tokio::net::TcpListener::bind(addr).await?;
        let (tx, rx) = tokio::sync::oneshot::channel();
        *self.shutdown.write().await = Some(tx);
        let running = self.running.clone();
        tokio::spawn(async move {
            let serve = axum::serve(listener, app).with_graceful_shutdown(async {
                let _ = rx.await;
            });
            if let Err(err) = serve.await {
                tracing::warn!("whatsapp webhook server stopped: {err:#}");
            }
            running.store(false, Ordering::SeqCst);
        });
        tracing::info!(channel = %self.config.id, %addr, %path, "whatsapp webhook listening");
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
        let (token, phone_number_id, _, _) = self.credentials()?;
        let client = reqwest::Client::new();
        let url = format!("{GRAPH}/{phone_number_id}/messages");
        for chunk in split_text(text, 3500) {
            let response = client
                .post(&url)
                .bearer_auth(&token)
                .json(&json!({
                    "messaging_product": "whatsapp",
                    "to": session_id,
                    "type": "text",
                    "text": { "body": chunk },
                }))
                .send()
                .await?;
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                anyhow::bail!(
                    "WhatsApp send failed HTTP {status}: {}",
                    body.chars().take(200).collect::<String>()
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
struct WhatsappState {
    channel_id: String,
    verify_token: String,
    app_secret: String,
    handler: InboundHandler,
}

#[derive(Debug, Deserialize)]
struct VerifyQuery {
    #[serde(rename = "hub.mode")]
    hub_mode: Option<String>,
    #[serde(rename = "hub.verify_token")]
    hub_verify_token: Option<String>,
    #[serde(rename = "hub.challenge")]
    hub_challenge: Option<String>,
}

async fn handle_whatsapp_verify(
    State(state): State<WhatsappState>,
    Query(query): Query<VerifyQuery>,
) -> impl IntoResponse {
    if query.hub_mode.as_deref() == Some("subscribe")
        && query.hub_verify_token.as_deref() == Some(state.verify_token.as_str())
    {
        if let Some(challenge) = query.hub_challenge {
            return (StatusCode::OK, challenge).into_response();
        }
    }
    (StatusCode::FORBIDDEN, "forbidden").into_response()
}

async fn handle_whatsapp_event(
    State(state): State<WhatsappState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    if !verify_whatsapp_signature(&headers, &body, &state.app_secret) {
        return StatusCode::UNAUTHORIZED;
    }
    let payload: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let entries = payload
        .get("entry")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for entry in entries {
        let changes = entry
            .get("changes")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for change in changes {
            let value = change.get("value").cloned().unwrap_or(Value::Null);
            let messages = value
                .get("messages")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for message in messages {
                if message.get("type").and_then(Value::as_str) != Some("text") {
                    continue;
                }
                let text = message
                    .pointer("/text/body")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|v| !v.is_empty());
                let Some(text) = text else { continue };
                let from = message
                    .get("from")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if from.is_empty() {
                    continue;
                }
                (state.handler)(InboundChannelMessage {
                    channel_id: state.channel_id.clone(),
                    external_user_id: from,
                    text: text.to_string(),
                    session_id: None,
                    is_mention: Some(true),
                });
            }
        }
    }
    StatusCode::OK
}

fn verify_whatsapp_signature(headers: &HeaderMap, body: &[u8], app_secret: &str) -> bool {
    let header = headers
        .get("x-hub-signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .trim();
    let Some(hex_sig) = header.strip_prefix("sha256=") else {
        return false;
    };
    let Ok(mut mac) = HmacSha256::new_from_slice(app_secret.as_bytes()) else {
        return false;
    };
    mac.update(body);
    let expected = hex::encode(mac.finalize().into_bytes());
    expected.eq_ignore_ascii_case(hex_sig)
}

fn normalize_path(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return "/whatsapp/webhook".to_string();
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
