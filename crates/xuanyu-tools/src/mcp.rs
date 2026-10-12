use crate::mcp_remote::{
    remote_tool_definition, McpRemoteClient, McpRemoteServerConfig,
};
use crate::{ToolDefinition, ToolOutput};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};
use xuanyu_storage::project_data_root;

const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
const MCP_CLIENT_NAME: &str = "xuanyu-agent";
const MCP_CLIENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_TIMEOUT_MS: u64 = 30_000;

#[derive(Debug, Clone, Default)]
pub struct McpLoadSummary {
    pub servers: usize,
    pub tools: usize,
    pub errors: Vec<String>,
}

#[derive(Default)]
pub struct McpRuntime {
    clients: Mutex<BTreeMap<String, McpClientHandle>>,
    tools: Mutex<BTreeMap<String, McpRegisteredTool>>,
}

#[derive(Clone)]
enum McpClientHandle {
    Stdio(Arc<McpStdioClient>),
    Remote(Arc<McpRemoteClient>),
}

impl McpClientHandle {
    fn close(&self) {
        match self {
            Self::Stdio(client) => client.close(),
            Self::Remote(client) => client.close(),
        }
    }

    fn call_tool(
        &self,
        server_name: &str,
        remote_name: &str,
        args: Map<String, Value>,
    ) -> ToolOutput {
        match self {
            Self::Stdio(client) => client.call_tool(server_name, remote_name, args),
            Self::Remote(client) => client.call_tool(remote_name, args, None),
        }
    }
}

#[derive(Clone)]
struct McpRegisteredTool {
    server_name: String,
    remote_name: String,
    client: McpClientHandle,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpServerConfig {
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
    #[serde(default)]
    url: Option<String>,
    #[serde(rename = "type", default)]
    transport_type: Option<String>,
    #[serde(default)]
    headers: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct McpConfig {
    #[serde(default)]
    mcp_servers: BTreeMap<String, McpServerConfig>,
    #[serde(default)]
    servers: BTreeMap<String, McpServerConfig>,
}

impl McpRuntime {
    pub fn load_from_project(&self, project_dir: &Path) -> (Vec<ToolDefinition>, McpLoadSummary) {
        self.clear();
        let Some(config) = load_mcp_config(project_dir) else {
            return (Vec::new(), McpLoadSummary::default());
        };
        let servers = config_servers(config);
        let mut definitions = Vec::new();
        let mut summary = McpLoadSummary {
            servers: servers.len(),
            ..McpLoadSummary::default()
        };

        for (server_name, server_config) in servers {
            let url = server_config
                .url
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            let command = server_config
                .command
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string);
            if let Some(url) = url {
                let remote_config = McpRemoteServerConfig {
                    url,
                    headers: server_config.headers,
                    timeout_ms: server_config.timeout_ms,
                    transport_type: server_config.transport_type.clone(),
                };
                match McpRemoteClient::new(server_name.clone(), remote_config)
                    .and_then(|client| {
                        client.start_with_backoff(3)?;
                        Ok(Arc::new(client))
                    }) {
                    Ok(client) => match client.list_tools() {
                        Ok(remote_tools) => {
                            let handle = McpClientHandle::Remote(client.clone());
                            self.clients
                                .lock()
                                .expect("mcp clients lock")
                                .insert(server_name.clone(), handle.clone());
                            for remote_tool in remote_tools {
                                let definition = remote_tool_definition(&server_name, &remote_tool);
                                self.tools.lock().expect("mcp tools lock").insert(
                                    definition.name.clone(),
                                    McpRegisteredTool {
                                        server_name: server_name.clone(),
                                        remote_name: remote_tool.name.clone(),
                                        client: handle.clone(),
                                    },
                                );
                                definitions.push(definition);
                            }
                        }
                        Err(err) => {
                            client.close();
                            summary
                                .errors
                                .push(format!("{server_name}: tools/list failed: {err}"));
                        }
                    },
                    Err(err) => summary
                        .errors
                        .push(format!("{server_name}: remote start failed: {err}")),
                }
                continue;
            }
            let Some(command) = command else {
                summary.errors.push(format!(
                    "{server_name}: MCP server requires `url` (remote HTTP) or local stdio `command`"
                ));
                continue;
            };
            let stdio_config = McpStdioServerConfig {
                command,
                args: server_config.args,
                env: server_config.env,
                cwd: server_config.cwd,
                timeout_ms: server_config.timeout_ms,
            };
            match McpStdioClient::start(server_name.clone(), stdio_config, project_dir) {
                Ok(client) => match client.list_tools() {
                    Ok(remote_tools) => {
                        let handle = McpClientHandle::Stdio(client.clone());
                        self.clients
                            .lock()
                            .expect("mcp clients lock")
                            .insert(server_name.clone(), handle.clone());
                        for remote_tool in remote_tools {
                            let local_name = local_mcp_tool_name(&server_name, &remote_tool.name);
                            self.tools.lock().expect("mcp tools lock").insert(
                                local_name.clone(),
                                McpRegisteredTool {
                                    server_name: server_name.clone(),
                                    remote_name: remote_tool.name.clone(),
                                    client: handle.clone(),
                                },
                            );
                            definitions.push(ToolDefinition {
                                name: local_name,
                                description: format!(
                                    "[MCP:{server_name}] {}",
                                    remote_tool
                                        .description
                                        .clone()
                                        .unwrap_or_else(|| remote_tool.name.clone())
                                ),
                                category: Some("mcp".to_string()),
                                aliases: Vec::new(),
                                is_write: false,
                                is_execute: false,
                                is_concurrent: false,
                                input_schema: remote_tool.input_schema.unwrap_or_else(|| {
                                    json!({ "type": "object", "properties": {} })
                                }),
                            });
                        }
                    }
                    Err(err) => {
                        client.close();
                        summary
                            .errors
                            .push(format!("{server_name}: tools/list failed: {err}"));
                    }
                },
                Err(err) => summary
                    .errors
                    .push(format!("{server_name}: start failed: {err}")),
            }
        }
        summary.tools = definitions.len();
        (definitions, summary)
    }

    pub fn call_tool(&self, local_name: &str, args: BTreeMap<String, Value>) -> ToolOutput {
        let handle = self
            .tools
            .lock()
            .expect("mcp tools lock")
            .get(local_name)
            .cloned();
        let Some(handle) = handle else {
            return ToolOutput {
                content: format!("MCP tool is not loaded: {local_name}"),
                is_error: true,
                data: Some(json!({ "errorType": "mcp_tool_not_loaded" })),
            };
        };
        handle.client.call_tool(
            &handle.server_name,
            &handle.remote_name,
            args.into_iter().collect(),
        )
    }

    fn clear(&self) {
        for client in self.clients.lock().expect("mcp clients lock").values() {
            client.close();
        }
        self.clients.lock().expect("mcp clients lock").clear();
        self.tools.lock().expect("mcp tools lock").clear();
    }
}

impl Drop for McpRuntime {
    fn drop(&mut self) {
        self.clear();
    }
}

#[derive(Debug, Clone)]
struct McpToolInfo {
    name: String,
    description: Option<String>,
    input_schema: Option<Value>,
}

type PendingResponse = Result<Value, String>;

#[derive(Debug, Clone)]
struct McpStdioServerConfig {
    command: String,
    args: Vec<String>,
    env: BTreeMap<String, String>,
    cwd: Option<String>,
    timeout_ms: Option<u64>,
}

struct McpStdioClient {
    server_name: String,
    config_timeout_ms: u64,
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<ChildStdin>>,
    next_id: AtomicU64,
    pending: Arc<Mutex<BTreeMap<u64, mpsc::Sender<PendingResponse>>>>,
}

impl McpStdioClient {
    fn start(
        server_name: String,
        config: McpStdioServerConfig,
        project_dir: &Path,
    ) -> anyhow::Result<Arc<Self>> {
        let cwd = config
            .cwd
            .as_deref()
            .map(|cwd| resolve_project_path(project_dir, cwd))
            .unwrap_or_else(|| project_dir.to_path_buf());
        let mut command = Command::new(&config.command);
        command
            .args(&config.args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .envs(&config.env);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let mut child = command.spawn()?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("MCP server stdin was not available"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("MCP server stdout was not available"))?;
        let stderr = child.stderr.take();
        let pending = Arc::new(Mutex::new(BTreeMap::new()));
        spawn_stdout_reader(server_name.clone(), stdout, pending.clone());
        if let Some(stderr) = stderr {
            spawn_stderr_reader(server_name.clone(), stderr);
        }
        let client = Arc::new(Self {
            server_name,
            config_timeout_ms: config.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS),
            child: Mutex::new(Some(child)),
            stdin: Mutex::new(Some(stdin)),
            next_id: AtomicU64::new(1),
            pending,
        });
        client.request(
            "initialize",
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": MCP_CLIENT_NAME, "version": MCP_CLIENT_VERSION }
            }),
            None,
        )?;
        client.notify("notifications/initialized", json!({}))?;
        Ok(client)
    }

    fn list_tools(&self) -> anyhow::Result<Vec<McpToolInfo>> {
        let result = self.request("tools/list", json!({}), None)?;
        Ok(parse_tool_list_result(&result))
    }

    fn call_tool(
        &self,
        server_name: &str,
        remote_name: &str,
        args: serde_json::Map<String, Value>,
    ) -> ToolOutput {
        match self.request(
            "tools/call",
            json!({ "name": remote_name, "arguments": Value::Object(args) }),
            None,
        ) {
            Ok(result) => {
                let content = format_mcp_content(&result);
                ToolOutput {
                    content: if content.trim().is_empty() {
                        "(empty MCP tool result)".to_string()
                    } else {
                        content
                    },
                    is_error: result
                        .get("isError")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    data: Some(json!({
                        "mcpServer": server_name,
                        "mcpTool": remote_name,
                        "raw": result
                    })),
                }
            }
            Err(err) => ToolOutput {
                content: format!("MCP tool {server_name}/{remote_name} failed: {err}"),
                is_error: true,
                data: Some(json!({
                    "errorType": if err.to_string().contains("timed out") { "timeout" } else { "mcp_error" },
                    "mcpServer": server_name,
                    "mcpTool": remote_name
                })),
            },
        }
    }

    fn request(
        &self,
        method: &str,
        params: Value,
        timeout_ms: Option<u64>,
    ) -> anyhow::Result<Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params
        });
        let (tx, rx) = mpsc::channel();
        self.pending
            .lock()
            .expect("mcp pending lock")
            .insert(id, tx);
        if let Err(err) = self.write_message(&payload) {
            self.pending.lock().expect("mcp pending lock").remove(&id);
            return Err(err);
        }
        match rx.recv_timeout(Duration::from_millis(
            timeout_ms.unwrap_or(self.config_timeout_ms),
        )) {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(message)) => anyhow::bail!("{message}"),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                self.pending.lock().expect("mcp pending lock").remove(&id);
                anyhow::bail!("MCP request timed out: {method}");
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("MCP server {} closed", self.server_name);
            }
        }
    }

    fn notify(&self, method: &str, params: Value) -> anyhow::Result<()> {
        self.write_message(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        }))
    }

    fn write_message(&self, value: &Value) -> anyhow::Result<()> {
        let mut guard = self.stdin.lock().expect("mcp stdin lock");
        let stdin = guard
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("MCP server {} is closed", self.server_name))?;
        let mut line = serde_json::to_vec(value)?;
        line.push(b'\n');
        stdin.write_all(&line)?;
        stdin.flush()?;
        Ok(())
    }

    fn close(&self) {
        self.pending.lock().expect("mcp pending lock").clear();
        let _ = self.stdin.lock().expect("mcp stdin lock").take();
        if let Some(mut child) = self.child.lock().expect("mcp child lock").take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn load_mcp_config(project_dir: &Path) -> Option<McpConfig> {
    for candidate in [
        project_dir.join("mcp.json"),
        project_data_root(project_dir).join("mcp.json"),
    ] {
        let Ok(raw) = std::fs::read_to_string(candidate) else {
            continue;
        };
        match serde_json::from_str::<McpConfig>(&raw) {
            Ok(config) => return Some(config),
            Err(_) => continue,
        }
    }
    None
}

fn config_servers(config: McpConfig) -> BTreeMap<String, McpServerConfig> {
    if config.mcp_servers.is_empty() {
        config.servers
    } else {
        config.mcp_servers
    }
}

fn spawn_stdout_reader(
    server_name: String,
    stdout: impl std::io::Read + Send + 'static,
    pending: Arc<Mutex<BTreeMap<u64, mpsc::Sender<PendingResponse>>>>,
) {
    thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines().map_while(Result::ok) {
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            let Some(id) = message.get("id").and_then(Value::as_u64) else {
                continue;
            };
            let Some(sender) = pending.lock().expect("mcp pending lock").remove(&id) else {
                continue;
            };
            let response = if let Some(error) = message.get("error") {
                let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("MCP error");
                Err(format!("MCP error {code}: {message}"))
            } else {
                Ok(message.get("result").cloned().unwrap_or(Value::Null))
            };
            let _ = sender.send(response);
        }
        let mut pending = pending.lock().expect("mcp pending lock");
        let message = format!("MCP server {server_name} exited");
        for (_, sender) in std::mem::take(&mut *pending) {
            let _ = sender.send(Err(message.clone()));
        }
    });
}

fn spawn_stderr_reader(server_name: String, stderr: impl std::io::Read + Send + 'static) {
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines().map_while(Result::ok) {
            let line = line.trim();
            if !line.is_empty() {
                eprintln!("[MCP:{server_name}] {line}");
            }
        }
    });
}

fn parse_tool_list_result(value: &Value) -> Vec<McpToolInfo> {
    value
        .get("tools")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|tool| {
            let name = tool.get("name").and_then(Value::as_str)?.trim();
            if name.is_empty() {
                return None;
            }
            Some(McpToolInfo {
                name: name.to_string(),
                description: tool
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                input_schema: tool.get("inputSchema").cloned(),
            })
        })
        .collect()
}

fn format_mcp_content(result: &Value) -> String {
    let Some(content) = result.get("content").and_then(Value::as_array) else {
        return result.to_string();
    };
    content
        .iter()
        .map(|item| {
            item.get("text")
                .or_else(|| item.get("data"))
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| item.to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn local_mcp_tool_name(server_name: &str, tool_name: &str) -> String {
    format!(
        "mcp__{}__{}",
        sanitize_mcp_name(server_name),
        sanitize_mcp_name(tool_name)
    )
}

fn sanitize_mcp_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn resolve_project_path(project_dir: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        project_dir.join(path)
    }
}
