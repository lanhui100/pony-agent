//! W30b 验收测试：工具级规则权限评估器（ToolRulesPolicyEvaluator）与 Plan 状态门禁
//!
//! 契约要求：
//! 1. 工具级 allow / ask / deny 规则评估器：
//!    - 支持精确匹配、通配符前缀匹配与正则匹配工具名称
//!    - 命中 deny 规则返回 `PermissionVerdict::Deny`，附带决策原因
//!    - 命中 ask 规则返回 `PermissionVerdict::ApprovalRequired`
//!    - 命中 allow 规则返回 `PermissionVerdict::Allow`
//!    - 默认策略回退（如未命中规则且无 Ask 介入时继承默认行为）
//! 2. Plan 相位只读门禁：
//!    - 在 Plan 为 Draft（草稿）状态下，写类工具（如 Write、Edit、Run）被拒绝（`PermissionVerdict::Deny`）
//!    - 只读工具（如 Read、Search、Glob、List）在 Draft 态下正常放行（`PermissionVerdict::Allow`）
//!    - 在 Plan 推进到 Executing（执行态）后，写类工具被放行解除只读门禁
//! 3. Todo 关联 Plan ID：
//!    - `TodoWriteArgs` 与 `TodoItem` 支持关联 `plan_id` / `step_id`
//!    - 读写往返保证 `plan_id` 字段不丢失

use pony_agent_core::agent::app_settings::{RuleAction, ToolRule, ToolRuleMatcher};
use pony_agent_core::agent::dispatcher::{
    PermissionVerdict, ToolPolicyEvaluator,
};
use pony_agent_core::agent::tool_runtime::InvocationOrigin;
use pony_agent_core::agent::governed_executor::GovernedPolicyEvaluator;
use pony_agent_core::agent::plan_state::{PlanLifecycleState, PlanPayload, PlanStepSpec, PlanStore};
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

    let evaluator = GovernedPolicyEvaluator::new(rules, None);
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

#[test]
fn test_plan_draft_mode_blocks_write_tools_and_allows_read_tools() {
    let plan_store = Arc::new(PlanStore::new());
    let session_id = "test-session-draft-gate";

    // 创建一个处于 Draft 状态的 Plan
    let plan = plan_store
        .create(
            session_id,
            PlanPayload {
                kind: "task".to_string(),
                summary: "Refactor core".to_string(),
                steps: vec![PlanStepSpec {
                    name: "step 1".to_string(),
                    summary: "read code".to_string(),
                }],
            },
        )
        .expect("create plan");
    assert_eq!(plan.lifecycle, PlanLifecycleState::Draft);

    let evaluator = GovernedPolicyEvaluator::new(vec![], Some(plan_store.clone()));
    let origin = InvocationOrigin::Model;

    // 1. 写类工具在 Draft 状态下被拦截（Deny）
    let write_desc = dummy_descriptor("Write", ToolKind::Write);
    let decision = evaluator.evaluate(&write_desc, &origin, &json!({}));
    assert_eq!(decision.verdict, PermissionVerdict::Deny);
    assert_eq!(decision.decision_source, "plan_draft_gate");
    assert!(decision.reason.unwrap().contains("Draft"));

    // 2. 执行类工具（Run）在 Draft 状态下被拦截
    let exec_desc = dummy_descriptor("Run", ToolKind::Execute);
    let decision = evaluator.evaluate(&exec_desc, &origin, &json!({}));
    assert_eq!(decision.verdict, PermissionVerdict::Deny);
    assert_eq!(decision.decision_source, "plan_draft_gate");

    // 3. 读类工具在 Draft 状态下正常放行
    let read_desc = dummy_descriptor("Read", ToolKind::Read);
    let decision = evaluator.evaluate(&read_desc, &origin, &json!({}));
    assert_eq!(decision.verdict, PermissionVerdict::Allow);

    // 4. 推进 Plan 到 Executing 状态后，写类工具解锁
    plan_store
        .start_execution(session_id, &plan.plan_id, plan.revision)
        .expect("start execution");
    let write_decision_after = evaluator.evaluate(&write_desc, &origin, &json!({}));
    assert_eq!(write_decision_after.verdict, PermissionVerdict::Allow);
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
