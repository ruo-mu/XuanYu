//! 远程 MCP（Streamable HTTP）：`{ type: "http"|"streamable-http", url }`。

use crate::{ToolDefinition, ToolOutput};
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};
use url::Url;

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const MCP_CLIENT_NAME: &str = "xuanyu-agent";
const MCP_CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone)]
pub struct McpRemoteServerConfig {
    pub url: String,
    pub headers: BTreeMap<String, String>,
    pub timeout_ms: Option<u64>,
    pub transport_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct McpRemoteToolInfo {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Option<Value>,
}

pub struct McpRemoteClient {
    server_name: String,
    config: McpRemoteServerConfig,
    next_id: AtomicU64,
    session_id: std::sync::Mutex<Option<String>>,
    started: std::sync::Mutex<bool>,
    http: reqwest::blocking::Client,
}

impl McpRemoteClient {
    pub fn new(server_name: String, config: McpRemoteServerConfig) -> anyhow::Result<Self> {
        assert_allowed_remote_mcp_url(&config.url)?;
        assert_remote_mcp_transport(config.transport_type.as_deref())?;
        let http = reqwest::blocking::Client::builder()
            .timeout(Duration::from_millis(
                config.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
            ))
            .build()?;
        Ok(Self {
            server_name,
            config,
            next_id: AtomicU64::new(1),
            session_id: std::sync::Mutex::new(None),
            started: std::sync::Mutex::new(false),
            http,
        })
    }

    pub fn start(&self) -> anyhow::Result<()> {
        if *self.started.lock().expect("mcp remote started lock") {
            return Ok(());
        }
        assert_allowed_remote_mcp_url(&self.config.url)?;
        assert_remote_mcp_transport(self.config.transport_type.as_deref())?;
        let _ = self.request(
            "initialize",
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": MCP_CLIENT_NAME, "version": MCP_CLIENT_VERSION }
            }),
            None,
        )?;
        self.notify("notifications/initialized", json!({}))?;
        *self.started.lock().expect("mcp remote started lock") = true;
        Ok(())
    }

    pub fn start_with_backoff(&self, max_attempts: u32) -> anyhow::Result<()> {
        let mut last_error = None;
        for attempt in 1..=max_attempts.max(1) {
            match self.start() {
                Ok(()) => return Ok(()),
                Err(err) => {
                    self.close();
                    last_error = Some(err);
                    if attempt < max_attempts {
                        std::thread::sleep(Duration::from_millis(250 * 2u64.pow(attempt - 1)));
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("MCP remote start failed")))
    }

    pub fn list_tools(&self) -> anyhow::Result<Vec<McpRemoteToolInfo>> {
        if !*self.started.lock().expect("mcp remote started lock") {
            self.start()?;
        }
        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;
        for _ in 0..100 {
            let params = match &cursor {
                Some(value) => json!({ "cursor": value }),
                None => json!({}),
            };
            let result = self.request("tools/list", params, None)?;
            tools.extend(parse_tool_list_result(&result));
            cursor = result
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string)
                .filter(|value| !value.is_empty());
            if cursor.is_none() {
                tools.truncate(10_000);
                return Ok(tools);
            }
        }
        anyhow::bail!(
            "MCP server {} exceeded tools/list pagination limit",
            self.server_name
        );
    }

    pub fn call_tool(
        &self,
        name: &str,
        args: Map<String, Value>,
        timeout_ms: Option<u64>,
    ) -> ToolOutput {
        if let Err(err) = (|| -> anyhow::Result<()> {
            if !*self.started.lock().expect("mcp remote started lock") {
                self.start()?;
            }
            Ok(())
        })() {
            return ToolOutput {
                content: format!("MCP remote start failed: {err}"),
                is_error: true,
                data: Some(json!({
                    "errorType": "mcp_remote_start_failed",
                    "mcpServer": self.server_name,
                    "mcpTool": name
                })),
            };
        }
        match self.request(
            "tools/call",
            json!({ "name": name, "arguments": Value::Object(args) }),
            timeout_ms,
        ) {
            Ok(result) => {
                let is_error = result.get("isError").and_then(Value::as_bool).unwrap_or(false);
                let content = format_mcp_content(&result);
                ToolOutput {
                    content: if content.is_empty() {
                        "(empty MCP tool result)".to_string()
                    } else {
                        content
                    },
                    is_error,
                    data: Some(json!({
                        "mcpServer": self.server_name,
                        "mcpTool": name,
                        "transport": "streamable-http",
                        "result": result
                    })),
                }
            }
            Err(err) => ToolOutput {
                content: format!("MCP tool {}/{} failed: {err}", self.server_name, name),
                is_error: true,
                data: Some(json!({
                    "errorType": "mcp_remote_call_failed",
                    "mcpServer": self.server_name,
                    "mcpTool": name
                })),
            },
        }
    }

    pub fn close(&self) {
        *self.started.lock().expect("mcp remote started lock") = false;
        *self.session_id.lock().expect("mcp remote session lock") = None;
    }

    fn request(
        &self,
        method: &str,
        params: Value,
        timeout_ms: Option<u64>,
    ) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        let response = self.post_to_url(&payload, timeout_ms)?;
        if let Some(session) = response
            .headers()
            .get("mcp-session-id")
            .or_else(|| response.headers().get("Mcp-Session-Id"))
            .and_then(|value| value.to_str().ok())
        {
            *self.session_id.lock().expect("mcp remote session lock") = Some(session.to_string());
        }
        let status = response.status();
        if status.as_u16() == 202 {
            return Ok(Value::Null);
        }
        if !status.is_success() {
            let body = response.text().unwrap_or_default();
            anyhow::bail!("MCP HTTP {status}: {body}");
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_ascii_lowercase();
        if content_type.contains("text/event-stream") {
            let text = response.text()?;
            return extract_json_rpc_result_from_sse(&text, id);
        }
        let body: Value = response.json()?;
        if let Some(message) = body
            .pointer("/error/message")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
        {
            let code = body
                .pointer("/error/code")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            anyhow::bail!("MCP error {code}: {message}");
        }
        Ok(body.get("result").cloned().unwrap_or(Value::Null))
    }

    fn notify(&self, method: &str, params: Value) -> anyhow::Result<()> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });
        let _ = self.post_to_url(&payload, None)?;
        Ok(())
    }

    fn post_to_url(
        &self,
        payload: &Value,
        timeout_ms: Option<u64>,
    ) -> anyhow::Result<reqwest::blocking::Response> {
        let mut request = self
            .http
            .post(&self.config.url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .header(reqwest::header::ACCEPT, "application/json, text/event-stream")
            .json(payload);
        for (key, value) in &self.config.headers {
            request = request.header(key, value);
        }
        if let Some(session) = self
            .session_id
            .lock()
            .expect("mcp remote session lock")
            .as_deref()
        {
            request = request.header("Mcp-Session-Id", session);
        }
        if let Some(timeout_ms) = timeout_ms {
            request = request.timeout(Duration::from_millis(timeout_ms.max(1)));
        }
        Ok(request.send()?)
    }
}

pub fn assert_allowed_remote_mcp_url(url_text: &str) -> anyhow::Result<Url> {
    let parsed = Url::parse(url_text)
        .map_err(|_| anyhow::anyhow!("Invalid MCP remote URL: {url_text}"))?;
    let host = parsed.host_str().unwrap_or("").to_ascii_lowercase();
    let is_local = host == "localhost"
        || host == "127.0.0.1"
        || host == "::1"
        || host.ends_with(".localhost");
    match parsed.scheme() {
        "http" => {
            if !is_local {
                anyhow::bail!("MCP remote URL must use HTTPS (got {url_text})");
            }
        }
        "https" => {}
        _ => anyhow::bail!("MCP remote URL must be http(s) (got {url_text})"),
    }
    Ok(parsed)
}

pub fn assert_remote_mcp_transport(transport: Option<&str>) -> anyhow::Result<()> {
    let raw = transport.unwrap_or("http").trim().to_ascii_lowercase();
    if raw.is_empty() || raw == "http" || raw == "streamable-http" {
        return Ok(());
    }
    if raw == "sse" {
        anyhow::bail!(
            "MCP remote transport \"sse\" is not supported; use type \"http\" (Streamable HTTP)"
        );
    }
    anyhow::bail!("Unsupported MCP remote transport: {raw}")
}

fn parse_tool_list_result(result: &Value) -> Vec<McpRemoteToolInfo> {
    result
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let name = item.get("name").and_then(Value::as_str)?.to_string();
            Some(McpRemoteToolInfo {
                name,
                description: item
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                input_schema: item.get("inputSchema").cloned(),
            })
        })
        .collect()
}

fn format_mcp_content(result: &Value) -> String {
    let Some(items) = result.get("content").and_then(Value::as_array) else {
        return result
            .get("structuredContent")
            .map(|value| value.to_string())
            .unwrap_or_default();
    };
    items
        .iter()
        .filter_map(|item| {
            if item.get("type").and_then(Value::as_str) == Some("text") {
                item.get("text").and_then(Value::as_str).map(str::to_string)
            } else {
                Some(item.to_string())
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn extract_json_rpc_result_from_sse(text: &str, expected_id: u64) -> anyhow::Result<Value> {
    for block in text.split("\n\n") {
        for line in block.lines() {
            let line = line.trim();
            let Some(data) = line.strip_prefix("data:") else {
                continue;
            };
            let data = data.trim();
            if data.is_empty() || data == "[DONE]" {
                continue;
            }
            let Ok(payload) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            let id = payload
                .get("id")
                .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()));
            if id != Some(expected_id) {
                continue;
            }
            if let Some(message) = payload
                .pointer("/error/message")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            {
                anyhow::bail!("MCP error: {message}");
            }
            return Ok(payload.get("result").cloned().unwrap_or(Value::Null));
        }
    }
    anyhow::bail!("MCP SSE response missing JSON-RPC result for id {expected_id}")
}

pub fn remote_tool_definition(
    server_name: &str,
    tool: &McpRemoteToolInfo,
) -> ToolDefinition {
    ToolDefinition {
        name: format!(
            "mcp__{}__{}",
            sanitize_segment(server_name),
            sanitize_segment(&tool.name)
        ),
        description: format!(
            "[MCP:{server_name}] {}",
            tool.description
                .clone()
                .unwrap_or_else(|| tool.name.clone())
        ),
        category: Some("mcp".to_string()),
        aliases: Vec::new(),
        is_write: false,
        is_execute: false,
        is_concurrent: false,
        input_schema: tool
            .input_schema
            .clone()
            .unwrap_or_else(|| json!({ "type": "object", "properties": {} })),
    }
}

fn sanitize_segment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}
