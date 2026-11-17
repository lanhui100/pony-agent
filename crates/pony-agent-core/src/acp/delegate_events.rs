//! ACP 子活动 → 父 turn 事件的归集（wave-acp-delegate C1.1–C1.5，**W1 签名骨架**）。
//!
//! 真源：`.dev-team/contract-matrix-acp-delegate-p0p2.md` C1.1–C1.5 与「设计裁决」——
//! **不使用**「把 live sink 塞进 `ToolExecutionContext`」方案，改用**父 turn 事件持久化通道**：
//! 外部 agent 的 `tool_call` / `tool_call_update` 被转成 [`TurnToolActivity`]，经
//! `crate::agent::turn_flow::emit_external_turn_event` 以**父 session/turn id** 发 `turn:tool`
//! 并落盘（live UI 渲染不在本期范围，Non-Goal 4）。
//!
//! 本文件在 W1 只声明类型 / 函数签名，函数体为 `unimplemented!()`；实现归 W2a。
//!
//! 实现侧必须满足的契约（此处仅冻结形状）：
//! - C1.1：归集事件的 `parent_activity_id == acp_delegate call_id`，事件 `session_id`/`turn_id`
//!   为**父**会话/父 turn，并落入 `turn_events`；
//! - C1.3：无父上下文或持久化通道未注册 ⇒ 返回 `false` 静默跳过（不 panic、不新增必填构造参数）；
//! - C1.4：状态词表由 [`map_acp_tool_status`] 单一映射（`pending|in_progress→running`、
//!   `completed→done`、`failed→failed`、`cancelled→aborted`），保证父 turn 事件被判为
//!   ToolCall/ToolResult；
//! - C1.5：`turn_id` 不得为 ACP session id，`parent_activity_id` 不得为空。
//!
//! 空桩期（W1）显式放行 `dead_code`：消费者在 W2a 才接入，而发版构建带 `-D warnings`；
//! 实现落地后该放行随「函数被调用」自然失效（不是永久豁免）。
#![allow(dead_code)]

use crate::acp::protocol::ToolCallStatus;
use crate::agent::telemetry::TurnToolActivity;

/// 父 turn 身份三要素（归集事件的归属真源，C1.1 / C1.5）。
#[derive(Debug, Clone)]
pub(crate) struct ParentTurnRef {
    /// 父会话 id（**不得**写成 ACP session id）。
    pub session_id: String,
    /// 父 turn id（不得为空）。
    pub turn_id: String,
    /// 触发本次委派的 `acp_delegate` 调用 id（子活动的 `parent_activity_id`）。
    pub call_id: String,
}

/// 把一条 ACP 工具活动归集为父 turn 的 `turn:tool` 事件（签名冻结；实现归 W2a）。
///
/// 返回 `true` 表示已交给父 turn 通道；`false` 表示无父上下文 / 通道未注册而静默跳过（C1.3）。
#[allow(unused_variables)]
pub(crate) fn emit_parent_tool_activity(
    parent: &ParentTurnRef,
    activity: &TurnToolActivity,
) -> bool {
    unimplemented!()
}

/// ACP `ToolCallStatus` → 运行时活动状态词表（C1.4 单一真源；签名冻结）。
#[allow(unused_variables)]
pub(crate) fn map_acp_tool_status(status: Option<&ToolCallStatus>) -> &'static str {
    unimplemented!()
}
