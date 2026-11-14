//! ACP（Agent Client Protocol）客户端协议层 —— Pony 作为 ACP **Client** 驱动外部 agent。
//!
//! 分层（真源 `docs/architecture/acp-client-integration.md` §3）：
//! - [`protocol`]：ACP v1 类型（serde，camelCase；`protocolVersion` 为整数 1）；
//! - [`transport`]：stdio 双向 JSON-RPC 2.0（出站请求/通知、入站请求分发、stderr 环形缓冲）；
//! - [`config`]：外部 agent 清单（`AcpAgentSpec`），独立文件，不动 provider schema。
//!
//! P0 只冻结契约：`transport` / `config` 的函数体一律 `unimplemented!()`，由 P1 实现。
//! `server`（旧的 agent 侧草稿，协议方向与 v1 相反）已删除；P1 才新增 `client` / `session`。

pub mod config;
pub mod protocol;
pub mod transport;
