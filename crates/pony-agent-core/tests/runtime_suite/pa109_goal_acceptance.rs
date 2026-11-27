//! PA-109: 长程自驱目标工具族黑盒验收测试
//!
//! 契约依据：`management/task-system/03_TASKS/PA-109-long-running-goal-tools.md`

use pony_agent_core::agent::tools::{
    create_goal, get_goal, update_goal, CreateGoalArgs, GetGoalArgs, GetGoalResult,
    UpdateGoalAction, UpdateGoalArgs,
};

/// PA-109 验收用的唯一会话 id。
///
/// 2026-11 修订：goal 存储已改为 **fail-closed**（`session_id` 缺失直接报错，不再折叠成
/// `"default"` 桶）。旧版本本文件靠 `session_id: None` 共享 `"default"` 槽位，既依赖了被
/// 删除的兜底语义，也正是跨会话串话的入口。改为显式会话 id；串行锁仍保留——进程级
/// store 在用例间共享，同一会话槽位必须互斥。
const SESSION: &str = "pa109-goal-acceptance";


fn goal_store_guard() -> std::sync::MutexGuard<'static, ()> {
    let guard = crate::env_lock::lock();
    pony_agent_core::agent::goal::reset_goal_store_for_test();
    let _ = pony_agent_core::agent::goal::clear_goal(SESSION);
    guard
}

#[test]
fn test_ac01_create_and_get_goal() {
    let _guard = goal_store_guard();
    let create_args = CreateGoalArgs {
        session_id: Some(SESSION.to_string()),
        objective: "Build payment module".to_string(),
        max_goal_rounds: Some(50),
    };
    let created = create_goal(create_args).expect("create_goal should succeed");
    assert_eq!(created.goal.objective, "Build payment module");
    assert_eq!(created.goal.revision, 1);
    assert_eq!(created.goal.phase, "active");

    let fetched: GetGoalResult = get_goal(GetGoalArgs {
        session_id: Some(SESSION.to_string()),
    })
    .expect("get_goal should succeed");
    assert!(fetched.goal.is_some());
}

#[test]
fn test_ac02_update_goal_cas_and_lifecycle() {
    let _guard = goal_store_guard();
    let create_args = CreateGoalArgs {
        session_id: Some(SESSION.to_string()),
        objective: "Multi-turn task".to_string(),
        max_goal_rounds: Some(20),
    };
    let created = create_goal(create_args).unwrap();

    // 暂停
    let pause_args = UpdateGoalArgs {
        session_id: Some(SESSION.to_string()),
        goal_id: created.goal.id.clone(),
        revision: created.goal.revision,
        action: UpdateGoalAction::Pause,
        objective: None,
        blocked_reason: None,
        max_goal_rounds: None,
    };
    let paused = update_goal(pause_args).expect("pause should succeed");
    assert_eq!(paused.goal.phase, "paused");
    assert_eq!(paused.goal.revision, 2);

    // 尝试用旧版本号更新应失败 (CAS guard)
    let stale_args = UpdateGoalArgs {
        session_id: Some(SESSION.to_string()),
        goal_id: created.goal.id.clone(),
        revision: 1,
        action: UpdateGoalAction::Resume,
        objective: None,
        blocked_reason: None,
        max_goal_rounds: None,
    };
    assert!(update_goal(stale_args).is_err(), "Stale revision must be rejected");

    // 正确 resume
    let resume_args = UpdateGoalArgs {
        session_id: Some(SESSION.to_string()),
        goal_id: created.goal.id.clone(),
        revision: 2,
        action: UpdateGoalAction::Resume,
        objective: None,
        blocked_reason: None,
        max_goal_rounds: None,
    };
    let resumed = update_goal(resume_args).unwrap();
    assert_eq!(resumed.goal.phase, "active");
    assert_eq!(resumed.goal.revision, 3);

    // 完成
    let complete_args = UpdateGoalArgs {
        session_id: Some(SESSION.to_string()),
        goal_id: created.goal.id.clone(),
        revision: 3,
        action: UpdateGoalAction::Complete,
        objective: None,
        blocked_reason: None,
        max_goal_rounds: None,
    };
    let completed = update_goal(complete_args).unwrap();
    assert_eq!(completed.goal.phase, "completed");

    // B127 守卫：completed 不可被 resume
    let illegal_resume = UpdateGoalArgs {
        session_id: Some(SESSION.to_string()),
        goal_id: created.goal.id.clone(),
        revision: 4,
        action: UpdateGoalAction::Resume,
        objective: None,
        blocked_reason: None,
        max_goal_rounds: None,
    };
    assert!(update_goal(illegal_resume).is_err(), "Resume on completed goal must fail");
}

#[test]
fn test_ac03_blocked_requires_reason() {
    let _guard = goal_store_guard();
    let create_args = CreateGoalArgs {
        session_id: Some(SESSION.to_string()),
        objective: "Test blocked".to_string(),
        max_goal_rounds: Some(10),
    };
    let created = create_goal(create_args).unwrap();

    // 2026-11-27（O5）：`Blocked` 新增最小轮次门槛（`GOAL_BLOCK_MIN_ROUNDS`），
    // 本用例的语义是「缺 blocked_reason 必须被拒 / 给了原因必须成功」，与轮次无关。
    // 故先推进到门槛之上，使被测分支（reason 校验）成为唯一决定因素。
    for _ in 0..pony_agent_core::agent::goal::GOAL_BLOCK_MIN_ROUNDS {
        pony_agent_core::agent::goal::advance_goal_round(&created.goal.id)
            .expect("O5 前置：advance_goal_round 必须成功");
    }
    let created = pony_agent_core::agent::goal::get_goal(GetGoalArgs {
        session_id: Some(SESSION.to_string()),
    })
    .expect("O5 前置：get_goal 必须成功")
    .goal
    .expect("O5 前置：目标必须存在");

    let invalid_block = UpdateGoalArgs {
        session_id: Some(SESSION.to_string()),
        goal_id: created.id.clone(),
        revision: created.revision,
        action: UpdateGoalAction::Blocked,
        objective: None,
        blocked_reason: None,
        max_goal_rounds: None,
    };
    assert!(update_goal(invalid_block).is_err(), "Blocked without reason must fail");

    let valid_block = UpdateGoalArgs {
        session_id: Some(SESSION.to_string()),
        goal_id: created.id.clone(),
        revision: created.revision,
        action: UpdateGoalAction::Blocked,
        objective: None,
        blocked_reason: Some("Network gateway unreachable".to_string()),
        max_goal_rounds: None,
    };
    let blocked_res = update_goal(valid_block).unwrap();
    assert_eq!(blocked_res.goal.phase, "blocked");
    assert_eq!(blocked_res.goal.blocked_reason, Some("Network gateway unreachable".to_string()));
}
