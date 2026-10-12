//! QQ Bot API v2：access token + WebSocket 收 C2C/群消息，REST 被动回复。

use crate::{ChannelConfig, InboundChannelMessage, InboundHandler};
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use tokio::sync::RwLock;
use tokio::time::{interval, MissedTickBehavior};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const TOKEN_URL: &str = "https://bots.qq.com/app/getAppAccessToken";
const INTENT_GROUP_AND_C2C: u64 = 1 << 25;
const MAX_TRACKED: usize = 5000;

pub struct QqGatewayAdapter {
    config: ChannelConfig,
    running: Arc<AtomicBool>,
    kinds: Arc<Mutex<HashMap<String, String>>>,
    last_msg_id: Arc<Mutex<HashMap<String, String>>>,
    msg_seq: Arc<Mutex<HashMap<String, u64>>>,
    access_token: Arc<RwLock<Option<(String, i64)>>>,
}

impl QqGatewayAdapter {
    pub fn new(config: ChannelConfig) -> Self {
        Self {
            config,
            running: Arc::new(AtomicBool::new(false)),
            kinds: Arc::new(Mutex::new(HashMap::new())),
            last_msg_id: Arc::new(Mutex::new(HashMap::new())),
            msg_seq: Arc::new(Mutex::new(HashMap::new())),
            access_token: Arc::new(RwLock::new(None)),
        }
    }

    fn credentials(&self) -> anyhow::Result<(String, String, bool)> {
        let extra = self.config.extra.as_ref();
        let app_id = self
            .config
            .app_id
            .clone()
            .or_else(|| extra.and_then(|e| e.get("appId").or_else(|| e.get("appid")).cloned()))
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("QQ adapter requires appId"))?;
        let secret = self
            .config
            .token
            .clone()
            .or_else(|| {
                extra.and_then(|e| {
                    e.get("clientSecret")
                        .or_else(|| e.get("appSecret"))
                        .cloned()
                })
            })
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("QQ adapter requires token (AppSecret)"))?;
        let sandbox = extra
            .and_then(|e| e.get("sandbox"))
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
        Ok((app_id, secret, sandbox))
    }

    fn api_base(sandbox: bool) -> &'static str {
        if sandbox {
            "https://sandbox.api.sgroup.qq.com"
        } else {
            "https://api.sgroup.qq.com"
        }
    }
}

#[async_trait::async_trait]
impl super::ChannelAdapter for QqGatewayAdapter {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn kind(&self) -> &str {
        &self.config.kind
    }

    async fn start(&self, handler: InboundHandler) -> anyhow::Result<()> {
        let (app_id, secret, sandbox) = self.credentials()?;
        let _ = ensure_access_token(&self.access_token, &app_id, &secret).await?;
        self.running.store(true, Ordering::SeqCst);
        let running = self.running.clone();
        let channel_id = self.config.id.clone();
        let kinds = self.kinds.clone();
        let last_msg_id = self.last_msg_id.clone();
        let access_token = self.access_token.clone();
        tokio::spawn(async move {
            let mut attempt = 0u32;
            while running.load(Ordering::SeqCst) {
                match run_qq_gateway_once(
                    &channel_id,
                    &app_id,
                    &secret,
                    sandbox,
                    access_token.clone(),
                    kinds.clone(),
                    last_msg_id.clone(),
                    handler.clone(),
                    &running,
                )
                .await
                {
                    Ok(()) => {
                        if !running.load(Ordering::SeqCst) {
                            break;
                        }
                        attempt = attempt.saturating_add(1);
                        tracing::warn!(
                            channel = %channel_id,
                            attempt,
                            "qq gateway disconnected; reconnecting"
                        );
                    }
                    Err(err) => {
                        attempt = attempt.saturating_add(1);
                        tracing::warn!(channel = %channel_id, attempt, "qq gateway error: {err:#}");
                    }
                }
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(crate::gateway_reconnect_delay(attempt)).await;
            }
        });
        tracing::info!(channel = %self.config.id, "qq gateway started");
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }

    async fn send_text(&self, session_id: &str, text: &str) -> anyhow::Result<()> {
        let (app_id, secret, sandbox) = self.credentials()?;
        let token = ensure_access_token(&self.access_token, &app_id, &secret).await?;
        let kind = self
            .kinds
            .lock()
            .ok()
            .and_then(|map| map.get(session_id).cloned())
            .unwrap_or_else(|| "c2c".to_string());
        let reply_to = self
            .last_msg_id
            .lock()
            .ok()
            .and_then(|map| map.get(session_id).cloned());
        let msg_seq = {
            let mut map = self
                .msg_seq
                .lock()
                .map_err(|_| anyhow::anyhow!("qq msg_seq lock poisoned"))?;
            let key = reply_to.clone().unwrap_or_else(|| session_id.to_string());
            let next = map.get(&key).copied().unwrap_or(0) + 1;
            if !map.contains_key(&key) && map.len() >= MAX_TRACKED {
                if let Some(oldest) = map.keys().next().cloned() {
                    map.remove(&oldest);
                }
            }
            map.insert(key, next);
            next
        };
        let mut body = json!({
            "content": text,
            "msg_type": 0,
            "msg_seq": msg_seq,
        });
        if let Some(msg_id) = reply_to {
            body["msg_id"] = json!(msg_id);
        }
        let path = if kind == "group" {
            format!("/v2/groups/{session_id}/messages")
        } else {
            format!("/v2/users/{session_id}/messages")
        };
        let url = format!("{}{path}", Self::api_base(sandbox));
        let client = reqwest::Client::new();
        let response = client
            .post(&url)
            .header("Authorization", format!("QQBot {token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("QQ send failed HTTP {status}: {}", body.chars().take(200).collect::<String>());
        }
        Ok(())
    }
}

async fn ensure_access_token(
    store: &Arc<RwLock<Option<(String, i64)>>>,
    app_id: &str,
    secret: &str,
) -> anyhow::Result<String> {
    let now = chrono::Utc::now().timestamp_millis();
    if let Some((token, expires_at)) = store.read().await.clone() {
        if now < expires_at - 60_000 {
            return Ok(token);
        }
    }
    let client = reqwest::Client::new();
    let response = client
        .post(TOKEN_URL)
        .json(&json!({ "appId": app_id, "clientSecret": secret }))
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!(
            "QQ getAppAccessToken failed HTTP {status}: {}",
            body.chars().take(200).collect::<String>()
        );
    }
    let data: Value = response.json().await?;
    let token = data
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("QQ getAppAccessToken missing access_token"))?
        .to_string();
    let expires_in = data
        .get("expires_in")
        .and_then(Value::as_i64)
        .unwrap_or(7200);
    *store.write().await = Some((token.clone(), now + expires_in * 1000));
    Ok(token)
}

#[allow(clippy::too_many_arguments)]
async fn run_qq_gateway_once(
    channel_id: &str,
    app_id: &str,
    secret: &str,
    sandbox: bool,
    access_token: Arc<RwLock<Option<(String, i64)>>>,
    kinds: Arc<Mutex<HashMap<String, String>>>,
    last_msg_id: Arc<Mutex<HashMap<String, String>>>,
    handler: InboundHandler,
    running: &AtomicBool,
) -> anyhow::Result<()> {
    let token = ensure_access_token(&access_token, app_id, secret).await?;
    let client = reqwest::Client::new();
    let gateway_url = {
        let response = client
            .get(format!("{}/gateway", QqGatewayAdapter::api_base(sandbox)))
            .header("Authorization", format!("QQBot {token}"))
            .send()
            .await?
            .error_for_status()?;
        let body: Value = response.json().await?;
        body.get("url")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| anyhow::anyhow!("QQ gateway URL missing"))?
            .to_string()
    };

    let (mut ws, _) = connect_async(&gateway_url).await?;
    let mut heartbeat = interval(std::time::Duration::from_secs(45));
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut heartbeat_enabled = false;
    let mut last_seq: Option<i64> = None;

    while running.load(Ordering::SeqCst) {
        tokio::select! {
            _ = heartbeat.tick(), if heartbeat_enabled => {
                let payload = json!({ "op": 1, "d": last_seq });
                ws.send(Message::Text(payload.to_string().into())).await?;
            }
            incoming = ws.next() => {
                let Some(incoming) = incoming else { break; };
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
                let payload: Value = serde_json::from_str(&text)?;
                if let Some(seq) = payload.get("s").and_then(Value::as_i64) {
                    last_seq = Some(seq);
                }
                let op = payload.get("op").and_then(Value::as_u64).unwrap_or(999);
                match op {
                    10 => {
                        let heartbeat_ms = payload
                            .pointer("/d/heartbeat_interval")
                            .and_then(Value::as_u64)
                            .unwrap_or(45_000)
                            .max(5_000);
                        heartbeat = interval(std::time::Duration::from_millis(
                            (heartbeat_ms as f64 * 0.8) as u64,
                        ));
                        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
                        heartbeat_enabled = true;
                        let identify = json!({
                            "op": 2,
                            "d": {
                                "token": format!("QQBot {token}"),
                                "intents": INTENT_GROUP_AND_C2C,
                                "shard": [0, 1],
                            }
                        });
                        ws.send(Message::Text(identify.to_string().into())).await?;
                    }
                    0 => {
                        let event_type = payload
                            .get("t")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        if let Some(message) = parse_qq_event(
                            channel_id,
                            event_type,
                            payload.get("d"),
                            &kinds,
                            &last_msg_id,
                        ) {
                            handler(message);
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

fn parse_qq_event(
    channel_id: &str,
    event_type: &str,
    payload: Option<&Value>,
    kinds: &Arc<Mutex<HashMap<String, String>>>,
    last_msg_id: &Arc<Mutex<HashMap<String, String>>>,
) -> Option<InboundChannelMessage> {
    let d = payload?;
    let remember = |chat_id: &str, msg_id: &str, kind: &str| {
        if let Ok(mut map) = kinds.lock() {
            if !map.contains_key(chat_id) && map.len() >= MAX_TRACKED {
                if let Some(oldest) = map.keys().next().cloned() {
                    map.remove(&oldest);
                }
            }
            map.insert(chat_id.to_string(), kind.to_string());
        }
        if let Ok(mut map) = last_msg_id.lock() {
            if !map.contains_key(chat_id) && map.len() >= MAX_TRACKED {
                if let Some(oldest) = map.keys().next().cloned() {
                    map.remove(&oldest);
                }
            }
            map.insert(chat_id.to_string(), msg_id.to_string());
        }
    };

    if event_type == "C2C_MESSAGE_CREATE" {
        let msg_id = d.get("id").and_then(Value::as_str)?.to_string();
        let author = d.get("author")?;
        let user_id = author
            .get("user_openid")
            .or_else(|| author.get("id"))
            .and_then(Value::as_str)?
            .to_string();
        let text = strip_at(d.get("content").and_then(Value::as_str).unwrap_or(""));
        if text.is_empty() {
            return None;
        }
        remember(&user_id, &msg_id, "c2c");
        return Some(InboundChannelMessage {
            channel_id: channel_id.to_string(),
            external_user_id: user_id,
            text,
            session_id: None,
            is_mention: Some(true),
        });
    }

    if event_type == "GROUP_AT_MESSAGE_CREATE" || event_type == "GROUP_MESSAGE_CREATE" {
        let msg_id = d.get("id").and_then(Value::as_str)?.to_string();
        let group_id = d.get("group_openid").and_then(Value::as_str)?.to_string();
        let text = strip_at(d.get("content").and_then(Value::as_str).unwrap_or(""));
        if text.is_empty() {
            return None;
        }
        remember(&group_id, &msg_id, "group");
        return Some(InboundChannelMessage {
            channel_id: channel_id.to_string(),
            external_user_id: group_id,
            text,
            session_id: None,
            is_mention: Some(true),
        });
    }
    None
}

fn strip_at(text: &str) -> String {
    let mut out = text.to_string();
    while let Some(start) = out.find("<@") {
        if let Some(end) = out[start..].find('>') {
            out.replace_range(start..start + end + 1, "");
        } else {
            break;
        }
    }
    out.split_whitespace()
        .filter(|part| !part.starts_with('@'))
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}
