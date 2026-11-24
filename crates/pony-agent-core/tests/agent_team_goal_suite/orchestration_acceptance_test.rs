//! 编排类工具的契约验收（W25 起迁移到「诚实能力」契约）。
//!
//! 历史背景：这些测试原本断言 `subagent` / `spawn_teammate` / `team_task_*` 经 `ToolRouter`
//! 调用后 `status == "ok"`，注释写的是「接线后此处转绿」。但审计（docs/audit/agent-core-gap-matrix.md
//! P0-2 / G-02）证实这些 handler 当时只是返回合成字符串的桩，`ok` 是**假回执**：模型会误信
//! 已派生子代理、已产生子任务，实际什么都没发生。
//!
//! W25 契约（docs/audit/agent-core-improvement-plan.md §W25）：
//! 子代理/团队运行时接线（`SUBAGENT_RUNTIME_WIRED`，计划 W29）之前，这些工具一律返回
//! 结构化 `capability_unavailable`，且 hint 指向 improvement-plan / W29。
//!
//! 因此本文件的断言从「期望 ok」翻转为「期望显式不可用」——测试意图不变（验证工具确实
//! 经 ToolRouter 路由到真实 handler），但不再为正名化的假成功背书。

use pony_agent_core::agent::tools::{ToolCall, ToolRouter};
use serde_json::{json, Value};

/// W25 冻结名单：子代理/团队工具（当前未接线）。
const NON_WIRED_TOOLS: [&str; 10] = [
    "subagent",
    "workflow",
    "spawn_teammate",
    "list_agents",
    "send_message",
    "interrupt_agent",
    "drain_inbox",
    "team_task_create",
    "team_task_update",
    "team_task_list",
];

fn make_call(name: &str, args: Value) -> ToolCall {
    ToolCall {
        call_id: None,
        name: name.to_string(),
        arguments: args,
        plan: None,
    }
}

fn sample_arguments(name: &str) -> Value {
    match name {
        "subagent" => json!({
            "prompt": "Inspect auth tokens in workspace",
            "description": "Security audit subagent",
            "run_in_background": false
        }),
        "workflow" => json!({
            "name": "w25-acceptance-workflow",
            "steps": [{ "id": "step-1", "action": "noop", "depends_on": [] }]
        }),
        "spawn_teammate" => json!({
            "name": "reviewer",
            "role": "code-reviewer",
            "description": "Reviews code PRs",
            "prompt": "You are a code reviewer"
        }),
        "send_message" => json!({
            "target": "reviewer",
            "message": "Please review this pull request"
        }),
        "interrupt_agent" => json!({ "target": "reviewer", "reason": "W25 acceptance" }),
        "drain_inbox" => json!({ "target": "reviewer" }),
        "team_task_create" => json!({
            "subject": "Refactor router",
            "description": "Clean up route branches"
        }),
        "team_task_update" => json!({
            "task_id": "task-w25-acceptance-missing",
            "expected_revision": 1,
            "action": "claim",
            "owner": "lead"
        }),
        _ => json!({}),
    }
}

/// 断言单个工具返回 W25 结构化不可用错误（而非假 ok）。
fn assert_capability_unavailable(tool: &str, result: &pony_agent_core::agent::tools::ToolResult) {
    assert_ne!(
        result.status, "ok",
        "`{tool}` 不应再返回假成功回执（W25 契约）：{:?}",
        result
    );

    let parsed: Value = serde_json::from_str(&result.output)
        .unwrap_or_else(|err| panic!("`{tool}` 输出不是合法 JSON（{err}）：{}", result.output));

    let code = parsed
        .pointer("/error/code")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("`{tool}` 缺少 error.code：{}", result.output));
    assert_eq!(
        code, "capability_unavailable",
        "`{tool}` 错误码应为 capability_unavailable，实际 {code}：{}",
        result.output
    );

    let message = parsed
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    assert!(
        !message.trim().is_empty(),
        "`{tool}` error.message 不得为空：{}",
        result.output
    );

    let hint = parsed
        .pointer("/error/hint")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let guidance = format!("{message}\n{hint}").to_lowercase();
    assert!(
        guidance.contains("improvement-plan") || guidance.contains("w29"),
        "`{tool}` 缺少可行动指引（应指向 improvement-plan / W29）：message={message:?} hint={hint:?}"
    );
}

#[test]
fn test_subagent_tool_routed_through_tool_router() {
    let router = ToolRouter::new();
    let call = make_call("subagent", sample_arguments("subagent"));
    let result = router.execute(&call);
    // 路由仍然生效（不是 unsupported_tool），但能力未接线 → 必须显式不可用。
    assert_capability_unavailable("subagent", &result);
}

#[test]
fn test_agent_teams_tools_routed_through_tool_router() {
    let router = ToolRouter::new();
    for name in ["spawn_teammate", "list_agents", "send_message"] {
        let call = make_call(name, sample_arguments(name));
        let result = router.execute(&call);
        assert_capability_unavailable(name, &result);
    }
}

#[test]
fn test_team_task_board_tools_routed_through_tool_router() {
    let router = ToolRouter::new();
    for name in ["team_task_create", "team_task_list"] {
        let call = make_call(name, sample_arguments(name));
        let result = router.execute(&call);
        assert_capability_unavailable(name, &result);
    }
}

/// 全名单覆盖：任何子代理/团队工具都不得出现"假 ok"回退。
#[test]
fn test_all_non_wired_orchestration_tools_are_honest() {
    let router = ToolRouter::new();
    for name in NON_WIRED_TOOLS {
        let call = make_call(name, sample_arguments(name));
        let result = router.execute(&call);
        assert_capability_unavailable(name, &result);
    }
}
