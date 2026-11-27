//! O1-B（2026-11-27）：桌面端「宿主作答后重入 goal 续轮」的唯一驱动者。
//!
//! 背景（`docs/analysis/goal-ask-blocking-audit-2026-11-26.md` §1.3 / P0-1）：本仓把「人类输入」
//! 建模成 turn 终态（`ask_user` ⇒ `WaitingHost` ⇒ `suspended`）。桌面端旧实现里
//! `ask_answer` / `graph_resume_ask` 之后只把 run 放回 `Ready`，**没有任何线程会再发下一轮**——
//! 用户答完界面什么都不动，目标永久停在 `active + Armed` 而横幅仍显示「进行中」。
//!
//! 本模块补上这个驱动者，且**不依赖前端 auto-continue**：
//! 1. 判定走领域层唯一 gate（`pony_agent_core::agent::goal::goal_round_gate`）：仍有 Armed 目标、
//!    无待作答 Ask/审批、目标预算/每线程预算未用尽 ⇒ 放行；
//! 2. 放行后以 `build_goal_round_message` 组装本轮提示，`prepare_continue_graph_run_stream` +
//!    `spawn_graph_run_stream`（后者的 `execute_graph_run_stream` 内已含 gate 判据的续轮循环，
//!    连续失败熔断与「失败轮不烧预算」因此与 TUI 同源）；
//! 3. 不放行时**必须**留下可见原因：预算耗尽 ⇒ 落 `blocked(round_limit)`；其余 ⇒
//!    领域事件 `RoundsStopped`（前端由 `goal://rounds_stopped` 消费）。
//!
//! 授权事实：goal 续轮的 `TurnInput` 是授权事实唯一载体（T7）。桌面端原轮的授权模式不在 run
//! 上持久化，故由宿主在每次 `*_graph_run_stream` 命令入口登记到 [`SessionAuthorizationModes`]，
//! 这里继承；**无记录时 fail-closed 取 `read_only`**，绝不回落 Allow。

use pony_agent_core::agent::control_plane::{
    ContinueGraphRunStreamCommand, GraphRunQuery, HostControlPlane,
};
use pony_agent_core::agent::goal::{
    self, GoalRoundGateDecision, GoalRoundGateInput, GoalRoundStopReason,
};
use pony_agent_core::agent::planner::MAX_CONSECUTIVE_AUTO_CONTINUES;
use pony_agent_core::agent::runtime::TurnInput;
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::AppHandle;

/// 会话 → 最近一次提交的授权模式（进程内，fail-closed 缺省 `read_only`）。
#[derive(Default)]
pub(crate) struct SessionAuthorizationModes {
    modes: Mutex<HashMap<String, String>>,
}

impl SessionAuthorizationModes {
    /// 登记一次提交的授权模式（每个 `*_graph_run_stream` 命令入口调用）。
    pub(crate) fn record(&self, session_id: Option<&str>, mode: Option<&str>) {
        let Some(session) = session_id.map(str::trim).filter(|s| !s.is_empty()) else {
            return;
        };
        let normalized = match mode.map(str::trim) {
            Some("read_only") => "read_only",
            Some("workspace_write") => "workspace_write",
            Some("full_access") => "full_access",
            // 非法/缺失：不登记，读取侧自然回落到 read_only（fail-closed）。
            _ => return,
        };
        let mut modes = self.modes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        modes.insert(session.to_string(), normalized.to_string());
    }

    /// 取该会话最近一次的授权模式；**无记录 ⇒ `read_only`**（绝不回落 Allow）。
    pub(crate) fn mode_for(&self, session_id: &str) -> String {
        let modes = self.modes.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        modes
            .get(session_id.trim())
            .cloned()
            .unwrap_or_else(|| "read_only".to_string())
    }
}

/// Ask / 审批作答成功后尝试把 goal 续轮重新接上。
///
/// 唯一调用点：`graph_resume_ask`（Ask 答案落地）与 `control_approve_request`（审批决策）。
/// 返回 `None` 表示无需/不能续轮（原因已按契约可见或无需告知）。
pub(crate) fn resume_goal_rounds_after_host_answer(
    app: &AppHandle,
    control_plane: &HostControlPlane,
    run_id: &str,
    authorization_modes: &SessionAuthorizationModes,
) -> Result<(), String> {
    let run = control_plane
        .load_graph_run(GraphRunQuery {
            run_id: Some(run_id.to_string()),
        })
        .ok_or_else(|| format!("graph run `{run_id}` not found for goal continuation"))?;
    let Some(session_id) = run
        .session_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Ok(());
    };
    let Some(active_goal) = goal::active_goal_for_session(session_id) else {
        // 无 Armed 目标：本轮作答只服务这一轮，没有可续的目标。
        return Ok(());
    };

    let pending_asks = control_plane
        .list_pending_asks(Some(session_id))
        .as_array()
        .map_or(0, Vec::len);
    let pending_approvals = control_plane
        .list_pending_approvals(session_id)
        .as_array()
        .map_or(0, Vec::len);

    // 驱动者自己的循环从 0 开始计数：连续失败/每线程预算由 `execute_graph_run_stream` 的循环接续。
    let decision = goal::goal_round_gate(GoalRoundGateInput {
        has_active_goal: true,
        cancelled: false,
        pending_asks,
        pending_approvals,
        consecutive_failures: 0,
        rounds_started: active_goal.rounds_started,
        max_rounds: active_goal.max_goal_rounds,
        consecutive_auto_continues: 0,
        // 收窄显式且可证：`as u32` 同时触发 `as_conversions` 与 `cast_possible_truncation`；
        // try_from 溢出（理论上不可能）退化为「预算极大」，不会多开轮次。
        max_auto_continues: u32::try_from(MAX_CONSECUTIVE_AUTO_CONTINUES).unwrap_or(u32::MAX),
    });

    if let Some(reason) = decision.stop_reason() {
        // 可见原因：领域事件（`goal://rounds_stopped`）+ 预算耗尽落 blocked。
        report_goal_rounds_stopped(session_id, &active_goal, &reason);
        if matches!(reason, GoalRoundStopReason::BudgetExhausted { .. }) {
            if let Err(error) = goal::block_goal_for_round_limit(&active_goal.id) {
                log_continuation_failure("block_on_round_limit", session_id, &error);
            }
        }
        return Ok(());
    }
    if decision != GoalRoundGateDecision::Continue {
        return Ok(());
    }

    let round = active_goal.rounds_started.saturating_add(1);
    let input = TurnInput {
        message: goal::build_goal_round_message(
            &active_goal.objective,
            round,
            active_goal.max_goal_rounds,
        ),
        display_message: Some(format!(
            "[goal round {round}] 推进目标：{}",
            active_goal.objective
        )),
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        tool_authorization_mode: Some(authorization_modes.mode_for(session_id)),
        session_id: Some(session_id.to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
    };
    let turn_id = format!("{run_id}-goal-{round}-{}", now_ms());
    let (_, prepared) = control_plane.prepare_continue_graph_run_stream(
        ContinueGraphRunStreamCommand {
            turn_id,
            run_id: run_id.to_string(),
            input,
        },
    )?;
    crate::tauri_adapter::register_turn_sink(app, &prepared.turn_id);
    crate::tauri_adapter::spawn_graph_run_stream(app, prepared)
}

fn report_goal_rounds_stopped(
    session_id: &str,
    goal: &pony_agent_core::agent::goal::GoalData,
    reason: &GoalRoundStopReason,
) {
    pony_agent_core::agent::otel::log::log_json_with_fields(
        "info",
        "goal_rounds_stopped",
        None,
        &[
            ("session_id", serde_json::json!(session_id)),
            ("goal_id", serde_json::json!(&goal.id)),
            ("reason", serde_json::json!(reason.code())),
            ("stage", serde_json::json!("desktop_host_answer")),
            ("rounds_started", serde_json::json!(goal.rounds_started)),
            ("max_rounds", serde_json::json!(goal.max_goal_rounds)),
        ],
    );
    goal::notify_goal_rounds_stopped(
        session_id,
        &goal.id,
        reason,
        0,
        goal.rounds_started,
        goal.max_goal_rounds,
    );
}

fn log_continuation_failure(stage: &str, session_id: &str, error: &str) {
    pony_agent_core::agent::otel::log::log_json_with_fields(
        "warn",
        "goal_round_continuation_failed",
        None,
        &[
            ("stage", serde_json::json!(stage)),
            ("session_id", serde_json::json!(session_id)),
            ("error", serde_json::json!(error)),
        ],
    );
}

fn now_ms() -> u64 {
    // 毫秒戳理论上不会溢出 u64；`as u64` 会触发 `as_conversions` / `cast_possible_truncation`，
    // 故用 try_from 并在溢出（不可能）时退化为 0——只用于 turn_id 去重后缀，不承载语义。
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authorization_mode_falls_back_to_read_only_without_record() {
        let modes = SessionAuthorizationModes::default();
        assert_eq!(modes.mode_for("session-unknown"), "read_only");
    }

    #[test]
    fn authorization_mode_records_and_normalizes() {
        let modes = SessionAuthorizationModes::default();
        modes.record(Some("session-1"), Some("workspace_write"));
        assert_eq!(modes.mode_for("session-1"), "workspace_write");
        // 非法值不登记（不能把未知串当授权），旧值保持不变。
        modes.record(Some("session-1"), Some("admin"));
        assert_eq!(modes.mode_for("session-1"), "workspace_write");
        // 空会话 id 不登记。
        modes.record(None, Some("full_access"));
        modes.record(Some("   "), Some("full_access"));
        assert_eq!(modes.mode_for(""), "read_only");
    }
}
