//! ACP stdio 传输层（newline-delimited JSON-RPC 2.0）。
//!
//! 分工（真源 §3/§3.2）：本层只做报文收发与配对，不做会话语义：
//! - 出站：`request`（id 关联 + deadline）/ `notify`（无 id）；
//! - 入站：reader 线程按「`id`+`method` = 请求 / `method` 无 `id` = 通知 / `id` 无 `method` = 响应」
//!   三路分流，请求交给 [`AcpInboundRequestHandler`] 应答（无 handler ⇒ fail-closed 回错误，
//!   绝不默认放行权限请求）；
//! - 进程：env 白名单 + Unix 独立进程组 + stderr 环形缓冲（尾部 32 行，崩溃诊断用），
//!   模式对齐 `agent/plugin_host/transport.rs`。
//!
//! P0 只冻结契约：所有函数体 `unimplemented!()`，由 P1 Executor 实现；本层**不**含任何
//! 会话 / 审批 / 事件映射逻辑（那是 `acp::session`，P1）。

// P0 空桩：函数体统一 `unimplemented!()`，形参「未使用」属预期；P1 实现时移除本允许。
#![allow(unused_variables)]

use std::path::Path;
use std::process::{Child, ChildStdin};
use std::sync::atomic::{AtomicU32, AtomicU64};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use crate::acp::config::AcpAgentSpec;

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
    /// 子进程启动失败（命令不存在 / 权限不足 / ETXTBSY 重试耗尽）。
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
        unimplemented!("P1：错误展示文案")
    }
}

impl std::error::Error for AcpTransportError {}

impl AcpTransportError {
    /// 稳定的机器可读错误码：
    /// `acp_spawn_failed` / `acp_initialize_failed` / `acp_protocol_error` / `acp_agent_error`
    /// / `acp_agent_refused` / `acp_auth_required` / `acp_timeout` / `acp_unavailable`。
    pub fn code(&self) -> &'static str {
        unimplemented!("P1：错误码映射（8 个 acp_* 常量，见本方法文档）")
    }
}

/// 一个已 spawn 的 ACP agent 子进程 + stdio JSON-RPC 通道。
#[allow(dead_code)]
pub struct AcpTransport {
    child: Child,
    /// `Option` 便于 `close()` 时 take 并关闭（EOF 让 agent 自行退出）。
    stdin: Mutex<Option<ChildStdin>>,
    /// reader 线程上行的入站帧。
    inbound: Mutex<Receiver<AcpInbound>>,
    /// 入站请求分发器（未注入 = fail-closed）。
    handler: Mutex<Option<Arc<dyn AcpInboundRequestHandler>>>,
    /// 入站通知分发器（未注入 = 丢弃 + stderr 诊断）。
    notification_handler: Mutex<Option<Arc<dyn AcpNotificationHandler>>>,
    /// stderr 尾部环形缓冲（诊断证据）。
    stderr_tail: Arc<Mutex<Vec<String>>>,
    /// 出站请求 id 分配器。
    next_id: AtomicU64,
    /// stdout 连续非法 JSON 行计数（达 [`MALFORMED_LINE_STREAK_LIMIT`] fail-closed）。
    malformed_streak: AtomicU32,
    /// 已发出终止信号（幂等标记）。
    terminated: Mutex<bool>,
}

impl AcpTransport {
    /// 启动外部 ACP agent（P1 实现）。
    ///
    /// 隔离面在此收口：`env_clear()` + 白名单（复用
    /// `agent::plugin_host::capability::apply_child_env`）、`current_dir = cwd`（必须是
    /// canonicalize 过的绝对路径）、Unix `process_group(0)`。
    pub fn spawn(
        spec: &AcpAgentSpec,
        cwd: &Path,
        extra_env: &[(String, String)],
    ) -> Result<Self, AcpTransportError> {
        unimplemented!("P1：Command + env 白名单 + 进程组 + stdout/stderr 读线程")
    }

    /// 发送请求并按 `deadline` 等待同 id 响应，返回 `result`（P1 实现）。
    ///
    /// 同一 transport 上的请求串行化；迟到响应有界暂存；入站请求先分流再按 id 匹配。
    pub fn request(
        &self,
        method: &str,
        params: Value,
        deadline: Duration,
    ) -> Result<Value, AcpTransportError> {
        unimplemented!("P1：写行 + 按 id 收包 + deadline + 入站请求分发")
    }

    /// 发送通知（无 id、不等待响应），如 `session/cancel`（P1 实现）。
    pub fn notify(&self, method: &str, params: Value) -> Result<(), AcpTransportError> {
        unimplemented!("P1：写一行 JSON-RPC notification")
    }

    /// 注入 / 替换入站请求分发器（P1 实现）。
    pub fn set_handler(&mut self, handler: Arc<dyn AcpInboundRequestHandler>) {
        unimplemented!("P1：写入 handler 槽位")
    }

    /// 注入 / 替换入站通知分发器（P1 实现）。
    ///
    /// 未注入时 reader 线程对 `session/update` 等通知只记 stderr 诊断并丢弃，不得 panic。
    pub fn set_notification_handler(&self, handler: Arc<dyn AcpNotificationHandler>) {
        unimplemented!("P1：写入通知处理槽位")
    }

    /// 关闭并回收子进程（幂等；P1 实现）。
    ///
    /// 顺序：关 stdin → 宽限 [`SPAWN_REAP_GRACE`] → Unix 进程组 SIGKILL → `kill()` + `wait()`。
    pub fn close(&mut self) {
        unimplemented!("P1：关 stdin + 宽限 + SIGKILL 进程组 + wait 回收")
    }

    /// stderr 尾部若干行（诊断用；P1 实现）。
    pub fn stderr_tail(&self) -> Vec<String> {
        unimplemented!("P1：克隆 stderr 环形缓冲")
    }

    /// 子进程 pid（P1 实现）。
    pub fn pid(&self) -> u32 {
        unimplemented!("P1：child.id()")
    }
}
