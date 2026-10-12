use chrono::{DateTime, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, RwLock};
use uuid::Uuid;
use xuanyu_storage::runtime_data_root;

mod embedding;
mod postgres_store;
mod tantivy_store;

pub use embedding::{
    MemoryEmbeddingConfig, MemoryEmbeddingProvider, MemoryEmbeddingProviderKind,
    MemoryEmbeddingProviderStatus,
};
pub use postgres_store::{postgres_memory_config_from_value, PostgresMemoryConfig, PostgresMemoryStore};
pub use tantivy_store::{tantivy_memory_config_from_value, TantivyMemoryConfig, TantivyMemoryStore};

const MEMORY_VECTOR_SIZE: usize = 1536;
const MEMORY_VECTOR_MATCH_THRESHOLD: f32 = 0.18;
const MEMORY_DUPLICATE_MERGE_THRESHOLD: f32 = 0.75;
const MEMORY_CONFLICT_PENDING_THRESHOLD: f32 = 0.6;
const MEMORY_SNAPSHOT_LIMIT: usize = 10;
const MEMORY_MAX_ENTRIES: usize = 1000;
const MEMORY_CLEANUP_IMPORTANCE_THRESHOLD: f32 = 0.3;
pub const MEMORY_SESSION_TAG_PREFIX: &str = "session:";
const MEMORY_RECALL_VECTOR_WEIGHT: f32 = 0.30;
const MEMORY_RECALL_KEYWORD_WEIGHT: f32 = 0.25;
const MEMORY_RECALL_TAG_WEIGHT: f32 = 0.15;
const MEMORY_RECALL_RECENCY_WEIGHT: f32 = 0.12;
const MEMORY_RECALL_IMPORTANCE_WEIGHT: f32 = 0.10;
const MEMORY_RECALL_FREQUENCY_WEIGHT: f32 = 0.05;
const MEMORY_RECALL_CONTEXT_WEIGHT: f32 = 0.03;
const DAY_MS: f32 = 86_400_000.0;
const FORGETTING_IDLE_DAYS: f32 = 7.0;
const DEFAULT_EMBEDDING_MODEL_ID: &str = "bge-small-zh-v1.5-onnx";
const DEFAULT_RERANK_MODEL_ID: &str = "bge-reranker-base-onnx";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Episodic,
    Semantic,
    Procedural,
    Relationship,
    SelfArchive,
    UserProfile,
    ProjectProfile,
    VoiceProfile,
    Preference,
    SafetyNote,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEntry {
    pub id: String,
    pub kind: MemoryKind,
    pub category: String,
    pub content: String,
    pub tags: Vec<String>,
    pub context: Option<String>,
    pub importance: f32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_accessed_at: Option<DateTime<Utc>>,
    pub access_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewMemoryEntry {
    pub kind: MemoryKind,
    pub category: String,
    pub content: String,
    pub tags: Vec<String>,
    pub context: Option<String>,
    pub importance: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionRole {
    User,
    Assistant,
    System,
    Tool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionMessage {
    pub role: ExtractionRole,
    pub content: String,
    pub tool_names: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    pub started_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<i64>,
    #[serde(default)]
    pub key_decisions: Vec<String>,
    #[serde(default)]
    pub files_modified: Vec<String>,
    #[serde(default)]
    pub tools_used: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_count: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryExtractionResult {
    pub memories: Vec<NewMemoryEntry>,
    pub summary: String,
    pub key_decisions: Vec<String>,
    pub files_modified: Vec<String>,
    pub tools_used: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryWriteDecision {
    Direct,
    Pending,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryConflictResolution {
    KeepExisting,
    Replace,
    Merge,
    Pending,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryConflict {
    pub existing_id: String,
    pub new_content: String,
    pub similarity: f32,
    pub resolution: MemoryConflictResolution,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryIngestInput {
    pub entry: NewMemoryEntry,
    pub source: String,
    pub confidence: Option<f32>,
    pub reason: String,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryIngestStatus {
    Stored,
    Pending,
    Denied,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryIngestResult {
    pub status: MemoryIngestStatus,
    pub memory: Option<MemoryEntry>,
    pub pending: Option<PendingMemory>,
    pub conflicts: Vec<MemoryConflict>,
}

/// Non-blocking memory write ingress queue (REFACTOR_PLAN §17.4).
pub struct MemoryIngress {
    service: Arc<MemoryService>,
}

impl MemoryIngress {
    pub fn new(service: Arc<MemoryService>) -> Self {
        Self { service }
    }

    pub fn from_service(service: &Arc<MemoryService>) -> Self {
        Self::new(service.clone())
    }

    pub fn enqueue(&self, input: MemoryIngestInput) {
        let service = self.service.clone();
        tokio::spawn(async move {
            let _ = service.ingest_memory_candidate(input).await;
        });
    }

    pub fn spawn_batch<F, C>(
        service: Arc<MemoryService>,
        inputs: Vec<MemoryIngestInput>,
        on_stored: F,
        on_complete: C,
    ) where
        F: Fn(MemoryEntry) + Send + Sync + 'static,
        C: Fn() + Send + 'static,
    {
        tokio::spawn(async move {
            for input in inputs {
                let result = service.ingest_memory_candidate(input).await;
                if let MemoryIngestStatus::Stored = result.status {
                    if let Some(entry) = result.memory {
                        on_stored(entry);
                    }
                }
            }
            on_complete();
        });
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchQuery {
    pub text: Option<String>,
    pub kind: Option<MemoryKind>,
    pub tags: Vec<String>,
    pub context: Option<String>,
    pub session_id: Option<String>,
    pub include_cross_session: bool,
    pub min_importance: Option<f32>,
    pub limit: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySearchResult {
    pub entry: MemoryEntry,
    pub score: f32,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingMemoryStatus {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingMemory {
    pub id: String,
    pub content: String,
    pub category: String,
    pub importance: u8,
    pub source: String,
    pub tags: Vec<String>,
    pub confidence: Option<f32>,
    pub reason: String,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub trace_id: Option<String>,
    pub status: PendingMemoryStatus,
    pub created_at: i64,
    pub decided_at: Option<i64>,
    pub rejected_until: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingMemoryInput {
    pub content: String,
    pub category: String,
    pub importance: u8,
    pub source: String,
    pub tags: Vec<String>,
    pub confidence: Option<f32>,
    pub reason: String,
    pub session_id: Option<String>,
    pub task_id: Option<String>,
    pub run_id: Option<String>,
    pub trace_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingMemoryEdit {
    pub content: Option<String>,
    pub category: Option<String>,
    pub importance: Option<u8>,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryStats {
    pub total_entries: usize,
    pub by_kind: BTreeMap<MemoryKind, usize>,
    pub by_category: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEmbeddingRecord {
    pub memory_id: String,
    pub provider: MemoryEmbeddingProviderKind,
    pub model: String,
    pub dimensions: usize,
    pub content_hash: String,
    pub vector: Vec<f32>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEmbeddingIndexStats {
    pub total: usize,
    pub ready: usize,
    pub stale: usize,
    pub provider: Option<MemoryEmbeddingProviderKind>,
    pub model: Option<String>,
    pub dimensions: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryEmbeddingRefreshResult {
    pub attempted: usize,
    pub refreshed: usize,
    pub skipped: usize,
    pub failed: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SemanticModelKind {
    Embedding,
    Rerank,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSemanticModelArtifacts {
    pub model_path: Option<String>,
    pub tokenizer_path: Option<String>,
    pub vocab_path: Option<String>,
    pub config_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSemanticModelStatus {
    pub id: String,
    pub kind: SemanticModelKind,
    pub engine: String,
    pub label: String,
    pub hint: Option<String>,
    pub archive: Option<String>,
    pub model_dir: String,
    pub ready: bool,
    pub missing_paths: Vec<String>,
    pub default: bool,
    pub dimensions: Option<u32>,
    pub max_tokens: Option<u32>,
    pub artifacts: LocalSemanticModelArtifacts,
    pub source: Option<LocalSemanticModelSourceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSemanticModelSourceInfo {
    pub repositories: Vec<String>,
    pub target_dir: String,
    pub required_files: Vec<String>,
    pub manual_hint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSemanticModelsStatus {
    pub root: String,
    pub embedding: Vec<LocalSemanticModelStatus>,
    pub rerank: Vec<LocalSemanticModelStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalSemanticModelImportResult {
    pub id: String,
    pub kind: SemanticModelKind,
    pub source_path: String,
    pub model_dir: String,
    pub copied_files: Vec<String>,
    pub ready: bool,
    pub missing_paths: Vec<String>,
    pub status: LocalSemanticModelStatus,
}

#[derive(Debug, Clone)]
struct SemanticModelLayout {
    status: LocalSemanticModelStatus,
    required_relative_paths: Vec<PathBuf>,
}

#[derive(Debug, Clone, Copy)]
struct SemanticModelSpec {
    id: &'static str,
    kind: SemanticModelKind,
    dir_name: &'static str,
    label: &'static str,
    hint: &'static str,
    dimensions: Option<u32>,
    max_tokens: Option<u32>,
    repositories: &'static [&'static str],
}

const SEMANTIC_MODEL_SPECS: &[SemanticModelSpec] = &[
    SemanticModelSpec {
        id: DEFAULT_EMBEDDING_MODEL_ID,
        kind: SemanticModelKind::Embedding,
        dir_name: "bge-small-zh-v1.5-onnx",
        label: "BGE small zh v1.5 ONNX",
        hint: "Chinese/English local semantic embedding model; expected layout: model.onnx + tokenizer.json or vocab.txt",
        dimensions: Some(512),
        max_tokens: Some(512),
        repositories: &[
            "https://huggingface.co/BAAI/bge-small-zh-v1.5",
            "https://modelscope.cn/models/BAAI/bge-small-zh-v1.5",
        ],
    },
    SemanticModelSpec {
        id: DEFAULT_RERANK_MODEL_ID,
        kind: SemanticModelKind::Rerank,
        dir_name: "bge-reranker-base-onnx",
        label: "BGE reranker base ONNX",
        hint: "Local cross-encoder reranker; expected layout: model.onnx + tokenizer.json or vocab.txt",
        dimensions: None,
        max_tokens: Some(512),
        repositories: &[
            "https://huggingface.co/BAAI/bge-reranker-base",
            "https://modelscope.cn/models/BAAI/bge-reranker-base",
        ],
    },
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshot {
    pub version: u64,
    pub timestamp: DateTime<Utc>,
    pub entry_count: usize,
    pub entries: Vec<MemoryEntry>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemorySnapshotInfo {
    pub version: u64,
    pub timestamp: DateTime<Utc>,
    pub entry_count: usize,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgettingPolicy {
    pub decay_rate_per_day: f32,
    pub importance_threshold: f32,
    pub retrieval_threshold: u64,
    pub max_retention_days: u32,
}

impl Default for ForgettingPolicy {
    fn default() -> Self {
        Self {
            decay_rate_per_day: 0.02,
            importance_threshold: 0.4,
            retrieval_threshold: 2,
            max_retention_days: 90,
        }
    }
}

pub fn semantic_models_root() -> PathBuf {
    std::env::var_os("XUANYU_SEMANTIC_MODELS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| runtime_data_root().join("models").join("semantic"))
}

pub fn local_semantic_models_status() -> LocalSemanticModelsStatus {
    local_semantic_models_status_for_root(&semantic_models_root())
}

pub fn import_local_semantic_model(
    model_id: &str,
    source_path: &Path,
) -> Result<LocalSemanticModelImportResult, String> {
    import_local_semantic_model_for_root(&semantic_models_root(), model_id, source_path)
}

fn local_semantic_models_status_for_root(root: &Path) -> LocalSemanticModelsStatus {
    let mut embedding = Vec::new();
    let mut rerank = Vec::new();
    for spec in SEMANTIC_MODEL_SPECS {
        let status = semantic_model_status(root, *spec);
        match spec.kind {
            SemanticModelKind::Embedding => embedding.push(status),
            SemanticModelKind::Rerank => rerank.push(status),
        }
    }
    LocalSemanticModelsStatus {
        root: path_string(root),
        embedding,
        rerank,
    }
}

fn import_local_semantic_model_for_root(
    root: &Path,
    model_id: &str,
    source_path: &Path,
) -> Result<LocalSemanticModelImportResult, String> {
    let layout = semantic_model_layout(root, model_id)
        .ok_or_else(|| format!("Unsupported local semantic model: {model_id}"))?;
    let metadata = fs::symlink_metadata(source_path).map_err(|err| {
        format!(
            "Cannot read semantic model source '{}': {err}",
            path_string(source_path)
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "Refusing to import semantic model through symlink '{}'",
            path_string(source_path)
        ));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "Semantic model source must be a directory: {}",
            path_string(source_path)
        ));
    }

    let model_dir = PathBuf::from(&layout.status.model_dir);
    let source_root = select_semantic_model_source_root(
        source_path,
        &model_dir,
        model_id,
        &layout.required_relative_paths,
    )
    .ok_or_else(|| {
        let mut required = layout
            .required_relative_paths
            .iter()
            .map(|path| path_string(path))
            .collect::<Vec<_>>();
        required.push("tokenizer.json or vocab.txt".to_string());
        format!(
            "Semantic model source '{}' does not contain required files for {}: {}",
            path_string(source_path),
            model_id,
            required.join(", ")
        )
    })?;

    let source_is_target = same_directory(&source_root, &model_dir);
    if !source_is_target && path_is_inside_directory(&source_root, &model_dir) {
        return Err(format!(
            "Refusing to import semantic model from inside target directory '{}'",
            path_string(&model_dir)
        ));
    }

    if !source_is_target && model_dir.exists() {
        fs::remove_dir_all(&model_dir).map_err(|err| {
            format!(
                "Cannot replace semantic model directory '{}': {err}",
                path_string(&model_dir)
            )
        })?;
    }
    fs::create_dir_all(&model_dir).map_err(|err| {
        format!(
            "Cannot create semantic model directory '{}': {err}",
            path_string(&model_dir)
        )
    })?;

    let mut copied_files = Vec::new();
    if !source_is_target {
        copy_semantic_model_directory(&source_root, &model_dir, &source_root, &mut copied_files)?;
        copied_files.sort();
        copied_files.dedup();
    }

    let imported_status = semantic_model_layout(root, &layout.status.id)
        .expect("semantic model layout should still exist")
        .status;
    Ok(LocalSemanticModelImportResult {
        id: imported_status.id.clone(),
        kind: imported_status.kind,
        source_path: path_string(&source_root),
        model_dir: imported_status.model_dir.clone(),
        copied_files,
        ready: imported_status.ready,
        missing_paths: imported_status.missing_paths.clone(),
        status: imported_status,
    })
}

fn semantic_model_layout(root: &Path, model_id: &str) -> Option<SemanticModelLayout> {
    let normalized = match model_id.trim() {
        "" | "embedding-default" | "bge-small-zh" => DEFAULT_EMBEDDING_MODEL_ID,
        "rerank-default" | "bge-reranker-base" => DEFAULT_RERANK_MODEL_ID,
        value => value,
    };
    let spec = SEMANTIC_MODEL_SPECS
        .iter()
        .copied()
        .find(|spec| spec.id == normalized)?;
    let status = semantic_model_status(root, spec);
    let model_dir = PathBuf::from(&status.model_dir);
    Some(SemanticModelLayout {
        status,
        required_relative_paths: vec![model_dir.join("model.onnx")]
            .into_iter()
            .filter_map(|path| path.strip_prefix(&model_dir).ok().map(Path::to_path_buf))
            .collect(),
    })
}

fn semantic_model_status(root: &Path, spec: SemanticModelSpec) -> LocalSemanticModelStatus {
    let kind_dir = match spec.kind {
        SemanticModelKind::Embedding => "embedding",
        SemanticModelKind::Rerank => "rerank",
    };
    let model_dir = root.join(kind_dir).join(spec.dir_name);
    let artifacts = semantic_model_artifacts(&model_dir);
    let mut missing_paths = Vec::new();
    if !model_dir.join("model.onnx").exists() {
        missing_paths.push(path_string(&model_dir.join("model.onnx")));
    }
    if artifacts.tokenizer_path.is_none() && artifacts.vocab_path.is_none() {
        missing_paths.push(format!(
            "{} or {}",
            path_string(&model_dir.join("tokenizer.json")),
            path_string(&model_dir.join("vocab.txt"))
        ));
    }
    LocalSemanticModelStatus {
        id: spec.id.to_string(),
        kind: spec.kind,
        engine: "onnxruntime".to_string(),
        label: spec.label.to_string(),
        hint: Some(spec.hint.to_string()),
        archive: None,
        model_dir: path_string(&model_dir),
        ready: missing_paths.is_empty(),
        missing_paths,
        default: spec.id == DEFAULT_EMBEDDING_MODEL_ID || spec.id == DEFAULT_RERANK_MODEL_ID,
        dimensions: spec.dimensions,
        max_tokens: spec.max_tokens,
        artifacts,
        source: Some(semantic_model_source_info(&model_dir, spec.repositories)),
    }
}

fn semantic_model_source_info(
    model_dir: &Path,
    repositories: &[&str],
) -> LocalSemanticModelSourceInfo {
    LocalSemanticModelSourceInfo {
        repositories: repositories.iter().map(|url| (*url).to_string()).collect(),
        target_dir: path_string(model_dir),
        required_files: vec![
            "model.onnx".to_string(),
            "tokenizer.json or vocab.txt".to_string(),
            "config.json optional".to_string(),
        ],
        manual_hint: "Place an ONNX export in targetDir with model.onnx and tokenizer.json or vocab.txt, then refresh or import that directory.".to_string(),
    }
}

fn semantic_model_artifacts(model_dir: &Path) -> LocalSemanticModelArtifacts {
    let tokenizer_json = model_dir.join("tokenizer.json");
    let vocab_txt = model_dir.join("vocab.txt");
    let config_json = model_dir.join("config.json");
    LocalSemanticModelArtifacts {
        model_path: model_dir
            .join("model.onnx")
            .exists()
            .then(|| path_string(&model_dir.join("model.onnx"))),
        tokenizer_path: tokenizer_json
            .exists()
            .then(|| path_string(&tokenizer_json)),
        vocab_path: vocab_txt.exists().then(|| path_string(&vocab_txt)),
        config_path: config_json.exists().then(|| path_string(&config_json)),
    }
}

fn select_semantic_model_source_root(
    source_path: &Path,
    model_dir: &Path,
    model_id: &str,
    required_relative_paths: &[PathBuf],
) -> Option<PathBuf> {
    let model_dir_name = model_dir.file_name()?.to_string_lossy().to_string();
    let kind_dir_name = model_dir
        .parent()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().to_string());
    let mut candidates = vec![
        source_path.to_path_buf(),
        source_path.join(&model_dir_name),
        source_path.join(model_id),
    ];
    if let Some(kind) = kind_dir_name {
        candidates.push(source_path.join(kind).join(&model_dir_name));
    }
    candidates.into_iter().find(|candidate| {
        candidate.is_dir()
            && required_relative_paths
                .iter()
                .all(|relative| candidate.join(relative).exists())
            && (candidate.join("tokenizer.json").exists() || candidate.join("vocab.txt").exists())
    })
}

fn copy_semantic_model_directory(
    source: &Path,
    dest: &Path,
    root: &Path,
    copied_files: &mut Vec<String>,
) -> Result<(), String> {
    for entry in fs::read_dir(source)
        .map_err(|err| format!("Cannot read model source '{}': {err}", path_string(source)))?
    {
        let entry = entry.map_err(|err| format!("Cannot read model source entry: {err}"))?;
        let source_path = entry.path();
        let metadata = fs::symlink_metadata(&source_path).map_err(|err| {
            format!(
                "Cannot read model source '{}': {err}",
                path_string(&source_path)
            )
        })?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "Refusing to copy model symlink '{}'",
                path_string(&source_path)
            ));
        }
        let relative = source_path.strip_prefix(root).map_err(|_| {
            format!(
                "Model source '{}' is outside '{}'",
                path_string(&source_path),
                path_string(root)
            )
        })?;
        let target_path = dest.join(relative);
        if metadata.is_dir() {
            fs::create_dir_all(&target_path).map_err(|err| {
                format!(
                    "Cannot create model directory '{}': {err}",
                    path_string(&target_path)
                )
            })?;
            copy_semantic_model_directory(&source_path, dest, root, copied_files)?;
        } else if metadata.is_file() {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent).map_err(|err| {
                    format!(
                        "Cannot create model directory '{}': {err}",
                        path_string(parent)
                    )
                })?;
            }
            fs::copy(&source_path, &target_path).map_err(|err| {
                format!(
                    "Cannot copy model file '{}' to '{}': {err}",
                    path_string(&source_path),
                    path_string(&target_path)
                )
            })?;
            copied_files.push(path_string(relative));
        }
    }
    Ok(())
}

fn same_directory(left: &Path, right: &Path) -> bool {
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn path_is_inside_directory(path: &Path, directory: &Path) -> bool {
    match (fs::canonicalize(path), fs::canonicalize(directory)) {
        (Ok(path), Ok(directory)) => path.starts_with(directory),
        _ => false,
    }
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ForgettingResult {
    pub decayed: Vec<String>,
    pub dropped: Vec<String>,
    pub kept: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryConsolidationResult {
    pub merged: usize,
    pub distilled: usize,
    pub pruned: usize,
}

pub struct MemoryService {
    entries: RwLock<BTreeMap<String, MemoryEntry>>,
    embeddings: RwLock<BTreeMap<String, MemoryEmbeddingRecord>>,
    pending: RwLock<BTreeMap<String, PendingMemory>>,
    snapshots: RwLock<Vec<MemorySnapshot>>,
    current_snapshot_version: RwLock<u64>,
    persist_path: Option<PathBuf>,
    initialized: RwLock<bool>,
    write_lock: Mutex<()>,
    postgres: RwLock<Option<Arc<PostgresMemoryStore>>>,
    tantivy: RwLock<Option<Arc<TantivyMemoryStore>>>,
    summaries: RwLock<BTreeMap<String, SessionSummary>>,
}

impl MemoryService {
    pub fn new() -> Self {
        Self {
            entries: RwLock::new(BTreeMap::new()),
            embeddings: RwLock::new(BTreeMap::new()),
            pending: RwLock::new(BTreeMap::new()),
            snapshots: RwLock::new(Vec::new()),
            current_snapshot_version: RwLock::new(0),
            persist_path: None,
            initialized: RwLock::new(true),
            write_lock: Mutex::new(()),
            postgres: RwLock::new(None),
            tantivy: RwLock::new(None),
            summaries: RwLock::new(BTreeMap::new()),
        }
    }

    pub fn persistent(path: PathBuf) -> Self {
        Self {
            entries: RwLock::new(BTreeMap::new()),
            embeddings: RwLock::new(BTreeMap::new()),
            pending: RwLock::new(BTreeMap::new()),
            snapshots: RwLock::new(Vec::new()),
            current_snapshot_version: RwLock::new(0),
            persist_path: Some(path),
            initialized: RwLock::new(false),
            write_lock: Mutex::new(()),
            postgres: RwLock::new(None),
            tantivy: RwLock::new(None),
            summaries: RwLock::new(BTreeMap::new()),
        }
    }

    pub async fn configure_tantivy(&self, config: TantivyMemoryConfig) -> anyhow::Result<()> {
        let store = Arc::new(TantivyMemoryStore::open(config)?);
        *self.tantivy.write().await = Some(store);
        Ok(())
    }

    pub async fn configure_postgres(&self, config: PostgresMemoryConfig) -> anyhow::Result<()> {
        let store = Arc::new(PostgresMemoryStore::open(config).await?);
        if let Ok(rows) = store.load_all().await {
            if !rows.is_empty() {
                let mut entries = self.entries.write().await;
                for entry in rows {
                    entries.entry(entry.id.clone()).or_insert(entry);
                }
            }
        }
        if let Ok(rows) = store.load_summaries().await {
            if !rows.is_empty() {
                let mut summaries = self.summaries.write().await;
                for summary in rows {
                    summaries.insert(summary.session_id.clone(), summary);
                }
            }
        }
        *self.postgres.write().await = Some(store);
        Ok(())
    }

    /** Whether the live Postgres/pgvector memory store is attached. */
    pub async fn has_postgres(&self) -> bool {
        self.postgres.read().await.is_some()
    }

    async fn mirror_memory_to_postgres(&self, entry: &MemoryEntry) {
        let Some(store) = self.postgres.read().await.clone() else {
            return;
        };
        let embedding = self
            .embeddings
            .read()
            .await
            .get(&entry.id)
            .map(|record| record.vector.clone());
        let input = NewMemoryEntry {
            kind: entry.kind,
            category: entry.category.clone(),
            content: entry.content.clone(),
            tags: entry.tags.clone(),
            context: entry.context.clone(),
            importance: entry.importance,
        };
        if let Err(err) = store
            .upsert(&input, &entry.id, embedding.as_deref())
            .await
        {
            tracing::warn!("postgres memory upsert failed: {err:#}");
        }
        self.mirror_memory_to_tantivy(&input, &entry.id).await;
    }

    async fn mirror_memory_to_tantivy(&self, entry: &NewMemoryEntry, id: &str) {
        let Some(store) = self.tantivy.read().await.clone() else {
            return;
        };
        if let Err(err) = store.upsert(entry, id) {
            tracing::warn!("tantivy memory upsert failed: {err:#}");
        }
    }

    async fn mirror_memory_delete_tantivy(&self, id: &str) {
        let Some(store) = self.tantivy.read().await.clone() else {
            return;
        };
        if let Err(err) = store.delete(id) {
            tracing::warn!("tantivy memory delete failed: {err:#}");
        }
    }

    async fn mirror_memory_delete_postgres(&self, id: &str) {
        let Some(store) = self.postgres.read().await.clone() else {
            return;
        };
        if let Err(err) = store.delete(id).await {
            tracing::warn!("postgres memory delete failed: {err:#}");
        }
    }

    async fn mirror_summary_to_postgres(&self, summary: &SessionSummary) {
        let Some(store) = self.postgres.read().await.clone() else {
            return;
        };
        if let Err(err) = store.upsert_summary(summary).await {
            tracing::warn!("postgres session summary upsert failed: {err:#}");
        }
    }

    pub async fn add_session_summary(&self, summary: SessionSummary) -> SessionSummary {
        let _ = self.initialize().await;
        self.summaries
            .write()
            .await
            .insert(summary.session_id.clone(), summary.clone());
        self.mirror_summary_to_postgres(&summary).await;
        summary
    }

    pub async fn get_session_summary(&self, session_id: &str) -> Option<SessionSummary> {
        let _ = self.initialize().await;
        self.summaries.read().await.get(session_id).cloned()
    }

    pub async fn recent_session_summaries(&self, limit: usize) -> Vec<SessionSummary> {
        let _ = self.initialize().await;
        let mut summaries = self
            .summaries
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        summaries.sort_by_key(|s| std::cmp::Reverse(s.started_at));
        summaries.truncate(limit.max(1));
        summaries
    }

    pub fn storage_path(&self) -> Option<PathBuf> {
        self.persist_path.clone()
    }

    pub async fn initialize(&self) -> anyhow::Result<()> {
        if *self.initialized.read().await {
            return Ok(());
        }
        let _guard = self.write_lock.lock().await;
        if *self.initialized.read().await {
            return Ok(());
        }
        let Some(path) = &self.persist_path else {
            *self.initialized.write().await = true;
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let data = match tokio::fs::read_to_string(path).await {
            Ok(text) => serde_json::from_str::<PersistedMemoryData>(
                text.trim_start_matches('\u{feff}'),
            )?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                PersistedMemoryData::default()
            }
            Err(err) => return Err(err.into()),
        };
        let mut entries = BTreeMap::new();
        for entry in data.entries {
            entries.insert(entry.id.clone(), entry);
        }
        let mut embeddings = BTreeMap::new();
        for record in data.embeddings {
            if entries.contains_key(&record.memory_id) {
                embeddings.insert(record.memory_id.clone(), record);
            }
        }
        let mut pending = BTreeMap::new();
        for item in data.pending {
            pending.insert(item.id.clone(), item);
        }
        let current_snapshot_version = data.current_snapshot_version.max(
            data.snapshots
                .iter()
                .map(|snapshot| snapshot.version)
                .max()
                .unwrap_or(0),
        );
        *self.entries.write().await = entries;
        *self.embeddings.write().await = embeddings;
        *self.pending.write().await = pending;
        *self.snapshots.write().await = data.snapshots;
        *self.current_snapshot_version.write().await = current_snapshot_version;
        *self.initialized.write().await = true;
        Ok(())
    }

    pub async fn add_memory(&self, input: NewMemoryEntry) -> MemoryEntry {
        let _ = self.initialize().await;
        let now = Utc::now();
        let category = normalize_category(input.category, input.kind);
        let content = input.content.trim().to_string();
        let tags = normalize_tags(input.tags);
        let mut entries = self.entries.write().await;
        if let Some(existing_id) =
            find_duplicate_entry_id(&entries, &content, &category, input.context.as_deref())
        {
            let entry = entries
                .get_mut(&existing_id)
                .expect("duplicate id should resolve to memory entry");
            entry.content = choose_better_content(&entry.content, &content);
            entry.importance = entry.importance.max(input.importance.clamp(0.0, 1.0));
            entry.tags = merge_tags(&entry.tags, &tags);
            if entry.context.is_none() {
                entry.context = input.context;
            }
            entry.updated_at = now;
            entry.last_accessed_at = Some(now);
            let entry = entry.clone();
            drop(entries);
            self.embeddings.write().await.remove(&entry.id);
            let _ = self.flush().await;
            self.mirror_memory_to_postgres(&entry).await;
            return entry;
        }
        let entry = MemoryEntry {
            id: Uuid::new_v4().to_string(),
            kind: input.kind,
            category,
            content,
            tags,
            context: input.context,
            importance: input.importance.clamp(0.0, 1.0),
            created_at: now,
            updated_at: now,
            last_accessed_at: None,
            access_count: 0,
        };
        entries.insert(entry.id.clone(), entry.clone());
        let removed = enforce_memory_capacity(&mut entries);
        drop(entries);
        self.prune_embeddings(&removed).await;
        let _ = self.flush().await;
        self.mirror_memory_to_postgres(&entry).await;
        entry
    }

    pub async fn import_memory_entry(&self, mut entry: MemoryEntry) -> MemoryEntry {
        let _ = self.initialize().await;
        if entry.id.trim().is_empty() {
            entry.id = Uuid::new_v4().to_string();
        }
        entry.category = normalize_category(entry.category, entry.kind);
        entry.tags = normalize_tags(entry.tags);
        entry.importance = entry.importance.clamp(0.0, 1.0);
        if entry.updated_at < entry.created_at {
            entry.updated_at = entry.created_at;
        }
        let mut entries = self.entries.write().await;
        entries.insert(entry.id.clone(), entry.clone());
        let removed = enforce_memory_capacity(&mut entries);
        drop(entries);
        self.embeddings.write().await.remove(&entry.id);
        self.prune_embeddings(&removed).await;
        let _ = self.flush().await;
        self.mirror_memory_to_postgres(&entry).await;
        entry
    }

    pub async fn get_memory(&self, id: &str) -> Option<MemoryEntry> {
        let _ = self.initialize().await;
        let mut entries = self.entries.write().await;
        let entry = entries.get_mut(id)?;
        entry.last_accessed_at = Some(Utc::now());
        entry.access_count = entry.access_count.saturating_add(1);
        let entry = entry.clone();
        drop(entries);
        let _ = self.flush().await;
        Some(entry)
    }

    pub async fn update_memory(&self, id: &str, updates: MemoryUpdate) -> Option<MemoryEntry> {
        let _ = self.initialize().await;
        let mut entries = self.entries.write().await;
        let entry = entries.get_mut(id)?;
        let mut invalidates_embedding = false;
        if let Some(content) = updates.content {
            entry.content = content;
            invalidates_embedding = true;
        }
        if let Some(tags) = updates.tags {
            entry.tags = normalize_tags(tags);
            invalidates_embedding = true;
        }
        if let Some(category) = updates.category {
            entry.category = normalize_category(category, entry.kind);
            invalidates_embedding = true;
        }
        if let Some(kind) = updates.kind {
            entry.kind = kind;
            entry.category = normalize_category(entry.category.clone(), kind);
            invalidates_embedding = true;
        }
        if let Some(importance) = updates.importance {
            entry.importance = importance.clamp(0.0, 1.0);
        }
        entry.updated_at = Utc::now();
        let entry = entry.clone();
        drop(entries);
        if invalidates_embedding {
            self.embeddings.write().await.remove(&entry.id);
        }
        let _ = self.flush().await;
        self.mirror_memory_to_postgres(&entry).await;
        Some(entry)
    }

    pub async fn delete_memory(&self, id: &str) -> bool {
        let _ = self.initialize().await;
        let deleted = self.entries.write().await.remove(id).is_some();
        if deleted {
            self.embeddings.write().await.remove(id);
            let _ = self.flush().await;
            self.mirror_memory_delete_postgres(id).await;
            self.mirror_memory_delete_tantivy(id).await;
        }
        deleted
    }

    pub async fn search(&self, query: MemorySearchQuery) -> Vec<MemorySearchResult> {
        self.search_internal(query, None).await
    }

    pub async fn search_with_embedding_provider(
        &self,
        query: MemorySearchQuery,
        embedding_provider: &MemoryEmbeddingProvider,
    ) -> Vec<MemorySearchResult> {
        self.search_internal(query, Some(embedding_provider)).await
    }

    async fn search_internal(
        &self,
        query: MemorySearchQuery,
        embedding_provider: Option<&MemoryEmbeddingProvider>,
    ) -> Vec<MemorySearchResult> {
        let MemorySearchQuery {
            text,
            kind,
            tags,
            context,
            session_id,
            include_cross_session,
            min_importance,
            limit,
        } = query;
        let _ = self.initialize().await;
        let limit = limit.max(1);
        let text = text.as_ref().map(|value| value.to_ascii_lowercase());
        let tags: Vec<String> = normalize_tags(tags);
        let scoped_candidates = {
            let entries = self.entries.read().await;
            let candidates = entries
                .values()
                .filter(|entry| {
                    matches_memory_session_scope(
                        entry,
                        session_id.as_deref(),
                        include_cross_session,
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            let context = normalize_context_filter(context.as_deref());
            let context_matches = context
                .as_deref()
                .map(|context| {
                    candidates
                        .iter()
                        .filter(|entry| {
                            entry
                                .context
                                .as_deref()
                                .map(normalize_context_value)
                                .as_deref()
                                == Some(context)
                        })
                        .cloned()
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            if context.is_some() && !context_matches.is_empty() {
                context_matches
            } else {
                candidates
            }
        };
        let vector_scores = if let Some(embedding_provider) = embedding_provider {
            self.embedding_vector_scores(text.as_deref(), &scoped_candidates, embedding_provider)
                .await
        } else {
            None
        };
        if let Some(store) = self.tantivy.read().await.clone() {
            if let Some(query_text) = text.as_deref().filter(|value| !value.is_empty()) {
                let tv_query = MemorySearchQuery {
                    text: Some(query_text.to_string()),
                    kind,
                    tags: tags.clone(),
                    context: context.clone(),
                    session_id: session_id.clone(),
                    include_cross_session,
                    min_importance,
                    limit,
                };
                if let Ok(tv_results) = store.search(&tv_query, limit) {
                    if !tv_results.is_empty() {
                        return tv_results;
                    }
                }
            }
        }
        if let Some(store) = self.postgres.read().await.clone() {
            if let Some(query_text) = text.as_deref().filter(|value| !value.is_empty()) {
                let pg_query = MemorySearchQuery {
                    text: Some(query_text.to_string()),
                    kind,
                    tags: tags.clone(),
                    context: context.clone(),
                    session_id: session_id.clone(),
                    include_cross_session,
                    min_importance,
                    limit,
                };
                if let Some(provider) = embedding_provider {
                    if let Ok(embedding) = provider.embed(query_text).await {
                        if let Ok(pg_results) = store.search_vector(&pg_query, &embedding, limit).await {
                            if !pg_results.is_empty() {
                                return pg_results;
                            }
                        }
                    }
                }
                if let Ok(pg_results) = store.search_fts(&pg_query, limit).await {
                    if !pg_results.is_empty() {
                        return pg_results;
                    }
                }
            }
        }
        let mut results = scoped_candidates
            .iter()
            .filter_map(|entry| {
                score_entry(
                    entry,
                    text.as_deref(),
                    kind,
                    &tags,
                    min_importance,
                    vector_scores
                        .as_ref()
                        .and_then(|scores| scores.get(&entry.id))
                        .copied(),
                )
            })
            .collect::<Vec<_>>();
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
        results.truncate(limit);
        self.track_search_access(&mut results).await;
        results
    }

    pub async fn refresh_embedding_for_entry(
        &self,
        memory_id: &str,
        embedding_provider: &MemoryEmbeddingProvider,
    ) -> Option<MemoryEmbeddingRecord> {
        let result = self
            .refresh_embeddings_for_ids(&[memory_id.to_string()], embedding_provider)
            .await;
        let record = self.embeddings.read().await.get(memory_id).cloned()?;
        let entry = self.entries.read().await.get(memory_id).cloned()?;
        if result.refreshed > 0 || embedding_record_matches(&record, &entry, embedding_provider) {
            Some(record)
        } else {
            None
        }
    }

    pub async fn refresh_embeddings(
        &self,
        embedding_provider: &MemoryEmbeddingProvider,
        limit: usize,
    ) -> MemoryEmbeddingRefreshResult {
        let _ = self.initialize().await;
        if !embedding_provider.uses_external_api() {
            return MemoryEmbeddingRefreshResult::default();
        }
        let mut entries = self
            .entries
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        entries.sort_by_key(|e| std::cmp::Reverse(e.updated_at));
        let embeddings = self.embeddings.read().await;
        let ids = entries
            .iter()
            .filter(|entry| {
                embeddings.get(&entry.id).is_none_or(|record| {
                    !embedding_record_matches(record, entry, embedding_provider)
                })
            })
            .take(limit.max(1))
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        drop(embeddings);
        self.refresh_embeddings_for_ids(&ids, embedding_provider)
            .await
    }

    async fn refresh_embeddings_for_ids(
        &self,
        memory_ids: &[String],
        embedding_provider: &MemoryEmbeddingProvider,
    ) -> MemoryEmbeddingRefreshResult {
        let _ = self.initialize().await;
        if memory_ids.is_empty() || !embedding_provider.uses_external_api() {
            return MemoryEmbeddingRefreshResult::default();
        }
        let entries_by_id = self.entries.read().await;
        let embeddings = self.embeddings.read().await;
        let mut entries = Vec::new();
        let mut result = MemoryEmbeddingRefreshResult::default();
        for id in memory_ids {
            let Some(entry) = entries_by_id.get(id) else {
                result.skipped += 1;
                continue;
            };
            if embeddings
                .get(id)
                .is_some_and(|record| embedding_record_matches(record, entry, embedding_provider))
            {
                result.skipped += 1;
                continue;
            }
            entries.push(entry.clone());
        }
        drop(embeddings);
        drop(entries_by_id);

        if entries.is_empty() {
            return result;
        }

        let batch_size = embedding_provider.config().batch_size.max(1);
        for batch in entries.chunks(batch_size) {
            let texts = batch.iter().map(memory_vector_haystack).collect::<Vec<_>>();
            result.attempted += batch.len();
            match embedding_provider.embed_batch(&texts).await {
                Ok(vectors) if vectors.len() == batch.len() => {
                    let now = Utc::now();
                    let mut index = self.embeddings.write().await;
                    let mut mirrored = Vec::new();
                    for (entry, vector) in batch.iter().zip(vectors) {
                        index.insert(
                            entry.id.clone(),
                            MemoryEmbeddingRecord::from_entry(
                                entry,
                                embedding_provider,
                                vector,
                                now,
                            ),
                        );
                        mirrored.push(entry.clone());
                        result.refreshed += 1;
                    }
                    drop(index);
                    for entry in mirrored {
                        self.mirror_memory_to_postgres(&entry).await;
                    }
                }
                _ => {
                    result.failed += batch.len();
                }
            }
        }

        if result.refreshed > 0 {
            let _ = self.flush().await;
        }
        result
    }

    pub async fn embedding_index_stats(
        &self,
        embedding_provider: Option<&MemoryEmbeddingProvider>,
    ) -> MemoryEmbeddingIndexStats {
        let _ = self.initialize().await;
        let entries = self.entries.read().await;
        let embeddings = self.embeddings.read().await;
        let mut stats = MemoryEmbeddingIndexStats {
            total: embeddings.len(),
            ..MemoryEmbeddingIndexStats::default()
        };
        if let Some(provider) = embedding_provider {
            stats.provider = Some(provider.config().provider);
            stats.model = Some(provider.config().model.clone());
            stats.dimensions = Some(provider.config().dimensions);
        }
        for record in embeddings.values() {
            let ready = entries.get(&record.memory_id).is_some_and(|entry| {
                embedding_provider
                    .map(|provider| embedding_record_matches(record, entry, provider))
                    .unwrap_or(record.vector.len() == record.dimensions)
            });
            if ready {
                stats.ready += 1;
            } else {
                stats.stale += 1;
            }
        }
        stats
    }

    async fn prune_embeddings(&self, removed: &[String]) {
        if removed.is_empty() {
            return;
        }
        let mut embeddings = self.embeddings.write().await;
        for id in removed {
            embeddings.remove(id);
        }
    }

    async fn retain_embeddings_for_current_entries(&self) {
        let ids = self
            .entries
            .read()
            .await
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        self.embeddings
            .write()
            .await
            .retain(|memory_id, _| ids.contains(memory_id));
    }

    async fn embedding_vector_scores(
        &self,
        text: Option<&str>,
        candidates: &[MemoryEntry],
        embedding_provider: &MemoryEmbeddingProvider,
    ) -> Option<BTreeMap<String, f32>> {
        if !embedding_provider.uses_external_api() || candidates.is_empty() {
            return None;
        }
        let text = normalize_search_text(text)?;
        let query_vector = embedding_provider.embed(&text).await.ok()?;
        let embeddings = self.embeddings.read().await;
        let mut scores = BTreeMap::new();
        for entry in candidates {
            let Some(record) = embeddings.get(&entry.id) else {
                continue;
            };
            if !embedding_record_matches(record, entry, embedding_provider) {
                continue;
            }
            scores.insert(
                entry.id.clone(),
                cosine_similarity(&query_vector, &record.vector),
            );
        }
        Some(scores)
    }

    async fn track_search_access(&self, results: &mut [MemorySearchResult]) {
        if results.is_empty() {
            return;
        }
        let mut entries = self.entries.write().await;
        let now = Utc::now();
        let mut changed = false;
        for result in results {
            if let Some(entry) = entries.get_mut(&result.entry.id) {
                entry.last_accessed_at = Some(now);
                entry.access_count = entry.access_count.saturating_add(1);
                result.entry = entry.clone();
                changed = true;
            }
        }
        drop(entries);
        if changed {
            let _ = self.flush().await;
        }
    }

    pub async fn recent(&self, limit: usize) -> Vec<MemoryEntry> {
        let _ = self.initialize().await;
        let mut entries = self
            .entries
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        entries.sort_by_key(|e| std::cmp::Reverse(e.created_at));
        entries.truncate(limit);
        entries
    }

    pub async fn stats(&self) -> MemoryStats {
        let _ = self.initialize().await;
        let entries = self.entries.read().await;
        let mut by_kind = BTreeMap::new();
        let mut by_category = BTreeMap::new();
        for entry in entries.values() {
            *by_kind.entry(entry.kind).or_insert(0) += 1;
            *by_category.entry(entry.category.clone()).or_insert(0) += 1;
        }
        MemoryStats {
            total_entries: entries.len(),
            by_kind,
            by_category,
        }
    }

    pub async fn save_snapshot(&self, reason: String) -> anyhow::Result<MemorySnapshotInfo> {
        let _ = self.initialize().await;
        let entries = self
            .entries
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut version = self.current_snapshot_version.write().await;
        *version += 1;
        let snapshot = MemorySnapshot {
            version: *version,
            timestamp: Utc::now(),
            entry_count: entries.len(),
            entries,
            reason: normalize_snapshot_reason(reason),
        };
        let info = snapshot.info();
        let mut snapshots = self.snapshots.write().await;
        snapshots.push(snapshot);
        let overflow = snapshots.len().saturating_sub(MEMORY_SNAPSHOT_LIMIT);
        if overflow > 0 {
            snapshots.drain(0..overflow);
        }
        drop(snapshots);
        drop(version);
        self.flush().await?;
        Ok(info)
    }

    pub async fn list_snapshots(&self) -> Vec<MemorySnapshotInfo> {
        let _ = self.initialize().await;
        self.snapshots
            .read()
            .await
            .iter()
            .map(MemorySnapshot::info)
            .collect()
    }

    pub async fn rollback_snapshot(
        &self,
        version: u64,
    ) -> anyhow::Result<Option<MemorySnapshotInfo>> {
        let _ = self.initialize().await;
        let snapshot = self
            .snapshots
            .read()
            .await
            .iter()
            .find(|snapshot| snapshot.version == version)
            .cloned();
        let Some(snapshot) = snapshot else {
            return Ok(None);
        };
        let restored_entries = snapshot
            .entries
            .iter()
            .cloned()
            .map(|entry| (entry.id.clone(), entry))
            .collect();
        *self.entries.write().await = restored_entries;
        self.retain_embeddings_for_current_entries().await;
        let info = snapshot.info();
        self.flush().await?;
        Ok(Some(info))
    }

    pub async fn consolidate(&self) -> MemoryConsolidationResult {
        let _ = self.initialize().await;
        let mut entries = self.entries.write().await;
        let result = consolidate_entries(&mut entries, Utc::now());
        let changed = result.merged > 0 || result.pruned > 0 || result.distilled > 0;
        drop(entries);
        if changed {
            let _ = self.flush().await;
        }
        result
    }

    pub async fn apply_forgetting(&self, policy: ForgettingPolicy) -> ForgettingResult {
        let _ = self.initialize().await;
        let mut entries = self.entries.write().await;
        let result = apply_forgetting_to_entries(&mut entries, &policy, Utc::now());
        let changed = !result.decayed.is_empty() || !result.dropped.is_empty();
        drop(entries);
        if changed {
            let _ = self.flush().await;
        }
        result
    }

    pub async fn detect_conflicts(&self, input: &NewMemoryEntry) -> Vec<MemoryConflict> {
        let _ = self.initialize().await;
        let category = normalize_category(input.category.clone(), input.kind);
        let content = input.content.trim().to_string();
        let entries = self.entries.read().await;
        let mut conflicts = entries
            .values()
            .filter(|entry| entry.category == category)
            .filter_map(|entry| {
                let similarity = jaccard_similarity(&entry.content, &content);
                (similarity >= MEMORY_CONFLICT_PENDING_THRESHOLD).then(|| {
                    let (resolution, reason) =
                        resolve_memory_conflict(entry, input.importance, similarity);
                    MemoryConflict {
                        existing_id: entry.id.clone(),
                        new_content: content.clone(),
                        similarity,
                        resolution,
                        reason: reason.to_string(),
                    }
                })
            })
            .collect::<Vec<_>>();
        conflicts.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(Ordering::Equal)
        });
        conflicts
    }

    pub async fn ingest_memory_candidate(&self, input: MemoryIngestInput) -> MemoryIngestResult {
        let decision =
            classify_memory_write(&input.entry.content, &input.entry.category, &input.source);
        if decision == MemoryWriteDecision::Deny {
            return MemoryIngestResult {
                status: MemoryIngestStatus::Denied,
                memory: None,
                pending: None,
                conflicts: Vec::new(),
            };
        }

        let conflicts = self.detect_conflicts(&input.entry).await;
        let pending_conflict = conflicts
            .iter()
            .find(|conflict| conflict.resolution == MemoryConflictResolution::Pending);
        if decision == MemoryWriteDecision::Pending || pending_conflict.is_some() {
            let reason = pending_reason(
                &input.reason,
                pending_conflict.map(|conflict| conflict.reason.as_str()),
            );
            let pending = self
                .propose_pending(PendingMemoryInput {
                    content: input.entry.content,
                    category: input.entry.category,
                    importance: score_to_pending_importance(input.entry.importance),
                    source: input.source,
                    tags: input.entry.tags,
                    confidence: input.confidence,
                    reason,
                    session_id: input.session_id,
                    task_id: input.task_id,
                    run_id: input.run_id,
                    trace_id: input.trace_id,
                })
                .await;
            return MemoryIngestResult {
                status: if pending.is_some() {
                    MemoryIngestStatus::Pending
                } else {
                    MemoryIngestStatus::Skipped
                },
                memory: None,
                pending,
                conflicts,
            };
        }

        if conflicts
            .first()
            .is_some_and(|conflict| conflict.resolution == MemoryConflictResolution::KeepExisting)
        {
            return MemoryIngestResult {
                status: MemoryIngestStatus::Skipped,
                memory: None,
                pending: None,
                conflicts,
            };
        }

        let memory = self.add_memory(input.entry).await;
        MemoryIngestResult {
            status: MemoryIngestStatus::Stored,
            memory: Some(memory),
            pending: None,
            conflicts,
        }
    }

    pub async fn propose_pending(&self, input: PendingMemoryInput) -> Option<PendingMemory> {
        let _ = self.initialize().await;
        let content = input.content.trim();
        if content.is_empty() || is_sensitive(content) {
            return None;
        }
        let category = input.category.trim().to_ascii_lowercase();
        let normalized = normalize_content(content);
        let now = Utc::now().timestamp_millis();
        {
            let pending = self.pending.read().await;
            if pending.values().any(|item| {
                item.status == PendingMemoryStatus::Rejected
                    && item.rejected_until.map(|until| until > now).unwrap_or(true)
                    && normalize_content(&item.content) == normalized
                    && item.category == category
            }) {
                return None;
            }
            if let Some(existing) = pending.values().find(|item| {
                item.category == category && normalize_content(&item.content) == normalized
            }) {
                return (existing.status == PendingMemoryStatus::Pending).then(|| existing.clone());
            }
        }
        let item = PendingMemory {
            id: format!("pmem_{}", Uuid::new_v4().simple()),
            content: content.to_string(),
            category,
            importance: input.importance.clamp(1, 10),
            source: if input.source.trim().is_empty() {
                "runtime".to_string()
            } else {
                input.source
            },
            tags: normalize_tags(input.tags),
            confidence: input.confidence,
            reason: if input.reason.trim().is_empty() {
                "memory_policy_pending".to_string()
            } else {
                input.reason
            },
            session_id: input.session_id,
            task_id: input.task_id,
            run_id: input.run_id,
            trace_id: input.trace_id,
            status: PendingMemoryStatus::Pending,
            created_at: now,
            decided_at: None,
            rejected_until: None,
        };
        self.pending
            .write()
            .await
            .insert(item.id.clone(), item.clone());
        let _ = self.flush().await;
        Some(item)
    }

    pub async fn list_pending(&self, include_decided: bool, limit: usize) -> Vec<PendingMemory> {
        let _ = self.initialize().await;
        let now = Utc::now().timestamp_millis();
        let mut items = self
            .pending
            .read()
            .await
            .values()
            .filter(|item| include_decided || item.status == PendingMemoryStatus::Pending)
            .filter(|item| {
                item.status != PendingMemoryStatus::Rejected
                    || item.rejected_until.map(|until| until > now).unwrap_or(true)
            })
            .cloned()
            .collect::<Vec<_>>();
        items.sort_by_key(|i| std::cmp::Reverse(i.created_at));
        items.truncate(limit.clamp(1, 500));
        items
    }

    pub async fn approve_pending(
        &self,
        id: &str,
        edits: PendingMemoryEdit,
    ) -> Option<(PendingMemory, MemoryEntry)> {
        let _ = self.initialize().await;
        let approved = {
            let mut pending = self.pending.write().await;
            let item = pending.get_mut(id)?;
            if item.status != PendingMemoryStatus::Pending {
                return None;
            }
            if let Some(content) = edits.content {
                item.content = content.trim().to_string();
            }
            if item.content.is_empty() {
                return None;
            }
            if let Some(category) = edits.category {
                item.category = category.trim().to_ascii_lowercase();
            }
            if let Some(importance) = edits.importance {
                item.importance = importance.clamp(1, 10);
            }
            if let Some(tags) = edits.tags {
                item.tags = normalize_tags(tags);
            }
            item.status = PendingMemoryStatus::Approved;
            item.decided_at = Some(Utc::now().timestamp_millis());
            item.clone()
        };
        let memory = self
            .add_memory(NewMemoryEntry {
                kind: kind_for_category(&approved.category),
                category: approved.category.clone(),
                content: approved.content.clone(),
                tags: approved.tags.clone(),
                context: approved.session_id.clone(),
                importance: (approved.importance.clamp(1, 10) as f32) / 10.0,
            })
            .await;
        Some((approved, memory))
    }

    pub async fn reject_pending(&self, id: &str, ttl_ms: Option<i64>) -> Option<PendingMemory> {
        let _ = self.initialize().await;
        let rejected = {
            let mut pending = self.pending.write().await;
            let item = pending.get_mut(id)?;
            if item.status != PendingMemoryStatus::Pending {
                return None;
            }
            let now = Utc::now().timestamp_millis();
            item.status = PendingMemoryStatus::Rejected;
            item.decided_at = Some(now);
            item.rejected_until = Some(now + ttl_ms.unwrap_or(14 * 24 * 3_600_000));
            item.clone()
        };
        let _ = self.flush().await;
        Some(rejected)
    }

    pub async fn flush(&self) -> anyhow::Result<()> {
        let Some(path) = &self.persist_path else {
            return Ok(());
        };
        let _guard = self.write_lock.lock().await;
        let data = PersistedMemoryData {
            version: 2,
            entries: self.entries.read().await.values().cloned().collect(),
            embeddings: self.embeddings.read().await.values().cloned().collect(),
            pending: self.pending.read().await.values().cloned().collect(),
            snapshots: self.snapshots.read().await.clone(),
            current_snapshot_version: *self.current_snapshot_version.read().await,
            updated_at: Utc::now(),
        };
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let text = serde_json::to_string_pretty(&data)?;
        let tmp = path.with_extension("json.tmp");
        tokio::fs::write(&tmp, format!("{text}\n")).await?;
        tokio::fs::rename(tmp, path).await?;
        Ok(())
    }
}

impl Default for MemoryService {
    fn default() -> Self {
        Self::new()
    }
}

pub fn extract_memories_from_messages(messages: &[ExtractionMessage]) -> MemoryExtractionResult {
    if messages.is_empty() {
        return MemoryExtractionResult::default();
    }

    let mut memories = Vec::new();
    let mut files_modified = BTreeSet::new();
    let mut tools_used = BTreeSet::new();
    let mut key_decisions = Vec::new();
    let mut errors = Vec::new();

    for message in messages {
        let content = message.content.trim();
        if content.is_empty() {
            continue;
        }

        for matched in file_path_regex().find_iter(content) {
            files_modified.insert(matched.as_str().to_string());
        }
        for tool in &message.tool_names {
            let tool = tool.trim();
            if !tool.is_empty() {
                tools_used.insert(tool.to_string());
            }
        }
        for pattern in decision_patterns() {
            for captures in pattern.captures_iter(content) {
                if let Some(value) = captures.get(1).map(|value| value.as_str().trim()) {
                    push_unique_limited(&mut key_decisions, value, 20);
                }
            }
        }
        for pattern in error_patterns() {
            for captures in pattern.captures_iter(content) {
                if let Some(value) = captures.get(1).map(|value| value.as_str().trim()) {
                    push_unique_limited(&mut errors, value, 20);
                }
            }
        }

        match message.role {
            ExtractionRole::User if content.chars().count() > 20 => {
                memories.push(extracted_memory(
                    "task_state",
                    truncate_chars(content, 200),
                    0.6,
                    extract_quick_tags(content),
                    Some("conversation".to_string()),
                ));
            }
            ExtractionRole::Assistant
                if content.chars().count() > 50 && content.contains("```") =>
            {
                memories.push(extracted_memory(
                    "code_knowledge",
                    format!("Code solution: {}", truncate_chars(content, 150)),
                    0.7,
                    extract_quick_tags(content),
                    Some("conversation".to_string()),
                ));
            }
            _ => {}
        }
    }

    if !key_decisions.is_empty() {
        memories.push(extracted_memory(
            "decision",
            format!("Key decisions: {}", key_decisions.join("; ")),
            0.8,
            vec!["decision".to_string()],
            Some("conversation".to_string()),
        ));
    }
    if !errors.is_empty() {
        memories.push(extracted_memory(
            "error_experience",
            format!(
                "Errors encountered: {}",
                errors
                    .iter()
                    .take(5)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
            0.7,
            vec!["error".to_string()],
            Some("conversation".to_string()),
        ));
    }
    if !files_modified.is_empty() {
        let files = files_modified.iter().cloned().collect::<Vec<_>>();
        memories.push(extracted_memory(
            "project_fact",
            format!("Files modified: {}", files.join(", ")),
            0.6,
            std::iter::once("file-change".to_string())
                .chain(files.iter().take(5).cloned())
                .collect(),
            Some("conversation".to_string()),
        ));
    }

    let files_modified = files_modified.into_iter().collect::<Vec<_>>();
    let tools_used = tools_used.into_iter().collect::<Vec<_>>();
    let summary =
        generate_basic_extraction_summary(messages, &key_decisions, &files_modified, &tools_used);

    MemoryExtractionResult {
        memories,
        summary,
        key_decisions,
        files_modified,
        tools_used,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedMemoryData {
    version: u32,
    #[serde(default, alias = "memories")]
    entries: Vec<MemoryEntry>,
    #[serde(default)]
    embeddings: Vec<MemoryEmbeddingRecord>,
    #[serde(default)]
    pending: Vec<PendingMemory>,
    #[serde(default)]
    snapshots: Vec<MemorySnapshot>,
    #[serde(default)]
    current_snapshot_version: u64,
    updated_at: DateTime<Utc>,
}

impl Default for PersistedMemoryData {
    fn default() -> Self {
        Self {
            version: 2,
            entries: Vec::new(),
            embeddings: Vec::new(),
            pending: Vec::new(),
            snapshots: Vec::new(),
            current_snapshot_version: 0,
            updated_at: Utc::now(),
        }
    }
}

impl MemoryEmbeddingRecord {
    fn from_entry(
        entry: &MemoryEntry,
        embedding_provider: &MemoryEmbeddingProvider,
        vector: Vec<f32>,
        updated_at: DateTime<Utc>,
    ) -> Self {
        let text = memory_vector_haystack(entry);
        Self {
            memory_id: entry.id.clone(),
            provider: embedding_provider.config().provider,
            model: embedding_provider.config().model.clone(),
            dimensions: embedding_provider.config().dimensions,
            content_hash: memory_embedding_content_hash(&text),
            vector,
            updated_at,
        }
    }
}

fn embedding_record_matches(
    record: &MemoryEmbeddingRecord,
    entry: &MemoryEntry,
    embedding_provider: &MemoryEmbeddingProvider,
) -> bool {
    record.memory_id == entry.id
        && record.provider == embedding_provider.config().provider
        && record.model == embedding_provider.config().model.as_str()
        && record.dimensions == embedding_provider.config().dimensions
        && record.vector.len() == record.dimensions
        && record.content_hash == memory_embedding_content_hash(&memory_vector_haystack(entry))
}

fn memory_embedding_content_hash(text: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

impl MemorySnapshot {
    fn info(&self) -> MemorySnapshotInfo {
        MemorySnapshotInfo {
            version: self.version,
            timestamp: self.timestamp,
            entry_count: self.entry_count,
            reason: self.reason.clone(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MemoryUpdate {
    pub kind: Option<MemoryKind>,
    pub category: Option<String>,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub importance: Option<f32>,
}

pub fn kind_for_category(category: &str) -> MemoryKind {
    let category = category_alias(&normalize_category_value(category));
    match category.as_str() {
        "workflow_rule" => MemoryKind::Procedural,
        "identity" => MemoryKind::SelfArchive,
        "user_profile" => MemoryKind::UserProfile,
        "user_preference" => MemoryKind::Preference,
        "project_fact" => MemoryKind::ProjectProfile,
        "voice_profile" => MemoryKind::VoiceProfile,
        "safety_note" => MemoryKind::SafetyNote,
        "task_lesson" | "task_state" | "error_pattern" | "error_experience" | "session"
        | "validation.failures" | "validation_failure" => {
            MemoryKind::Episodic
        }
        "relationship" => MemoryKind::Relationship,
        _ => MemoryKind::Semantic,
    }
}

pub fn classify_memory_write(content: &str, category: &str, source: &str) -> MemoryWriteDecision {
    if is_sensitive(content) {
        return MemoryWriteDecision::Deny;
    }
    let source = source.trim().to_ascii_lowercase();
    if source.starts_with("explicit:file") || source == "web" {
        return MemoryWriteDecision::Direct;
    }
    match category_alias(&normalize_category_value(category)).as_str() {
        "user_profile" | "user_preference" | "decision" => MemoryWriteDecision::Pending,
        _ => MemoryWriteDecision::Direct,
    }
}

fn resolve_memory_conflict(
    existing: &MemoryEntry,
    incoming_importance: f32,
    similarity: f32,
) -> (MemoryConflictResolution, &'static str) {
    if similarity > 0.9 {
        if incoming_importance >= existing.importance {
            (
                MemoryConflictResolution::Replace,
                "near_duplicate_incoming_higher",
            )
        } else {
            (
                MemoryConflictResolution::KeepExisting,
                "near_duplicate_existing_higher",
            )
        }
    } else if similarity > MEMORY_DUPLICATE_MERGE_THRESHOLD {
        (MemoryConflictResolution::Merge, "high_similarity")
    } else {
        (
            MemoryConflictResolution::Pending,
            "moderate_similarity_needs_review",
        )
    }
}

fn score_to_pending_importance(value: f32) -> u8 {
    ((value.clamp(0.0, 1.0) * 10.0).round() as u8).clamp(1, 10)
}

fn pending_reason(base: &str, conflict_reason: Option<&str>) -> String {
    let base = base.trim();
    match (base.is_empty(), conflict_reason) {
        (true, Some(reason)) => reason.to_string(),
        (false, Some(reason)) => format!("{base} ({reason})"),
        (false, None) => base.to_string(),
        (true, None) => "memory_policy_pending".to_string(),
    }
}

fn normalize_snapshot_reason(reason: String) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        "manual_snapshot".to_string()
    } else {
        reason.chars().take(240).collect()
    }
}

pub fn memory_session_tag(session_id: &str) -> String {
    format!(
        "{MEMORY_SESSION_TAG_PREFIX}{}",
        normalize_session_id(session_id)
    )
}

pub fn memory_entry_session_id(entry: &MemoryEntry) -> Option<String> {
    entry
        .tags
        .iter()
        .find_map(|tag| tag.strip_prefix(MEMORY_SESSION_TAG_PREFIX))
        .map(str::to_string)
}

pub fn has_memory_session_tag(tags: &[String]) -> bool {
    tags.iter()
        .any(|tag| tag.starts_with(MEMORY_SESSION_TAG_PREFIX))
}

pub fn with_memory_session_tag(tags: Vec<String>, session_id: Option<&str>) -> Vec<String> {
    let Some(session_id) = session_id
        .map(normalize_session_id)
        .filter(|session_id| !session_id.is_empty())
    else {
        return normalize_tags(tags);
    };
    let mut tags = normalize_tags(tags);
    let tag = format!("{MEMORY_SESSION_TAG_PREFIX}{session_id}");
    if !tags.iter().any(|existing| existing == &tag) {
        tags.push(tag);
        tags.sort();
    }
    tags
}

fn matches_memory_session_scope(
    entry: &MemoryEntry,
    session_id: Option<&str>,
    include_cross_session: bool,
) -> bool {
    if include_cross_session {
        return true;
    }
    let Some(session_id) = session_id
        .map(normalize_session_id)
        .filter(|session_id| !session_id.is_empty())
    else {
        return true;
    };
    memory_entry_session_id(entry).is_none_or(|entry_session| entry_session == session_id)
}

fn normalize_session_id(session_id: &str) -> String {
    session_id.trim().to_ascii_lowercase()
}

fn normalize_context_filter(context: Option<&str>) -> Option<String> {
    context
        .map(normalize_context_value)
        .filter(|context| !context.is_empty())
}

fn normalize_context_value(context: &str) -> String {
    context.trim().to_ascii_lowercase()
}

fn score_entry(
    entry: &MemoryEntry,
    text: Option<&str>,
    kind: Option<MemoryKind>,
    tags: &[String],
    min_importance: Option<f32>,
    vector_score_override: Option<f32>,
) -> Option<MemorySearchResult> {
    if kind.is_some_and(|kind| kind != entry.kind) {
        return None;
    }
    if min_importance.is_some_and(|min| entry.importance < min) {
        return None;
    }
    let mut reasons = Vec::new();
    let mut keyword_score: f32 = 0.0;
    let mut vector_score: f32 = 0.0;
    let mut tag_score: f32 = 0.0;
    let mut context_score: f32 = 0.0;
    let content = entry.content.to_ascii_lowercase();
    let context = entry
        .context
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let haystack = memory_vector_haystack(entry);
    if let Some(text) = normalize_search_text(text) {
        let terms = query_terms(&text);
        if content.contains(&text) {
            keyword_score = 1.0;
            push_reason(&mut reasons, "content_match");
        }
        if context.contains(&text) {
            context_score = 1.0;
            push_reason(&mut reasons, "context_match");
        }
        if !terms.is_empty() {
            let matched = terms
                .iter()
                .filter(|term| haystack.contains(term.as_str()))
                .count();
            if matched > 0 {
                keyword_score = keyword_score.max(matched as f32 / terms.len() as f32);
                push_reason(&mut reasons, "keyword_match");
            }
            let query_tag_score = tag_overlap_score(&terms, &entry.tags);
            if query_tag_score > 0.0 {
                tag_score = tag_score.max(query_tag_score);
                push_reason(&mut reasons, "tag_match");
            }
        }
        vector_score = vector_score_override.unwrap_or_else(|| {
            cosine_similarity(
                &embed_text(&text, MEMORY_VECTOR_SIZE),
                &embed_text(&haystack, MEMORY_VECTOR_SIZE),
            )
        });
        if vector_score >= MEMORY_VECTOR_MATCH_THRESHOLD {
            push_reason(&mut reasons, "semantic_match");
        }
        if keyword_score == 0.0
            && tag_score == 0.0
            && context_score == 0.0
            && vector_score < MEMORY_VECTOR_MATCH_THRESHOLD
        {
            return None;
        }
    }
    if !tags.is_empty() {
        let explicit_tag_score = tag_overlap_score(tags, &entry.tags);
        if explicit_tag_score == 0.0 {
            return None;
        }
        tag_score = tag_score.max(explicit_tag_score);
        push_reason(&mut reasons, "tag_match");
    }
    let recency = recency_score(entry);
    if recency > 0.7 {
        push_reason(&mut reasons, "recent");
    }
    let frequency = frequency_score(entry.access_count);
    if entry.access_count > 0 {
        push_reason(&mut reasons, "frequent");
    }
    let importance_score = entry.importance.clamp(0.0, 1.0);
    let score = vector_score * MEMORY_RECALL_VECTOR_WEIGHT
        + keyword_score * MEMORY_RECALL_KEYWORD_WEIGHT
        + tag_score * MEMORY_RECALL_TAG_WEIGHT
        + recency * MEMORY_RECALL_RECENCY_WEIGHT
        + importance_score * MEMORY_RECALL_IMPORTANCE_WEIGHT
        + frequency * MEMORY_RECALL_FREQUENCY_WEIGHT
        + context_score * MEMORY_RECALL_CONTEXT_WEIGHT;
    if reasons.is_empty() {
        push_reason(&mut reasons, "importance");
    }
    Some(MemorySearchResult {
        entry: entry.clone(),
        score,
        reasons,
    })
}

fn memory_vector_haystack(entry: &MemoryEntry) -> String {
    let content = entry.content.to_ascii_lowercase();
    let context = entry
        .context
        .as_deref()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let tags_text = entry.tags.join(" ").to_ascii_lowercase();
    let category = entry.category.to_ascii_lowercase();
    format!("{content}\n{context}\n{tags_text}\n{category}")
}

fn normalize_search_text(text: Option<&str>) -> Option<String> {
    text.map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_ascii_lowercase)
}

fn tag_overlap_score(query_tags: &[String], entry_tags: &[String]) -> f32 {
    if query_tags.is_empty() || entry_tags.is_empty() {
        return 0.0;
    }
    let matched = query_tags
        .iter()
        .filter(|tag| entry_tags.iter().any(|entry_tag| entry_tag == *tag))
        .count();
    if matched == 0 {
        0.0
    } else {
        matched as f32 / query_tags.len().max(entry_tags.len()) as f32
    }
}

fn push_reason(reasons: &mut Vec<String>, reason: &str) {
    if !reasons.iter().any(|existing| existing == reason) {
        reasons.push(reason.to_string());
    }
}

fn normalize_tags(tags: Vec<String>) -> Vec<String> {
    let mut tags = tags
        .into_iter()
        .map(|tag| tag.trim().to_ascii_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();
    tags.sort();
    tags.dedup();
    tags.truncate(24);
    tags
}

fn merge_tags(existing: &[String], incoming: &[String]) -> Vec<String> {
    normalize_tags(
        existing
            .iter()
            .cloned()
            .chain(incoming.iter().cloned())
            .collect(),
    )
}

fn choose_better_content(current: &str, next: &str) -> String {
    let current = current.trim();
    let next = next.trim();
    if current.is_empty() {
        return next.to_string();
    }
    if next.len() > current.len() && (next.len() as f32) < (current.len() as f32 * 2.5) {
        next.to_string()
    } else {
        current.to_string()
    }
}

fn enforce_memory_capacity(entries: &mut BTreeMap<String, MemoryEntry>) -> Vec<String> {
    if entries.len() <= MEMORY_MAX_ENTRIES {
        return Vec::new();
    }
    let mut candidates = entries
        .values()
        .map(|entry| {
            (
                entry.id.clone(),
                entry.importance,
                entry.last_accessed_at.unwrap_or(entry.created_at),
            )
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        a.1.partial_cmp(&b.1)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.0.cmp(&b.0))
    });
    let target_removals = entries.len().saturating_sub(MEMORY_MAX_ENTRIES);
    let mut removed = Vec::new();
    for (id, importance, _) in candidates.into_iter().take(target_removals) {
        if importance >= MEMORY_CLEANUP_IMPORTANCE_THRESHOLD {
            break;
        }
        if entries.remove(&id).is_some() {
            removed.push(id);
        }
    }
    removed
}

fn find_duplicate_entry_id(
    entries: &BTreeMap<String, MemoryEntry>,
    content: &str,
    category: &str,
    context: Option<&str>,
) -> Option<String> {
    let normalized = normalize_content(content);
    let context = context.map(str::trim).filter(|value| !value.is_empty());
    entries.values().find_map(|entry| {
        if entry.category != category {
            return None;
        }
        if let (Some(incoming), Some(existing)) = (context, entry.context.as_deref()) {
            if existing != incoming {
                return None;
            }
        }
        if normalize_content(&entry.content) == normalized
            || jaccard_similarity(&entry.content, content) >= MEMORY_DUPLICATE_MERGE_THRESHOLD
        {
            Some(entry.id.clone())
        } else {
            None
        }
    })
}

fn jaccard_similarity(a: &str, b: &str) -> f32 {
    let a_tokens = similarity_tokens(a);
    let b_tokens = similarity_tokens(b);
    if a_tokens.is_empty() && b_tokens.is_empty() {
        return 0.0;
    }
    let intersection = a_tokens.intersection(&b_tokens).count();
    let union = a_tokens.len() + b_tokens.len() - intersection;
    if union == 0 {
        0.0
    } else {
        intersection as f32 / union as f32
    }
}

fn similarity_tokens(text: &str) -> BTreeSet<String> {
    let mut tokens = tokenize_for_vector(text)
        .into_iter()
        .collect::<BTreeSet<_>>();
    if tokens.is_empty() {
        tokens.extend(
            normalize_content(text)
                .split_whitespace()
                .map(str::to_string)
                .filter(|token| !token.is_empty()),
        );
    }
    tokens
}

fn recency_score(entry: &MemoryEntry) -> f32 {
    const DAY_MS: f32 = 86_400_000.0;
    let reference = entry.last_accessed_at.unwrap_or(entry.created_at);
    let age_ms = (Utc::now() - reference).num_milliseconds().max(0) as f32;
    let half_life_ms = 14.0 * DAY_MS;
    (-std::f32::consts::LN_2 * age_ms / half_life_ms).exp()
}

fn frequency_score(access_count: u64) -> f32 {
    match access_count {
        0 => 0.0,
        1 | 2 => 0.3,
        3..=5 => 0.55,
        6..=10 => 0.75,
        _ => 1.0,
    }
}

fn extracted_memory(
    category: &str,
    content: String,
    importance: f32,
    tags: Vec<String>,
    context: Option<String>,
) -> NewMemoryEntry {
    NewMemoryEntry {
        kind: kind_for_category(category),
        category: category.to_string(),
        content,
        tags,
        context,
        importance,
    }
}

fn file_path_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"[\w\-./]+\.(?:ts|js|tsx|jsx|py|rs|go|java|cpp|c|h|css|html|json|yaml|yml|md|txt|sh|bat|sql|xml|toml|cfg|ini|env)")
            .expect("valid file path regex")
    })
}

fn decision_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r"(?i)(?:决定|选择|采用|使用|decided?|chose|using|adopted?)\s+(.{10,80})",
            r"(?i)(?:方案|策略|approach|strategy|solution)\s*[:：]\s*(.{10,80})",
            r"(?i)(?:改为|切换到|switched?\s+to|changed?\s+to)\s+(.{5,60})",
        ]
        .into_iter()
        .map(|pattern| Regex::new(pattern).expect("valid decision regex"))
        .collect()
    })
}

fn error_patterns() -> &'static [Regex] {
    static PATTERNS: OnceLock<Vec<Regex>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        [
            r"(?i)(?:error|错误|失败|failed|failure)\s*[:：]\s*(.{10,120})",
            r"(?i)(?:bug|issue|问题)\s*[:：]\s*(.{10,120})",
        ]
        .into_iter()
        .map(|pattern| Regex::new(pattern).expect("valid error regex"))
        .collect()
    })
}

fn push_unique_limited(values: &mut Vec<String>, value: &str, limit: usize) {
    let value = compact_extracted_line(value);
    if value.is_empty() || values.iter().any(|item| item == &value) || values.len() >= limit {
        return;
    }
    values.push(value);
}

fn compact_extracted_line(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|ch: char| matches!(ch, '.' | ',' | ';' | '，' | '。' | '；'))
        .trim()
        .to_string()
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut text = value.chars().take(max_chars).collect::<String>();
    if value.chars().count() > max_chars {
        text.push('…');
    }
    text
}

fn extract_quick_tags(text: &str) -> Vec<String> {
    let mut tags = Vec::new();
    for matched in extension_regex().find_iter(text).take(3) {
        tags.push(
            matched
                .as_str()
                .trim_start_matches('.')
                .to_ascii_lowercase(),
        );
    }
    for matched in technical_term_regex().find_iter(text) {
        push_unique_limited(&mut tags, matched.as_str(), 8);
        if tags.len() >= 8 {
            break;
        }
    }
    normalize_tags(tags)
}

fn extension_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\.\w{2,4}\b").expect("valid extension regex"))
}

fn technical_term_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\b[a-z][a-z0-9_]{3,}\b").expect("valid technical term regex"))
}

fn generate_basic_extraction_summary(
    messages: &[ExtractionMessage],
    decisions: &[String],
    files: &[String],
    tools: &[String],
) -> String {
    let mut parts = Vec::new();
    if let Some(first_user) = messages
        .iter()
        .find(|message| message.role == ExtractionRole::User && !message.content.trim().is_empty())
    {
        parts.push(format!(
            "User requested: {}",
            truncate_chars(first_user.content.trim(), 100)
        ));
    }
    if !decisions.is_empty() {
        parts.push(format!(
            "Decisions: {}",
            decisions
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    if !files.is_empty() {
        parts.push(format!(
            "Files: {}",
            files.iter().take(5).cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    if !tools.is_empty() {
        parts.push(format!("Tools: {}", tools.join(", ")));
    }
    if parts.is_empty() {
        "Conversation session".to_string()
    } else {
        parts.join(". ")
    }
}

fn consolidate_entries(
    entries: &mut BTreeMap<String, MemoryEntry>,
    now: DateTime<Utc>,
) -> MemoryConsolidationResult {
    let mut by_scope: BTreeMap<(String, Option<String>), Vec<String>> = BTreeMap::new();
    for entry in entries.values() {
        by_scope
            .entry((
                entry.category.clone(),
                normalized_context_key(&entry.context),
            ))
            .or_default()
            .push(entry.id.clone());
    }

    let mut removed = BTreeSet::new();
    let mut merged = 0usize;
    for ids in by_scope.values_mut() {
        ids.sort_by_key(|id| {
            entries
                .get(id)
                .map(|entry| entry.content.len())
                .unwrap_or(0)
        });
        for i in 0..ids.len() {
            let base_id = ids[i].clone();
            if removed.contains(&base_id) {
                continue;
            }
            for candidate_id in ids.iter().skip(i + 1) {
                if removed.contains(candidate_id) {
                    continue;
                }
                let Some(base) = entries.get(&base_id) else {
                    continue;
                };
                let Some(candidate) = entries.get(candidate_id) else {
                    continue;
                };
                if length_similarity_ratio(&base.content, &candidate.content) < 0.5 {
                    continue;
                }
                if jaccard_similarity(&base.content, &candidate.content)
                    < MEMORY_DUPLICATE_MERGE_THRESHOLD
                {
                    continue;
                }
                let candidate = candidate.clone();
                if let Some(base) = entries.get_mut(&base_id) {
                    merge_memory_entry(base, &candidate, now);
                    removed.insert(candidate_id.clone());
                    merged += 1;
                }
            }
        }
    }

    let pruned = removed.len();
    for id in removed {
        entries.remove(&id);
    }

    MemoryConsolidationResult {
        merged,
        distilled: 0,
        pruned,
    }
}

fn merge_memory_entry(base: &mut MemoryEntry, incoming: &MemoryEntry, now: DateTime<Utc>) {
    base.content = choose_better_content(&base.content, &incoming.content);
    base.importance = base.importance.max(incoming.importance);
    base.tags = merge_tags(&base.tags, &incoming.tags);
    if base.context.is_none() {
        base.context = incoming.context.clone();
    }
    base.created_at = base.created_at.min(incoming.created_at);
    base.updated_at = now;
    base.last_accessed_at = match (base.last_accessed_at, incoming.last_accessed_at) {
        (Some(left), Some(right)) => Some(left.max(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    };
    base.access_count = base.access_count.saturating_add(incoming.access_count);
}

fn normalized_context_key(context: &Option<String>) -> Option<String> {
    context
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn length_similarity_ratio(left: &str, right: &str) -> f32 {
    let shorter = left.len().min(right.len()) as f32;
    let longer = left.len().max(right.len()) as f32;
    if longer <= 0.0 {
        0.0
    } else {
        shorter / longer
    }
}

fn apply_forgetting_to_entries(
    entries: &mut BTreeMap<String, MemoryEntry>,
    policy: &ForgettingPolicy,
    now: DateTime<Utc>,
) -> ForgettingResult {
    let ids = entries.keys().cloned().collect::<Vec<_>>();
    let mut decayed = Vec::new();
    let mut dropped = Vec::new();

    for id in ids {
        let Some(entry) = entries.get_mut(&id) else {
            continue;
        };
        let age_days = days_between(now, entry.created_at);
        let since_last_access_days = days_between(now, forgetting_last_touch(entry));

        if entry.importance >= 0.7 {
            continue;
        }

        if age_days > policy.max_retention_days as f32 && entry.importance < 0.6 {
            dropped.push(entry.id.clone());
            continue;
        }

        if entry.importance < policy.importance_threshold
            && entry.access_count < policy.retrieval_threshold
            && since_last_access_days > FORGETTING_IDLE_DAYS
        {
            let next = decay_importance(
                entry.importance,
                since_last_access_days,
                policy.decay_rate_per_day,
            );
            if next < entry.importance {
                entry.importance = next;
                entry.updated_at = now;
                decayed.push(entry.id.clone());
            }
        }
    }

    for id in &dropped {
        entries.remove(id);
    }

    ForgettingResult {
        decayed,
        dropped,
        kept: entries.len(),
    }
}

fn forgetting_last_touch(entry: &MemoryEntry) -> DateTime<Utc> {
    match entry.last_accessed_at {
        Some(last_accessed) if last_accessed > entry.updated_at => last_accessed,
        _ => entry.updated_at,
    }
}

fn days_between(now: DateTime<Utc>, then: DateTime<Utc>) -> f32 {
    (now - then).num_milliseconds().max(0) as f32 / DAY_MS
}

fn decay_importance(current: f32, days_since_access: f32, rate: f32) -> f32 {
    let factor = (-rate.max(0.0) * days_since_access.max(0.0)).exp();
    let decayed = ((current * factor) * 100.0).round() / 100.0;
    decayed.clamp(current.min(0.1), 1.0)
}

fn embed_text(text: &str, size: usize) -> Vec<f32> {
    let mut vector = vec![0.0; size.max(1)];
    for token in tokenize_for_vector(text) {
        let hash = hash_token(&token);
        let index = hash as usize % vector.len();
        let sign = if hash.is_multiple_of(2) { 1.0 } else { -1.0 };
        vector[index] += sign;
    }
    let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut vector {
            *value /= norm;
        }
    }
    vector
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let len = a.len().min(b.len());
    let mut dot = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;
    for idx in 0..len {
        dot += a[idx] * b[idx];
        norm_a += a[idx] * a[idx];
        norm_b += b[idx] * b[idx];
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        0.0
    } else {
        (dot / denom).clamp(0.0, 1.0)
    }
}

fn tokenize_for_vector(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut ascii = String::new();
    let mut cjk = String::new();
    for ch in text.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() || matches!(ch, '_' | '.' | '/' | '-') {
            push_cjk_vector_tokens(&mut tokens, &cjk);
            cjk.clear();
            ascii.push(ch);
        } else if is_cjk(ch) {
            if ascii.len() > 1 {
                tokens.push(std::mem::take(&mut ascii));
            } else {
                ascii.clear();
            }
            cjk.push(ch);
        } else {
            if ascii.len() > 1 {
                tokens.push(std::mem::take(&mut ascii));
            } else {
                ascii.clear();
            }
            push_cjk_vector_tokens(&mut tokens, &cjk);
            cjk.clear();
        }
    }
    if ascii.len() > 1 {
        tokens.push(ascii);
    }
    push_cjk_vector_tokens(&mut tokens, &cjk);
    tokens
}

fn push_cjk_vector_tokens(tokens: &mut Vec<String>, segment: &str) {
    let chars = segment.chars().collect::<Vec<_>>();
    if chars.len() > 1 {
        tokens.push(segment.to_string());
        for window in chars.windows(2) {
            tokens.push(window.iter().collect());
        }
    }
}

fn hash_token(token: &str) -> u32 {
    let mut hash = 2_166_136_261u32;
    for ch in token.chars() {
        hash ^= ch as u32;
        hash = hash.wrapping_mul(16_777_619);
    }
    hash
}

fn query_terms(text: &str) -> Vec<String> {
    let mut terms = Vec::new();
    terms.extend(
        text.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '-')
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .filter(|term| term.len() >= 3)
            .filter(|term| {
                !matches!(
                    term.as_str(),
                    "the" | "and" | "for" | "with" | "that" | "this"
                )
            }),
    );
    let mut cjk_segment = String::new();
    for ch in text.chars() {
        if is_cjk(ch) {
            cjk_segment.push(ch);
        } else {
            push_cjk_terms(&mut terms, &cjk_segment);
            cjk_segment.clear();
        }
    }
    push_cjk_terms(&mut terms, &cjk_segment);
    terms.sort();
    terms.dedup();
    terms
}

fn push_cjk_terms(terms: &mut Vec<String>, segment: &str) {
    let chars = segment.chars().collect::<Vec<_>>();
    if chars.len() >= 2 {
        terms.push(segment.to_string());
    }
    for window in chars.windows(2) {
        terms.push(window.iter().collect::<String>());
    }
}

fn is_cjk(ch: char) -> bool {
    matches!(
        ch,
        '\u{4e00}'..='\u{9fff}' | '\u{3400}'..='\u{4dbf}' | '\u{f900}'..='\u{faff}'
    )
}

fn normalize_content(content: &str) -> String {
    content
        .to_ascii_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn is_sensitive(content: &str) -> bool {
    let lower = content.to_ascii_lowercase();
    [
        "api_key",
        "apikey",
        "secret",
        "token",
        "password",
        "id_rsa",
        "id_ed25519",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn normalize_category(category: String, kind: MemoryKind) -> String {
    let category = category_alias(&normalize_category_value(&category));
    if !category.is_empty() {
        return category;
    }
    match kind {
        MemoryKind::Episodic => "session",
        MemoryKind::Semantic => "code_knowledge",
        MemoryKind::Procedural => "workflow_rule",
        MemoryKind::Relationship => "relationship",
        MemoryKind::SelfArchive => "identity",
        MemoryKind::UserProfile => "user_profile",
        MemoryKind::ProjectProfile => "project_fact",
        MemoryKind::VoiceProfile => "voice_profile",
        MemoryKind::Preference => "user_preference",
        MemoryKind::SafetyNote => "safety_note",
    }
    .to_string()
}

fn normalize_category_value(category: &str) -> String {
    let mut normalized = String::new();
    let mut last_was_separator = false;
    for ch in category.trim().chars().flat_map(char::to_lowercase) {
        if ch.is_whitespace() || ch == '-' {
            if !last_was_separator && !normalized.is_empty() {
                normalized.push('_');
                last_was_separator = true;
            }
        } else {
            normalized.push(ch);
            last_was_separator = false;
        }
    }
    normalized.trim_matches('_').to_string()
}

/// Map informal / extractor category tokens onto the stable taxonomy used by
/// kind routing, TTL, and retrieval. This is ingest normalization, not dual
/// tool-name compatibility.
fn category_alias(value: &str) -> String {
    match value {
        "preference" | "user_pref" => "user_preference",
        "persona" | "agent_identity" | "soul" => "identity",
        "user" | "profile" => "user_profile",
        "project" | "project_context" => "project_fact",
        "workflow" | "process" | "rule" => "workflow_rule",
        "lesson" | "experience" => "task_lesson",
        "task" | "todo" | "user_request" => "task_state",
        "error" | "bug" | "failure" => "error_experience",
        "validation.failures" | "validation_failure" | "validation_failures" => "validation.failures",
        "code" | "code_solution" => "code_knowledge",
        _ => value,
    }
    .to_string()
}
