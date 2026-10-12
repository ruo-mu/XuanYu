//! WeChat iLink long-poll adapter.

use crate::wechat_ilink::{
    load_updates_cursor, parse_wechat_inbound, save_updates_cursor, wechat_get_typing_ticket,
    wechat_get_updates, wechat_send_text, wechat_send_typing, WechatCredentials,
};
use crate::{ChannelConfig, InboundChannelMessage, InboundHandler};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tokio::sync::RwLock;

pub struct WechatLongPollAdapter {
    config: ChannelConfig,
    running: Arc<AtomicBool>,
    context_tokens: Arc<RwLock<HashMap<String, String>>>,
    typing_tickets: Arc<RwLock<HashMap<String, String>>>,
    creds: Arc<RwLock<Option<WechatCredentials>>>,
    data_dir: PathBuf,
}

impl WechatLongPollAdapter {
    pub fn new(config: ChannelConfig, data_dir: PathBuf) -> Self {
        Self {
            config,
            running: Arc::new(AtomicBool::new(false)),
            context_tokens: Arc::new(RwLock::new(HashMap::new())),
            typing_tickets: Arc::new(RwLock::new(HashMap::new())),
            creds: Arc::new(RwLock::new(None)),
            data_dir,
        }
    }

    fn credentials_from_config(&self) -> anyhow::Result<WechatCredentials> {
        let token = self
            .config
            .token
            .clone()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow::anyhow!("wechat token required"))?;
        let account_id = self
            .config
            .extra
            .as_ref()
            .and_then(|extra| {
                extra
                    .get("accountId")
                    .or_else(|| extra.get("account_id"))
                    .cloned()
            })
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                self.config
                    .id
                    .strip_prefix("wechat:")
                    .map(str::to_string)
            })
            .unwrap_or_else(|| self.config.id.clone());
        let base_url = self
            .config
            .extra
            .as_ref()
            .and_then(|extra| {
                extra
                    .get("baseUrl")
                    .or_else(|| extra.get("base_url"))
                    .cloned()
            })
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "https://ilinkai.weixin.qq.com".to_string());
        Ok(WechatCredentials {
            token,
            account_id,
            user_id: self.config.extra.as_ref().and_then(|extra| {
                extra
                    .get("userId")
                    .or_else(|| extra.get("user_id"))
                    .cloned()
            }),
            base_url: base_url.trim_end_matches('/').to_string(),
            saved_at: chrono::Utc::now().to_rfc3339(),
        })
    }

    async fn send_typing_status(&self, user_id: &str, status: u8) -> anyhow::Result<()> {
        let creds = self
            .creds
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("wechat adapter is not connected"))?;
        let context_token = self.context_tokens.read().await.get(user_id).cloned();
        let client = reqwest::Client::new();
        let mut ticket = self.typing_tickets.read().await.get(user_id).cloned();
        if ticket.is_none() {
            let fetched =
                wechat_get_typing_ticket(&client, &creds, user_id, context_token.as_deref())
                    .await?;
            self.typing_tickets
                .write()
                .await
                .insert(user_id.to_string(), fetched.clone());
            ticket = Some(fetched);
        }
        let Some(ticket) = ticket else {
            anyhow::bail!("missing typing ticket");
        };
        match wechat_send_typing(&client, &creds, user_id, &ticket, status).await {
            Ok(()) => Ok(()),
            Err(err) => {
                self.typing_tickets.write().await.remove(user_id);
                Err(err)
            }
        }
    }
}

#[async_trait::async_trait]
impl super::ChannelAdapter for WechatLongPollAdapter {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn kind(&self) -> &str {
        &self.config.kind
    }

    async fn start(&self, handler: InboundHandler) -> anyhow::Result<()> {
        let creds = self.credentials_from_config()?;
        *self.creds.write().await = Some(creds.clone());
        self.running.store(true, Ordering::SeqCst);
        let running = self.running.clone();
        let context_tokens = self.context_tokens.clone();
        let data_dir = self.data_dir.clone();
        let channel_id = self.config.id.clone();
        let account_id = creds.account_id.clone();
        tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(90))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new());
            let mut buf = load_updates_cursor(&data_dir, &account_id);
            while running.load(Ordering::SeqCst) {
                match wechat_get_updates(&client, &creds, &buf).await {
                    Ok((msgs, next_buf)) => {
                        if next_buf != buf {
                            buf = next_buf;
                            let _ = save_updates_cursor(&data_dir, &account_id, &buf);
                        }
                        for raw in msgs {
                            let Some(inbound) = parse_wechat_inbound(&raw) else {
                                continue;
                            };
                            {
                                let mut tokens = context_tokens.write().await;
                                tokens.insert(
                                    inbound.from_user_id.clone(),
                                    inbound.context_token.clone(),
                                );
                            }
                            handler(InboundChannelMessage {
                                channel_id: channel_id.clone(),
                                external_user_id: inbound.from_user_id,
                                text: inbound.text,
                                session_id: None,
                                is_mention: Some(true),
                            });
                        }
                    }
                    Err(err) => {
                        if !running.load(Ordering::SeqCst) {
                            break;
                        }
                        tracing::warn!(
                            channel = %channel_id,
                            "wechat getupdates failed: {err:#}; retrying"
                        );
                        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    }
                }
            }
        });
        tracing::info!(channel = %self.config.id, "wechat long poll started");
        Ok(())
    }

    async fn stop(&self) -> anyhow::Result<()> {
        self.running.store(false, Ordering::SeqCst);
        *self.creds.write().await = None;
        self.typing_tickets.write().await.clear();
        Ok(())
    }

    async fn send_text(&self, session_id: &str, text: &str) -> anyhow::Result<()> {
        let to_user_id = session_id.trim();
        if to_user_id.is_empty() {
            anyhow::bail!("wechat send_text requires user id");
        }
        let creds = self
            .creds
            .read()
            .await
            .clone()
            .ok_or_else(|| anyhow::anyhow!("wechat adapter is not connected"))?;
        let context_token = self
            .context_tokens
            .read()
            .await
            .get(to_user_id)
            .cloned()
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "No context_token for WeChat user {to_user_id}; wait for an inbound message first"
                )
            })?;
        let client = reqwest::Client::new();
        wechat_send_text(&client, &creds, to_user_id, text, &context_token).await
    }

    async fn set_typing(&self, user_id: &str, active: bool) -> anyhow::Result<()> {
        let status = if active { 1 } else { 2 };
        self.send_typing_status(user_id, status).await
    }
}
