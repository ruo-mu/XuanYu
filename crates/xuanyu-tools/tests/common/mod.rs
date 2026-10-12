use std::path::PathBuf;

use uuid::Uuid;
use xuanyu_governance::GovernanceEngine;

/// Governance policy that allows file writes without approval prompts.
pub fn allowing_governance() -> GovernanceEngine {
    let mut policy = xuanyu_governance::PermissionPolicy {
        allow_file_write: true,
        ..Default::default()
    };
    policy.approval_tools.clear();
    GovernanceEngine::new(policy)
}

/// Create a unique throwaway workspace directory for an integration test.
pub fn temp_workspace() -> PathBuf {
    let path = std::env::temp_dir().join(format!("xuanyu-tools-test-{}", Uuid::new_v4()));
    std::fs::create_dir_all(&path).expect("temp workspace");
    path
}
