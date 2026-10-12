use std::collections::BTreeMap;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;

use crate::main_turn::{iteration_input, MainTurnLoopState};
use crate::ProviderToolCall;

/// Model response surface for the main turn loop.
#[derive(Debug, Clone, Default)]
pub struct MainTurnModelResponse {
    pub provider: String,
    pub model: String,
    pub text: String,
    pub reasoning: Option<String>,
    pub error: Option<String>,
    pub streamed: bool,
    pub tool_calls: Vec<ProviderToolCall>,
    pub usage: Option<crate::TokenUsage>,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MainTurnToolResultMessage {
    pub call_id: String,
    pub tool_name: String,
    pub arguments_json: String,
    pub content: String,
}

#[derive(Debug, Clone, Default)]
pub struct MainTurnToolDispatchResult {
    pub transcript_entries: Vec<String>,
    pub tool_results: Vec<MainTurnToolResultMessage>,
    pub loaded_tool_names: Vec<String>,
    pub context_patches: Vec<String>,
    pub loaded_context_sections: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MainTurnRuntimeToolCall {
    pub id: String,
    pub name: String,
    pub args: BTreeMap<String, Value>,
    pub source: String,
}

#[derive(Debug, Clone, Default)]
pub struct MainTurnLoopResult {
    pub final_text: String,
    pub final_provider: String,
    pub final_model: String,
    pub final_error: Option<String>,
    pub completed_iterations: usize,
    pub runtime_tool_count: usize,
    pub response_streamed: bool,
    pub cancelled: bool,
}

#[derive(Debug, Clone)]
pub struct MainTurnLoopConfig {
    pub max_iterations: usize,
    pub model_input: String,
    pub max_transcript_tokens: u64,
    /// Absolute unix-ms deadline for the whole turn; None = no wall-clock limit.
    pub deadline_ms: Option<u64>,
}

#[async_trait]
pub trait MainTurnLoopHost: Send + Sync {
    async fn on_iteration_start(
        &self,
        iteration: usize,
        iteration_input: &str,
        runtime_tool_count: usize,
    ) -> Result<()>;
    async fn generate_response(
        &self,
        iteration: usize,
        iteration_input: &str,
    ) -> Result<MainTurnModelResponse>;
    async fn on_model_response(&self, iteration: usize, response: &MainTurnModelResponse)
        -> Result<()>;
    async fn is_cancelled(&self) -> Result<bool>;
    async fn on_cancelled(&self) -> Result<()>;
    async fn extract_native_tool_calls(
        &self,
        response: &MainTurnModelResponse,
        iteration: usize,
    ) -> Vec<MainTurnRuntimeToolCall>;
    async fn extract_printed_tool_call(
        &self,
        response: &MainTurnModelResponse,
        iteration: usize,
    ) -> Option<MainTurnRuntimeToolCall>;
    fn looks_like_printed_tool_call(&self, text: &str) -> bool;
    async fn dispatch_tool_calls(
        &self,
        calls: Vec<MainTurnRuntimeToolCall>,
        iteration: usize,
        source: &str,
    ) -> Result<MainTurnToolDispatchResult>;
    async fn apply_tool_dispatch(
        &self,
        dispatch: MainTurnToolDispatchResult,
        guard_inputs: Vec<(String, Value)>,
        model_output: &str,
        loop_state: &mut MainTurnLoopState,
    ) -> Result<Option<String>>;
    /// Inject a mid-turn user note (truncation / loop correction) into the protocol history.
    async fn note_mid_turn_user_message(&self, content: &str) -> Result<()>;
}

pub async fn run_main_turn_loop<H: MainTurnLoopHost>(
    host: &H,
    config: MainTurnLoopConfig,
    state: &mut MainTurnLoopState,
) -> Result<MainTurnLoopResult> {
    let mut result = MainTurnLoopResult {
        final_provider: String::from("unknown"),
        final_model: String::from("unknown"),
        ..MainTurnLoopResult::default()
    };
    for iteration in 1..=config.max_iterations {
        result.completed_iterations = iteration;
        if let Some(deadline_ms) = config.deadline_ms {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            if now_ms >= deadline_ms {
                host.on_cancelled().await?;
                result.cancelled = true;
                result.final_error = Some("Run deadline exceeded".to_string());
                return Ok(result);
            }
        }
        let iteration_input = iteration_input(
            &config.model_input,
            &state.tool_transcript,
            Some(config.max_transcript_tokens),
        );
        host.on_iteration_start(iteration, &iteration_input, result.runtime_tool_count)
            .await?;
        let response = host.generate_response(iteration, &iteration_input).await?;
        if response.streamed {
            result.response_streamed = true;
        }
        result.final_provider = response.provider.clone();
        result.final_model = response.model.clone();
        host.on_model_response(iteration, &response).await?;
        if host.is_cancelled().await? {
            host.on_cancelled().await?;
            result.cancelled = true;
            return Ok(result);
        }
        if let Some(error) = response.error.clone() {
            result.final_text = response.text.clone();
            result.final_error = Some(error);
            break;
        }
        let native_tool_calls = host.extract_native_tool_calls(&response, iteration).await;
        if !native_tool_calls.is_empty() {
            result.runtime_tool_count += native_tool_calls.len();
            let guard_inputs = native_tool_calls
                .iter()
                .map(|call| {
                    (
                        call.name.clone(),
                        Value::Object(call.args.iter().map(|(k, v)| (k.clone(), v.clone())).collect()),
                    )
                })
                .collect::<Vec<_>>();
            let dispatch = host
                .dispatch_tool_calls(native_tool_calls, iteration, "native.tool_calls")
                .await?;
            if let Some(stop_text) = host
                .apply_tool_dispatch(
                    dispatch,
                    guard_inputs,
                    &response.text,
                    state,
                )
                .await?
            {
                result.final_text = stop_text;
                break;
            }
            continue;
        }
        if let Some(tool_call) = host.extract_printed_tool_call(&response, iteration).await {
            result.runtime_tool_count += 1;
            let guard_inputs = vec![(
                tool_call.name.clone(),
                Value::Object(
                    tool_call
                        .args
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                ),
            )];
            let dispatch = host
                .dispatch_tool_calls(vec![tool_call], iteration, "printed.tool_call")
                .await?;
            if let Some(stop_text) = host
                .apply_tool_dispatch(
                    dispatch,
                    guard_inputs,
                    &response.text,
                    state,
                )
                .await?
            {
                result.final_text = stop_text;
                break;
            }
            continue;
        }
        if host.looks_like_printed_tool_call(&response.text) && iteration < config.max_iterations {
            let note = "System correction: the previous assistant message printed invalid tool-call syntax. If a tool is needed, use a native tool call. Otherwise answer normally.";
            state.tool_transcript.push(note.to_string());
            host.note_mid_turn_user_message(note).await?;
            continue;
        }
        // Output hit max_tokens mid-turn — do not treat as a finished answer.
        let truncated = response
            .finish_reason
            .as_deref()
            .is_some_and(|reason| {
                let r = reason.to_ascii_lowercase();
                r == "length" || r == "max_tokens"
            });
        if truncated && iteration < config.max_iterations {
            let note = "System: previous model output was truncated (max_tokens/length). Continue the task with a native tool call or a complete final answer. Do not re-read a file you already have in this turn unless you need a different line range.";
            state.tool_transcript.push(note.to_string());
            host.note_mid_turn_user_message(note).await?;
            if !response.text.trim().is_empty() {
                state.final_text = response.text.clone();
            }
            continue;
        }
        result.final_text = response.text.clone();
        break;
    }
    if result.final_text.trim().is_empty() && result.final_error.is_none() {
        result.final_text = format!(
            "工具调用循环已达到上限（{} 轮）。已执行 {} 个主脑工具调用；请根据当前工具结果继续下一步。",
            config.max_iterations, result.runtime_tool_count
        );
    }
    Ok(result)
}
