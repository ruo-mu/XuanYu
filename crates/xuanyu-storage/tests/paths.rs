//! `xuanyu_storage::paths` 纯函数回归测试。
//!
//! 覆盖目标：BOM 剥除、appDataDir 解析、store 子路径拼接、SQLite 默认路径、Postgres 子目录。
//! 这些纯函数被 desktop-host/cli 启动路径大量调用，破坏会导致运行时崩。

use std::path::PathBuf;
use xuanyu_storage::{
    default_app_data_dir, default_sqlite_database_path, embedded_postgres_root,
    resolve_app_dir_from_selection, resolve_global_config_file, store_path, strip_utf8_bom,
};

#[test]
fn strip_utf8_bom_removes_leading_feff_and_leaves_plain_text_untouched() {
    assert_eq!(strip_utf8_bom("\u{feff}{\"k\":1}"), "{\"k\":1}");
    assert_eq!(strip_utf8_bom("{\"k\":1}"), "{\"k\":1}");
    assert_eq!(strip_utf8_bom(""), "");
}

#[test]
fn resolve_app_dir_from_selection_appends_xuanyu_dir_for_plain_paths() {
    let dir = resolve_app_dir_from_selection("/tmp/some-project");
    assert_eq!(dir, PathBuf::from("/tmp/some-project/.xuanyu"));
}

#[test]
fn resolve_app_dir_from_selection_keeps_existing_xuanyu_dir_verbatim() {
    let dir = resolve_app_dir_from_selection("/tmp/some-project/.xuanyu");
    assert_eq!(dir, PathBuf::from("/tmp/some-project/.xuanyu"));
}

#[test]
fn resolve_app_dir_from_selection_falls_back_to_global_when_empty() {
    let dir = resolve_app_dir_from_selection("");
    assert_eq!(dir, default_app_data_dir());
    let dir = resolve_app_dir_from_selection("   ");
    assert_eq!(dir, default_app_data_dir());
}

#[test]
fn resolve_global_config_file_resides_in_global_app_dir() {
    let path = resolve_global_config_file();
    assert!(path.ends_with("xuanyu.json"));
    assert!(path
        .components()
        .any(|c| c.as_os_str() == ".xuanyu"));
}

#[test]
fn store_path_joins_segments_under_store_root() {
    let root = PathBuf::from("/data");
    assert_eq!(
        store_path(&root, &["sqlite", "xuanyu.sqlite"]),
        PathBuf::from("/data/store/sqlite/xuanyu.sqlite")
    );
    assert_eq!(
        store_path(&root, &["postgres"]),
        PathBuf::from("/data/store/postgres")
    );
}

#[test]
fn embedded_postgres_root_is_store_postgres_under_data_dir() {
    let data = PathBuf::from("/data");
    assert_eq!(
        embedded_postgres_root(&data),
        PathBuf::from("/data/store/postgres")
    );
}

#[test]
fn default_sqlite_database_path_points_at_xuanyu_sqlite() {
    let data = PathBuf::from("/data");
    let path = default_sqlite_database_path(&data);
    assert_eq!(path, PathBuf::from("/data/store/sqlite/xuanyu.sqlite"));
    assert!(path.to_string_lossy().ends_with("xuanyu.sqlite"));
}
