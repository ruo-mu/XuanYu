use anyhow::Context;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use xuanyu_core::AppCore;
use xuanyu_protocol::{AppEvent, SessionId};
use xuanyu_storage::{
    embedded_postgres_status, install_embedded_postgres, runtime_data_root,
    EmbeddedPostgresInstallOptions, EmbeddedPostgresPaths, StorageConfig,
};

#[derive(Parser)]
#[command(name = "xuanyu", about = "XuanYu Agent headless CLI")]
struct Cli {
    #[arg(long)]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Init,
    SessionList,
    SessionCreate,
    SessionSend {
        session_id: String,
        message: String,
    },
    EventReplay {
        #[arg(long, default_value_t = 0)]
        last_sequence: u64,
        #[arg(long)]
        session_id: Option<String>,
    },
    ControlPlaneTasks {
        #[arg(long)]
        session_id: Option<String>,
        #[arg(long, default_value_t = 32)]
        limit: usize,
    },
    /// 验证并保存外部 PostgreSQL 连接（安装器「数据库配置」步骤用）。
    DbExternal {
        #[arg(long)]
        host: String,
        #[arg(long)]
        port: String,
        #[arg(long)]
        username: String,
        #[arg(long)]
        password: String,
        #[arg(long)]
        database: String,
        /// 可选：把结果 JSON 追加写入该文件（NSIS 通过文件读取结果）。
        #[arg(long)]
        result_file: Option<String>,
    },
    SmokeTurn,
    /// 便携式：下载并安装内置 PostgreSQL 到 `<data-dir>/store/postgres`。
    /// 由 NSIS 安装器在「便携式」模式下调用，进度逐行打印到 stdout。
    PortableInstall,
}

fn main() -> anyhow::Result<()> {
    let Cli { data_dir, command } = Cli::parse();
    let data_dir = data_dir.unwrap_or_else(runtime_data_root);
    // 便携式安装全程阻塞（下载 / 解压 / initdb），且 reqwest::blocking 的客户端不能在
    // tokio 运行时内创建或销毁（会 panic "Cannot drop a runtime in a context where
    // blocking is not allowed"），所以这条分支不建 runtime。
    if matches!(command, Commands::PortableInstall) {
        return run_portable_install(&data_dir);
    }
    tokio::runtime::Runtime::new()?.block_on(async move {
        match command {
            Commands::DbExternal {
                host,
                port,
                username,
                password,
                database,
                result_file,
            } => {
                run_db_external(&host, &port, &username, &password, &database, result_file.as_deref())
                    .await
            }
            command => {
                let core = Arc::new(AppCore::new(StorageConfig::for_data_dir(data_dir)));
                dispatch(core, command).await
            }
        }
    })
}

async fn dispatch(core: Arc<AppCore>, command: Commands) -> anyhow::Result<()> {
    match command {
        Commands::Init => {
            let status = core.initialize().await?;
            println!("{}", serde_json::to_string_pretty(&status)?);
        }
        Commands::SessionList => {
            core.initialize().await?;
            let sessions = core.list_sessions().await;
            println!("{}", serde_json::to_string_pretty(&sessions)?);
        }
        Commands::SessionCreate => {
            core.initialize().await?;
            let session = core.create_main_session().await?;
            println!("{}", serde_json::to_string_pretty(&session)?);
        }
        Commands::SessionSend {
            session_id,
            message,
        } => {
            core.initialize().await?;
            let turn = core
                .send_message(SessionId(session_id), message)
                .await
                .context("send message failed")?;
            println!("{}", serde_json::to_string_pretty(&turn)?);
        }
        Commands::EventReplay {
            last_sequence,
            session_id,
        } => {
            core.initialize().await?;
            let batch = if let Some(session_id) = session_id {
                core.event_replay_filtered(
                    last_sequence,
                    xuanyu_eventbus::EventFilter {
                        session_id: Some(session_id),
                        ..Default::default()
                    },
                )
                .await
            } else {
                core.event_replay(last_sequence).await
            };
            println!("{}", serde_json::to_string_pretty(&batch)?);
        }
        Commands::ControlPlaneTasks {
            session_id,
            limit,
        } => {
            core.initialize().await?;
            let tasks = core.control_plane_list_tasks(session_id, limit).await?;
            println!("{}", serde_json::to_string_pretty(&tasks)?);
        }
        Commands::SmokeTurn => run_smoke_turn(core).await?,
        Commands::DbExternal { .. } => unreachable!("handled in main"),
        Commands::PortableInstall => unreachable!("handled in main"),
    }
    Ok(())
}

async fn run_smoke_turn(core: Arc<AppCore>) -> anyhow::Result<()> {
    core.initialize().await?;
    let session = core.create_main_session().await?;
    let session_id = session.id.clone();
    let mut rx = core.eventbus().subscribe();
    let send = core.clone();
    let session_for_send = session_id.clone();
    let send_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        send.send_message(session_for_send, "smoke ping".to_string())
            .await
    });
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    let mut saw_turn = false;
    let mut saw_sequence = false;
    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(envelope) => {
                        if envelope.meta.contains_key("sequence") {
                            saw_sequence = true;
                        }
                        match &envelope.payload {
                            AppEvent::TurnFinished { session_id: sid, turn_id, .. }
                            | AppEvent::TurnFailed { session_id: sid, turn_id, .. }
                                if *sid == session_id =>
                            {
                                println!(
                                    "{}",
                                    serde_json::to_string_pretty(&serde_json::json!({
                                        "ok": matches!(envelope.payload, AppEvent::TurnFinished { .. }),
                                        "sessionId": sid.0,
                                        "turnId": turn_id.0,
                                    }))?
                                );
                                saw_turn = true;
                                break;
                            }
                            _ => {}
                        }
                    }
                    Err(_) => break,
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
    let _ = send_task.await;
    anyhow::ensure!(saw_sequence, "smoke failed: no event sequence observed");
    anyhow::ensure!(saw_turn, "smoke failed: turn did not finish in time");
    Ok(())
}

/// `portable-install`：下载并安装内置 PostgreSQL（安装器「便携式」模式用）。
///
/// 安装目标为 `<data-dir>/store/postgres`（`pgsql` 程序 + `data` 数据目录）。安装流程内部会
/// 临时拉起 PostgreSQL 以建库/装扩展，结束时这里再把它停掉，不把运行中的实例留给安装器；
/// 应用首次启动时自行拉起。失败时退出码为 1。
///
/// 输出必须是**纯 ASCII**：NSIS 的 `nsExec::ExecToLog` 把子进程字节流按系统 ANSI 代码页
/// （简体中文为 GBK）解释，而 Rust `println!` 输出 UTF-8，中文会变成乱码。因此中文提示一律
/// 交给 NSIS 侧 `DetailPrint`（NSIS 内部是 UTF-16，中文正常），这里只打印英文。
fn run_portable_install(data_dir: &std::path::Path) -> anyhow::Result<()> {
    let paths = EmbeddedPostgresPaths::from_data_dir(data_dir);
    println!("Installing embedded PostgreSQL (reuses the download cache when available).");
    println!("Keep the network connected and do not close the installer.");

    let mut last_percent = 0u8;
    let result = install_embedded_postgres(
        &paths,
        EmbeddedPostgresInstallOptions {
            skip_download: false,
            install_pgvector: true,
        },
        |progress| {
            // 下载阶段按 5% 步进打印，避免刷屏；其余阶段每次变化都打印
            let is_download = progress.phase.starts_with("download");
            if is_download && progress.percent < last_percent.saturating_add(5) {
                return;
            }
            last_percent = progress.percent;
            match (progress.downloaded, progress.total) {
                (Some(downloaded), Some(total)) if total > 0 => println!(
                    "[{:>3}%] {} ({:.1}/{:.1} MB)",
                    progress.percent,
                    progress.message,
                    downloaded as f64 / 1_048_576.0,
                    total as f64 / 1_048_576.0
                ),
                _ => println!("[{:>3}%] {}", progress.percent, progress.message),
            }
            // 解压这一步 tar/PowerShell 都不输出进度，实测约 3 分钟，明确告知用户避免以为卡死
            if progress.phase == "extract_postgres" {
                println!("      Unpacking about 30k files, this takes ~3 minutes. Please wait...");
            }
        },
    );

    // PostgreSQL 本体（下载/解压/initdb）已就绪即算安装成功。pgvector 走 GitHub，
    // 国内网络可能整体失败，此时只是缺向量扩展，不该让安装器报「安装失败」。
    let installed = embedded_postgres_status(&paths).installed;
    let (connection_string, logs) = match result {
        Ok(value) => value,
        Err(err) if installed => {
            println!("WARNING: {err}");
            println!(
                "PostgreSQL itself is installed and usable; the missing part (usually the \
                 pgvector extension) is retried automatically on first launch."
            );
            (paths.connection_string(), Vec::new())
        }
        Err(err) => {
            println!("ERROR: {err}");
            anyhow::bail!(err)
        }
    };

    for line in logs {
        println!("{line}");
    }
    // 装完保持实例运行：用户装完立刻打开应用就能直接连上，不必再等数据库冷启动。
    // 应用退出时引擎不再停库，实例保持常驻，下次启动直接复用；重装/卸载前由安装器清理残留进程。
    let status = embedded_postgres_status(&paths);
    if status.running {
        println!("Embedded PostgreSQL is running: {connection_string}");
    } else {
        println!(
            "NOTE: embedded PostgreSQL is not listening on port {} yet; the app starts it on launch.",
            paths.port
        );
    }
    // 装完就把数据库配置写进全局配置，应用首次启动即可直接连上，向导不必再问数据库。
    let config_path = xuanyu_storage::resolve_global_config_file();
    save_memory_postgres(&connection_string)?;
    println!(
        "Wrote database configuration to {}",
        config_path.display()
    );
    println!("Embedded PostgreSQL installed: {connection_string}");
    Ok(())
}

/// `db-external`：验证并保存外部 PostgreSQL 连接（安装器「数据库配置」步骤用）。
///
/// 成功时把连接串写入全局配置 `~/.xuanyu/xuanyu.json` 的 `memory.postgres.connectionString`
/// （保留其余字段），并输出一行 JSON `{"ok":true,"version":...,"connectionString":...}`；
/// 失败时输出 `{"ok":false,"message":...}` 且退出码为 1。
async fn run_db_external(
    host: &str,
    port: &str,
    username: &str,
    password: &str,
    database: &str,
    result_file: Option<&str>,
) -> anyhow::Result<()> {
    let outcome = build_and_probe_external(host, port, username, password, database)
        .await
        .and_then(|(connection_string, version)| {
            save_memory_postgres(&connection_string)?;
            Ok(serde_json::json!({
                "ok": true,
                "version": version,
                "connectionString": connection_string,
            }))
        })
        .unwrap_or_else(|err| serde_json::json!({ "ok": false, "message": err.to_string() }));

    let line = outcome.to_string();
    println!("{line}");
    if let Some(path) = result_file {
        // The installer reads this file with FileReadUTF16LE (it is built as a
        // Unicode installer), so encode as UTF-16LE with no BOM. Anything else
        // is decoded with the system ANSI code page and shows up as mojibake.
        // Single line, format: "OK <version>" / "ERR <message>".
        let result = if outcome.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
            let version = outcome
                .get("version")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            format!("OK {}", short_version(version))
        } else {
            let message = outcome
                .get("message")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("connection failed");
            format!("ERR {}", single_line(message))
        };
        if let Err(err) = std::fs::write(path, encode_utf16le(&result)) {
            eprintln!("warning: failed to write result file {path}: {err:#}");
        }
    }
    if outcome.get("ok").and_then(serde_json::Value::as_bool) != Some(true) {
        std::process::exit(1);
    }
    Ok(())
}

/// `SELECT version()` answers with something like
/// "PostgreSQL 17.6 on x86_64-windows, compiled by ...". The installer shows
/// this on a single label, so keep only the product name and version.
fn short_version(version: &str) -> String {
    version
        .split(" on ")
        .next()
        .unwrap_or(version)
        .trim()
        .chars()
        .take(32)
        .collect()
}

/// The installer reads the result file as a single line.
fn single_line(message: &str) -> String {
    let flattened: String = message
        .chars()
        .map(|c| if c == '\r' || c == '\n' { ' ' } else { c })
        .collect();
    flattened.trim().chars().take(120).collect()
}

/// UTF-16LE without a BOM, matching NSIS `FileReadUTF16LE`.
fn encode_utf16le(text: &str) -> Vec<u8> {
    text.encode_utf16().flat_map(u16::to_le_bytes).collect()
}

async fn build_and_probe_external(
    host: &str,
    port: &str,
    username: &str,
    password: &str,
    database: &str,
) -> anyhow::Result<(String, String)> {
    let port_num: u16 = port.trim().parse().context("端口无效 / invalid port")?;
    let host = host.trim();
    let username = username.trim();
    let database = database.trim();
    if host.is_empty() || username.is_empty() {
        anyhow::bail!("请填写主机和用户名 / host and username are required");
    }
    let host_for_url = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let connection_string = format!(
        "postgres://{}:{}@{host_for_url}:{port_num}/{}",
        percent_encode(username),
        percent_encode(password),
        percent_encode(database),
    );
    let version = xuanyu_storage::probe_external_postgres_connection(&connection_string).await?;
    Ok((connection_string, version))
}

/// 把数据库连接串写入全局配置 `memory.backend` + `memory.postgres.connectionString`，
/// 保留其余字段原样。
///
/// 安装器在装完数据库后立刻调用，这样应用首次启动时 `xuanyu.json` 里已经有数据库配置，
/// 配置向导不需要再问一遍数据库。
fn save_memory_postgres(connection_string: &str) -> anyhow::Result<()> {
    let config_path = xuanyu_storage::resolve_global_config_file();
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut next = std::fs::read_to_string(&config_path)
        .ok()
        .and_then(|text| {
            serde_json::from_str::<serde_json::Value>(xuanyu_storage::strip_utf8_bom(&text)).ok()
        })
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    let object = next.as_object_mut().expect("object");
    let memory = object
        .entry("memory".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !memory.is_object() {
        *memory = serde_json::json!({});
    }
    let memory = memory.as_object_mut().expect("object");
    memory.insert(
        "backend".to_string(),
        serde_json::Value::String("postgres".to_string()),
    );
    let postgres = memory
        .entry("postgres".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !postgres.is_object() {
        *postgres = serde_json::json!({});
    }
    postgres
        .as_object_mut()
        .expect("object")
        .insert(
            "connectionString".to_string(),
            serde_json::Value::String(connection_string.to_string()),
        );
    std::fs::write(&config_path, serde_json::to_string_pretty(&next)?)?;
    Ok(())
}

/// Percent-encode，等价于 JS `encodeURIComponent`（保留 A-Z a-z 0-9 - _ . ~）。
fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
