use serde_json::Value;

use crate::agent::Agent;
use crate::context_assembler::{
    prepare_turn, LazyContextOverride, PrepareTurnInput, PrepareTurnPlan,
};
use crate::estimate_tokens;
use crate::tool_loop::ToolLoopGuard;

/// Bundles the main-turn prepare phase — extracted from `complete_main_turn_after_user_message`.
pub struct MainTurnBootstrap {
    pub turn_plan: PrepareTurnPlan,
    pub agent: Agent,
    pub turn_intent: String,
}

pub struct MainTurnLoopState {
    pub tool_transcript: Vec<String>,
    pub final_text: String,
    pub final_provider: String,
    pub final_model: String,
    pub final_error: Option<String>,
    pub completed_iterations: usize,
    pub runtime_tool_count: usize,
    pub response_streamed: bool,
    pub loop_guard: ToolLoopGuard,
}

impl Default for MainTurnLoopState {
    fn default() -> Self {
        Self {
            tool_transcript: Vec::new(),
            final_text: String::new(),
            final_provider: String::from("unknown"),
            final_model: String::from("unknown"),
            final_error: None,
            completed_iterations: 0,
            runtime_tool_count: 0,
            response_streamed: false,
            loop_guard: ToolLoopGuard::new(),
        }
    }
}

pub fn bootstrap_main_turn(
    config: &Value,
    user_message: &str,
    turn_intent: &str,
    lazy_override: Option<LazyContextOverride>,
) -> MainTurnBootstrap {
    let turn_plan = prepare_turn(PrepareTurnInput {
        config: config.clone(),
        intent: turn_intent.to_string(),
        lazy_override,
    });
    let mut agent = Agent::new();
    agent.prepare_turn_context(
        &turn_plan,
        &turn_plan.context_policy,
        user_message,
        turn_intent,
        false,
    );
    MainTurnBootstrap {
        turn_plan,
        agent,
        turn_intent: turn_intent.to_string(),
    }
}

pub fn max_iterations_from_config(config: &Value) -> usize {
    // 与参考主循环一致：默认不设实用硬上限；仅在配置显式给出时采用，并保留极高安全顶。
    const SOFT_SAFETY_CEILING: u64 = 10_000;
    config
        .get("maxIterations")
        .or_else(|| config.get("max_iterations"))
        .or_else(|| {
            config
                .get("agent")
                .and_then(|agent| agent.get("maxIterations"))
        })
        .or_else(|| {
            config
                .get("runtime")
                .and_then(|runtime| runtime.get("maxIterations"))
        })
        .or_else(|| {
            config
                .get("mainBrain")
                .and_then(|main| main.get("maxIterations"))
        })
        .and_then(Value::as_u64)
        .map(|value| value.clamp(1, SOFT_SAFETY_CEILING) as usize)
        .unwrap_or(SOFT_SAFETY_CEILING as usize)
}

pub fn iteration_input(
    user_input: &str,
    transcript: &[String],
    max_transcript_tokens: Option<u64>,
) -> String {
    if transcript.is_empty() {
        return user_input.to_string();
    }
    let mut selected = Vec::<String>::new();
    let mut used_tokens = 0u64;
    for entry in transcript.iter().rev() {
        let entry_tokens = estimate_tokens(entry);
        if let Some(max_tokens) = max_transcript_tokens {
            if !selected.is_empty() && used_tokens.saturating_add(entry_tokens) > max_tokens {
                break;
            }
        } else if selected.len() >= 256 {
            // 无 token 预算时也不再只留 8 条（长任务会丢早期工具证据）。
            break;
        }
        selected.push(entry.clone());
        used_tokens = used_tokens.saturating_add(entry_tokens);
    }
    selected.reverse();
    if selected.is_empty() {
        selected.push(transcript.last().cloned().unwrap_or_default());
    }
    let visible_transcript = selected.join("\n\n");
    format!(
        "User request:\n{user_input}\n\nTool results so far:\n{visible_transcript}\n\nContinue from these results. Reply in the user's language. If a write/edit just succeeded, do not re-read that same path only to verify."
    )
}
