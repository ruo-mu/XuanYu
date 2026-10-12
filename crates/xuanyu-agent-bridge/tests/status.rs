use uuid::Uuid;
use xuanyu_agent_bridge::{AgentWorkerBridge, AgentWorkerState};

#[test]
fn status_reports_missing_worker_script_without_spawn() {
    let missing = std::env::temp_dir().join(format!(
        "xuanyu-missing-agent-worker-{}.py",
        Uuid::new_v4().simple()
    ));
    let status = AgentWorkerBridge::with_script("missing-python", missing).status();
    assert_eq!(status.state, AgentWorkerState::Unavailable);
    assert!(!status.configured);
    assert!(status.detail.contains("not found"));
}
