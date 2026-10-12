//! 会话工作清单（todo_write）与主目标完成（update_goal）的本地文件账本。

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
};
use xuanyu_storage::project_data_root;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

impl TodoStatus {
    pub fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "pending" => Some(Self::Pending),
            "in_progress" | "in-progress" => Some(Self::InProgress),
            "completed" | "complete" => Some(Self::Completed),
            "cancelled" | "canceled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TodoItem {
    pub id: String,
    pub content: String,
    pub status: TodoStatus,
}

pub fn todos_path(project_root: &Path, session_id: &str) -> PathBuf {
    project_data_root(project_root)
        .join("todos")
        .join(format!("{session_id}.json"))
}

pub fn load_todos(project_root: &Path, session_id: &str) -> Vec<TodoItem> {
    let path = todos_path(project_root, session_id);
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    serde_json::from_str::<Vec<TodoItem>>(&text).unwrap_or_default()
}

pub fn save_todos(project_root: &Path, session_id: &str, todos: &[TodoItem]) -> anyhow::Result<()> {
    let path = todos_path(project_root, session_id);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(&tmp, format!("{}\n", serde_json::to_string_pretty(todos)?))?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub fn sync_todos(
    project_root: &Path,
    session_id: &str,
    incoming: Vec<TodoItem>,
    merge: bool,
) -> anyhow::Result<Vec<TodoItem>> {
    let mut next = if merge {
        load_todos(project_root, session_id)
    } else {
        Vec::new()
    };
    for item in incoming {
        if let Some(existing) = next.iter_mut().find(|todo| todo.id == item.id) {
            *existing = item;
        } else {
            next.push(item);
        }
    }
    save_todos(project_root, session_id, &next)?;
    Ok(next)
}

pub fn todos_prompt_block(project_root: &Path, session_id: &str) -> Option<String> {
    let todos = load_todos(project_root, session_id);
    if todos.is_empty() {
        return None;
    }
    let mut lines = vec!["## Work checklist (todo_write)".to_string()];
    for todo in todos {
        lines.push(format!(
            "- [{}] {}: {}",
            todo.status.as_str(),
            todo.id,
            todo.content
        ));
    }
    Some(lines.join("\n"))
}

pub fn goal_path(project_root: &Path, session_id: &str) -> PathBuf {
    project_data_root(project_root)
        .join("goals")
        .join(format!("{session_id}.json"))
}

pub fn complete_session_goal(
    project_root: &Path,
    session_id: &str,
    evidence_summary: &str,
    progress_note: Option<&str>,
) -> Result<serde_json::Value, String> {
    let path = goal_path(project_root, session_id);
    let text = fs::read_to_string(&path).map_err(|_| "No active goal to complete.".to_string())?;
    let mut goal: serde_json::Value =
        serde_json::from_str(&text).map_err(|err| format!("Invalid goal file: {err}"))?;
    let status = goal
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("active");
    if status == "complete" {
        return Err("Goal already complete.".to_string());
    }
    if status == "cleared" {
        return Err("No active goal to complete.".to_string());
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let ledger = goal
        .get_mut("ledger")
        .and_then(|v| v.as_array_mut())
        .map(|arr| {
            if let Some(note) = progress_note.map(str::trim).filter(|s| !s.is_empty()) {
                arr.push(json!({
                    "at": now,
                    "kind": "progress",
                    "summary": note,
                }));
            }
            arr.push(json!({
                "at": now,
                "kind": "completed",
                "summary": evidence_summary.trim(),
            }));
        });
    let _ = ledger;
    if let Some(object) = goal.as_object_mut() {
        object.insert("status".to_string(), json!("complete"));
        object.insert("updatedAt".to_string(), json!(now));
        object.insert("completedAt".to_string(), json!(now));
        object.insert("suppressNextContinuation".to_string(), json!(true));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::write(
        &tmp,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&goal).map_err(|err| err.to_string())?
        ),
    )
    .map_err(|err| err.to_string())?;
    fs::rename(&tmp, &path).map_err(|err| err.to_string())?;
    Ok(goal)
}
