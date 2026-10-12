use std::collections::BTreeMap;

use serde_json::json;
use uuid::Uuid;
use xuanyu_agents::{AgentRuntime, SubAgentTaskInput};

#[tokio::test]
async fn task_metadata_supplies_control_plane_links() {
    let root = std::env::temp_dir().join(format!("xuanyu-agents-{}", Uuid::new_v4()));
    let runtime = AgentRuntime::new(root.clone(), None);
    runtime.initialize().await.expect("init");

    let ledger = runtime
        .submit_task(
            SubAgentTaskInput {
                agent_id: "developer".to_string(),
                task: "Trace parent task links".to_string(),
                context: None,
                write_scope: Vec::new(),
                metadata: BTreeMap::from([
                    ("controlPlaneTaskId".to_string(), json!("parent-task-42")),
                    ("controlPlaneSessionId".to_string(), json!("session-meta")),
                    ("controlPlaneRunId".to_string(), json!("run-meta")),
                ]),
            },
            None,
            None,
        )
        .await
        .expect("task");

    assert_eq!(
        ledger.control_plane_task_id.as_deref(),
        Some("parent-task-42")
    );
    assert_eq!(
        ledger.control_plane_session_id.as_deref(),
        Some("session-meta")
    );
    assert_eq!(ledger.control_plane_run_id.as_deref(), Some("run-meta"));
    assert_eq!(
        runtime
            .list_tasks(Some("session-meta".to_string()), 10)
            .await
            .len(),
        1
    );

    let restored = AgentRuntime::new(root.clone(), None);
    restored.initialize().await.expect("restore");
    let restored_task = restored
        .task_status(ledger.task_id)
        .await
        .expect("restored task");
    assert_eq!(
        restored_task.control_plane_task_id.as_deref(),
        Some("parent-task-42")
    );
    assert_eq!(
        restored_task.timeline[0]
            .payload
            .get("controlPlaneSessionId"),
        Some(&json!("session-meta"))
    );

    let _ = tokio::fs::remove_dir_all(root).await;
}

#[tokio::test]
async fn task_plan_reports_replace_initial_plan_and_persist() {
    let root = std::env::temp_dir().join(format!("xuanyu-agents-{}", Uuid::new_v4()));
    let runtime = AgentRuntime::new(root.clone(), None);
    runtime.initialize().await.expect("init");
    let ledger = runtime
        .submit_task(
            SubAgentTaskInput {
                agent_id: "architect".to_string(),
                task: "Plan the worker bridge".to_string(),
                context: None,
                write_scope: Vec::new(),
                metadata: BTreeMap::new(),
            },
            Some("session-plan".to_string()),
            None,
        )
        .await
        .expect("task");

    runtime.start_task(&ledger.task_id).await.expect("start");
    let planned = runtime
        .report_task_plan(
            &ledger.task_id,
            Some("Worker bridge plan".to_string()),
            vec![
                "Inspect worker protocol".to_string(),
                "Inspect worker protocol".to_string(),
                "Define Rust bridge".to_string(),
                "Verify integration".to_string(),
            ],
        )
        .await
        .expect("plan");

    let plan = planned.plan.as_ref().expect("plan");
    assert_eq!(plan.title, "Worker bridge plan");
    assert_eq!(plan.steps.len(), 3);
    assert_eq!(plan.steps[0].status, "running");
    assert_eq!(
        planned.current_step_id.as_deref(),
        Some(plan.steps[0].id.as_str())
    );
    assert!(planned
        .timeline
        .iter()
        .any(|event| event.title.as_deref() == Some("Sub-agent task plan reported")));

    let restored = AgentRuntime::new(root.clone(), None);
    restored.initialize().await.expect("restore");
    let task = restored
        .task_status(ledger.task_id)
        .await
        .expect("restored task");
    assert_eq!(task.plan.as_ref().unwrap().title, "Worker bridge plan");

    let _ = tokio::fs::remove_dir_all(root).await;
}
