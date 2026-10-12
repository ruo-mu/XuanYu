use anyhow::{bail, Context};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    path::{Component, Path, PathBuf},
    process::Command,
};
use xuanyu_storage::project_data_root;

const WRITE_TOOLS: &[&str] = &[
    "file_write",
    "file_edit",
    "apply_patch",
    "git_patch_apply",
    "sandbox_apply",
    "changeset_apply",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnFileAction {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnFileMutation {
    pub path: String,
    pub action: TurnFileAction,
    pub before_blob: Option<String>,
    pub after_blob: Option<String>,
    pub tool_name: String,
    pub at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnLedgerRecord {
    pub turn_id: String,
    pub session_id: String,
    pub created_at: i64,
    pub user_message_preview: String,
    pub mutations: Vec<TurnFileMutation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreConflict {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRestoreResult {
    pub restored: Vec<String>,
    pub conflicts: Vec<RestoreConflict>,
    pub deleted: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct RejectFileMutationInput<'a> {
    pub session_id: &'a str,
    pub turn_id: &'a str,
    pub file_path: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceCheckpointRecord {
    pub id: String,
    pub session_id: String,
    pub turn_id: String,
    pub project_root: String,
    pub git_head: String,
    pub patch_path: String,
    pub has_changes: bool,
    pub files: Vec<String>,
    pub created_at: i64,
}

#[derive(Debug, Clone)]
pub struct TurnLedgerStore {
    project_root: PathBuf,
}

impl TurnLedgerStore {
    pub fn new(project_root: impl Into<PathBuf>) -> Self {
        Self {
            project_root: normalize_pathbuf(project_root.into()),
        }
    }

    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub fn ledger_root(&self, session_id: &str) -> anyhow::Result<PathBuf> {
        Ok(project_data_root(&self.project_root)
            .join("turn-ledger")
            .join(safe_segment(session_id)?))
    }

    pub fn load(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> anyhow::Result<Option<TurnLedgerRecord>> {
        let path = self.record_path(session_id, turn_id)?;
        if !path.exists() {
            return Ok(None);
        }
        let text = fs::read_to_string(&path)
            .with_context(|| format!("failed to read turn ledger {}", path.display()))?;
        let record = serde_json::from_str::<TurnLedgerRecord>(&text)
            .with_context(|| format!("failed to parse turn ledger {}", path.display()))?;
        Ok(Some(record))
    }

    pub fn save(&self, record: &TurnLedgerRecord) -> anyhow::Result<()> {
        let dir = self.ledger_root(&record.session_id)?;
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create turn ledger dir {}", dir.display()))?;
        let path = dir.join(format!("{}.json", safe_segment(&record.turn_id)?));
        let text = serde_json::to_string_pretty(record)?;
        fs::write(&path, format!("{text}\n"))
            .with_context(|| format!("failed to write turn ledger {}", path.display()))
    }

    pub fn ensure(
        &self,
        session_id: &str,
        turn_id: &str,
        user_message_preview: &str,
    ) -> anyhow::Result<TurnLedgerRecord> {
        if let Some(record) = self.load(session_id, turn_id)? {
            return Ok(record);
        }
        let record = TurnLedgerRecord {
            turn_id: safe_segment(turn_id)?,
            session_id: safe_segment(session_id)?,
            created_at: now_ms(),
            user_message_preview: user_message_preview.chars().take(240).collect(),
            mutations: Vec::new(),
        };
        self.save(&record)?;
        Ok(record)
    }

    pub fn list(&self, session_id: &str) -> anyhow::Result<Vec<TurnLedgerRecord>> {
        let dir = self.ledger_root(session_id)?;
        let Ok(entries) = fs::read_dir(&dir) else {
            return Ok(Vec::new());
        };
        let mut records = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|item| item.to_str()) == Some("json"))
            .filter_map(|path| read_record_file(&path))
            .collect::<Vec<_>>();
        records.sort_by_key(|a| a.created_at);
        Ok(records)
    }

    pub fn workspace_checkpoint_root(&self, session_id: &str) -> anyhow::Result<PathBuf> {
        let root = git_root(&self.project_root).unwrap_or_else(|_| self.project_root.clone());
        Ok(project_data_root(&root)
            .join("turn-checkpoints")
            .join(safe_segment(session_id)?))
    }

    pub fn capture_workspace_checkpoint(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> anyhow::Result<Option<WorkspaceCheckpointRecord>> {
        let Ok(project_root) = git_root(&self.project_root) else {
            return Ok(None);
        };
        let git_head = match git_output(&project_root, &["rev-parse", "HEAD"]) {
            Ok(value) if !value.trim().is_empty() => value,
            _ => return Ok(None),
        };
        let session_id = safe_segment(session_id)?;
        let turn_id = safe_segment(turn_id)?;
        let dir = project_data_root(&project_root)
            .join("turn-checkpoints")
            .join(&session_id);
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create checkpoint dir {}", dir.display()))?;
        let patch_path = dir.join(format!("{turn_id}.patch"));
        let patch = workspace_checkpoint_diff(&project_root).unwrap_or_default();
        fs::write(&patch_path, &patch).with_context(|| {
            format!("failed to write checkpoint patch {}", patch_path.display())
        })?;
        let record = WorkspaceCheckpointRecord {
            id: format!("checkpoint_{turn_id}"),
            session_id,
            turn_id,
            project_root: path_to_string(&project_root),
            git_head,
            patch_path: path_to_string(&patch_path),
            has_changes: !patch.trim().is_empty(),
            files: files_in_patch_text(&patch),
            created_at: now_ms(),
        };
        self.save_workspace_checkpoint(&record)?;
        Ok(Some(record))
    }

    pub fn load_workspace_checkpoint(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> anyhow::Result<Option<WorkspaceCheckpointRecord>> {
        let path = self
            .workspace_checkpoint_root(session_id)?
            .join(format!("{}.json", safe_segment(turn_id)?));
        Ok(read_workspace_checkpoint_record(&path))
    }

    pub fn list_workspace_checkpoints(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Vec<WorkspaceCheckpointRecord>> {
        let dir = self.workspace_checkpoint_root(session_id)?;
        let Ok(entries) = fs::read_dir(&dir) else {
            return Ok(Vec::new());
        };
        let mut records = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|item| item.to_str()) == Some("json"))
            .filter_map(|path| read_workspace_checkpoint_record(&path))
            .collect::<Vec<_>>();
        records.sort_by_key(|a| a.created_at);
        Ok(records)
    }

    /// 安全地恢复工作区到 checkpoint 记录时的状态（turn 开始前，即 W0）。
    ///
    /// 与旧实现的区别（安全边界）：绝不使用 `git reset --hard` / `git clean -fd` /
    /// `git restore`，绝不回退 HEAD。做法是先反向撤销当前工作区未提交改动回到 HEAD，
    /// 再应用 checkpoint 快照的 diff，最终精确落在 W0；只接触出现在 diff 里的文件，
    /// HEAD 一旦前进则拒绝执行以保护用户已提交的历史。
    pub fn restore_workspace_checkpoint(
        &self,
        session_id: &str,
        turn_id: &str,
    ) -> anyhow::Result<Option<WorkspaceCheckpointRecord>> {
        let Some(record) = self.load_workspace_checkpoint(session_id, turn_id)? else {
            return Ok(None);
        };
        let project_root = PathBuf::from(&record.project_root);
        // 安全护栏：只在 HEAD 从未前进（期间无新提交）时才允许还原，否则拒绝触碰 git。
        let current_head = git_output(&project_root, &["rev-parse", "HEAD"]).ok();
        if current_head.as_deref() != Some(record.git_head.as_str()) {
            bail!(
                "cannot restore workspace checkpoint {turn_id}: repository HEAD moved since the checkpoint; refusing to touch git history"
            );
        }
        // W1(当前) --反向apply当前diff--> HEAD --apply checkpoint diff--> W0(turn开始前)
        let current_patch = workspace_checkpoint_diff(&project_root).unwrap_or_default();
        if !current_patch.trim().is_empty() {
            apply_patch_safely(&project_root, &current_patch, true)?;
        }
        let patch = fs::read_to_string(&record.patch_path).unwrap_or_default();
        if !patch.trim().is_empty() {
            apply_patch_safely(&project_root, &patch, false)?;
        }
        Ok(Some(record))
    }

    pub fn delete_workspace_checkpoints_after(
        &self,
        session_id: &str,
        turn_id: &str,
        turn_order: &[String],
    ) -> anyhow::Result<()> {
        let Some(start) = turn_order.iter().position(|turn| turn == turn_id) else {
            return Ok(());
        };
        let root = self.workspace_checkpoint_root(session_id)?;
        for id in &turn_order[start..] {
            let safe = safe_segment(id)?;
            let _ = fs::remove_file(root.join(format!("{safe}.json")));
            let _ = fs::remove_file(root.join(format!("{safe}.patch")));
        }
        Ok(())
    }

    pub fn snapshot_before_write(
        &self,
        session_id: &str,
        turn_id: &str,
        cwd: &Path,
        relative_paths: &[String],
    ) -> anyhow::Result<()> {
        let blob_dir = self.blob_dir(session_id, turn_id)?;
        fs::create_dir_all(&blob_dir)
            .with_context(|| format!("failed to create blob dir {}", blob_dir.display()))?;
        for input_path in relative_paths {
            let Some((rel, abs)) = self.resolve_workspace_path(cwd, input_path) else {
                continue;
            };
            let blob_path = blob_dir.join(format!("{}.before", hash_text(&rel)));
            if abs.is_file() {
                fs::copy(&abs, &blob_path).with_context(|| {
                    format!("failed to snapshot {} before write", abs.display())
                })?;
            } else if blob_path.exists() {
                let _ = fs::remove_file(blob_path);
            }
        }
        Ok(())
    }

    pub fn record_write_result(&self, input: RecordWriteInput<'_>) -> anyhow::Result<()> {
        let mut record =
            self.ensure(input.session_id, input.turn_id, input.user_message_preview)?;
        let blob_dir = self.blob_dir(input.session_id, input.turn_id)?;
        fs::create_dir_all(&blob_dir)
            .with_context(|| format!("failed to create blob dir {}", blob_dir.display()))?;
        let now = now_ms();

        for affected in input.affected_files {
            let Some((rel, abs)) = self.resolve_workspace_path(input.cwd, affected) else {
                continue;
            };
            let name = hash_text(&rel);
            let before_blob = blob_dir.join(format!("{name}.before"));
            let after_blob = blob_dir.join(format!("{name}.after"));
            let action = if abs.is_file() {
                fs::copy(&abs, &after_blob)
                    .with_context(|| format!("failed to snapshot {} after write", abs.display()))?;
                if before_blob.exists() {
                    TurnFileAction::Modified
                } else {
                    TurnFileAction::Added
                }
            } else {
                if after_blob.exists() {
                    let _ = fs::remove_file(&after_blob);
                }
                TurnFileAction::Deleted
            };

            record.mutations.retain(|item| item.path != rel);
            record.mutations.push(TurnFileMutation {
                path: rel,
                action,
                before_blob: before_blob.exists().then(|| path_to_string(&before_blob)),
                after_blob: after_blob.exists().then(|| path_to_string(&after_blob)),
                tool_name: input.tool_name.to_string(),
                at: now,
            });
        }

        self.save(&record)
    }

    pub fn files_changed_after_turn(
        &self,
        session_id: &str,
        turn_order: &[String],
        target_turn_id: &str,
    ) -> anyhow::Result<Vec<String>> {
        let Some(start) = turn_order.iter().position(|turn| turn == target_turn_id) else {
            return Ok(Vec::new());
        };
        let mut files = BTreeSet::new();
        for turn_id in &turn_order[start..] {
            if let Some(ledger) = self.load(session_id, turn_id)? {
                for mutation in ledger.mutations {
                    files.insert(mutation.path);
                }
            }
        }
        Ok(files.into_iter().collect())
    }

    pub fn restore_workspace_from_ledger(
        &self,
        input: RestoreWorkspaceInput<'_>,
    ) -> anyhow::Result<WorkspaceRestoreResult> {
        let mut result = WorkspaceRestoreResult::default();
        let Some(start) = input
            .turn_order
            .iter()
            .position(|turn| turn == input.target_turn_id)
        else {
            return Ok(result);
        };

        let mut later_mutations = Vec::new();
        for turn_id in input.turn_order[start..].iter().rev() {
            if let Some(ledger) = self.load(input.session_id, turn_id)? {
                later_mutations.extend(ledger.mutations.into_iter().rev());
            }
        }

        let mut seen = BTreeSet::new();
        for mutation in later_mutations {
            if !seen.insert(mutation.path.clone()) {
                continue;
            }
            let abs = self.project_root.join(&mutation.path);
            let current_hash = file_hash(&abs)?;
            if let Some(after_blob) = mutation.after_blob.as_deref() {
                let expected_hash = file_hash(Path::new(after_blob))?;
                if current_hash.is_some() && current_hash != expected_hash {
                    result.conflicts.push(RestoreConflict {
                        path: mutation.path,
                        reason: "File changed after the agent edit; manual merge required."
                            .to_string(),
                    });
                    continue;
                }
            }

            if let Some(before_blob) = mutation.before_blob.as_deref() {
                let before_blob = Path::new(before_blob);
                if before_blob.exists() {
                    if let Some(parent) = abs.parent() {
                        fs::create_dir_all(parent).with_context(|| {
                            format!("failed to create restore parent {}", parent.display())
                        })?;
                    }
                    fs::copy(before_blob, &abs).with_context(|| {
                        format!("failed to restore {} from ledger blob", abs.display())
                    })?;
                    result.restored.push(mutation.path);
                    continue;
                }
            }

            if mutation.action == TurnFileAction::Added && abs.exists() {
                fs::remove_file(&abs)
                    .with_context(|| format!("failed to delete added file {}", abs.display()))?;
                result.deleted.push(mutation.path);
            }
        }

        result.restored.sort();
        result.conflicts.sort_by(|a, b| a.path.cmp(&b.path));
        result.deleted.sort();
        Ok(result)
    }

    pub fn reject_file_mutation(
        &self,
        input: RejectFileMutationInput<'_>,
    ) -> anyhow::Result<WorkspaceRestoreResult> {
        let mut result = WorkspaceRestoreResult::default();
        let Some(ledger) = self.load(input.session_id, input.turn_id)? else {
            return Ok(result);
        };
        let target = normalize_rel_text(input.file_path);
        let Some(mutation) = ledger
            .mutations
            .iter()
            .rev()
            .find(|item| item.path == target)
            .cloned()
        else {
            result.conflicts.push(RestoreConflict {
                path: target,
                reason: "No ledger mutation for this file in the turn.".to_string(),
            });
            return Ok(result);
        };

        let abs = self.project_root.join(&mutation.path);
        let current_hash = file_hash(&abs)?;
        if let Some(after_blob) = mutation.after_blob.as_deref() {
            let expected_hash = file_hash(Path::new(after_blob))?;
            if current_hash.is_some() && current_hash != expected_hash {
                result.conflicts.push(RestoreConflict {
                    path: mutation.path,
                    reason: "File changed after the agent edit; manual merge required."
                        .to_string(),
                });
                return Ok(result);
            }
        }

        if let Some(before_blob) = mutation.before_blob.as_deref() {
            let before_blob = Path::new(before_blob);
            if before_blob.exists() {
                if let Some(parent) = abs.parent() {
                    fs::create_dir_all(parent).with_context(|| {
                        format!("failed to create restore parent {}", parent.display())
                    })?;
                }
                fs::copy(before_blob, &abs).with_context(|| {
                    format!("failed to restore {} from ledger blob", abs.display())
                })?;
                result.restored.push(mutation.path);
                return Ok(result);
            }
        }

        if mutation.action == TurnFileAction::Added && abs.exists() {
            fs::remove_file(&abs)
                .with_context(|| format!("failed to delete added file {}", abs.display()))?;
            result.deleted.push(mutation.path);
        }
        Ok(result)
    }

    pub fn delete_after(
        &self,
        session_id: &str,
        turn_id: &str,
        turn_order: &[String],
    ) -> anyhow::Result<()> {
        let Some(start) = turn_order.iter().position(|turn| turn == turn_id) else {
            return Ok(());
        };
        let root = self.ledger_root(session_id)?;
        for id in &turn_order[start..] {
            let safe = safe_segment(id)?;
            let _ = fs::remove_dir_all(root.join(&safe));
            let _ = fs::remove_file(root.join(format!("{safe}.json")));
        }
        Ok(())
    }

    fn save_workspace_checkpoint(&self, record: &WorkspaceCheckpointRecord) -> anyhow::Result<()> {
        let dir = project_data_root(&PathBuf::from(&record.project_root))
            .join("turn-checkpoints")
            .join(safe_segment(&record.session_id)?);
        fs::create_dir_all(&dir)
            .with_context(|| format!("failed to create checkpoint dir {}", dir.display()))?;
        let path = dir.join(format!("{}.json", safe_segment(&record.turn_id)?));
        let text = serde_json::to_string_pretty(record)?;
        fs::write(&path, format!("{text}\n"))
            .with_context(|| format!("failed to write checkpoint record {}", path.display()))
    }

    fn record_path(&self, session_id: &str, turn_id: &str) -> anyhow::Result<PathBuf> {
        Ok(self
            .ledger_root(session_id)?
            .join(format!("{}.json", safe_segment(turn_id)?)))
    }

    fn blob_dir(&self, session_id: &str, turn_id: &str) -> anyhow::Result<PathBuf> {
        Ok(self
            .ledger_root(session_id)?
            .join(safe_segment(turn_id)?)
            .join("blobs"))
    }

    fn resolve_workspace_path(&self, cwd: &Path, input: &str) -> Option<(String, PathBuf)> {
        let path = Path::new(input);
        let abs = normalize_pathbuf(if path.is_absolute() {
            path.to_path_buf()
        } else {
            cwd.join(path)
        });
        let rel = abs.strip_prefix(&self.project_root).ok()?;
        let rel = normalize_rel_path(rel);
        if rel.is_empty() || rel.starts_with("..") || rel.starts_with(".xuanyu/") {
            return None;
        }
        Some((rel, abs))
    }
}

#[derive(Debug, Clone)]
pub struct RecordWriteInput<'a> {
    pub session_id: &'a str,
    pub turn_id: &'a str,
    pub user_message_preview: &'a str,
    pub tool_name: &'a str,
    pub cwd: &'a Path,
    pub affected_files: &'a [String],
}

#[derive(Debug, Clone)]
pub struct RestoreWorkspaceInput<'a> {
    pub session_id: &'a str,
    pub target_turn_id: &'a str,
    pub turn_order: &'a [String],
}

pub fn is_write_tool_name(tool_name: &str) -> bool {
    WRITE_TOOLS.contains(&tool_name)
}

pub fn extract_write_paths(tool_name: &str, args: &Value) -> Vec<String> {
    match tool_name {
        "file_write" | "file_edit" => args
            .get("path")
            .and_then(Value::as_str)
            .map(normalize_rel_text)
            .into_iter()
            .collect(),
        "apply_patch" => args
            .get("patches")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|patch| patch.get("path").and_then(Value::as_str))
            .map(normalize_rel_text)
            .collect(),
        _ => Vec::new(),
    }
}

fn read_record_file(path: &Path) -> Option<TurnLedgerRecord> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<TurnLedgerRecord>(&text).ok())
}

fn safe_segment(value: &str) -> anyhow::Result<String> {
    let value = value.trim();
    if value.is_empty()
        || value.contains("..")
        || value.contains('/')
        || value.contains('\\')
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        bail!("unsafe turn ledger segment: {value}");
    }
    Ok(value.to_string())
}

fn normalize_rel_text(value: &str) -> String {
    value
        .trim()
        .replace('\\', "/")
        .trim_start_matches("./")
        .to_string()
}

fn normalize_rel_path(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn normalize_pathbuf(path: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            _ => out.push(component.as_os_str()),
        }
    }
    out
}

fn git_root(cwd: &Path) -> anyhow::Result<PathBuf> {
    let output = git_output(cwd, &["rev-parse", "--show-toplevel"])?;
    Ok(normalize_pathbuf(PathBuf::from(output)))
}

fn workspace_checkpoint_diff(root: &Path) -> anyhow::Result<String> {
    let mut diff = git_output_raw(
        root,
        &[
            "diff",
            "--binary",
            "HEAD",
            "--",
            ".",
            ":(exclude).xuanyu/**",
        ],
    )
    .or_else(|_| {
        git_output_raw(
            root,
            &["diff", "--binary", "--", ".", ":(exclude).xuanyu/**"],
        )
    })?;

    let status = git_output(root, &["status", "--porcelain=v1", "--untracked-files=all"])?;
    let synthetic = status
        .lines()
        .filter_map(untracked_path_from_status)
        .filter(|path| !is_xuanyu_internal_path(path))
        .map(|path| build_untracked_file_diff(root, &path))
        .collect::<anyhow::Result<Vec<_>>>()?
        .into_iter()
        .filter(|text| !text.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");

    if !synthetic.trim().is_empty() {
        diff = [diff.trim_end(), synthetic.trim_end()]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        diff.push('\n');
    }
    Ok(diff)
}

fn build_untracked_file_diff(root: &Path, rel_path: &str) -> anyhow::Result<String> {
    let safe = normalize_rel_text(rel_path);
    if safe.is_empty()
        || safe.starts_with('-')
        || Path::new(&safe).is_absolute()
        || safe.split('/').any(|part| part == "..")
        || safe.contains('\0')
    {
        return Ok(String::new());
    }
    let abs = root.join(&safe);
    let Ok(metadata) = fs::metadata(&abs) else {
        return Ok(String::new());
    };
    if !metadata.is_file() {
        return Ok(String::new());
    }
    if metadata.len() > 1024 * 1024 {
        return Ok(binary_untracked_diff(&safe));
    }
    let buffer = fs::read(&abs)
        .with_context(|| format!("failed to read untracked file {}", abs.display()))?;
    if buffer.contains(&0) {
        return Ok(binary_untracked_diff(&safe));
    }

    let content = String::from_utf8_lossy(&buffer).replace("\r\n", "\n");
    let mut body_lines = if content.is_empty() {
        Vec::new()
    } else {
        content.split('\n').collect::<Vec<_>>()
    };
    if body_lines.last() == Some(&"") {
        body_lines.pop();
    }
    let line_count = body_lines.len().max(1);
    Ok([
        format!("diff --git a/{safe} b/{safe}"),
        "new file mode 100644".to_string(),
        "index 0000000..0000000".to_string(),
        "--- /dev/null".to_string(),
        format!("+++ b/{safe}"),
        format!("@@ -0,0 +1,{line_count} @@"),
    ]
    .into_iter()
    .chain(body_lines.into_iter().map(|line| format!("+{line}")))
    .chain(std::iter::once(String::new()))
    .collect::<Vec<_>>()
    .join("\n"))
}

fn binary_untracked_diff(path: &str) -> String {
    [
        format!("diff --git a/{path} b/{path}"),
        "new file mode 100644".to_string(),
        "index 0000000..0000000".to_string(),
        format!("Binary files /dev/null and b/{path} differ"),
        String::new(),
    ]
    .join("\n")
}

fn untracked_path_from_status(line: &str) -> Option<String> {
    line.strip_prefix("?? ").map(normalize_rel_text)
}

fn is_xuanyu_internal_path(path: &str) -> bool {
    let normalized = normalize_rel_text(path);
    normalized == ".xuanyu" || normalized.starts_with(".xuanyu/")
}

fn files_in_patch_text(patch: &str) -> Vec<String> {
    let mut files = BTreeSet::new();
    for line in patch.lines() {
        if let Some(path) = line.strip_prefix("+++ b/") {
            if path != "/dev/null" {
                files.insert(path.to_string());
            }
        } else if let Some(path) = line.strip_prefix("--- a/") {
            if path != "/dev/null" {
                files.insert(path.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("Binary files /dev/null and b/") {
            if let Some(path) = rest.strip_suffix(" differ") {
                files.insert(path.to_string());
            }
        } else if let Some(rest) = line.strip_prefix("Binary files a/") {
            if let Some(path) = rest.strip_suffix(" and /dev/null differ") {
                files.insert(path.to_string());
            }
        }
    }
    files.into_iter().collect()
}

/// 把 `patch` 应用到工作区（`reverse=false` 正向、`reverse=true` 反向即"撤销"）。
/// 走临时文件 + `git apply`（原子、失败不改工作区），失败时清理临时文件并返回错误。
fn apply_patch_safely(root: &Path, patch: &str, reverse: bool) -> anyhow::Result<()> {
    let temp_dir = project_data_root(root)
        .join("turn-checkpoints")
        .join(format!("_apply_{}.patch", now_ms()));
    fs::write(&temp_dir, patch)
        .with_context(|| format!("failed to write apply patch {}", temp_dir.display()))?;
    let mut args = vec![
        "apply".to_string(),
        "--binary".to_string(),
        "--whitespace=nowarn".to_string(),
    ];
    if reverse {
        args.push("--reverse".to_string());
    }
    args.push(temp_dir.to_string_lossy().into_owned());
    let result = git_output(root, &args.iter().map(String::as_str).collect::<Vec<_>>());
    let _ = fs::remove_file(&temp_dir);
    result.map(|_| ()).with_context(|| "failed to apply patch during workspace restore".to_string())
}

fn read_workspace_checkpoint_record(path: &Path) -> Option<WorkspaceCheckpointRecord> {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<WorkspaceCheckpointRecord>(&text).ok())
}

fn git_output(cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
    git_output_raw(cwd, args).map(|output| output.trim_end().to_string())
}

fn git_output_raw(cwd: &Path, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .with_context(|| format!("failed to run git in {}", cwd.display()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!(
            "{}",
            if stderr.is_empty() {
                format!("git exited with {:?}", output.status.code())
            } else {
                stderr
            }
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

fn file_hash(path: &Path) -> anyhow::Result<Option<String>> {
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(path).with_context(|| format!("failed to hash {}", path.display()))?;
    Ok(Some(hash_bytes(&bytes)))
}

fn hash_text(text: &str) -> String {
    hash_bytes(text.as_bytes())
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}
