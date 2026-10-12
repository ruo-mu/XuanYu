use xuanyu_eventbus::{EventBus, EventFilter, EventPublishContext};
use xuanyu_protocol::{AppEvent, AppStatus, SessionId, TurnId, WorkerHealth};

#[tokio::test]
async fn publishes_events_to_subscribers() {
    let bus = EventBus::new(16);
    let mut rx = bus.subscribe();
    bus.publish(AppEvent::AppReady {
        status: AppStatus {
            protocol_version: "test".to_string(),
            initialized: true,
            storage_backend: "memory".to_string(),
            active_sessions: 0,
            modules: Vec::new(),
            workers: Vec::<WorkerHealth>::new(),
        },
    });
    let event = rx.recv().await.expect("event");
    assert_eq!(event.kind, "xuanyu.event");
    assert_eq!(event.meta.get("sequence").map(String::as_str), Some("1"));
}

#[tokio::test]
async fn replay_since_returns_events_after_sequence() {
    let bus = EventBus::new(16);
    bus.publish(AppEvent::LogEntry {
        level: "info".to_string(),
        message: "first".to_string(),
    });
    bus.publish(AppEvent::LogEntry {
        level: "info".to_string(),
        message: "second".to_string(),
    });
    let batch = bus.replay_since(1).await;
    assert_eq!(batch.events.len(), 1);
    assert_eq!(batch.next_sequence, 2);
}

#[tokio::test]
async fn filtered_replay_honors_session_scope() {
    let bus = EventBus::new(16);
    bus.publish_scoped(
        EventPublishContext {
            session_id: Some("session-a".to_string()),
            turn_id: None,
            trace_id: None,
        },
        AppEvent::TurnStarted {
            session_id: SessionId("session-a".to_string()),
            turn_id: TurnId("turn-1".to_string()),
        },
    );
    bus.publish_scoped(
        EventPublishContext {
            session_id: Some("session-b".to_string()),
            turn_id: None,
            trace_id: None,
        },
        AppEvent::TurnStarted {
            session_id: SessionId("session-b".to_string()),
            turn_id: TurnId("turn-2".to_string()),
        },
    );
    let batch = bus
        .replay_since_filtered(
            0,
            EventFilter {
                session_id: Some("session-a".to_string()),
                ..EventFilter::default()
            },
        )
        .await;
    assert_eq!(batch.events.len(), 1);
    assert_eq!(batch.events[0].session_id.as_deref(), Some("session-a"));
}
