//! W30b 验收测试：工具级规则权限评估器（ToolRulesPolicyEvaluator）与 Plan 状态门禁
//!
//! 契约要求：
//! 1. 工具级 allow / ask / deny 规则评估器：
//!    - 支持精确匹配、通配符前缀匹配与正则匹配工具名称
//!    - 命中 deny 规则返回 `PermissionVerdict::Deny`，附带决策原因
//!    - 命中 ask 规则返回 `PermissionVerdict::ApprovalRequired`
//!    - 命中 allow 规则返回 `PermissionVerdict::Allow`
//!    - 默认策略回退（如未命中规则且无 Ask 介入时继承默认行为）
//! 2. Plan 模式硬拦截（ADR 2026-11-27-plan-mode-dsh-alignment-and-task-count）：
//!    - 会话处于 Plan 模式时，写类工具（Write、Edit、Run）被拒绝（`PermissionVerdict::Deny`）
//!    - 只读工具（Read、Search、Glob、List）在 Plan 模式下正常放行（`PermissionVerdict::Allow`）
//!    - 用户批准方案（`ExitPlanMode` 审核通过）或 `/plan off` 后，写类工具解锁
//! 3. Todo 关联 Plan ID：
//!    - `TodoWriteArgs` 与 `TodoItem` 支持关联 `plan_id` / `step_id`
//!    - 读写往返保证 `plan_id` 字段不丢失

use pony_agent_core::agent::app_settings::{RuleAction, ToolRule, ToolRuleMatcher};
use pony_agent_core::agent::dispatcher::{
    DispatchContext, PermissionVerdict, ToolPolicyEvaluator,
};
use pony_agent_core::agent::tool_runtime::InvocationOrigin;
use pony_agent_core::agent::governed_executor::GovernedPolicyEvaluator;
use pony_agent_core::agent::todo::{todo_list, todo_write, TodoItem, TodoListArgs, TodoStatus, TodoWriteArgs};
use pony_agent_core::agent::tools::{
    ToolDescriptor, ToolDescriptorSource, ToolExposure, ToolHandlerProvenance, ToolIdentity, ToolKind,
    ToolPermissionDeclaration,
};
use serde_json::json;
use std::sync::Arc;

fn dummy_descriptor(name: &str, kind: ToolKind) -> ToolDescriptor {
    ToolDescriptor {
        identity: ToolIdentity {
            descriptor_id: name.to_string(),
            canonical_name: name.to_string(),
            primitive_name: name.to_string(),
            model_name: name.to_string(),
            source: ToolDescriptorSource::Builtin,
        },
        aliases: vec![],
        description: format!("dummy {name}"),
        input_schema: json!({"type": "object"}),
        kind,
        exposure: ToolExposure::ModelVisible,
        permission_declaration: ToolPermissionDeclaration {
            requires_approval: false,
            host_mediated: false,
            scopes: std::collections::BTreeSet::new(),
            approval_mode: None,
        },
        execution_policy: Default::default(),
        display_metadata: Default::default(),
        handler_provenance: ToolHandlerProvenance {
            handler_kind: "test".to_string(),
            source_id: "test".to_string(),
        },
        source_revision: "v1".to_string(),
        composed_descriptor_ids: vec![],
    }
}

#[test]
fn test_tool_rule_matching_and_actions() {
    let rules = vec![
        ToolRule {
            matcher: ToolRuleMatcher::Exact("dangerous_exec".to_string()),
            action: RuleAction::Deny,
            reason: Some("Dangerous execution prohibited".to_string()),
        },
        ToolRule {
            matcher: ToolRuleMatcher::Prefix("ask_".to_string()),
            action: RuleAction::Ask,
            reason: Some("Requires user confirmation".to_string()),
        },
        ToolRule {
            matcher: ToolRuleMatcher::Exact("trusted_write".to_string()),
            action: RuleAction::Allow,
            reason: None,
        },
    ];

    let evaluator = GovernedPolicyEvaluator::new(rules);
    let origin = InvocationOrigin::Model;

    // 1. Exact Deny
    let desc_deny = dummy_descriptor("dangerous_exec", ToolKind::Execute);
    let decision = evaluator.evaluate(&desc_deny, &origin, &json!({}));
    assert_eq!(decision.verdict, PermissionVerdict::Deny);
    assert_eq!(decision.decision_source, "tool_rule_deny");
    assert!(decision.reason.unwrap().contains("Dangerous execution"));

    // 2. Prefix Ask (ApprovalRequired)
    let desc_ask = dummy_descriptor("ask_something", ToolKind::Write);
    let decision = evaluator.evaluate(&desc_ask, &origin, &json!({}));
    assert_eq!(decision.verdict, PermissionVerdict::ApprovalRequired);
    assert_eq!(decision.decision_source, "tool_rule_ask");

    // 3. Exact Allow
    let desc_allow = dummy_descriptor("trusted_write", ToolKind::Write);
    let decision = evaluator.evaluate(&desc_allow, &origin, &json!({}));
    assert_eq!(decision.verdict, PermissionVerdict::Allow);
    assert_eq!(decision.decision_source, "tool_rule_allow");

    // 4. Fallback Default
    let desc_fallback = dummy_descriptor("normal_read", ToolKind::Read);
    let decision = evaluator.evaluate(&desc_fallback, &origin, &json!({}));
    assert_eq!(decision.verdict, PermissionVerdict::Allow);
}

/// Plan 模式硬拦截：写/执行工具 Deny，读工具放行；退出模式后解锁。
///
/// 会话维度必须经 `DispatchContext.session_id` 注入（模型实参不是会话键），故本用例直接
/// 调用 `evaluate_with_context` 注入 `DispatchContext`——无上下文的 `evaluate` 按「未激活」处理。
#[test]
fn test_plan_mode_blocks_write_tools_and_allows_read_tools() {
    let session_id = format!("test-session-plan-mode-{}", uuid::Uuid::new_v4());
    let evaluator = GovernedPolicyEvaluator::new(vec![]);
    let origin = InvocationOrigin::Model;

    // 1. 未进入 Plan 模式：写工具放行（默认策略）。
    let write_desc = dummy_descriptor("Write", ToolKind::Write);
    let before = evaluator.evaluate(&write_desc, &origin, &json!({}));
    assert_eq!(before.verdict, PermissionVerdict::Allow);

    // 2. 进入 Plan 模式后：写/执行工具被拒绝，来源可追溯。
    pony_agent_core::agent::plan_mode::set_active(&session_id, true);
    let context = DispatchContext {
        session_id: Some(session_id.clone()),
        ..DispatchContext::default()
    };
    let write_decision = evaluator.evaluate_with_context(
        &write_desc,
        &origin,
        &json!({}),
        &context,
    );
    assert_eq!(write_decision.verdict, PermissionVerdict::Deny);
    assert_eq!(write_decision.decision_source, "plan_mode_gate");

    let exec_desc = dummy_descriptor("Run", ToolKind::Execute);
    let exec_decision =
        evaluator.evaluate_with_context(&exec_desc, &origin, &json!({}), &context);
    assert_eq!(exec_decision.verdict, PermissionVerdict::Deny);
    assert_eq!(exec_decision.decision_source, "plan_mode_gate");

    // 3. 读类工具在 Plan 模式下照常放行（只锁写/执行）。
    let read_desc = dummy_descriptor("Read", ToolKind::Read);
    let read_decision =
        evaluator.evaluate_with_context(&read_desc, &origin, &json!({}), &context);
    assert_eq!(read_decision.verdict, PermissionVerdict::Allow);

    // 4. 退出 Plan 模式（批准或 /plan off）后写工具解锁。
    pony_agent_core::agent::plan_mode::set_active(&session_id, false);
    let after = evaluator.evaluate_with_context(&write_desc, &origin, &json!({}), &context);
    assert_eq!(after.verdict, PermissionVerdict::Allow);
}

#[test]
fn test_todo_associated_with_plan_id() {
    let session_id = "test-session-todo-plan";

    let todo_items = vec![
        TodoItem {
            content: "Check module interfaces".to_string(),
            status: TodoStatus::InProgress,
            plan_id: Some("plan-100".to_string()),
            step_id: Some("step-1".to_string()),
        },
        TodoItem {
            content: "Update tests".to_string(),
            status: TodoStatus::Pending,
            plan_id: Some("plan-100".to_string()),
            step_id: None,
        },
    ];

    let write_res = todo_write(TodoWriteArgs {
        todos: todo_items.clone(),
        session_id: Some(session_id.to_string()),
    })
    .expect("write todo");
    assert_eq!(write_res.count, 2);

    let list_res = todo_list(TodoListArgs {
        session_id: Some(session_id.to_string()),
    })
    .expect("list todo");
    assert_eq!(list_res.todos.len(), 2);
    assert_eq!(list_res.todos[0].plan_id, Some("plan-100".to_string()));
    assert_eq!(list_res.todos[0].step_id, Some("step-1".to_string()));
    assert_eq!(list_res.todos[1].plan_id, Some("plan-100".to_string()));
    assert_eq!(list_res.todos[1].step_id, None);
}
