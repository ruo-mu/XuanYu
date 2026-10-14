use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::paths::embedded_postgres_data_dir;

/// 内置实例的固定连接参数：用户不接触任何数据库配置，连接串由应用写死。
const PORT: u16 = 55432;
const DATABASE: &str = "xuanyu";
const USERNAME: &str = "xuanyu";

/// 随产物落位的 PostgreSQL 程序目录名，与引擎可执行文件同级。
const PROGRAM_DIR_NAME: &str = "pg";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EmbeddedPostgresPhase {
    Ready,
    NeedsInit,
    NeedsStart,
    NeedsPgvector,
    MissingProgram,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddedPostgresStatus {
    pub phase: EmbeddedPostgresPhase,
    /// 程序文件（bin/lib/share 与 pgvector 扩展）是否齐备。
    pub program: bool,
    /// 数据目录是否已初始化（`initdb` 产物存在）。
    pub installed: bool,
    pub running: bool,
    pub pgvector: bool,
    pub port: u16,
    pub database: String,
    pub username: String,
    pub program_dir: String,
    pub data_dir: String,
    pub connection_string: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// 程序与数据的落位分离：程序文件随产物落安装目录（只读），
/// 数据目录落平台标准数据目录（可写），随应用启停。
#[derive(Debug, Clone)]
pub struct EmbeddedPostgresPaths {
    pub program_dir: PathBuf,
    pub bin_dir: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub database: String,
    pub username: String,
}

impl EmbeddedPostgresPaths {
    pub fn resolve(data_root: &Path) -> Self {
        let program_dir = program_dir();
        Self {
            bin_dir: program_dir.join("bin"),
            program_dir,
            data_dir: embedded_postgres_data_dir(data_root),
            port: PORT,
            database: DATABASE.to_string(),
            username: USERNAME.to_string(),
        }
    }

    pub fn connection_string(&self) -> String {
        format!(
            "postgres://{}@127.0.0.1:{}/{}",
            self.username, self.port, self.database
        )
    }

    #[cfg(unix)]
    fn lib_dir(&self) -> PathBuf {
        self.program_dir.join("lib")
    }

    /// 控制面执行 SQL 用的客户端：与内置实例同源，不依赖宿主环境是否有 psql。
    pub fn psql_path(&self) -> PathBuf {
        self.executable("psql")
    }

    fn executable(&self, name: &str) -> PathBuf {
        #[cfg(windows)]
        {
            self.bin_dir.join(format!("{name}.exe"))
        }
        #[cfg(not(windows))]
        {
            self.bin_dir.join(name)
        }
    }

    fn program_ready(&self) -> bool {
        self.executable("postgres").is_file()
            && self.executable("initdb").is_file()
            && self.executable("pg_ctl").is_file()
            && self.executable("psql").is_file()
    }

    fn data_initialized(&self) -> bool {
        self.data_dir.join("PG_VERSION").is_file()
    }
}

/// 内置 PostgreSQL 程序目录：与引擎可执行文件同级（产物内 `bin/pg`）。
fn program_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(PROGRAM_DIR_NAME)))
        .unwrap_or_else(|| PathBuf::from(PROGRAM_DIR_NAME))
}

pub fn embedded_postgres_status(paths: &EmbeddedPostgresPaths) -> EmbeddedPostgresStatus {
    let program = paths.program_ready();
    let installed = paths.data_initialized();
    let running = program
        && installed
        && paths.data_dir.join("postmaster.pid").is_file()
        && postgres_port_open(paths.port);
    let pgvector = running && postgres_has_pgvector(paths);
    let phase = if !program {
        EmbeddedPostgresPhase::MissingProgram
    } else if !installed {
        EmbeddedPostgresPhase::NeedsInit
    } else if !running {
        EmbeddedPostgresPhase::NeedsStart
    } else if !pgvector {
        EmbeddedPostgresPhase::NeedsPgvector
    } else {
        EmbeddedPostgresPhase::Ready
    };
    EmbeddedPostgresStatus {
        phase,
        program,
        installed,
        running,
        pgvector,
        port: paths.port,
        database: paths.database.clone(),
        username: paths.username.clone(),
        program_dir: path_to_string(&paths.program_dir),
        data_dir: path_to_string(&paths.data_dir),
        connection_string: paths.connection_string(),
        error: (!program).then(|| {
            format!(
                "Bundled PostgreSQL not found under {}",
                path_to_string(&paths.program_dir)
            )
        }),
    }
}

#[derive(Debug, Clone)]
pub struct EmbeddedPostgresProgress {
    pub phase: &'static str,
    pub percent: u8,
    pub message: String,
}

/// 首次运行：初始化数据目录（`initdb`）、拉起实例、建库、启用 pgvector。
///
/// 程序文件来自产物，不再有下载与解压；这里只产出数据目录。
pub fn prepare_embedded_postgres(
    paths: &EmbeddedPostgresPaths,
    mut on_progress: impl FnMut(EmbeddedPostgresProgress),
) -> Result<(String, Vec<String>), String> {
    if !paths.program_ready() {
        return Err(format!(
            "Bundled PostgreSQL not found under {}",
            path_to_string(&paths.program_dir)
        ));
    }

    let mut logs = Vec::new();
    fs::create_dir_all(&paths.data_dir).map_err(|err| err.to_string())?;
    #[cfg(unix)]
    restrict_dir_permissions(&paths.data_dir)?;
    logs.push("Prepared embedded PostgreSQL data directory".to_string());

    if !paths.data_initialized() {
        on_progress(EmbeddedPostgresProgress {
            phase: "initdb",
            percent: 25,
            message: "Initializing PostgreSQL data directory".to_string(),
        });
        let initdb = paths.executable("initdb");
        run_command(
            paths,
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
        tighten_local_postgres_config(&paths.data_dir, paths.port)?;
        logs.push("Initialized PostgreSQL data directory".to_string());
    }

    on_progress(EmbeddedPostgresProgress {
        phase: "start",
        percent: 55,
        message: "Starting embedded PostgreSQL".to_string(),
    });
    start_postgres(paths)?;
    logs.push("Started embedded PostgreSQL".to_string());

    on_progress(EmbeddedPostgresProgress {
        phase: "database",
        percent: 75,
        message: format!("Ensuring database {}", paths.database),
    });
    ensure_database(paths)?;
    logs.push(format!("Ensured database {}", paths.database));

    on_progress(EmbeddedPostgresProgress {
        phase: "pgvector_ext",
        percent: 90,
        message: "Enabling pgvector extension".to_string(),
    });
    logs.push(if enable_pgvector(paths) {
        "pgvector extension enabled".to_string()
    } else {
        "pgvector extension not available".to_string()
    });

    on_progress(EmbeddedPostgresProgress {
        phase: "done",
        percent: 100,
        message: "Embedded PostgreSQL is ready".to_string(),
    });
    Ok((paths.connection_string(), logs))
}

pub fn start_embedded_postgres(
    paths: &EmbeddedPostgresPaths,
) -> Result<(String, Vec<String>), String> {
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
/// 引擎退出并不会带走它。
pub fn stop_embedded_postgres(paths: &EmbeddedPostgresPaths) -> Result<(), String> {
    if !paths.data_dir.join("postmaster.pid").exists() {
        return Ok(());
    }
    let pg_ctl = paths.executable("pg_ctl");
    if !pg_ctl.exists() {
        return Ok(());
    }
    let mut child = pg_command(paths, &pg_ctl)
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

/// Unix 下 PostgreSQL 拒绝启动在权限过宽的数据目录上，创建后立即收紧为仅当前用户可访问。
#[cfg(unix)]
fn restrict_dir_permissions(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(|err| err.to_string())
}

/// 构造 PostgreSQL 子进程：可重定位构建的共享库位于程序目录的 `lib`，
/// 按可执行文件位置解析，不依赖宿主环境变量。
fn pg_command(paths: &EmbeddedPostgresPaths, program: &Path) -> Command {
    let command = Command::new(program);
    #[cfg(unix)]
    let command = {
        let lib_dir = paths.lib_dir();
        let mut command = command;
        command
            .env("DYLD_LIBRARY_PATH", &lib_dir)
            .env("LD_LIBRARY_PATH", &lib_dir);
        command
    };
    #[cfg(windows)]
    let _ = paths;
    command
}

fn start_postgres(paths: &EmbeddedPostgresPaths) -> Result<(), String> {
    if !paths.program_ready() {
        return Err(format!(
            "Bundled PostgreSQL not found under {}",
            path_to_string(&paths.program_dir)
        ));
    }
    let pg_ctl = paths.executable("pg_ctl");
    if postgres_port_open(paths.port) {
        if paths.data_dir.join("postmaster.pid").is_file() {
            return Ok(());
        }
        return Err(format!(
            "Port {} is already in use by another process; embedded PostgreSQL cannot start",
            paths.port
        ));
    }
    // 上次进程被强杀时 postmaster.pid 会留下；端口未监听说明它已失效，清掉再启动。
    let stale_pid = paths.data_dir.join("postmaster.pid");
    if stale_pid.is_file() {
        fs::remove_file(&stale_pid).map_err(|err| err.to_string())?;
    }
    // 不能用 run_command（内部是 Command::output()）：pg_ctl 把 postgres 拉成独立常驻进程，
    // 后者会继承管道句柄且不会退出，output() 永远等不到 EOF，表现为「启动数据库」永久卡死。
    // 因此丢弃子进程输出，改为轮询端口判断启动结果。
    let mut child = pg_command(paths, &pg_ctl)
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
    let sql = format!(
        "SELECT 1 FROM pg_database WHERE datname = '{}'",
        paths.database.replace('\'', "''")
    );
    if psql_scalar(paths, "postgres", &sql)?.trim() == "1" {
        return Ok(());
    }
    psql_exec(
        paths,
        "postgres",
        &format!("CREATE DATABASE \"{}\"", paths.database.replace('"', "\"\"")),
    )
}

fn enable_pgvector(paths: &EmbeddedPostgresPaths) -> bool {
    psql_exec(
        paths,
        &paths.database,
        "CREATE EXTENSION IF NOT EXISTS vector;",
    )
    .is_ok()
}

fn postgres_has_pgvector(paths: &EmbeddedPostgresPaths) -> bool {
    psql_scalar(
        paths,
        &paths.database,
        "SELECT EXISTS (SELECT 1 FROM pg_extension WHERE extname = 'vector');",
    )
    .map(|value| value.eq_ignore_ascii_case("t"))
    .unwrap_or(false)
}

fn postgres_port_open(port: u16) -> bool {
    std::net::TcpStream::connect(("127.0.0.1", port)).is_ok()
}

fn psql_exec(paths: &EmbeddedPostgresPaths, database: &str, sql: &str) -> Result<(), String> {
    let psql = paths.executable("psql");
    let output = pg_command(paths, &psql)
        .args(psql_args(paths, database, sql))
        .output()
        .map_err(|err| format!("Failed to run psql: {err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "psql failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn psql_scalar(
    paths: &EmbeddedPostgresPaths,
    database: &str,
    sql: &str,
) -> Result<String, String> {
    let output = pg_command(paths, &paths.executable("psql"))
        .args(psql_args(paths, database, sql))
        .arg("-At")
        .output()
        .map_err(|err| format!("Failed to run psql: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "psql failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn psql_args(paths: &EmbeddedPostgresPaths, database: &str, sql: &str) -> Vec<String> {
    vec![
        "-h".to_string(),
        "127.0.0.1".to_string(),
        "-p".to_string(),
        paths.port.to_string(),
        "-U".to_string(),
        paths.username.clone(),
        "-d".to_string(),
        database.to_string(),
        "-v".to_string(),
        "ON_ERROR_STOP=1".to_string(),
        "-c".to_string(),
        sql.to_string(),
    ]
}

fn run_command(
    paths: &EmbeddedPostgresPaths,
    program: &Path,
    args: &[&str],
) -> Result<(), String> {
    let output = pg_command(paths, program)
        .args(args)
        .output()
        .map_err(|err| format!("Failed to run {}: {err}", program.display()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{} failed: {}",
            program.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// 实例只监听回环地址；Unix 下把 socket 放进数据目录（仅当前用户可访问），
/// 不对其它用户暴露；pg_hba 只信任本机回环连接。
fn tighten_local_postgres_config(data_dir: &Path, port: u16) -> Result<(), String> {
    let conf_path = data_dir.join("postgresql.conf");
    let mut conf = fs::read_to_string(&conf_path).map_err(|err| err.to_string())?;
    conf.push_str(&format!("\nport = {port}\nlisten_addresses = '127.0.0.1'\n"));
    #[cfg(unix)]
    {
        let socket_dir = data_dir.to_string_lossy().replace('\'', "''");
        conf.push_str(&format!("unix_socket_directories = '{socket_dir}'\n"));
    }
    fs::write(conf_path, conf).map_err(|err| err.to_string())?;
    let hba = "local all all trust\nhost all all 127.0.0.1/32 trust\nhost all all ::1/128 trust\n";
    fs::write(data_dir.join("pg_hba.conf"), hba).map_err(|err| err.to_string())?;
    Ok(())
}

fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
