//! Todo 与 Goal 控制面命令。

use serde_json::Value;

/// List todos for a session.
#[tauri::command]
pub(crate) fn todo_list(session_id: String) -> Result<Value, String> {
    let args = pony_agent_core::agent::todo::TodoListArgs {
        session_id: Some(session_id),
    };
    let res = pony_agent_core::agent::todo::todo_list(args)?;
    serde_json::to_value(res.todos).map_err(|e| e.to_string())
}

/// Get current goal for a session.
#[tauri::command]
pub(crate) fn goal_get(session_id: String) -> Result<Value, String> {
    let args = pony_agent_core::agent::goal::GetGoalArgs {
        session_id: Some(session_id),
    };
    let res = pony_agent_core::agent::goal::get_goal(args)?;
    serde_json::to_value(res.goal).map_err(|e| e.to_string())
}

/// O7（2026-11-27）：双预算的**展示文案**（每输入预算 + 目标预算），文案真源在
/// `pony_storage::goal::budget_summary_lines`（与 TUI `/goal show` 同源）。
///
/// 为什么单开命令而不塞进 `goal://changed` 载荷：该载荷的字段集合是 §7 冻结契约
/// （`goal_events.rs` 的形状测试逐键断言），改它要让桌面端契约门与既有 spec 一起漂移；
/// 而双预算文案是**纯展示**，与 goal 状态变更事件无关（目标没变也要能读）。
#[tauri::command]
pub(crate) fn goal_budget_lines(session_id: String) -> Result<Value, String> {
    let args = pony_agent_core::agent::goal::GetGoalArgs {
        session_id: Some(session_id),
    };
    let goal = pony_agent_core::agent::goal::get_goal(args)?.goal;
    let Some(goal) = goal else {
        return Ok(Value::Null);
    };
    let (per_input, target) = pony_agent_core::agent::goal::budget_summary_lines(&goal);
    Ok(serde_json::json!({
        "perInputLine": per_input,
        "targetBudgetLine": target,
    }))
}

/// Create a goal for a session.
#[tauri::command]
pub(crate) fn goal_create(
    session_id: String,
    objective: String,
    max_goal_rounds: Option<u32>,
) -> Result<Value, String> {
    let args = pony_agent_core::agent::goal::CreateGoalArgs {
        session_id: Some(session_id),
        objective,
        max_goal_rounds,
    };
    let res = pony_agent_core::agent::goal::create_goal(args)?;
    serde_json::to_value(res.goal).map_err(|e| e.to_string())
}

/// Update a goal for a session using CAS.
#[tauri::command]
pub(crate) fn goal_update(
    session_id: String,
    goal_id: String,
    revision: u64,
    action: String,
    objective: Option<String>,
    blocked_reason: Option<String>,
    max_goal_rounds: Option<u32>,
) -> Result<Value, String> {
    let update_action = match action.to_lowercase().as_str() {
        "complete" => pony_agent_core::agent::goal::UpdateGoalAction::Complete,
        "pause" => pony_agent_core::agent::goal::UpdateGoalAction::Pause,
        "resume" => pony_agent_core::agent::goal::UpdateGoalAction::Resume,
        "edit" => pony_agent_core::agent::goal::UpdateGoalAction::Edit,
        "blocked" => pony_agent_core::agent::goal::UpdateGoalAction::Blocked,
        other => return Err(format!("Unsupported goal action: {}", other)),
    };
    let args = pony_agent_core::agent::goal::UpdateGoalArgs {
        session_id: Some(session_id),
        goal_id,
        revision,
        action: update_action,
        objective,
        blocked_reason,
        max_goal_rounds,
    };
    let res = pony_agent_core::agent::goal::update_goal(args)?;
    serde_json::to_value(res.goal).map_err(|e| e.to_string())
}

/// Execute a `/goal` slash command for a session (syntax/state machine lives in core).
#[tauri::command]
pub(crate) fn goal_command(session_id: String, args: String) -> Result<Value, String> {
    let outcome = pony_agent_core::agent::goal_command::execute_goal_command(&session_id, &args)?;
    serde_json::to_value(outcome).map_err(|e| e.to_string())
}
