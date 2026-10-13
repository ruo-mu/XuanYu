use chrono::Utc;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;
use xuanyu_eventbus::EventBus;
use xuanyu_protocol::{
    AppError, AppEvent, ChatAttachment, ChatMessage, ErrorCode, MessageRole, SessionId,
    SessionInfo, TurnId,
};
use xuanyu_storage::{
    ConversationMessageRecord, ConversationSegmentRecord, ConversationSessionPatch,
    ConversationSessionRecord, ConversationSessionStatus, ConversationSnapshotRecord,
    ConversationStoreHealth, PostgresConversationStore,
};

mod message_queue;
mod session_bridge;
mod session_manager;
mod participants;
mod threads;
mod mailbox;
mod isolation;
mod turn_write;

pub use message_queue::{QueuedSessionMessage, SessionMessageQueue};
pub mod runs;
pub use runs::{
    AttemptComparison, CreateRunInput, EvidencePack, EvidenceResourceUsage, RunEvent, RunRecord,
    RunStore, RunTimeline, StartStepInput, StepRecord, TraceEvent, TraceSummary, TraceTimeline,
    UserFeedbackRecord,
};
pub use session_bridge::SessionBridge;
pub use session_manager::SessionManager;
pub use participants::SessionParticipantRegistry;
pub use threads::SessionThreadRegistry;
pub use mailbox::SessionMailboxRegistry;
pub use isolation::SessionIsolationRegistry;

use turn_write::{
    build_text_segment_record, build_tool_start_segment, estimate_text_bytes, new_segment_id,
    BuildToolStartSegmentInput,
    text_segment_fingerprint, PendingTextSegment, TurnPersistState, TurnWriteBuffer,
};

pub struct SessionKernel {
    eventbus: EventBus,
    store: Arc<PostgresConversationStore>,
    sessions: RwLock<BTreeMap<String, SessionRecord>>,
    /** 本轮共用一条 assistant，工具以 turn_segments 挂载。 */
    turn_persist: RwLock<BTreeMap<String, TurnPersistState>>,
    message_queue: SessionMessageQueue,
    participants: SessionParticipantRegistry,
    threads: SessionThreadRegistry,
    mailbox: SessionMailboxRegistry,
    isolation: SessionIsolationRegistry,
}

#[derive(Debug, Clone)]
struct SessionRecord {
    info: SessionInfo,
    status: ConversationSessionStatus,
}

fn turn_persist_key(session_id: &str, turn_id: &str) -> String {
    format!("{session_id}::{turn_id}")
}

impl SessionKernel {
    pub fn new(eventbus: EventBus, store: Arc<PostgresConversationStore>) -> Self {
        Self {
            eventbus,
            store,
            sessions: RwLock::new(BTreeMap::new()),
            turn_persist: RwLock::new(BTreeMap::new()),
            message_queue: SessionMessageQueue::new(),
            participants: SessionParticipantRegistry::default(),
            threads: SessionThreadRegistry::default(),
            mailbox: SessionMailboxRegistry::default(),
            isolation: SessionIsolationRegistry::default(),
        }
    }

    pub fn participants(&self) -> &SessionParticipantRegistry {
        &self.participants
    }

    pub fn threads(&self) -> &SessionThreadRegistry {
        &self.threads
    }

    pub fn mailbox(&self) -> &SessionMailboxRegistry {
        &self.mailbox
    }

    pub fn isolation(&self) -> &SessionIsolationRegistry {
        &self.isolation
    }

    async fn init_session_containers(&self, session_id: &str, workspace_root: Option<&str>) {
        self.participants.ensure_default(session_id).await;
        self.threads.ensure_main(session_id).await;
        if self.isolation.get(session_id).await.is_none() {
            let root = workspace_root
                .filter(|value| !value.is_empty())
                .unwrap_or(".")
                .to_string();
            self.isolation
                .set(
                    session_id,
                    xuanyu_protocol::SessionIsolationScope {
                        workspace_root: root,
                        allowed_paths: Vec::new(),
                        readonly: false,
                    },
                )
                .await;
        }
    }

    pub fn message_queue(&self) -> &SessionMessageQueue {
        &self.message_queue
    }

    pub fn eventbus(&self) -> &EventBus {
        &self.eventbus
    }

    pub async fn is_processing(&self, session_id: &SessionId) -> bool {
        self.message_queue.is_processing(&session_id.0).await
    }

    pub async fn set_processing(&self, session_id: &SessionId, processing: bool) {
        self.message_queue
            .set_processing(&session_id.0, processing)
            .await
    }

    pub async fn create_main_session(&self) -> anyhow::Result<SessionInfo> {
        if let Ok(existing) = self.store.list_sessions(false, 500).await {
            if let Some(session) = existing.into_iter().find(|session| session.id == "main") {
                let id = SessionId(session.id);
                if let Some(record) = self.restore_session_record(&id).await? {
                    self.eventbus.publish(AppEvent::SessionCreated {
                        session: record.info.clone(),
                    });
                    return Ok(record.info);
                }
            }
        }

        let base_id = SessionId("main".to_string());
        let id = match self.store.get_history(&base_id.0).await?.session {
            Some(session)
                if session.status == ConversationSessionStatus::Archived
                    || session.status == ConversationSessionStatus::Deleted =>
            {
                SessionId(format!("main-{}", Uuid::new_v4().simple()))
            }
            _ => base_id,
        };
        if let Some(record) = self.restore_session_record(&id).await? {
            self.eventbus.publish(AppEvent::SessionCreated {
                session: record.info.clone(),
            });
            return Ok(record.info);
        }

        let now = Utc::now();
        let info = SessionInfo {
            id: id.clone(),
            name: "新对话".to_string(),
            created_at: now,
            updated_at: now,
            message_count: 0,
            is_processing: false,
            storage_backend: "postgres".to_string(),
            title_status: None,
            title_generated_by: None,
            title_generation_error: None,
            title_provider: None,
            title_model: None,
            pinned: false,
            project_dir: None,
        };
        let mut sessions = self.sessions.write().await;
        let record = sessions
            .entry(id.0.clone())
            .or_insert_with(|| SessionRecord {
                info: info.clone(),
                status: ConversationSessionStatus::Idle,
            });
        self.store
            .create_session(id.0.clone(), record.info.name.clone())
            .await?;
        self.eventbus.publish(AppEvent::SessionCreated {
            session: record.info.clone(),
        });
        self.init_session_containers(&id.0, None).await;
        Ok(record.info.clone())
    }

    /// Create a fresh Web conversation thread.
    /// `project_dir = None` → unscoped「最近」; Some(path) → project-scoped branch.
    pub async fn create_branch_session(
        &self,
        title: Option<String>,
        project_dir: Option<String>,
    ) -> anyhow::Result<SessionInfo> {
        let id = SessionId(format!("web_{}", Uuid::new_v4().simple()));
        let name = title
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "新对话".to_string());
        let resolved_project = project_dir
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let mut metadata = BTreeMap::new();
        if let Some(dir) = resolved_project.as_ref() {
            metadata.insert("projectDir".to_string(), json!(dir));
            metadata.insert("workspace".to_string(), json!(true));
        }
        let now = Utc::now();
        let info = SessionInfo {
            id: id.clone(),
            name: name.clone(),
            created_at: now,
            updated_at: now,
            message_count: 0,
            is_processing: false,
            storage_backend: "postgres".to_string(),
            title_status: None,
            title_generated_by: None,
            title_generation_error: None,
            title_provider: None,
            title_model: None,
            pinned: false,
            project_dir: resolved_project.clone(),
        };
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(
                id.0.clone(),
                SessionRecord {
                    info: info.clone(),
                    status: ConversationSessionStatus::Idle,
                },
            );
        }
        let mut record = self.store.create_session(id.0.clone(), name).await?;
        if !metadata.is_empty() {
            self.store
                .update_session(
                    &id.0,
                    ConversationSessionPatch {
                        metadata: Some(metadata),
                        ..Default::default()
                    },
                )
                .await?;
            if let Ok(history) = self.store.get_history(&id.0).await {
                if let Some(updated) = history.session {
                    record = updated;
                }
            }
        }
        let info = session_info_from_record(record);
        {
            let mut sessions = self.sessions.write().await;
            if let Some(entry) = sessions.get_mut(&id.0) {
                entry.info = info.clone();
            }
        }
        self.eventbus.publish(AppEvent::SessionCreated {
            session: info.clone(),
        });
        self.init_session_containers(&id.0, None).await;
        Ok(info)
    }

    pub async fn list_sessions(&self) -> Vec<SessionInfo> {
        self.query_sessions(false, None, 500).await
    }

    pub async fn query_sessions(
        &self,
        include_archived: bool,
        search: Option<&str>,
        limit: usize,
    ) -> Vec<SessionInfo> {
        let normalized_search = search
            .map(str::trim)
            .filter(|query| !query.is_empty())
            .map(str::to_lowercase);
        let mut sessions = self
            .sessions
            .read()
            .await
            .values()
            .filter(|record| {
                (include_archived || record.status != ConversationSessionStatus::Archived)
                    && record.status != ConversationSessionStatus::Deleted
            })
            .filter(|record| {
                normalized_search
                    .as_ref()
                    .map(|query| record.info.name.to_lowercase().contains(query))
                    .unwrap_or(true)
            })
            .map(|record| record.info.clone())
            .collect::<Vec<_>>();
        if let Ok(persisted) = self
            .store
            .query_sessions(include_archived, search, limit)
            .await
        {
            let known = sessions
                .iter()
                .map(|session| session.id.0.clone())
                .collect::<std::collections::BTreeSet<_>>();
            sessions.extend(
                persisted
                    .into_iter()
                    .filter(|record| !known.contains(&record.id))
                    .map(session_info_from_record),
            );
        }
        sessions.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
        sessions.truncate(limit.max(1));
        sessions
    }

    pub async fn search_conversations(&self, query: &str, limit: usize) -> Vec<SessionInfo> {
        self.query_sessions(false, Some(query), limit).await
    }

    pub async fn history(&self, session_id: &SessionId) -> anyhow::Result<Vec<ChatMessage>> {
        // 打开历史会话时把记录 hydrate 进内存，后续续聊 / begin_turn 才能命中。
        let _ = self.restore_session_record(session_id).await?;
        // 始终从存储读完整 history（含 metadata / segments），避免内存瘦消息导致时间线丢工具。
        let history = self.store.get_history(&session_id.0).await?;
        Ok(history_to_chat_messages(history))
    }

    /** 本轮仍在写入 assistant/segments（尚未 finish_turn）。 */
    pub async fn turn_persist_active(&self, session_id: &str, turn_id: &str) -> bool {
        self.turn_persist
            .read()
            .await
            .contains_key(&turn_persist_key(session_id, turn_id))
    }

    pub async fn replace_history_records(
        &self,
        session_id: &SessionId,
        messages: Vec<ConversationMessageRecord>,
    ) -> anyhow::Result<SessionInfo> {
        self.store.replace_messages(&session_id.0, messages).await?;
        let history = self.store.get_history(&session_id.0).await?;
        let session = history
            .session
            .clone()
            .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id.0))?;
        let status = session.status;
        let info = session_info_from_record(session);
        let record = SessionRecord {
            info: info.clone(),
            status,
        };
        let chat_messages = history_to_chat_messages(history);
        self.sessions
            .write()
            .await
            .insert(session_id.0.clone(), record);
        self.eventbus.publish(AppEvent::SessionUpdated {
            session: info.clone(),
        });
        self.eventbus.publish(AppEvent::SessionHistory {
            session_id: session_id.clone(),
            messages: chat_messages,
        });
        Ok(info)
    }

    pub async fn restore_session(
        &self,
        session_id: &SessionId,
    ) -> anyhow::Result<Option<SessionInfo>> {
        Ok(self
            .restore_session_record(session_id)
            .await?
            .map(|record| record.info))
    }

    pub async fn pin_session(
        &self,
        session_id: &SessionId,
        pinned: bool,
    ) -> anyhow::Result<Option<SessionInfo>> {
        if self.store.get_session(&session_id.0).await?.is_none() {
            return Ok(None);
        }
        let now = Utc::now();
        self.store
            .update_session(
                &session_id.0,
                ConversationSessionPatch {
                    pinned: Some(pinned),
                    updated_at: Some(now),
                    ..Default::default()
                },
            )
            .await?;
        if let Some(mut record) = self.restore_session_record(session_id).await? {
            record.info.pinned = pinned;
            record.info.updated_at = now;
            self.sessions
                .write()
                .await
                .insert(session_id.0.clone(), record.clone());
            self.eventbus.publish(AppEvent::SessionUpdated {
                session: record.info.clone(),
            });
            return Ok(Some(record.info));
        }
        Ok(None)
    }

    pub async fn rename_session(
        &self,
        session_id: &SessionId,
        name: impl Into<String>,
    ) -> anyhow::Result<Option<SessionInfo>> {
        let title = name.into().trim().to_string();
        if title.is_empty() {
            anyhow::bail!("Session name is required");
        }

        let Some(mut record) = self.restore_session_record(session_id).await? else {
            return Ok(None);
        };
        self.store
            .update_session(
                &session_id.0,
                xuanyu_storage::ConversationSessionPatch {
                    title: Some(title.clone()),
                    updated_at: Some(Utc::now()),
                    ..Default::default()
                },
            )
            .await?;
        record.info.name = title;
        record.info.updated_at = Utc::now();
        self.sessions
            .write()
            .await
            .insert(session_id.0.clone(), record.clone());
        self.eventbus.publish(AppEvent::SessionUpdated {
            session: record.info.clone(),
        });
        Ok(Some(record.info))
    }

    pub async fn archive_session(
        &self,
        session_id: &SessionId,
        archived: bool,
    ) -> anyhow::Result<bool> {
        if self.store.get_session(&session_id.0).await?.is_none() {
            return Ok(false);
        }
        self.store.archive_session(&session_id.0, archived).await?;
        self.sessions.write().await.remove(&session_id.0);
        if !archived {
            if let Some(record) = self.restore_session_record(session_id).await? {
                self.eventbus.publish(AppEvent::SessionUpdated {
                    session: record.info,
                });
            }
        }
        Ok(true)
    }

    pub async fn delete_session(&self, session_id: &SessionId) -> anyhow::Result<bool> {
        if self.store.get_session(&session_id.0).await?.is_none() {
            return Ok(false);
        }
        self.store.delete_session(&session_id.0).await?;
        self.sessions.write().await.remove(&session_id.0);
        Ok(true)
    }

    pub async fn abort_session(
        &self,
        session_id: &SessionId,
    ) -> anyhow::Result<Option<SessionInfo>> {
        let Some(mut record) = self.restore_session_record(session_id).await? else {
            return Ok(None);
        };
        let was_processing =
            record.info.is_processing || record.status == ConversationSessionStatus::Active;
        let active_turn_id = if was_processing {
            let prefix = format!("{}::", session_id.0);
            let guard = self.turn_persist.read().await;
            guard.keys().find_map(|key| {
                key.strip_prefix(&prefix)
                    .map(|turn| TurnId(turn.to_string()))
            })
        } else {
            None
        };
        // 中止前刷出本轮未落库的正文缓冲。
        if let Some(turn_id) = active_turn_id.as_ref() {
            let _ = self.flush_turn_write_buffer(session_id, turn_id).await;
        }
        let now = Utc::now();
        self.store
            .update_session(
                &session_id.0,
                ConversationSessionPatch {
                    status: Some(ConversationSessionStatus::Idle),
                    current_run_id: Some(None),
                    updated_at: Some(now),
                    ..Default::default()
                },
            )
            .await?;
        record.status = ConversationSessionStatus::Idle;
        record.info.is_processing = false;
        record.info.updated_at = now;
        self.sessions
            .write()
            .await
            .insert(session_id.0.clone(), record.clone());
        self.eventbus.publish(AppEvent::SessionUpdated {
            session: record.info.clone(),
        });
        if let Some(turn_id) = active_turn_id {
            self.eventbus.publish(AppEvent::TurnFailed {
                session_id: session_id.clone(),
                turn_id,
                error: AppError {
                    code: ErrorCode::TurnAborted,
                    message: "Turn aborted by user".to_string(),
                    recoverable: true,
                    suggestion: None,
                    context: BTreeMap::new(),
                },
            });
        }
        Ok(Some(record.info))
    }

    pub async fn conversation_store_health(&self) -> ConversationStoreHealth {
        self.store.health().await
    }

    pub async fn append_user_message(
        &self,
        session_id: &SessionId,
        content: String,
    ) -> anyhow::Result<TurnId> {
        self.append_user_message_with_attachments(session_id, content, None)
            .await
    }

    pub async fn append_user_message_with_attachments(
        &self,
        session_id: &SessionId,
        content: String,
        attachments: Option<Vec<ChatAttachment>>,
    ) -> anyhow::Result<TurnId> {
        // 引擎重启后内存表为空：续聊前必须从存储 hydrate。
        self.require_session_record(session_id).await?;
        let turn_id = TurnId(Uuid::new_v4().to_string());
        let (message_count, last_message_id) = self
            .store
            .last_message_pointer(&session_id.0)
            .await
            .unwrap_or((0, None));
        let attachments = attachments.filter(|items| !items.is_empty());
        let message_metadata = attachments
            .as_ref()
            .map(|items| BTreeMap::from([("attachments".to_string(), serde_json::json!(items))]))
            .unwrap_or_default();
        // 指针检查点：禁止嵌入全量 messages（避免 O(n²) 内存/磁盘尖峰）。
        self.store
            .add_snapshot(ConversationSnapshotRecord {
                id: format!("turn_cp_{}", turn_id.0),
                session_id: session_id.0.clone(),
                run_id: None,
                checkpoint_id: Some(turn_id.0.clone()),
                summary: Some(content.chars().take(240).collect()),
                conversation: json!({
                    "phase": "pre-turn",
                    "turnId": turn_id.0,
                    "userMessagePreview": content.chars().take(240).collect::<String>(),
                    "messageCount": message_count,
                    "lastMessageId": last_message_id,
                }),
                context_budget: None,
                created_at: Utc::now(),
            })
            .await?;
        let mut sessions = self.sessions.write().await;
        let record = sessions
            .get_mut(&session_id.0)
            .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id.0))?;
        record.info.message_count += 1;
        record.info.updated_at = Utc::now();
        record.info.is_processing = true;
        record.status = ConversationSessionStatus::Active;
        self.store
            .add_message(ConversationMessageRecord {
                id: format!("msg_{}", Uuid::new_v4()),
                session_id: session_id.0.clone(),
                role: "user".to_string(),
                content,
                created_at: Utc::now(),
                token_count: None,
                run_id: None,
                turn_id: Some(turn_id.0.clone()),
                metadata: message_metadata,
                tool_call_id: None,
                name: None,
            })
            .await?;
        let updated = record.info.clone();
        drop(sessions);
        self.eventbus.publish(AppEvent::SessionUpdated {
            session: updated,
        });
        self.eventbus.publish(AppEvent::TurnStarted {
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
        });
        Ok(turn_id)
    }

    pub async fn begin_turn(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
    ) -> anyhow::Result<()> {
        self.require_session_record(session_id).await?;
        let mut sessions = self.sessions.write().await;
        let record = sessions
            .get_mut(&session_id.0)
            .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id.0))?;
        record.info.updated_at = Utc::now();
        record.info.is_processing = true;
        record.status = ConversationSessionStatus::Active;
        drop(sessions);
        self.eventbus.publish(AppEvent::TurnStarted {
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
        });
        Ok(())
    }

    pub async fn append_assistant_message(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
        content: String,
    ) -> anyhow::Result<()> {
        self.append_assistant_message_with_metadata(
            session_id,
            turn_id,
            content,
            BTreeMap::new(),
            true,
        )
        .await
    }

    pub async fn append_assistant_message_with_metadata(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
        content: String,
        metadata: BTreeMap<String, serde_json::Value>,
        finish_turn: bool,
    ) -> anyhow::Result<()> {
        let message_id = self
            .ensure_turn_assistant_message(session_id, turn_id, &metadata)
            .await?;
        if finish_turn {
            // 先刷缓冲，再按指纹决定是否补最终 text。
            self.flush_turn_write_buffer(session_id, turn_id).await?;
            if !content.trim().is_empty() {
                self.enqueue_assistant_text(session_id, turn_id, &content, None, true)
                    .await?;
            }
            self.flush_turn_write_buffer(session_id, turn_id).await?;
        }

        let now = Utc::now();
        {
            let mut sessions = self.sessions.write().await;
            let record = sessions
                .get_mut(&session_id.0)
                .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id.0))?;
            record.info.updated_at = now;
            record.info.is_processing = !finish_turn;
            record.status = if finish_turn {
                ConversationSessionStatus::Idle
            } else {
                ConversationSessionStatus::Active
            };
        }
        let store_content = if content.is_empty() {
            self.turn_persist
                .read()
                .await
                .get(&turn_persist_key(&session_id.0, &turn_id.0))
                .and_then(|state| state.latest_assistant_content.clone())
                .unwrap_or_default()
        } else {
            content.clone()
        };
        self.store
            .add_message(ConversationMessageRecord {
                id: message_id,
                session_id: session_id.0.clone(),
                role: "assistant".to_string(),
                content: store_content,
                created_at: now,
                token_count: None,
                run_id: None,
                turn_id: Some(turn_id.0.clone()),
                metadata,
                tool_call_id: None,
                name: None,
            })
            .await?;
        if finish_turn {
            self.turn_persist
                .write()
                .await
                .remove(&turn_persist_key(&session_id.0, &turn_id.0));
            self.eventbus.publish(AppEvent::TurnFinished {
                session_id: session_id.clone(),
                turn_id: turn_id.clone(),
            });
        }
        Ok(())
    }

    /**
     * 本轮模型正文/思考：进入有界写缓冲；满缓冲或调用方强制 flush 时落库。
     * 工具边界会先 flush，保证崩溃后旁白仍在工具之前。
     */
    pub async fn record_assistant_text_segment(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
        content: &str,
        thinking: Option<&str>,
    ) -> anyhow::Result<()> {
        self.enqueue_assistant_text(session_id, turn_id, content, thinking, false)
            .await
    }

    async fn enqueue_assistant_text(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
        content: &str,
        thinking: Option<&str>,
        force_flush: bool,
    ) -> anyhow::Result<()> {
        let content = content.trim();
        let thinking = thinking.map(str::trim).filter(|value| !value.is_empty());
        if content.is_empty() && thinking.is_none() {
            return Ok(());
        }
        let fingerprint = text_segment_fingerprint(content, thinking);
        let _message_id = self
            .ensure_turn_assistant_message(session_id, turn_id, &BTreeMap::new())
            .await?;
        let key = turn_persist_key(&session_id.0, &turn_id.0);
        let add_bytes = estimate_text_bytes(content, thinking);
        let need_flush = {
            let guard = self.turn_persist.read().await;
            guard
                .get(&key)
                .map(|state| {
                    state.write_buffer.would_exceed(add_bytes) && !state.write_buffer.is_empty()
                })
                .unwrap_or(false)
        };
        if need_flush {
            self.flush_turn_write_buffer(session_id, turn_id).await?;
        }
        {
            let mut guard = self.turn_persist.write().await;
            let state = guard.entry(key).or_default();
            if state.last_text_fingerprint.as_ref() == Some(&fingerprint) {
                return Ok(());
            }
            if state
                .write_buffer
                .pending_text
                .last()
                .map(|item| item.fingerprint.as_str())
                == Some(fingerprint.as_str())
            {
                return Ok(());
            }
            if !content.is_empty() {
                state.latest_assistant_content = Some(content.to_string());
            }
            state.write_buffer.push_text(PendingTextSegment {
                id: new_segment_id(),
                content: content.to_string(),
                thinking: thinking.map(str::to_string),
                fingerprint,
                bytes: add_bytes,
            });
        }
        if force_flush {
            self.flush_turn_write_buffer(session_id, turn_id).await?;
        }
        Ok(())
    }

    async fn flush_turn_write_buffer(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
    ) -> anyhow::Result<()> {
        let key = turn_persist_key(&session_id.0, &turn_id.0);
        let (message_id, pending, latest_content) = {
            let mut guard = self.turn_persist.write().await;
            let Some(state) = guard.get_mut(&key) else {
                return Ok(());
            };
            if state.write_buffer.is_empty() {
                return Ok(());
            }
            (
                state.assistant_message_id.clone(),
                state.write_buffer.take_text(),
                state.latest_assistant_content.clone(),
            )
        };
        if pending.is_empty() {
            return Ok(());
        }
        let start_seq = self
            .store
            .reserve_segment_sequences(&session_id.0, pending.len())
            .await?;
        let records = pending
            .iter()
            .enumerate()
            .map(|(index, item)| {
                build_text_segment_record(
                    &session_id.0,
                    &turn_id.0,
                    &message_id,
                    start_seq + index as u64,
                    item,
                )
            })
            .collect::<Vec<_>>();
        self.store.add_segments_batch(&records).await?;
        if let Some(content) = latest_content.filter(|value| !value.is_empty()) {
            self.store
                .add_message(ConversationMessageRecord {
                    id: message_id,
                    session_id: session_id.0.clone(),
                    role: "assistant".to_string(),
                    content,
                    created_at: Utc::now(),
                    token_count: None,
                    run_id: None,
                    turn_id: Some(turn_id.0.clone()),
                    metadata: BTreeMap::new(),
                    tool_call_id: None,
                    name: None,
                })
                .await?;
        }
        if let Some(last) = pending.last() {
            let mut guard = self.turn_persist.write().await;
            if let Some(state) = guard.get_mut(&key) {
                state.last_text_fingerprint = Some(last.fingerprint.clone());
            }
        }
        Ok(())
    }

    /** 确保本轮只有一条 assistant 消息。 */
    pub async fn ensure_turn_assistant_message(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
        metadata: &BTreeMap<String, serde_json::Value>,
    ) -> anyhow::Result<String> {
        let key = turn_persist_key(&session_id.0, &turn_id.0);
        {
            let guard = self.turn_persist.read().await;
            if let Some(state) = guard.get(&key) {
                if !state.assistant_message_id.is_empty() {
                    return Ok(state.assistant_message_id.clone());
                }
            }
        }
        let message_id = format!("msg_{}", Uuid::new_v4());
        let now = Utc::now();
        let _ = self.restore_session_record(session_id).await?;
        {
            let mut sessions = self.sessions.write().await;
            let record = sessions
                .get_mut(&session_id.0)
                .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id.0))?;
            record.info.is_processing = true;
            record.status = ConversationSessionStatus::Active;
            record.info.updated_at = now;
            record.info.message_count += 1;
        }
        self.store
            .add_message(ConversationMessageRecord {
                id: message_id.clone(),
                session_id: session_id.0.clone(),
                role: "assistant".to_string(),
                content: String::new(),
                created_at: now,
                token_count: None,
                run_id: None,
                turn_id: Some(turn_id.0.clone()),
                metadata: metadata.clone(),
                tool_call_id: None,
                name: None,
            })
            .await?;
        self.turn_persist.write().await.insert(
            key,
            TurnPersistState {
                assistant_message_id: message_id.clone(),
                segment_by_tool_call: BTreeMap::new(),
                last_text_fingerprint: None,
                write_buffer: TurnWriteBuffer::default(),
                latest_assistant_content: None,
            },
        );
        Ok(message_id)
    }

    /** 工具调用开始：先 flush 正文缓冲，再立即写入 calling segment。 */
    pub async fn record_tool_call_segment(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
        tool_call_id: &str,
        tool_name: &str,
        args: &Value,
    ) -> anyhow::Result<()> {
        let message_id = self
            .ensure_turn_assistant_message(session_id, turn_id, &BTreeMap::new())
            .await?;
        self.flush_turn_write_buffer(session_id, turn_id).await?;
        let key = turn_persist_key(&session_id.0, &turn_id.0);
        {
            let guard = self.turn_persist.read().await;
            if let Some(state) = guard.get(&key) {
                if state.segment_by_tool_call.contains_key(tool_call_id) {
                    return Ok(());
                }
            }
        }
        let segment_id = new_segment_id();
        let seq = self.store.next_segment_sequence(&session_id.0).await?;
        let segment_type = tool_name_to_segment_type(tool_name);
        let title_content = describe_tool_segment(tool_name);
        let record = build_tool_start_segment(BuildToolStartSegmentInput {
            session_id: &session_id.0,
            turn_id: &turn_id.0,
            message_id: &message_id,
            sequence_number: seq,
            segment_id: &segment_id,
            tool_call_id,
            tool_name,
            args,
            segment_type,
            title_content: &title_content,
        });
        self.store.add_segments_batch(std::slice::from_ref(&record)).await?;
        self.turn_persist
            .write()
            .await
            .entry(key)
            .or_default()
            .segment_by_tool_call
            .insert(tool_call_id.to_string(), segment_id);
        Ok(())
    }

    /** 工具结果：立即更新对应 segment。 */
    pub async fn complete_tool_call_segment(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
        tool_call_id: &str,
        tool_name: &str,
        content: &str,
        is_error: bool,
    ) -> anyhow::Result<()> {
        let key = turn_persist_key(&session_id.0, &turn_id.0);
        let segment_id = {
            let guard = self.turn_persist.read().await;
            guard
                .get(&key)
                .and_then(|state| state.segment_by_tool_call.get(tool_call_id).cloned())
        };
        let Some(segment_id) = segment_id else {
            return Ok(());
        };
        let mut payload = BTreeMap::new();
        payload.insert("name".to_string(), json!(tool_name));
        payload.insert("toolCallId".to_string(), json!(tool_call_id));
        payload.insert("result".to_string(), json!(content));
        payload.insert("isError".to_string(), json!(is_error));
        payload.insert(
            "content".to_string(),
            json!(describe_tool_segment(tool_name)),
        );
        self.store
            .update_segment(
                &segment_id,
                Some(if is_error {
                    "error".to_string()
                } else {
                    "done".to_string()
                }),
                Some(payload),
                Some(tool_name.to_string()),
            )
            .await?;
        Ok(())
    }

    pub async fn append_tool_message(
        &self,
        session_id: &SessionId,
        turn_id: &TurnId,
        tool_call_id: String,
        name: String,
        content: String,
        metadata: BTreeMap<String, serde_json::Value>,
    ) -> anyhow::Result<()> {
        let mut meta = metadata;
        meta.entry("toolCallId".to_string())
            .or_insert_with(|| json!(tool_call_id.clone()));
        meta.entry("toolName".to_string())
            .or_insert_with(|| json!(name.clone()));
        self.require_session_record(session_id).await?;
        let mut sessions = self.sessions.write().await;
        let record = sessions
            .get_mut(&session_id.0)
            .ok_or_else(|| anyhow::anyhow!("session not found: {}", session_id.0))?;
        let now = Utc::now();
        record.info.updated_at = now;
        record.info.is_processing = true;
        record.status = ConversationSessionStatus::Active;
        drop(sessions);
        self.store
            .add_message(ConversationMessageRecord {
                id: format!("msg_{}", Uuid::new_v4()),
                session_id: session_id.0.clone(),
                role: "tool".to_string(),
                content,
                created_at: now,
                token_count: None,
                run_id: None,
                turn_id: Some(turn_id.0.clone()),
                metadata: meta,
                tool_call_id: Some(tool_call_id),
                name: Some(name),
            })
            .await?;
        Ok(())
    }

    async fn require_session_record(&self, session_id: &SessionId) -> anyhow::Result<()> {
        if self.restore_session_record(session_id).await?.is_none() {
            anyhow::bail!("session not found: {}", session_id.0);
        }
        Ok(())
    }

    async fn restore_session_record(
        &self,
        session_id: &SessionId,
    ) -> anyhow::Result<Option<SessionRecord>> {
        if let Some(record) = self.sessions.read().await.get(&session_id.0).cloned() {
            self.init_session_containers(&session_id.0, None).await;
            return Ok(Some(record));
        }
        let history = self.store.get_history(&session_id.0).await?;
        let Some(session) = history.session.clone() else {
            return Ok(None);
        };
        if session.status == ConversationSessionStatus::Deleted {
            return Ok(None);
        }
        let status = session.status;
        let info = session_info_from_record(session);
        let record = SessionRecord {
            info: info.clone(),
            status,
        };
        self.sessions
            .write()
            .await
            .insert(session_id.0.clone(), record.clone());
        self.init_session_containers(&session_id.0, None).await;
        Ok(Some(record))
    }
}

fn role_from_str(value: &str) -> Option<MessageRole> {
    match value {
        "user" => Some(MessageRole::User),
        "assistant" => Some(MessageRole::Assistant),
        "system" => Some(MessageRole::System),
        "tool" => Some(MessageRole::Tool),
        _ => None,
    }
}

fn session_info_from_record(record: ConversationSessionRecord) -> SessionInfo {
    let project_dir = metadata_string(&record.metadata, "projectDir")
        .or_else(|| metadata_string(&record.metadata, "project_dir"));
    SessionInfo {
        id: SessionId(record.id),
        name: record.title,
        created_at: record.created_at,
        updated_at: record.updated_at,
        message_count: record.message_count,
        is_processing: record.status == ConversationSessionStatus::Active,
        storage_backend: "postgres".to_string(),
        title_status: metadata_string(&record.metadata, "titleStatus"),
        title_generated_by: metadata_string(&record.metadata, "titleGeneratedBy"),
        title_generation_error: metadata_string(&record.metadata, "titleGenerationError"),
        title_provider: metadata_string(&record.metadata, "titleProvider"),
        title_model: metadata_string(&record.metadata, "titleModel"),
        pinned: record.pinned,
        project_dir,
    }
}

fn metadata_string(metadata: &BTreeMap<String, serde_json::Value>, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn chat_message_from_record(record: ConversationMessageRecord) -> Option<ChatMessage> {
    let attachments = record
        .metadata
        .get("attachments")
        .cloned()
        .and_then(|value| serde_json::from_value::<Vec<ChatAttachment>>(value).ok())
        .filter(|items| !items.is_empty());
    Some(ChatMessage {
        role: role_from_str(&record.role)?,
        content: record.content,
        timestamp: record.created_at,
        turn_id: record.turn_id.map(TurnId),
        attachments,
        id: Some(record.id),
        metadata: record.metadata,
        tool_call_id: record.tool_call_id,
        name: record.name,
        segments: None,
    })
}

fn history_to_chat_messages(history: xuanyu_storage::ConversationHistory) -> Vec<ChatMessage> {
    let mut by_message: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for segment in history.segments {
        let Some(message_id) = segment.message_id.clone() else {
            continue;
        };
        if !is_chat_visible_segment(&segment.segment_type) {
            continue;
        }
        by_message
            .entry(message_id)
            .or_default()
            .push(segment_to_client_value(&segment));
    }
    // 与参考 toHistoryPayload 一致：不把 role=tool 行作为时间线消息返回。
    history
        .messages
        .into_iter()
        .filter(|record| record.role != "tool")
        .filter_map(|record| {
            let id = record.id.clone();
            let mut message = chat_message_from_record(record)?;
            if let Some(segments) = by_message.remove(&id) {
                if !segments.is_empty() {
                    message.segments = Some(segments);
                }
            }
            Some(message)
        })
        .collect()
}

fn is_chat_visible_segment(segment_type: &str) -> bool {
    matches!(
        segment_type,
        "text"
            | "tool_call"
            | "sandbox_event"
            | "agent_run"
            | "memory_hit"
            | "permission"
            | "system_notice"
            | "task_event"
            | "task_result"
            | "agent_delegate"
    )
}

fn tool_name_to_segment_type(name: &str) -> &'static str {
    if name.starts_with("sandbox_") {
        "sandbox_event"
    } else {
        "tool_call"
    }
}

fn describe_tool_segment(name: &str) -> String {
    if name.starts_with("sandbox_") {
        "沙箱操作".to_string()
    } else {
        "工具调用".to_string()
    }
}

fn segment_to_client_value(segment: &ConversationSegmentRecord) -> Value {
    let payload = &segment.payload;
    match segment.segment_type.as_str() {
        "text" => json!({
            "type": "text",
            "id": segment.id,
            "content": payload.get("content").and_then(Value::as_str).unwrap_or(""),
            "thinking": payload.get("thinking").and_then(Value::as_str),
        }),
        "tool_call" | "sandbox_event" => {
            let name = payload
                .get("name")
                .and_then(Value::as_str)
                .or(segment.title.as_deref())
                .unwrap_or(segment.segment_type.as_str());
            let args = payload.get("args").cloned().unwrap_or_else(|| json!({}));
            let raw_args = payload
                .get("rawArgs")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| serde_json::to_string(&args).unwrap_or_default());
            // 前端/直播用 toolCallId 对齐；没有则退回 segment.id（与参考一致）。
            let id = payload
                .get("toolCallId")
                .or_else(|| payload.get("tool_call_id"))
                .and_then(Value::as_str)
                .unwrap_or(segment.id.as_str());
            json!({
                "type": segment.segment_type,
                "id": id,
                "name": name,
                "content": payload.get("content").and_then(Value::as_str)
                    .or(segment.title.as_deref())
                    .unwrap_or(""),
                "args": args,
                "rawArgs": raw_args,
                "status": segment.status.clone().unwrap_or_else(|| "done".to_string()),
                "result": payload.get("result").and_then(Value::as_str),
                "title": segment.title,
            })
        }
        _ => json!({
            "type": segment.segment_type,
            "id": segment.id,
            "status": segment.status.clone().unwrap_or_else(|| "done".to_string()),
            "title": segment.title,
            "payload": payload,
        }),
    }
}
