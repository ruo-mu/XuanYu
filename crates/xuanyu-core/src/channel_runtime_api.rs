//! Channel connection / QR / inbound delivery APIs for AppCore.

use crate::channel_manager;
use crate::AppCore;
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::time::Duration;
use xuanyu_channels::{
    channel_webhook_port_from_config, login_wechat_via_qr, ChannelConfig, ChannelHub,
    InboundChannelMessage, InboundHandler,
};
use xuanyu_protocol::{AppEvent, EventId, MessageRole, SessionId, TurnId, SESSION_MAIN_THREAD_ID};

impl AppCore {
    pub(crate) async fn start_delivery_channels(&self, config: &Value) -> anyhow::Result<()> {
        let configs = channel_hub_configs_from_app_config(config);
        let enabled_count = configs.iter().filter(|config| config.enabled).count();
        if enabled_count == 0 {
            *self.channel_hub.write().await = None;
            return Ok(());
        }
        let port = channel_webhook_port_from_config(config);
        let hub = Arc::new(ChannelHub::from_configs_with_data_dir(
            &configs,
            port,
            self.storage.config().data_dir.clone(),
        ));
        let core = self.clone();
        let handler: InboundHandler = Arc::new(move |message| {
            let core = core.clone();
            tokio::spawn(async move {
                if let Err(err) = core.handle_inbound_channel_message(message).await {
                    tracing::warn!("inbound channel message failed: {err:#}");
                }
            });
        });
        match hub.start_all(handler).await {
            Ok(()) => {
                *self.channel_hub.write().await = Some(hub);
                self.observability
                    .log(
                        "info",
                        &format!(
                            "Started {enabled_count} delivery channel adapter(s) on webhook port {port}"
                        ),
                    )
                    .await;
                Ok(())
            }
            Err(err) => {
                tracing::warn!("channel hub start failed: {err:#}");
                Err(err)
            }
        }
    }

    async fn restart_delivery_channels(&self) -> anyhow::Result<()> {
        if let Some(hub) = self.channel_hub.write().await.take() {
            if let Err(err) = hub.stop_all().await {
                tracing::warn!("channel hub stop failed: {err:#}");
            }
        }
        let config = self.config_get().await.unwrap_or_else(|_| json!({}));
        self.start_delivery_channels(&config).await
    }

    pub async fn channel_adapters_list(&self) -> Value {
        channel_manager::adapters_list_payload()
    }

    pub async fn channel_connections_list(&self) -> Value {
        let config = self.config_get().await.unwrap_or_else(|_| json!({}));
        let configs = channel_manager::load_channel_configs_from_app_config(&config);
        let mut runtime = self.channel_runtime.write().await;
        sync_channel_runtime(&mut runtime, &configs);
        let connections = runtime
            .entries
            .values()
            .map(channel_manager::connection_runtime_payload)
            .collect::<Vec<_>>();
        json!({ "connections": connections })
    }

    pub async fn channel_connection_upsert(&self, payload: Value) -> Value {
        let config_value = payload
            .get("config")
            .cloned()
            .unwrap_or_else(|| payload.clone());
        let connect_after = payload
            .get("connectAfter")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let Some(incoming) = channel_manager::parse_managed_channel_config(&config_value) else {
            return json!({ "ok": false, "error": "Invalid channel config" });
        };
        let mut app_config = self.config_get().await.unwrap_or_else(|_| json!({}));
        let mut configs = channel_manager::load_channel_configs_from_app_config(&app_config);
        let key = channel_manager::connection_key(&incoming);
        let existing = configs
            .iter()
            .find(|item| channel_manager::connection_key(item) == key)
            .cloned();
        let merged = channel_manager::merge_managed_channel_config(existing.as_ref(), incoming);
        let key = channel_manager::connection_key(&merged);
        if let Some(index) = configs
            .iter()
            .position(|item| channel_manager::connection_key(item) == key)
        {
            configs[index] = merged.clone();
        } else {
            configs.push(merged.clone());
        }
        channel_manager::write_channel_configs_to_app_config(&mut app_config, &configs);
        if let Err(err) = self.storage.save_app_config(app_config).await {
            return json!({ "ok": false, "error": err.to_string() });
        }
        {
            let mut runtime = self.channel_runtime.write().await;
            sync_channel_runtime(&mut runtime, &configs);
            if let Some(entry) = runtime.entries.get_mut(&key) {
                entry.config = merged;
                entry.status = "idle".to_string();
                entry.error = None;
            }
        }
        if connect_after {
            let _ = self.channel_connection_connect(json!({ "key": key })).await;
        }
        json!({ "ok": true, "key": key })
    }

    pub async fn channel_connection_connect(&self, payload: Value) -> Value {
        let Some(key) = payload
            .get("key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
        else {
            return json!({ "connections": [], "error": "key required" });
        };
        {
            let mut runtime = self.channel_runtime.write().await;
            if let Some(entry) = runtime.entries.get_mut(&key) {
                entry.status = "connecting".to_string();
                entry.error = None;
                entry.config.enabled = true;
            }
        }
        let mut app_config = self.config_get().await.unwrap_or_else(|_| json!({}));
        let mut configs = channel_manager::load_channel_configs_from_app_config(&app_config);
        if let Some(config) = configs
            .iter_mut()
            .find(|item| channel_manager::connection_key(item) == key)
        {
            config.enabled = true;
        }
        channel_manager::write_channel_configs_to_app_config(&mut app_config, &configs);
        let _ = self.storage.save_app_config(app_config).await;

        let channel_cfg = configs
            .iter()
            .find(|item| channel_manager::connection_key(item) == key)
            .cloned();
        let has_creds = channel_cfg
            .as_ref()
            .map(channel_manager::channel_has_credentials)
            .unwrap_or(false);

        let start_error = match self.restart_delivery_channels().await {
            Ok(()) => None,
            Err(err) => Some(err.to_string()),
        };
        let hub_running = self.channel_hub.read().await.is_some();

        {
            let mut runtime = self.channel_runtime.write().await;
            if let Some(entry) = runtime.entries.get_mut(&key) {
                if !has_creds {
                    entry.status = "error".to_string();
                    entry.active = false;
                    entry.error = Some(
                        "Credentials are missing (token / appId / required extras).".to_string(),
                    );
                } else if let Some(err) = start_error {
                    entry.status = "error".to_string();
                    entry.active = false;
                    entry.error = Some(err);
                } else if hub_running {
                    entry.status = "connected".to_string();
                    entry.active = true;
                    entry.error = None;
                } else {
                    entry.status = "error".to_string();
                    entry.active = false;
                    entry.error = Some("Failed to start channel adapter".to_string());
                }
            }
        }
        self.channel_connections_list().await
    }

    pub async fn channel_connection_disconnect(&self, payload: Value) -> Value {
        let Some(key) = payload
            .get("key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
        else {
            return json!({ "connections": [], "error": "key required" });
        };
        let mut app_config = self.config_get().await.unwrap_or_else(|_| json!({}));
        let mut configs = channel_manager::load_channel_configs_from_app_config(&app_config);
        if let Some(config) = configs
            .iter_mut()
            .find(|item| channel_manager::connection_key(item) == key)
        {
            config.enabled = false;
        }
        channel_manager::write_channel_configs_to_app_config(&mut app_config, &configs);
        let _ = self.storage.save_app_config(app_config).await;
        {
            let mut runtime = self.channel_runtime.write().await;
            if let Some(entry) = runtime.entries.get_mut(&key) {
                entry.config.enabled = false;
                entry.status = "disconnected".to_string();
                entry.active = false;
                entry.error = None;
            }
        }
        let _ = self.restart_delivery_channels().await;
        self.channel_connections_list().await
    }

    pub async fn channel_connection_remove(&self, payload: Value) -> Value {
        let Some(key) = payload
            .get("key")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
        else {
            return json!({ "connections": [], "error": "key required" });
        };
        let mut app_config = self.config_get().await.unwrap_or_else(|_| json!({}));
        let mut configs = channel_manager::load_channel_configs_from_app_config(&app_config);
        configs.retain(|item| channel_manager::connection_key(item) != key);
        channel_manager::write_channel_configs_to_app_config(&mut app_config, &configs);
        let _ = self.storage.save_app_config(app_config).await;
        {
            let mut runtime = self.channel_runtime.write().await;
            runtime.entries.remove(&key);
            if runtime.qr_draft_id.as_deref() == Some(key.as_str()) {
                runtime.qr_draft_id = None;
            }
        }
        let _ = self.restart_delivery_channels().await;
        self.channel_connections_list().await
    }

    pub async fn channel_qr_begin(&self, payload: Value) -> Value {
        let channel_type = payload
            .get("type")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("wechat")
            .to_ascii_lowercase();
        let tenant_id = payload
            .get("tenantId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);

        if channel_type != "wechat" {
            return json!({
                "ok": false,
                "draftId": null,
                "error": format!("Channel {channel_type} does not support QR bind")
            });
        }

        // Cancel any in-flight QR bind first.
        let _ = self.channel_qr_cancel().await;

        let draft_id = format!(
            "__pending_{}_{}",
            channel_type,
            EventId::new().0.chars().take(8).collect::<String>()
        );
        let config = channel_manager::ManagedChannelConfig {
            channel_type: channel_type.clone(),
            name: draft_id.clone(),
            enabled: false,
            token: None,
            mention_only: None,
            allowlist: None,
            extra: None,
            tenant_id: tenant_id.clone(),
            connection_id: Some(draft_id.clone()),
        };
        let mut app_config = self.config_get().await.unwrap_or_else(|_| json!({}));
        let mut configs = channel_manager::load_channel_configs_from_app_config(&app_config);
        configs.push(config);
        channel_manager::write_channel_configs_to_app_config(&mut app_config, &configs);
        let _ = self.storage.save_app_config(app_config).await;

        let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
        {
            let mut runtime = self.channel_runtime.write().await;
            runtime.qr_draft_id = Some(draft_id.clone());
            runtime.qr_cancel_tx = Some(cancel_tx);
            sync_channel_runtime(&mut runtime, &configs);
            if let Some(entry) = runtime.entries.get_mut(&draft_id) {
                entry.status = "awaiting_qr".to_string();
                entry.error = None;
            }
        }

        let core = self.clone();
        let draft_for_task = draft_id.clone();
        let data_dir = self.storage.config().data_dir.clone();
        tokio::spawn(async move {
            let result = login_wechat_via_qr(
                &data_dir,
                None,
                Duration::from_secs(5 * 60),
                |qr_url| {
                    core.eventbus.publish(AppEvent::ChannelQr {
                        draft_id: draft_for_task.clone(),
                        qr_url,
                    });
                },
                cancel_rx,
            )
            .await;

            match result {
                Ok(creds) => {
                    let upsert = core
                        .channel_connection_upsert(json!({
                            "config": {
                                "type": "wechat",
                                "name": creds.account_id,
                                "enabled": true,
                                "token": creds.token,
                                "connectionId": format!("wechat:{}", creds.account_id),
                                "extra": {
                                    "accountId": creds.account_id,
                                    "baseUrl": creds.base_url,
                                    "userId": creds.user_id,
                                },
                                "tenantId": tenant_id,
                            },
                            "connectAfter": true
                        }))
                        .await;
                    let key = upsert
                        .get("key")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    let ok = upsert.get("ok").and_then(Value::as_bool).unwrap_or(false);
                    // Drop pending draft if still present.
                    let _ = core
                        .channel_connection_remove(json!({ "key": draft_for_task }))
                        .await;
                    {
                        let mut runtime = core.channel_runtime.write().await;
                        if runtime.qr_draft_id.as_deref() == Some(draft_for_task.as_str()) {
                            runtime.qr_draft_id = None;
                            runtime.qr_cancel_tx = None;
                        }
                    }
                    if ok {
                        core.eventbus.publish(AppEvent::ChannelQrResult {
                            draft_id: draft_for_task,
                            ok: true,
                            key,
                            error: None,
                        });
                    } else {
                        let error = upsert
                            .get("error")
                            .and_then(Value::as_str)
                            .unwrap_or("Failed to register WeChat connection after QR")
                            .to_string();
                        core.eventbus.publish(AppEvent::ChannelQrResult {
                            draft_id: draft_for_task,
                            ok: false,
                            key: None,
                            error: Some(error),
                        });
                    }
                }
                Err(err) => {
                    let message = err.to_string();
                    let aborted = message.contains("aborted");
                    if !aborted {
                        let _ = core
                            .channel_connection_remove(json!({ "key": draft_for_task }))
                            .await;
                    }
                    {
                        let mut runtime = core.channel_runtime.write().await;
                        if runtime.qr_draft_id.as_deref() == Some(draft_for_task.as_str()) {
                            runtime.qr_draft_id = None;
                            runtime.qr_cancel_tx = None;
                        }
                    }
                    if !aborted {
                        core.eventbus.publish(AppEvent::ChannelQrResult {
                            draft_id: draft_for_task,
                            ok: false,
                            key: None,
                            error: Some(message),
                        });
                    }
                }
            }
        });

        json!({
            "ok": true,
            "draftId": draft_id,
            "key": draft_id,
            "pending": true
        })
    }

    pub async fn channel_qr_cancel(&self) -> Value {
        let (draft_id, cancel_tx) = {
            let mut runtime = self.channel_runtime.write().await;
            let draft_id = runtime.qr_draft_id.take();
            let cancel_tx = runtime.qr_cancel_tx.take();
            (draft_id, cancel_tx)
        };
        if let Some(tx) = cancel_tx {
            let _ = tx.send(true);
        }
        if let Some(draft_id) = draft_id {
            let _ = self
                .channel_connection_remove(json!({ "key": draft_id }))
                .await;
        }
        json!({ "ok": true })
    }

    pub async fn channel_delivery_list(&self, limit: Option<usize>) -> Value {
        let limit = limit.unwrap_or(50);
        crate::channel_delivery::deliveries_payload(&self.storage.config().data_dir, limit)
    }

    async fn handle_inbound_channel_message(
        &self,
        message: InboundChannelMessage,
    ) -> anyhow::Result<()> {
        let session_id = if let Some(session_id) = message.session_id.filter(|id| !id.is_empty())
        {
            SessionId(session_id)
        } else {
            let sessions = self.sessions.list_sessions().await;
            if let Some(session) = sessions.first() {
                session.id.clone()
            } else {
                let session = self.create_main_session().await?;
                session.id.clone()
            }
        };
        crate::channel_delivery::append_delivery(crate::channel_delivery::AppendDeliveryInput {
            data_dir: &self.storage.config().data_dir,
            channel_id: &message.channel_id,
            direction: "inbound",
            external_user_id: &message.external_user_id,
            session_id: Some(&session_id.0),
            text: &message.text,
            status: "accepted",
            error: None,
        });
        if let Some(existing) = self
            .sessions
            .participants()
            .find_by_external_id(&session_id.0, &message.external_user_id)
            .await
        {
            let _ = self
                .sessions
                .participants()
                .touch(&session_id.0, &existing.id)
                .await;
        } else {
            let _ = self
                .sessions
                .join_participant(
                    &session_id,
                    "user",
                    Some(format!("{} ({})", message.channel_id, message.external_user_id)),
                    Some(message.external_user_id.clone()),
                )
                .await;
        }
        let _ = self
            .sessions
            .mailbox_push(
                &session_id,
                Some(message.external_user_id.clone()),
                Some(SESSION_MAIN_THREAD_ID.to_string()),
                json!({
                    "channelId": message.channel_id,
                    "text": message.text,
                    "externalUserId": message.external_user_id,
                }),
            )
            .await;
        self.observability
            .log(
                "info",
                &format!(
                    "Channel {} inbound from {} -> session {}",
                    message.channel_id, message.external_user_id, session_id.0
                ),
            )
            .await;

        let hub = self.channel_hub.read().await.clone();
        let typing_stop = Arc::new(AtomicBool::new(false));
        if let Some(hub) = hub.clone() {
            for adapter in hub.adapters() {
                if adapter.id() == message.channel_id {
                    let _ = adapter
                        .set_typing(&message.external_user_id, true)
                        .await;
                    let adapter = adapter.clone();
                    let user_id = message.external_user_id.clone();
                    let stop = typing_stop.clone();
                    tokio::spawn(async move {
                        while !stop.load(Ordering::SeqCst) {
                            tokio::time::sleep(Duration::from_secs(5)).await;
                            if stop.load(Ordering::SeqCst) {
                                break;
                            }
                            let _ = adapter.set_typing(&user_id, true).await;
                        }
                    });
                    break;
                }
            }
        }

        let turn_id = self
            .send_message(session_id.clone(), message.text.clone())
            .await?;
        let turn_result = self
            .wait_for_turn(&session_id, &turn_id, Duration::from_secs(300))
            .await;
        typing_stop.store(true, Ordering::SeqCst);
        if let Some(hub) = hub.clone() {
            for adapter in hub.adapters() {
                if adapter.id() == message.channel_id {
                    let _ = adapter
                        .set_typing(&message.external_user_id, false)
                        .await;
                    break;
                }
            }
        }
        if let Err(err) = turn_result {
            tracing::warn!(
                "channel inbound turn did not finish for {}: {err:#}",
                message.channel_id
            );
            let delivery_error = err.to_string();
            crate::channel_delivery::append_delivery(crate::channel_delivery::AppendDeliveryInput {
                data_dir: &self.storage.config().data_dir,
                channel_id: &message.channel_id,
                direction: "outbound",
                external_user_id: &message.external_user_id,
                session_id: Some(&session_id.0),
                text: "",
                status: "turn_failed",
                error: Some(&delivery_error),
            });
            return Ok(());
        }
        let reply = self
            .assistant_reply_for_turn(&session_id, &turn_id)
            .await
            .unwrap_or_default();
        let reply = reply.trim().to_string();
        if reply.is_empty() {
            crate::channel_delivery::append_delivery(crate::channel_delivery::AppendDeliveryInput {
                data_dir: &self.storage.config().data_dir,
                channel_id: &message.channel_id,
                direction: "outbound",
                external_user_id: &message.external_user_id,
                session_id: Some(&session_id.0),
                text: "",
                status: "empty_reply",
                error: None,
            });
            return Ok(());
        }
        if let Some(hub) = hub {
            let outbound_target = channel_outbound_target(&session_id.0, &message.external_user_id);
            for adapter in hub.adapters() {
                if adapter.id() == message.channel_id {
                    match adapter
                        .send_text(&outbound_target, &reply)
                        .await
                    {
                        Ok(()) => {
                            crate::channel_delivery::append_delivery(
                                crate::channel_delivery::AppendDeliveryInput {
                                    data_dir: &self.storage.config().data_dir,
                                    channel_id: &message.channel_id,
                                    direction: "outbound",
                                    external_user_id: &message.external_user_id,
                                    session_id: Some(&session_id.0),
                                    text: &reply,
                                    status: "sent",
                                    error: None,
                                },
                            );
                        }
                        Err(err) => {
                            tracing::warn!(
                                "channel {} outbound reply failed: {err:#}",
                                message.channel_id
                            );
                            let delivery_error = err.to_string();
                            crate::channel_delivery::append_delivery(
                                crate::channel_delivery::AppendDeliveryInput {
                                    data_dir: &self.storage.config().data_dir,
                                    channel_id: &message.channel_id,
                                    direction: "outbound",
                                    external_user_id: &message.external_user_id,
                                    session_id: Some(&session_id.0),
                                    text: &reply,
                                    status: "send_failed",
                                    error: Some(&delivery_error),
                                },
                            );
                        }
                    }
                    break;
                }
            }
        }
        Ok(())
    }

    async fn assistant_reply_for_turn(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
    ) -> Option<String> {
        let messages = self.session_history(session_id.clone()).await.ok()?;
        messages
            .into_iter()
            .rev()
            .find(|message| {
                message.role == MessageRole::Assistant
                    && message.turn_id.as_ref() == Some(turn_id)
            })
            .map(|message| message.content)
    }

}

fn channel_outbound_target(session_key: &str, external_user_id: &str) -> String {
    if session_key.starts_with("discord:")
        || session_key.starts_with("telegram:")
        || session_key.starts_with("feishu:")
        || session_key.starts_with("lark:")
    {
        session_key.to_string()
    } else {
        external_user_id.to_string()
    }
}

fn channel_hub_configs_from_app_config(config: &Value) -> Vec<ChannelConfig> {
    channel_manager::load_channel_configs_from_app_config(config)
        .into_iter()
        .filter(|item| item.enabled)
        .map(|item| {
            let id = channel_manager::connection_key(&item);
            let app_id = item
                .extra
                .as_ref()
                .and_then(|extra| {
                    extra
                        .get("appId")
                        .or_else(|| extra.get("app_id"))
                        .cloned()
                })
                .filter(|value| !value.trim().is_empty());
            let webhook_url = item.extra.as_ref().and_then(|extra| {
                extra
                    .get("webhookUrl")
                    .or_else(|| extra.get("webhook_url"))
                    .cloned()
            });
            let transport = item.extra.as_ref().and_then(|extra| {
                extra.get("transport").cloned()
            });
            let mut extra = item.extra.clone().unwrap_or_default();
            if let Some(mention_only) = item.mention_only {
                extra.insert(
                    "mentionOnly".to_string(),
                    if mention_only { "true" } else { "false" }.to_string(),
                );
            }
            if item.channel_type.eq_ignore_ascii_case("wechat") {
                if !extra.contains_key("accountId") && !extra.contains_key("account_id") {
                    let name = item.name.trim();
                    if !name.is_empty() && !name.starts_with("__pending_") {
                        extra.insert("accountId".to_string(), name.to_string());
                    }
                }
                if !extra.contains_key("transport") {
                    extra.insert("transport".to_string(), "long_poll".to_string());
                }
            }
            ChannelConfig {
                id,
                kind: item.channel_type,
                enabled: item.enabled,
                token: item.token,
                webhook_url,
                transport: transport.or_else(|| {
                    extra.get("transport").cloned()
                }),
                app_id,
                extra: if extra.is_empty() { None } else { Some(extra) },
            }
        })
        .collect()
}

fn sync_channel_runtime(
    runtime: &mut channel_manager::ChannelRuntimeStore,
    configs: &[channel_manager::ManagedChannelConfig],
) {
    let keys: BTreeSet<String> = configs
        .iter()
        .map(channel_manager::connection_key)
        .collect();
    runtime.entries.retain(|key, _| keys.contains(key));
    for config in configs {
        let key = channel_manager::connection_key(config);
        match runtime.entries.get_mut(&key) {
            Some(entry) => {
                entry.config = config.clone();
            }
            None => {
                runtime.entries.insert(
                    key,
                    channel_manager::ChannelRuntimeEntry {
                        config: config.clone(),
                        status: if config.enabled {
                            "idle".to_string()
                        } else {
                            "disconnected".to_string()
                        },
                        error: None,
                        message_count: 0,
                        active: false,
                    },
                );
            }
        }
    }
}

