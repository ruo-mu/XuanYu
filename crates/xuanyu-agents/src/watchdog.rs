use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::{SubAgentTaskLedger, SubAgentTimelineEvent};

#[derive(Debug, Clone)]
pub struct WatchdogToolCall {
    pub name: String,
    pub args: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy)]
pub struct WatchdogToolOutput {
    pub is_error: bool,
}
const DEFAULT_POLL_MS: u64 = 30_000;
const STALE_MS: i64 = 120_000;
const TIMEOUT_MS: i64 = 600_000;
const MAX_REPEATED_TOOLS: u32 = 4;
const MAX_CONSECUTIVE_ERRORS: u32 = 3;

#[derive(Debug, Clone)]
pub struct WatchdogSettings {
    pub max_retries: u32,
    pub auto_recover: bool,
}

impl Default for WatchdogSettings {
    fn default() -> Self {
        Self {
            max_retries: 1,
            auto_recover: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchdogRecoveryAction {
    Retry,
    Block,
}

#[derive(Debug, Clone)]
pub struct WatchdogRecoveryPlan {
    pub task_id: String,
    pub new_task_id: Option<String>,
    pub action: WatchdogRecoveryAction,
    pub reason: String,
}

#[derive(Debug, Clone)]
pub struct WatchdogIssue {
    pub kind: String,
    pub message: String,
}

pub fn build_recovery_hint(issue: &WatchdogIssue) -> String {
    match issue.kind.as_str() {
        "repeated_tool" => {
            "Watchdog detected repeated identical tool calls. Change approach instead of retrying the same tool.".to_string()
        }
        "consecutive_errors" => {
            "Watchdog detected consecutive tool errors. Verify arguments and environment before continuing.".to_string()
        }
        "stale" => {
            "Watchdog detected no progress for too long. Summarize current state and continue from the last verified milestone.".to_string()
        }
        "timeout" => {
            "Watchdog detected task runtime exceeded the limit. Narrow scope and finish the highest-value remaining work.".to_string()
        }
        _ => issue.message.clone(),
    }
}

/// Port of `capabilities/subagents/watchdog.ts` `SubAgentWatchdog`.
pub struct SubAgentWatchdog {
    poll_interval: Duration,
    last_poll: Option<Instant>,
}

impl SubAgentWatchdog {
    pub fn new() -> Self {
        Self {
            poll_interval: Duration::from_millis(DEFAULT_POLL_MS),
            last_poll: None,
        }
    }

    pub fn should_poll(&mut self) -> bool {
        let now = Instant::now();
        if self
            .last_poll
            .map(|last| now.duration_since(last) < self.poll_interval)
            .unwrap_or(false)
        {
            return false;
        }
        self.last_poll = Some(now);
        true
    }

    pub fn track_repeated_tool(ledger: &mut SubAgentTaskLedger, call: &WatchdogToolCall) {
        let fingerprint = tool_fingerprint(call);
        if ledger.watchdog.last_tool_fingerprint.as_deref() == Some(fingerprint.as_str()) {
            ledger.watchdog.repeated_tool_count = ledger.watchdog.repeated_tool_count.saturating_add(1);
        } else {
            ledger.watchdog.last_tool_fingerprint = Some(fingerprint);
            ledger.watchdog.repeated_tool_count = 1;
        }
    }

    pub fn record_tool_error(ledger: &mut SubAgentTaskLedger, output: &WatchdogToolOutput) {
        if output.is_error {
            ledger.watchdog.consecutive_errors =
                ledger.watchdog.consecutive_errors.saturating_add(1);
        } else {
            ledger.watchdog.consecutive_errors = 0;
        }
    }

    pub fn check(ledger: &SubAgentTaskLedger, now_ms: i64) -> Vec<WatchdogIssue> {
        let mut issues = Vec::new();
        if matches!(ledger.status.as_str(), "completed" | "cancelled" | "failed") {
            return issues;
        }
        let started = ledger.started_at;
        if now_ms - started > TIMEOUT_MS {
            issues.push(WatchdogIssue {
                kind: "timeout".to_string(),
                message: "Sub-agent task exceeded watchdog timeout.".to_string(),
            });
        }
        let last_event = ledger.watchdog.last_event_at;
        if now_ms - last_event > STALE_MS {
            issues.push(WatchdogIssue {
                kind: "stale".to_string(),
                message: "Sub-agent task has been silent for too long.".to_string(),
            });
        }
        if ledger.watchdog.repeated_tool_count >= MAX_REPEATED_TOOLS {
            issues.push(WatchdogIssue {
                kind: "repeated_tool".to_string(),
                message: "Sub-agent repeated the same tool call too many times.".to_string(),
            });
        }
        if ledger.watchdog.consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
            issues.push(WatchdogIssue {
                kind: "consecutive_errors".to_string(),
                message: "Sub-agent hit consecutive tool errors.".to_string(),
            });
        }
        issues
    }
}

fn tool_fingerprint(call: &WatchdogToolCall) -> String {
    format!("{}:{}", call.name, serde_json::to_string(&call.args).unwrap_or_default())
}

/// Timeline text buffer per task — `capabilities/subagents/timeline.ts`.
#[derive(Debug, Default)]
pub struct SubAgentTimelineBuffer {
    buffers: BTreeMap<String, TimelineScratch>,
}

#[derive(Debug, Default, Clone)]
struct TimelineScratch {
    reasoning: String,
    text: String,
}

impl SubAgentTimelineBuffer {
    pub fn append_reasoning(&mut self, task_id: &str, chunk: &str) {
        let entry = self.buffers.entry(task_id.to_string()).or_default();
        entry.reasoning.push_str(chunk);
    }

    pub fn append_text(&mut self, task_id: &str, chunk: &str) {
        let entry = self.buffers.entry(task_id.to_string()).or_default();
        entry.text.push_str(chunk);
    }

    pub fn flush(
        &mut self,
        task_id: &str,
        event_type: &str,
    ) -> Option<SubAgentTimelineEvent> {
        let scratch = self.buffers.remove(task_id)?;
        let content = match event_type {
            "reasoning" if !scratch.reasoning.trim().is_empty() => scratch.reasoning,
            "text" if !scratch.text.trim().is_empty() => scratch.text,
            _ => return None,
        };
        let now = chrono::Utc::now().timestamp_millis();
        Some(SubAgentTimelineEvent {
            id: format!("subevt_{now}_{}", uuid::Uuid::new_v4()),
            task_id: task_id.to_string(),
            agent_id: String::new(),
            event_type: event_type.to_string(),
            timestamp: now,
            title: Some(truncate_timeline_text(&content)),
            status: None,
            payload: BTreeMap::from([("content".to_string(), Value::String(content))]),
        })
    }
}

fn truncate_timeline_text(text: &str) -> String {
    const MAX: usize = 1200;
    if text.chars().count() <= MAX {
        return text.to_string();
    }
    let mut out = text.chars().take(MAX).collect::<String>();
    out.push_str("...");
    out
}

impl Default for SubAgentWatchdog {
    fn default() -> Self {
        Self::new()
    }
}
