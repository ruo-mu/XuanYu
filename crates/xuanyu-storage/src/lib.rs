use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

mod embedded_postgres;
mod paths;
mod postgres_control_plane;
mod postgres_conversation;

pub use embedded_postgres::{
    embedded_postgres_status, prepare_embedded_postgres, start_embedded_postgres,
    stop_embedded_postgres, EmbeddedPostgresPaths, EmbeddedPostgresPhase, EmbeddedPostgresProgress,
    EmbeddedPostgresStatus,
};
pub use paths::{
    config_dir, embedded_postgres_data_dir, logs_dir, platform_data_dir, project_data_key,
    project_data_root, resolve_global_config_file, runtime_data_root, strip_utf8_bom, APP_DIR_NAME,
    PROJECT_CONFIG_FILE_NAME,
};
pub use postgres_control_plane::{
    JournalEventRecord, PostgresControlPlaneConfig, PostgresControlPlaneStore, PostgresEventJournal,
    UpsertTaskInput,
};
pub use postgres_conversation::{PostgresConversationStore, PostgresConversationStoreConfig};

/// 控制面统一走 PostgreSQL。
pub type ControlPlaneHandle = Arc<PostgresControlPlaneStore>;

/// Conversation persistence — PostgreSQL only.
pub type ConversationStore = PostgresConversationStore;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageConfig {
    pub data_dir: PathBuf,
    pub project_name: String,
}

impl StorageConfig {
    pub fn for_data_dir(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            project_name: "default".to_string(),
        }
    }

    /// 配置文件固定在平台配置目录（`%APPDATA%\XuanYu` / `~/Library/Application Support/XuanYu`
    /// / `$XDG_CONFIG_HOME/xuanyu`），不随 `data_dir` 变化：升级或重装不会丢配置。
    pub fn config_path(&self) -> PathBuf {
        resolve_global_config_file()
    }

    pub fn with_project_name(mut self, project_name: impl Into<String>) -> Self {
        self.project_name = sanitize_segment(&project_name.into());
        self
    }
}

pub struct StorageService {
    config: StorageConfig,
    initialized: RwLock<bool>,
    conversation_store: Arc<PostgresConversationStore>,
    control_plane: RwLock<Option<ControlPlaneHandle>>,
    journal: Arc<PostgresEventJournal>,
}

impl StorageService {
    pub fn new(config: StorageConfig) -> Self {
        let conversation_store = Arc::new(PostgresConversationStore::new(
            config.project_name.clone(),
        ));
        Self {
            config,
            initialized: RwLock::new(false),
            conversation_store,
            control_plane: RwLock::new(None),
            journal: Arc::new(PostgresEventJournal::new()),
        }
    }

    pub async fn initialize(&self) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(config_dir()).await?;
        tokio::fs::create_dir_all(logs_dir()).await?;
        tokio::fs::create_dir_all(self.config.data_dir.join("events")).await?;
        tokio::fs::create_dir_all(self.config.data_dir.join("blobs")).await?;
        tokio::fs::create_dir_all(self.config.data_dir.join("models")).await?;
        let mut initialized = self.initialized.write().await;
        *initialized = true;
        Ok(())
    }

    /** Attach the built-in PostgreSQL store (required before session IO). */
    pub async fn configure_conversation_store(
        &self,
        connection_string: impl Into<String>,
        schema: impl Into<String>,
        table_prefix: impl Into<String>,
    ) -> anyhow::Result<()> {
        let connection_string = connection_string.into();
        let schema = schema.into();
        self.conversation_store
            .configure(PostgresConversationStoreConfig {
                connection_string: connection_string.clone(),
                project_name: self.config.project_name.clone(),
                schema: schema.clone(),
                table_prefix: table_prefix.into(),
            })
            .await?;
        let store = Arc::new(PostgresControlPlaneStore::open(PostgresControlPlaneConfig {
            connection_string,
            schema,
            psql_path: self.psql_path(),
        })?);
        self.journal.attach(store.clone())?;
        *self.control_plane.write().await = Some(store);
        Ok(())
    }

    pub async fn is_initialized(&self) -> bool {
        *self.initialized.read().await
    }

    pub fn config(&self) -> &StorageConfig {
        &self.config
    }

    pub fn conversation_store(&self) -> Arc<PostgresConversationStore> {
        self.conversation_store.clone()
    }

    pub fn event_journal(&self) -> Arc<PostgresEventJournal> {
        self.journal.clone()
    }

    pub fn control_plane(&self) -> Option<ControlPlaneHandle> {
        self.control_plane
            .try_read()
            .ok()
            .and_then(|guard| guard.clone())
    }

    pub fn control_plane_backend(&self) -> &'static str {
        "postgres"
    }

    pub fn app_config_path(&self) -> PathBuf {
        self.config.config_path()
    }

    pub fn memory_store_path(&self) -> PathBuf {
        self.config
            .data_dir
            .join("memory")
            .join(&self.config.project_name)
            .join("memories.json")
    }

    /// 内置实例自带的 psql，控制面用它执行 SQL。
    fn psql_path(&self) -> PathBuf {
        EmbeddedPostgresPaths::resolve(&self.config.data_dir).psql_path()
    }

    pub async fn load_app_config(&self) -> anyhow::Result<Option<serde_json::Value>> {
        let path = self.app_config_path();
        match tokio::fs::read_to_string(&path).await {
            Ok(text) => Ok(Some(serde_json::from_str(strip_utf8_bom(&text)).map_err(
                |err| anyhow::anyhow!("invalid app config {}: {err}", path.display()),
            )?)),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    pub async fn save_app_config(
        &self,
        config: serde_json::Value,
    ) -> anyhow::Result<serde_json::Value> {
        let path = self.app_config_path();
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = tokio::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
                    .await;
            }
        }
        let text = serde_json::to_string_pretty(&config)?;
        let tmp = path.with_extension("json.tmp");
        {
            use tokio::io::AsyncWriteExt;
            let mut file = tokio::fs::File::create(&tmp).await?;
            file.write_all(text.as_bytes()).await?;
            file.sync_all().await?;
        }
        tokio::fs::rename(&tmp, &path).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).await;
        }
        Ok(config)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationStoreConfig {
    pub connection_string: String,
    pub project_name: String,
    pub schema: String,
    pub table_prefix: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConversationSessionStatus {
    Active,
    Idle,
    Archived,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSessionRecord {
    pub id: String,
    pub project_name: String,
    pub title: String,
    pub status: ConversationSessionStatus,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_message_at: Option<DateTime<Utc>>,
    pub message_count: usize,
    pub current_run_id: Option<String>,
    pub archived_at: Option<DateTime<Utc>>,
    pub pinned: bool,
    pub tags: Vec<String>,
    pub metadata: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationMessageRecord {
    pub id: String,
    pub session_id: String,
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub token_count: Option<u64>,
    pub run_id: Option<String>,
    pub turn_id: Option<String>,
    pub metadata: BTreeMap<String, serde_json::Value>,
    pub tool_call_id: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSegmentRecord {
    pub id: String,
    pub session_id: String,
    pub sequence_number: u64,
    pub message_id: Option<String>,
    pub turn_id: Option<String>,
    pub segment_type: String,
    pub status: Option<String>,
    pub title: Option<String>,
    pub payload: BTreeMap<String, serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSnapshotRecord {
    pub id: String,
    pub session_id: String,
    pub run_id: Option<String>,
    pub checkpoint_id: Option<String>,
    pub summary: Option<String>,
    pub conversation: serde_json::Value,
    pub context_budget: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryHitRecord {
    pub id: String,
    pub session_id: String,
    pub run_id: Option<String>,
    pub turn_id: Option<String>,
    pub query: Option<String>,
    pub source: String,
    pub memory_id: String,
    pub score: Option<f32>,
    pub category: Option<String>,
    pub injected: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationHistory {
    pub session: Option<ConversationSessionRecord>,
    pub messages: Vec<ConversationMessageRecord>,
    pub segments: Vec<ConversationSegmentRecord>,
    pub memory_hits: Vec<MemoryHitRecord>,
    pub latest_snapshot: Option<ConversationSnapshotRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationStoreHealth {
    pub backend: String,
    pub ok: bool,
    pub location: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pgvector: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationSessionPatch {
    pub title: Option<String>,
    pub status: Option<ConversationSessionStatus>,
    pub updated_at: Option<DateTime<Utc>>,
    pub last_message_at: Option<DateTime<Utc>>,
    pub message_count: Option<usize>,
    pub current_run_id: Option<Option<String>>,
    pub archived_at: Option<Option<DateTime<Utc>>>,
    pub pinned: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<BTreeMap<String, serde_json::Value>>,
}

fn sanitize_segment(value: &str) -> String {
    let sanitized = value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>();
    let sanitized = sanitized.trim_matches('_');
    if sanitized.is_empty() {
        "default".to_string()
    } else {
        sanitized.chars().take(80).collect()
    }
}
