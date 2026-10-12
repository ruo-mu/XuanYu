use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

mod control_plane;
mod control_plane_handle;
mod embedded_postgres;
mod event_journal;
mod paths;
mod postgres_control_plane;
mod postgres_conversation;

pub use control_plane::{ControlPlaneStore, UpsertTaskInput};
pub use control_plane_handle::{ControlPlaneBackend, ControlPlaneHandle};
pub use embedded_postgres::{
    embedded_postgres_status, install_embedded_postgres, start_embedded_postgres,
    stop_embedded_postgres, EmbeddedPostgresInstallOptions, EmbeddedPostgresPaths,
    EmbeddedPostgresProgress, EmbeddedPostgresStatus,
};
pub use event_journal::{EventJournalStore, JournalEventRecord};
pub use paths::{
    default_app_data_dir, default_sqlite_database_path, embedded_postgres_root,
    global_app_dir, project_data_key, project_data_root, resolve_app_dir_from_selection,
    resolve_global_config_file, runtime_data_root, store_path, strip_utf8_bom, APP_DIR_NAME,
    PROJECT_CONFIG_FILE_NAME,
};
pub use postgres_control_plane::{
    postgres_control_plane_from_config, PostgresControlPlaneConfig, PostgresControlPlaneStore,
};
pub use postgres_conversation::{PostgresConversationStore, PostgresConversationStoreConfig};

/// Conversation persistence — PostgreSQL only.
pub type ConversationStore = PostgresConversationStore;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageConfig {
    pub data_dir: PathBuf,
    pub database_path: PathBuf,
    pub project_name: String,
}

impl StorageConfig {
    pub fn for_data_dir(data_dir: PathBuf) -> Self {
        Self {
            database_path: default_sqlite_database_path(&data_dir),
            data_dir,
            project_name: "default".to_string(),
        }
    }

    /// 配置文件固定在 `{用户主目录}/.xuanyu/xuanyu.json`，不随 `data_dir` 变化。
    /// 便携式安装的 `data_dir` 指向 `<安装目录>\data`（内置 PostgreSQL、插件、skill、
    /// mcp、memory 等数据都在那里），但配置与密钥仍留在用户主目录，这样升级/卸载重装
    /// 不会丢配置，也与安装器 `db-external` 的写入位置保持一致。
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
    event_journal: Arc<EventJournalStore>,
    control_plane: RwLock<ControlPlaneHandle>,
}

/// 探测外部 PostgreSQL 连接（连接 + `SELECT version()`），8 秒超时。
/// 无需构造 `StorageService`（不触碰任何磁盘状态），用于安装器/CLI 的纯连接验证。
pub async fn probe_external_postgres_connection(
    connection_string: &str,
) -> anyhow::Result<String> {
    use std::str::FromStr;
    let config = tokio_postgres::Config::from_str(connection_string.trim())
        .map_err(|err| anyhow::anyhow!("invalid connection string: {err}"))?;
    let (client, connection) =
        tokio::time::timeout(
            std::time::Duration::from_secs(8),
            config.connect(tokio_postgres::NoTls),
        )
        .await
        .map_err(|_| anyhow::anyhow!("连接超时（8 秒）/connection timed out (8s)"))?
        .map_err(|err| anyhow::anyhow!("连接失败/connection failed: {err}"))?;
    let _keepalive = tokio::spawn(async move {
        let _ = connection.await;
    });
    let row = tokio::time::timeout(
        std::time::Duration::from_secs(8),
        client.query_one("SELECT version()", &[]),
    )
    .await
    .map_err(|_| anyhow::anyhow!("查询超时（8 秒）/query timed out (8s)"))?
    .map_err(|err| anyhow::anyhow!("查询失败/query failed: {err}"))?;
    row.try_get::<_, String>(0)
        .map_err(|err| anyhow::anyhow!("读取版本失败/read version failed: {err}"))
}

impl StorageService {
    pub fn new(config: StorageConfig) -> Self {
        let conversation_store = Arc::new(PostgresConversationStore::new(
            config.project_name.clone(),
        ));
        let event_journal = Arc::new(
            EventJournalStore::open(&config.database_path)
                .expect("event journal store should open"),
        );
        let app_config = load_app_config_at(&config.config_path());
        let control_plane = RwLock::new(resolve_control_plane(
            &config.database_path,
            app_config.as_ref(),
        ));
        Self {
            config,
            initialized: RwLock::new(false),
            conversation_store,
            event_journal,
            control_plane,
        }
    }

    pub async fn initialize(&self) -> anyhow::Result<()> {
        tokio::fs::create_dir_all(global_app_dir()).await?;
        tokio::fs::create_dir_all(self.config.data_dir.join("store").join("sqlite")).await?;
        tokio::fs::create_dir_all(self.config.data_dir.join("logs")).await?;
        tokio::fs::create_dir_all(self.config.data_dir.join("events")).await?;
        tokio::fs::create_dir_all(self.config.data_dir.join("blobs")).await?;
        tokio::fs::create_dir_all(self.config.data_dir.join("models")).await?;
        if let Ok(Some(config)) = self.load_app_config().await {
            *self.control_plane.write().await =
                resolve_control_plane(&self.config.database_path, Some(&config));
        }
        let mut initialized = self.initialized.write().await;
        *initialized = true;
        Ok(())
    }

    /** Attach PostgreSQL conversation store (required before session IO). */
    pub async fn configure_conversation_store(
        &self,
        connection_string: impl Into<String>,
        schema: impl Into<String>,
        table_prefix: impl Into<String>,
    ) -> anyhow::Result<()> {
        self.conversation_store
            .configure(PostgresConversationStoreConfig {
                connection_string: connection_string.into(),
                project_name: self.config.project_name.clone(),
                schema: schema.into(),
                table_prefix: table_prefix.into(),
            })
            .await
    }

    /// 测试外部 PostgreSQL 连接（连接 + `SELECT version()`），8 秒超时。
    /// 只做连接验证，不写任何配置。返回数据库版本字符串。
    pub async fn test_external_postgres_connection(
        &self,
        connection_string: &str,
    ) -> anyhow::Result<String> {
        probe_external_postgres_connection(connection_string).await
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

    pub fn event_journal(&self) -> Arc<EventJournalStore> {
        self.event_journal.clone()
    }

    pub fn control_plane(&self) -> ControlPlaneHandle {
        self.control_plane
            .try_read()
            .map(|guard| guard.clone())
            .unwrap_or_else(|_| {
                resolve_control_plane(&self.config.database_path, None)
            })
    }

    pub async fn control_plane_async(&self) -> ControlPlaneHandle {
        self.control_plane.read().await.clone()
    }

    pub fn control_plane_backend(&self) -> &'static str {
        self.control_plane
            .try_read()
            .map(|guard| guard.backend_name())
            .unwrap_or("sqlite")
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
fn load_app_config_at(path: &std::path::Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(strip_utf8_bom(&text)).ok()
}

fn resolve_control_plane(
    database_path: &std::path::Path,
    config: Option<&serde_json::Value>,
) -> ControlPlaneHandle {
    if let Some(cfg) = config {
        if let Some(pg_cfg) = postgres_control_plane_from_config(cfg) {
            if let Ok(store) = PostgresControlPlaneStore::open(pg_cfg) {
                return Arc::new(ControlPlaneBackend::Postgres(Arc::new(store)));
            }
        }
    }
    Arc::new(ControlPlaneBackend::Sqlite(Arc::new(
        ControlPlaneStore::open(database_path).expect("control plane store should open"),
    )))
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

