use xuanyu_session::SessionParticipantRegistry;

#[tokio::test]
async fn join_and_list_participants() {
    let registry = SessionParticipantRegistry::default();
    registry.ensure_default("s1").await;
    registry
        .join("s1", "agent", Some("Bot".to_string()), Some("ext-1".to_string()))
        .await;
    let list = registry.list("s1").await;
    assert!(list.len() >= 2);
    assert!(registry.touch("s1", "ext-1").await);
}
