use anyhow::{anyhow, Context};
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI64, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// Directory containing the running desktop executable (for portable worker bundles).
pub fn executable_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|dir| dir.to_path_buf()))
}

/// Hide the Windows console window for background worker subprocesses.
pub fn prepare_worker_command(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(unix)]
    let _ = command;
}

pub struct PersistentWorkerSession {
    child: Child,
    stdout: BufReader<std::process::ChildStdout>,
    _stderr_guard: thread::JoinHandle<()>,
}

impl PersistentWorkerSession {
    pub fn spawn(mut command: Command) -> anyhow::Result<Self> {
        prepare_worker_command(&mut command);
        command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .context("failed to spawn persistent worker process")?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| anyhow!("worker stderr unavailable"))?;
        let stderr_guard = thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                let trimmed = line.trim();
                if !trimmed.is_empty() {
                    tracing::debug!(target: "worker.stderr", "{trimmed}");
                }
                line.clear();
            }
        });
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("worker stdout unavailable"))?;
        Ok(Self {
            child,
            stdout: BufReader::new(stdout),
            _stderr_guard: stderr_guard,
        })
    }

    pub fn is_alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn request_line(&mut self, line: &str) -> anyhow::Result<String> {
        if !self.is_alive() {
            anyhow::bail!("persistent worker process is not running");
        }
        let stdin = self
            .child
            .stdin
            .as_mut()
            .ok_or_else(|| anyhow!("worker stdin unavailable"))?;
        stdin
            .write_all(line.as_bytes())
            .context("failed to write worker request")?;
        stdin
            .write_all(b"\n")
            .context("failed to write worker request newline")?;
        stdin.flush().context("failed to flush worker stdin")?;
        let mut response = String::new();
        self.stdout
            .read_line(&mut response)
            .context("failed to read worker response")?;
        if response.trim().is_empty() {
            anyhow::bail!("worker returned an empty response line");
        }
        Ok(response)
    }
}

impl fmt::Debug for PersistentWorkerSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersistentWorkerSession")
            .finish_non_exhaustive()
    }
}

pub fn persistent_workers_enabled() -> bool {
    !matches!(
        std::env::var("XUANYU_WORKER_PERSIST").ok().as_deref(),
        Some("0") | Some("false") | Some("FALSE")
    )
}

#[derive(Clone)]
pub struct WorkerSessionSlot {
    session: Arc<Mutex<Option<PersistentWorkerSession>>>,
    last_ok_at: Arc<AtomicI64>,
    consecutive_failures: Arc<AtomicU32>,
}

impl WorkerSessionSlot {
    pub fn new() -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
            last_ok_at: Arc::new(AtomicI64::new(0)),
            consecutive_failures: Arc::new(AtomicU32::new(0)),
        }
    }

    pub fn session_handle(&self) -> Arc<Mutex<Option<PersistentWorkerSession>>> {
        self.session.clone()
    }

    pub fn last_ok_ms(&self) -> Option<i64> {
        let value = self.last_ok_at.load(Ordering::Relaxed);
        if value > 0 {
            Some(value)
        } else {
            None
        }
    }

    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures.load(Ordering::Relaxed)
    }

    pub fn mark_success(&self) {
        self.last_ok_at.store(
            chrono::Utc::now().timestamp_millis(),
            Ordering::Relaxed,
        );
        self.consecutive_failures.store(0, Ordering::Relaxed);
    }

    pub fn mark_failure(&self) {
        self.consecutive_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub fn ensure<F>(&self, spawn: F) -> anyhow::Result<()>
    where
        F: FnOnce() -> anyhow::Result<Command>,
    {
        if !persistent_workers_enabled() {
            return Ok(());
        }
        let mut guard = self
            .session
            .lock()
            .map_err(|_| anyhow!("worker session lock poisoned"))?;
        let alive = guard
            .as_mut()
            .map(|session| session.is_alive())
            .unwrap_or(false);
        if alive {
            return Ok(());
        }
        let command = spawn()?;
        *guard = Some(PersistentWorkerSession::spawn(command)?);
        Ok(())
    }

    pub fn request_persistent<F>(
        &self,
        mut spawn: F,
        line: &str,
    ) -> anyhow::Result<String>
    where
        F: FnMut() -> anyhow::Result<Command>,
    {
        let mut last_error = None;
        for attempt in 0..2 {
            if let Err(err) = self.ensure(&mut spawn) {
                last_error = Some(err);
                if attempt == 0 {
                    continue;
                }
                return Err(last_error.unwrap());
            }
            let mut guard = self
                .session
                .lock()
                .map_err(|_| anyhow!("worker session lock poisoned"))?;
            match guard
                .as_mut()
                .ok_or_else(|| anyhow!("worker session unavailable"))?
                .request_line(line)
            {
                Ok(response) => {
                    self.mark_success();
                    return Ok(response);
                }
                Err(err) => {
                    last_error = Some(err);
                    self.mark_failure();
                    *guard = None;
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow!("worker persistent request failed")))
    }

    pub fn cache_fresh(last_ok_ms: Option<i64>, ttl: Duration) -> bool {
        let Some(last_ok_ms) = last_ok_ms else {
            return false;
        };
        let now = chrono::Utc::now().timestamp_millis();
        now.saturating_sub(last_ok_ms) <= ttl.as_millis() as i64
    }

    pub fn since_last_ok(last_ok_ms: Option<i64>) -> Option<Duration> {
        let last_ok_ms = last_ok_ms?;
        let elapsed = chrono::Utc::now().timestamp_millis().saturating_sub(last_ok_ms);
        if elapsed < 0 {
            return None;
        }
        Some(Duration::from_millis(elapsed as u64))
    }
}

impl Default for WorkerSessionSlot {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for WorkerSessionSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WorkerSessionSlot")
            .field("last_ok_ms", &self.last_ok_at.load(Ordering::Relaxed))
            .field(
                "consecutive_failures",
                &self.consecutive_failures.load(Ordering::Relaxed),
            )
            .finish()
    }
}
