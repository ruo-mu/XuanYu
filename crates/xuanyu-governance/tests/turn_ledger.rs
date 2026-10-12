use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::Utc;
use xuanyu_governance::{
    extract_write_paths, is_write_tool_name, RecordWriteInput, RestoreWorkspaceInput,
    TurnFileAction, TurnLedgerStore,
};

#[test]
fn extracts_write_paths_for_known_tools() {
    assert!(is_write_tool_name("file_write"));
    assert_eq!(
        extract_write_paths(
            "file_write",
            &serde_json::json!({ "path": ".\\src\\main.rs" })
        ),
        vec!["src/main.rs"]
    );
    assert_eq!(
        extract_write_paths(
            "apply_patch",
            &serde_json::json!({
                "patches": [
                    { "path": "a.txt" },
                    { "path": "nested/b.txt" },
                    { "content": "ignored" }
                ]
            })
        ),
        vec!["a.txt", "nested/b.txt"]
    );
    assert!(extract_write_paths("git_commit", &serde_json::json!({})).is_empty());
}

#[test]
fn records_lists_restores_and_deletes_turn_ledgers() {
    let root = temp_workspace();
    let store = TurnLedgerStore::new(root.clone());
    fs::write(root.join("note.txt"), "base\n").expect("base file");

    store
        .snapshot_before_write("session-1", "turn-1", &root, &["note.txt".to_string()])
        .expect("snapshot before t1");
    fs::write(root.join("note.txt"), "turn 1\n").expect("turn 1 write");
    store
        .record_write_result(RecordWriteInput {
            session_id: "session-1",
            turn_id: "turn-1",
            user_message_preview: "change note",
            tool_name: "file_edit",
            cwd: &root,
            affected_files: &["note.txt".to_string()],
        })
        .expect("record t1");

    store
        .snapshot_before_write("session-1", "turn-2", &root, &["new.txt".to_string()])
        .expect("snapshot before t2");
    fs::write(root.join("new.txt"), "new\n").expect("new file");
    store
        .record_write_result(RecordWriteInput {
            session_id: "session-1",
            turn_id: "turn-2",
            user_message_preview: "add note",
            tool_name: "file_write",
            cwd: &root,
            affected_files: &["new.txt".to_string()],
        })
        .expect("record t2");

    let ledgers = store.list("session-1").expect("list");
    assert_eq!(ledgers.len(), 2);
    assert_eq!(ledgers[0].mutations[0].action, TurnFileAction::Modified);
    assert_eq!(ledgers[1].mutations[0].action, TurnFileAction::Added);

    let turn_order = vec!["turn-1".to_string(), "turn-2".to_string()];
    assert_eq!(
        store
            .files_changed_after_turn("session-1", &turn_order, "turn-1")
            .expect("changed files"),
        vec!["new.txt", "note.txt"]
    );

    let restored = store
        .restore_workspace_from_ledger(RestoreWorkspaceInput {
            session_id: "session-1",
            target_turn_id: "turn-1",
            turn_order: &turn_order,
        })
        .expect("restore");
    assert!(restored.conflicts.is_empty());
    assert_eq!(restored.restored, vec!["note.txt"]);
    assert_eq!(restored.deleted, vec!["new.txt"]);
    assert_eq!(fs::read_to_string(root.join("note.txt")).unwrap(), "base\n");
    assert!(!root.join("new.txt").exists());

    store
        .delete_after("session-1", "turn-1", &turn_order)
        .expect("delete ledgers");
    assert!(store
        .list("session-1")
        .expect("list after delete")
        .is_empty());
}

#[test]
fn restore_reports_conflict_when_file_changed_after_agent_edit() {
    let root = temp_workspace();
    let store = TurnLedgerStore::new(root.clone());
    fs::write(root.join("note.txt"), "base\n").expect("base file");

    store
        .snapshot_before_write("session-1", "turn-1", &root, &["note.txt".to_string()])
        .expect("snapshot before");
    fs::write(root.join("note.txt"), "agent\n").expect("agent write");
    store
        .record_write_result(RecordWriteInput {
            session_id: "session-1",
            turn_id: "turn-1",
            user_message_preview: "change",
            tool_name: "file_edit",
            cwd: &root,
            affected_files: &["note.txt".to_string()],
        })
        .expect("record");

    fs::write(root.join("note.txt"), "manual\n").expect("manual edit");
    let result = store
        .restore_workspace_from_ledger(RestoreWorkspaceInput {
            session_id: "session-1",
            target_turn_id: "turn-1",
            turn_order: &["turn-1".to_string()],
        })
        .expect("restore");

    assert_eq!(result.conflicts.len(), 1);
    assert_eq!(result.conflicts[0].path, "note.txt");
    assert_eq!(
        fs::read_to_string(root.join("note.txt")).unwrap(),
        "manual\n"
    );
}

#[test]
fn workspace_checkpoint_returns_none_outside_git_repo() {
    let root = temp_workspace();
    let store = TurnLedgerStore::new(root);
    let record = store
        .capture_workspace_checkpoint("session-1", "turn-1")
        .expect("capture outside git");
    assert!(record.is_none());
}

#[test]
fn workspace_checkpoint_captures_and_restores_git_state() {
    let root = temp_workspace();
    init_git_repo(&root);
    fs::write(root.join("note.txt"), "base\n").expect("base file");
    fs::write(root.join("remove_me.txt"), "delete before turn\n").expect("delete fixture");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "init"]);

    fs::write(root.join("note.txt"), "dirty before turn\n").expect("dirty tracked");
    fs::remove_file(root.join("remove_me.txt")).expect("delete tracked");
    fs::write(root.join("scratch.txt"), "scratch before turn\n").expect("dirty untracked");
    fs::create_dir_all(root.join(".xuanyu").join("internal")).expect("internal dir");
    fs::write(
        root.join(".xuanyu").join("internal").join("ignored.txt"),
        "ignore\n",
    )
    .expect("internal file");

    let store = TurnLedgerStore::new(root.clone());
    let checkpoint = store
        .capture_workspace_checkpoint("session-1", "turn-1")
        .expect("capture checkpoint")
        .expect("git checkpoint");
    assert!(checkpoint.has_changes);
    assert_eq!(
        checkpoint.files,
        vec!["note.txt", "remove_me.txt", "scratch.txt"]
    );
    assert!(Path::new(&checkpoint.patch_path).is_file());
    let patch_text = fs::read_to_string(&checkpoint.patch_path).expect("patch text");
    assert!(!patch_text.contains(".xuanyu/internal"));

    fs::write(root.join("note.txt"), "after turn\n").expect("after tracked");
    fs::write(root.join("scratch.txt"), "after scratch\n").expect("after untracked");
    fs::write(root.join("after_only.txt"), "created after turn\n").expect("after untracked");
    let restored = store
        .restore_workspace_checkpoint("session-1", "turn-1")
        .expect("restore checkpoint")
        .expect("restored checkpoint");

    assert_eq!(restored.turn_id, "turn-1");
    assert_eq!(
        normalize_test_text(
            &fs::read_to_string(root.join("note.txt")).expect("restored tracked")
        ),
        "dirty before turn\n"
    );
    assert_eq!(
        normalize_test_text(
            &fs::read_to_string(root.join("scratch.txt")).expect("restored untracked")
        ),
        "scratch before turn\n"
    );
    assert!(!root.join("remove_me.txt").exists());
    assert!(!root.join("after_only.txt").exists());

    let listed = store
        .list_workspace_checkpoints("session-1")
        .expect("list checkpoints");
    assert_eq!(listed.len(), 1);
    store
        .delete_workspace_checkpoints_after("session-1", "turn-1", &["turn-1".to_string()])
        .expect("delete checkpoints");
    assert!(store
        .list_workspace_checkpoints("session-1")
        .expect("list after delete")
        .is_empty());
}

#[test]
fn rewind_preserves_unrelated_uncommitted_changes() {
    let root = temp_workspace();
    let store = TurnLedgerStore::new(root.clone());
    fs::write(root.join("a.txt"), "base\n").expect("a base");
    fs::write(root.join("b.txt"), "user draft\n").expect("unrelated file");

    store
        .snapshot_before_write("session-1", "turn-1", &root, &["a.txt".to_string()])
        .expect("snapshot before");
    fs::write(root.join("a.txt"), "agent\n").expect("agent write");
    store
        .record_write_result(RecordWriteInput {
            session_id: "session-1",
            turn_id: "turn-1",
            user_message_preview: "edit a",
            tool_name: "file_edit",
            cwd: &root,
            affected_files: &["a.txt".to_string()],
        })
        .expect("record");

    let result = store
        .restore_workspace_from_ledger(RestoreWorkspaceInput {
            session_id: "session-1",
            target_turn_id: "turn-1",
            turn_order: &["turn-1".to_string()],
        })
        .expect("restore");

    assert!(result.conflicts.is_empty());
    assert_eq!(result.restored, vec!["a.txt"]);
    assert!(result.deleted.is_empty());
    assert_eq!(fs::read_to_string(root.join("a.txt")).unwrap(), "base\n");
    // 用户与 turn 无关的未提交改动必须原样保留。
    assert_eq!(
        fs::read_to_string(root.join("b.txt")).unwrap(),
        "user draft\n"
    );
}

#[test]
fn workspace_checkpoint_restore_refuses_when_head_moved() {
    let root = temp_workspace();
    init_git_repo(&root);
    fs::write(root.join("a.txt"), "base\n").expect("a base");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "init"]);

    let store = TurnLedgerStore::new(root.clone());
    store
        .capture_workspace_checkpoint("session-1", "turn-1")
        .expect("capture")
        .expect("git checkpoint");

    // 产生一次新提交，使 HEAD 前进到 checkpoint 记录的 git_head 之后。
    fs::write(root.join("a.txt"), "committed after turn\n").expect("later write");
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "after turn"]);

    let err = store
        .restore_workspace_checkpoint("session-1", "turn-1")
        .expect_err("restore must refuse when HEAD moved");
    assert!(err.to_string().contains("HEAD moved"), "{err}");
    // 拒绝时不得改动 git 历史或工作区内容。
    assert_eq!(
        fs::read_to_string(root.join("a.txt")).unwrap(),
        "committed after turn\n"
    );
}

fn init_git_repo(root: &Path) {
    run_git(root, &["init"]);
    run_git(root, &["config", "user.email", "xuanyu@example.test"]);
    run_git(root, &["config", "user.name", "Xuanyu Test"]);
}

fn run_git(root: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .status()
        .expect("run git");
    assert!(status.success(), "git {:?} failed", args);
}

fn normalize_test_text(text: &str) -> String {
    text.replace("\r\n", "\n")
}

fn temp_workspace() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "xuanyu-turn-ledger-test-{}",
        Utc::now().timestamp_nanos_opt().unwrap_or_default()
    ));
    fs::create_dir_all(&path).expect("temp workspace");
    path
}
