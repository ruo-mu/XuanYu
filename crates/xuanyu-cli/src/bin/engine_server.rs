use std::io;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::Mutex;
use xuanyu_core::AppCore;
use xuanyu_desktop_host::invoke_command;
use xuanyu_protocol::AppEvent;
use xuanyu_storage::{runtime_data_root, StorageConfig};

#[derive(Debug, Deserialize)]
struct RpcRequest {
    id: String,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    id: String,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Debug, Serialize)]
struct RpcEvent {
    channel: &'static str,
    event: String,
    payload: Value,
}

struct EngineState {
    core: Arc<AppCore>,
    initialized: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_writer(io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let data_dir = runtime_data_root();
    let core = Arc::new(AppCore::new(StorageConfig::for_data_dir(data_dir)));
    // Resume incomplete marketplace catalogs immediately (MCP/skills can be tens of thousands).
    std::thread::Builder::new()
        .name("xy-market-warm".into())
        .spawn(|| {
            xuanyu_core::warm_remote_marketplace_cache();
        })
        .ok();
    let stdout = Arc::new(Mutex::new(tokio::io::stdout()));
    let state = Arc::new(Mutex::new(EngineState {
        core: core.clone(),
        initialized: false,
    }));

    spawn_event_bridge(core.clone());
    spawn_worker_health_loop(core);

    let stdin = tokio::io::stdin();
    let mut reader = tokio::io::BufReader::new(stdin).lines();
    while let Some(line) = reader.next_line().await? {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let request: RpcRequest = match serde_json::from_str(trimmed) {
            Ok(req) => req,
            Err(err) => {
                emit_response(
                    &stdout,
                    RpcResponse {
                        id: String::new(),
                        ok: false,
                        result: None,
                        error: Some(format!("invalid request json: {err}")),
                    },
                )
                .await?;
                continue;
            }
        };
        let response = handle_request(state.clone(), request).await;
        emit_response(&stdout, response).await?;
    }

    Ok(())
}

async fn handle_request(state: Arc<Mutex<EngineState>>, request: RpcRequest) -> RpcResponse {
    let method = request.method.as_str();
    let result = match method {
        "engine.init" => engine_init(state.clone()).await,
        "engine.status" => engine_status(state.clone()).await,
        "engine.shutdown" => {
            // 内置 PostgreSQL 保持常驻运行：关闭应用时不停库。下次启动引擎时
            // `start_postgres` 会先探测端口，已运行则直接复用，避免每次冷启动等待数秒。
            std::process::exit(0)
        }
        "core.invoke" => core_invoke(state.clone(), request.params).await,
        other => {
            if other.starts_with("core.") {
                core_invoke(
                    state.clone(),
                    serde_json::json!({
                        "command": other.strip_prefix("core.").unwrap_or(other),
                        "args": request.params,
                    }),
                )
                .await
            } else {
                Err(anyhow::anyhow!("unknown method: {other}"))
            }
        }
    };

    match result {
        Ok(value) => RpcResponse {
            id: request.id,
            ok: true,
            result: Some(value),
            error: None,
        },
        Err(err) => RpcResponse {
            id: request.id,
            ok: false,
            result: None,
            error: Some(err.to_string()),
        },
    }
}

async fn engine_init(state: Arc<Mutex<EngineState>>) -> anyhow::Result<Value> {
    let mut guard = state.lock().await;
    if !guard.initialized {
        let status = guard.core.initialize().await?;
        guard.initialized = true;
        return Ok(serde_json::to_value(status)?);
    }
    let status = guard.core.status().await;
    Ok(serde_json::to_value(status)?)
}

async fn engine_status(state: Arc<Mutex<EngineState>>) -> anyhow::Result<Value> {
    let guard = state.lock().await;
    let status = guard.core.status().await;
    Ok(serde_json::to_value(status)?)
}

async fn core_invoke(state: Arc<Mutex<EngineState>>, params: Value) -> anyhow::Result<Value> {
    ensure_initialized(&state).await?;
    let command = params
        .get("command")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("missing command"))?;
    let args = params.get("args").cloned().unwrap_or(Value::Null);
    let guard = state.lock().await;
    invoke_command(guard.core.clone(), command, args)
        .await
        .map_err(|err| anyhow::anyhow!(err))
}

async fn ensure_initialized(state: &Arc<Mutex<EngineState>>) -> anyhow::Result<()> {
    let guard = state.lock().await;
    if guard.initialized {
        return Ok(());
    }
    drop(guard);
    engine_init(state.clone()).await?;
    Ok(())
}

fn spawn_worker_health_loop(core: Arc<AppCore>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
        loop {
            interval.tick().await;
            let _ = core.refresh_workers_with_alerts().await;
        }
    });
}

fn spawn_event_bridge(core: Arc<AppCore>) {
    tokio::spawn(async move {
        let mut rx = core.eventbus().subscribe();
        loop {
            match rx.recv().await {
                Ok(envelope) => {
                    let event_name = match &envelope.payload {
                        AppEvent::TurnDelta { .. } => "xuanyu:turn-delta",
                        AppEvent::LogEntry { .. } => "xuanyu:log",
                        _ => "xuanyu:event",
                    };
                    let payload = serde_json::to_value(&envelope).unwrap_or(Value::Null);
                    let event = RpcEvent {
                        channel: "event",
                        event: event_name.to_string(),
                        payload,
                    };
                    if emit_event(event).await.is_err() {
                        break;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    });
}

async fn emit_response(stdout: &Arc<Mutex<tokio::io::Stdout>>, response: RpcResponse) -> io::Result<()> {
    let line = serde_json::to_string(&response).unwrap_or_else(|_| {
        r#"{"id":"","ok":false,"error":"response serialization failed"}"#.to_string()
    });
    let mut guard = stdout.lock().await;
    guard.write_all(line.as_bytes()).await?;
    guard.write_all(b"\n").await?;
    guard.flush().await?;
    Ok(())
}

async fn emit_event(event: RpcEvent) -> io::Result<()> {
    let line = serde_json::to_string(&event).unwrap_or_default();
    let mut stderr = tokio::io::stderr();
    stderr.write_all(line.as_bytes()).await?;
    stderr.write_all(b"\n").await?;
    stderr.flush().await?;
    Ok(())
}
