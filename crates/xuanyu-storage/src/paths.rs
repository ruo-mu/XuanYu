use std::path::{Path, PathBuf};

/// 全局应用数据目录名（`.xuanyu`）。
pub const APP_DIR_NAME: &str = ".xuanyu";

/// 全局配置文件名。
pub const PROJECT_CONFIG_FILE_NAME: &str = "xuanyu.json";

/// 用户主目录。
pub fn user_home() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(home);
    }
    if let Some(profile) = std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
        return PathBuf::from(profile);
    }
    PathBuf::from(".")
}

/// 未配置 appDataDir 时的默认应用数据目录（`{用户主目录}/.xuanyu`）。
pub fn global_app_dir() -> PathBuf {
    user_home().join(APP_DIR_NAME)
}

/// 全局配置文件路径（固定）：`{用户主目录}/.xuanyu/xuanyu.json`。
pub fn resolve_global_config_file() -> PathBuf {
    global_app_dir().join(PROJECT_CONFIG_FILE_NAME)
}

/// 默认运行时应用数据根目录（与参考项目默认 appDataDir 一致）。
pub fn default_app_data_dir() -> PathBuf {
    global_app_dir()
}

/// Windows 记事本等工具写入的 UTF-8 BOM，serde_json 会直接解析失败。
pub fn strip_utf8_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// Init 选择父目录时规范化为 `.xuanyu` 数据根。
pub fn resolve_app_dir_from_selection(selected_dir: &str) -> PathBuf {
    let trimmed = selected_dir.trim();
    if trimmed.is_empty() {
        return global_app_dir();
    }
    let resolved = PathBuf::from(trimmed);
    if resolved
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(APP_DIR_NAME))
    {
        return resolved;
    }
    resolved.join(APP_DIR_NAME)
}

/// `{appDataDir}/store/...`
pub fn store_path(data_dir: &Path, parts: &[&str]) -> PathBuf {
    let mut path = data_dir.join("store");
    for part in parts {
        path.push(part);
    }
    path
}

/// 嵌入式 PostgreSQL 根目录：`{appDataDir}/store/postgres`。
pub fn embedded_postgres_root(data_dir: &Path) -> PathBuf {
    store_path(data_dir, &["postgres"])
}

/// 内置 PostgreSQL 安装包缓存目录（跨重装复用）。
///
/// 归档包有数百 MB，而安装目录每次重装都会被清空，所以缓存必须放在安装目录之外：
/// Windows 为 `%LOCALAPPDATA%\XuanYu\cache\postgres`，其它平台为 `~/.cache/xuanyu/postgres`。
/// 可用环境变量 `XUANYU_POSTGRES_CACHE_DIR` 覆盖（测试时可指向已有归档的目录）。
pub fn postgres_cache_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("XUANYU_POSTGRES_CACHE_DIR").filter(|value| !value.is_empty())
    {
        return PathBuf::from(dir);
    }
    #[cfg(windows)]
    {
        if let Some(local) = std::env::var_os("LOCALAPPDATA").filter(|value| !value.is_empty()) {
            return PathBuf::from(local)
                .join("XuanYu")
                .join("cache")
                .join("postgres");
        }
    }
    user_home().join(".cache").join("xuanyu").join("postgres")
}

/// Desktop sqlite 控制面：`{appDataDir}/store/sqlite/xuanyu.sqlite`。
pub fn default_sqlite_database_path(data_dir: &Path) -> PathBuf {
    store_path(data_dir, &["sqlite", "xuanyu.sqlite"])
}

/// 运行时数据根：优先取环境变量 `XUANYU_DATA_DIR`（便携式/桌面安装时由启动器注入，
/// 指向 `<安装目录>\data`），未设置时回退 `{用户主目录}/.xuanyu`。
///
/// 所有运行时数据（项目级与全局）都应放在此目录之下；唯一例外是配置文件
/// `xuanyu.json`，它固定位于 `{用户主目录}/.xuanyu/xuanyu.json`。
/// 业务代码不应再在项目目录内创建 `.xuanyu` 目录。
pub fn runtime_data_root() -> PathBuf {
    if let Some(dir) = std::env::var_os("XUANYU_DATA_DIR").filter(|value| !value.is_empty()) {
        return PathBuf::from(dir);
    }
    global_app_dir()
}

/// 从项目根目录派生稳定、安全的项目键：目录名 sanitize + 绝对路径短哈希。
/// 保证不同项目（即使同名）在数据根下彼此隔离。
///
/// 哈希前先解析为绝对路径并把分隔符统一为 `/`，避免同一目录的
/// `C:\a\b` 与 `C:/a/b` 两种写法被算成不同的项目键。
pub fn project_data_key(project_dir: &Path) -> String {
    let abs = resolve_abs(project_dir);
    let name = abs
        .file_name()
        .and_then(|value| value.to_str())
        .map(sanitize_dir_segment)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "project".to_string());
    let normalized = abs.to_string_lossy().replace('\\', "/");
    format!("{name}-{}", short_hash_str(&normalized))
}

/// 项目级运行时数据根：`{runtime_data_root}/projects/{project_data_key}`。
/// 项目相关的插件、skill、工作区状态、turn-ledger 等都应放在此处，而不是项目目录内的 `.xuanyu`。
pub fn project_data_root(project_dir: &Path) -> PathBuf {
    runtime_data_root()
        .join("projects")
        .join(project_data_key(project_dir))
}

fn resolve_abs(path: &Path) -> PathBuf {
    path.canonicalize()
        .or_else(|_| std::path::absolute(path))
        .unwrap_or_else(|_| path.to_path_buf())
}

fn sanitize_dir_segment(value: &str) -> String {
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

fn short_hash_str(normalized: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalized.as_bytes().hash(&mut hasher);
    format!("{:08x}", hasher.finish() & 0xffff_ffff)
}
