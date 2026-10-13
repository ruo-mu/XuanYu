//! `xuanyu_storage::paths` 纯函数回归测试。
//!
//! 覆盖目标：BOM 剥除、配置路径、内置 PostgreSQL 数据目录拼接、项目数据键隔离。
//! 这些纯函数被 desktop-host/cli 启动路径大量调用，破坏会导致运行时崩。

use std::path::PathBuf;
use xuanyu_storage::{
    embedded_postgres_data_dir, project_data_key, resolve_global_config_file, strip_utf8_bom,
    APP_DIR_NAME,
};

#[test]
fn strip_utf8_bom_removes_leading_feff_and_leaves_plain_text_untouched() {
    assert_eq!(strip_utf8_bom("\u{feff}{\"k\":1}"), "{\"k\":1}");
    assert_eq!(strip_utf8_bom("{\"k\":1}"), "{\"k\":1}");
    assert_eq!(strip_utf8_bom(""), "");
}

#[test]
fn resolve_global_config_file_resides_in_app_dir() {
    let path = resolve_global_config_file();
    assert!(path.ends_with("xuanyu.json"));
    assert!(path.components().any(|c| c.as_os_str() == APP_DIR_NAME));
}

#[test]
fn embedded_postgres_data_dir_is_store_postgres_data_under_data_root() {
    let data = PathBuf::from("/data");
    assert_eq!(
        embedded_postgres_data_dir(&data),
        PathBuf::from("/data/store/postgres/data")
    );
}

#[test]
fn project_data_key_is_stable_and_isolated_per_directory() {
    let one = PathBuf::from("/data/project-a");
    let two = PathBuf::from("/data/project-b");
    assert_eq!(project_data_key(&one), project_data_key(&one));
    assert_ne!(project_data_key(&one), project_data_key(&two));
    assert!(project_data_key(&one).starts_with("project-a-"));
}
