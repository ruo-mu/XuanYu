use anyhow::Context;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use xuanyu_core::AppCore;
use xuanyu_protocol::{AppEvent, SessionId};
use xuanyu_storage::{runtime_data_root, StorageConfig};

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
    SmokeTurn,
}

fn main() -> anyhow::Result<()> {
    let Cli { data_dir, command } = Cli::parse();
    let data_dir = data_dir.unwrap_or_else(runtime_data_root);
    tokio::runtime::Runtime::new()?.block_on(async move {
        let core = Arc::new(AppCore::new(StorageConfig::for_data_dir(data_dir)));
        dispatch(core, command).await
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
