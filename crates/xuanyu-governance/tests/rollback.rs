use serde_json::json;

use xuanyu_governance::RollbackRegistry;

#[tokio::test]
async fn register_and_find_rollback_action() {
    let registry = RollbackRegistry::default();
    let action = registry
        .register(
            "Undo applied change set",
            "changeset",
            json!({ "changeSetId": "cs-1" }),
        )
        .await;
    assert_eq!(registry.list().await.len(), 1);
    assert_eq!(
        registry.find(&action.id).await.expect("action").payload["changeSetId"],
        "cs-1"
    );
}
