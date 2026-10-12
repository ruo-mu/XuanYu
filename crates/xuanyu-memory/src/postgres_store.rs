use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use deadpool_postgres::{Config, ManagerConfig, Pool, RecyclingMethod, Runtime};
use serde_json::Value;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio_postgres::NoTls;

use crate::{kind_for_category, MemoryEntry, MemorySearchQuery, MemorySearchResult, NewMemoryEntry, SessionSummary};

/// Postgres + pgvector memory backend — port of `memory/storage/postgresStore.ts`.
#[derive(Debug, Clone)]
pub struct PostgresMemoryConfig {
    pub connection_string: String,
    pub schema: String,
    pub table_prefix: String,
    pub vector_dim: usize,
    pub project_name: String,
}

pub struct PostgresMemoryStore {
    config: PostgresMemoryConfig,
    pool: Pool,
    vector_failures: AtomicU32,
}

impl PostgresMemoryStore {
    pub async fn open(config: PostgresMemoryConfig) -> Result<Self> {
        let mut pg_config = Config::new();
        pg_config.url = Some(config.connection_string.clone());
        pg_config.manager = Some(ManagerConfig {
            recycling_method: RecyclingMethod::Fast,
        });
        let pool = pg_config
            .create_pool(Some(Runtime::Tokio1), NoTls)
            .context("failed to create postgres memory pool")?;
        let store = Self {
            config,
            pool,
            vector_failures: AtomicU32::new(0),
        };
        store.migrate().await?;
        Ok(store)
    }

    fn table(&self, suffix: &str) -> String {
        format!(
            "{}.{}_{}",
            sql_identifier(&self.config.schema, "public"),
            sql_identifier(&self.config.table_prefix, "xuanyu"),
            suffix
        )
    }

    async fn migrate(&self) -> Result<()> {
        let client = self.pool.get().await.context("postgres pool get")?;
        let memories = self.table("memories");
        let summaries = self.table("memory_summaries");
        let dim = self.config.vector_dim.max(8);
        let schema = sql_identifier(&self.config.schema, "public");
        let prefix = sql_identifier(&self.config.table_prefix, "xuanyu");
        let sql = format!(
            r#"
            CREATE EXTENSION IF NOT EXISTS vector;
            CREATE SCHEMA IF NOT EXISTS {schema};
            CREATE TABLE IF NOT EXISTS {memories} (
                id TEXT PRIMARY KEY,
                content TEXT NOT NULL,
                category TEXT NOT NULL,
                importance REAL NOT NULL DEFAULT 0.5,
                tags TEXT[] NOT NULL DEFAULT '{{}}',
                metadata JSONB NOT NULL DEFAULT '{{}}'::jsonb,
                embedding vector({dim}),
                created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
                access_count BIGINT NOT NULL DEFAULT 0
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_memories_category ON {memories}(category);
            CREATE INDEX IF NOT EXISTS idx_{prefix}_memories_fts ON {memories} USING gin (to_tsvector('simple', content));
            CREATE TABLE IF NOT EXISTS {summaries} (
                session_id TEXT PRIMARY KEY,
                project_name TEXT NOT NULL,
                started_at BIGINT NOT NULL,
                ended_at BIGINT,
                key_decisions TEXT[] NOT NULL DEFAULT '{{}}',
                files_modified TEXT[] NOT NULL DEFAULT '{{}}',
                tools_used TEXT[] NOT NULL DEFAULT '{{}}',
                summary TEXT,
                token_count INTEGER,
                metadata JSONB NOT NULL DEFAULT '{{}}'::jsonb
            );
            CREATE INDEX IF NOT EXISTS idx_{prefix}_summaries_project ON {summaries}(project_name);
            "#,
        );
        client
            .batch_execute(&sql)
            .await
            .context("postgres memory migration failed")?;
        Ok(())
    }

    pub async fn upsert(
        &self,
        entry: &NewMemoryEntry,
        id: &str,
        embedding: Option<&[f32]>,
    ) -> Result<()> {
        let client = self.pool.get().await.context("postgres pool get")?;
        let memories = self.table("memories");
        let vector_sql = embedding.map(vector_literal).unwrap_or_else(|| "NULL".to_string());
        let sql = format!(
            "INSERT INTO {memories} (id, content, category, importance, tags, embedding, updated_at)
             VALUES ($1, $2, $3, $4, $5, {vector_sql}::vector, NOW())
             ON CONFLICT (id) DO UPDATE SET
               content = EXCLUDED.content,
               category = EXCLUDED.category,
               importance = EXCLUDED.importance,
               tags = EXCLUDED.tags,
               embedding = EXCLUDED.embedding,
               updated_at = NOW()",
            memories = memories,
            vector_sql = vector_sql,
        );
        client
            .execute(
                &sql,
                &[
                    &id,
                    &entry.content,
                    &entry.category,
                    &entry.importance,
                    &entry.tags,
                ],
            )
            .await
            .context("postgres memory upsert failed")?;
        Ok(())
    }

    pub async fn delete(&self, id: &str) -> Result<()> {
        let client = self.pool.get().await.context("postgres pool get")?;
        let memories = self.table("memories");
        let sql = format!("DELETE FROM {memories} WHERE id = $1");
        client
            .execute(&sql, &[&id])
            .await
            .context("postgres memory delete failed")?;
        Ok(())
    }

    pub async fn load_all(&self) -> Result<Vec<MemoryEntry>> {
        let client = self.pool.get().await.context("postgres pool get")?;
        let memories = self.table("memories");
        let sql = format!(
            "SELECT id, content, category, importance, tags, created_at, updated_at, access_count
             FROM {memories}
             ORDER BY updated_at DESC"
        );
        let rows = client.query(&sql, &[]).await.context("postgres memory load failed")?;
        Ok(rows
            .into_iter()
            .filter_map(|row| parse_memory_row(&row))
            .collect())
    }

    pub async fn upsert_summary(&self, summary: &SessionSummary) -> Result<()> {
        let client = self.pool.get().await.context("postgres pool get")?;
        let summaries = self.table("memory_summaries");
        let sql = format!(
            "INSERT INTO {summaries} (
                session_id, project_name, started_at, ended_at, key_decisions,
                files_modified, tools_used, summary, token_count, metadata
             ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, '{{}}'::jsonb)
             ON CONFLICT (session_id) DO UPDATE SET
                project_name = EXCLUDED.project_name,
                started_at = EXCLUDED.started_at,
                ended_at = EXCLUDED.ended_at,
                key_decisions = EXCLUDED.key_decisions,
                files_modified = EXCLUDED.files_modified,
                tools_used = EXCLUDED.tools_used,
                summary = EXCLUDED.summary,
                token_count = EXCLUDED.token_count"
        );
        client
            .execute(
                &sql,
                &[
                    &summary.session_id,
                    &self.config.project_name,
                    &summary.started_at,
                    &summary.ended_at,
                    &summary.key_decisions,
                    &summary.files_modified,
                    &summary.tools_used,
                    &summary.summary,
                    &summary
                        .token_count
                        .map(|count| count as i32),
                ],
            )
            .await
            .context("postgres session summary upsert failed")?;
        Ok(())
    }

    pub async fn load_summaries(&self) -> Result<Vec<SessionSummary>> {
        let client = self.pool.get().await.context("postgres pool get")?;
        let summaries = self.table("memory_summaries");
        let sql = format!(
            "SELECT session_id, started_at, ended_at, key_decisions, files_modified, tools_used, summary, token_count
             FROM {summaries}
             WHERE project_name = $1
             ORDER BY started_at DESC",
        );
        let rows = client
            .query(&sql, &[&self.config.project_name])
            .await
            .context("postgres session summary load failed")?;
        Ok(rows
            .into_iter()
            .map(|row| SessionSummary {
                session_id: row.get(0),
                started_at: row.get(1),
                ended_at: row.get(2),
                key_decisions: row.get(3),
                files_modified: row.get(4),
                tools_used: row.get(5),
                summary: row.get(6),
                token_count: row.get::<_, Option<i32>>(7).map(|count| count.max(0) as u32),
            })
            .collect())
    }

    pub async fn search_vector(
        &self,
        query: &MemorySearchQuery,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<MemorySearchResult>> {
        if self.vector_disabled() {
            return self.search_fts(query, limit).await;
        }
        let client = self.pool.get().await.context("postgres pool get")?;
        let memories = self.table("memories");
        let vector = vector_literal(embedding);
        let sql = format!(
            "SELECT id, content, category, importance, 1 - (embedding <=> {vector}::vector) AS score
             FROM {memories}
             WHERE embedding IS NOT NULL
             ORDER BY embedding <=> {vector}::vector
             LIMIT $1",
            memories = memories,
            vector = vector,
        );
        match client.query(&sql, &[&(limit.max(1) as i64)]).await {
            Ok(rows) => {
                self.vector_failures.store(0, Ordering::Relaxed);
                Ok(rows
                    .into_iter()
                    .filter_map(|row| parse_search_row(&row, query, "postgres_vector"))
                    .collect())
            }
            Err(_) => {
                self.vector_failures.fetch_add(1, Ordering::Relaxed);
                self.search_fts(query, limit).await
            }
        }
    }

    /// Keyword search via Postgres GIN FTS — legacy `to_tsvector('simple', content)`.
    pub async fn search_fts(
        &self,
        query: &MemorySearchQuery,
        limit: usize,
    ) -> Result<Vec<MemorySearchResult>> {
        let Some(text) = query.text.as_ref().filter(|value| !value.trim().is_empty()) else {
            return Ok(Vec::new());
        };
        let client = self.pool.get().await.context("postgres pool get")?;
        let memories = self.table("memories");
        let sql = format!(
            "SELECT id, content, category, importance,
                    ts_rank(to_tsvector('simple', content), plainto_tsquery('simple', $1)) AS score
             FROM {memories}
             WHERE to_tsvector('simple', content) @@ plainto_tsquery('simple', $1)
             ORDER BY score DESC
             LIMIT $2",
            memories = memories,
        );
        let rows = client
            .query(&sql, &[text, &(limit.max(1) as i64)])
            .await
            .context("postgres memory fts search failed")?;
        Ok(rows
            .into_iter()
            .filter_map(|row| parse_search_row(&row, query, "postgres_fts"))
            .collect())
    }

    fn vector_disabled(&self) -> bool {
        self.vector_failures.load(Ordering::Relaxed) >= 3
    }
}

fn parse_memory_row(row: &tokio_postgres::Row) -> Option<MemoryEntry> {
    let id: String = row.try_get(0).ok()?;
    let content: String = row.try_get(1).ok()?;
    let category: String = row.try_get(2).ok()?;
    let importance = read_f32_column(row, 3);
    let tags: Vec<String> = row.try_get(4).unwrap_or_default();
    let created_at = read_datetime_column(row, 5);
    let updated_at = read_datetime_column(row, 6);
    let access_count = read_i64_column(row, 7).max(0) as u64;
    Some(MemoryEntry {
        id,
        kind: kind_for_category(&category),
        category,
        content,
        tags,
        context: None,
        importance,
        created_at,
        updated_at,
        last_accessed_at: None,
        access_count,
    })
}

fn parse_search_row(
    row: &tokio_postgres::Row,
    query: &MemorySearchQuery,
    reason: &str,
) -> Option<MemorySearchResult> {
    let id: String = row.try_get(0).ok()?;
    let content: String = row.try_get(1).ok()?;
    let category: String = row.try_get(2).ok()?;
    let importance = read_f32_column(row, 3);
    let score = read_f32_column(row, 4);
    Some(MemorySearchResult {
        entry: MemoryEntry {
            id,
            kind: kind_for_category(&category),
            category,
            content,
            tags: Vec::new(),
            context: query.session_id.clone(),
            importance,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_accessed_at: None,
            access_count: 0,
        },
        score,
        reasons: vec![reason.to_string()],
    })
}

fn read_f32_column(row: &tokio_postgres::Row, idx: usize) -> f32 {
    row.try_get::<_, f32>(idx)
        .or_else(|_| row.try_get::<_, f64>(idx).map(|value| value as f32))
        .or_else(|_| row.try_get::<_, i32>(idx).map(|value| value as f32))
        .or_else(|_| row.try_get::<_, i64>(idx).map(|value| value as f32))
        .unwrap_or(0.5)
}

fn read_i64_column(row: &tokio_postgres::Row, idx: usize) -> i64 {
    row.try_get::<_, i64>(idx)
        .or_else(|_| row.try_get::<_, i32>(idx).map(|value| value as i64))
        .unwrap_or(0)
}

fn read_datetime_column(row: &tokio_postgres::Row, idx: usize) -> DateTime<Utc> {
    if let Ok(value) = row.try_get::<_, DateTime<Utc>>(idx) {
        return value;
    }
    if let Ok(ms) = row.try_get::<_, i64>(idx) {
        return DateTime::from_timestamp_millis(ms).unwrap_or_else(Utc::now);
    }
    if let Ok(secs) = row.try_get::<_, i32>(idx) {
        return DateTime::from_timestamp(secs as i64, 0).unwrap_or_else(Utc::now);
    }
    Utc::now()
}

fn sql_identifier(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return fallback.to_string();
    }
    if trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    {
        trimmed.to_string()
    } else {
        fallback.to_string()
    }
}

fn vector_literal(values: &[f32]) -> String {
    let body = values
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(",");
    format!("'[{body}]'")
}

pub fn postgres_memory_config_from_value(value: &Value) -> Option<PostgresMemoryConfig> {
    let memory = value.get("memory")?;
    if memory.get("backend").and_then(Value::as_str) != Some("postgres") {
        return None;
    }
    let pg = memory.get("postgres")?;
    Some(PostgresMemoryConfig {
        connection_string: pg
            .get("connectionString")
            .or_else(|| pg.get("connection_string"))
            .and_then(Value::as_str)?
            .to_string(),
        schema: pg
            .get("schema")
            .and_then(Value::as_str)
            .unwrap_or("public")
            .to_string(),
        table_prefix: pg
            .get("tablePrefix")
            .or_else(|| pg.get("table_prefix"))
            .and_then(Value::as_str)
            .unwrap_or("xuanyu")
            .to_string(),
        vector_dim: pg
            .get("vectorDim")
            .or_else(|| pg.get("vector_dim"))
            .and_then(Value::as_u64)
            .unwrap_or(1536) as usize,
        project_name: pg
            .get("projectName")
            .or_else(|| pg.get("project_name"))
            .and_then(Value::as_str)
            .or_else(|| memory.get("projectName").and_then(Value::as_str))
            .or_else(|| value.get("projectName").and_then(Value::as_str))
            .unwrap_or("default")
            .to_string(),
    })
}
