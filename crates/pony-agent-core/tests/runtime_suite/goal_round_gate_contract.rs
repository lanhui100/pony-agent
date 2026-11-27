//! 契约 C1（O2）红相验收：goal 续轮**单一 gate** 的冻结真值表。
//!
//! 契约真值源：`.dev-team/plan-goal-gate-v1.md` §0 C1（本文件由独立 Test Agent 冻结，
//! Executor 只读，禁改断言迁就实现）。
//!
//! 要防的回归（`docs/analysis/goal-ask-blocking-audit-2026-11-26.md`）：
//! - P0-2：TUI 与 core 两套续轮判据能力不对等（待作答让位 / 连续失败熔断 / 预算耗尽
//!   只有 TUI 有）⇒ 必须收敛到**同一个** `goal_round_gate`；
//! - P0-1：`ask_user` 挂起后目标永久失活 ⇒ 待作答必须是 `Pause`（保持 Armed、作答后重入），
//!   **不得**被误判成 `Stop`；
//! - P1-2：预算耗尽静默 `return`，UI 永久显示「进行中」⇒ 必须产出结构化停止原因。
//!
//! 冻结判定顺序（不得调换）：
//! `has_active_goal=false ⇒ Stop(NoActiveGoal)` → `cancelled ⇒ Stop(Cancelled)` →
//! `pending_asks + pending_approvals > 0 ⇒ Pause` →
//! `consecutive_failures ≥ MAX_CONSECUTIVE_GOAL_ROUND_FAILURES ⇒ Stop(ConsecutiveFailures)` →
//! `rounds_started ≥ max_rounds ⇒ Stop(BudgetExhausted)` →
//! `consecutive_auto_continues ≥ max_auto_continues ⇒ Stop(AutoContinueBudget)` → `Continue`。
//!
//! 铁律：每条用例真实断言；无 `#[ignore]` / skip / 同义反复断言；不修改 `src/`。

use pony_agent_core::agent::goal::{
    goal_round_gate, goal_round_outcome_gate, GoalRoundGateDecision, GoalRoundGateInput,
    GoalRoundStepOutcome, GoalRoundStopReason, MAX_CONSECUTIVE_GOAL_ROUND_FAILURES,
};

/// 冻结真值表里的「全部绿灯」输入：任何一条停止条件都不满足。
///
/// 每个用例只改动**自己关心的那一个字段**，从而把「谁优先」变成可机械判定的问题。
fn passable_input() -> GoalRoundGateInput {
    GoalRoundGateInput {
        has_active_goal: true,
        cancelled: false,
        pending_asks: 0,
        pending_approvals: 0,
        consecutive_failures: 0,
        rounds_started: 0,
        max_rounds: 10,
        consecutive_auto_continues: 0,
        max_auto_continues: 8,
    }
}

/// 把决策渲染成稳定短串（失败信息用；避免依赖契约外的 `Debug` 派生）。
fn describe(decision: &GoalRoundGateDecision) -> String {
    match decision {
        GoalRoundGateDecision::Continue => "Continue".to_string(),
        GoalRoundGateDecision::Pause => "Pause".to_string(),
        GoalRoundGateDecision::Stop(reason) => format!("Stop({})", reason.code()),
    }
}

/// 断言 `Continue`（不做任何断言放松：只接受精确变体）。
fn assert_continue(decision: GoalRoundGateDecision, case: &str) {
    assert!(
        matches!(decision, GoalRoundGateDecision::Continue),
        "契约 C1 违约（{case}）：必须判定 Continue，实际 {}",
        describe(&decision)
    );
}

/// 断言 `Pause`（等人类作答：目标保持 Armed，作答后重入）。
fn assert_pause(decision: GoalRoundGateDecision, case: &str) {
    assert!(
        matches!(decision, GoalRoundGateDecision::Pause),
        "契约 C1 违约（{case}）：必须判定 Pause（不得误判为 Stop），实际 {}",
        describe(&decision)
    );
}

/// 断言 `Stop` 且变体与字段精确匹配。
fn assert_stop(decision: GoalRoundGateDecision, expected: GoalRoundStopReason, case: &str) {
    match decision {
        GoalRoundGateDecision::Stop(reason) => assert_eq!(
            reason, expected,
            "契约 C1 违约（{case}）：停止原因必须精确等于 {expected:?}，实际 {reason:?}"
        ),
        other => panic!(
            "契约 C1 违约（{case}）：必须判定 Stop({expected:?})，实际 {}",
            describe(&other)
        ),
    }
}

// ── 优先级 1：无 active goal ───────────────────────────────────────────────────────

/// 无 active goal ⇒ `Stop(NoActiveGoal)`，且**压过**取消/待作答/失败/预算全部条件。
#[test]
fn c1_no_active_goal_stops_first() {
    // Arrange：把「会停止」的其它条件全部打开，只留 has_active_goal=false。
    let input = GoalRoundGateInput {
        has_active_goal: false,
        cancelled: true,
        pending_asks: 3,
        pending_approvals: 1,
        consecutive_failures: 9,
        rounds_started: 99,
        max_rounds: 10,
        consecutive_auto_continues: 99,
        max_auto_continues: 8,
    };

    // Act
    let decision = goal_round_gate(input);

    // Assert
    assert_stop(decision, GoalRoundStopReason::NoActiveGoal, "无 active goal");
}

// ── 优先级 2：取消 ───────────────────────────────────────────────────────────────

/// 取消 ⇒ `Stop(Cancelled)`，且**压过**待作答（用户已按 Esc，不得再判「等你作答」）。
#[test]
fn c1_cancelled_beats_pending_asks() {
    // Arrange
    let input = GoalRoundGateInput {
        cancelled: true,
        pending_asks: 2,
        pending_approvals: 2,
        ..passable_input()
    };

    // Act
    let decision = goal_round_gate(input);

    // Assert
    assert_stop(decision, GoalRoundStopReason::Cancelled, "取消 + 待作答");
}

/// 取消 ⇒ `Stop(Cancelled)`，且压过连续失败与预算耗尽（不得被失败计数掩盖）。
#[test]
fn c1_cancelled_beats_failures_and_budget() {
    // Arrange
    let input = GoalRoundGateInput {
        cancelled: true,
        consecutive_failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES,
        rounds_started: 10,
        max_rounds: 10,
        ..passable_input()
    };

    // Act
    let decision = goal_round_gate(input);

    // Assert
    assert_stop(decision, GoalRoundStopReason::Cancelled, "取消 + 失败 + 预算");
}

// ── 优先级 3：待人类作答 ⇒ Pause（核心：goal 不得因 ask 永久失活） ───────────────────

/// `pending_asks > 0` ⇒ `Pause`，且压过连续失败与预算耗尽。
///
/// 这是 P0-1 的唯一防线：若此处返回 `Stop`，作答后没有任何驱动者会重入续轮，
/// `/goal` 的能力就退化成「最多跑到第一次 ask」。
#[test]
fn c1_pending_asks_pause_beats_failures_and_budget() {
    // Arrange
    let input = GoalRoundGateInput {
        pending_asks: 1,
        consecutive_failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES + 5,
        rounds_started: 10,
        max_rounds: 10,
        consecutive_auto_continues: 8,
        max_auto_continues: 8,
        ..passable_input()
    };

    // Act
    let decision = goal_round_gate(input);

    // Assert
    assert_pause(decision, "pending_asks + 失败 + 预算 + auto-continue 预算");
}

/// `pending_approvals > 0` ⇒ `Pause`（审批待作答与 Ask 同权）。
#[test]
fn c1_pending_approvals_pause() {
    // Arrange
    let input = GoalRoundGateInput {
        pending_approvals: 1,
        ..passable_input()
    };

    // Act
    let decision = goal_round_gate(input);

    // Assert
    assert_pause(decision, "pending_approvals=1");
}

// ── 优先级 4：连续失败熔断 ────────────────────────────────────────────────────────

/// 冻结常量锁：连续失败阈值必须是 2 轮。
#[test]
fn c1_consecutive_failure_threshold_is_two() {
    assert_eq!(
        MAX_CONSECUTIVE_GOAL_ROUND_FAILURES, 2,
        "契约 C1：连续失败阈值冻结为 2 轮，实际 {MAX_CONSECUTIVE_GOAL_ROUND_FAILURES}"
    );
}

/// 恰好达到阈值 ⇒ `Stop(ConsecutiveFailures { failures })`，且压过预算耗尽。
#[test]
fn c1_consecutive_failures_at_threshold_stops() {
    // Arrange
    let input = GoalRoundGateInput {
        consecutive_failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES,
        rounds_started: 10,
        max_rounds: 10,
        ..passable_input()
    };

    // Act
    let decision = goal_round_gate(input);

    // Assert
    assert_stop(
        decision,
        GoalRoundStopReason::ConsecutiveFailures {
            failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES,
        },
        "consecutive_failures == 阈值",
    );
}

/// 超过阈值 ⇒ 变体携带**真实**失败次数（不得写死为阈值）。
#[test]
fn c1_consecutive_failures_reports_actual_count() {
    // Arrange
    let input = GoalRoundGateInput {
        consecutive_failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES + 3,
        ..passable_input()
    };

    // Act
    let decision = goal_round_gate(input);

    // Assert
    assert_stop(
        decision,
        GoalRoundStopReason::ConsecutiveFailures {
            failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES + 3,
        },
        "consecutive_failures > 阈值",
    );
}

/// 未达阈值 ⇒ 不因失败计数停止（1 轮失败后仍可续轮）。
#[test]
fn c1_single_failure_still_continues() {
    // Arrange
    let input = GoalRoundGateInput {
        consecutive_failures: MAX_CONSECUTIVE_GOAL_ROUND_FAILURES - 1,
        ..passable_input()
    };

    // Act
    let decision = goal_round_gate(input);

    // Assert
    assert_continue(decision, "consecutive_failures == 阈值-1");
}

// ── 优先级 5：轮次预算 ───────────────────────────────────────────────────────────

/// `rounds_started >= max_rounds` ⇒ `Stop(BudgetExhausted { .. })`，携带真实计数。
#[test]
fn c1_round_budget_exhausted_stops() {
    // Arrange
    let input = GoalRoundGateInput {
        rounds_started: 7,
        max_rounds: 7,
        ..passable_input()
    };

    // Act
    let decision = goal_round_gate(input);

    // Assert
    assert_stop(
        decision,
        GoalRoundStopReason::BudgetExhausted {
            rounds_started: 7,
            max_rounds: 7,
        },
        "rounds_started == max_rounds",
    );
}

/// 预算耗尽压过 auto-continue 预算（不得把「预算用尽」说成「连续自动推进达上限」）。
#[test]
fn c1_round_budget_beats_auto_continue_budget() {
    // Arrange
    let input = GoalRoundGateInput {
        rounds_started: 10,
        max_rounds: 10,
        consecutive_auto_continues: 8,
        max_auto_continues: 8,
        ..passable_input()
    };

    // Act
    let decision = goal_round_gate(input);

    // Assert
    assert_stop(
        decision,
        GoalRoundStopReason::BudgetExhausted {
            rounds_started: 10,
            max_rounds: 10,
        },
        "预算耗尽 + auto-continue 预算用尽",
    );
}

// ── 优先级 6：每线程自动续轮预算 ──────────────────────────────────────────────────

/// `consecutive_auto_continues >= max_auto_continues` ⇒ `Stop(AutoContinueBudget)`。
#[test]
fn c1_auto_continue_budget_stops() {
    // Arrange
    let input = GoalRoundGateInput {
        consecutive_auto_continues: 8,
        max_auto_continues: 8,
        ..passable_input()
    };

    // Act
    let decision = goal_round_gate(input);

    // Assert
    assert_stop(
        decision,
        GoalRoundStopReason::AutoContinueBudget,
        "consecutive_auto_continues == max",
    );
}

// ── 优先级 7：全部条件不满足 ⇒ Continue ───────────────────────────────────────────

/// 全部绿灯 ⇒ `Continue`（防止「为了过门把默认值改成 Pause/Stop」）。
#[test]
fn c1_all_green_continues() {
    // Arrange
    let input = passable_input();

    // Act
    let decision = goal_round_gate(input);

    // Assert
    assert_continue(decision, "全部绿灯");
}

// ── 单轮终态 → 决策（首轮门与每步归约共用） ──────────────────────────────────────

/// `Suspended ⇒ Pause`：模型挂起等用户时必须**保持目标 Armed 等重入**，不得当作停止。
#[test]
fn c1_outcome_suspended_pauses() {
    // Arrange
    let outcome = GoalRoundStepOutcome::Suspended;

    // Act
    let decision = goal_round_outcome_gate(outcome);

    // Assert
    assert_pause(decision, "outcome=Suspended");
}

/// `Failed ⇒ Stop(FirstRoundNotSucceeded)`：首轮/本轮失败 ⇒ 停止，且原因必须是
/// 「首轮未成功」而非「连续 N 轮失败」（后者读作「已经连跑了好几轮」）。
#[test]
fn c1_outcome_failed_stops_first_round_not_succeeded() {
    // Arrange
    let outcome = GoalRoundStepOutcome::Failed;

    // Act
    let decision = goal_round_outcome_gate(outcome);

    // Assert
    assert_stop(
        decision,
        GoalRoundStopReason::FirstRoundNotSucceeded,
        "outcome=Failed",
    );
}

/// `Completed ⇒ Continue`：只有成功轮才推进续轮。
#[test]
fn c1_outcome_completed_continues() {
    // Arrange
    let outcome = GoalRoundStepOutcome::Completed;

    // Act
    let decision = goal_round_outcome_gate(outcome);

    // Assert
    assert_continue(decision, "outcome=Completed");
}

/// `Cancelled ⇒ Stop(Cancelled)`：用户取消是最高优先级原因。
#[test]
fn c1_outcome_cancelled_stops() {
    // Arrange
    let outcome = GoalRoundStepOutcome::Cancelled;

    // Act
    let decision = goal_round_outcome_gate(outcome);

    // Assert
    assert_stop(decision, GoalRoundStopReason::Cancelled, "outcome=Cancelled");
}

/// `Unclassified ⇒ Stop(TurnUnclassified)`：不知道成没成 ⇒ 既不冒充成功也不冒充失败。
#[test]
fn c1_outcome_unclassified_stops_turn_unclassified() {
    // Arrange
    let outcome = GoalRoundStepOutcome::Unclassified;

    // Act
    let decision = goal_round_outcome_gate(outcome);

    // Assert
    assert_stop(
        decision,
        GoalRoundStopReason::TurnUnclassified,
        "outcome=Unclassified",
    );
}

// ── Pause 的可见原因与枚举基数 ───────────────────────────────────────────────────

/// `Pause` 的用户可见原因复用既有 `TurnSuspended`，文案必须表达「等你作答 + 作答后自动继续」。
///
/// 反例（修复前）：headline「目标已暂停：续轮已停止」+ hint「先回答上面的提问」
/// ⇒ 用户读到的结论是「目标停了」，于是不会去作答，也就永远不会重入（P0-1 的可见面）。
#[test]
fn c1_pause_visible_reason_is_turn_suspended_with_resume_copy() {
    // Arrange：待作答 ⇒ Pause
    let input = GoalRoundGateInput {
        pending_asks: 1,
        ..passable_input()
    };

    // Act
    let decision = goal_round_gate(input);

    // Assert：Pause 的唯一可见原因是 TurnSuspended（不新增枚举变体）
    assert_pause(decision, "pending_asks=1（可见原因）");
    let reason = GoalRoundStopReason::TurnSuspended;
    let line = reason.visible_line();
    assert!(
        line.contains("等你作答"),
        "Pause 的可见文案必须表达「等你作答」（作答是唯一的解锁动作），实际：{line}"
    );
    assert!(
        line.contains("作答后自动继续"),
        "Pause 的可见文案必须承诺「作答后自动继续」（否则用户以为目标已死），实际：{line}"
    );
}

/// 枚举基数锁：不新增变体（Pause 复用 `TurnSuspended`，`all_variants()` 仍为 10）。
#[test]
fn c1_stop_reason_variant_count_stays_ten() {
    assert_eq!(
        GoalRoundStopReason::all_variants().len(),
        10,
        "契约 C1 禁止新增 `GoalRoundStopReason` 变体（Pause 必须复用 TurnSuspended）"
    );
}
