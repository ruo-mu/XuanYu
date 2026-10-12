use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenCounterBackend {
    Heuristic,
    Tiktoken,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenCountBreakdown {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub tool_schema_tokens: u64,
}

#[derive(Debug, Clone)]
pub struct TokenCounterConfig {
    pub max_context_tokens: u64,
    pub max_output_tokens: u64,
    pub backend: TokenCounterBackend,
}

impl Default for TokenCounterConfig {
    fn default() -> Self {
        Self {
            max_context_tokens: 128_000,
            max_output_tokens: 4_096,
            backend: TokenCounterBackend::Heuristic,
        }
    }
}

static RUNTIME_CONFIG: OnceLock<TokenCounterConfig> = OnceLock::new();
static TIKTOKEN_BPE: OnceLock<tiktoken_rs::CoreBPE> = OnceLock::new();

pub fn set_token_counter_runtime_config(config: TokenCounterConfig) {
    let _ = RUNTIME_CONFIG.set(config);
}

pub fn token_counter_runtime_config() -> TokenCounterConfig {
    RUNTIME_CONFIG.get().cloned().unwrap_or_default()
}

pub fn token_counter_backend_from_value(value: Option<&Value>) -> TokenCounterBackend {
    if matches!(
        std::env::var("XUANYU_TOKEN_COUNTER").ok().as_deref(),
        Some("tiktoken")
    ) {
        return TokenCounterBackend::Tiktoken;
    }
    let Some(raw) = value else {
        return TokenCounterBackend::Heuristic;
    };
    let text = raw.as_str().or_else(|| {
        raw.get("backend")
            .or_else(|| raw.get("mode"))
            .and_then(Value::as_str)
    });
    let Some(raw) = text else {
        return TokenCounterBackend::Heuristic;
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "tiktoken" | "js-tiktoken" | "cl100k" => TokenCounterBackend::Tiktoken,
        _ => TokenCounterBackend::Heuristic,
    }
}

fn cl100k_bpe() -> &'static tiktoken_rs::CoreBPE {
    TIKTOKEN_BPE.get_or_init(|| {
        tiktoken_rs::cl100k_base().expect("failed to initialize cl100k tokenizer")
    })
}

pub fn estimate_tokens(content: &str) -> u64 {
    if content.is_empty() {
        return 0;
    }
    match token_counter_runtime_config().backend {
        TokenCounterBackend::Tiktoken => estimate_tokens_tiktoken(content),
        TokenCounterBackend::Heuristic => estimate_tokens_heuristic(content),
    }
}

fn estimate_tokens_tiktoken(content: &str) -> u64 {
    cl100k_bpe()
        .encode_ordinary(content)
        .len()
        .max(1) as u64
}

fn estimate_tokens_heuristic(content: &str) -> u64 {
    let mut ascii = 0u64;
    let mut cjk = 0u64;
    for ch in content.chars() {
        if is_cjk_char(ch) {
            cjk += 1;
        } else {
            ascii += 1;
        }
    }
    let estimated = ascii.div_ceil(4) + ((cjk as f64 * 1.5).ceil() as u64);
    estimated.max(1)
}

pub fn estimate_tool_schema_tokens(tools: &[Value]) -> u64 {
    if tools.is_empty() {
        return 0;
    }
    let serialized = serde_json::to_string(tools).unwrap_or_default();
    estimate_tokens(&serialized).saturating_add(12 * tools.len() as u64)
}

pub fn count_message_roles(
    messages: &[(MessageRoleKind, String)],
) -> TokenCountBreakdown {
    let mut input_tokens = 0u64;
    let mut output_tokens = 0u64;
    for (role, content) in messages {
        let tokens = estimate_tokens(content);
        match role {
            MessageRoleKind::Assistant | MessageRoleKind::Tool => output_tokens += tokens,
            MessageRoleKind::User | MessageRoleKind::System => input_tokens += tokens,
        }
    }
    TokenCountBreakdown {
        input_tokens,
        output_tokens,
        total_tokens: input_tokens + output_tokens,
        tool_schema_tokens: 0,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageRoleKind {
    User,
    Assistant,
    System,
    Tool,
}

pub fn max_input_tokens(config: &TokenCounterConfig, tool_schema_tokens: u64) -> u64 {
    config
        .max_context_tokens
        .saturating_sub(config.max_output_tokens)
        .saturating_sub(tool_schema_tokens)
        .max(1)
}

fn is_cjk_char(ch: char) -> bool {
    matches!(ch,
        '\u{4E00}'..='\u{9FFF}'
            | '\u{3400}'..='\u{4DBF}'
            | '\u{F900}'..='\u{FAFF}'
    )
}
