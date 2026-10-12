mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use serde_json::{json, Value};

use xuanyu_governance::GovernanceEngine;
use xuanyu_memory::{
    kind_for_category, with_memory_session_tag, MemoryService, NewMemoryEntry,
};
use xuanyu_storage::project_data_root;
use xuanyu_tools::{ToolCall, ToolRuntime};

use common::{allowing_governance, temp_workspace};

#[tokio::test]
async fn builtin_catalog_exposes_read_and_write_tools() {
    let root = temp_workspace();
    let runtime = ToolRuntime::with_builtins(Arc::new(GovernanceEngine::default()), vec![root]);
    let catalog = runtime.catalog().await;
    assert!(catalog.iter().any(|tool| tool.name == "file_read"));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "file_write" && tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "apply_patch" && tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "start_process" && tool.is_execute));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "write_process_stdin" && tool.is_execute));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "git_status" && !tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "git_commit" && tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "validation_discover" && !tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "codebase_index" && !tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "write_memory" && tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "web_search" && !tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "web_fetch" && !tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "sandbox_create" && tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "sandbox_diff" && !tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "workspace_checkpoint_create" && tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "workspace_checkpoint_list" && !tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "workspace_checkpoint_restore" && tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "submit_task" && !tool.is_write));
    assert!(catalog.iter().any(|tool| tool.name == "todo_write"));
    assert!(catalog.iter().any(|tool| tool.name == "update_goal"));
    assert!(catalog.iter().any(|tool| tool.name == "enter_plan_mode"));
    assert!(catalog.iter().any(|tool| tool.name == "exit_plan_mode"));
    assert!(catalog.iter().any(|tool| tool.name == "activate_skill"));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "get_task_status" && !tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "list_tasks" && !tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "cancel_task" && !tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "delivery_create_local_pr" && tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "git_platform_status" && !tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "search_tools" && !tool.is_write));
    assert!(catalog
        .iter()
        .any(|tool| tool.name == "search_memory" && !tool.is_write));
}

#[tokio::test]
async fn file_read_defaults_to_2000_line_window_with_partial() {
    let root = temp_workspace();
    let mut body = String::new();
    for i in 1..=2500 {
        body.push_str(&format!("line-{i}\n"));
    }
    fs::write(root.join("big.txt"), &body).expect("write fixture");
    let runtime = ToolRuntime::with_builtins(Arc::new(allowing_governance()), vec![root.clone()]);
    let read = runtime
        .execute(ToolCall {
            name: "file_read".to_string(),
            args: BTreeMap::from([("path".to_string(), json!("big.txt"))]),
            cwd: root.clone(),
        })
        .await;
    assert!(!read.is_error, "{}", read.content);
    assert!(read.content.contains("     1\tline-1"));
    assert!(read.content.contains("  2000\tline-2000"));
    assert!(!read.content.contains("line-2001"));
    assert!(read.content.contains("[PARTIAL view]"));
    assert_eq!(
        read.data
            .as_ref()
            .and_then(|d| d.get("endLine"))
            .and_then(|v| v.as_u64()),
        Some(2000)
    );

    let cont = runtime
        .execute(ToolCall {
            name: "file_read".to_string(),
            args: BTreeMap::from([
                ("path".to_string(), json!("big.txt")),
                ("start_line".to_string(), json!(2001)),
                ("limit".to_string(), json!(100)),
            ]),
            cwd: root.clone(),
        })
        .await;
    assert!(!cont.is_error, "{}", cont.content);
    assert!(cont.content.contains("line-2001"));
    assert!(cont.content.contains("line-2100"));
}

#[tokio::test]
async fn large_file_write_spills_content_but_keeps_affected_files() {
    let root = temp_workspace();
    let runtime =
        ToolRuntime::with_builtins(Arc::new(allowing_governance()), vec![root.clone()]);
    let content = format!("header\n{}", "payload-line\n".repeat(20_000));
    let write = runtime
        .execute(ToolCall {
            name: "file_write".to_string(),
            args: BTreeMap::from([
                ("path".to_string(), json!("huge.txt")),
                ("content".to_string(), json!(content)),
            ]),
            cwd: root.clone(),
        })
        .await;
    assert!(!write.is_error, "{}", write.content);
    assert!(write.content.contains("Successfully wrote") || write.content.contains("huge.txt"));
    // file_write 对输出豁免截断：完整 diff 全量返回，不含截断标记。
    assert!(write.content.contains("huge.txt"));
    assert!(!write.content.contains("[output truncated"));
    assert!(write.content.contains("payload-line"));
    let affected = write
        .data
        .as_ref()
        .and_then(|d| d.get("affectedFiles"))
        .cloned()
        .unwrap_or(json!([]));
    assert!(affected.as_array().unwrap().iter().any(|v| {
        v.as_str()
            .map(|s| s.ends_with("huge.txt") || s == "huge.txt")
            .unwrap_or(false)
    }));
    assert!(root.join("huge.txt").is_file());
}

#[tokio::test]
async fn delivery_tools_create_local_pr_and_issue_draft() {
    let root = temp_workspace();
    run_git(&root, &["init"]);
    run_git(&root, &["config", "user.email", "xuanyu@example.test"]);
    run_git(&root, &["config", "user.name", "Xuanyu Test"]);
    fs::write(root.join("note.txt"), "old\n").expect("write fixture");
    run_git(&root, &["add", "note.txt"]);
    run_git(&root, &["commit", "-m", "initial"]);
    run_git(
        &root,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/example/repo.git",
        ],
    );
    fs::write(root.join("note.txt"), "new\n").expect("modify fixture");

    let runtime =
        ToolRuntime::with_builtins(Arc::new(allowing_governance()), vec![root.clone()]);

    let local_pr = runtime
        .execute(ToolCall {
            name: "delivery_create_local_pr".to_string(),
            args: BTreeMap::from([
                ("title".to_string(), json!("Update note")),
                ("issue".to_string(), json!("123")),
                ("platform".to_string(), json!("github")),
            ]),
            cwd: root.clone(),
        })
        .await;
    assert!(!local_pr.is_error, "{}", local_pr.content);
    let markdown_path = local_pr
        .data
        .as_ref()
        .and_then(|data| data.pointer("/localPr/markdownPath"))
        .and_then(Value::as_str)
        .expect("markdown path");
    let patch_path = local_pr
        .data
        .as_ref()
        .and_then(|data| data.pointer("/localPr/patchPath"))
        .and_then(Value::as_str)
        .expect("patch path");
    assert!(Path::new(markdown_path).exists());
    assert!(fs::read_to_string(patch_path).unwrap().contains("+new"));
    assert!(local_pr
        .data
        .as_ref()
        .and_then(|data| data.pointer("/localPr/changeSet"))
        .is_some());

    let platform = runtime
        .execute(ToolCall {
            name: "git_platform_status".to_string(),
            args: BTreeMap::from([("platform".to_string(), json!("github"))]),
            cwd: root.clone(),
        })
        .await;
    assert!(!platform.is_error, "{}", platform.content);
    assert_eq!(
        platform
            .data
            .as_ref()
            .and_then(|data| data.pointer("/integrations/0/configured"))
            .and_then(Value::as_bool),
        Some(true)
    );

    let draft = runtime
        .execute(ToolCall {
            name: "git_issue_task_draft".to_string(),
            args: BTreeMap::from([
                ("platform".to_string(), json!("github")),
                ("issue".to_string(), json!("123")),
            ]),
            cwd: root,
        })
        .await;
    assert!(!draft.is_error, "{}", draft.content);
    assert!(draft.content.contains("task/123"));
}

#[tokio::test]
async fn discovery_tools_search_tools_and_memory() {
    let root = temp_workspace();
    let data_root = temp_workspace();
    std::env::set_var("XUANYU_DATA_DIR", &data_root);
    fs::write(
        root.join("AGENTS.md"),
        "Use tested project instructions before editing.\n",
    )
    .expect("project instructions");
    let skill_dir = project_data_root(&root).join("skills").join("reviewer");
    fs::create_dir_all(&skill_dir).expect("skill dir");
    fs::write(
        skill_dir.join("SKILL.md"),
        r#"---
name: Reviewer
description: Review code changes
tools: [search, file_read]
context: review
---
Check correctness and regressions.
"#,
    )
    .expect("skill file");
    let memory = Arc::new(MemoryService::new());
    memory
        .add_memory(NewMemoryEntry {
            kind: kind_for_category("project_fact"),
            category: "project_fact".to_string(),
            content: "Delivery tooling creates local PR packages.".to_string(),
            tags: vec!["delivery".to_string()],
            context: Some("tests".to_string()),
            importance: 0.9,
        })
        .await;
    for (session_id, owner) in [("session-alpha", "Alpha"), ("session-beta", "Beta")] {
        memory
            .add_memory(NewMemoryEntry {
                kind: kind_for_category("project_fact"),
                category: "project_fact".to_string(),
                content: format!("Session scoped PR package memory belongs to {owner}."),
                tags: with_memory_session_tag(vec!["delivery".to_string()], Some(session_id)),
                context: Some(session_id.to_string()),
                importance: 0.9,
            })
            .await;
    }
    let runtime =
        ToolRuntime::with_builtins(Arc::new(GovernanceEngine::default()), vec![root.clone()])
            .with_memory(memory);

    let tools = runtime
        .execute(ToolCall {
            name: "search_tools".to_string(),
            args: BTreeMap::from([("query".to_string(), json!("delivery"))]),
            cwd: root.clone(),
        })
        .await;
    assert!(!tools.is_error, "{}", tools.content);
    assert!(tools.content.contains("delivery_create_local_pr"));

    let loaded = runtime
        .execute(ToolCall {
            name: "load_tools".to_string(),
            args: BTreeMap::from([("names".to_string(), json!(["git_status", "missing"]))]),
            cwd: root.clone(),
        })
        .await;
    assert!(!loaded.is_error, "{}", loaded.content);
    assert!(loaded.content.contains("Loaded: git_status"));
    assert!(loaded.content.contains("Rejected: missing"));

    let memories = runtime
        .execute(ToolCall {
            name: "search_memory".to_string(),
            args: BTreeMap::from([("query".to_string(), json!("PR packages"))]),
            cwd: root.clone(),
        })
        .await;
    assert!(!memories.is_error, "{}", memories.content);
    assert!(memories.content.contains("Delivery tooling"));

    let loaded_memory_only = runtime
        .execute(ToolCall {
            name: "load_memories".to_string(),
            args: BTreeMap::from([
                ("sections".to_string(), json!(["memory"])),
                ("query".to_string(), json!("PR packages")),
            ]),
            cwd: root.clone(),
        })
        .await;
    assert!(
        !loaded_memory_only.is_error,
        "{}",
        loaded_memory_only.content
    );
    assert!(loaded_memory_only.content.contains("Delivery tooling"));
    assert!(loaded_memory_only
        .data
        .as_ref()
        .and_then(|data| data.pointer("/sections/memory/text"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .contains("Delivery tooling"));

    let scoped_memories = runtime
        .execute(ToolCall {
            name: "search_memory".to_string(),
            args: BTreeMap::from([
                (
                    "query".to_string(),
                    json!("Session scoped PR package memory"),
                ),
                ("_sessionId".to_string(), json!("session-alpha")),
                ("limit".to_string(), json!(5)),
            ]),
            cwd: root.clone(),
        })
        .await;
    assert!(!scoped_memories.is_error, "{}", scoped_memories.content);
    assert!(scoped_memories.content.contains("Alpha"));
    assert!(!scoped_memories.content.contains("Beta"));

    let cross_session_memories = runtime
        .execute(ToolCall {
            name: "search_memory".to_string(),
            args: BTreeMap::from([
                (
                    "query".to_string(),
                    json!("Session scoped PR package memory"),
                ),
                ("_sessionId".to_string(), json!("session-alpha")),
                ("includeCrossSession".to_string(), json!(true)),
                ("limit".to_string(), json!(5)),
            ]),
            cwd: root.clone(),
        })
        .await;
    assert!(
        cross_session_memories.content.contains("Beta"),
        "{}",
        cross_session_memories.content
    );

    let loaded_memory = runtime
        .execute(ToolCall {
            name: "load_memories".to_string(),
            args: BTreeMap::from([
                ("sections".to_string(), json!(["memory", "skills"])),
                ("query".to_string(), json!("review these changes")),
                ("includePrompt".to_string(), json!(true)),
            ]),
            cwd: root.clone(),
        })
        .await;
    assert!(!loaded_memory.is_error, "{}", loaded_memory.content);
    assert!(loaded_memory.content.contains("Loaded sections"));
    assert!(loaded_memory.content.contains("memory"));
    assert!(loaded_memory.content.contains("use_activate_skill") || loaded_memory.content.contains("Rejected"));
    let activated = runtime
        .execute(ToolCall {
            name: "activate_skill".to_string(),
            args: BTreeMap::from([("id".to_string(), json!("reviewer"))]),
            cwd: root.clone(),
        })
        .await;
    assert!(!activated.is_error, "{}", activated.content);
    assert!(
        activated.content.contains("Reviewer") || activated.content.to_lowercase().contains("skill"),
        "{}",
        activated.content
    );

    let loaded_instructions = runtime
        .execute(ToolCall {
            name: "load_memories".to_string(),
            args: BTreeMap::from([("sections".to_string(), json!(["project_instructions"]))]),
            cwd: root,
        })
        .await;
    assert!(
        !loaded_instructions.is_error,
        "{}",
        loaded_instructions.content
    );
    assert!(loaded_instructions
        .content
        .contains("[Project instructions]"));
    assert!(loaded_instructions
        .content
        .contains("Use tested project instructions"));
    let sources = loaded_instructions.data.as_ref().unwrap()["sections"]
        ["project_instructions"]["sources"]
        .as_array()
        .unwrap();
    assert!(sources.iter().any(|source| {
        source["relativePath"]
            .as_str()
            .map(|path| path.eq_ignore_ascii_case("agents.md"))
            .unwrap_or(false)
    }));
}

#[tokio::test]
async fn write_edit_and_patch_modify_workspace_when_policy_allows() {
    let root = temp_workspace();
    let runtime =
        ToolRuntime::with_builtins(Arc::new(allowing_governance()), vec![root.clone()]);
    let write = runtime
        .execute(ToolCall {
            name: "file_write".to_string(),
            args: BTreeMap::from([
                ("path".to_string(), json!("note.txt")),
                ("content".to_string(), json!("hello\n")),
            ]),
            cwd: root.clone(),
        })
        .await;
    assert!(!write.is_error, "{}", write.content);

    let edit = runtime
        .execute(ToolCall {
            name: "file_edit".to_string(),
            args: BTreeMap::from([
                ("path".to_string(), json!("note.txt")),
                ("old_text".to_string(), json!("hello")),
                ("new_text".to_string(), json!("hi")),
            ]),
            cwd: root.clone(),
        })
        .await;
    assert!(!edit.is_error, "{}", edit.content);

    let patch = runtime
        .execute(ToolCall {
            name: "apply_patch".to_string(),
            args: BTreeMap::from([(
                "patches".to_string(),
                json!([
                    { "path": "note.txt", "action": "update", "old_text": "hi", "new_text": "hey" },
                    { "path": "new.txt", "action": "add", "content": "created\n" }
                ]),
            )]),
            cwd: root.clone(),
        })
        .await;
    assert!(!patch.is_error, "{}", patch.content);
    assert_eq!(fs::read_to_string(root.join("note.txt")).unwrap(), "hey\n");
    assert_eq!(
        fs::read_to_string(root.join("new.txt")).unwrap(),
        "created\n"
    );
}

#[tokio::test]
async fn write_scope_allows_file_write_inside_glob() {
    let root = temp_workspace();
    let runtime =
        ToolRuntime::with_builtins(Arc::new(allowing_governance()), vec![root.clone()]);
    let write = runtime
        .execute(ToolCall {
            name: "file_write".to_string(),
            args: BTreeMap::from([
                ("path".to_string(), json!("src/note.txt")),
                ("content".to_string(), json!("scoped\n")),
                ("writeScope".to_string(), json!(["src/**"])),
            ]),
            cwd: root.clone(),
        })
        .await;

    assert!(!write.is_error, "{}", write.content);
    assert_eq!(
        fs::read_to_string(root.join("src").join("note.txt")).unwrap(),
        "scoped\n"
    );
}

#[tokio::test]
async fn write_scope_blocks_file_write_outside_glob() {
    let root = temp_workspace();
    let runtime =
        ToolRuntime::with_builtins(Arc::new(allowing_governance()), vec![root.clone()]);
    let write = runtime
        .execute(ToolCall {
            name: "file_write".to_string(),
            args: BTreeMap::from([
                ("path".to_string(), json!("README.md")),
                ("content".to_string(), json!("outside\n")),
                ("writeScope".to_string(), json!(["src/**"])),
            ]),
            cwd: root.clone(),
        })
        .await;

    assert!(write.is_error, "{}", write.content);
    assert!(write.content.contains("outside writeScope"));
    assert!(!root.join("README.md").exists());
}

#[tokio::test]
async fn write_scope_blocks_file_edit_outside_glob() {
    let root = temp_workspace();
    fs::write(root.join("README.md"), "base\n").expect("write fixture");
    let runtime =
        ToolRuntime::with_builtins(Arc::new(allowing_governance()), vec![root.clone()]);
    let edit = runtime
        .execute(ToolCall {
            name: "file_edit".to_string(),
            args: BTreeMap::from([
                ("path".to_string(), json!("README.md")),
                ("old_text".to_string(), json!("base")),
                ("new_text".to_string(), json!("changed")),
                ("write_scope".to_string(), json!(["src/**"])),
            ]),
            cwd: root.clone(),
        })
        .await;

    assert!(edit.is_error, "{}", edit.content);
    assert!(edit.content.contains("outside writeScope"));
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "base\n"
    );
}

#[tokio::test]
async fn write_scope_blocks_out_of_scope_patch_before_any_write() {
    let root = temp_workspace();
    let runtime =
        ToolRuntime::with_builtins(Arc::new(allowing_governance()), vec![root.clone()]);
    let patch = runtime
        .execute(ToolCall {
            name: "apply_patch".to_string(),
            args: BTreeMap::from([
                (
                    "patches".to_string(),
                    json!([
                        { "path": "src/ok.txt", "action": "add", "content": "ok\n" },
                        { "path": "README.md", "action": "add", "content": "outside\n" }
                    ]),
                ),
                ("writeScope".to_string(), json!(["src/**"])),
            ]),
            cwd: root.clone(),
        })
        .await;

    assert!(patch.is_error, "{}", patch.content);
    assert!(patch.content.contains("outside writeScope"));
    assert!(!root.join("src").join("ok.txt").exists());
    assert!(!root.join("README.md").exists());
}

#[tokio::test]
async fn readonly_context_blocks_write_tools_without_write_scope() {
    let root = temp_workspace();
    let runtime =
        ToolRuntime::with_builtins(Arc::new(allowing_governance()), vec![root.clone()]);
    let write = runtime
        .execute(ToolCall {
            name: "file_write".to_string(),
            args: BTreeMap::from([
                ("path".to_string(), json!("note.txt")),
                ("content".to_string(), json!("blocked\n")),
                ("readonly".to_string(), json!(true)),
            ]),
            cwd: root.clone(),
        })
        .await;

    assert!(write.is_error, "{}", write.content);
    assert!(write.content.contains("read-only tool context"));
    assert!(!root.join("note.txt").exists());
}

#[tokio::test]
async fn write_tools_record_turn_ledger_when_turn_metadata_is_present() {
    let root = temp_workspace();
    fs::write(root.join("note.txt"), "base\n").expect("write fixture");
    let ledger = Arc::new(xuanyu_governance::TurnLedgerStore::new(root.clone()));
    let runtime =
        ToolRuntime::with_builtins(Arc::new(allowing_governance()), vec![root.clone()])
            .with_turn_ledger(ledger.clone());

    let edit = runtime
        .execute(ToolCall {
            name: "file_edit".to_string(),
            args: BTreeMap::from([
                ("path".to_string(), json!("note.txt")),
                ("old_text".to_string(), json!("base")),
                ("new_text".to_string(), json!("changed")),
                ("sessionId".to_string(), json!("session-1")),
                ("turnId".to_string(), json!("turn-1")),
                (
                    "userMessagePreview".to_string(),
                    json!("please change note"),
                ),
            ]),
            cwd: root.clone(),
        })
        .await;
    assert!(!edit.is_error, "{}", edit.content);

    let records = ledger.list("session-1").expect("turn ledger list");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].turn_id, "turn-1");
    assert_eq!(records[0].user_message_preview, "please change note");
    assert_eq!(records[0].mutations.len(), 1);
    assert_eq!(records[0].mutations[0].path, "note.txt");
    assert_eq!(
        records[0].mutations[0].action,
        xuanyu_governance::TurnFileAction::Modified
    );
    assert!(records[0].mutations[0].before_blob.is_some());
    assert!(records[0].mutations[0].after_blob.is_some());

    let restored = ledger
        .restore_workspace_from_ledger(xuanyu_governance::RestoreWorkspaceInput {
            session_id: "session-1",
            target_turn_id: "turn-1",
            turn_order: &["turn-1".to_string()],
        })
        .expect("restore from ledger");
    assert!(restored.conflicts.is_empty());
    assert_eq!(restored.restored, vec!["note.txt"]);
    assert_eq!(
        normalize_newlines(&fs::read_to_string(root.join("note.txt")).unwrap()),
        "base\n"
    );
}

fn run_git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .expect("run git fixture command");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn normalize_newlines(value: &str) -> String {
    value.replace("\r\n", "\n").replace('\r', "\n")
}
