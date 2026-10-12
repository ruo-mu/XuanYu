use serde_json::json;
use xuanyu_execution::ContextSurface;

#[test]
fn reset_and_load_sections() {
    let mut surface = ContextSurface::new();
    surface.reset_for_turn(
        &json!({ "autoActivateSections": ["memory_profile"] }),
        "task",
    );
    assert!(surface.is_active("memory_profile"));
    let result = surface.load(&["project_instructions".to_string(), "bogus".to_string()]);
    assert_eq!(result.loaded, vec!["project_instructions"]);
    assert_eq!(result.rejected.len(), 1);
}
