use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::paths::{embedded_postgres_root, postgres_cache_dir};

const DEFAULT_PORT: u16 = 55432;
const DEFAULT_DATABASE: &str = "xuanyu";
const DEFAULT_USERNAME: &str = "xuanyu";
const POSTGRES_ARCHIVE_NAMES: &[&str] = &[
    "postgresql-17.6-1-windows-x64-binaries.zip",
    "postgresql-windows-x64-binaries.zip",
    "postgres.zip",
];
const PGVECTOR_ARCHIVE_NAMES: &[&str] = &[
    "vector.v0.8.1-pg17.zip",
    "pgvector.zip",
];

#[cfg(windows)]
const POSTGRES_DOWNLOAD_URL: &str =
    "https://get.enterprisedb.com/postgresql/postgresql-17.6-1-windows-x64-binaries.zip";
#[cfg(windows)]
const POSTGRES_SHA256: &str = "d378882abd001a186735acd6f6ba716bca6ccd192e800412d4fd15ed25376b3e";
#[cfg(windows)]
const PGVECTOR_DOWNLOAD_URL: &str = "https://github.com/andreiramani/pgvector_pgsql_windows/releases/download/0.8.1_17.6/vector.v0.8.1-pg17.zip";
#[cfg(windows)]
const PGVECTOR_SHA256: &str = "0ccf5dafc5a98c290871244aa71f3189afc6a441c67cc12eca938473faa3132e";
#[cfg(windows)]
const PGVECTOR_DOWNLOAD_MIRRORS: &[&str] = &[
    PGVECTOR_DOWNLOAD_URL,
    "https://gh-proxy.com/https://github.com/andreiramani/pgvector_pgsql_windows/releases/download/0.8.1_17.6/vector.v0.8.1-pg17.zip",
    "https://ghproxy.net/https://github.com/andreiramani/pgvector_pgsql_windows/releases/download/0.8.1_17.6/vector.v0.8.1-pg17.zip",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedPostgresArchiveInfo {
    pub path: String,
    pub exists: bool,
    pub usable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddedPostgresPhase {
    Ready,
    NeedsInstall,
    NeedsStart,
    NeedsPgvector,
    NotRunning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedPostgresStatus {
    pub phase: EmbeddedPostgresPhase,
    pub installed: bool,
    pub pgvector_files: bool,
    pub running: bool,
    pub pgvector: bool,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub install_dir: String,
    pub data_dir: String,
    pub cache_dir: String,
    pub archives: Value,
    pub connection_string: String,
    pub embedded: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct EmbeddedPostgresPaths {
    pub install_dir: PathBuf,
    pub data_dir: PathBuf,
    /// 归档包缓存目录（安装目录之外，重装不丢）。
    pub cache_dir: PathBuf,
    pub bin_dir: PathBuf,
    pub port: u16,
    pub database: String,
    pub username: String,
}

impl EmbeddedPostgresPaths {
    pub fn from_data_dir(data_dir: &Path) -> Self {
        let root_dir = embedded_postgres_root(data_dir);
        let install_dir = root_dir.join("pgsql");
        Self {
            bin_dir: install_dir.join("bin"),
            data_dir: root_dir.join("data"),
            cache_dir: postgres_cache_dir(),
            install_dir,
            port: DEFAULT_PORT,
            database: DEFAULT_DATABASE.to_string(),
            username: DEFAULT_USERNAME.to_string(),
        }
    }

    pub fn connection_string(&self) -> String {
        format!(
            "postgres://{}@127.0.0.1:{}/{}",
            self.username, self.port, self.database
        )
    }
}

pub fn embedded_postgres_status(paths: &EmbeddedPostgresPaths) -> EmbeddedPostgresStatus {
    let installed = paths.bin_dir.join("initdb.exe").exists()
        || paths.bin_dir.join("initdb").exists();
    let data_initialized = paths.data_dir.join("PG_VERSION").exists();
    let pgvector_files = paths
        .install_dir
        .join("lib")
        .join("vector.dll")
        .exists()
        || paths.install_dir.join("lib").join("vector.so").exists();
    let running = installed
        && data_initialized
        && paths.data_dir.join("postmaster.pid").exists()
        && postgres_port_open(paths.port);
    let pgvector = running && postgres_has_pgvector(paths);
    let phase = if !installed || !data_initialized {
        EmbeddedPostgresPhase::NeedsInstall
    } else if !running {
        EmbeddedPostgresPhase::NeedsStart
    } else if !pgvector {
        EmbeddedPostgresPhase::NeedsPgvector
    } else {
        EmbeddedPostgresPhase::Ready
    };
    EmbeddedPostgresStatus {
        phase,
        installed: installed && data_initialized,
        pgvector_files,
        running,
        pgvector,
        port: paths.port,
        database: paths.database.clone(),
        username: paths.username.clone(),
        install_dir: path_to_string(&paths.install_dir),
        data_dir: path_to_string(&paths.data_dir),
        cache_dir: path_to_string(&paths.cache_dir),
        archives: json!({
            "postgres": postgres_archive_info(&paths.cache_dir),
            "pgvector": pgvector_archive_info(&paths.cache_dir),
        }),
        connection_string: paths.connection_string(),
        embedded: json!({
            "backend": "postgres",
            "enabled": true,
            "installDir": path_to_string(&paths.install_dir),
            "dataDir": path_to_string(&paths.data_dir),
            "port": paths.port,
            "database": paths.database,
            "username": paths.username,
        }),
        error: None,
    }
}

#[derive(Debug, Clone, Default)]
pub struct EmbeddedPostgresInstallOptions {
    pub skip_download: bool,
    pub install_pgvector: bool,
}

#[derive(Debug, Clone)]
pub struct EmbeddedPostgresProgress {
    pub phase: &'static str,
    pub percent: u8,
    pub message: String,
    pub downloaded: Option<u64>,
    pub total: Option<u64>,
}

pub fn install_embedded_postgres(
    paths: &EmbeddedPostgresPaths,
    options: EmbeddedPostgresInstallOptions,
    mut on_progress: impl FnMut(EmbeddedPostgresProgress),
) -> Result<(String, Vec<String>), String> {
    let mut logs = Vec::new();
    on_progress(EmbeddedPostgresProgress {
        phase: "prepare",
        percent: 5,
        message: "Preparing embedded PostgreSQL directories".to_string(),
        downloaded: None,
        total: None,
    });
    fs::create_dir_all(&paths.install_dir).map_err(|err| err.to_string())?;
    fs::create_dir_all(&paths.cache_dir).map_err(|err| err.to_string())?;
    fs::create_dir_all(&paths.data_dir).map_err(|err| err.to_string())?;
    logs.push("Created install/cache/data directories".to_string());

    let mut postgres_archive =
        find_usable_archive(&paths.cache_dir, POSTGRES_ARCHIVE_NAMES, postgres_expected_sha256());
    if let Some(archive) = &postgres_archive {
        logs.push(format!(
            "Using cached PostgreSQL archive {}",
            path_to_string(archive)
        ));
    }
    if !options.skip_download && postgres_archive.is_none() {
        #[cfg(windows)]
        {
            let destination = paths.cache_dir.join(POSTGRES_ARCHIVE_NAMES[0]);
            on_progress(EmbeddedPostgresProgress {
                phase: "download_postgres",
                percent: 10,
                message: "Downloading PostgreSQL binaries".to_string(),
                downloaded: None,
                total: None,
            });
            download_file_with_progress(
                &[POSTGRES_DOWNLOAD_URL],
                &destination,
                Some(POSTGRES_SHA256),
                "download_postgres",
                &mut on_progress,
            )?;
            logs.push(format!(
                "Downloaded PostgreSQL archive to {}",
                path_to_string(&destination)
            ));
            postgres_archive = Some(destination);
        }
        #[cfg(not(windows))]
        {
            return Err(format!(
                "PostgreSQL archive not found in {}. Download one of {:?} first.",
                path_to_string(&paths.cache_dir),
                POSTGRES_ARCHIVE_NAMES
            ));
        }
    }
    if let Some(archive) = postgres_archive {
        on_progress(EmbeddedPostgresProgress {
            phase: "extract_postgres",
            percent: 25,
            message: format!(
                "Extracting {}",
                archive
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
                    .unwrap_or_else(|| path_to_string(&archive))
            ),
            downloaded: None,
            total: None,
        });
        extract_zip(&archive, &paths.install_dir)?;
        logs.push(format!("Extracted {}", path_to_string(&archive)));
        normalize_postgres_install_dir(&paths.install_dir)?;
    }

    let initdb = paths.bin_dir.join("initdb.exe");
    if !initdb.exists() {
        return Err(format!(
            "initdb not found under {}. Extract a PostgreSQL binary archive first.",
            path_to_string(&paths.bin_dir)
        ));
    }

    if !paths.data_dir.join("PG_VERSION").exists() {
        on_progress(EmbeddedPostgresProgress {
            phase: "initdb",
            percent: 45,
            message: "Initializing PostgreSQL data directory".to_string(),
            downloaded: None,
            total: None,
        });
        run_command(
            &initdb,
            &[
                "-D",
                &path_to_string(&paths.data_dir),
                "-U",
                &paths.username,
                "--encoding=UTF8",
                "--locale=C",
            ],
        )?;
        logs.push("Initialized PostgreSQL data directory".to_string());
        tighten_local_postgres_config(&paths.data_dir, paths.port)?;
    }

    if options.install_pgvector {
        let mut pgvector_archive =
            find_usable_archive(&paths.cache_dir, PGVECTOR_ARCHIVE_NAMES, pgvector_expected_sha256());
        if let Some(archive) = &pgvector_archive {
            logs.push(format!(
                "Using cached pgvector archive {}",
                path_to_string(archive)
            ));
        }
        if !options.skip_download && pgvector_archive.is_none() {
            #[cfg(windows)]
            {
                let destination = paths.cache_dir.join(PGVECTOR_ARCHIVE_NAMES[0]);
                on_progress(EmbeddedPostgresProgress {
                    phase: "download_pgvector",
                    percent: 50,
                    message: "Downloading pgvector extension".to_string(),
                    downloaded: None,
                    total: None,
                });
                download_file_with_progress(
                    PGVECTOR_DOWNLOAD_MIRRORS,
                    &destination,
                    Some(PGVECTOR_SHA256),
                    "download_pgvector",
                    &mut on_progress,
                )?;
                logs.push(format!(
                    "Downloaded pgvector archive to {}",
                    path_to_string(&destination)
                ));
                pgvector_archive = Some(destination);
            }
        }
        if let Some(archive) = pgvector_archive {
            on_progress(EmbeddedPostgresProgress {
                phase: "install_pgvector",
                percent: 60,
                message: "Installing pgvector extension files".to_string(),
                downloaded: None,
                total: None,
            });
            extract_zip(&archive, &paths.install_dir)?;
            logs.push(format!(
                "Installed pgvector from {}",
                path_to_string(&archive)
            ));
        }
    }

    on_progress(EmbeddedPostgresProgress {
        phase: "start",
        percent: 75,
        message: "Starting embedded PostgreSQL".to_string(),
        downloaded: None,
        total: None,
    });
    start_postgres(paths)?;
    logs.push("Started embedded PostgreSQL".to_string());

    on_progress(EmbeddedPostgresProgress {
        phase: "database",
        percent: 85,
        message: format!("Ensuring database {}", paths.database),
        downloaded: None,
        total: None,
    });
    ensure_database(paths)?;
    logs.push(format!("Ensured database {}", paths.database));

    on_progress(EmbeddedPostgresProgress {
        phase: "pgvector_ext",
        percent: 95,
        message: "Enabling pgvector extension".to_string(),
        downloaded: None,
        total: None,
    });
    let pgvector = enable_pgvector(paths);
    logs.push(if pgvector {
        "pgvector extension enabled".to_string()
    } else {
        "pgvector extension not available yet".to_string()
    });

    on_progress(EmbeddedPostgresProgress {
        phase: "done",
        percent: 100,
        message: "Embedded PostgreSQL is ready".to_string(),
        downloaded: None,
        total: None,
    });
    Ok((paths.connection_string(), logs))
}

pub fn start_embedded_postgres(paths: &EmbeddedPostgresPaths) -> Result<(String, Vec<String>), String> {
    start_postgres(paths)?;
    ensure_database(paths)?;
    let _ = enable_pgvector(paths);
    Ok((
        paths.connection_string(),
        vec!["Embedded PostgreSQL started".to_string()],
    ))
}

/// 停止内置 PostgreSQL（若该数据目录下确有实例在运行）。
///
/// 应用退出时调用：`pg_ctl start` 拉起的 postgres 进程与引擎进程相互独立，
/// 引擎退出并不会带走它，会留下孤儿进程并锁住数据目录（进而导致卸载删不干净）。
/// 只处理内置实例：外部数据库没有本地的 `postmaster.pid`，天然是空操作。
pub fn stop_embedded_postgres(paths: &EmbeddedPostgresPaths) -> Result<(), String> {
    if !paths.data_dir.join("postmaster.pid").exists() {
        return Ok(());
    }
    let pg_ctl = paths.bin_dir.join(if cfg!(windows) { "pg_ctl.exe" } else { "pg_ctl" });
    if !pg_ctl.exists() {
        return Ok(());
    }
    let mut child = Command::new(&pg_ctl)
        .args([
            "stop",
            "-D",
            &path_to_string(&paths.data_dir),
            "-m",
            "fast",
            "-w",
            "-t",
            "15",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("Failed to launch {}: {err}", pg_ctl.display()))?;

    // 与 start_postgres 同理：不读子进程管道，用端口关闭与否判断结果。
    if !wait_for_port(paths.port, false, Duration::from_secs(30)) {
        let _ = child.kill();
        return Err(format!(
            "PostgreSQL still listening on port {} after stop request",
            paths.port
        ));
    }
    wait_child_exit(&mut child, Duration::from_secs(10));
    Ok(())
}

fn postgres_archive_info(cache_dir: &Path) -> EmbeddedPostgresArchiveInfo {
    archive_info(cache_dir, POSTGRES_ARCHIVE_NAMES, postgres_expected_sha256())
}

fn pgvector_archive_info(cache_dir: &Path) -> EmbeddedPostgresArchiveInfo {
    archive_info(cache_dir, PGVECTOR_ARCHIVE_NAMES, pgvector_expected_sha256())
}

#[cfg(windows)]
fn postgres_expected_sha256() -> Option<&'static str> {
    Some(POSTGRES_SHA256)
}

#[cfg(not(windows))]
fn postgres_expected_sha256() -> Option<&'static str> {
    None
}

#[cfg(windows)]
fn pgvector_expected_sha256() -> Option<&'static str> {
    Some(PGVECTOR_SHA256)
}

#[cfg(not(windows))]
fn pgvector_expected_sha256() -> Option<&'static str> {
    None
}

fn archive_info(
    cache_dir: &Path,
    names: &[&str],
    expected_sha256: Option<&str>,
) -> EmbeddedPostgresArchiveInfo {
    match find_archive(cache_dir, names) {
        Some(path) => {
            let metadata = fs::metadata(&path).ok();
            let size_ok = metadata
                .as_ref()
                .map(|item| item.is_file() && item.len() > 1024)
                .unwrap_or(false);
            let sha_error = expected_sha256.and_then(|expected| {
                if size_ok {
                    verify_file_sha256(&path, expected).err()
                } else {
                    None
                }
            });
            EmbeddedPostgresArchiveInfo {
                path: path_to_string(&path),
                exists: true,
                usable: size_ok && sha_error.is_none(),
                size_bytes: metadata.map(|item| item.len()),
                error: sha_error,
            }
        }
        None => EmbeddedPostgresArchiveInfo {
            path: path_to_string(&cache_dir.join(names[0])),
            exists: false,
            usable: false,
            size_bytes: None,
            error: Some("Archive not downloaded yet".to_string()),
        },
    }
}

/// 流式校验，避免把 300+ MB 的安装包整个读进内存（状态查询会频繁调用）。
fn verify_file_sha256(path: &Path, expected: &str) -> Result<(), String> {
    let mut file = File::open(path).map_err(|err| err.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let digest = format!("{:x}", hasher.finalize());
    if digest.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(format!(
            "SHA256 mismatch for {} (expected {}, got {})",
            path_to_string(path),
            expected,
            digest
        ))
    }
}

fn download_file_with_progress(
    urls: &[&str],
    destination: &Path,
    expected_sha256: Option<&str>,
    phase: &'static str,
    on_progress: &mut impl FnMut(EmbeddedPostgresProgress),
) -> Result<(), String> {
    if destination.is_file() {
        if let Some(expected) = expected_sha256 {
            if verify_file_sha256(destination, expected).is_ok() {
                return Ok(());
            }
            fs::remove_file(destination).map_err(|err| err.to_string())?;
        } else if fs::metadata(destination)
            .map(|item| item.len() > 1024)
            .unwrap_or(false)
        {
            return Ok(());
        }
    }

    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }

    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(3600))
        .build()
        .map_err(|err| err.to_string())?;

    let mut last_error = String::new();
    for url in urls {
        match download_single_url(
            &client,
            url,
            destination,
            expected_sha256,
            phase,
            on_progress,
        ) {
            Ok(()) => return Ok(()),
            Err(error) => last_error = error,
        }
    }

    Err(format!(
        "Download failed from all mirrors. Last error: {last_error}. Place the archive manually in {}.",
        path_to_string(destination.parent().unwrap_or(Path::new(".")))
    ))
}

fn download_single_url(
    client: &reqwest::blocking::Client,
    url: &str,
    destination: &Path,
    expected_sha256: Option<&str>,
    phase: &'static str,
    on_progress: &mut impl FnMut(EmbeddedPostgresProgress),
) -> Result<(), String> {
    let temp_path = destination.with_extension("part");
    let mut response = client
        .get(url)
        .send()
        .map_err(|err| format!("GET {url} failed: {err}"))?;
    if !response.status().is_success() {
        return Err(format!("GET {url} returned HTTP {}", response.status()));
    }

    let total = response.content_length();
    let mut file = File::create(&temp_path).map_err(|err| err.to_string())?;
    let mut hasher = Sha256::new();
    let mut downloaded = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];

    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|err| format!("Download read failed: {err}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|err| err.to_string())?;
        hasher.update(&buffer[..read]);
        downloaded = downloaded.saturating_add(read as u64);

        let percent = match total {
            Some(total) if total > 0 => {
                10_u8.saturating_add(((downloaded * 35) / total).min(35) as u8)
            }
            _ => 20,
        };
        let message = match total {
            Some(total) if total > 0 => format!(
                "Downloading ({:.0}%)",
                (downloaded as f64 / total as f64) * 100.0
            ),
            _ => "Downloading…".to_string(),
        };
        on_progress(EmbeddedPostgresProgress {
            phase,
            percent,
            message,
            downloaded: Some(downloaded),
            total,
        });
    }

    file.sync_all().map_err(|err| err.to_string())?;
    drop(file);

    if let Some(expected) = expected_sha256 {
        let digest = format!("{:x}", hasher.finalize());
        if !digest.eq_ignore_ascii_case(expected) {
            let _ = fs::remove_file(&temp_path);
            return Err(format!(
                "SHA256 mismatch for downloaded file (expected {expected}, got {digest})"
            ));
        }
    }

    if destination.exists() {
        fs::remove_file(destination).map_err(|err| err.to_string())?;
    }
    fs::rename(&temp_path, destination).map_err(|err| err.to_string())?;
    Ok(())
}

fn find_archive(cache_dir: &Path, names: &[&str]) -> Option<PathBuf> {
    for name in names {
        let candidate = cache_dir.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    // 兜底：允许用户手工放入文件名不固定的包，但必须排除其它已知包。
    // 否则找 pgvector 时会匹配到 PostgreSQL 主包（同为 .zip），导致 pgvector 下载被跳过、
    // 还白解压一次 800MB 的主包。
    fs::read_dir(cache_dir)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| {
            let file_name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            path.extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("zip"))
                && !POSTGRES_ARCHIVE_NAMES
                    .iter()
                    .chain(PGVECTOR_ARCHIVE_NAMES.iter())
                    .any(|known| file_name.eq_ignore_ascii_case(known))
        })
}

/// 在缓存目录里找一个**校验通过**的归档包；损坏的包会被删掉，让后续重新下载。
///
/// `find_archive` 只看文件是否存在，缓存跨重装复用后一个残缺的包会导致解压出半套
/// PostgreSQL，所以在真正安装前必须校验 SHA256。
fn find_usable_archive(
    cache_dir: &Path,
    names: &[&str],
    expected_sha256: Option<&str>,
) -> Option<PathBuf> {
    let path = find_archive(cache_dir, names)?;
    let Some(expected) = expected_sha256 else {
        return Some(path);
    };
    match verify_file_sha256(&path, expected) {
        Ok(()) => Some(path),
        Err(_) => {
            let _ = fs::remove_file(&path);
            None
        }
    }
}

fn extract_zip(archive: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination).map_err(|err| err.to_string())?;

    // PowerShell 的 Expand-Archive 解压 PostgreSQL 这种上万文件的包要 8-10 分钟，
    // 期间安装器没有任何输出，用户会以为卡死。Windows 10 1803+ 自带 bsdtar，快一个量级。
    if let Some(tar) = system_tar() {
        let status = Command::new(&tar)
            .arg("-xf")
            .arg(archive)
            .arg("-C")
            .arg(destination)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if matches!(status, Ok(status) if status.success()) {
            return Ok(());
        }
    }

    let status = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "Expand-Archive -LiteralPath '{}' -DestinationPath '{}' -Force",
                archive.display(),
                destination.display()
            ),
        ])
        .status()
        .map_err(|err| format!("Failed to launch Expand-Archive: {err}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "Failed to extract archive {}",
            path_to_string(archive)
        ))
    }
}

fn system_tar() -> Option<PathBuf> {
    let root = std::env::var_os("SystemRoot")?;
    let candidate = PathBuf::from(root).join("System32").join("tar.exe");
    candidate.is_file().then_some(candidate)
}

fn normalize_postgres_install_dir(install_dir: &Path) -> Result<(), String> {
    let nested = fs::read_dir(install_dir)
        .map_err(|err| err.to_string())?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| path.is_dir() && path.join("bin").join("initdb.exe").exists());
    if let Some(nested) = nested {
        for entry in fs::read_dir(&nested).map_err(|err| err.to_string())? {
            let entry = entry.map_err(|err| err.to_string())?;
            let target = install_dir.join(entry.file_name());
            if target.exists() {
                continue;
            }
            fs::rename(entry.path(), target).map_err(|err| err.to_string())?;
        }
    }
    Ok(())
}

fn tighten_local_postgres_config(data_dir: &Path, port: u16) -> Result<(), String> {
    let conf_path = data_dir.join("postgresql.conf");
    let mut conf = fs::read_to_string(&conf_path).map_err(|err| err.to_string())?;
    conf.push_str(&format!("\nport = {port}\nlisten_addresses = '127.0.0.1'\n"));
    fs::write(conf_path, conf).map_err(|err| err.to_string())?;
    let hba_path = data_dir.join("pg_hba.conf");
    let hba = "host all all 127.0.0.1/32 trust\nhost all all ::1/128 trust\nlocal all all trust\n";
    fs::write(hba_path, hba).map_err(|err| err.to_string())?;
    Ok(())
}

fn start_postgres(paths: &EmbeddedPostgresPaths) -> Result<(), String> {
    let pg_ctl = paths.bin_dir.join("pg_ctl.exe");
    if !pg_ctl.exists() {
        return Err(format!(
            "pg_ctl not found under {}",
            path_to_string(&paths.bin_dir)
        ));
    }
    if postgres_port_open(paths.port) {
        return Ok(());
    }
    // 不能用 run_command（内部是 Command::output()）：pg_ctl 把 postgres 拉成独立常驻进程，
    // 后者会继承管道句柄且不会退出，output() 永远等不到 EOF，表现为「启动数据库」永久卡死。
    // 因此丢弃子进程输出，改为轮询端口判断启动结果。
    let mut child = Command::new(&pg_ctl)
        .args([
            "-D",
            &path_to_string(&paths.data_dir),
            "-l",
            &path_to_string(&paths.data_dir.join("server.log")),
            "start",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| format!("Failed to launch {}: {err}", pg_ctl.display()))?;

    if !wait_for_port(paths.port, true, Duration::from_secs(90)) {
        let _ = child.kill();
        return Err(format!(
            "PostgreSQL did not start listening on port {} within 90s (see {})",
            paths.port,
            path_to_string(&paths.data_dir.join("server.log"))
        ));
    }
    wait_child_exit(&mut child, Duration::from_secs(10));
    Ok(())
}

/// 轮询端口直到达到期望状态（`open = true` 等待监听、`false` 等待关闭）。
fn wait_for_port(port: u16, open: bool, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if postgres_port_open(port) == open {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// 有限等待子进程退出，超时则结束它，避免调用方被卡住。
fn wait_child_exit(child: &mut std::process::Child, timeout: Duration) {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        match child.try_wait() {
            Ok(Some(_)) => return,
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => return,
        }
    }
    let _ = child.kill();
}

fn ensure_database(paths: &EmbeddedPostgresPaths) -> Result<(), String> {
    let createdb = paths.bin_dir.join("createdb.exe");
    if !createdb.exists() {
        return Ok(());
    }
    let _ = run_command(
        &createdb,
        &[
            "-h",
            "127.0.0.1",
            "-p",
            &paths.port.to_string(),
            "-U",
            &paths.username,
            &paths.database,
        ],
    );
    Ok(())
}

fn enable_pgvector(paths: &EmbeddedPostgresPaths) -> bool {
    let psql = paths.bin_dir.join("psql.exe");
    if !psql.exists() {
        return false;
    }
    run_command(
        &psql,
        &[
            "-h",
            "127.0.0.1",
            "-p",
            &paths.port.to_string(),
            "-U",
            &paths.username,
            "-d",
            &paths.database,
            "-c",
            "CREATE EXTENSION IF NOT EXISTS vector;",
        ],
    )
    .is_ok()
}

fn postgres_has_pgvector(paths: &EmbeddedPostgresPaths) -> bool {
    let psql = paths.bin_dir.join("psql.exe");
    if !psql.exists() || !postgres_port_open(paths.port) {
        return false;
    }
    let output = match Command::new(&psql)
        .args([
            "-h",
            "127.0.0.1",
            "-p",
            &paths.port.to_string(),
            "-U",
            &paths.username,
            "-d",
            &paths.database,
            "-At",
            "-c",
            "SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'vector');",
        ])
        .output()
    {
        Ok(output) => output,
        Err(_) => return false,
    };
    output.status.success()
        && String::from_utf8_lossy(&output.stdout)
            .trim()
            .eq_ignore_ascii_case("t")
}

fn postgres_port_open(port: u16) -> bool {
    std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
}

fn run_command(program: &Path, args: &[&str]) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|err| format!("Failed to run {}: {err}", program.display()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} failed: {}",
            program.display(),
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
