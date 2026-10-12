//! WeChat iLink Bot HTTP client (QR + getupdates + send).
//! WeChat iLink Bot long-poll client (QR bind + getupdates / sendmessage).

use anyhow::{anyhow, bail, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const DEFAULT_BASE_URL: &str = "https://ilinkai.weixin.qq.com";
const BOT_TYPE: &str = "3";
const CHANNEL_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WechatCredentials {
    pub token: String,
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_id: Option<String>,
    pub base_url: String,
    pub saved_at: String,
}

#[derive(Debug, Clone)]
pub struct WechatInboundMessage {
    pub id: String,
    pub from_user_id: String,
    pub text: String,
    pub context_token: String,
}

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 3);
    for b in input.as_bytes() {
        match *b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn random_wechat_uin() -> String {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| (d.as_nanos() as u32) ^ (d.as_secs() as u32))
        .unwrap_or(0);
    STANDARD.encode(n.to_string().as_bytes())
}

fn account_dir(data_dir: &Path, account_id: &str) -> PathBuf {
    let safe: String = account_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '@' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    data_dir.join("channels").join("wechat").join(safe)
}

pub fn save_wechat_credentials(data_dir: &Path, creds: &WechatCredentials) -> Result<()> {
    let dir = account_dir(data_dir, &creds.account_id);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join("credentials.json"),
        serde_json::to_string_pretty(creds)?,
    )?;
    Ok(())
}

pub fn load_updates_cursor(data_dir: &Path, account_id: &str) -> String {
    let path = account_dir(data_dir, account_id).join("updates-buf.txt");
    std::fs::read_to_string(path).unwrap_or_default()
}

pub fn save_updates_cursor(data_dir: &Path, account_id: &str, buf: &str) -> Result<()> {
    let dir = account_dir(data_dir, account_id);
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("updates-buf.txt"), buf)?;
    Ok(())
}

async fn get_json(
    client: &reqwest::Client,
    url: &str,
    headers: &[(&str, &str)],
) -> Result<Value> {
    let mut req = client.get(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let res = req.send().await.with_context(|| format!("GET {url}"))?;
    let status = res.status();
    if !status.is_success() {
        bail!("WeChat iLink GET {url} → HTTP {status}");
    }
    Ok(res.json().await?)
}

async fn post_json(
    client: &reqwest::Client,
    base_url: &str,
    api_path: &str,
    body: Value,
    token: &str,
) -> Result<Value> {
    let url = format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        api_path.trim_start_matches('/')
    );
    let res = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("AuthorizationType", "ilink_bot_token")
        .header("Authorization", format!("Bearer {token}"))
        .header("X-WECHAT-UIN", random_wechat_uin())
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST {url}"))?;
    let status = res.status();
    if !status.is_success() {
        let text = res.text().await.unwrap_or_default();
        bail!(
            "WeChat iLink POST {api_path} → HTTP {status}: {}",
            text.chars().take(200).collect::<String>()
        );
    }
    Ok(res.json().await?)
}

/// Fetch QR and poll until confirmed / expired / timeout / cancelled.
pub async fn login_wechat_via_qr(
    data_dir: &Path,
    base_url: Option<&str>,
    timeout: Duration,
    mut on_qr: impl FnMut(String),
    cancelled: tokio::sync::watch::Receiver<bool>,
) -> Result<WechatCredentials> {
    let base_url = base_url
        .unwrap_or(DEFAULT_BASE_URL)
        .trim_end_matches('/')
        .to_string();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()?;
    let started = Instant::now();

    let throw_if_cancelled = |cancelled: &tokio::sync::watch::Receiver<bool>| -> Result<()> {
        if *cancelled.borrow() {
            bail!("WeChat QR login aborted");
        }
        Ok(())
    };

    throw_if_cancelled(&cancelled)?;
    let qr_fetch_url = format!("{base_url}/ilink/bot/get_bot_qrcode?bot_type={BOT_TYPE}");
    let qr = get_json(&client, &qr_fetch_url, &[]).await?;
    throw_if_cancelled(&cancelled)?;
    let qrcode = qr
        .get("qrcode")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let qr_img = qr
        .get("qrcode_img_content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if qrcode.is_empty() {
        bail!("WeChat iLink did not return a QR code");
    }
    on_qr(if qr_img.is_empty() {
        qrcode.clone()
    } else {
        qr_img
    });

    loop {
        throw_if_cancelled(&cancelled)?;
        if started.elapsed() >= timeout {
            bail!("WeChat QR login timed out");
        }
        let status_url = format!(
            "{base_url}/ilink/bot/get_qrcode_status?qrcode={}",
            percent_encode(&qrcode)
        );
        let status = match get_json(
            &client,
            &status_url,
            &[("iLink-App-ClientVersion", "1")],
        )
        .await
        {
            Ok(value) => value,
            Err(_) => {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                continue;
            }
        };
        throw_if_cancelled(&cancelled)?;
        let state = status
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if state == "confirmed" {
            let token = status
                .get("bot_token")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let account_id = status
                .get("ilink_bot_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if token.is_empty() || account_id.is_empty() {
                bail!("WeChat QR confirmed but credentials missing");
            }
            let creds = WechatCredentials {
                token,
                account_id,
                user_id: status
                    .get("ilink_user_id")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                base_url: status
                    .get("baseurl")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&base_url)
                    .trim_end_matches('/')
                    .to_string(),
                saved_at: Utc::now().to_rfc3339(),
            };
            save_wechat_credentials(data_dir, &creds)?;
            return Ok(creds);
        }
        if state == "expired" {
            return Err(anyhow!("WeChat QR code expired; reconnect to get a new one"));
        }
        tokio::time::sleep(Duration::from_millis(1500)).await;
    }
}

pub async fn wechat_get_updates(
    client: &reqwest::Client,
    creds: &WechatCredentials,
    updates_buf: &str,
) -> Result<(Vec<Value>, String)> {
    let data = post_json(
        client,
        &creds.base_url,
        "ilink/bot/getupdates",
        json!({
            "get_updates_buf": updates_buf,
            "base_info": { "channel_version": CHANNEL_VERSION },
        }),
        &creds.token,
    )
    .await?;
    if let Some(ret) = data.get("ret").and_then(|v| v.as_i64()) {
        if ret != 0 {
            bail!("WeChat getupdates ret={ret}");
        }
    }
    let msgs = data
        .get("msgs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let next_buf = data
        .get("get_updates_buf")
        .and_then(|v| v.as_str())
        .unwrap_or(updates_buf)
        .to_string();
    Ok((msgs, next_buf))
}

pub async fn wechat_send_text(
    client: &reqwest::Client,
    creds: &WechatCredentials,
    to_user_id: &str,
    text: &str,
    context_token: &str,
) -> Result<()> {
    if context_token.trim().is_empty() {
        bail!("WeChat reply requires context_token from the inbound message");
    }
    let client_id = format!(
        "xy-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0)
    );
    for chunk in split_text(text, 1800) {
        let data = post_json(
            client,
            &creds.base_url,
            "ilink/bot/sendmessage",
            json!({
                "msg": {
                    "to_user_id": to_user_id,
                    "from_user_id": "",
                    "message_type": 2,
                    "message_state": 2,
                    "context_token": context_token,
                    "client_id": client_id,
                    "item_list": [{ "type": 1, "text_item": { "text": chunk } }],
                },
                "base_info": { "channel_version": CHANNEL_VERSION },
            }),
            &creds.token,
        )
        .await?;
        if let Some(ret) = data.get("ret").and_then(|v| v.as_i64()) {
            if ret != 0 {
                bail!("WeChat sendmessage ret={ret}");
            }
        }
    }
    Ok(())
}

pub async fn wechat_get_typing_ticket(
    client: &reqwest::Client,
    creds: &WechatCredentials,
    user_id: &str,
    context_token: Option<&str>,
) -> Result<String> {
    let mut body = json!({
        "ilink_user_id": user_id,
        "base_info": { "channel_version": CHANNEL_VERSION },
    });
    if let Some(token) = context_token.filter(|value| !value.trim().is_empty()) {
        body["context_token"] = json!(token);
    }
    let data = post_json(client, &creds.base_url, "ilink/bot/getconfig", body, &creds.token).await?;
    if let Some(ret) = data.get("ret").and_then(|v| v.as_i64()) {
        if ret != 0 {
            bail!("WeChat getconfig ret={ret}");
        }
    }
    let ticket = data
        .get("typing_ticket")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("WeChat getconfig did not return typing_ticket"))?;
    Ok(ticket.to_string())
}

/// status 1 = typing / keepalive, status 2 = cancel.
pub async fn wechat_send_typing(
    client: &reqwest::Client,
    creds: &WechatCredentials,
    user_id: &str,
    typing_ticket: &str,
    status: u8,
) -> Result<()> {
    let data = post_json(
        client,
        &creds.base_url,
        "ilink/bot/sendtyping",
        json!({
            "ilink_user_id": user_id,
            "typing_ticket": typing_ticket,
            "status": status,
            "base_info": { "channel_version": CHANNEL_VERSION },
        }),
        &creds.token,
    )
    .await?;
    if let Some(ret) = data.get("ret").and_then(|v| v.as_i64()) {
        if ret != 0 {
            bail!("WeChat sendtyping ret={ret}");
        }
    }
    Ok(())
}

pub fn parse_wechat_inbound(msg: &Value) -> Option<WechatInboundMessage> {
    if !msg.is_object() {
        return None;
    }
    if let Some(message_type) = msg.get("message_type").and_then(|v| v.as_i64()) {
        if message_type != 1 {
            return None;
        }
    }
    let from_user_id = msg
        .get("from_user_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let context_token = msg
        .get("context_token")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if from_user_id.is_empty() || context_token.is_empty() {
        return None;
    }
    let mut texts = Vec::new();
    if let Some(items) = msg.get("item_list").and_then(|v| v.as_array()) {
        for item in items {
            if let Some(text) = item
                .pointer("/text_item/text")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                texts.push(text.to_string());
            }
        }
    }
    let text = texts.join("\n").trim().to_string();
    if text.is_empty() {
        return None;
    }
    let id = msg
        .get("msg_id")
        .or_else(|| msg.get("message_id"))
        .or_else(|| msg.get("client_id"))
        .and_then(|v| v.as_str())
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            let mut hasher = DefaultHasher::new();
            format!("{from_user_id}:{context_token}:{text}").hash(&mut hasher);
            format!("{:x}", hasher.finish())
        });
    Some(WechatInboundMessage {
        id,
        from_user_id,
        text,
        context_token,
    })
}

fn split_text(text: &str, max: usize) -> Vec<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    if trimmed.chars().count() <= max {
        return vec![trimmed.to_string()];
    }
    let chars: Vec<char> = trimmed.chars().collect();
    chars
        .chunks(max)
        .map(|chunk| chunk.iter().collect())
        .collect()
}
