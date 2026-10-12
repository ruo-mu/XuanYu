//! 验证命令运行时状态：记录本轮 exec 的 typecheck/lint/test/build，供回放。

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use xuanyu_storage::runtime_data_root;

const STORE_VERSION: u32 = 1;
const MAX_RECENT_RUNS: usize = 200;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValidationCommandRun {
    pub command: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub cwd: String,
    #[serde(default)]
    pub related_files: Vec<String>,
    pub started_at: u64,
    pub duration_ms: u64,
    pub success: bool,
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub output: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct ValidationStoreData {
    version: u32,
    #[serde(default)]
    commands: Vec<Value>,
    #[serde(default)]
    recent_runs: Vec<ValidationCommandRun>,
    #[serde(default)]
    failures: Vec<Value>,
    updated_at: u64,
}

#[derive(Debug, Clone)]
pub struct ValidationRuntimeStore {
    state_path: PathBuf,
}

impl ValidationRuntimeStore {
    pub fn for_project(project_dir: &Path) -> Self {
        let project_name = project_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("project");
        let root = runtime_validation_root().join(sanitize_segment(project_name));
        let _ = fs::create_dir_all(&root);
        Self {
            state_path: root.join("validation.json"),
        }
    }

    pub fn record_run(&self, run: ValidationCommandRun) {
        let mut data = self.load();
        data.recent_runs.insert(0, run);
        if data.recent_runs.len() > MAX_RECENT_RUNS {
            data.recent_runs.truncate(MAX_RECENT_RUNS);
        }
        data.updated_at = now_ms();
        data.version = STORE_VERSION;
        self.flush(&data);
    }

    pub fn recent_runs_since(&self, since_ms: u64) -> Vec<ValidationCommandRun> {
        self.load()
            .recent_runs
            .into_iter()
            .filter(|run| run.started_at >= since_ms && run.kind != "unknown")
            .collect()
    }

    #[allow(dead_code)]
    pub fn snapshot(&self) -> Value {
        let data = self.load();
        json!({
            "version": data.version,
            "commands": data.commands,
            "recentRuns": data.recent_runs,
            "failures": data.failures,
            "updatedAt": data.updated_at,
            "statePath": path_to_string(&self.state_path)
        })
    }

    fn load(&self) -> ValidationStoreData {
        let Ok(text) = fs::read_to_string(&self.state_path) else {
            return ValidationStoreData {
                version: STORE_VERSION,
                updated_at: now_ms(),
                ..Default::default()
            };
        };
        serde_json::from_str(&text).unwrap_or_else(|_| ValidationStoreData {
            version: STORE_VERSION,
            updated_at: now_ms(),
            ..Default::default()
        })
    }

    fn flush(&self, data: &ValidationStoreData) {
        if let Some(parent) = self.state_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string_pretty(data) {
            let _ = fs::write(&self.state_path, text);
        }
    }
}

/// 从本轮记录中按 typecheck → lint → test（可选 build）各挑一条回放。
pub fn select_replay_validation_commands(
    runs: &[ValidationCommandRun],
    max_commands: usize,
    include_build: bool,
) -> Vec<ValidationCommandRun> {
    let order: &[&str] = if include_build {
        &["typecheck", "lint", "test", "build"]
    } else {
        &["typecheck", "lint", "test"]
    };
    let mut best: Vec<(String, ValidationCommandRun)> = Vec::new();
    for run in runs {
        if !order.contains(&run.kind.as_str()) {
            continue;
        }
        if let Some((_, existing)) = best.iter_mut().find(|(kind, _)| kind == &run.kind) {
            if !existing.success && run.success {
                *existing = run.clone();
            }
            continue;
        }
        best.push((run.kind.clone(), run.clone()));
    }
    let mut selected = Vec::new();
    for kind in order {
        if let Some((_, run)) = best.iter().find(|(k, _)| k == kind) {
            selected.push(run.clone());
            if selected.len() >= max_commands {
                break;
            }
        }
    }
    selected
}

fn runtime_validation_root() -> PathBuf {
    runtime_data_root().join("validation")
}

fn sanitize_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

pub fn now_ms_public() -> u64 {
    now_ms()
}
