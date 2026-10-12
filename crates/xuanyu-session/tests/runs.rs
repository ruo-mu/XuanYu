use std::collections::BTreeMap;

use serde_json::json;
use uuid::Uuid;
use xuanyu_session::{CreateRunInput, RunStore, StartStepInput};

#[tokio::test]
async fn run_store_persists_timeline_and_trace() {
    let root = std::env::temp_dir().join(format!("xuanyu-session-runs-{}", Uuid::new_v4()));
    let store = RunStore::new(root.clone(), "test-project");
    store.initialize().await.expect("init");
    let run = store
        .create_run(CreateRunInput {
            session_id: "session-1".to_string(),
            goal: "Test run store".to_string(),
            metadata: BTreeMap::from([("source".to_string(), json!("test"))]),
        })
        .await
        .expect("run");
    let step = store
        .start_step(StartStepInput {
            run_id: run.id.clone(),
            step_type: "model".to_string(),
            name: "Generate reply".to_string(),
            metadata: BTreeMap::new(),
        })
        .await
        .expect("step");
    store
        .record_event(
            &run.id,
            "trace.model.request",
            Some(json!({ "model": "test-model" })),
            Some(step.id.clone()),
        )
        .await
        .expect("event");
    store
        .complete_step(
            &step.id,
            BTreeMap::from([("model".to_string(), json!("test-model"))]),
        )
        .await
        .expect("complete step");
    store
        .complete_run(
            &run.id,
            BTreeMap::from([("model".to_string(), json!("test-model"))]),
        )
        .await
        .expect("complete run");

    let restored = RunStore::new(root.clone(), "test-project");
    restored.initialize().await.expect("restore");
    let runs = restored.list_runs(10).await;
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].status, "completed");
    let timeline = restored.timeline(&run.id).await.expect("timeline");
    assert_eq!(timeline.steps.len(), 1);
    assert!(timeline.events.len() >= 4);
    let trace = restored.trace_timeline(&run.trace_id).await.expect("trace");
    assert_eq!(trace.summary.model_requests, 1);
    let evidence = restored.evidence_pack(&run.id).await.expect("evidence");
    assert_eq!(evidence.resource_usage.model_requests, 1);
    assert!(restored
        .evidence_markdown(&run.id)
        .await
        .expect("markdown")
        .contains("Evidence Pack"));

    let _ = tokio::fs::remove_dir_all(root).await;
}
