//! ACP 子活动 → 父 turn 事件的归集（wave-acp-delegate C1.1–C1.5）。
//!
//! 真源：`.dev-team/contract-matrix-acp-delegate-p0p2.md` C1.1–C1.5 与「设计裁决」——
//! **不使用**「把 live sink 塞进 `ToolExecutionContext`」方案，改用**父 turn 事件持久化通道**：
//! 外部 agent 的 `tool_call` / `tool_call_update` 被转成 [`TurnToolActivity`]，经
//! `crate::agent::turn_flow::emit_external_turn_event` 以**父 session/turn id** 发 `turn:tool`
//! 并落盘（live UI 渲染不在本期范围，Non-Goal 4）。
//!
//! 契约（实现已落地）：
//! - C1.1：归集事件的 `parent_activity_id == acp_delegate call_id`，事件 `session_id`/`turn_id`
//!   为**父**会话/父 turn，并落入 `turn_events`；
//! - C1.3：无父上下文或持久化通道未注册 ⇒ 返回 `false` 静默跳过（不 panic、不新增必填构造参数）；
//! - C1.4：状态词表由 [`map_acp_tool_status`] 单一映射（`pending|in_progress→running`、
//!   `completed→done`、`failed→failed`、`cancelled→aborted`），保证父 turn 事件被判为
//!   ToolCall/ToolResult；
//! - C1.5：`turn_id` 不得为 ACP session id，`parent_activity_id` 不得为空。

use crate::acp::protocol::ToolCallStatus;
use crate::agent::runtime::TurnStreamEvent;
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

impl ParentTurnRef {
    /// 三要素是否齐备（任一为空 ⇒ 视同无父上下文，C1.3 静默跳过）。
    fn is_complete(&self) -> bool {
        !self.session_id.trim().is_empty()
            && !self.turn_id.trim().is_empty()
            && !self.call_id.trim().is_empty()
    }
}

/// 把一条 ACP 工具活动归集为父 turn 的 `turn:tool` 事件（C1.1）。
///
/// 返回 `true` 表示已交给父 turn 通道；`false` 表示无父上下文 / 通道未注册而静默跳过（C1.3）。
pub(crate) fn emit_parent_tool_activity(
    parent: &ParentTurnRef,
    activity: &TurnToolActivity,
) -> bool {
    if !parent.is_complete() {
        // C1.3：父上下文不完整 ⇒ 不发射、不 panic、不落半截事件。
        return false;
    }
    let mut activity = activity.clone();
    // C1.1/C1.5：子活动的父活动 id 恒为本次 `acp_delegate` 的 call_id，绝不为空。
    activity.parent_activity_id = Some(parent.call_id.clone());
    let payload = parent_tool_event(parent, activity);
    crate::agent::turn_flow::emit_external_turn_event(
        &parent.session_id,
        &parent.turn_id,
        "turn:tool",
        payload,
    )
}

/// 父 turn 身份的 `turn:tool` 载荷骨架（`session_id`/`turn_id` 一律取父 turn 真值）。
///
/// `step` 留空（None ≡ 0）：ACP 子活动没有父 turn 的 hop 索引（C4.1「ACP 子活动 step=0」）。
fn parent_tool_event(parent: &ParentTurnRef, activity: TurnToolActivity) -> TurnStreamEvent {
    TurnStreamEvent {
        event_id: None,
        session_id: Some(parent.session_id.clone()),
        turn_id: parent.turn_id.clone(),
        trace_id: crate::agent::otel::exported_trace_id(),
        cost_usd: None,
        kind: "tool".to_string(),
        event_type: None,
        event_version: None,
        sequence: None,
        emitted_at_ms: None,
        phase: Some("executing_tool".to_string()),
        text: None,
        reasoning_content: None,
        error: None,
        provider_requested_name: None,
        provider_name: None,
        provider_protocol: None,
        provider_model: None,
        provider_source: None,
        provider_mode: None,
        fallback_reason: None,
        build_context_observation: None,
        input_tokens: None,
        cache_hit_input_tokens: None,
        reasoning_tokens: None,
        output_tokens: None,
        total_tokens: None,
        first_token_latency_ms: None,
        turn_duration_ms: None,
        trace_steps: None,
        trace_timeline: None,
        tool_activities: Some(vec![activity]),
        provider_call_records: None,
        hook_trace_records: None,
        session_summary: None,
        step: None,
        stop_reason: None,
        metrics: None,
        error_class: None,
    }
}

/// ACP `ToolCallStatus` → 运行时活动状态词表（C1.4 单一真源）。
///
/// 词表刻意与父 turn 的运行期活动一致（`running`/`done`/`failed`/`aborted`），
/// 因为父 turn 事件按 `status == "running"` 判 ToolCall、其余判 ToolResult，
/// 而 ACP 的 `pending|in_progress|completed|failed|cancelled` 是另一套词表。
/// `None`（ACP 未给状态）按进行中处理 ⇒ `running`（草稿态工具调用必须先落 ToolCall）。
pub(crate) fn map_acp_tool_status(status: Option<&ToolCallStatus>) -> &'static str {
    match status {
        None | Some(ToolCallStatus::Pending) | Some(ToolCallStatus::InProgress) => "running",
        Some(ToolCallStatus::Completed) => "done",
        Some(ToolCallStatus::Failed) => "failed",
        Some(ToolCallStatus::Cancelled) => "aborted",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parent() -> ParentTurnRef {
        ParentTurnRef {
            session_id: "sess-parent".to_string(),
            turn_id: "turn-parent".to_string(),
            call_id: "call-acp-delegate".to_string(),
        }
    }

    fn activity() -> TurnToolActivity {
        TurnToolActivity {
            id: "acp-tool-1".to_string(),
            name: "edit a.rs".to_string(),
            canonical_tool_name: None,
            display_name_zh: Some("外部代理工具".to_string()),
            status: "running".to_string(),
            description: "edit a.rs（edit）".to_string(),
            arguments_text: None,
            result_text: None,
            duration_seconds: None,
            parent_activity_id: None,
            artifacts: None,
            error: None,
            capability_invocation: None,
        }
    }

    /// C1.4：状态词表单一真源。
    #[test]
    fn map_acp_tool_status_covers_full_vocabulary() {
        assert_eq!(map_acp_tool_status(None), "running");
        assert_eq!(
            map_acp_tool_status(Some(&ToolCallStatus::Pending)),
            "running"
        );
        assert_eq!(
            map_acp_tool_status(Some(&ToolCallStatus::InProgress)),
            "running"
        );
        assert_eq!(
            map_acp_tool_status(Some(&ToolCallStatus::Completed)),
            "done"
        );
        assert_eq!(map_acp_tool_status(Some(&ToolCallStatus::Failed)), "failed");
        assert_eq!(
            map_acp_tool_status(Some(&ToolCallStatus::Cancelled)),
            "aborted"
        );
    }

    /// C1.3 / C1.5：父身份不完整 ⇒ 静默跳过（绝不 panic、绝不发半截事件）。
    #[test]
    fn emit_parent_tool_activity_skips_incomplete_parent() {
        let mut incomplete = parent();
        incomplete.turn_id = String::new();
        assert!(!emit_parent_tool_activity(&incomplete, &activity()));
    }

    /// C1.1：完整父上下文 ⇒ 载荷以父 turn 身份承载子活动，且不改动入参活动。
    #[test]
    fn parent_tool_event_uses_parent_identity_and_child_parent_id() {
        let parent = parent();
        let mut child = activity();
        child.parent_activity_id = Some("call-acp-delegate".to_string());
        let payload = parent_tool_event(&parent, child);
        assert_eq!(payload.kind, "tool");
        assert_eq!(payload.session_id.as_deref(), Some("sess-parent"));
        assert_eq!(payload.turn_id, "turn-parent");
        assert_eq!(payload.step, None, "ACP 子活动 step 未知（None ≡ 0）");
        assert_eq!(payload.sequence, None);
        let activities = payload.tool_activities.expect("必须携带子活动");
        assert_eq!(activities.len(), 1);
        assert_eq!(
            activities[0].parent_activity_id.as_deref(),
            Some("call-acp-delegate")
        );
    }
}
