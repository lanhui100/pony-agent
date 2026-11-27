//! 契约 C4（O3）红相验收：**armed goal 判定先于正文问号启发式**。
//!
//! 契约真值源：`.dev-team/plan-goal-gate-v1.md` §0 C4 第一条（本文件由独立 Test Agent 冻结，
//! Executor 只读，禁改断言迁就实现）。
//!
//! 要防的回归（`docs/analysis/goal-ask-blocking-audit-2026-11-26.md` §1.4 / P0-3）：
//! `DefaultGraphPlanner::decide_after_turn` 在 armed goal 判定**之前**先跑
//! `assistant_requests_user_input`（trim 后以 `?`/`？` 结尾即命中）⇒ goal 轮里模型只要
//! 正文以问号收尾（「要不要我顺手把 B 也改掉？」），循环就静默退出，且**不弹 Ask 面板**，
//! 用户只看到「目标不动了」。
//!
//! 修复口径：armed goal 存在时不再走文本问号启发式，判定为 `Continue`。
//! 注意反向钉子——**无** armed goal 时末尾问号仍必须 `WaitUser`（不得把人类交互通道整体废掉）。
//!
//! 铁律：每条用例真实断言；无 `#[ignore]` / skip / 同义反复断言；不修改 `src/`。

use pony_agent_core::agent::goal::test_support::TempGoalStore;
use pony_agent_core::agent::goal::{
    active_goal_for_session, clear_goal, create_goal, update_goal, CreateGoalArgs, GoalData,
    UpdateGoalAction, UpdateGoalArgs,
};
use pony_agent_core::agent::graph::{
    GraphDecision, GraphDecisionKind, GraphRun, GraphRunPhase, GraphTurnHandoff,
};
use pony_agent_core::agent::planner::{DefaultGraphPlanner, GraphPlanner, GraphPlanningContext};

/// 构造一个 armed/active 目标（objective 刻意不含任何续行关键词）。
fn create_armed_goal(session: &str) -> GoalData {
    let _ = clear_goal(session);
    create_goal(CreateGoalArgs {
        session_id: Some(session.to_string()),
        objective: "把审计文档的 P0 项逐条落地".to_string(),
        max_goal_rounds: Some(10),
    })
    .unwrap_or_else(|err| panic!("Arrange：create_goal 必须成功（session={session}），Err: {err}"))
    .goal
}

/// 收尾：把目标标成 completed（Disarmed），避免进程级 store 影响后续用例。
fn disarm(goal: &GoalData) {
    update_goal(UpdateGoalArgs {
        session_id: Some(goal.session_id.clone()),
        goal_id: goal.id.clone(),
        revision: goal.revision,
        action: UpdateGoalAction::Complete,
        objective: None,
        blocked_reason: None,
        max_goal_rounds: None,
    })
    .unwrap_or_else(|err| panic!("Teardown：update_goal(Complete) 必须成功，Err: {err}"));
}

/// 构造 run（session 绑定，goal 文本与 armed 目标无关）。
fn run_for_session(session: &str) -> GraphRun {
    GraphRun {
        id: "run-o3".to_string(),
        goal: "无关文本".to_string(),
        session_id: Some(session.to_string()),
        phase: GraphRunPhase::Running,
        steps: Vec::new(),
        active_turn_id: None,
        last_completed_turn_id: None,
        stop_reason: None,
        last_handoff: None,
        resume_count: 0,
        last_decision: None,
        control_boundary_evidence: Vec::new(),
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

/// 构造 handoff（turn 已正常收口：phase=ready、无 checkpoint 残留）。
fn handoff_for_session(session: &str, assistant_message: &str) -> GraphTurnHandoff {
    GraphTurnHandoff {
        contract_version: "graph-run-contract-v1".to_string(),
        turn_id: Some("turn-o3".to_string()),
        session_id: Some(session.to_string()),
        turn_phase: "ready".to_string(),
        checkpoint_status: Some("completed".to_string()),
        checkpoint_phase: Some("ready".to_string()),
        user_message: "继续推进目标".to_string(),
        assistant_message: assistant_message.to_string(),
        session_summary: "已完成 P0-2 的 gate 收敛，正在推进 P0-3。".to_string(),
        conversation_id: session.to_string(),
        session_turn_count: 2,
        run_id: Some("run-o3".to_string()),
        run_phase: Some("waiting_user".to_string()),
        active_task_focus: None,
        acceptance_focus: None,
        closeout_focus: None,
        last_referenced_file: None,
        recent_attachment_asset_count: 0,
        long_term_memory_status: "empty".to_string(),
        long_term_memory_entry_count: 0,
        trace_step_count: 2,
        tool_activity_count: 1,
        provider_name: "test-provider".to_string(),
        provider_model: "test-model".to_string(),
    }
}

fn assert_continue(decision: &GraphDecision, case: &str) {
    assert_eq!(
        decision.kind,
        GraphDecisionKind::Continue,
        "契约 C4 违约（{case}）：armed 目标存在时必须 Continue，实际 kind={:?}, reason={:?}, summary={:?}",
        decision.kind,
        decision.reason,
        decision.summary
    );
}

// ── 核心：armed goal + 正文以问号结尾 ⇒ Continue ────────────────────────────────

/// O3 主用例：armed 目标存在时，正文以「？」结尾**不得**再判 `WaitUser`。
///
/// 三种形态各锚一次：中文全角问号、ASCII 问号、以及启发式关键词（「是否需要我继续」），
/// 防止只对某一个字面量打补丁。
#[test]
fn c4_armed_goal_yields_continue_even_when_body_ends_with_question() {
    // Arrange
    // 2026-11-27：串行化必须用**套件级** env_lock（runtime_suite 里 pa109 / wave_plan 等
    // 既有用例都用它）；`goal::test_support::lock()` 是库 crate 内另一把互不相斥的锁，
    // 两把混用会让 TempGoalStore 的临时目录被并发用例删掉（实测 `No such file or directory`）。
    let _guard = crate::env_lock::lock();
    let _store = TempGoalStore::install("o3-planner-question");
    let session = "o3-planner-question-session";
    let goal = create_armed_goal(session);
    assert!(
        active_goal_for_session(session).is_some(),
        "Arrange：目标必须是 active+Armed"
    );
    let run = run_for_session(session);
    let planner = DefaultGraphPlanner;

    // Act + Assert
    for assistant_message in [
        "P0-2 的 gate 已收敛，要不要我顺手把 P0-3 的问号启发式也一起改掉？",
        "P0-2 的 gate 已收敛，shall I also fix P0-3?",
        "P0-2 的 gate 已收敛。是否需要我继续推进 P0-3",
    ] {
        let handoff = handoff_for_session(session, assistant_message);
        let decision = planner.decide_after_turn(GraphPlanningContext::from_run(&run, &handoff));
        assert_continue(&decision, assistant_message);
    }

    // Teardown
    disarm(&goal);
}

// ── 反向钉子：无 armed goal 时问号仍是人类交互通道 ──────────────────────────────

/// 无 armed 目标时，正文末尾问号必须仍判 `WaitUser`。
///
/// 若这条变红，说明修复把「人类交互通道」整体废掉了（模型自问自答、无限自转）。
#[test]
fn c4_without_armed_goal_trailing_question_still_waits_user() {
    // Arrange：确保该 session 无 active goal
    // 2026-11-27：串行化必须用**套件级** env_lock（runtime_suite 里 pa109 / wave_plan 等
    // 既有用例都用它）；`goal::test_support::lock()` 是库 crate 内另一把互不相斥的锁，
    // 两把混用会让 TempGoalStore 的临时目录被并发用例删掉（实测 `No such file or directory`）。
    let _guard = crate::env_lock::lock();
    let _store = TempGoalStore::install("o3-planner-no-goal");
    let session = "o3-planner-no-goal-session";
    let _ = clear_goal(session);
    assert!(
        active_goal_for_session(session).is_none(),
        "Arrange：该 session 不得存在 active goal"
    );
    let run = run_for_session(session);
    let handoff = handoff_for_session(session, "还需要我继续核对下一层 provider fallback 吗？");

    // Act
    let decision = DefaultGraphPlanner
        .decide_after_turn(GraphPlanningContext::from_run(&run, &handoff));

    // Assert
    assert_eq!(
        decision.kind,
        GraphDecisionKind::WaitUser,
        "反向钉子违约：无 armed 目标时末尾问号必须 WaitUser，实际 kind={:?}, summary={:?}",
        decision.kind,
        decision.summary
    );
    assert_eq!(decision.target_phase, GraphRunPhase::WaitingUser);
}
