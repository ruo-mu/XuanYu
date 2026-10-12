use std::collections::BTreeSet;

use serde_json::Value;

use crate::main_turn_loop::MainTurnToolDispatchResult;
use crate::tool_loop::{arg_shape, LoopIntervention, ToolLoopGuard, TurnFingerprint};
use crate::ContextBudgetAllocation;

/// Applies loaded tools / context patches / transcript entries, then records loop-guard state.
/// Memory retrieval remains the host's responsibility before calling this helper.
pub struct FinalizeMainTurnToolDispatchInput<'a> {
    pub runtime_context: &'a mut String,
    pub tool_transcript: &'a mut Vec<String>,
    pub active_tool_names: &'a mut BTreeSet<String>,
    pub loop_guard: &'a mut ToolLoopGuard,
    pub guard_inputs: &'a [(&'a str, &'a Value)],
    pub model_output: &'a str,
    pub context_budget_allocation: &'a ContextBudgetAllocation,
    pub dispatch_result: MainTurnToolDispatchResult,
}

pub fn finalize_main_turn_tool_dispatch(
    input: FinalizeMainTurnToolDispatchInput<'_>,
) -> Option<String> {
    let FinalizeMainTurnToolDispatchInput {
        runtime_context,
        tool_transcript,
        active_tool_names,
        loop_guard,
        guard_inputs,
        model_output,
        context_budget_allocation,
        dispatch_result,
    } = input;
    for name in dispatch_result.loaded_tool_names {
        active_tool_names.insert(name);
    }
    for patch in dispatch_result.context_patches {
        runtime_context.push_str("\n\n");
        runtime_context.push_str(&patch);
    }
    tool_transcript.extend(dispatch_result.transcript_entries);

    let context_pressure = context_budget_allocation.used_input_tokens as f64
        / context_budget_allocation.max_input_tokens.max(1) as f64;
    let intervention = loop_guard.record_tool_iteration(guard_inputs, model_output, context_pressure);
    if intervention == LoopIntervention::ForceStop {
        return Some(if model_output.trim().is_empty() {
            "任务已因重复操作自动停止。已完成的工作已保留在上下文中。".to_string()
        } else {
            model_output.trim().to_string()
        });
    }
    if let Some(message) = ToolLoopGuard::intervention_message(
        intervention,
        &TurnFingerprint {
            tool_names: guard_inputs
                .iter()
                .map(|(name, _)| (*name).to_string())
                .collect(),
            output_prefix: model_output.chars().take(200).collect::<String>(),
            arg_shapes: guard_inputs
                .iter()
                .map(|(_, args)| arg_shape(args))
                .collect(),
        },
        context_pressure,
    ) {
        tool_transcript.push(message);
    }
    None
}

/// Patch or append the retrieved-memory block inside the runtime context string.
pub fn patch_retrieved_memory_context(runtime_context: &mut String, memory: &str) {
    if runtime_context.contains("[Retrieved memory]") {
        if let Some(start) = runtime_context.find("[Retrieved memory]") {
            if let Some(end) = runtime_context[start..].find("\n\n[") {
                runtime_context.replace_range(
                    start..start + end,
                    &format!("[Retrieved memory]\n{memory}"),
                );
            } else {
                runtime_context.replace_range(start.., &format!("[Retrieved memory]\n{memory}"));
            }
        }
    } else {
        runtime_context.push_str("\n\n");
        runtime_context.push_str(&format!("[Retrieved memory]\n{memory}"));
    }
}
