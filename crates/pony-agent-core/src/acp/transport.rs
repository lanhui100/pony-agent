//! ACP stdio 传输层（newline-delimited JSON-RPC 2.0）。
//!
//! 分工（真源 §3/§3.2）：本层只做报文收发与配对，不做会话语义：
//! - 出站：`request`（id 关联 + deadline）/ `notify`（无 id）；
//! - 入站：reader 线程按「`id`+`method` = 请求 / `method` 无 `id` = 通知 / `id` 无 `method` = 响应」
//!   三路分流，请求交给 [`AcpInboundRequestHandler`] 应答（无 handler ⇒ fail-closed 回错误，
//!   绝不默认放行权限请求），通知交给 [`AcpNotificationHandler`]（未注入 ⇒ 丢弃 + stderr 诊断）；
//! - 进程：env 白名单 + Unix 独立进程组 + stderr 环形缓冲（尾部 32 行，崩溃诊断用），
//!   模式对齐 `agent/plugin_host/transport.rs`。
//!
//! 失败语义（禁吞异常）：所有外部进程异常都映射到 [`AcpTransportError`] 的 8 个 `acp_*` 错误码
//! （§3.2.9），不 panic、不重试、不静默丢弃；连续非法 JSON 行达
//! [`MALFORMED_LINE_STREAK_LIMIT`] 即 fail-closed（`acp_protocol_error`）。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::acp::config::AcpAgentSpec;
use crate::acp::protocol::method as acp_method;
use crate::acp::protocol::{RequestPermissionOutcome, RequestPermissionResponse};
use crate::agent::plugin_host::capability::{apply_child_env, resolve_workspace_dir};

/// 单次出站请求预算（NFR：`acp.request_timeout_ms` = 5000）。
///
/// 覆盖 `initialize` / `session/new` / `session/prompt`；超时即 fail-closed（`acp_timeout`）。
pub const ACP_REQUEST_TIMEOUT: Duration = Duration::from_millis(5000);

/// stdout 连续非法 JSON 行阈值（NFR：`acp.malformed_line_streak_limit` = 3）。
///
/// 未达阈值：记 stderr 诊断并跳过该行（绝不 panic）；达到阈值即 fail-closed（`acp_protocol_error`）。
pub const MALFORMED_LINE_STREAK_LIMIT: u32 = 3;

/// 子进程回收宽限（NFR：`acp.spawn_reap_grace_ms` = 500）。
///
/// `close()`：先关 stdin 等自然退出，超过宽限再对 Unix 进程组 SIGKILL，并同步 `wait()` 回收。
pub const SPAWN_REAP_GRACE: Duration = Duration::from_millis(500);

/// stderr 环形缓冲保留行数（对齐 `plugin_host::transport`）。
pub const STDERR_TAIL_LINES: usize = 32;

/// 迟到响应的有界暂存上限（与 `plugin_host::transport` 的 `pending` 同口径：32 条）。
const LATE_RESPONSE_CAPACITY: usize = 32;

/// JSON-RPC 的「方法不存在」错误码（无 handler 时的通用 fail-closed 应答）。
const JSONRPC_METHOD_NOT_FOUND: i64 = -32601;

/// JSON-RPC 的「内部错误」错误码（本层自身无法构造应答时使用）。
const JSONRPC_INTERNAL_ERROR: i64 = -32603;

/// JSON-RPC 的「需要认证」错误码（ACP v1 的 `auth_required`）。
const JSONRPC_AUTH_REQUIRED: i64 = -32000;

/// reader 线程上行的入站帧（内部类型，不上 wire）。
#[derive(Debug, Clone)]
pub enum AcpInbound {
    /// 响应：`id` 匹配挂起请求。
    Response {
        id: u64,
        result: Result<Value, AcpTransportError>,
    },
    /// 通知：有 `method` 无 `id`（如 `session/update`）。
    Notification { method: String, params: Value },
    /// 请求：有 `id` 且有 `method`（如 `session/request_permission`），必须应答。
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

/// 入站请求（agent→client）的已解析形态。
#[derive(Debug, Clone)]
pub struct AcpInboundRequest {
    pub id: Value,
    pub method: String,
    pub params: Value,
}

/// 入站请求的应答形态；`Error` 的 `code` 用 JSON-RPC 数字码。
#[derive(Debug, Clone)]
pub enum AcpInboundResponse {
    Result(Value),
    Error {
        code: i64,
        message: String,
        data: Option<Value>,
    },
}

/// 入站请求分发器：由 P1 的 `acp::session` 注入（权限走 `ask_control`、`fs/*` 走治理链、
/// `terminal/*` 走 `ProcessManager`）。
///
/// **fail-closed 硬约束**：未注入 handler 时，`session/request_permission` 必须回
/// `cancelled`（或 JSON-RPC 错误），任何情况下不得默认 `allow`。
pub trait AcpInboundRequestHandler: Send + Sync {
    fn handle(&self, request: AcpInboundRequest) -> AcpInboundResponse;
}

/// 入站**通知**分发器（agent→client 的 `method` 无 `id` 报文，如 `session/update`）。
///
/// 与 [`AcpInboundRequestHandler`] 分开：通知**不产生应答**，只做观测/事件上行
/// （P1 的 `acp::session` 在此把 update 映射为 `turn:delta` / `turn:tool`）。
///
/// 硬约束：未注入 handler 时通知**丢弃并记 stderr 诊断**（不 panic、不阻断读线程）；
/// 通知处理不得影响请求 id 配对（响应按 `id` 匹配，通知永不消耗挂起请求）。
pub trait AcpNotificationHandler: Send + Sync {
    fn handle_notification(&self, method: &str, params: serde_json::Value);
}

/// stdio 传输层错误；`code()` 是 `acp_*` 错误码唯一真源（§3.2.9）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AcpTransportError {
    /// 子进程启动失败（命令不存在 / 权限不足 / cwd 不合法）。
    Spawn(String),
    /// `initialize` 握手失败（含 `protocolVersion != 1`）。
    Initialize(String),
    /// 协议层错误（非法 JSON、id 无法归属、缺 result）。
    Protocol(String),
    /// agent 明确返回 JSON-RPC error。
    Agent(String),
    /// agent 以 `stopReason: refusal` 结束（P1 归约用）。
    Refused(String),
    /// agent 要求认证（JSON-RPC `-32000`）。
    AuthRequired(String),
    /// 单次请求超过 deadline。
    Timeout(String),
    /// 会话不可用（stdin 已关 / 进程已退出）。
    Unavailable(String),
}

impl std::fmt::Display for AcpTransportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (code, message) = match self {
            Self::Spawn(message) => (self.code(), message),
            Self::Initialize(message) => (self.code(), message),
            Self::Protocol(message) => (self.code(), message),
            Self::Agent(message) => (self.code(), message),
            Self::Refused(message) => (self.code(), message),
            Self::AuthRequired(message) => (self.code(), message),
            Self::Timeout(message) => (self.code(), message),
            Self::Unavailable(message) => (self.code(), message),
        };
        write!(formatter, "{code}: {message}")
    }
}

impl std::error::Error for AcpTransportError {}

impl AcpTransportError {
    /// 稳定的机器可读错误码：
    /// `acp_spawn_failed` / `acp_initialize_failed` / `acp_protocol_error` / `acp_agent_error`
    /// / `acp_agent_refused` / `acp_auth_required` / `acp_timeout` / `acp_unavailable`。
    pub fn code(&self) -> &'static str {
        match self {
            Self::Spawn(_) => "acp_spawn_failed",
            Self::Initialize(_) => "acp_initialize_failed",
            Self::Protocol(_) => "acp_protocol_error",
            Self::Agent(_) => "acp_agent_error",
            Self::Refused(_) => "acp_agent_refused",
            Self::AuthRequired(_) => "acp_auth_required",
            Self::Timeout(_) => "acp_timeout",
            Self::Unavailable(_) => "acp_unavailable",
        }
    }
}

/// 一个已 spawn 的 ACP agent 子进程 + stdio JSON-RPC 通道。
pub struct AcpTransport {
    child: Child,
    /// `Option` 便于 `close()` 时 take 并关闭（EOF 让 agent 自行退出）。
    ///
    /// `Arc` 是为了让 reader 线程也能写应答（入站请求必须回写 result/error）：进程是唯一的
    /// writer，两条写路径共用同一把锁，保证每次写出的是一整行（不交错、不撕裂）。
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    /// reader 线程上行的入站帧。
    inbound: Mutex<Receiver<AcpInbound>>,
    /// 入站请求分发器（未注入 = fail-closed）。
    handler: Arc<Mutex<Option<Arc<dyn AcpInboundRequestHandler>>>>,
    /// 入站通知分发器（未注入 = 丢弃 + stderr 诊断）。
    notification_handler: Arc<Mutex<Option<Arc<dyn AcpNotificationHandler>>>>,
    /// stderr 尾部环形缓冲（诊断证据）。
    stderr_tail: Arc<Mutex<Vec<String>>>,
    /// 出站请求 id 分配器。
    next_id: AtomicU64,
    /// stdout 连续非法 JSON 行计数（达 [`MALFORMED_LINE_STREAK_LIMIT`] fail-closed）。
    ///
    /// 计数读写都在 reader 线程（持同一 `Arc`）；结构体侧保留该字段是 P0 冻结契约的一部分，
    /// 供 P1 会话层/诊断读取，故此处允许「结构体侧未读」。
    #[allow(dead_code)]
    malformed_streak: Arc<AtomicU32>,
    /// 已发出终止信号（幂等标记）。
    terminated: Mutex<bool>,
    /// 迟到/乱序响应有界暂存（按 id）：超时后到达的响应绝不静默丢弃。
    late_responses: Mutex<HashMap<u64, Result<Value, AcpTransportError>>>,
    /// 出站请求串行闸：同一 transport 上的请求必须排队（stdout 不可重放，收包方会互相抢响应）。
    request_lock: Mutex<()>,
    /// reader 线程记录的 fail-closed 原因（协议错乱 / agent 退出），让挂起的 `request`
    /// 立即返回真源错误码而不是塌缩成 `acp_unavailable`。
    fatal: Arc<Mutex<Option<AcpTransportError>>>,
}

impl AcpTransport {
    /// 启动外部 ACP agent。
    ///
    /// 隔离面在此收口：`env_clear()` + 白名单（复用
    /// `agent::plugin_host::capability::apply_child_env`）、`current_dir = cwd`（必须是
    /// canonicalize 过的绝对路径）、Unix `process_group(0)`。
    pub fn spawn(
        spec: &AcpAgentSpec,
        cwd: &Path,
        extra_env: &[(String, String)],
    ) -> Result<Self, AcpTransportError> {
        // §3.2.2：cwd 必须绝对（`session/new` 也要求绝对路径）；`resolve_workspace_dir` 是
        // 插件/命令钩子同一把校验尺（存在性 + 是目录 + canonicalize，符号链接按真实落点）。
        if !cwd.is_absolute() {
            return Err(AcpTransportError::Spawn(format!(
                "session cwd 必须是绝对路径（§3.2.2），实际 `{}`。",
                cwd.display()
            )));
        }
        let workspace = resolve_workspace_dir(cwd).map_err(AcpTransportError::Spawn)?;

        let mut process = Command::new(&spec.command);
        process
            .args(&spec.args)
            .current_dir(&workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Unix：独立进程组，`close()`/`Drop` 时对整组 SIGKILL（覆盖 agent 派生的孙进程）。
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            process.process_group(0);
        }
        // 隔离面唯一入口（F8a）：清单 env 与调用方 extra_env 合并后仍受白名单/拒绝键约束，
        // provider key、`SSH_AUTH_SOCK`、`LD_*` 等一律不外泄。
        let mut env_extra: Vec<(String, String)> = spec
            .env
            .iter()
            .map(|item| (item.name.clone(), item.value.clone()))
            .collect();
        env_extra.extend(extra_env.iter().cloned());
        apply_child_env(&mut process, &env_extra);

        let mut child = process.spawn().map_err(|error| {
            AcpTransportError::Spawn(format!(
                "启动 ACP agent `{}` 失败：{error}。",
                spec.command
            ))
        })?;

        let Some(stdin) = child.stdin.take() else {
            reap_child(child);
            return Err(AcpTransportError::Spawn("无法获取 agent stdin。".into()));
        };
        let Some(stdout) = child.stdout.take() else {
            reap_child(child);
            return Err(AcpTransportError::Spawn("无法获取 agent stdout。".into()));
        };
        let Some(stderr) = child.stderr.take() else {
            reap_child(child);
            return Err(AcpTransportError::Spawn("无法获取 agent stderr。".into()));
        };

        let stdin = Arc::new(Mutex::new(Some(stdin)));
        let stderr_tail = Arc::new(Mutex::new(Vec::<String>::new()));
        let handler = Arc::new(Mutex::new(None));
        let notification_handler = Arc::new(Mutex::new(None));
        let malformed_streak = Arc::new(AtomicU32::new(0));
        let fatal = Arc::new(Mutex::new(None));
        let (sender, receiver) = mpsc::channel::<AcpInbound>();

        // stdout 读线程：按行解析 + 三路分流 + 应答入站请求。
        let reader = std::thread::Builder::new()
            .name("acp-stdout".to_string())
            .spawn({
                let stdin = Arc::clone(&stdin);
                let stderr_tail = Arc::clone(&stderr_tail);
                let handler = Arc::clone(&handler);
                let notification_handler = Arc::clone(&notification_handler);
                let malformed_streak = Arc::clone(&malformed_streak);
                let fatal = Arc::clone(&fatal);
                move || {
                    read_loop(
                        stdout,
                        sender,
                        stdin,
                        handler,
                        notification_handler,
                        stderr_tail,
                        malformed_streak,
                        fatal,
                    )
                }
            });
        let reader = match reader {
            Ok(handle) => handle,
            Err(error) => {
                reap_child(child);
                return Err(AcpTransportError::Spawn(format!(
                    "启动 stdout 读线程失败：{error}。"
                )));
            }
        };

        // stderr 排空线程：只保留尾部 [`STDERR_TAIL_LINES`] 行作为崩溃诊断证据。
        let stderr_reader = std::thread::Builder::new()
            .name("acp-stderr".to_string())
            .spawn({
                let tail = Arc::clone(&stderr_tail);
                move || {
                    let reader = BufReader::new(stderr);
                    for line in reader.lines().map_while(Result::ok) {
                        push_bounded(&tail, line);
                    }
                }
            });
        if let Err(error) = stderr_reader {
            // 读线程已就绪但诊断通道起不来：按启动失败处理，同步回收子进程（不泄漏）。
            reap_child(child);
            let _ = reader.join();
            return Err(AcpTransportError::Spawn(format!(
                "启动 stderr 读线程失败：{error}。"
            )));
        }

        Ok(Self {
            child,
            stdin,
            inbound: Mutex::new(receiver),
            handler,
            notification_handler,
            stderr_tail,
            next_id: AtomicU64::new(1),
            malformed_streak,
            terminated: Mutex::new(false),
            late_responses: Mutex::new(HashMap::new()),
            request_lock: Mutex::new(()),
            fatal,
        })
    }

    /// 子进程 pid。
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// stderr 尾部若干行（诊断用）。
    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr_tail
            .lock()
            .map(|tail| tail.clone())
            .unwrap_or_default()
    }

    /// 注入 / 替换入站请求分发器。
    pub fn set_handler(&mut self, handler: Arc<dyn AcpInboundRequestHandler>) {
        if let Ok(mut slot) = self.handler.lock() {
            *slot = Some(handler);
        }
    }

    /// 注入 / 替换入站通知分发器（`&self`：reader 线程运行期可热替换）。
    ///
    /// 未注入时 reader 线程对 `session/update` 等通知只记 stderr 诊断并丢弃，不得 panic。
    pub fn set_notification_handler(&self, handler: Arc<dyn AcpNotificationHandler>) {
        if let Ok(mut slot) = self.notification_handler.lock() {
            *slot = Some(handler);
        }
    }

    /// 发送通知（无 id、不等待响应），如 `session/cancel`。
    pub fn notify(&self, method: &str, params: Value) -> Result<(), AcpTransportError> {
        if let Some(error) = self.fatal_error() {
            return Err(error);
        }
        write_line(
            &self.stdin,
            &json!({ "jsonrpc": "2.0", "method": method, "params": params }),
        )
    }

    /// 发送请求并按 `deadline` 等待同 id 响应，返回 `result`。
    ///
    /// 同一 transport 上的请求串行化；迟到响应有界暂存；入站请求由 reader 线程先分流再匹配。
    pub fn request(
        &self,
        method: &str,
        params: Value,
        deadline: Duration,
    ) -> Result<Value, AcpTransportError> {
        // 串行闸：收包通道是单流的，两个请求同时等会把对方的响应吞掉。
        let _serial = self.request_lock.lock().map_err(|_| {
            AcpTransportError::Unavailable("request 串行锁已中毒。".into())
        })?;
        // 已 fail-closed 的会话不得再发新请求（不重试、不放行）。
        if let Some(error) = self.fatal_error() {
            return Err(error);
        }

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        write_line(
            &self.stdin,
            &json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }),
        )?;

        let started = Instant::now();
        let end = started
            .checked_add(deadline)
            .unwrap_or(started + Duration::from_secs(3600));

        loop {
            if let Some(buffered) = self.take_late_response(id) {
                return buffered;
            }
            if let Some(error) = self.fatal_error() {
                return Err(error);
            }

            let remaining = end.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(AcpTransportError::Timeout(format!(
                    "`{method}` 请求超过 {deadline:?} 未响应。stderr: {:?}",
                    self.stderr_tail()
                )));
            }

            let received = {
                let receiver = self.inbound.lock().map_err(|_| {
                    AcpTransportError::Unavailable("收包锁已中毒。".into())
                })?;
                receiver.recv_timeout(remaining)
            };

            match received {
                Ok(AcpInbound::Response {
                    id: response_id,
                    result,
                }) if response_id == id => return result,
                Ok(AcpInbound::Response {
                    id: response_id,
                    result,
                }) => {
                    // 迟到/乱序响应：有界暂存，绝不静默丢弃。
                    if let Ok(mut late) = self.late_responses.lock() {
                        late.insert(response_id, result);
                        while late.len() > LATE_RESPONSE_CAPACITY {
                            let Some(oldest) = late.keys().copied().min() else {
                                break;
                            };
                            late.remove(&oldest);
                        }
                    }
                }
                // reader 线程已内联消费请求/通知，本通道只上送响应；防御性跳过。
                Ok(_) => continue,
                Err(RecvTimeoutError::Timeout) => {
                    // reader 线程可能刚记录 fail-closed（通道尚未断开）：优先返回真源错误码。
                    if let Some(error) = self.fatal_error() {
                        return Err(error);
                    }
                    return Err(AcpTransportError::Timeout(format!(
                        "`{method}` 请求超过 {deadline:?} 未响应。stderr: {:?}",
                        self.stderr_tail()
                    )));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    if let Some(error) = self.fatal_error() {
                        return Err(error);
                    }
                    return Err(AcpTransportError::Unavailable(format!(
                        "agent 管道已关闭（`{method}` 期间崩溃或自行退出）。stderr: {:?}",
                        self.stderr_tail()
                    )));
                }
            }
        }
    }

    /// 关闭并回收子进程（幂等）。
    ///
    /// 顺序：关 stdin → 宽限 [`SPAWN_REAP_GRACE`] → Unix 进程组 SIGKILL → `kill()` + `wait()`。
    pub fn close(&mut self) {
        {
            let Ok(mut terminated) = self.terminated.lock() else {
                return;
            };
            if *terminated {
                return;
            }
            *terminated = true;
        }

        if let Ok(mut guard) = self.stdin.lock() {
            if let Some(mut stdin) = guard.take() {
                let _ = stdin.flush();
                drop(stdin);
            }
        }

        let grace_deadline = Instant::now() + SPAWN_REAP_GRACE;
        while Instant::now() < grace_deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                // 观察失败不折叠为「已退出」：继续走 SIGKILL 兜底（禁止静默放行）。
                Err(_) => break,
            }
        }

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

    /// 已记录的 fail-closed 原因（只读快照，不清空：会话级失败不可恢复）。
    fn fatal_error(&self) -> Option<AcpTransportError> {
        self.fatal
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
    }

    fn take_late_response(&self, id: u64) -> Option<Result<Value, AcpTransportError>> {
        self.late_responses
            .lock()
            .ok()
            .and_then(|mut late| late.remove(&id))
    }
}

impl Drop for AcpTransport {
    fn drop(&mut self) {
        self.close();
    }
}

/// 启动失败时同步回收已 spawn 的子进程（不泄漏、不产生僵尸）。
fn reap_child(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// 写一行 JSON-RPC 报文（整行一次写出，两条写路径共用同一把 stdin 锁）。
fn write_line(
    stdin: &Arc<Mutex<Option<ChildStdin>>>,
    payload: &Value,
) -> Result<(), AcpTransportError> {
    let mut line = serde_json::to_string(payload).map_err(|error| {
        AcpTransportError::Protocol(format!("序列化 JSON-RPC 报文失败：{error}。"))
    })?;
    line.push('\n');
    let mut guard = stdin.lock().map_err(|_| {
        AcpTransportError::Unavailable("stdin 锁已中毒。".into())
    })?;
    let stdin = guard.as_mut().ok_or_else(|| {
        AcpTransportError::Unavailable("agent stdin 已关闭（进程已退出）。".into())
    })?;
    stdin
        .write_all(line.as_bytes())
        .and_then(|_| stdin.flush())
        .map_err(|error| {
            AcpTransportError::Unavailable(format!(
                "写入 agent stdin 失败（进程可能已退出）：{error}。"
            ))
        })
}

/// stdout 读线程主循环：三路分流 + fail-closed 计数 + stderr 诊断。
#[allow(clippy::too_many_arguments)]
fn read_loop(
    stdout: ChildStdout,
    sender: Sender<AcpInbound>,
    stdin: Arc<Mutex<Option<ChildStdin>>>,
    handler: Arc<Mutex<Option<Arc<dyn AcpInboundRequestHandler>>>>,
    notification_handler: Arc<Mutex<Option<Arc<dyn AcpNotificationHandler>>>>,
    stderr_tail: Arc<Mutex<Vec<String>>>,
    malformed_streak: Arc<AtomicU32>,
    fatal: Arc<Mutex<Option<AcpTransportError>>>,
) {
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            // EOF：agent 自行退出/崩溃（挂起的请求由 `request` 以管道断开报错）。
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let value: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(error) => {
                let streak = malformed_streak.fetch_add(1, Ordering::SeqCst) + 1;
                push_bounded(
                    &stderr_tail,
                    format!(
                        "[acp] 非法 JSON 行（连续第 {streak} 行）：{error}；原文={trimmed}"
                    ),
                );
                if streak >= MALFORMED_LINE_STREAK_LIMIT {
                    record_fatal(
                        &fatal,
                        AcpTransportError::Protocol(format!(
                            "stdout 连续 {streak} 行非法 JSON（上限 {MALFORMED_LINE_STREAK_LIMIT}），传输层 fail-closed。"
                        )),
                    );
                    break;
                }
                continue;
            }
        };
        // 任何一条合法报文都会打断「连续」计数（A8 语义：连续 N 行才算违约）。
        malformed_streak.store(0, Ordering::SeqCst);

        if let Some(method) = value.get("method").and_then(Value::as_str) {
            let params = value.get("params").cloned().unwrap_or(Value::Null);
            let id = value.get("id").filter(|id| !id.is_null()).cloned();
            let outcome = match id {
                Some(id) => respond_to_inbound_request(&stdin, &handler, id, method, params),
                None => {
                    dispatch_notification(&notification_handler, &stderr_tail, method, params);
                    Ok(())
                }
            };
            if let Err(error) = outcome {
                // 应答写不出去 = agent 已不可写：记录真源错误码后停止读线程（不重试）。
                record_fatal(&fatal, error);
                break;
            }
            continue;
        }

        // 无 `method`：只可能是响应。缺可解析 id 的报文永远无法归属 → fail-closed。
        let Some(response_id) = response_id(&value) else {
            record_fatal(
                &fatal,
                AcpTransportError::Protocol(format!(
                    "响应帧缺可解析 id，无法归属：{trimmed}"
                )),
            );
            break;
        };
        let result = extract_result(&value);
        if sender.send(AcpInbound::Response {
            id: response_id,
            result,
        })
        .is_err()
        {
            // 接收端已销毁（transport 已 drop）：读线程安静退出，调用方已不在。
            break;
        }
    }
}

/// 分发一条入站**请求**：有 handler 用 handler 的应答，无 handler 走 fail-closed。
fn respond_to_inbound_request(
    stdin: &Arc<Mutex<Option<ChildStdin>>>,
    handler: &Arc<Mutex<Option<Arc<dyn AcpInboundRequestHandler>>>>,
    id: Value,
    method: &str,
    params: Value,
) -> Result<(), AcpTransportError> {
    let handler = handler.lock().ok().and_then(|slot| slot.clone());
    let response = match handler {
        Some(handler) => handler.handle(AcpInboundRequest {
            id: id.clone(),
            method: method.to_string(),
            params,
        }),
        None => fail_closed_inbound_response(method),
    };
    write_line(stdin, &inbound_response_payload(&id, response))
}

/// 无 handler 时的 fail-closed 应答（§6 风险 1）：权限请求一律 `cancelled`，其他方法回
/// JSON-RPC `-32601`。任何情况下不得默认 `allow`。
fn fail_closed_inbound_response(method: &str) -> AcpInboundResponse {
    if method == acp_method::SESSION_REQUEST_PERMISSION {
        // 用协议类型构造应答，保证 wire 形状与 A5 冻结契约同源。
        match serde_json::to_value(RequestPermissionResponse {
            outcome: RequestPermissionOutcome::Cancelled,
        }) {
            Ok(value) => AcpInboundResponse::Result(value),
            Err(error) => AcpInboundResponse::Error {
                code: JSONRPC_INTERNAL_ERROR,
                message: format!("构造权限取消应答失败：{error}。"),
                data: None,
            },
        }
    } else {
        AcpInboundResponse::Error {
            code: JSONRPC_METHOD_NOT_FOUND,
            message: format!(
                "acp client 未注入入站请求 handler，方法 `{method}` 不可用（fail-closed）。"
            ),
            data: None,
        }
    }
}

/// 把入站应答编码成 JSON-RPC 响应帧。
fn inbound_response_payload(id: &Value, response: AcpInboundResponse) -> Value {
    match response {
        AcpInboundResponse::Result(result) => {
            json!({ "jsonrpc": "2.0", "id": id, "result": result })
        }
        AcpInboundResponse::Error {
            code,
            message,
            data,
        } => {
            let mut error = serde_json::Map::new();
            error.insert("code".to_string(), json!(code));
            error.insert("message".to_string(), json!(message));
            if let Some(data) = data {
                error.insert("data".to_string(), data);
            }
            json!({ "jsonrpc": "2.0", "id": id, "error": Value::Object(error) })
        }
    }
}

/// 分发一条入站**通知**：不产生应答；未注入 handler 时丢弃并记 stderr 诊断（不 panic）。
fn dispatch_notification(
    notification_handler: &Arc<Mutex<Option<Arc<dyn AcpNotificationHandler>>>>,
    stderr_tail: &Arc<Mutex<Vec<String>>>,
    method: &str,
    params: Value,
) {
    let handler = notification_handler.lock().ok().and_then(|slot| slot.clone());
    match handler {
        Some(handler) => handler.handle_notification(method, params),
        None => push_bounded(
            stderr_tail,
            format!("[acp] 未注入通知 handler，丢弃通知 `{method}`。"),
        ),
    }
}

/// 从响应帧解析 id（数字或数字字符串，与 `plugin_host::transport` 同口径）。
fn response_id(value: &Value) -> Option<u64> {
    match value.get("id") {
        Some(Value::Number(number)) => number.as_u64(),
        Some(Value::String(text)) => text.trim().parse::<u64>().ok(),
        _ => None,
    }
}

/// 把响应帧归约为 `result` 或真源错误（`error` 分支按 JSON-RPC 码分源）。
fn extract_result(value: &Value) -> Result<Value, AcpTransportError> {
    if let Some(error) = value.get("error") {
        let code = error.get("code").and_then(Value::as_i64);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string());
        return Err(if code == Some(JSONRPC_AUTH_REQUIRED) {
            AcpTransportError::AuthRequired(format!(
                "agent 要求先认证（-32000）：{message}。"
            ))
        } else {
            AcpTransportError::Agent(format!(
                "agent 返回 JSON-RPC error（{}）：{message}。",
                code.map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            ))
        });
    }
    value.get("result").cloned().ok_or_else(|| {
        AcpTransportError::Protocol(format!("响应帧既缺 result 也缺 error：{value}"))
    })
}

/// 记录 fail-closed 原因（首个原因优先：后续错误多半是它的级联）。
fn record_fatal(fatal: &Arc<Mutex<Option<AcpTransportError>>>, error: AcpTransportError) {
    if let Ok(mut slot) = fatal.lock() {
        if slot.is_none() {
            *slot = Some(error);
        }
    }
}

/// 有界诊断缓冲：严格保留尾部 [`STDERR_TAIL_LINES`] 行（超限丢最旧）。
fn push_bounded(tail: &Arc<Mutex<Vec<String>>>, line: String) {
    if let Ok(mut tail) = tail.lock() {
        tail.push(line);
        while tail.len() > STDERR_TAIL_LINES {
            tail.remove(0);
        }
    }
}
