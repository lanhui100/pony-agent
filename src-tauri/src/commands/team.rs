//! Team 控制面命令（roster / tasks）。

use serde_json::Value;

#[tauri::command]
pub(crate) fn team_get_roster() -> Result<Value, String> {
    let res = pony_agent_core::agent::tools::list_agents()?;
    serde_json::to_value(res.agents).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn team_get_tasks() -> Result<Value, String> {
    let res = pony_agent_core::agent::tools::team_task_list()?;
    serde_json::to_value(res.tasks).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn team_claim_task(
    task_id: String,
    expected_revision: u64,
    owner: String,
) -> Result<Value, String> {
    let args = pony_agent_core::agent::tools::TeamTaskUpdateArgs {
        task_id,
        expected_revision,
        action: pony_agent_core::agent::tools::TeamTaskAction::Claim,
        owner: Some(owner),
        subject: None,
        description: None,
        write_scopes: None,
        blocked_by: None,
    };
    let res = pony_agent_core::agent::tools::team_task_update(args)?;
    serde_json::to_value(res).map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn team_complete_task(task_id: String, expected_revision: u64) -> Result<Value, String> {
    let args = pony_agent_core::agent::tools::TeamTaskUpdateArgs {
        task_id,
        expected_revision,
        action: pony_agent_core::agent::tools::TeamTaskAction::Complete,
        owner: None,
        subject: None,
        description: None,
        write_scopes: None,
        blocked_by: None,
    };
    let res = pony_agent_core::agent::tools::team_task_update(args)?;
    serde_json::to_value(res).map_err(|e| e.to_string())
}
