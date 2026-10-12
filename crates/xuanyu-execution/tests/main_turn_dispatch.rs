use std::collections::BTreeSet;

use xuanyu_execution::{
    finalize_main_turn_tool_dispatch, patch_retrieved_memory_context, ContextBudgetAllocation,
    FinalizeMainTurnToolDispatchInput, MainTurnToolDispatchResult, TokenCountBreakdown,
    ToolLoopGuard,
};

fn empty_budget() -> ContextBudgetAllocation {
    ContextBudgetAllocation {
        sections: Vec::new(),
        max_input_tokens: 1000,
        tool_schema_tokens: 0,
        used_input_tokens: 100,
        remaining_input_tokens: 900,
        dropped_message_count: 0,
        truncated_message_count: 0,
        breakdown: TokenCountBreakdown {
            input_tokens: 100,
            output_tokens: 0,
            total_tokens: 100,
            tool_schema_tokens: 0,
        },
    }
}

#[test]
fn finalize_dispatch_merges_loaded_tools_and_patches() {
    let mut runtime = String::from("base");
    let mut transcript = Vec::new();
    let mut tools = BTreeSet::new();
    let mut guard = ToolLoopGuard::default();
    let dispatch = MainTurnToolDispatchResult {
        transcript_entries: vec!["ran search".to_string()],
        tool_results: Vec::new(),
        loaded_tool_names: vec!["file_read".to_string()],
        context_patches: vec!["[patch] ok".to_string()],
        loaded_context_sections: Vec::new(),
    };
    let stop = finalize_main_turn_tool_dispatch(FinalizeMainTurnToolDispatchInput {
        runtime_context: &mut runtime,
        tool_transcript: &mut transcript,
        active_tool_names: &mut tools,
        loop_guard: &mut guard,
        guard_inputs: &[],
        model_output: "done",
        context_budget_allocation: &empty_budget(),
        dispatch_result: dispatch,
    });
    assert!(stop.is_none());
    assert!(tools.contains("file_read"));
    assert!(runtime.contains("[patch] ok"));
    assert_eq!(transcript, vec!["ran search".to_string()]);
}

#[test]
fn patch_memory_replaces_existing_block() {
    let mut runtime = String::from("head\n\n[Retrieved memory]\nold\n\n[Tools]\nx");
    patch_retrieved_memory_context(&mut runtime, "fresh");
    assert!(runtime.contains("[Retrieved memory]\nfresh"));
    assert!(!runtime.contains("old"));
    assert!(runtime.contains("[Tools]\nx"));
}
