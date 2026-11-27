//! 契约 C2（O5）红相验收：`blocked` 的**轮次门槛**与**预算耗尽落 blocked 相位**。
//!
//! 契约真值源：`.dev-team/plan-goal-gate-v1.md` §0 C2（本文件由独立 Test Agent 冻结，
//! Executor 只读，禁改断言迁就实现）。
//!
//! 要防的回归（`docs/analysis/goal-ask-blocking-audit-2026-11-26.md` P1-2）：
//! - `blocked` 只有提示词软约束（「至少 3 轮」），代码只校验 `blocked_reason` 非空
//!   ⇒ 模型第 1 轮就能把目标标成 blocked，人类还没机会看到推进；
//! - 预算耗尽只 `return`，不落 `blocked` 相位 ⇒ GoalBar 永久显示「进行中」，
//!   `resume` 又被「预算耗尽」拒绝 ⇒ 死局且无解释。
//!
//! 冻结面：
//! - `GOAL_BLOCK_MIN_ROUNDS: u32 = 3`；
//! - `update_goal(Blocked)`：`rounds_started < 3` ⇒ `Err` 且错误串以 `goal_block_threshold`
//!   开头（工具层据此给 error_code）；`>= 3` ⇒ `Ok` 且 `phase == "blocked"`；
//! - `block_goal_for_round_limit(goal_id)` ⇒ `phase=blocked`、`activation=Disarmed`、
//!   `blocked_reason=Some("round_limit")`，且 `active_goal_for_session` 立即 `None`。
//!
//! 铁律：每条用例真实断言；无 `#[ignore]` / skip / 同义反复断言；不修改 `src/`。

use pony_agent_core::agent::goal::test_support::TempGoalStore;
use pony_agent_core::agent::goal::{
    active_goal_for_session, advance_goal_round, block_goal_for_round_limit, create_goal,
    update_goal, CreateGoalArgs, GoalActivation, UpdateGoalAction, UpdateGoalArgs,
    GOAL_BLOCK_MIN_ROUNDS,
};

/// 冻结常量锁：门槛是 3 轮（与 DSH `GOAL_TOOL_BLOCK_THRESHOLD` 对齐）。
#[test]
fn c2_block_min_rounds_is_three() {
    assert_eq!(
        GOAL_BLOCK_MIN_ROUNDS, 3,
        "契约 C2：blocked 门槛冻结为 3 轮，实际 {GOAL_BLOCK_MIN_ROUNDS}"
    );
}

/// 造一个已跑 `rounds` 轮、仍 active+Armed 的目标（`rounds` 由 `advance_goal_round` 累加）。
fn armed_goal_after_rounds(session: &str, rounds: u32) -> String {
    let goal = create_goal(CreateGoalArgs {
        session_id: Some(session.to_string()),
        objective: "O5 blocked 门槛探针".to_string(),
        max_goal_rounds: Some(20),
    })
    .unwrap_or_else(|err| panic!("Arrange：create_goal 必须成功（session={session}），Err: {err}"))
    .goal;
    for round in 0..rounds {
        advance_goal_round(&goal.id)
            .unwrap_or_else(|err| panic!("Arrange：第 {round} 次 advance_goal_round 必须成功，Err: {err}"));
    }
    goal.id
}

/// 读当前快照（`update_goal` 的 CAS 需要 revision，故必须每次现读）。
fn current_revision(session: &str) -> (String, u64) {
    let goal = active_goal_for_session(session)
        .unwrap_or_else(|| panic!("Arrange：session={session} 应存在 active+Armed 目标"));
    (goal.id, goal.revision)
}

fn blocked_args(session: &str, goal_id: &str, revision: u64) -> UpdateGoalArgs {
    UpdateGoalArgs {
        session_id: Some(session.to_string()),
        goal_id: goal_id.to_string(),
        revision,
        action: UpdateGoalAction::Blocked,
        objective: None,
        blocked_reason: Some("模型自称受阻".to_string()),
        max_goal_rounds: None,
    }
}

// ── 门槛内（0/1/2 轮）必须硬拒 ───────────────────────────────────────────────────

/// `rounds_started = 0/1/2` 时 `Blocked` 必须 `Err`，且错误串以 `goal_block_threshold` 开头。
///
/// 这是 O5 的唯一机械防线：提示词说「至少 3 轮」但代码不校验时，模型首轮即可 blocked。
#[test]
fn c2_blocked_rejected_below_threshold_with_stable_prefix() {
    // Arrange：三个独立会话，各自跑 0/1/2 轮。
    // 2026-11-27：串行化必须用**套件级** env_lock（runtime_suite 里 pa109 / wave_plan 等
    // 既有用例都用它）；`goal::test_support::lock()` 是库 crate 内另一把互不相斥的锁，
    // 两把混用会让 TempGoalStore 的临时目录被并发用例删掉（实测 `No such file or directory`）。
    let _guard = crate::env_lock::lock();
    let _store = TempGoalStore::install("o5-block-threshold");

    for rounds in 0..GOAL_BLOCK_MIN_ROUNDS {
        let session = format!("o5-block-below-{rounds}");
        let goal_id = armed_goal_after_rounds(&session, rounds);
        let (current_id, revision) = current_revision(&session);
        assert_eq!(current_id, goal_id, "Arrange：goal id 必须稳定");

        // Act
        let result = update_goal(blocked_args(&session, &goal_id, revision));

        // Assert：必须是 Err，且前缀固定（工具层按前缀给 error_code）
        let error = result.err().unwrap_or_else(|| {
            panic!("契约 C2 违约（rounds_started={rounds}）：门槛内 Blocked 必须被拒，实际 Ok")
        });
        assert!(
            error.starts_with("goal_block_threshold"),
            "契约 C2 违约（rounds_started={rounds}）：错误串必须以 `goal_block_threshold` 开头，实际：{error}"
        );

        // Assert：被拒后目标仍 active+Armed（不得半途落盘）
        let still_active = active_goal_for_session(&session)
            .unwrap_or_else(|| panic!("契约 C2 违约（rounds_started={rounds}）：被拒后目标不得消失"));
        assert_eq!(still_active.phase, "active", "被拒后 phase 必须仍是 active");
        assert_eq!(
            still_active.activation,
            GoalActivation::Armed,
            "被拒后 activation 必须仍是 Armed"
        );
        assert_eq!(
            still_active.blocked_reason, None,
            "被拒后不得留下 blocked_reason"
        );
    }
}

// ── 门槛之上（>= 3 轮）必须成功 ─────────────────────────────────────────────────

/// `rounds_started = 3`（恰好达门槛）⇒ `Ok` 且 `phase == "blocked"`、`activation == Disarmed`。
#[test]
fn c2_blocked_accepted_at_threshold() {
    // Arrange
    // 2026-11-27：串行化必须用**套件级** env_lock（runtime_suite 里 pa109 / wave_plan 等
    // 既有用例都用它）；`goal::test_support::lock()` 是库 crate 内另一把互不相斥的锁，
    // 两把混用会让 TempGoalStore 的临时目录被并发用例删掉（实测 `No such file or directory`）。
    let _guard = crate::env_lock::lock();
    let _store = TempGoalStore::install("o5-block-accept");
    let session = "o5-block-at-threshold";
    let goal_id = armed_goal_after_rounds(session, GOAL_BLOCK_MIN_ROUNDS);
    let (_, revision) = current_revision(session);

    // Act
    let result = update_goal(blocked_args(session, &goal_id, revision));

    // Assert
    let updated = result
        .unwrap_or_else(|err| panic!("契约 C2 违约：达门槛的 Blocked 必须成功，实际 Err: {err}"))
        .goal;
    assert_eq!(updated.phase, "blocked", "达门槛后 phase 必须落为 blocked");
    assert_eq!(
        updated.activation,
        GoalActivation::Disarmed,
        "blocked 必须 Disarmed（不得继续自动续轮）"
    );
    assert_eq!(
        updated.blocked_reason.as_deref(),
        Some("模型自称受阻"),
        "blocked_reason 必须保留调用方给的原因"
    );
    assert!(
        active_goal_for_session(session).is_none(),
        "契约 C2 违约：blocked 后 active_goal_for_session 必须返回 None"
    );
}

/// 超过门槛（5 轮）同样必须成功（门槛是下限，不是精确值）。
#[test]
fn c2_blocked_accepted_above_threshold() {
    // Arrange
    // 2026-11-27：串行化必须用**套件级** env_lock（runtime_suite 里 pa109 / wave_plan 等
    // 既有用例都用它）；`goal::test_support::lock()` 是库 crate 内另一把互不相斥的锁，
    // 两把混用会让 TempGoalStore 的临时目录被并发用例删掉（实测 `No such file or directory`）。
    let _guard = crate::env_lock::lock();
    let _store = TempGoalStore::install("o5-block-above");
    let session = "o5-block-above-threshold";
    let goal_id = armed_goal_after_rounds(session, GOAL_BLOCK_MIN_ROUNDS + 2);
    let (_, revision) = current_revision(session);

    // Act
    let result = update_goal(blocked_args(session, &goal_id, revision));

    // Assert
    let updated = result
        .unwrap_or_else(|err| panic!("契约 C2 违约：超过门槛的 Blocked 必须成功，Err: {err}"))
        .goal;
    assert_eq!(updated.phase, "blocked");
}

// ── 预算耗尽 ⇒ 落 blocked（P1-2 的修复面） ────────────────────────────────────────

/// `block_goal_for_round_limit` 必须落 `phase=blocked` / `activation=Disarmed` /
/// `blocked_reason=Some("round_limit")`，并让 `active_goal_for_session` 立即 `None`。
///
/// 反例（修复前）：预算耗尽只 `return`，相位仍是 `active+Armed` ⇒ UI 永久「进行中」，
/// `resume` 又被拒 ⇒ 死局且无解释。
#[test]
fn c2_block_goal_for_round_limit_disarms_and_records_reason() {
    // Arrange：跑到预算耗尽（3/3），目标仍 active+Armed（旧实现就停在这里）。
    // 2026-11-27：串行化必须用**套件级** env_lock（runtime_suite 里 pa109 / wave_plan 等
    // 既有用例都用它）；`goal::test_support::lock()` 是库 crate 内另一把互不相斥的锁，
    // 两把混用会让 TempGoalStore 的临时目录被并发用例删掉（实测 `No such file or directory`）。
    let _guard = crate::env_lock::lock();
    let _store = TempGoalStore::install("o5-round-limit");
    let session = "o5-round-limit-session";
    let goal = create_goal(CreateGoalArgs {
        session_id: Some(session.to_string()),
        objective: "O5 预算耗尽落 blocked 探针".to_string(),
        max_goal_rounds: Some(GOAL_BLOCK_MIN_ROUNDS),
    })
    .expect("Arrange：create_goal 必须成功")
    .goal;
    for round in 0..GOAL_BLOCK_MIN_ROUNDS {
        advance_goal_round(&goal.id)
            .unwrap_or_else(|err| panic!("Arrange：第 {round} 次 advance 必须成功，Err: {err}"));
    }
    let exhausted = active_goal_for_session(session)
        .expect("Arrange：预算耗尽前目标应仍 active+Armed");
    assert_eq!(
        exhausted.rounds_started, GOAL_BLOCK_MIN_ROUNDS,
        "Arrange：rounds_started 必须已用满"
    );

    // Act
    let blocked = block_goal_for_round_limit(&goal.id)
        .unwrap_or_else(|err| panic!("契约 C2 违约：预算耗尽落 blocked 必须成功，Err: {err}"));

    // Assert：领域事实三件套
    assert_eq!(blocked.phase, "blocked", "必须落 blocked 相位");
    assert_eq!(
        blocked.activation,
        GoalActivation::Disarmed,
        "必须 Disarmed（否则 GoalBar 仍显示进行中）"
    );
    assert_eq!(
        blocked.blocked_reason.as_deref(),
        Some("round_limit"),
        "blocked_reason 必须精确为 `round_limit`（前端/工具按此字面量区分原因）"
    );

    // Assert：读面立即生效（唯一真值源不再返回 active goal）
    assert!(
        active_goal_for_session(session).is_none(),
        "契约 C2 违约：blocked 后 active_goal_for_session 必须立即返回 None"
    );
}
