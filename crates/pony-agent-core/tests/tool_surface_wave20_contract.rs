use pony_agent_core::agent::tools::{
    builtin_turn_tool_contract_views, canonical_tool_name, ToolRegistrySnapshot,
};

#[test]
fn test_stub_tools_are_no_longer_model_visible() {
    let views = builtin_turn_tool_contract_views();
    let names: Vec<_> = views.iter().map(|v| v.name.as_str()).collect();

    assert!(!names.contains(&"subagent"), "subagent must not be exposed to models");
    assert!(!names.contains(&"workflow"), "workflow must not be exposed to models");
    assert!(!names.contains(&"spawn_teammate"), "spawn_teammate must not be exposed to models");
    assert!(!names.contains(&"list_agents"), "list_agents must not be exposed to models");
    assert!(!names.contains(&"send_message"), "send_message must not be exposed to models");
    assert!(!names.contains(&"interrupt_agent"), "interrupt_agent must not be exposed to models");
    assert!(!names.contains(&"drain_inbox"), "drain_inbox must not be exposed to models");
    assert!(!names.contains(&"team_task_create"), "team_task_create must not be exposed to models");
    assert!(!names.contains(&"team_task_update"), "team_task_update must not be exposed to models");
    assert!(!names.contains(&"team_task_list"), "team_task_list must not be exposed to models");
}

#[test]
fn test_goal_tools_are_canonical_and_model_visible() {
    assert_eq!(canonical_tool_name("create_goal"), Some("create_goal"));
    assert_eq!(canonical_tool_name("get_goal"), Some("get_goal"));
    assert_eq!(canonical_tool_name("update_goal"), Some("update_goal"));

    let views = builtin_turn_tool_contract_views();
    let names: Vec<_> = views.iter().map(|v| v.name.as_str()).collect();

    assert!(names.contains(&"create_goal"), "create_goal must be exposed to models");
    assert!(names.contains(&"get_goal"), "get_goal must be exposed to models");
    assert!(names.contains(&"update_goal"), "update_goal must be exposed to models");
}
