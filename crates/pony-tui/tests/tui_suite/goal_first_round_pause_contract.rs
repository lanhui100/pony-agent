//! 契约 C5（O1-B）红相验收：首轮门 `Suspended` 的可见原因必须是「暂停等你作答」。
//!
//! 契约真值源：`.dev-team/plan-goal-gate-v1.md` §0 C5 第三条（本文件由独立 Test Agent 冻结，
//! Executor 只读，禁改断言迁就实现）。
//!
//! 要防的回归（`docs/analysis/goal-ask-blocking-audit-2026-11-26.md` §1.2 / P0-1）：
//! 首轮门把 `TurnStreamOutcome::Suspended` 直接判成停止，文案走
//! `GoalRoundStopReason::TurnSuspended` 的旧 headline「目标已暂停：续轮已停止」+ hint
//! 「先回答上面的提问」⇒ 用户读到的结论是「目标死了」，于是不去作答，也就永远不会重入。
//!
//! 修复口径：`first_round_stop_reason` 委托 `goal_round_outcome_gate`
//! （Suspended ⇒ Pause ⇒ `TurnSuspended`），且 `TurnSuspended` 的可见文案必须表达
//! 「等你作答」+「作答后自动继续」；`Completed` 仍返回 `None`（进入续轮循环）。
//!
//! 铁律：每条用例真实断言；无 `#[ignore]` / skip / 同义反复断言；不修改 `src/`。

use pony_agent_core::agent::control_plane::TurnStreamOutcome;
use pony_agent_core::agent::goal::GoalRoundStopReason;
use pony_tui::first_round_stop_reason;

/// 首轮 `Suspended` ⇒ `Some(TurnSuspended)`（Pause 的唯一可见原因）。
///
/// 这一条同时锁住「委托 `goal_round_outcome_gate`」：若 TUI 侧仍自行拼装
/// （例如新增 `Pause` 专属变体或改用 `FirstRoundNotSucceeded`），本断言即红。
#[test]
fn c5_suspended_first_round_maps_to_turn_suspended() {
    // Arrange
    let outcome = TurnStreamOutcome::Suspended;

    // Act
    let reason = first_round_stop_reason(&outcome);

    // Assert
    assert_eq!(
        reason,
        Some(GoalRoundStopReason::TurnSuspended),
        "契约 C5 违约：首轮 Suspended 必须映射 TurnSuspended（Pause 的可见原因），实际 {reason:?}"
    );
}

/// `Suspended` 的可见文案必须表达「暂停/等你作答」，且**不得**读成「首轮失败」。
///
/// 反例（修复前）：「目标已暂停：续轮已停止｜本轮在等你作答/审批｜先回答上面的提问」
/// —— 首段就说「续轮已停止」，用户不会意识到「作答即自动继续」。
#[test]
fn c5_suspended_copy_says_paused_waiting_for_answer_not_failure() {
    // Arrange
    let outcome = TurnStreamOutcome::Suspended;

    // Act
    let reason = first_round_stop_reason(&outcome)
        .expect("契约 C5 违约：Suspended 必须给出停止原因（Pause）");
    let line = reason.visible_line();

    // Assert：必须说「等你作答」（作答是唯一解锁动作）
    assert!(
        line.contains("等你作答"),
        "契约 C5 违约：文案必须表达「等你作答」，实际：{line}"
    );
    // Assert：必须承诺「作答后自动继续」（否则用户以为目标已死）
    assert!(
        line.contains("作答后自动继续"),
        "契约 C5 违约：文案必须承诺「作答后自动继续」，实际：{line}"
    );
    // Assert：必须读作「暂停」而不是「失败/未成功」
    assert!(
        line.contains("暂停"),
        "契约 C5 违约：文案必须表达「暂停」（而非终止），实际：{line}"
    );
    assert!(
        !line.contains("首轮失败"),
        "契约 C5 违约：挂起不是失败，不得读成「首轮失败」，实际：{line}"
    );
    assert!(
        !line.contains("未成功"),
        "契约 C5 违约：挂起不是「未成功」，实际：{line}"
    );
}

/// 首轮 `Completed` ⇒ `None`（进入 `run_goal_rounds`，不得伪造停止原因）。
#[test]
fn c5_completed_first_round_enters_round_loop() {
    // Arrange
    let outcome = TurnStreamOutcome::Completed;

    // Act
    let reason = first_round_stop_reason(&outcome);

    // Assert
    assert_eq!(
        reason, None,
        "契约 C5 违约：首轮 Completed 必须返回 None（进入续轮循环），实际 {reason:?}"
    );
}

/// 反向钉子：`Failed` 仍映射 `FirstRoundNotSucceeded`（Pause 改造不得吞掉真失败）。
#[test]
fn c5_failed_first_round_still_reports_first_round_not_succeeded() {
    // Arrange
    let outcome = TurnStreamOutcome::Failed {
        error: Some("provider SendRequest".to_string()),
    };

    // Act
    let reason = first_round_stop_reason(&outcome);

    // Assert
    assert_eq!(
        reason,
        Some(GoalRoundStopReason::FirstRoundNotSucceeded),
        "契约 C5 违约：真失败必须仍报 FirstRoundNotSucceeded（不得被 Pause 文案吞掉）"
    );
}
