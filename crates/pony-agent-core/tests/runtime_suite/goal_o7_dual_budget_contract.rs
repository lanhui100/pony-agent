//! O7（P2）验收：goal 的**双预算**语义必须显式声明并分别展示。
//!
//! 契约真值源：`docs/analysis/goal-ask-blocking-audit-2026-11-26.md` §4 O7
//! （「要么把 8 轮并入 `max_goal_rounds` 单闸门，要么显式声明『每次用户输入最多 8 轮、
//! 目标预算 50 轮跨多次输入』，并在 `/goal show` 与 GoalBar 上分别展示，
//! 避免『承诺 50 实跑 8』。」）
//!
//! 本仓选择**显式声明**路线（不合并闸门）：两个预算回答的是两个不同问题
//! ——「一次输入能自治多久」vs「这个目标总共值多少轮」——合并会让其中一个语义消失。
//!
//! 要防的回归（审核报告 P2-1）：界面只展示目标预算（50），用户看到实跑 8 轮就停，
//! 观感是「承诺没兑现」，且没有任何界面解释这 8 轮的来源。
//!
//! 铁律：每条用例真实断言；无 `#[ignore]` / skip；不修改 `src/`。

use pony_agent_core::agent::goal::test_support::TempGoalStore;
use pony_agent_core::agent::goal::{
    advance_goal_round, budget_summary_lines, create_goal, CreateGoalArgs,
    MAX_AUTO_CONTINUES_PER_INPUT,
};
use pony_agent_core::agent::goal_command::execute_goal_command;
use pony_agent_core::agent::planner::MAX_CONSECUTIVE_AUTO_CONTINUES;

/// 冻结契约 1：每输入预算 = 8（与 NFR `auto_continue_max` 一致）。
#[test]
fn o7_per_input_budget_is_eight() {
    assert_eq!(
        MAX_AUTO_CONTINUES_PER_INPUT, 8,
        "O7：每输入自动续轮预算冻结为 8 轮（NFR auto_continue_max）"
    );
}

/// 冻结契约 2：**单源**——core 侧读面必须与领域层常量一致，不得另立字面量。
#[test]
fn o7_core_read_surface_matches_domain_constant() {
    assert_eq!(
        MAX_CONSECUTIVE_AUTO_CONTINUES, MAX_AUTO_CONTINUES_PER_INPUT,
        "O7：planner::MAX_CONSECUTIVE_AUTO_CONTINUES 必须引用 goal::MAX_AUTO_CONTINUES_PER_INPUT"
    );
}

/// 冻结契约 3：两个预算**不相等**（相等就意味着有人把它们悄悄合并成一个闸门）。
#[test]
fn o7_two_budgets_are_distinct_numbers() {
    let _env_lock = crate::env_lock::lock();
    let _store = TempGoalStore::install("o7-distinct");
    let session = "o7-distinct-session";
    let goal = create_goal(CreateGoalArgs {
        session_id: Some(session.to_string()),
        objective: "验证双预算是两个数".to_string(),
        max_goal_rounds: None,
    })
    .expect("create goal")
    .goal;

    assert_eq!(goal.max_goal_rounds, 50, "O7：目标预算默认 50 轮");
    assert_ne!(
        usize::try_from(goal.max_goal_rounds).expect("max_goal_rounds fits usize"),
        MAX_AUTO_CONTINUES_PER_INPUT,
        "O7：目标预算与每输入预算必须是两个独立的数"
    );
}

/// 展示面：`budget_summary_lines` 必须把两个预算**分别**讲清楚。
#[test]
fn o7_budget_summary_lines_state_both_budgets() {
    let _env_lock = crate::env_lock::lock();
    let _store = TempGoalStore::install("o7-summary");
    let session = "o7-summary-session";
    let goal = create_goal(CreateGoalArgs {
        session_id: Some(session.to_string()),
        objective: "验证双预算文案".to_string(),
        max_goal_rounds: Some(20),
    })
    .expect("create goal")
    .goal;

    let (per_input, target) = budget_summary_lines(&goal);

    assert!(
        per_input.contains("每次输入最多 8 轮"),
        "O7：每输入行必须写明 8 轮上限，实际 `{per_input}`"
    );
    assert!(
        per_input.contains("等你发话"),
        "O7：每输入行必须说明停止条件是回到人类，实际 `{per_input}`"
    );
    assert!(
        target.contains("0/20"),
        "O7：目标行必须带当前进度，实际 `{target}`"
    );
    assert!(
        target.contains("跨多次输入累计"),
        "O7：目标行必须写明跨输入累计口径，实际 `{target}`"
    );
    assert!(
        target.contains("round_limit"),
        "O7：目标行必须写明用满后的后果，实际 `{target}`"
    );
}

/// 展示面：进度推进后目标行必须跟着变（不是静态文案）。
#[test]
fn o7_budget_summary_tracks_round_progress() {
    let _env_lock = crate::env_lock::lock();
    let _store = TempGoalStore::install("o7-progress");
    let session = "o7-progress-session";
    let goal = create_goal(CreateGoalArgs {
        session_id: Some(session.to_string()),
        objective: "验证目标行随轮次前进".to_string(),
        max_goal_rounds: Some(10),
    })
    .expect("create goal")
    .goal;

    advance_goal_round(&goal.id).expect("advance round 1");
    advance_goal_round(&goal.id).expect("advance round 2");

    let (_per_input, target) = budget_summary_lines(
        &pony_agent_core::agent::goal::get_goal(pony_agent_core::agent::goal::GetGoalArgs {
            session_id: Some(session.to_string()),
        })
        .expect("get goal")
        .goal
        .expect("goal present"),
    );
    assert!(
        target.contains("2/10"),
        "O7：目标行必须反映真实轮次进度，实际 `{target}`"
    );
}

/// `/goal show` 必须同时输出两行预算（此前只有「轮次: N/M」一行）。
#[test]
fn o7_goal_show_renders_both_budget_lines() {
    let _env_lock = crate::env_lock::lock();
    let _store = TempGoalStore::install("o7-show");
    let session = "o7-show-session";
    create_goal(CreateGoalArgs {
        session_id: Some(session.to_string()),
        objective: "验证 /goal show 双预算".to_string(),
        max_goal_rounds: None,
    })
    .expect("create goal");

    let outcome = execute_goal_command(session, "").expect("/goal show");
    assert_eq!(outcome.action, "show");

    assert!(
        outcome.message.contains("自动续轮: 每次输入最多 8 轮"),
        "O7：/goal show 必须展示每输入预算，实际:\n{}",
        outcome.message
    );
    assert!(
        outcome.message.contains("目标预算: 0/50 轮"),
        "O7：/goal show 必须展示目标预算与进度，实际:\n{}",
        outcome.message
    );
    // 反向钉子：原有轮次行不得被双预算行取代（两个口径都要在）。
    assert!(
        outcome.message.contains("轮次: 0/50"),
        "O7：既有「轮次: N/M」行必须保留，实际:\n{}",
        outcome.message
    );
}
