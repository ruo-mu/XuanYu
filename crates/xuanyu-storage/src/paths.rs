use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// 应用目录名：Windows / macOS 用产品名，Linux 遵循 XDG 小写惯例。
#[cfg(any(windows, target_os = "macos"))]
pub const APP_DIR_NAME: &str = "XuanYu";
#[cfg(not(any(windows, target_os = "macos")))]
pub const APP_DIR_NAME: &str = "xuanyu";

/// 全局配置文件名。
pub const PROJECT_CONFIG_FILE_NAME: &str = "xuanyu.json";

/// 数据根显式覆盖用环境变量（命令行与自动化）。
const DATA_ROOT_ENV: &str = "XUANYU_DATA_DIR";

fn non_empty_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// 用户主目录：Windows 取 `USERPROFILE`，其余平台取 `HOME`。
fn user_home() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(profile) = non_empty_env("USERPROFILE") {
            return profile;
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(home) = non_empty_env("HOME") {
            return home;
        }
    }
    PathBuf::from(".")
}

/// 配置目录：Windows `%APPDATA%\XuanYu`、macOS `~/Library/Application Support/XuanYu`、
/// 其余 unix `$XDG_CONFIG_HOME/xuanyu`。
#[cfg(windows)]
pub fn config_dir() -> PathBuf {
    non_empty_env("APPDATA")
        .unwrap_or_else(|| user_home().join("AppData").join("Roaming"))
        .join(APP_DIR_NAME)
}

/// 配置目录：见 `config_dir` 的 Windows 分支。
#[cfg(target_os = "macos")]
pub fn config_dir() -> PathBuf {
    user_home()
        .join("Library")
        .join("Application Support")
        .join(APP_DIR_NAME)
}

/// 配置目录：见 `config_dir` 的 Windows 分支。
#[cfg(all(unix, not(target_os = "macos")))]
pub fn config_dir() -> PathBuf {
    non_empty_env("XDG_CONFIG_HOME")
        .unwrap_or_else(|| user_home().join(".config"))
        .join(APP_DIR_NAME)
}

/// 平台标准数据目录：Windows `%LOCALAPPDATA%\XuanYu`、
/// macOS `~/Library/Application Support/XuanYu`、其余 unix `$XDG_DATA_HOME/xuanyu`。
#[cfg(windows)]
pub fn platform_data_dir() -> PathBuf {
    non_empty_env("LOCALAPPDATA")
        .unwrap_or_else(|| user_home().join("AppData").join("Local"))
        .join(APP_DIR_NAME)
}

/// 平台标准数据目录：见 `platform_data_dir` 的 Windows 分支。
#[cfg(target_os = "macos")]
pub fn platform_data_dir() -> PathBuf {
    user_home()
        .join("Library")
        .join("Application Support")
        .join(APP_DIR_NAME)
}

/// 平台标准数据目录：见 `platform_data_dir` 的 Windows 分支。
#[cfg(all(unix, not(target_os = "macos")))]
pub fn platform_data_dir() -> PathBuf {
    non_empty_env("XDG_DATA_HOME")
        .unwrap_or_else(|| user_home().join(".local").join("share"))
        .join(APP_DIR_NAME)
}

/// 平台标准日志目录：Windows `%LOCALAPPDATA%\XuanYu\logs`、
/// macOS `~/Library/Logs/XuanYu`、其余 unix `$XDG_STATE_HOME/xuanyu/logs`。
#[cfg(windows)]
pub fn logs_dir() -> PathBuf {
    platform_data_dir().join("logs")
}

/// 平台标准日志目录：见 `logs_dir` 的 Windows 分支。
#[cfg(target_os = "macos")]
pub fn logs_dir() -> PathBuf {
    user_home().join("Library").join("Logs").join(APP_DIR_NAME)
}

/// 平台标准日志目录：见 `logs_dir` 的 Windows 分支。
#[cfg(all(unix, not(target_os = "macos")))]
pub fn logs_dir() -> PathBuf {
    non_empty_env("XDG_STATE_HOME")
        .unwrap_or_else(|| user_home().join(".local").join("state"))
        .join(APP_DIR_NAME)
        .join("logs")
}

/// 全局配置文件路径：`{配置目录}/xuanyu.json`。
pub fn resolve_global_config_file() -> PathBuf {
    config_dir().join(PROJECT_CONFIG_FILE_NAME)
}

/// Windows 记事本等工具写入的 UTF-8 BOM，serde_json 会直接解析失败。
pub fn strip_utf8_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// 内置 PostgreSQL 数据目录（`initdb` 产物）：`{数据根}/store/postgres/data`。
///
/// 程序文件随产物落在安装目录（只读），数据目录落平台标准数据目录（可写）。
pub fn embedded_postgres_data_dir(data_root: &Path) -> PathBuf {
    data_root.join("store").join("postgres").join("data")
}

static CONFIGURED_DATA_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();

/// 运行时数据根，按三级覆盖链解析：环境变量 `XUANYU_DATA_DIR` > 配置 `appDataDir` >
/// 平台标准数据目录。
///
/// 所有运行时数据（项目级与全局）都放在此目录之下；配置文件独立位于配置目录。
pub fn runtime_data_root() -> PathBuf {
    if let Some(dir) = non_empty_env(DATA_ROOT_ENV) {
        return dir;
    }
    if let Some(dir) = configured_data_dir() {
        return dir;
    }
    platform_data_dir()
}

/// 配置 `appDataDir`；进程内只读取一次。
fn configured_data_dir() -> Option<PathBuf> {
    CONFIGURED_DATA_DIR
        .get_or_init(read_configured_data_dir)
        .clone()
}

fn read_configured_data_dir() -> Option<PathBuf> {
    let text = std::fs::read_to_string(resolve_global_config_file()).ok()?;
    let config: serde_json::Value = serde_json::from_str(strip_utf8_bom(&text)).ok()?;
    let dir = config.get("appDataDir")?.as_str()?.trim();
    if dir.is_empty() {
        None
    } else {
        Some(PathBuf::from(dir))
    }
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
