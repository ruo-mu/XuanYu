//! 投机工具执行：模型流式过程中参数封口即入队执行，dispatch 时按入队序收获结果。

use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;
use xuanyu_execution::ProviderToolCall;
use xuanyu_tools::{ToolCall, ToolDefinition, ToolOutput, ToolRuntime};

const DEFAULT_MAX_CONCURRENCY: usize = 10;

#[derive(Clone)]
pub struct SpeculativeToolOutput {
    pub content: String,
    pub is_error: bool,
    pub data: Option<Value>,
}

impl From<ToolOutput> for SpeculativeToolOutput {
    fn from(value: ToolOutput) -> Self {
        Self {
            content: value.content,
            is_error: value.is_error,
            data: value.data,
        }
    }
}

impl From<SpeculativeToolOutput> for ToolOutput {
    fn from(value: SpeculativeToolOutput) -> Self {
        ToolOutput {
            content: value.content,
            is_error: value.is_error,
            data: value.data,
        }
    }
}

enum EntryStatus {
    Queued,
    Executing,
    Completed,
}

struct TrackedTool {
    call: ProviderToolCall,
    concurrent_safe: bool,
    sibling_abort_on_error: bool,
    status: EntryStatus,
    result: Option<SpeculativeToolOutput>,
    notify: Arc<Notify>,
}

struct BatchInner {
    tools: Vec<TrackedTool>,
    discarded: bool,
    aborted: bool,
    abort_reason: Option<String>,
}

/// 主回合一轮模型流共用的投机工具批次。
pub struct SpeculativeToolBatch {
    tools_runtime: Arc<ToolRuntime>,
    cwd: PathBuf,
    session_id: String,
    definitions: Vec<ToolDefinition>,
    max_concurrency: usize,
    inner: Arc<Mutex<BatchInner>>,
    wake: Arc<Notify>,
}

impl SpeculativeToolBatch {
    pub fn new(
        tools_runtime: Arc<ToolRuntime>,
        cwd: PathBuf,
        session_id: String,
        definitions: Vec<ToolDefinition>,
    ) -> Arc<Self> {
        Arc::new(Self {
            tools_runtime,
            cwd,
            session_id,
            definitions,
            max_concurrency: DEFAULT_MAX_CONCURRENCY,
            inner: Arc::new(Mutex::new(BatchInner {
                tools: Vec::new(),
                discarded: false,
                aborted: false,
                abort_reason: None,
            })),
            wake: Arc::new(Notify::new()),
        })
    }

    pub fn enqueue(self: &Arc<Self>, call: ProviderToolCall) {
        if call.name == "ask_user" {
            return;
        }
        {
            let mut inner = self.inner.lock().expect("speculative tools lock");
            if inner.discarded || inner.aborted {
                return;
            }
            if inner.tools.iter().any(|tool| tool.call.id == call.id) {
                return;
            }
            let concurrent_safe = is_concurrency_safe(&self.definitions, &call.name);
            let sibling_abort_on_error = is_sibling_abort_on_error(&call.name);
            inner.tools.push(TrackedTool {
                call,
                concurrent_safe,
                sibling_abort_on_error,
                status: EntryStatus::Queued,
                result: None,
                notify: Arc::new(Notify::new()),
            });
        }
        self.spawn_process_queue();
    }

    pub fn discard(&self, reason: &str) {
        let mut inner = self.inner.lock().expect("speculative tools lock");
        inner.discarded = true;
        inner.aborted = true;
        inner.abort_reason = Some(reason.to_string());
        for tool in &mut inner.tools {
            if matches!(tool.status, EntryStatus::Queued) {
                tool.result = Some(SpeculativeToolOutput {
                    content: reason.to_string(),
                    is_error: true,
                    data: Some(serde_json::json!({ "errorType": "speculative_discarded" })),
                });
                tool.status = EntryStatus::Completed;
                tool.notify.notify_waiters();
            }
        }
        self.wake.notify_waiters();
    }

    /// 按 tool_call id 取投机结果；未投机则返回 None（走普通 dispatch）。
    pub async fn take(&self, tool_call_id: &str) -> Option<ToolOutput> {
        let notify = {
            let inner = self.inner.lock().expect("speculative tools lock");
            let tool = inner.tools.iter().find(|tool| tool.call.id == tool_call_id)?;
            if let Some(result) = &tool.result {
                return Some(result.clone().into());
            }
            tool.notify.clone()
        };
        loop {
            {
                let inner = self.inner.lock().expect("speculative tools lock");
                if let Some(tool) = inner.tools.iter().find(|tool| tool.call.id == tool_call_id) {
                    if let Some(result) = &tool.result {
                        return Some(result.clone().into());
                    }
                    if matches!(tool.status, EntryStatus::Completed) {
                        return Some(ToolOutput {
                            content: "Tool completed without result".to_string(),
                            is_error: true,
                            data: None,
                        });
                    }
                } else {
                    return None;
                }
            }
            notify.notified().await;
        }
    }

    fn spawn_process_queue(self: &Arc<Self>) {
        let batch = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let started = batch.start_ready_tools();
                if started == 0 {
                    break;
                }
                batch.wake.notified().await;
            }
        });
    }

    fn start_ready_tools(self: &Arc<Self>) -> usize {
        let mut started = 0usize;
        loop {
            let next = {
                let mut inner = self.inner.lock().expect("speculative tools lock");
                if inner.discarded || inner.aborted {
                    return started;
                }
                let executing: Vec<(bool,)> = inner
                    .tools
                    .iter()
                    .filter(|tool| matches!(tool.status, EntryStatus::Executing))
                    .map(|tool| (tool.concurrent_safe,))
                    .collect();
                let mut chosen = None;
                for (index, tool) in inner.tools.iter().enumerate() {
                    if !matches!(tool.status, EntryStatus::Queued) {
                        continue;
                    }
                    if !can_start(tool.concurrent_safe, &executing, self.max_concurrency) {
                        if !tool.concurrent_safe {
                            break;
                        }
                        continue;
                    }
                    chosen = Some(index);
                    break;
                }
                let Some(index) = chosen else {
                    return started;
                };
                inner.tools[index].status = EntryStatus::Executing;
                let call = inner.tools[index].call.clone();
                let concurrent_safe = inner.tools[index].concurrent_safe;
                let sibling_abort = inner.tools[index].sibling_abort_on_error;
                let notify = inner.tools[index].notify.clone();
                (
                    index,
                    call,
                    concurrent_safe,
                    sibling_abort,
                    notify,
                )
            };
            let (index, call, _concurrent_safe, sibling_abort, notify) = next;
            let batch = Arc::clone(self);
            tokio::spawn(async move {
                let output = batch.invoke_tool(call).await;
                let should_abort_siblings = output.is_error && sibling_abort;
                {
                    let mut inner = batch.inner.lock().expect("speculative tools lock");
                    if let Some(tool) = inner.tools.get_mut(index) {
                        tool.result = Some(output.into());
                        tool.status = EntryStatus::Completed;
                        tool.notify.notify_waiters();
                    }
                    if should_abort_siblings && !inner.aborted {
                        let reason = "Cancelled: parallel tool call errored (sibling abort)".to_string();
                        inner.aborted = true;
                        inner.abort_reason = Some(reason.clone());
                        for (other_index, tool) in inner.tools.iter_mut().enumerate() {
                            if other_index == index {
                                continue;
                            }
                            if matches!(tool.status, EntryStatus::Queued) {
                                tool.result = Some(SpeculativeToolOutput {
                                    content: reason.clone(),
                                    is_error: true,
                                    data: Some(serde_json::json!({
                                        "errorType": "speculative_sibling_abort"
                                    })),
                                });
                                tool.status = EntryStatus::Completed;
                                tool.notify.notify_waiters();
                            }
                        }
                    }
                }
                notify.notify_waiters();
                batch.wake.notify_waiters();
                batch.spawn_process_queue();
            });
            started += 1;
        }
    }

    async fn invoke_tool(&self, call: ProviderToolCall) -> ToolOutput {
        let args = parse_tool_arguments(&call.arguments);
        let mut map = args;
        map.entry("_sessionId".to_string())
            .or_insert_with(|| serde_json::json!(self.session_id));
        let tool_call = ToolCall {
            name: call.name,
            args: map,
            cwd: self.cwd.clone(),
        };
        self.tools_runtime.execute(tool_call).await
    }
}

fn can_start(concurrent_safe: bool, executing: &[(bool,)], max_concurrency: usize) -> bool {
    if executing.is_empty() {
        return true;
    }
    if executing.len() >= max_concurrency {
        return false;
    }
    if !concurrent_safe {
        return false;
    }
    executing.iter().all(|(safe,)| *safe)
}

fn is_concurrency_safe(definitions: &[ToolDefinition], name: &str) -> bool {
    definitions
        .iter()
        .find(|tool| tool.name == name || tool.aliases.iter().any(|alias| alias == name))
        .map(|tool| tool.is_concurrent && !tool.is_execute)
        .unwrap_or(false)
}

fn is_sibling_abort_on_error(name: &str) -> bool {
    matches!(
        name,
        "exec" | "start_process" | "validation_replay" | "bash" | "shell"
    )
}

fn parse_tool_arguments(raw: &str) -> BTreeMap<String, Value> {
    match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(map)) => map.into_iter().collect(),
        Ok(other) => BTreeMap::from([("value".to_string(), other)]),
        Err(_) => BTreeMap::from([("raw".to_string(), Value::String(raw.to_string()))]),
    }
}
