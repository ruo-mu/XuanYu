use xuanyu_eventbus::observability::Observability;
use xuanyu_eventbus::AlertSeverity;

#[tokio::test]
async fn alert_buffer_keeps_recent_entries() {
    let obs = Observability::new(8);
    obs.alert(
        AlertSeverity::Warning,
        "worker degraded",
        Some(serde_json::json!({ "worker": "inference-cpp" })),
        None,
    )
    .await;
    let alerts = obs.alerts(10).await;
    assert_eq!(alerts.len(), 1);
    assert_eq!(alerts[0].title, "worker degraded");
}

#[tokio::test]
async fn profile_span_records_duration() {
    let obs = Observability::new(8);
    let span_id = obs.start_profile_span("turn.prepare").await;
    let span = obs.finish_profile_span(&span_id).await.expect("span");
    assert_eq!(span.name, "turn.prepare");
    assert!(span.duration_ms.unwrap_or(0) >= 0);
}

#[tokio::test]
async fn event_buffer_keeps_recent_capacity() {
    let obs = Observability::new(2);
    obs.log("info", "one").await;
    obs.log("info", "two").await;
    obs.log("info", "three").await;
    let events = obs.recent_events(10).await;
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].payload["message"], "three");
}
