//! OS 原生沙箱：macOS sandbox-exec / Linux bwrap；不可用则 fail-closed。

use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

/// If sandbox is enabled and this platform cannot wrap, return a reject reason.
pub fn os_sandbox_exec_block_reason() -> Option<String> {
    if !os_sandbox_enabled_from_config() {
        return None;
    }
    match std::env::consts::OS {
        "macos" if which("sandbox-exec").is_some() => None,
        "linux" if which("bwrap").is_some() => None,
        "macos" => Some(
            "OS sandbox is enabled but sandbox-exec was not found. Refuse exec (fail-closed)."
                .to_string(),
        ),
        "linux" => Some(
            "OS sandbox is enabled but bwrap was not found. Refuse exec (fail-closed).".to_string(),
        ),
        _ => None,
    }
}

/// Build a shell Command, wrapping with Seatbelt/bwrap when OS sandbox is enabled.
pub fn shell_command_with_os_sandbox(command: &str, cwd: &Path) -> Result<Command, String> {
    if let Some(reason) = os_sandbox_exec_block_reason() {
        return Err(reason);
    }
    let inner = base_shell_command(command);
    if !os_sandbox_enabled_from_config() {
        let mut cmd = inner;
        cmd.current_dir(cwd);
        return Ok(cmd);
    }
    let network_off = os_sandbox_network_off_from_config();
    match std::env::consts::OS {
        "macos" => wrap_macos(inner, cwd, network_off),
        "linux" => wrap_linux(inner, cwd, network_off),
        _ => {
            let mut cmd = inner;
            cmd.current_dir(cwd);
            Ok(cmd)
        }
    }
}

fn base_shell_command(command: &str) -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new("powershell.exe");
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            command,
        ]);
        cmd
    } else {
        let shell = std::env::var_os("SHELL")
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| std::ffi::OsString::from("/bin/sh"));
        let mut cmd = Command::new(shell);
        cmd.args(["-lc", command]);
        cmd
    }
}

fn wrap_macos(inner: Command, cwd: &Path, network_off: bool) -> Result<Command, String> {
    let profile = write_seatbelt_profile(cwd, network_off)
        .map_err(|err| format!("failed to write Seatbelt profile: {err}"))?;
    let (prog, args) = program_and_args(&inner);
    let mut cmd = Command::new("sandbox-exec");
    cmd.arg("-f").arg(profile).arg(prog).args(args);
    cmd.current_dir(cwd);
    Ok(cmd)
}

fn wrap_linux(inner: Command, cwd: &Path, network_off: bool) -> Result<Command, String> {
    let cwd_s = cwd.to_string_lossy().to_string();
    let (prog, args) = program_and_args(&inner);
    let mut cmd = Command::new("bwrap");
    cmd.args([
        "--die-with-parent",
        "--new-session",
        "--ro-bind",
        "/usr",
        "/usr",
        "--ro-bind",
        "/bin",
        "/bin",
        "--ro-bind",
        "/lib",
        "/lib",
        "--ro-bind-try",
        "/lib64",
        "/lib64",
        "--ro-bind-try",
        "/etc",
        "/etc",
        "--bind",
        &cwd_s,
        &cwd_s,
        "--chdir",
        &cwd_s,
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
    ]);
    if network_off {
        cmd.arg("--unshare-net");
    }
    cmd.arg(prog).args(args);
    Ok(cmd)
}

fn program_and_args(cmd: &Command) -> (String, Vec<String>) {
    let program = cmd.get_program().to_string_lossy().to_string();
    let args = cmd
        .get_args()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect();
    (program, args)
}

fn write_seatbelt_profile(cwd: &Path, network_off: bool) -> std::io::Result<PathBuf> {
    let cwd_s = cwd.to_string_lossy().replace('\\', "/");
    let network = if network_off {
        "(deny network*)"
    } else {
        "(allow network*)"
    };
    let body = format!(
        r#"(version 1)
(deny default)
(allow process-exec)
(allow process-fork)
(allow sysctl-read)
(allow file-read*)
(allow file-write* (subpath "{cwd_s}"))
(allow file-write* (subpath "/private/tmp"))
(allow file-write* (subpath "/tmp"))
{network}
"#
    );
    let dir = std::env::temp_dir().join("xuanyu-os-sandbox");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("seatbelt-{}.sb", std::process::id()));
    std::fs::write(&path, body)?;
    Ok(path)
}

fn which(bin: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).find_map(|dir| {
            let candidate = dir.join(bin);
            candidate.is_file().then_some(candidate)
        })
    })
}

fn os_sandbox_enabled_from_config() -> bool {
    if env_truthy("XUANYU_OS_SANDBOX") {
        return true;
    }
    if let Some(enabled) = read_bool_at(
        &config_file_path(),
        &["/sandbox/osSandbox/enabled", "/sandbox/os_sandbox/enabled"],
    ) {
        return enabled;
    }
    false
}

fn os_sandbox_network_off_from_config() -> bool {
    if let Some(value) = read_bool_at(
        &config_file_path(),
        &[
            "/sandbox/osSandbox/networkOff",
            "/sandbox/os_sandbox/network_off",
        ],
    ) {
        return value;
    }
    true
}

fn env_truthy(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn config_file_path() -> PathBuf {
    xuanyu_storage::resolve_global_config_file()
}

fn read_bool_at(path: &Path, pointers: &[&str]) -> Option<bool> {
    let text = std::fs::read_to_string(path).ok()?;
    let value: Value = serde_json::from_str(&text).ok()?;
    for pointer in pointers {
        if let Some(flag) = value.pointer(pointer).and_then(Value::as_bool) {
            return Some(flag);
        }
    }
    None
}

#[allow(dead_code)]
pub fn shell_stdio_null(cmd: &mut Command) {
    cmd.stdin(Stdio::null());
}
