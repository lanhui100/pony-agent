//! MCP（Model Context Protocol）客户端：标准握手 + 长连接会话（W27）。
//!
//! 背景：审计（`docs/audit/agent-core-gap-matrix.md` P0-3 / A13）指出本项目此前的 stdio 传输
//! 是「每次请求 spawn 一个进程 + 自造 method 名（`list_resources`/`read_resource`）」的一次性
//! 往返，既没有 MCP 规范的 `initialize` 能力协商 / `tools/list` 握手，也不是长连接会话；资源面
//! 因此在生产路径上只能 fail-closed。对比基线（claude-code `services/mcp/client.ts`、DSH
//! `packages/mcp/mcp-client`）都是「长连接 + 握手 + 规范 method 名」。
//!
//! 本模块实现规范最小子集（MCP 2024-11-05，JSON-RPC 2.0，按行分隔）：
//! - [`McpClient::connect`]：spawn 子进程 → `initialize` 握手 → `notifications/initialized`；
//! - [`McpClient::list_tools`] / [`McpClient::call_tool`]：`tools/list`、`tools/call`；
//! - [`McpClient::list_resources`] / [`McpClient::read_resource`]：`resources/list`、`resources/read`；
//! - 请求 id 递增并按 id 匹配响应（跳过服务端通知）；每请求超时；`Drop` 终止子进程。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// 本客户端实现的 MCP 协议版本。
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";
/// 默认单请求超时。
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
/// 默认握手（initialize）超时：npx/uvx 这类冷启动较慢，必须比普通请求预算大。
pub const DEFAULT_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// 超时下限：零时长既无意义又容易触发「刚发出就超时」。
pub const MIN_TIMEOUT: Duration = Duration::from_millis(1);

/// 一个 MCP server 的启动描述。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct McpServerSpec {
    /// 本会话内唯一的 server id（用于工具名 `mcp__<id>__<tool>` 命名空间）。
    pub id: String,
    /// 启动命令（如 `npx`、`python3`、可执行文件路径）。
    pub command: String,
    /// 命令行参数。
    pub args: Vec<String>,
    /// 额外环境变量。
    pub env: Vec<(String, String)>,
}

impl McpServerSpec {
    pub fn new(id: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            command: command.into(),
            args: Vec::new(),
            env: Vec::new(),
        }
    }

    pub fn with_arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }
}

/// `tools/list` 返回的工具描述。
#[derive(Clone, Debug, PartialEq)]
pub struct McpToolSpec {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

/// MCP 客户端错误（结构化，调用方按 [`McpClientError::code`] 分支）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum McpClientError {
    /// 子进程启动失败。
    Spawn(String),
    /// 握手（`initialize`）失败。
    Initialize(String),
    /// 请求超时。
    Timeout(String),
    /// 协议层错误（非法 JSON、id 不匹配、缺 result）。
    Protocol(String),
    /// 服务端返回 `error`。
    Server(String),
    /// 会话已不可用（管道关闭等）。
    NotConnected,
}

impl McpClientError {
    /// 稳定的机器可读错误码。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Spawn(_) => "spawn_failed",
            Self::Initialize(_) => "initialize_failed",
            Self::Timeout(_) => "timeout",
            Self::Protocol(_) => "protocol_error",
            Self::Server(_) => "server_error",
            Self::NotConnected => "not_connected",
        }
    }

    /// 面向模型/日志的中文说明。
    pub fn message(&self) -> &str {
        match self {
            Self::Spawn(msg)
            | Self::Initialize(msg)
            | Self::Timeout(msg)
            | Self::Protocol(msg)
            | Self::Server(msg) => msg,
            Self::NotConnected => "MCP 会话已断开（子进程管道关闭）。",
        }
    }
}

/// 长连接 MCP 客户端。
pub struct McpClient {
    child: Child,
    /// 写入端：`Option` 以便 `Drop` 时 take 并关闭（EOF 让 server 自行退出）。
    stdin: std::sync::Mutex<Option<ChildStdin>>,
    responses: std::sync::Mutex<Receiver<String>>,
    /// 请求串行锁：MCP 会话是「一问一答」模型，同一 client 上并发请求必须排队，
    /// 否则收包方会互相抢走响应（stdout 不可重放）。
    request_lock: std::sync::Mutex<()>,
    /// 迟到/乱序响应缓冲：按 id 暂存，后续同 id 请求可命中，绝不丢弃。
    pending: std::sync::Mutex<HashMap<u64, Value>>,
    next_id: AtomicU64,
    timeout: Duration,
    /// 握手预算（initialize 使用）。
    connect_timeout: Duration,
    spec_id: String,
    server_name: String,
    server_version: String,
    server_capabilities: Value,
    stderr_tail: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl std::fmt::Debug for McpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpClient")
            .field("server_id", &self.spec_id)
            .field("server_name", &self.server_name)
            .field("timeout", &self.timeout)
            .finish()
    }
}

impl McpClient {
    /// 启动子进程并完成 MCP 握手（握手使用 [`DEFAULT_CONNECT_TIMEOUT`]，请求使用 `timeout`）。
    pub fn connect(spec: &McpServerSpec, timeout: Duration) -> Result<Self, McpClientError> {
        Self::connect_with_timeouts(spec, DEFAULT_CONNECT_TIMEOUT, timeout)
    }

    /// 启动子进程并完成 MCP 握手；握手与普通请求可使用不同预算。
    pub fn connect_with_timeouts(
        spec: &McpServerSpec,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> Result<Self, McpClientError> {
        let mut command = Command::new(&spec.command);
        command
            .args(&spec.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Unix：独立进程组，便于 Drop 时连同 npx/uvx 这类 wrapper 的孙进程一起回收。
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        for (key, value) in &spec.env {
            command.env(key, value);
        }

        let mut child = command.spawn().map_err(|err| {
            McpClientError::Spawn(format!(
                "启动 MCP server `{}`（{}）失败：{err}。",
                spec.id, spec.command
            ))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| McpClientError::Spawn("无法获取 MCP 子进程 stdin。".to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| McpClientError::Spawn("无法获取 MCP 子进程 stdout。".to_string()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| McpClientError::Spawn("无法获取 MCP 子进程 stderr。".to_string()))?;

        // stdout 逐行读取线程：阻塞读 → mpsc，主线程按超时取用。
        let (tx, responses) = mpsc::channel::<String>();
        std::thread::Builder::new()
            .name(format!("mcp-stdout-{}", spec.id))
            .spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    match line {
                        Ok(line) => {
                            if tx.send(line).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|err| McpClientError::Spawn(format!("启动 MCP 读取线程失败：{err}。")))?;

        // stderr 排空线程：避免子进程写满管道阻塞；保留尾部若干行用于诊断。
        let stderr_tail = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let tail = std::sync::Arc::clone(&stderr_tail);
        std::thread::Builder::new()
            .name(format!("mcp-stderr-{}", spec.id))
            .spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    if let Ok(mut tail) = tail.lock() {
                        tail.push(line);
                        if tail.len() > 20 {
                            tail.remove(0);
                        }
                    }
                }
            })
            .ok();

        let mut client = Self {
            child,
            stdin: std::sync::Mutex::new(Some(stdin)),
            responses: std::sync::Mutex::new(responses),
            request_lock: std::sync::Mutex::new(()),
            pending: std::sync::Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            timeout: request_timeout.max(MIN_TIMEOUT),
            connect_timeout: connect_timeout.max(MIN_TIMEOUT),
            spec_id: spec.id.clone(),
            server_name: String::new(),
            server_version: String::new(),
            server_capabilities: Value::Null,
            stderr_tail,
        };

        let init = client.request_with_timeout(
            "initialize",
            client.connect_timeout,
            json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {
                    "name": "pony-agent",
                    "version": env!("CARGO_PKG_VERSION"),
                }
            }),
        )?;
        let negotiated = init
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if negotiated != MCP_PROTOCOL_VERSION {
            return Err(McpClientError::Initialize(format!(
                "MCP server `{}` 协商的 protocolVersion=`{negotiated}` 与客户端支持的 `{MCP_PROTOCOL_VERSION}` 不兼容。",
                spec.id
            )));
        }

        let server_info = init.get("serverInfo").cloned().unwrap_or(Value::Null);
        client.server_name = server_info
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        client.server_version = server_info
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        client.server_capabilities = init.get("capabilities").cloned().unwrap_or(Value::Null);

        if client.server_name.is_empty() {
            return Err(McpClientError::Initialize(format!(
                "MCP server `{}` 的 initialize 结果缺少 serverInfo.name。",
                spec.id
            )));
        }

        client.notify("notifications/initialized", json!({}))?;
        Ok(client)
    }

    /// `initialize` 握手得到的 server 名称。
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// `initialize` 握手得到的 server 版本。
    pub fn server_version(&self) -> &str {
        &self.server_version
    }

    /// `initialize` 握手得到的 server capabilities（原样 JSON）。
    pub fn server_capabilities(&self) -> &Value {
        &self.server_capabilities
    }

    /// 本会话唯一 server id。
    pub fn server_id(&self) -> &str {
        &self.spec_id
    }

    /// stderr 尾部若干行（诊断用）。
    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr_tail
            .lock()
            .map(|tail| tail.clone())
            .unwrap_or_default()
    }

    /// `tools/list`：返回工具描述列表。
    pub fn list_tools(&self) -> Result<Vec<McpToolSpec>, McpClientError> {
        let result = self.request("tools/list", json!({}))?;
        let tools = result
            .get("tools")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                McpClientError::Protocol("tools/list 结果缺少 tools 数组。".to_string())
            })?;

        Ok(tools
            .iter()
            .filter_map(|tool| {
                let name = tool.get("name").and_then(Value::as_str)?.to_string();
                Some(McpToolSpec {
                    name,
                    description: tool
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    input_schema: tool
                        .get("inputSchema")
                        .cloned()
                        .unwrap_or_else(|| json!({"type": "object"})),
                })
            })
            .collect())
    }

    /// `tools/call`：调用工具并返回 `result` 对象。
    pub fn call_tool(&self, name: &str, arguments: Value) -> Result<Value, McpClientError> {
        self.request("tools/call", json!({ "name": name, "arguments": arguments }))
    }

    /// `resources/list`：返回资源列表（原样 `resources` 数组元素）。
    pub fn list_resources(&self) -> Result<Vec<Value>, McpClientError> {
        let result = self.request("resources/list", json!({}))?;
        let resources = result
            .get("resources")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                McpClientError::Protocol("resources/list 结果缺少 resources 数组。".to_string())
            })?;
        Ok(resources.clone())
    }

    /// `resources/templates/list`：返回资源模板列表（原样 `resourceTemplates` 数组元素）。
    pub fn list_resource_templates(&self) -> Result<Vec<Value>, McpClientError> {
        let result = self.request("resources/templates/list", json!({}))?;
        Ok(result
            .get("resourceTemplates")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// `resources/read`：按 uri 读取资源，返回 `result` 对象。
    pub fn read_resource(&self, uri: &str) -> Result<Value, McpClientError> {
        self.request("resources/read", json!({ "uri": uri }))
    }

    /// 发送通知（无 id、不等响应）。
    pub fn notify(&self, method: &str, params: Value) -> Result<(), McpClientError> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.write_line(&payload)
    }

    fn write_line(&self, payload: &Value) -> Result<(), McpClientError> {
        let mut line = serde_json::to_string(payload)
            .map_err(|err| McpClientError::Protocol(format!("序列化 JSON-RPC 失败：{err}。")))?;
        line.push('\n');
        let mut guard = self.stdin.lock().map_err(|_| McpClientError::NotConnected)?;
        let stdin = guard.as_mut().ok_or(McpClientError::NotConnected)?;
        stdin
            .write_all(line.as_bytes())
            .and_then(|_| stdin.flush())
            .map_err(|_| McpClientError::NotConnected)
    }

    fn request(&self, method: &str, params: Value) -> Result<Value, McpClientError> {
        self.request_with_timeout(method, self.timeout, params)
    }

    fn request_with_timeout(
        &self,
        method: &str,
        timeout: Duration,
        params: Value,
    ) -> Result<Value, McpClientError> {
        // 串行化：MCP 会话一问一答，stdout 不可重放，因此同一 client 上的请求必须排队。
        let _serial = self
            .request_lock
            .lock()
            .map_err(|_| McpClientError::NotConnected)?;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let payload = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.write_line(&payload)?;

        let timeout = timeout.max(MIN_TIMEOUT);
        let started = Instant::now();
        let deadline = started.checked_add(timeout).unwrap_or(started + Duration::from_secs(3600));

        loop {
            // 先查迟到响应缓冲（上一次同 id 的响应可能已被暂存）。
            if let Some(buffered) = self
                .pending
                .lock()
                .ok()
                .and_then(|mut pending| pending.remove(&id))
            {
                return Self::extract_result(method, buffered);
            }

            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(McpClientError::Timeout(format!(
                    "MCP `{method}` 请求超时（{timeout:?}）：server `{}`。会话保留，调用方可重试同一次读取（资源读取是幂等的）。",
                    self.spec_id
                )));
            }

            let receiver = self
                .responses
                .lock()
                .map_err(|_| McpClientError::NotConnected)?;
            let line = match receiver.recv_timeout(remaining) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(McpClientError::Timeout(format!(
                        "MCP `{method}` 请求超时（{timeout:?}）：server `{}`。会话保留。",
                        self.spec_id
                    )))
                }
                Err(RecvTimeoutError::Disconnected) => return Err(McpClientError::NotConnected),
            };
            drop(receiver);

            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(&line).map_err(|err| {
                McpClientError::Protocol(format!("MCP 响应不是合法 JSON（{err}）：{line}"))
            })?;

            match Self::response_id(&value) {
                Some(response_id) if response_id == id => {
                    return Self::extract_result(method, value)
                }
                Some(response_id) => {
                    // 迟到的其它响应：暂存以备未来命中，绝不丢弃。
                    if let Ok(mut pending) = self.pending.lock() {
                        pending.insert(response_id, value);
                        if pending.len() > 32 {
                            // 有界：超出上限时丢最早的（正常会话不会出现）。
                            if let Some(oldest) = pending.keys().copied().min() {
                                pending.remove(&oldest);
                            }
                        }
                    }
                }
                None => {
                    if value.get("method").is_some() {
                        // 服务端通知：无需响应，直接跳过。
                        continue;
                    }
                    if let Some(error) = value.get("error") {
                        // 无 id 的错误响应：这是协议级/会话级错误，必须上抛而非吞掉。
                        return Err(McpClientError::Server(format!(
                            "MCP `{method}` 收到无 id 的错误响应：{}",
                            Self::error_message(error)
                        )));
                    }
                    // 其余无 id 且非通知的报文：协议异常，绝不静默吞掉。
                    return Err(McpClientError::Protocol(format!(
                        "MCP `{method}` 收到无法归属的报文：{value}"
                    )));
                }
            }
        }
    }

    /// 解析响应 id：接受数字或数字字符串（JSON-RPC 允许两者）。
    fn response_id(value: &Value) -> Option<u64> {
        match value.get("id") {
            Some(Value::Number(number)) => number.as_u64(),
            Some(Value::String(text)) => text.trim().parse::<u64>().ok(),
            _ => None,
        }
    }

    fn error_message(error: &Value) -> String {
        error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string())
    }

    fn extract_result(method: &str, value: Value) -> Result<Value, McpClientError> {
        if let Some(error) = value.get("error") {
            return Err(McpClientError::Server(format!(
                "MCP `{method}` 返回错误：{}",
                Self::error_message(error)
            )));
        }
        value.get("result").cloned().ok_or_else(|| {
            McpClientError::Protocol(format!("MCP `{method}` 响应缺少 result：{value}"))
        })
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // 1) 关闭 stdin（EOF）让规范的 server 自行退出。
        if let Ok(mut guard) = self.stdin.lock() {
            if let Some(mut stdin) = guard.take() {
                let _ = stdin.flush();
                drop(stdin);
            }
        }

        // 2) 宽限 200ms 等自然退出。
        let deadline = Instant::now() + Duration::from_millis(200);
        while Instant::now() < deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(_) => break,
            }
        }

        // 3) 兜底终止：Unix 直接对进程组发 SIGKILL（覆盖 npx/uvx 的孙进程），其它平台 kill 子进程。
        #[cfg(unix)]
        {
            let pid = self.child.id() as i32;
            // safety: 进程组由本模块以 process_group(0) 创建，pgid == 子进程 pid。
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 便捷构造：从 `env` 风格的键值数组建 spec。
pub fn spec_from_env_pairs(
    id: impl Into<String>,
    command: impl Into<String>,
    args: Vec<String>,
    env_pairs: Vec<(String, String)>,
) -> McpServerSpec {
    McpServerSpec {
        id: id.into(),
        command: command.into(),
        args,
        env: env_pairs,
    }
}

/// 把 `tools/list` 结果映射成 `mcp__<server>__<tool>` 命名的注册项（供能力注册表使用）。
pub fn namespaced_tools(server_id: &str, tools: &[McpToolSpec]) -> HashMap<String, McpToolSpec> {
    tools
        .iter()
        .map(|tool| (format!("mcp__{server_id}__{}", tool.name), tool.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_spec_builder_accumulates_args_and_env() {
        let spec = McpServerSpec::new("fs", "npx")
            .with_arg("-y")
            .with_env("TOKEN", "abc");
        assert_eq!(spec.id, "fs");
        assert_eq!(spec.command, "npx");
        assert_eq!(spec.args, vec!["-y".to_string()]);
        assert_eq!(spec.env, vec![("TOKEN".to_string(), "abc".to_string())]);
    }

    #[test]
    fn error_codes_are_stable() {
        assert_eq!(McpClientError::Spawn(String::new()).code(), "spawn_failed");
        assert_eq!(
            McpClientError::Initialize(String::new()).code(),
            "initialize_failed"
        );
        assert_eq!(McpClientError::Timeout(String::new()).code(), "timeout");
        assert_eq!(
            McpClientError::Protocol(String::new()).code(),
            "protocol_error"
        );
        assert_eq!(McpClientError::Server(String::new()).code(), "server_error");
        assert_eq!(McpClientError::NotConnected.code(), "not_connected");
    }

    #[test]
    fn namespaced_tools_prefix_server_id() {
        let tools = vec![McpToolSpec {
            name: "echo".to_string(),
            description: None,
            input_schema: json!({"type": "object"}),
        }];
        let map = namespaced_tools("demo", &tools);
        assert!(map.contains_key("mcp__demo__echo"));
    }

    /// 写一个最小的假 MCP server（Python3），按行处理 JSON-RPC。
    fn write_fake_server(name: &str, body: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "pony-mcp-unit-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create fake server dir");
        let script = dir.join(format!("{name}.py"));
        std::fs::write(&script, body).expect("write fake server");
        (dir, script)
    }

    fn spec_for(script: &std::path::Path, extra: &[&str], id: &str) -> McpServerSpec {
        #[cfg(windows)]
        let mut spec = McpServerSpec::new(id, "python");
        #[cfg(not(windows))]
        let mut spec = McpServerSpec::new(id, "/usr/bin/env").with_arg("python3");

        spec = spec.with_arg(script.to_string_lossy().to_string());
        for arg in extra {
            spec = spec.with_arg(*arg);
        }
        spec
    }

    const FAKE_SERVER_PRELUDE: &str = r#"
import json, sys

def reply(payload):
    sys.stdout.write(json.dumps(payload) + "\n")
    sys.stdout.flush()

for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    msg = json.loads(raw)
    method = msg.get("method")
    rid = msg.get("id")
    if method == "initialize":
        reply({"jsonrpc": "2.0", "id": rid, "result": {
            "protocolVersion": PROTOCOL,
            "capabilities": {"tools": {}, "resources": {}},
            "serverInfo": {"name": "unit-fake", "version": "0.0.0"},
        }})
    elif method == "notifications/initialized":
        continue
    elif method == "tools/list":
        if MODE == "no-id-error":
            reply({"jsonrpc": "2.0", "error": {"code": -32000, "message": "no-id boom"}})
        else:
            reply({"jsonrpc": "2.0", "id": rid, "result": {"tools": []}})
"#;

    #[test]
    fn string_ids_in_responses_are_accepted() {
        let body = FAKE_SERVER_PRELUDE
            .replace("PROTOCOL", "\"2024-11-05\"")
            .replace("MODE", "\"string-id\"")
            .replace(
                "reply({\"jsonrpc\": \"2.0\", \"id\": rid, \"result\": {\"tools\": []}})",
                "reply({\"jsonrpc\": \"2.0\", \"id\": str(rid), \"result\": {\"tools\": []}})",
            );
        let (dir, script) = write_fake_server("string-id", &body);
        let spec = spec_for(&script, &[], "unit-string-id");
        let client = McpClient::connect(&spec, Duration::from_secs(5)).expect("connect");
        let tools = client.list_tools().expect("string id responses must be accepted");
        assert!(tools.is_empty());
        drop(client);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_id_error_is_surfaced_not_swallowed() {
        let body = FAKE_SERVER_PRELUDE
            .replace("PROTOCOL", "\"2024-11-05\"")
            .replace("MODE", "\"no-id-error\"");
        let (dir, script) = write_fake_server("no-id-error", &body);
        let spec = spec_for(&script, &[], "unit-no-id-error");
        let client = McpClient::connect(&spec, Duration::from_secs(5)).expect("connect");
        let err = client
            .list_tools()
            .expect_err("no-id error must surface as server_error");
        assert_eq!(err.code(), "server_error", "{err:?}");
        assert!(err.message().contains("no-id boom"), "{err:?}");
        drop(client);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn incompatible_protocol_version_fails_closed() {
        let body = FAKE_SERVER_PRELUDE
            .replace("PROTOCOL", "\"1999-01-01\"")
            .replace("MODE", "\"ignored\"");
        let (dir, script) = write_fake_server("bad-version", &body);
        let spec = spec_for(&script, &[], "unit-bad-version");
        let err = McpClient::connect(&spec, Duration::from_secs(5))
            .expect_err("incompatible protocolVersion must fail closed");
        assert_eq!(err.code(), "initialize_failed", "{err:?}");
        assert!(err.message().contains("protocolVersion"), "{err:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn spawn_failure_maps_to_spawn_error() {
        let spec = McpServerSpec::new("missing", "/definitely/not/a/binary");
        let err = McpClient::connect(&spec, Duration::from_millis(200))
            .expect_err("connect must fail for missing binary");
        assert_eq!(err.code(), "spawn_failed");
    }
}
