//! ACP（Agent Client Protocol）客户端协议层 —— Pony 作为 ACP **Client** 驱动外部 agent。
//!
//! 分层（真源 `docs/architecture/acp-client-integration.md` §3）：
//! - [`protocol`]：ACP v1 类型（serde，camelCase；`protocolVersion` 为整数 1）；
//! - [`transport`]：stdio 双向 JSON-RPC 2.0（出站请求/通知、入站请求分发、stderr 环形缓冲）；
//! - [`config`]：外部 agent 清单（`AcpAgentSpec`），独立文件，不动 provider schema；
//! - [`client`]：单条会话的协议编排（initialize / session/new / prompt / cancel）；
//! - [`session`]：会话驱动 + update 映射 + stopReason 归约 + ask_control 往返；
//! - [`handlers`]：反向请求宿主实现骨架（`WorkspaceFsHandler` / `ManagedTerminalHandler`，B175）。
//!
//! `server`（旧的 agent 侧草稿，协议方向与 v1 相反）已删除。`client` / `session` 已由 executor
//! 实现；`handlers` 仍是签名骨架（函数体 `unimplemented!()`），由 executor 落地。

pub mod client;
pub mod config;
/// wave-acp-delegate C1：ACP 子活动 → 父 turn 事件归集（W1 签名骨架，实现归 W2a）。
pub(crate) mod delegate_events;
pub mod handlers;
/// ACP 宿主桥（B163）：Tauri 命令 / 前端用的薄入口（list / run / cancel）。
pub mod host;
/// ACP agent 子进程树回收（B165）：Windows 用 Job Object 纳管孙进程。
pub(crate) mod process_tree;
pub mod protocol;
/// 活跃 ACP 委派注册表（进程内只读可见面；见模块文档）。
pub mod registry;
/// wave-acp-delegate C0：`acp_delegate` 最终正文归约（W1 签名骨架，实现归 W2a）。
pub(crate) mod result;
pub mod session;
pub mod transport;

use std::time::Duration;

/// 外部 ACP agent **一轮委派的默认整轮预算**（单一真源）。
///
/// 消费方（禁止在别处再写字面量 120s）：
/// - `agent/tools/names.rs`：`acp_delegate` 的 `ToolExecutionPolicy.default_timeout_ms`；
/// - `bin/cli.rs`：`ponycli acp` 的 `--timeout-secs` 缺省值；
/// - `agent/tools/orchestration.rs`：`acp_delegate` 工具入口缺省预算。
///
/// 语义：这是**整轮**（一次 `session/prompt` 到 agent 收尾）的预算，不是单请求 5s
/// （`transport::ACP_REQUEST_TIMEOUT`）——缺省预算不得回落到 5s 硬顶。
pub const ACP_DEFAULT_TURN_BUDGET: Duration = Duration::from_secs(120);
