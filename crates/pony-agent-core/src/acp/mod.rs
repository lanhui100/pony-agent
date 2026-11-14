//! ACP（Agent Client Protocol）客户端协议层 —— Pony 作为 ACP **Client** 驱动外部 agent。
//!
//! 分层（真源 `docs/architecture/acp-client-integration.md` §3）：
//! - [`protocol`]：ACP v1 类型（serde，camelCase；`protocolVersion` 为整数 1）；
//! - [`transport`]：stdio 双向 JSON-RPC 2.0（出站请求/通知、入站请求分发、stderr 环形缓冲）；
//! - [`config`]：外部 agent 清单（`AcpAgentSpec`），独立文件，不动 provider schema；
//! - [`client`]：单条会话的协议编排（initialize / session/new / prompt / cancel，P1 空桩）；
//! - [`session`]：会话驱动 + update 映射 + stopReason 归约 + ask_control 往返（P1 空桩）。
//!
//! `server`（旧的 agent 侧草稿，协议方向与 v1 相反）已删除；P1 的 `client` / `session`
//! 均为签名空桩（函数体 `unimplemented!()`），由 executor 实现。

pub mod client;
pub mod config;
pub mod protocol;
pub mod session;
pub mod transport;
