//! O2（2026-11-27）：goal 续轮的**唯一判据接线**（`execute_graph_run_stream` /
//! `advance_graph_run` 两个循环共用）。
//!
//! 为什么独立成模块（而不是留在 `mod.rs` 里）：两处循环的判据完全同构，但此前各自内联一份，
//! 于是「谁推进预算、失败轮算不算一轮、待作答要不要让位」三个口径按调用方漂移——这正是
//! `docs/analysis/goal-ask-blocking-audit-2026-11-26.md` 的 O2 缺陷本体。抽到这里后，
//! `mod.rs` 只剩「拿结果、决定是否再起一轮」的骨架，判据真值源**唯一**。
//!
//! 判据分两层，且都来自领域层 `pony_storage::goal`（不在本模块重写任何真值表）：
//! 1. **单轮终态归约** `goal_round_outcome_gate`：成功 ⇒ 推进预算；失败 ⇒ 只计数不写盘；
//!    挂起 ⇒ 暂停（作答后由宿主重入）；取消/未分类 ⇒ 停。
//! 2. **开轮前边界** `goal_round_gate`：无 Armed 目标 / 已取消 / **有待作答 Ask 或审批（让位）** /
//!    连续失败熔断 / 目标预算用尽 / 每线程预算用尽 ⇒ 停。
//!
//! 区分「失败」与「首轮失败」的文案：`goal_round_outcome_gate(Failed)` 产出
//! `FirstRoundNotSucceeded`（「首轮失败，自动续轮未启动」），该文案只对**首轮门**成立。本循环里
//! 已经续过轮（`consecutive_continues > 0`）时改报 `ConsecutiveFailures { failures }`，否则会对
//! 用户说假话。

use crate::agent::control_plane::{graph_run_phase_wire, turn_stream_outcome_from_terminal};
use crate::agent::control_plane::{GraphRunTurnResponse, HostControlPlane};
use crate::agent::goal::{
    self, GoalData, GoalRoundGateDecision, GoalRoundGateInput, GoalRoundOutcome,
    GoalRoundStepOutcome, GoalRoundStopReason,
};
use crate::agent::graph::GraphDecisionKind;
use crate::agent::control_plane::TurnStreamOutcome;
use crate::agent::runtime::TurnInput;

/// 续轮循环的局部计数（连续失败 / 已连续续过的轮数）。
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct ContinuationState {
    pub(crate) consecutive_continues: u32,
    pub(crate) consecutive_failures: u32,
}

impl ContinuationState {
    pub(crate) fn new() -> Self {
        Self::default()
    }
}

/// 一轮之后的结论。
pub(crate) enum Advance {
    /// 停：调用方必须 `return Ok(response)`（停止/暂停原因已上报）。
    Stop,
    /// 继续：调用方开下一轮（输入已按 gate 通过后的快照组装）。
    ///
    /// 装箱理由：`TurnInput` 含历史/图片等大字段，直接内联会让 `Advance` 每个实例都付出该体积
    /// （`clippy::large_enum_variant`）；`Stop` 是本枚举的热路径分支。
    Continue(Box<TurnInput>),
}

/// 把一次单步响应归约为 [`GoalRoundStepOutcome`]。
///
/// 真值来源与 `start_turn_stream_checked` 同源：`turn_stream_outcome_from_terminal`（决策真值优先，
/// 终态 phase 回落）。`TurnResult` 无 `error` 字段，故 error 传 `None`——失败轮的**分类**不依赖
/// error 文本，只依赖相位与决策。
pub(crate) fn step_outcome(response: &GraphRunTurnResponse) -> GoalRoundStepOutcome {
    match turn_stream_outcome_from_terminal(
        graph_run_phase_wire(&response.run.phase),
        None,
        Some(response.decision.kind.clone()),
    ) {
        TurnStreamOutcome::Completed => GoalRoundStepOutcome::Completed,
        TurnStreamOutcome::Failed { .. } => GoalRoundStepOutcome::Failed,
        TurnStreamOutcome::Suspended => GoalRoundStepOutcome::Suspended,
        TurnStreamOutcome::Cancelled => GoalRoundStepOutcome::Cancelled,
        TurnStreamOutcome::Unknown => GoalRoundStepOutcome::Unclassified,
    }
}

/// 一轮之后的判据接线：上报停止原因、推进预算、给出「是否开下一轮」。
///
/// `pending` 是会话内待作答宿主请求分量 `(asks, approvals)`；只在需要开下一轮时才是真值输入
/// （一次性取数即可，避免每次循环多两次读）。
pub(crate) fn after_goal_round<F>(
    response: &GraphRunTurnResponse,
    state: &mut ContinuationState,
    inherited_authorization_mode: Option<&str>,
    pending: F,
) -> Advance
where
    F: FnOnce() -> (usize, usize),
{
    let outcome = step_outcome(response);
    let session_goal = goal::continuation_goal_for_run(response.run.session_id.as_deref());

    if response.decision.kind != GraphDecisionKind::Continue {
        // `Complete` 是图/模型自己收口，不冒充「续轮已停止」；其余（WaitUser / Fail / Cancel /
        // Pause）必须给出可见原因，否则用户只看到「这一轮结束了」。
        if response.decision.kind != GraphDecisionKind::Complete {
            if let Some((session_id, goal)) = session_goal.as_ref() {
                if let Some(reason) = step_stop_reason(
                    outcome,
                    state.consecutive_continues,
                    state.consecutive_failures,
                ) {
                    report_stopped(session_id, goal, &reason, state.consecutive_failures);
                }
            }
        }
        return Advance::Stop;
    }

    let Some((session_id, active_goal)) = session_goal else {
        return Advance::Stop;
    };

    // 本轮终态归约：失败轮**不推进预算**（C2），挂起轮是「暂停」而不是「终止」。
    match goal::goal_round_outcome_gate(outcome) {
        GoalRoundGateDecision::Continue => {
            state.consecutive_failures = 0;
            if let Err(error) =
                goal::advance_goal_round_for_outcome(&active_goal.id, GoalRoundOutcome::Succeeded)
            {
                // 预算推进落盘失败 ⇒ 停（不得让「轮次已跑完但预算没记」演变成无限续轮）。
                log_json(
                    "warn",
                    "goal_round_advance_failed",
                    &[
                        ("stage", serde_json::json!("after_turn")),
                        ("session_id", serde_json::json!(&session_id)),
                        ("goal_id", serde_json::json!(&active_goal.id)),
                        ("error", serde_json::json!(error)),
                    ],
                );
                report_stopped(
                    &session_id,
                    &active_goal,
                    &GoalRoundStopReason::BudgetPersistFailed,
                    state.consecutive_failures,
                );
                return Advance::Stop;
            }
        }
        GoalRoundGateDecision::Pause => {
            report_stopped(
                &session_id,
                &active_goal,
                &GoalRoundStopReason::TurnSuspended,
                state.consecutive_failures,
            );
            return Advance::Stop;
        }
        GoalRoundGateDecision::Stop(reason) => {
            // 失败轮：只计数、不写盘，交由下面的边界 gate 决定是否熔断（文案才能如实报「连续 N
            // 轮失败」）。
            if outcome != GoalRoundStepOutcome::Failed {
                report_stopped(&session_id, &active_goal, &reason, state.consecutive_failures);
                return Advance::Stop;
            }
            state.consecutive_failures = state.consecutive_failures.saturating_add(1);
        }
    }

    // 边界判定走**唯一** gate（含待作答让位 / 连续失败熔断 / 目标预算 / 每线程预算）。
    let Some(updated_goal) = plan_continuation(
        &session_id,
        &active_goal,
        state.consecutive_failures,
        state.consecutive_continues,
        pending(),
    ) else {
        return Advance::Stop;
    };

    state.consecutive_continues = state.consecutive_continues.saturating_add(1);
    Advance::Continue(Box::new(build_continuation_input(
        &session_id,
        &updated_goal,
        updated_goal.rounds_started,
        inherited_authorization_mode,
    )))
}

/// 续轮的 `TurnInput`（下一轮提示 + 继承的授权事实）。
///
/// T7：授权事实继承原会话输入（缺失 ⇒ `read_only`），**绝不**回落为 `None`（= Allow）。
pub(crate) fn build_continuation_input(
    session_id: &str,
    goal: &GoalData,
    round: u32,
    inherited_authorization_mode: Option<&str>,
) -> TurnInput {
    TurnInput {
        message: goal::build_goal_round_message(&goal.objective, round, goal.max_goal_rounds),
        display_message: Some(format!(
            "[goal round {round}] 推进目标：{}",
            goal.objective
        )),
        session_id: Some(session_id.to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_mode: None,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_id: None,
        plan_execution_approval: None,
        tool_authorization_mode: Some(crate::agent::control_plane::continuation_tool_authorization_mode(
            inherited_authorization_mode,
        )),
    }
}

/// 循环内停止原因的**文案真值**（`goal_round_outcome_gate` 的一处必要修正，见模块头注释）。
fn step_stop_reason(
    outcome: GoalRoundStepOutcome,
    consecutive_continues: u32,
    consecutive_failures: u32,
) -> Option<GoalRoundStopReason> {
    if outcome == GoalRoundStepOutcome::Failed && consecutive_continues > 0 {
        return Some(GoalRoundStopReason::ConsecutiveFailures {
            failures: consecutive_failures.saturating_add(1),
        });
    }
    goal::goal_round_outcome_gate(outcome).stop_reason()
}

/// 开下一轮之前的**唯一边界判定**（gate + 预算耗尽落 blocked）。
///
/// 返回 `Some(goal)` ⇒ 允许开下一轮；`None` ⇒ 已上报可见停止原因并落 `blocked`（若因预算耗尽）。
fn plan_continuation(
    session_id: &str,
    goal: &GoalData,
    consecutive_failures: u32,
    consecutive_auto_continues: u32,
    pending: (usize, usize),
) -> Option<GoalData> {
    let decision = goal::goal_round_gate(GoalRoundGateInput {
        has_active_goal: true,
        cancelled: false,
        pending_asks: pending.0,
        pending_approvals: pending.1,
        consecutive_failures,
        rounds_started: goal.rounds_started,
        max_rounds: goal.max_goal_rounds,
        consecutive_auto_continues,
        // 收窄必须显式且可证：`MAX_CONSECUTIVE_AUTO_CONTINUES` 是 `usize`，`as u32` 会触发
        // `as_conversions` + `cast_possible_truncation`；try_from 溢出退化为「预算极大」。
        max_auto_continues: u32::try_from(crate::agent::planner::MAX_CONSECUTIVE_AUTO_CONTINUES)
            .unwrap_or(u32::MAX),
    });
    let reason = decision.stop_reason()?;
    report_stopped(session_id, goal, &reason, consecutive_failures);
    // O5：预算耗尽 ⇒ 落 blocked（否则目标永久 active+Armed：UI 说谎且 `resume` 无解释地拒绝）。
    if matches!(reason, GoalRoundStopReason::BudgetExhausted { .. }) {
        if let Err(error) = goal::block_goal_for_round_limit(&goal.id) {
            log_json(
                "warn",
                "goal_round_block_on_round_limit_failed",
                &[
                    ("session_id", serde_json::json!(session_id)),
                    ("goal_id", serde_json::json!(&goal.id)),
                    ("error", serde_json::json!(error)),
                ],
            );
        }
    }
    None
}

/// 会话内待作答宿主请求的分量计数 `(asks, approvals)`（gate 的让位输入）。
pub(crate) fn pending_host_request_counts(
    plane: &HostControlPlane,
    session_id: &str,
) -> (usize, usize) {
    let asks = plane
        .list_pending_asks(Some(session_id))
        .as_array()
        .map_or(0, Vec::len);
    let approvals = plane
        .list_pending_approvals(session_id)
        .as_array()
        .map_or(0, Vec::len);
    (asks, approvals)
}

/// 续轮停止/暂停的**唯一上报点**（领域事件 + 结构化日志；两端 UI 同源消费）。
fn report_stopped(
    session_id: &str,
    goal: &GoalData,
    reason: &GoalRoundStopReason,
    consecutive_failures: u32,
) {
    log_json(
        "info",
        "goal_rounds_stopped",
        &[
            ("session_id", serde_json::json!(session_id)),
            ("goal_id", serde_json::json!(&goal.id)),
            ("reason", serde_json::json!(reason.code())),
            (
                "consecutive_failures",
                serde_json::json!(consecutive_failures),
            ),
            ("rounds_started", serde_json::json!(goal.rounds_started)),
            ("max_rounds", serde_json::json!(goal.max_goal_rounds)),
        ],
    );
    goal::notify_goal_rounds_stopped(
        session_id,
        &goal.id,
        reason,
        consecutive_failures,
        goal.rounds_started,
        goal.max_goal_rounds,
    );
}

fn log_json(level: &str, event: &str, fields: &[(&str, serde_json::Value)]) {
    crate::agent::otel::log::log_json_with_fields(level, event, None, fields);
}
