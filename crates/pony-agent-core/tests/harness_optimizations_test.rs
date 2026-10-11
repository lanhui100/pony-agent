//! 验证 4 项 Harness 调优需求：
//! 1. 单轮工具调用上限提升为 500，且 MAX_ALLOWED 提升；
//! 2. TodoWrite 工具命名映射正确，且不降级为 Run；
//! 3. BatchExecute 子调用失败时，父级 Activity/Telemetry 必须包含子调用错误信息，不能为 null；
//! 4. 搜索工具默认忽略 target/、node_modules/、.git/。

use pony_agent_core::agent::runtime::{
    max_tool_followups_per_turn, parse_max_tool_followups_per_turn,
};
use pony_agent_core::agent::tools::{
    canonical_tool_name, model_visible_tool_name,
};

#[test]
fn test_tool_followups_limit_500() {
    assert_eq!(parse_max_tool_followups_per_turn(None), 500);
    assert_eq!(max_tool_followups_per_turn(), 500);
    assert_eq!(parse_max_tool_followups_per_turn(Some("1000")), 1000);
}

#[test]
fn test_todowrite_tool_mapping() {
    assert_eq!(canonical_tool_name("TodoWrite"), Some("todo_write"));
    assert_eq!(canonical_tool_name("todo_write"), Some("todo_write"));
    assert_eq!(model_visible_tool_name("todo_write"), "TodoWrite");
    assert_eq!(model_visible_tool_name("TodoWrite"), "TodoWrite");
    assert_ne!(canonical_tool_name("TodoWrite"), Some("workspace_run_command"));
}

#[test]
fn test_batch_execute_error_propagation_in_telemetry() {
    use pony_agent_core::agent::telemetry::{DefaultTurnTelemetryBuilder, TurnTelemetryBuilder};
    use pony_agent_core::agent::tools::{ToolCall, ToolResult};
    use serde_json::json;

    let telemetry = DefaultTurnTelemetryBuilder;
    let call = ToolCall {
        name: "workspace_batch".to_string(),
        arguments: json!({
            "calls": [
                {
                    "name": "workspace_read_file",
                    "arguments": { "path": "non_existent.txt" }
                }
            ]
        }),
        call_id: Some("call-1".to_string()),
        plan: None,
    };
    let result = ToolResult {
        tool_name: "workspace_batch".to_string(),
        status: "error".to_string(),
        duration_ms: 10,
        output: json!({
            "ok": false,
            "status": "error",
            "results": [
                {
                    "index": 0,
                    "tool": "workspace_read_file",
                    "ok": false,
                    "aggregateStatus": "error",
                    "error": {
                        "code": "file_not_found",
                        "message": "文件不存在: non_existent.txt"
                    }
                }
            ]
        }).to_string(),
    };

    let activities = telemetry.tool_activities_after_result(&call, &result);
    assert!(!activities.is_empty());
    let parent_activity = &activities[0];
    assert!(parent_activity.error.is_some(), "Parent activity error must not be null when subcall fails");
    let err_val = parent_activity.error.as_ref().unwrap();
    assert_eq!(err_val["kind"], "file_not_found");
    assert_eq!(err_val["message"], "文件不存在: non_existent.txt");
}

#[test]
fn test_search_engine_ignores_heavy_directories() {
    use pony_agent_core::agent::search::{SearchEngine, SearchOptions};
    use std::fs;
    let temp_dir = std::env::temp_dir().join(format!("test_harness_search_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let root = &temp_dir;
    
    // 创建普通文件
    fs::write(root.join("normal.txt"), "TARGET_KEYWORD_MATCH").expect("write normal");
    
    // 创建 target/, node_modules/, .git/ 并写入匹配内容
    let target_dir = root.join("target");
    fs::create_dir_all(&target_dir).expect("create target");
    fs::write(target_dir.join("build.txt"), "TARGET_KEYWORD_MATCH").expect("write target match");

    let nm_dir = root.join("node_modules");
    fs::create_dir_all(&nm_dir).expect("create node_modules");
    fs::write(nm_dir.join("pkg.txt"), "TARGET_KEYWORD_MATCH").expect("write nm match");

    let git_dir = root.join(".git");
    fs::create_dir_all(&git_dir).expect("create .git");
    fs::write(git_dir.join("config.txt"), "TARGET_KEYWORD_MATCH").expect("write git match");

    let engine = SearchEngine::new();
    let opts = SearchOptions::default();
    let result = engine.search_text("TARGET_KEYWORD_MATCH", root, &opts).expect("search text");

    let _ = fs::remove_dir_all(&temp_dir);

    assert_eq!(result.match_count, 1, "Only normal.txt should match, heavy dirs must be ignored");
    assert_eq!(result.matches[0].path, "normal.txt");
}
