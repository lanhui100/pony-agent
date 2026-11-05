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

/// F8a：MCP 子进程的环境白名单。
///
/// 第三方 MCP server 是**外部代码**，不允许继承父进程完整环境（provider API key、
/// 内部 token、云厂商凭据都在其中）。启动前 `env_clear()`，再按下表 + `spec.env`
/// 显式透传；非白名单变量一律丢弃。
///
/// 名单只保留"让 launcher 能跑起来"的变量：`npx`/`uvx`/`python` 依赖 PATH 与 HOME
/// （家目录缓存/配置），Windows 还需 COMSPEC/SystemRoot/PATHEXT 才能解析命令。
pub const MCP_ENV_ALLOWLIST: &[&str] = &[
    // 命令解析与 shell
    "PATH",
    "PATHEXT",
    "COMSPEC",
    "SHELL",
    // 用户归属与家目录（npm/npx/uvx 的缓存与配置根）
    "HOME",
    "USER",
    "LOGNAME",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    // Windows 系统盘符/系统根
    "SYSTEMROOT",
    "SYSTEMDRIVE",
    "WINDIR",
    // 临时目录
    "TMPDIR",
    "TMP",
    "TEMP",
    // 本地化（缺失会让部分 launcher 的输出编码/文案漂移）
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
    "LANGUAGE",
    "TZ",
    // 注意：**故意不**透传 TLS 信任覆盖项（SSL_CERT_FILE / SSL_CERT_DIR /
    // NODE_EXTRA_CA_CERTS / CURL_CA_BUNDLE / REQUESTS_CA_BUNDLE）。这些变量能把第三方
    // server 的出站信任锚整体替换掉（等价 MITM 授权），收益远小于风险。
    // 出站代理：MCP server 自身要联网时与 provider 保持同一出口策略
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "NO_PROXY",
    "ALL_PROXY",
    "http_proxy",
    "https_proxy",
    "no_proxy",
    "all_proxy",
];

/// F8a：构造 MCP 子进程的环境透传表（白名单变量 + `spec.env` 显式声明）。
///
/// 返回顺序稳定（白名单顺序在前，spec 覆写在后的语义由 `Command::env` 保证：
/// 同名后写覆盖前写）。
/// `spec.env` 中**禁止**的键：进程/加载器劫持向量与凭据通道。
///
/// 继承路径已由 [`MCP_ENV_ALLOWLIST`] 收口；`spec.env` 是"显式声明"，仍必须拒绝这些键 ——
/// 否则一条配置就能给第三方 server 注入 `LD_PRELOAD`、劫持 `PATH` 解析、或借
/// `SSH_AUTH_SOCK` 读取宿主 agent，等同把隔离面重新打开。
const MCP_SPEC_ENV_DENYLIST: &[&str] = &[
    "PATH",
    "PATHEXT",
    "COMSPEC",
    "SHELL",
    "BASH_ENV",
    "ENV",
    "NODE_OPTIONS",
    "NODE_PATH",
    "PYTHONPATH",
    "PYTHONSTARTUP",
    "PYTHONHOME",
    "SSH_AUTH_SOCK",
    "SSH_AGENT_PID",
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    "NODE_EXTRA_CA_CERTS",
    "CURL_CA_BUNDLE",
    "REQUESTS_CA_BUNDLE",
];

/// 前缀拒绝：动态加载器与云厂商凭据族（`LD_*` / `DYLD_*` / `AWS_*`）。
const MCP_SPEC_ENV_DENY_PREFIXES: &[&str] = &["LD_", "DYLD_", "AWS_"];

pub(crate) fn is_denied_spec_env_key(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    MCP_SPEC_ENV_DENYLIST.iter().any(|denied| upper == *denied)
        || MCP_SPEC_ENV_DENY_PREFIXES
            .iter()
            .any(|prefix| upper.starts_with(prefix))
}

/// 代理变量脱敏：剥掉 userinfo 后返回可透传值；`None` = **整体丢弃该变量**。
///
/// 代理 URL 内嵌的 userinfo 是宿主凭据；第三方 MCP server 只需要知道"走哪个代理"，
/// 不需要拿到宿主的代理账号口令。
///
/// 处理顺序与 `plugin_host/capability.rs` 的 `redact_proxy_userinfo` **同口径**（安全 > 便利）：
/// 1. **无 scheme 形态**（`user:pass@10.0.0.1:8080`）必须在 `Url::parse` **之前**处理——
///    `url` 会把它解析成 scheme=`user`、path=`pass@10.0.0.1:8080` 的"合法 URL"，username
///    为空，于是旧逻辑会把 userinfo 原样交给子进程（这正是要堵的凭据泄漏路径）；
///    这里按 `userinfo@host[:port]` 形状剥掉 `@` 之前的部分。
/// 2. 标准 URL：解析后清空 username/password，保留 scheme/host/port。
/// 3. 含 `@` 但形状无法安全解析 ⇒ **整条丢弃**（调用方只把变量名打到 stderr，绝不打值）；
///    不含 `@` 且无 `://` 的值没有 userinfo 可泄漏，原样保留（不做无谓改写）。
fn redact_proxy_userinfo(value: &std::ffi::OsStr) -> Option<std::ffi::OsString> {
    let Some(text) = value.to_str() else {
        return None;
    };

    if !text.contains("://") {
        return match text.rsplit_once('@') {
            Some((userinfo, remainder)) if !userinfo.is_empty() && !remainder.is_empty() => {
                Some(std::ffi::OsString::from(remainder))
            }
            None => Some(value.to_os_string()),
            Some(_) => None,
        };
    }

    let Ok(mut parsed) = url::Url::parse(text) else {
        return None;
    };
    if parsed.username().is_empty() && parsed.password().is_none() {
        return Some(value.to_os_string());
    }
    if parsed.set_username("").is_err() || parsed.set_password(None).is_err() {
        return None;
    }
    let redacted = parsed.as_str().replace("://@", "://");
    let redacted = if !text.ends_with('/') && redacted.ends_with('/') {
        redacted.trim_end_matches('/').to_string()
    } else {
        redacted
    };
    Some(std::ffi::OsString::from(redacted))
}

fn is_proxy_env_name(name: &str) -> bool {
    matches!(
        name.to_ascii_uppercase().as_str(),
        "HTTP_PROXY" | "HTTPS_PROXY" | "ALL_PROXY" | "NO_PROXY"
    )
}

fn mcp_child_env(spec_env: &[(String, String)]) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    let mut vars: Vec<(std::ffi::OsString, std::ffi::OsString)> = Vec::new();
    for name in MCP_ENV_ALLOWLIST {
        // 白名单统一写大写：POSIX 下真实变量名本身即大写，Windows 侧
        // `std::env::var_os` 走 `GetEnvironmentVariableW`，大小写不敏感。
        if let Some(value) = std::env::var_os(name) {
            let value = if is_proxy_env_name(name) {
                match redact_proxy_userinfo(&value) {
                    Some(redacted) => redacted,
                    None => {
                        // 只打变量名，绝不打值（值里可能带代理账号口令）。
                        eprintln!(
                            "[pony-agent] 丢弃代理环境变量 `{name}`（无法安全剥离 userinfo）。"
                        );
                        continue;
                    }
                }
            } else {
                value
            };
            vars.push((std::ffi::OsString::from(name), value));
        }
    }
    for (key, value) in spec_env {
        if is_denied_spec_env_key(key) {
            // 只打印键名，绝不打印值（值可能就是凭据）。
            eprintln!(
                "[pony-agent] MCP server env 拒绝透传被禁键 `{key}`（进程/加载器劫持或凭据通道向量）。"
            );
            continue;
        }
        let value = if is_proxy_env_name(key) {
            match redact_proxy_userinfo(std::ffi::OsStr::new(value)) {
                Some(redacted) => redacted,
                None => {
                    eprintln!(
                        "[pony-agent] 丢弃显式声明的代理环境变量 `{key}`（无法安全剥离 userinfo）。"
                    );
                    continue;
                }
            }
        } else {
            std::ffi::OsString::from(value)
        };
        vars.push((std::ffi::OsString::from(key), value));
    }
    vars
}

/// F8a：MCP 子进程环境隔离收口 —— 先 `env_clear()` 再按 [`MCP_ENV_ALLOWLIST`]
/// 与 `spec.env` 透传。这是唯一允许为 MCP 子进程设置环境的入口。
fn apply_mcp_child_env(command: &mut Command, spec_env: &[(String, String)]) {
    command.env_clear();
    for (key, value) in mcp_child_env(spec_env) {
        command.env(key, value);
    }
}

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
    /// MCP `annotations.readOnlyHint`（2024-11-05）。`None` = server 未声明；
    /// 未声明一律按**可写**处理（fail-closed：读/写事实不猜测）。
    pub read_only_hint: Option<bool>,
}

impl McpToolSpec {
    /// 该工具声明的权限事实是否只读。缺失声明 ⇒ `false`（保守）。
    pub fn is_read_only(&self) -> bool {
        self.read_only_hint == Some(true)
    }
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

    /// 分源（外部能力）错误码（ADR 0065 decision 9 / ADR 0066 decision 4）。
    ///
    /// 与 [`Self::code`] 的分工：`code()` 是 MCP 客户端内部的传输/协议分类（既有冻结契约，
    /// 不改值）；`wire_code()` 是暴露给模型与宿主的**分源**错误码，让调用方无需知道 MCP
    /// 客户端内部实现就能区分「server 挂了 / 超时 / 工具不存在」。
    pub fn wire_code(&self) -> &'static str {
        match self {
            Self::Spawn(_) | Self::Initialize(_) | Self::NotConnected | Self::Protocol(_) => {
                "mcp_server_unavailable"
            }
            Self::Timeout(_) => "mcp_timeout",
            // server 可达但这次工具调用被 server 拒绝（JSON-RPC error）：既不是「服务器不可用」，
            // 也不是「工具不存在」。ADR 0066 未枚举该情形，这里用分源码 `mcp_tool_error`
            // （不使用含糊的 `unsupported_tool`）。
            Self::Server(_) => "mcp_tool_error",
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
        // F8a：MCP server 是第三方代码，绝不能继承父进程的完整环境
        //（provider API key、内部 token、云厂商凭据都在其中）。
        apply_mcp_child_env(&mut command, &spec.env);

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
                    read_only_hint: tool
                        .get("annotations")
                        .and_then(|annotations| annotations.get("readOnlyHint"))
                        .and_then(Value::as_bool),
                })
            })
            .collect())
    }

    /// 非阻塞探测子进程是否已退出（崩溃/自行退出诊断用）。
    ///
    /// `Ok(None)` = 仍在运行；`Ok(Some(status))` = 已退出。调用方据此把「会话已死」翻译成
    /// 分源错误码（`mcp_server_unavailable`），而不是等到下一次请求超时。
    pub fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
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
            read_only_hint: None,
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

    /// ADVISORY 修复：`spec.env` 不得成为进程/加载器劫持通道。
    #[test]
    fn spec_env_must_reject_loader_hijack_and_tls_override_keys() {
        let spec_env = vec![
            ("LD_PRELOAD".to_string(), "/tmp/evil.so".to_string()),
            ("PATH".to_string(), "/tmp/evil-bin".to_string()),
            ("AWS_SECRET_ACCESS_KEY".to_string(), "leaked".to_string()),
            ("SSH_AUTH_SOCK".to_string(), "/tmp/agent.sock".to_string()),
            ("SSL_CERT_FILE".to_string(), "/tmp/evil.pem".to_string()),
            ("MCP_ALLOWED_TOKEN".to_string(), "third-party-token".to_string()),
        ];
        let vars = mcp_child_env(&spec_env);
        let keys: Vec<String> = vars
            .iter()
            .map(|(key, _)| key.to_string_lossy().to_string())
            .collect();
        for denied in [
            "LD_PRELOAD",
            "AWS_SECRET_ACCESS_KEY",
            "SSH_AUTH_SOCK",
            "SSL_CERT_FILE",
        ] {
            assert!(
                !keys.iter().any(|key| key == denied),
                "`{denied}` 必须被拒绝透传给第三方 MCP server"
            );
        }
        // PATH 由继承白名单提供（launcher 必需），但 spec 的同名值不得覆盖它。
        let path_values: Vec<String> = vars
            .iter()
            .filter(|(key, _)| key.to_string_lossy() == "PATH")
            .map(|(_, value)| value.to_string_lossy().to_string())
            .collect();
        assert!(
            !path_values.iter().any(|value| value == "/tmp/evil-bin"),
            "spec.env 不得覆盖 PATH（命令解析劫持），实际 {path_values:?}"
        );
        assert!(
            keys.iter().any(|key| key == "MCP_ALLOWED_TOKEN"),
            "非劫持向量的显式声明键必须照常透传（否则 MCP server 拿不到自己的凭据）"
        );
    }

    /// ADVISORY 修复：TLS 信任覆盖项整体不透传（继承路径同理）。
    #[test]
    fn tls_trust_override_vars_are_never_inherited() {
        for name in [
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "NODE_EXTRA_CA_CERTS",
            "CURL_CA_BUNDLE",
            "REQUESTS_CA_BUNDLE",
        ] {
            assert!(
                !MCP_ENV_ALLOWLIST.contains(&name),
                "`{name}` 能把子进程的出站信任锚整体替换掉，不得出现在继承白名单中"
            );
        }
    }

    /// ADVISORY 修复：代理 URL 内嵌凭据必须脱敏后再交给子进程。
    #[test]
    fn proxy_userinfo_is_redacted_before_reaching_the_child() {
        let redacted = redact_proxy_userinfo(std::ffi::OsStr::new(
            "http://proxy-user:s3cr3t@proxy.local:8080",
        ))
        .expect("standard URL with userinfo must stay usable after redaction");
        let text = redacted.to_string_lossy().to_string();
        assert!(
            !text.contains("s3cr3t") && !text.contains("proxy-user"),
            "代理 userinfo 必须剥除，实际 {text}"
        );
        assert!(
            text.contains("proxy.local:8080"),
            "代理地址本身必须保留，实际 {text}"
        );

        // 无 userinfo 时原样返回（不做猜测式改写）。
        let untouched = redact_proxy_userinfo(std::ffi::OsStr::new("http://proxy.local:8080"))
            .expect("no userinfo => pass through");
        assert_eq!(untouched.to_string_lossy(), "http://proxy.local:8080");
    }

    /// L3-B-D：代理 userinfo 脱敏的 7 种形态矩阵（与 `plugin_host/capability.rs` 同口径）。
    ///
    /// 关键回归：**无 scheme** 的 `user:pass@10.0.0.1:8080` 会被 `Url::parse` 当成
    /// scheme=`user` 的合法 URL（username 为空），旧逻辑因此把凭据原样交给子进程。
    #[test]
    fn proxy_userinfo_redaction_covers_seven_shapes() {
        fn redact(input: &str) -> Option<String> {
            redact_proxy_userinfo(std::ffi::OsStr::new(input))
                .map(|value| value.to_string_lossy().to_string())
        }

        // 1. 带 scheme + 凭据 → 剥凭据保留 host:port
        assert_eq!(
            redact("http://user:pass@10.0.0.1:8080").as_deref(),
            Some("http://10.0.0.1:8080")
        );
        // 2. 无 scheme + 凭据（真实存在的写法）→ `@` 前段整体剥掉
        assert_eq!(redact("user:pass@10.0.0.1:8080").as_deref(), Some("10.0.0.1:8080"));
        // 3. 带 scheme 无凭据 → 原样
        assert_eq!(
            redact("https://proxy.local:3128").as_deref(),
            Some("https://proxy.local:3128")
        );
        // 4. 无 scheme 无凭据 → 原样（`10.0.0.1:8080` 形状不能被误改）
        assert_eq!(redact("10.0.0.1:8080").as_deref(), Some("10.0.0.1:8080"));
        // 5. 畸形：`@` 后为空 → 整条丢弃，绝不原样透传
        assert_eq!(redact("user:pass@"), None);
        // 6. 畸形：`@` 前为空 → 丢弃
        assert_eq!(redact("@10.0.0.1:8080"), None);
        // 7. 非 UTF-8 不可得，用"含凭据但 host 缺失"的畸形 URL 代表不可安全解析 → 丢弃
        assert_eq!(redact("http://user:pass@"), None);
    }

    /// F8a 真实验证：真实启动一个子进程（走 `/usr/bin/env python3` 这条需要 PATH
    /// 的 launcher 路径），让它把自己看到的全部环境变量名 dump 出来，断言
    /// ① 子进程只看到白名单 + `spec.env` 的变量（父进程完整环境未泄漏）；
    /// ② PATH 被保留（否则 launcher 直接"命令找不到"）；
    /// ③ `spec.env` 显式声明的变量照常透传。
    #[test]
    fn child_process_env_is_cleared_and_only_allowlisted_vars_survive() {
        let (dir, script) = write_fake_server(
            "env-dump",
            &format!(
                r#"
import json, os, sys

with open(sys.argv[1], "w", encoding="utf-8") as fh:
    json.dump(sorted(os.environ.keys()), fh)
{prelude}
"#,
                prelude = FAKE_SERVER_PRELUDE
                    .replace("PROTOCOL", "\"2024-11-05\"")
                    .replace("MODE", "\"ignored\"")
            ),
        );
        let dump_path = dir.join("child-env.json");
        let dump_arg = dump_path.to_string_lossy().to_string();
        let mut spec = spec_for(&script, &[dump_arg.as_str()], "unit-env-dump");
        spec = spec.with_env("PONY_MCP_TEST_TOKEN", "explicit-value");

        let client = McpClient::connect(&spec, Duration::from_secs(10))
            .expect("handshake must succeed: PATH preserved so python3 launcher resolves");
        drop(client);

        let raw = std::fs::read_to_string(&dump_path).expect("child must dump its env");
        let child_keys: Vec<String> = serde_json::from_str(&raw).expect("env dump must be json");
        let allowlist: std::collections::BTreeSet<&str> =
            MCP_ENV_ALLOWLIST.iter().copied().collect();
        let leaked: Vec<&String> = child_keys
            .iter()
            .filter(|key| !allowlist.contains(key.as_str()) && key.as_str() != "PONY_MCP_TEST_TOKEN")
            .collect();
        assert!(
            leaked.is_empty(),
            "MCP 子进程继承了白名单之外的父进程环境变量：{leaked:?}"
        );
        assert!(
            child_keys.iter().any(|key| key == "PATH"),
            "PATH 必须透传，否则 npx/uvx/python 无法启动：{child_keys:?}"
        );
        assert!(
            child_keys.iter().any(|key| key == "PONY_MCP_TEST_TOKEN"),
            "spec.env 显式声明的变量必须透传：{child_keys:?}"
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}
