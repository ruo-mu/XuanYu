//! PostgreSQL conversation store (sessions / messages / segments / snapshots / memory hits).
//! Same shape as the product conversation store — no file backend.

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::RwLock;
use tokio_postgres::NoTls;

use crate::{
    ConversationHistory, ConversationMessageRecord, ConversationSegmentRecord,
    ConversationSessionPatch, ConversationSessionRecord, ConversationSessionStatus,
    ConversationSnapshotRecord, ConversationStoreHealth, MemoryHitRecord,
};

#[derive(Debug, Clone)]
pub struct PostgresConversationStoreConfig {
    pub connection_string: String,
    pub project_name: String,
    pub schema: String,
    pub table_prefix: String,
}

pub struct PostgresConversationStore {
    project_name: String,
    schema: RwLock<String>,
    prefix: RwLock<String>,
    connection_string: RwLock<Option<String>>,
    pool: RwLock<Option<Pool>>,
    ready: AtomicBool,
}

impl PostgresConversationStore {
    pub fn new(project_name: impl Into<String>) -> Self {
        Self {
            project_name: sanitize_ident(&project_name.into(), "default"),
            schema: RwLock::new("public".to_string()),
            prefix: RwLock::new("xy".to_string()),
            connection_string: RwLock::new(None),
            pool: RwLock::new(None),
            ready: AtomicBool::new(false),
        }
    }

    pub fn project_name(&self) -> &str {
        &self.project_name
    }

    pub async fn configure(&self, config: PostgresConversationStoreConfig) -> Result<()> {
        let schema = sanitize_ident(&config.schema, "public");
        let prefix = sanitize_ident(&config.table_prefix, "xy");
        let mut pg_config = Config::new();
        pg_config.url = Some(config.connection_string.clone());
        pg_config.manager = Some(ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        });
        let pool = pg_config
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .context("failed to create conversation postgres pool")?;
        {
            let client = pool.get().await.context("conversation postgres pool get")?;
            migrate(&client, &schema, &prefix).await?;
        }
        *self.schema.write().await = schema;
        *self.prefix.write().await = prefix;
        *self.connection_string.write().await = Some(config.connection_string);
        *self.pool.write().await = Some(pool);
        self.ready.store(true, Ordering::SeqCst);
        Ok(())
    }

    pub async fn initialize(&self) -> Result<()> {
        if self.ready.load(Ordering::SeqCst) {
            return Ok(());
        }
        anyhow::bail!(
            "Conversation history requires PostgreSQL. Configure memory.postgres.connectionString."
        )
    }

    async fn pool(&self) -> Result<Pool> {
        self.initialize().await?;
        self.pool
            .read()
            .await
            .clone()
            .context("conversation postgres pool missing")
    }

    async fn table(&self, suffix: &str) -> String {
        let schema = self.schema.read().await.clone();
        let prefix = self.prefix.read().await.clone();
        format!("{schema}.{prefix}_{suffix}")
    }

    pub async fn health(&self) -> ConversationStoreHealth {
        let location = self
            .connection_string
            .read()
            .await
            .clone()
            .unwrap_or_else(|| "unconfigured".to_string());
        match self.pool().await {
            Ok(pool) => match pool.get().await {
                Ok(client) => {
                    let pgvector = client
                        .query_one(
                            "SELECT EXISTS(SELECT 1 FROM pg_extension WHERE extname = 'vector') AS pgvector",
                            &[],
                        )
                        .await
                        .ok()
                        .and_then(|row| row.try_get::<_, bool>(0).ok())
                        .unwrap_or(false);
                    ConversationStoreHealth {
                        backend: "postgres".to_string(),
                        ok: true,
                        location,
                        pgvector: Some(pgvector),
                        error: None,
                    }
                }
                Err(err) => ConversationStoreHealth {
                    backend: "postgres".to_string(),
                    ok: false,
                    location,
                    pgvector: None,
                    error: Some(err.to_string()),
                },
            },
            Err(err) => ConversationStoreHealth {
                backend: "postgres".to_string(),
                ok: false,
                location,
                pgvector: None,
                error: Some(err.to_string()),
            },
        }
    }

    pub async fn create_session(
        &self,
        id: impl Into<String>,
        title: impl Into<String>,
    ) -> Result<ConversationSessionRecord> {
        let now = Utc::now();
        let session = ConversationSessionRecord {
            id: id.into(),
            project_name: self.project_name.clone(),
            title: title.into(),
            status: ConversationSessionStatus::Idle,
            provider: None,
            model: None,
            created_at: now,
            updated_at: now,
            last_message_at: None,
            message_count: 0,
            current_run_id: None,
            archived_at: None,
            pinned: false,
            tags: Vec::new(),
            metadata: BTreeMap::new(),
        };
        self.upsert_session(session.clone()).await?;
        Ok(session)
    }

    pub async fn upsert_session(&self, session: ConversationSessionRecord) -> Result<()> {
        let pool = self.pool().await?;
        let client = pool.get().await.context("pool get")?;
        let table = self.table("sessions").await;
        let status = status_to_str(session.status);
        let tags = serde_json::to_value(&session.tags)?;
        let metadata = serde_json::to_value(&session.metadata)?;
        let sql = format!(
            "INSERT INTO {table}
              (id, project_name, title, status, provider, model, created_at, updated_at,
               last_message_at, message_count, current_run_id, archived_at, pinned, tags_json, metadata_json)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)
             ON CONFLICT (id) DO UPDATE SET
               project_name = EXCLUDED.project_name,
               title = EXCLUDED.title,
               status = EXCLUDED.status,
               provider = EXCLUDED.provider,
               model = EXCLUDED.model,
               updated_at = EXCLUDED.updated_at,
               last_message_at = EXCLUDED.last_message_at,
               message_count = EXCLUDED.message_count,
               current_run_id = EXCLUDED.current_run_id,
               archived_at = EXCLUDED.archived_at,
               pinned = EXCLUDED.pinned,
               tags_json = EXCLUDED.tags_json,
               metadata_json = EXCLUDED.metadata_json"
        );
        client
            .execute(
                &sql,
                &[
                    &session.id,
                    &session.project_name,
                    &session.title,
                    &status,
                    &session.provider,
                    &session.model,
                    &dt_millis(session.created_at),
                    &dt_millis(session.updated_at),
                    &session.last_message_at.map(dt_millis),
                    &(session.message_count as i32),
                    &session.current_run_id,
                    &session.archived_at.map(dt_millis),
                    &session.pinned,
                    &tags,
                    &metadata,
                ],
            )
            .await
            .context("upsert session")?;
        Ok(())
    }

    pub async fn update_session(&self, id: &str, patch: ConversationSessionPatch) -> Result<()> {
        let Some(mut session) = self.get_session_including_deleted(id).await? else {
            return Ok(());
        };
        if let Some(title) = patch.title {
            session.title = title;
        }
        if let Some(status) = patch.status {
            session.status = status;
        }
        if let Some(message_count) = patch.message_count {
            session.message_count = message_count;
        }
        if let Some(last_message_at) = patch.last_message_at {
            session.last_message_at = Some(last_message_at);
        }
        if let Some(current_run_id) = patch.current_run_id {
            session.current_run_id = current_run_id;
        }
        if let Some(archived_at) = patch.archived_at {
            session.archived_at = archived_at;
        }
        if let Some(pinned) = patch.pinned {
            session.pinned = pinned;
        }
        if let Some(metadata) = patch.metadata {
            for (key, value) in metadata {
                session.metadata.insert(key, value);
            }
        }
        session.updated_at = patch.updated_at.unwrap_or_else(Utc::now);
        self.upsert_session(session).await
    }

    pub async fn archive_session(&self, id: &str, archived: bool) -> Result<()> {
        let now = Utc::now();
        self.update_session(
            id,
            ConversationSessionPatch {
                status: Some(if archived {
                    ConversationSessionStatus::Archived
                } else {
                    ConversationSessionStatus::Idle
                }),
                archived_at: Some(archived.then_some(now)),
                updated_at: Some(now),
                ..Default::default()
            },
        )
        .await
    }

    pub async fn delete_session(&self, id: &str) -> Result<()> {
        self.update_session(
            id,
            ConversationSessionPatch {
                status: Some(ConversationSessionStatus::Deleted),
                updated_at: Some(Utc::now()),
                ..Default::default()
            },
        )
        .await
    }

    async fn get_session_including_deleted(
        &self,
        id: &str,
    ) -> Result<Option<ConversationSessionRecord>> {
        let pool = self.pool().await?;
        let client = pool.get().await.context("pool get")?;
        let table = self.table("sessions").await;
        let sql = format!(
            "SELECT id, project_name, title, status, provider, model, created_at, updated_at,
                    last_message_at, message_count, current_run_id, archived_at, pinned, tags_json, metadata_json
             FROM {table} WHERE id = $1"
        );
        let rows = client.query(&sql, &[&id]).await.context("get session")?;
        rows.first().map(row_to_session).transpose()
    }

    pub async fn get_session(&self, id: &str) -> Result<Option<ConversationSessionRecord>> {
        Ok(self
            .get_session_including_deleted(id)
            .await?
            .filter(|session| session.status != ConversationSessionStatus::Deleted))
    }

    pub async fn list_sessions(
        &self,
        include_archived: bool,
        limit: usize,
    ) -> Result<Vec<ConversationSessionRecord>> {
        self.query_sessions(include_archived, None, limit).await
    }

    pub async fn query_sessions(
        &self,
        include_archived: bool,
        search: Option<&str>,
        limit: usize,
    ) -> Result<Vec<ConversationSessionRecord>> {
        let pool = self.pool().await?;
        let client = pool.get().await.context("pool get")?;
        let table = self.table("sessions").await;
        let limit = limit.max(1) as i64;
        let search = search.map(str::trim).filter(|q| !q.is_empty());
        let sql = if include_archived {
            format!(
                "SELECT id, project_name, title, status, provider, model, created_at, updated_at,
                        last_message_at, message_count, current_run_id, archived_at, pinned, tags_json, metadata_json
                 FROM {table}
                 WHERE project_name = $1 AND status <> 'deleted'
                   AND ($2::text IS NULL OR lower(title) LIKE '%' || lower($2) || '%')
                 ORDER BY COALESCE(last_message_at, updated_at) DESC
                 LIMIT $3"
            )
        } else {
            format!(
                "SELECT id, project_name, title, status, provider, model, created_at, updated_at,
                        last_message_at, message_count, current_run_id, archived_at, pinned, tags_json, metadata_json
                 FROM {table}
                 WHERE project_name = $1 AND status <> 'deleted' AND status <> 'archived'
                   AND ($2::text IS NULL OR lower(title) LIKE '%' || lower($2) || '%')
                 ORDER BY COALESCE(last_message_at, updated_at) DESC
                 LIMIT $3"
            )
        };
        let rows = client
            .query(&sql, &[&self.project_name, &search, &limit])
            .await
            .context("query sessions")?;
        rows.iter().map(row_to_session).collect()
    }

    pub async fn add_message(&self, message: ConversationMessageRecord) -> Result<()> {
        let pool = self.pool().await?;
        let client = pool.get().await.context("pool get")?;
        let table = self.table("conversation_messages").await;
        let metadata = serde_json::to_value(&message.metadata)?;
        let sql = format!(
            "INSERT INTO {table}
              (id, session_id, role, content, created_at, token_count, run_id, turn_id, metadata_json, tool_call_id, name)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             ON CONFLICT (id) DO UPDATE SET
               role = EXCLUDED.role,
               content = EXCLUDED.content,
               created_at = EXCLUDED.created_at,
               token_count = EXCLUDED.token_count,
               run_id = EXCLUDED.run_id,
               turn_id = EXCLUDED.turn_id,
               metadata_json = EXCLUDED.metadata_json,
               tool_call_id = EXCLUDED.tool_call_id,
               name = EXCLUDED.name"
        );
        let inserted = client
            .execute(
                &sql,
                &[
                    &message.id,
                    &message.session_id,
                    &message.role,
                    &message.content,
                    &dt_millis(message.created_at),
                    &message.token_count.map(|v| v as i64),
                    &message.run_id,
                    &message.turn_id,
                    &metadata,
                    &message.tool_call_id,
                    &message.name,
                ],
            )
            .await
            .context("add message")?;
        self.bump_session_counters_on_insert(&message, inserted > 0)
            .await?;
        Ok(())
    }

    async fn bump_session_counters_on_insert(
        &self,
        message: &ConversationMessageRecord,
        inserted: bool,
    ) -> Result<()> {
        let pool = self.pool().await?;
        let client = pool.get().await.context("pool get")?;
        let table = self.table("sessions").await;
        let visible = message.role == "user" || message.role == "assistant";
        let now = dt_millis(Utc::now());
        let created = dt_millis(message.created_at);
        if inserted {
            let delta: i32 = if visible { 1 } else { 0 };
            let sql = format!(
                "UPDATE {table}
                 SET message_count = message_count + $2,
                     last_message_at = GREATEST(COALESCE(last_message_at, 0), $3),
                     updated_at = $4
                 WHERE id = $1"
            );
            client
                .execute(&sql, &[&message.session_id, &delta, &created, &now])
                .await?;
        } else {
            let sql = format!(
                "UPDATE {table}
                 SET last_message_at = GREATEST(COALESCE(last_message_at, 0), $2),
                     updated_at = $3
                 WHERE id = $1"
            );
            client
                .execute(&sql, &[&message.session_id, &created, &now])
                .await?;
        }
        Ok(())
    }

    pub async fn list_messages(&self, session_id: &str) -> Result<Vec<ConversationMessageRecord>> {
        let pool = self.pool().await?;
        let client = pool.get().await.context("pool get")?;
        let table = self.table("conversation_messages").await;
        let sql = format!(
            "SELECT id, session_id, role, content, created_at, token_count, run_id, turn_id,
                    metadata_json, tool_call_id, name
             FROM {table} WHERE session_id = $1 ORDER BY created_at ASC"
        );
        let rows = client.query(&sql, &[&session_id]).await?;
        rows.iter().map(row_to_message).collect()
    }

    pub async fn replace_messages(
        &self,
        session_id: &str,
        mut messages: Vec<ConversationMessageRecord>,
    ) -> Result<()> {
        messages.sort_by_key(|a| a.created_at);
        let pool = self.pool().await?;
        let mut client = pool.get().await.context("pool get")?;
        let tx = client.transaction().await?;
        let table = self.table("conversation_messages").await;
        let sessions = self.table("sessions").await;
        tx.execute(
            &format!("DELETE FROM {table} WHERE session_id = $1"),
            &[&session_id],
        )
        .await?;
        for message in &messages {
            let metadata = serde_json::to_value(&message.metadata)?;
            let sql = format!(
                "INSERT INTO {table}
                  (id, session_id, role, content, created_at, token_count, run_id, turn_id, metadata_json, tool_call_id, name)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"
            );
            tx.execute(
                &sql,
                &[
                    &message.id,
                    &message.session_id,
                    &message.role,
                    &message.content,
                    &dt_millis(message.created_at),
                    &message.token_count.map(|v| v as i64),
                    &message.run_id,
                    &message.turn_id,
                    &metadata,
                    &message.tool_call_id,
                    &message.name,
                ],
            )
            .await?;
        }
        let visible = messages
            .iter()
            .filter(|m| m.role == "user" || m.role == "assistant")
            .count() as i32;
        let last = messages.last().map(|m| dt_millis(m.created_at));
        let now = dt_millis(Utc::now());
        tx.execute(
            &format!(
                "UPDATE {sessions}
                 SET message_count = $2, last_message_at = $3, updated_at = $4
                 WHERE id = $1"
            ),
            &[&session_id, &visible, &last, &now],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn add_segment(&self, segment: ConversationSegmentRecord) -> Result<()> {
        self.add_segments_batch(std::slice::from_ref(&segment)).await
    }

    /** 单事务批量写入/更新 turn_segments，避免回合内逐条往返。 */
    pub async fn add_segments_batch(&self, segments: &[ConversationSegmentRecord]) -> Result<()> {
        if segments.is_empty() {
            return Ok(());
        }
        let pool = self.pool().await?;
        let mut client = pool.get().await.context("pool get")?;
        let tx = client.transaction().await?;
        let table = self.table("turn_segments").await;
        let sql = format!(
            "INSERT INTO {table}
              (id, session_id, sequence_number, message_id, turn_id, type, status, title, payload_json, created_at, updated_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
             ON CONFLICT (id) DO UPDATE SET
               sequence_number = EXCLUDED.sequence_number,
               message_id = EXCLUDED.message_id,
               turn_id = EXCLUDED.turn_id,
               type = EXCLUDED.type,
               status = EXCLUDED.status,
               title = EXCLUDED.title,
               payload_json = EXCLUDED.payload_json,
               updated_at = EXCLUDED.updated_at"
        );
        for segment in segments {
            let payload = serde_json::to_value(&segment.payload)?;
            tx.execute(
                &sql,
                &[
                    &segment.id,
                    &segment.session_id,
                    &(segment.sequence_number as i32),
                    &segment.message_id,
                    &segment.turn_id,
                    &segment.segment_type,
                    &segment.status,
                    &segment.title,
                    &payload,
                    &dt_millis(segment.created_at),
                    &dt_millis(segment.updated_at),
                ],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /** 预分配连续 sequence，供批写使用。 */
    pub async fn reserve_segment_sequences(&self, session_id: &str, count: usize) -> Result<u64> {
        if count == 0 {
            return self.next_segment_sequence(session_id).await;
        }
        let pool = self.pool().await?;
        let client = pool.get().await.context("pool get")?;
        let table = self.table("turn_segments").await;
        let sql = format!(
            "SELECT COALESCE(MAX(sequence_number), 0) + 1 AS next_seq
             FROM {table}
             WHERE session_id = $1"
        );
        let row = client.query_one(&sql, &[&session_id]).await?;
        let next: i32 = row.get("next_seq");
        Ok(next.max(1) as u64)
    }

    pub async fn last_message_pointer(
        &self,
        session_id: &str,
    ) -> Result<(i32, Option<String>)> {
        let pool = self.pool().await?;
        let client = pool.get().await.context("pool get")?;
        let messages = self.table("conversation_messages").await;
        let sessions = self.table("sessions").await;
        let count_row = client
            .query_opt(
                &format!(
                    "SELECT message_count FROM {sessions} WHERE id = $1"
                ),
                &[&session_id],
            )
            .await?;
        let message_count: i32 = count_row
            .map(|row| row.get::<_, i32>("message_count"))
            .unwrap_or(0);
        let last_id: Option<String> = client
            .query_opt(
                &format!(
                    "SELECT id FROM {messages}
                     WHERE session_id = $1
                     ORDER BY created_at DESC
                     LIMIT 1"
                ),
                &[&session_id],
            )
            .await?
            .map(|row| row.get("id"));
        Ok((message_count, last_id))
    }

    pub async fn update_segment(
        &self,
        segment_id: &str,
        status: Option<String>,
        payload: Option<BTreeMap<String, serde_json::Value>>,
        title: Option<String>,
    ) -> Result<()> {
        let pool = self.pool().await?;
        let client = pool.get().await.context("pool get")?;
        let table = self.table("turn_segments").await;
        let now = dt_millis(Utc::now());
        if let Some(payload) = payload {
            let payload_json = serde_json::to_value(&payload)?;
            // 与参考一致：payload 用 jsonb || 合并，避免 tool_result 覆盖掉 args/rawArgs。
            let sql = format!(
                "UPDATE {table}
                 SET status = COALESCE($2, status),
                     title = COALESCE($3, title),
                     payload_json = COALESCE(payload_json, '{{}}'::jsonb) || $4::jsonb,
                     updated_at = $5
                 WHERE id = $1"
            );
            client
                .execute(
                    &sql,
                    &[&segment_id, &status, &title, &payload_json, &now],
                )
                .await?;
        } else {
            let sql = format!(
                "UPDATE {table}
                 SET status = COALESCE($2, status),
                     title = COALESCE($3, title),
                     updated_at = $4
                 WHERE id = $1"
            );
            client
                .execute(&sql, &[&segment_id, &status, &title, &now])
                .await?;
        }
        Ok(())
    }

    pub async fn next_segment_sequence(&self, session_id: &str) -> Result<u64> {
        let pool = self.pool().await?;
        let client = pool.get().await.context("pool get")?;
        let table = self.table("turn_segments").await;
        let sql = format!(
            "SELECT COALESCE(MAX(sequence_number), 0) + 1 AS next_seq
             FROM {table}
             WHERE session_id = $1"
        );
        let row = client.query_one(&sql, &[&session_id]).await?;
        let next: i32 = row.get("next_seq");
        Ok(next.max(1) as u64)
    }

    pub async fn list_segments(&self, session_id: &str) -> Result<Vec<ConversationSegmentRecord>> {
        let pool = self.pool().await?;
        let client = pool.get().await.context("pool get")?;
        let table = self.table("turn_segments").await;
        let sql = format!(
            "SELECT id, session_id, sequence_number, message_id, turn_id, type, status, title,
                    payload_json, created_at, updated_at
             FROM {table}
             WHERE session_id = $1
             ORDER BY sequence_number ASC, created_at ASC"
        );
        let rows = client.query(&sql, &[&session_id]).await?;
        rows.iter().map(row_to_segment).collect()
    }

    pub async fn replace_segments(
        &self,
        session_id: &str,
        mut segments: Vec<ConversationSegmentRecord>,
    ) -> Result<()> {
        segments.sort_by_key(|a| a.sequence_number);
        let pool = self.pool().await?;
        let mut client = pool.get().await.context("pool get")?;
        let tx = client.transaction().await?;
        let table = self.table("turn_segments").await;
        tx.execute(
            &format!("DELETE FROM {table} WHERE session_id = $1"),
            &[&session_id],
        )
        .await?;
        for segment in segments {
            let payload = serde_json::to_value(&segment.payload)?;
            let sql = format!(
                "INSERT INTO {table}
                  (id, session_id, sequence_number, message_id, turn_id, type, status, title, payload_json, created_at, updated_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)"
            );
            tx.execute(
                &sql,
                &[
                    &segment.id,
                    &segment.session_id,
                    &(segment.sequence_number as i32),
                    &segment.message_id,
                    &segment.turn_id,
                    &segment.segment_type,
                    &segment.status,
                    &segment.title,
                    &payload,
                    &dt_millis(segment.created_at),
                    &dt_millis(segment.updated_at),
                ],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn add_snapshot(&self, snapshot: ConversationSnapshotRecord) -> Result<()> {
        let pool = self.pool().await?;
        let client = pool.get().await.context("pool get")?;
        let table = self.table("session_snapshots").await;
        let conversation = snapshot.conversation.clone();
        let budget = snapshot.context_budget.clone();
        let sql = format!(
            "INSERT INTO {table}
              (id, session_id, run_id, checkpoint_id, summary, conversation_json, context_budget_json, created_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
             ON CONFLICT (id) DO UPDATE SET
               run_id = EXCLUDED.run_id,
               checkpoint_id = EXCLUDED.checkpoint_id,
               summary = EXCLUDED.summary,
               conversation_json = EXCLUDED.conversation_json,
               context_budget_json = EXCLUDED.context_budget_json,
               created_at = EXCLUDED.created_at"
        );
        client
            .execute(
                &sql,
                &[
                    &snapshot.id,
                    &snapshot.session_id,
                    &snapshot.run_id,
                    &snapshot.checkpoint_id,
                    &snapshot.summary,
                    &conversation,
                    &budget,
                    &dt_millis(snapshot.created_at),
                ],
            )
            .await?;
        Ok(())
    }

    pub async fn list_snapshots(
        &self,
        session_id: &str,
    ) -> Result<Vec<ConversationSnapshotRecord>> {
        let pool = self.pool().await?;
        let client = pool.get().await.context("pool get")?;
        let table = self.table("session_snapshots").await;
        let sql = format!(
            "SELECT id, session_id, run_id, checkpoint_id, summary, conversation_json, context_budget_json, created_at
             FROM {table} WHERE session_id = $1 ORDER BY created_at ASC"
        );
        let rows = client.query(&sql, &[&session_id]).await?;
        rows.iter().map(row_to_snapshot).collect()
    }

    pub async fn delete_snapshots_for_turns(
        &self,
        session_id: &str,
        turn_ids: &[String],
    ) -> Result<()> {
        if turn_ids.is_empty() {
            return Ok(());
        }
        let pool = self.pool().await?;
        let client = pool.get().await.context("pool get")?;
        let table = self.table("session_snapshots").await;
        let sql = format!(
            "DELETE FROM {table}
             WHERE session_id = $1 AND checkpoint_id = ANY($2)"
        );
        client.execute(&sql, &[&session_id, &turn_ids]).await?;
        Ok(())
    }

    pub async fn add_memory_hits(&self, hits: Vec<MemoryHitRecord>) -> Result<()> {
        if hits.is_empty() {
            return Ok(());
        }
        let pool = self.pool().await?;
        let client = pool.get().await.context("pool get")?;
        let table = self.table("memory_hits").await;
        for hit in hits {
            let sql = format!(
                "INSERT INTO {table}
                  (id, session_id, run_id, turn_id, query, source, memory_id, score, category, injected, created_at)
                 VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)
                 ON CONFLICT (id) DO UPDATE SET
                   run_id = EXCLUDED.run_id,
                   turn_id = EXCLUDED.turn_id,
                   query = EXCLUDED.query,
                   source = EXCLUDED.source,
                   memory_id = EXCLUDED.memory_id,
                   score = EXCLUDED.score,
                   category = EXCLUDED.category,
                   injected = EXCLUDED.injected,
                   created_at = EXCLUDED.created_at"
            );
            client
                .execute(
                    &sql,
                    &[
                        &hit.id,
                        &hit.session_id,
                        &hit.run_id,
                        &hit.turn_id,
                        &hit.query,
                        &hit.source,
                        &hit.memory_id,
                        &hit.score.map(|v| v as f64),
                        &hit.category,
                        &hit.injected,
                        &dt_millis(hit.created_at),
                    ],
                )
                .await?;
        }
        Ok(())
    }

    pub async fn list_memory_hits(
        &self,
        session_id: &str,
        run_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MemoryHitRecord>> {
        let pool = self.pool().await?;
        let client = pool.get().await.context("pool get")?;
        let table = self.table("memory_hits").await;
        let limit = limit.clamp(1, 500) as i64;
        let sql = format!(
            "SELECT id, session_id, run_id, turn_id, query, source, memory_id, score, category, injected, created_at
             FROM {table}
             WHERE session_id = $1 AND ($2::text IS NULL OR run_id = $2)
             ORDER BY created_at DESC
             LIMIT $3"
        );
        let rows = client
            .query(&sql, &[&session_id, &run_id, &limit])
            .await?;
        rows.iter().map(row_to_memory_hit).collect()
    }

    pub async fn get_history(&self, session_id: &str) -> Result<ConversationHistory> {
        let session = self.get_session_including_deleted(session_id).await?;
        let messages = self.list_messages(session_id).await?;
        let segments = self.list_segments(session_id).await?;
        let memory_hits = self.list_memory_hits(session_id, None, 500).await?;
        let latest_snapshot = self
            .list_snapshots(session_id)
            .await?
            .into_iter()
            .max_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(ConversationHistory {
            session,
            messages,
            segments,
            memory_hits,
            latest_snapshot,
        })
    }
}

async fn migrate(
    client: &deadpool_postgres::Client,
    schema: &str,
    prefix: &str,
) -> Result<()> {
    let sessions = format!("{schema}.{prefix}_sessions");
    let messages = format!("{schema}.{prefix}_conversation_messages");
    let segments = format!("{schema}.{prefix}_turn_segments");
    let snapshots = format!("{schema}.{prefix}_session_snapshots");
    let hits = format!("{schema}.{prefix}_memory_hits");
    let sql = format!(
        r#"
        CREATE SCHEMA IF NOT EXISTS {schema};
        CREATE TABLE IF NOT EXISTS {sessions} (
          id TEXT PRIMARY KEY,
          project_name TEXT NOT NULL,
          title TEXT NOT NULL,
          status TEXT NOT NULL,
          provider TEXT,
          model TEXT,
          created_at BIGINT NOT NULL,
          updated_at BIGINT NOT NULL,
          last_message_at BIGINT,
          message_count INTEGER NOT NULL DEFAULT 0,
          current_run_id TEXT,
          archived_at BIGINT,
          pinned BOOLEAN DEFAULT FALSE,
          tags_json JSONB,
          metadata_json JSONB
        );
        CREATE TABLE IF NOT EXISTS {messages} (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          role TEXT NOT NULL,
          content TEXT NOT NULL,
          created_at BIGINT NOT NULL,
          token_count BIGINT,
          run_id TEXT,
          turn_id TEXT,
          metadata_json JSONB,
          tool_call_id TEXT,
          name TEXT
        );
        CREATE TABLE IF NOT EXISTS {segments} (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          sequence_number INTEGER NOT NULL DEFAULT 0,
          message_id TEXT,
          turn_id TEXT,
          type TEXT NOT NULL,
          status TEXT,
          title TEXT,
          payload_json JSONB NOT NULL,
          created_at BIGINT NOT NULL,
          updated_at BIGINT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS {snapshots} (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          run_id TEXT,
          checkpoint_id TEXT,
          summary TEXT,
          conversation_json JSONB NOT NULL,
          context_budget_json JSONB,
          created_at BIGINT NOT NULL
        );
        CREATE TABLE IF NOT EXISTS {hits} (
          id TEXT PRIMARY KEY,
          session_id TEXT NOT NULL,
          run_id TEXT,
          turn_id TEXT,
          query TEXT,
          source TEXT NOT NULL,
          memory_id TEXT NOT NULL,
          score DOUBLE PRECISION,
          category TEXT,
          injected BOOLEAN NOT NULL,
          created_at BIGINT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS {prefix}_sessions_project_idx ON {sessions}(project_name, updated_at DESC);
        CREATE INDEX IF NOT EXISTS {prefix}_messages_session_idx ON {messages}(session_id, created_at ASC);
        CREATE INDEX IF NOT EXISTS {prefix}_segments_session_seq_idx ON {segments}(session_id, sequence_number ASC, created_at ASC);
        CREATE INDEX IF NOT EXISTS {prefix}_memory_hits_session_idx ON {hits}(session_id, created_at DESC);
        "#
    );
    client
        .batch_execute(&sql)
        .await
        .context("conversation store migrate")?;
    Ok(())
}

fn sanitize_ident(value: &str, fallback: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let trimmed = sanitized.trim_matches('_');
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.chars().take(63).collect()
    }
}

fn dt_millis(value: DateTime<Utc>) -> i64 {
    value.timestamp_millis()
}

fn millis_to_dt(value: i64) -> DateTime<Utc> {
    Utc.timestamp_millis_opt(value)
        .single()
        .unwrap_or_else(Utc::now)
}

fn status_to_str(status: ConversationSessionStatus) -> &'static str {
    match status {
        ConversationSessionStatus::Active => "active",
        ConversationSessionStatus::Idle => "idle",
        ConversationSessionStatus::Archived => "archived",
        ConversationSessionStatus::Deleted => "deleted",
    }
}

fn status_from_str(value: &str) -> ConversationSessionStatus {
    match value {
        "active" => ConversationSessionStatus::Active,
        "archived" => ConversationSessionStatus::Archived,
        "deleted" => ConversationSessionStatus::Deleted,
        _ => ConversationSessionStatus::Idle,
    }
}

fn parse_json_map(value: Option<Value>) -> BTreeMap<String, Value> {
    match value {
        Some(Value::Object(map)) => map.into_iter().collect(),
        _ => BTreeMap::new(),
    }
}

fn parse_json_vec(value: Option<Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .into_iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

fn row_to_session(row: &tokio_postgres::Row) -> Result<ConversationSessionRecord> {
    Ok(ConversationSessionRecord {
        id: row.get(0),
        project_name: row.get(1),
        title: row.get(2),
        status: status_from_str(row.get::<_, String>(3).as_str()),
        provider: row.get(4),
        model: row.get(5),
        created_at: millis_to_dt(row.get(6)),
        updated_at: millis_to_dt(row.get(7)),
        last_message_at: row.get::<_, Option<i64>>(8).map(millis_to_dt),
        message_count: row.get::<_, i32>(9).max(0) as usize,
        current_run_id: row.get(10),
        archived_at: row.get::<_, Option<i64>>(11).map(millis_to_dt),
        pinned: row.get::<_, Option<bool>>(12).unwrap_or(false),
        tags: parse_json_vec(row.get(13)),
        metadata: parse_json_map(row.get(14)),
    })
}

fn row_to_message(row: &tokio_postgres::Row) -> Result<ConversationMessageRecord> {
    Ok(ConversationMessageRecord {
        id: row.get(0),
        session_id: row.get(1),
        role: row.get(2),
        content: row.get(3),
        created_at: millis_to_dt(row.get(4)),
        token_count: row.get::<_, Option<i64>>(5).map(|v| v.max(0) as u64),
        run_id: row.get(6),
        turn_id: row.get(7),
        metadata: parse_json_map(row.get(8)),
        tool_call_id: row.get(9),
        name: row.get(10),
    })
}

fn row_to_segment(row: &tokio_postgres::Row) -> Result<ConversationSegmentRecord> {
    Ok(ConversationSegmentRecord {
        id: row.get(0),
        session_id: row.get(1),
        sequence_number: row.get::<_, i32>(2).max(0) as u64,
        message_id: row.get(3),
        turn_id: row.get(4),
        segment_type: row.get(5),
        status: row.get(6),
        title: row.get(7),
        payload: parse_json_map(row.get(8)),
        created_at: millis_to_dt(row.get(9)),
        updated_at: millis_to_dt(row.get(10)),
    })
}

fn row_to_snapshot(row: &tokio_postgres::Row) -> Result<ConversationSnapshotRecord> {
    Ok(ConversationSnapshotRecord {
        id: row.get(0),
        session_id: row.get(1),
        run_id: row.get(2),
        checkpoint_id: row.get(3),
        summary: row.get(4),
        conversation: row.get::<_, Option<Value>>(5).unwrap_or(Value::Null),
        context_budget: row.get(6),
        created_at: millis_to_dt(row.get(7)),
    })
}

fn row_to_memory_hit(row: &tokio_postgres::Row) -> Result<MemoryHitRecord> {
    Ok(MemoryHitRecord {
        id: row.get(0),
        session_id: row.get(1),
        run_id: row.get(2),
        turn_id: row.get(3),
        query: row.get(4),
        source: row.get(5),
        memory_id: row.get(6),
        score: row.get::<_, Option<f64>>(7).map(|v| v as f32),
        category: row.get(8),
        injected: row.get(9),
        created_at: millis_to_dt(row.get(10)),
    })
}
