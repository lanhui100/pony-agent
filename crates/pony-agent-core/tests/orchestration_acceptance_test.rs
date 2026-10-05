use pony_agent_core::agent::tools::{ToolCall, ToolRouter};
use serde_json::json;

fn make_call(name: &str, args: serde_json::Value) -> ToolCall {
    ToolCall {
        call_id: None,
        name: name.to_string(),
        arguments: args,
        plan: None,
    }
}

#[test]
fn test_subagent_tool_routed_through_tool_router() {
    let router = ToolRouter::new();
    let call = make_call(
        "subagent",
        json!({
            "prompt": "Inspect auth tokens in workspace",
            "description": "Security audit subagent",
            "run_in_background": false
        }),
    );
    let result = router.execute(&call);
    // 红相期望：由于 tools.rs 尚未接线，执行必然失败（返回 unsupported_tool），后续绿相接线后此处转绿
    assert_eq!(result.status, "ok", "subagent tool should execute successfully once wired; currently unsupported: {:?}", result);
}

#[test]
fn test_agent_teams_tools_routed_through_tool_router() {
    let router = ToolRouter::new();
    let spawn_call = make_call(
        "spawn_teammate",
        json!({
            "name": "reviewer",
            "role": "code-reviewer",
            "description": "Reviews code PRs",
            "prompt": "You are a code reviewer"
        }),
    );
    let spawn_res = router.execute(&spawn_call);
    assert_eq!(spawn_res.status, "ok", "spawn_teammate tool should execute successfully once wired; currently unsupported: {:?}", spawn_res);

    let list_call = make_call("list_agents", json!({}));
    let list_res = router.execute(&list_call);
    assert_eq!(list_res.status, "ok", "list_agents tool should execute successfully once wired; currently unsupported: {:?}", list_res);

    let send_call = make_call(
        "send_message",
        json!({
            "target": "reviewer",
            "message": "Please review this pull request"
        }),
    );
    let send_res = router.execute(&send_call);
    assert_eq!(send_res.status, "ok", "send_message tool should execute successfully once wired; currently unsupported: {:?}", send_res);
}

#[test]
fn test_team_task_board_tools_routed_through_tool_router() {
    let router = ToolRouter::new();
    let create_call = make_call(
        "team_task_create",
        json!({
            "subject": "Refactor router",
            "description": "Clean up route branches"
        }),
    );
    let create_res = router.execute(&create_call);
    assert_eq!(create_res.status, "ok", "team_task_create tool should execute successfully once wired; currently unsupported: {:?}", create_res);

    let list_call = make_call("team_task_list", json!({}));
    let list_res = router.execute(&list_call);
    assert_eq!(list_res.status, "ok", "team_task_list tool should execute successfully once wired; currently unsupported: {:?}", list_res);
}

