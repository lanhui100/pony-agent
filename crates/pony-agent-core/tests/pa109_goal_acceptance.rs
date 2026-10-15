//! PA-109: 长程自驱目标工具族黑盒验收测试
//!
//! 契约依据：`management/task-system/03_TASKS/PA-109-long-running-goal-tools.md`

use pony_agent_core::agent::tools::{
    create_goal, get_goal, update_goal, CreateGoalArgs, GetGoalArgs, GetGoalResult,
    UpdateGoalAction, UpdateGoalArgs,
};

/// wave-13：目标工具族使用**进程级全局 store**（`goal.rs` 的 `SESSION_GOALS`，key=session_id，
/// `None` → "default"）。本文件 3 个用例都用 `session_id: None`，cargo 默认并行执行会让它们
/// 互相覆盖同一 "default" 槽位，从而偶发 `Goal ID mismatch`（CAS 断言误伤）。
/// 用进程内互斥把本文件用例串行化，保持语义不变、消除并行干扰。
static GOAL_STORE_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn goal_store_guard() -> std::sync::MutexGuard<'static, ()> {
    GOAL_STORE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn test_ac01_create_and_get_goal() {
    let _guard = goal_store_guard();
    let create_args = CreateGoalArgs {
        session_id: None,
        objective: "Build payment module".to_string(),
        max_goal_rounds: Some(50),
    };
    let created = create_goal(create_args).expect("create_goal should succeed");
    assert_eq!(created.goal.objective, "Build payment module");
    assert_eq!(created.goal.revision, 1);
    assert_eq!(created.goal.phase, "active");

    let fetched: GetGoalResult = get_goal(GetGoalArgs::default()).expect("get_goal should succeed");
    assert!(fetched.goal.is_some());
}

#[test]
fn test_ac02_update_goal_cas_and_lifecycle() {
    let _guard = goal_store_guard();
    let create_args = CreateGoalArgs {
        session_id: None,
        objective: "Multi-turn task".to_string(),
        max_goal_rounds: Some(20),
    };
    let created = create_goal(create_args).unwrap();

    // 暂停
    let pause_args = UpdateGoalArgs {
        session_id: None,
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
        session_id: None,
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
        session_id: None,
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
        session_id: None,
        goal_id: created.goal.id.clone(),
        revision: 3,
        action: UpdateGoalAction::Complete,
        objective: None,
        blocked_reason: None,
        max_goal_rounds: None,
    };
    let completed = update_goal(complete_args).unwrap();
    assert_eq!(completed.goal.phase, "completed");
}

#[test]
fn test_ac03_blocked_requires_reason() {
    let _guard = goal_store_guard();
    let create_args = CreateGoalArgs {
        session_id: None,
        objective: "Test blocked".to_string(),
        max_goal_rounds: Some(10),
    };
    let created = create_goal(create_args).unwrap();

    let invalid_block = UpdateGoalArgs {
        session_id: None,
        goal_id: created.goal.id.clone(),
        revision: created.goal.revision,
        action: UpdateGoalAction::Blocked,
        objective: None,
        blocked_reason: None,
        max_goal_rounds: None,
    };
    assert!(update_goal(invalid_block).is_err(), "Blocked without reason must fail");

    let valid_block = UpdateGoalArgs {
        session_id: None,
        goal_id: created.goal.id.clone(),
        revision: created.goal.revision,
        action: UpdateGoalAction::Blocked,
        objective: None,
        blocked_reason: Some("Network gateway unreachable".to_string()),
        max_goal_rounds: None,
    };
    let blocked_res = update_goal(valid_block).unwrap();
    assert_eq!(blocked_res.goal.phase, "blocked");
    assert_eq!(blocked_res.goal.blocked_reason, Some("Network gateway unreachable".to_string()));
}
