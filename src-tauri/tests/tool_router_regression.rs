use pony_agent_core::agent::tools::{ToolCall, ToolRouter};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_workspace() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("pony-agent-tool-regression-{stamp}"));
    fs::create_dir_all(&dir).expect("create temp workspace");
    dir
}

#[test]
fn search_text_respects_file_pattern_and_skips_node_modules() {
    let workspace = temp_workspace();
    fs::create_dir_all(workspace.join("src")).expect("create src dir");
    fs::create_dir_all(workspace.join("node_modules")).expect("create node_modules");
    fs::write(
        workspace.join("src").join("hit.rs"),
        "const NEEDLE: &str = \"needle\";\n",
    )
    .expect("write hit.rs");
    fs::write(
        workspace.join("node_modules").join("ignored.txt"),
        "needle from dependency\n",
    )
    .expect("write ignored.txt");

    let router = ToolRouter::with_workspace_root(workspace.clone());
    let result = router.execute(&ToolCall {
        call_id: None,
        name: "workspace_search_text".to_string(),
        arguments: json!({
            "query": "needle",
            "path": ".",
            "filePattern": ".rs",
            "limit": 10,
            "ignoreCase": true
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("search output json");
    let matches = payload
        .get("matches")
        .and_then(Value::as_array)
        .expect("matches array");

    assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(1));
    assert_eq!(matches.len(), 1);
    assert_eq!(
        matches[0].get("path").and_then(Value::as_str),
        Some("src/hit.rs")
    );
    assert_eq!(
        payload.get("skippedLargeFiles").and_then(Value::as_u64),
        Some(0)
    );

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn batch_stops_following_calls_when_continue_on_error_is_false() {
    let workspace = temp_workspace();
    fs::write(workspace.join("ok.txt"), "ok\n").expect("write ok.txt");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: "workspace_batch".to_string(),
        arguments: json!({
            "parallel": true,
            "continueOnError": false,
            "calls": [
                {
                    "name": "workspace_read_file",
                    "arguments": { "path": "missing.txt" }
                },
                {
                    "name": "workspace_read_file",
                    "arguments": { "path": "ok.txt" }
                }
            ]
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .expect("batch results");

    assert_eq!(payload.get("status").and_then(Value::as_str), Some("error"));
    assert_eq!(payload.get("abortedCount").and_then(Value::as_u64), Some(1));
    assert_eq!(
        results[0].get("aggregateStatus").and_then(Value::as_str),
        Some("error")
    );
    assert_eq!(
        results[1].get("aggregateStatus").and_then(Value::as_str),
        Some("aborted")
    );

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn gather_context_with_query_on_file_returns_search_and_segment_results() {
    let workspace = temp_workspace();
    fs::write(
        workspace.join("demo.rs"),
        "fn main() {\n    let alpha = 1;\n    let beta = 2;\n    let gamma = 3;\n    let needle = beta + gamma;\n}\n",
    )
    .expect("write demo.rs");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: "workspace_gather_context".to_string(),
        arguments: json!({
            "path": "demo.rs",
            "query": "needle",
            "lineCount": 3,
            "limit": 5
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
    let results = payload
        .get("results")
        .and_then(Value::as_array)
        .expect("gather results");

    assert_eq!(
        payload
            .get("meta")
            .and_then(Value::as_object)
            .and_then(|meta| meta.get("mode"))
            .and_then(Value::as_str),
        Some("search")
    );
    assert!(results.iter().any(|entry| {
        entry.get("tool").and_then(Value::as_str) == Some("workspace_search_text")
    }));
    let segment_output = results
        .iter()
        .find(|entry| {
            entry.get("tool").and_then(Value::as_str) == Some("workspace_read_file_segment")
        })
        .and_then(|entry| entry.get("output"))
        .and_then(Value::as_str)
        .expect("segment output should be plain text");
    assert!(segment_output.contains("needle = beta + gamma"));

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn write_and_edit_tools_work_for_workspace_files() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let write_result = router.execute(&ToolCall {
        call_id: None,
        name: "workspace_write_file".to_string(),
        arguments: json!({
            "path": "docs/plan.txt",
            "content": "alpha\nbeta\n"
        }),
        plan: None,
    });
    assert_eq!(write_result.status, "ok");

    let edit_result = router.execute(&ToolCall {
        call_id: None,
        name: "workspace_edit_file".to_string(),
        arguments: json!({
            "path": "docs/plan.txt",
            "oldText": "beta",
            "newText": "gamma"
        }),
        plan: None,
    });
    assert_eq!(edit_result.status, "ok");
    assert_eq!(
        fs::read_to_string(workspace.join("docs/plan.txt")).expect("read edited file"),
        "alpha\ngamma\n"
    );

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn run_command_returns_structured_result() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: "workspace_run_command".to_string(),
        arguments: json!({
            "command": "echo regression",
            "cwd": ".",
            "timeoutMs": 5000
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
    assert_eq!(payload.get("exitCode").and_then(Value::as_i64), Some(0));
    assert_eq!(payload.get("cwd").and_then(Value::as_str), Some("."));
    assert!(payload
        .get("stdout")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase()
        .contains("regression"));

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn write_tool_rejects_existing_file_when_overwrite_false() {
    let workspace = temp_workspace();
    fs::write(workspace.join("demo.txt"), "original").expect("write original");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: "workspace_write_file".to_string(),
        arguments: json!({
            "path": "demo.txt",
            "content": "changed",
            "overwrite": false
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("write output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("file_exists")
    );
    assert_eq!(
        fs::read_to_string(workspace.join("demo.txt")).expect("read original"),
        "original"
    );

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn glob_files_returns_repo_matches() {
    let workspace = temp_workspace();
    fs::create_dir_all(workspace.join("src/agent")).expect("create agent dir");
    fs::write(workspace.join("src/agent/tools.rs"), "pub fn demo() {}\n").expect("write tools");
    fs::write(
        workspace.join("src/agent/context.rs"),
        "pub struct AgentContext;\n",
    )
    .expect("write context");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: "workspace_glob_files".to_string(),
        arguments: json!({
            "pattern": "src/agent/*.rs",
            "path": ".",
            "limit": 10
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("glob output json");
    assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(2));

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn glob_files_respects_limit() {
    let workspace = temp_workspace();
    fs::create_dir_all(workspace.join("src/agent")).expect("create agent dir");
    fs::write(workspace.join("src/agent/tools.rs"), "pub fn demo() {}\n").expect("write tools");
    fs::write(
        workspace.join("src/agent/context.rs"),
        "pub struct AgentContext;\n",
    )
    .expect("write context");
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: "workspace_glob_files".to_string(),
        arguments: json!({
            "pattern": "src/agent/*.rs",
            "path": ".",
            "limit": 1
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("glob output json");
    assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(1));

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn web_fetch_rejects_non_http_urls() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: "web_fetch_url".to_string(),
        arguments: json!({
            "url": "file:///tmp/demo.txt"
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("web fetch output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("invalid_url")
    );

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn web_search_rejects_empty_query() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: "web_search_query".to_string(),
        arguments: json!({
            "query": "   "
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("web search output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("empty_query")
    );

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn mcp_resource_rejected_by_tool_router() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: "mcp_resource_read".to_string(),
        arguments: json!({
            "capabilityId": "mcp:resource:repo-index"
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("mcp resource output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("deferred_to_registry")
    );

    let _ = fs::remove_dir_all(workspace);
}

#[test]
fn tool_search_rejected_by_tool_router() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: "tool_search".to_string(),
        arguments: json!({
            "query": "workspace"
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("tool search output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("deferred_to_registry")
    );

    let _ = fs::remove_dir_all(workspace);
}
