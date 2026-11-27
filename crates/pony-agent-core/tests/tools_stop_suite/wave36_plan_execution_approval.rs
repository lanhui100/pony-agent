//! W36（2026-11-27 重写）：**Plan 模式审核闸门**取代原 `Draft → Executing` 审批面。
//!
//! 旧契约 `.dev-team/contracts/wave-36-pa138-145.md` §4 冻结的是 `PlanStore` 步骤机的
//! `plan_execution_approval` 证据（绑定 `(session_id, plan_id)` + TTL + 一次性消费）。
//! 步骤机已下线（ADR `2026-11-27-plan-mode-dsh-alignment-and-task-count`），审批面收敛为：
//!
//! | 旧契约 | 新契约（本文件） |
//! |---|---|
//! | 「宿主注入 `plan_execution_approval` 才能 Draft → Executing」 | 「**用户批准 `ExitPlanMode` 提交的方案**才能退出 Plan 模式」 |
//! | 模型参数不得伪造审批（C-141-3） | 模型参数只能**提交方案**（`ExitPlanMode`），批准只能由宿主 `plan_mode_review(true)` 落 |
//! | 一次性 `(session, plan)` 证据 | 审批是一次性状态迁移：批准后 `active=false`，再次进入需重新提交 |
//! | 审批不能解锁普通写工具（C-141-7） | 批准后写/执行工具**立即解锁**；未批准则一直被 `plan_mode_gate` 拦 |
//!
//! 断言 ID（C141′）：审批状态只能由宿主产生、会话隔离、批准前写工具被拦、批准后解锁、
//! 「继续规划」不放大权限、跨会话不得互相批准。

use pony_agent_core::agent::control_plane::HostControlPlaneBuilder;
use pony_agent_core::agent::dispatcher::DispatchContext;
use pony_agent_core::agent::dispatcher_composites::GovernedToolExecutor;
use pony_agent_core::agent::governed_executor::build_governed_executor;
use pony_agent_core::agent::runtime::AgentRuntimeBuilder;
use pony_agent_core::agent::session::SessionStore;
use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolExecutor};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_workspace() -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "pony-wave36-c141-{}-{seq}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create c141 test workspace");
    root
}

fn session(name: &str) -> String {
    format!("c141-{name}-{}", uuid::Uuid::new_v4())
}

fn plan_control_plane() -> pony_agent_core::agent::control_plane::HostControlPlane {
    HostControlPlaneBuilder::new()
        .runtime(
            AgentRuntimeBuilder::new()
                .session_store(SessionStore::memory_only())
                .build(),
        )
        .build()
}

fn write_call(path: &str) -> ToolCall {
    ToolCall {
        call_id: Some(format!("call-{path}")),
        name: "workspace_write_file".to_string(),
        arguments: json!({ "path": path, "content": "hello", "description": "写入测试文件" }),
        plan: None,
    }
}

fn execute_in_session(
    executor: &GovernedToolExecutor,
    session_id: &str,
    call: &ToolCall,
) -> String {
    let exec_context = ToolExecutionContext {
        dispatch_context: Some(DispatchContext {
            session_id: Some(session_id.to_string()),
            ..DispatchContext::default()
        }),
        ..ToolExecutionContext::default()
    };
    executor.execute_with_context(call, &exec_context).status
}

fn error_code(output: &str) -> Option<String> {
    let parsed: Value = serde_json::from_str(output).ok()?;
    parsed
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// C141′-1：审批只能由宿主产生——模型提交方案不等于批准。
#[test]
fn c141_1_submitting_a_plan_does_not_approve_it() {
    let cp = plan_control_plane();
    let target = session("submit-only");
    cp.plan_mode_set(&target, true).expect("enter");
    cp.plan_mode_commit(&target);

    let submitted = cp.plan_mode_get(&target);
    assert_eq!(submitted["active"].as_bool(), Some(true), "提交方案后仍在 Plan 模式");
    assert_eq!(
        submitted["planApproved"].as_bool(),
        Some(false),
        "模型不能自己把方案标记为已批准"
    );
}

/// C141′-2：模型工具参数无法伪造批准（`ExitPlanMode` 只登记方案）。
#[test]
fn c141_2_model_arguments_cannot_forge_approval() {
    let cp = plan_control_plane();
    let target = session("forge");
    cp.plan_mode_set(&target, true).expect("enter");
    cp.plan_mode_commit(&target);

    // 模型没有任何"带 approved 参数"的入口：ExitPlanMode 的 schema 只有 plan。
    let schema = pony_agent_core::agent::tools::builtin_tools()
        .into_iter()
        .find(|tool| tool.name == "plan_control")
        .expect("ExitPlanMode 必须在工具面")
        .input_schema;
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .expect("properties");
    assert!(
        !properties.contains_key("approved"),
        "schema 不得暴露任何批准入参（批准只能来自宿主）: {schema}"
    );
    assert!(properties.contains_key("plan"), "schema 必须只有 plan 入参: {schema}");
}

/// C141′-3：批准前写工具被 `plan_mode_gate` 拦，批准后立即解锁。
#[test]
fn c141_3_approval_unlocks_writes_only_after_the_host_decides() {
    let workspace = temp_workspace();
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let cp = plan_control_plane();
    let target = session("unlock");
    cp.plan_mode_set(&target, true).expect("enter");
    cp.plan_mode_commit(&target);

    let exec_context = ToolExecutionContext {
        dispatch_context: Some(DispatchContext {
            session_id: Some(target.clone()),
            ..DispatchContext::default()
        }),
        ..ToolExecutionContext::default()
    };
    let blocked = executor.execute_with_context(&write_call("before.txt"), &exec_context);
    assert_eq!(blocked.status, "error", "批准前写工具必须被拒: {}", blocked.output);
    assert_eq!(error_code(&blocked.output).as_deref(), Some("permission_denied"));
    assert!(!workspace.join("before.txt").exists(), "拒绝必须发生在副作用之前");

    // 宿主批准（必须先有提交的方案）。
    pony_agent_core::agent::plan_mode::submit_plan(&target, "# 方案");
    cp.plan_mode_review(&target, true).expect("approve");

    let allowed = execute_in_session(&executor, &target, &write_call("after.txt"));
    assert_eq!(allowed, "ok", "批准后写工具必须立刻解锁");
    assert!(workspace.join("after.txt").exists());

    let _ = fs::remove_dir_all(&workspace);
}

/// C141′-4：会话隔离——A 的批准不改变 B 的模式，反之亦然。
#[test]
fn c141_4_approval_is_scoped_to_one_session() {
    let cp = plan_control_plane();
    let session_a = session("scope-a");
    let session_b = session("scope-b");
    cp.plan_mode_set(&session_a, true).expect("enter a");
    cp.plan_mode_commit(&session_a);
    cp.plan_mode_set(&session_b, true).expect("enter b");
    cp.plan_mode_commit(&session_b);

    pony_agent_core::agent::plan_mode::submit_plan(&session_a, "# A 的方案");
    cp.plan_mode_review(&session_a, true).expect("approve a");

    assert_eq!(cp.plan_mode_get(&session_a)["active"].as_bool(), Some(false));
    assert_eq!(
        cp.plan_mode_get(&session_b)["active"].as_bool(),
        Some(true),
        "A 的批准不得让 B 退出 Plan 模式"
    );
}

/// C141′-5：批准是一次性状态迁移——再次进入必须重新提交方案。
#[test]
fn c141_5_approval_is_one_shot_per_plan() {
    let cp = plan_control_plane();
    let target = session("one-shot");
    cp.plan_mode_set(&target, true).expect("enter");
    cp.plan_mode_commit(&target);
    pony_agent_core::agent::plan_mode::submit_plan(&target, "# 第一版方案");
    cp.plan_mode_review(&target, true).expect("approve");

    // 重新进入：方案仍是上一份（已批准标记保留），但模式重新激活后必须重新走审核。
    cp.plan_mode_set(&target, true).expect("re-enter");
    cp.plan_mode_commit(&target);
    let state = cp.plan_mode_get(&target);
    assert_eq!(state["active"].as_bool(), Some(true), "重新进入必须重新锁定写权限");
    assert_eq!(
        state["planApproved"].as_bool(),
        Some(false),
        "重新进入后上一份方案不再算「已批准」"
    );
}

/// C141′-6：「继续规划」不放大权限——模式保持激活，写工具仍被拦。
#[test]
fn c141_6_keep_planning_keeps_the_write_gate_closed() {
    let workspace = temp_workspace();
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let cp = plan_control_plane();
    let target = session("keep");
    cp.plan_mode_set(&target, true).expect("enter");
    cp.plan_mode_commit(&target);
    pony_agent_core::agent::plan_mode::submit_plan(&target, "# 草案");

    cp.plan_mode_review(&target, false).expect("keep planning");
    let status = execute_in_session(&executor, &target, &write_call("still-blocked.txt"));
    assert_eq!(status, "error", "「继续规划」必须保持写门禁关闭");
    assert!(!workspace.join("still-blocked.txt").exists());

    let _ = fs::remove_dir_all(&workspace);
}

/// C141′-7：没有提交方案时批准必须 fail-closed（不许空批准换解锁）。
#[test]
fn c141_7_review_without_a_plan_fails_closed() {
    let cp = plan_control_plane();
    let target = session("no-plan");
    cp.plan_mode_set(&target, true).expect("enter");
    cp.plan_mode_commit(&target);

    let error = cp
        .plan_mode_review(&target, true)
        .expect_err("无方案不得批准");
    assert!(error.starts_with("plan_review_no_plan:"), "{error}");
    assert_eq!(
        cp.plan_mode_get(&target)["active"].as_bool(),
        Some(true),
        "失败的批准不得改变模式状态"
    );
}
