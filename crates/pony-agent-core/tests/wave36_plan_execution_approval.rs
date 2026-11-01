//! W36 / PA-141 契约测试：Draft 态「宿主审批」闸门（用户裁定方案 (b)，矩阵 §0 supersede）。
//!
//! 冻结契约：`.dev-team/contracts/wave-36-pa138-145.md` §4（C-141-1..9）。
//! 公共 seam 由 executor-a 落盘（`ToolPolicyEvaluator::evaluate_with_context`、
//! `PlanStore::has_any_draft_plan`）；**审批证据类型与 `DispatchContext`/`TurnInput` 字段
//! 由 executor-b 落地**。因此本文件当前以**编译红**（E0609/E0560/E0425：字段/类型不存在）
//! 落盘 —— 属 C-141-1 允许的合法红相，原始 cargo stderr 收据：
//! `tests/acceptance/receipts/wave36/T-W36-141-01_compile_red.log`。
//!
//! 设计约束（不得偏离）：
//! - 审批只能经 `DispatchContext`（宿主注入）传递；**模型工具参数不是来源**（C-141-3）。
//! - 绑定 `(session_id, plan_id)` + TTL（`now_ms < expires_at_ms` 才有效）（C-141-4/5）。
//! - 无审批 / 不匹配 / 过期 ⇒ `Deny`，`error.code == "plan_execution_approval_required"`。
//! - 审批只解锁 `Draft→Executing` 迁移；普通写工具在 Draft 仍 `plan_draft_gate`（C-141-7）。
//! - 只使用公开 API，不触碰私有字段。

use pony_agent_core::agent::dispatcher::{
    DispatchContext, GovernedDispatcher, PermissionDecision, PermissionVerdict, ToolPolicyEvaluator,
};
use pony_agent_core::agent::governed_executor::build_governed_executor;
use pony_agent_core::agent::runtime::TurnInput;
use pony_agent_core::agent::dispatcher_composites::GovernedToolExecutor;
use pony_agent_core::agent::tool_runtime::FakeClock;
use pony_agent_core::agent::tools::{
    ToolCall, ToolDescriptor, ToolExecutionContext, ToolExecutor, ToolRegistrySnapshot, ToolResult,
};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

const SESSION_A: &str = "sess-A";
const SESSION_B: &str = "sess-B";
const DENY_CODE: &str = "plan_execution_approval_required";

fn temp_workspace() -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "pony-wave36-pa141-{}-{seq}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create pa141 test workspace");
    root
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock must be after UNIX_EPOCH")
        .as_millis() as u64
}

/// 宿主审批证据的 JSON 形状（camelCase，冻结）。
fn approval_json(session_id: &str, plan_id: &str, expires_at_ms: u64) -> Value {
    json!({
        "sessionId": session_id,
        "planId": plan_id,
        "expiresAtMs": expires_at_ms,
    })
}

/// 构造 `DispatchContext`（只经公开 serde 面；`planExecutionApproval` 为宿主字段）。
fn context(session_id: Option<&str>, plan_approval: Option<Value>) -> DispatchContext {
    let mut value = json!({ "sessionId": session_id });
    if let Some(approval) = plan_approval {
        value["planExecutionApproval"] = approval;
    }
    serde_json::from_value(value)
        .expect("DispatchContext must deserialize host planExecutionApproval (camelCase)")
}

fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        call_id: None,
        name: name.to_string(),
        arguments,
        plan: None,
    }
}

fn parse_output(output: &str) -> Value {
    serde_json::from_str(output).unwrap_or_else(|error| {
        panic!("tool output must be JSON: {error}; raw={output}");
    })
}

fn error_code(payload: &Value) -> String {
    payload["error"]["code"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

fn error_message(payload: &Value) -> String {
    payload["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// 经真实 Plan 工具在 `session_id` 下创建 Draft，返回 plan JSON 投影。
fn create_draft(executor: &impl ToolExecutor, session_id: &str) -> Value {
    let created = executor.execute(&call(
        "plan_control",
        json!({
            "op": "create",
            "session_id": session_id,
            "description": "PA-141 approval-gate fixture plan",
            "payload": {
                "kind": "task",
                "summary": "host approval gate",
                "steps": [
                    { "name": "step-1", "summary": "first" },
                    { "name": "step-2", "summary": "second" }
                ]
            }
        }),
    ));
    assert_eq!(
        created.status, "ok",
        "Plan create must succeed under the injected session: {}",
        created.output
    );
    let plan = parse_output(&created.output);
    assert_eq!(plan["lifecycle"], "draft", "fixture must be Draft: {plan}");
    assert!(
        plan["planId"].as_str().is_some_and(|id| !id.is_empty()),
        "plan projection must carry planId: {plan}"
    );
    plan
}

fn start_execution_args(session_id: &str, plan: &Value) -> Value {
    json!({
        "op": "start_execution",
        "session_id": session_id,
        "plan_id": plan["planId"],
        "revision": plan["revision"],
        "description": "PA-141 start_execution probe",
    })
}

fn complete_step_args(session_id: &str, plan: &Value, step_index: usize) -> Value {
    json!({
        "op": "complete_step",
        "session_id": session_id,
        "plan_id": plan["planId"],
        "revision": plan["revision"],
        "step_id": plan["steps"][step_index]["stepId"],
        "description": "PA-141 complete_step probe",
    })
}

fn assert_approval_denied(result_status: &str, payload: &Value, context_label: &str) {
    assert_eq!(
        result_status, "error",
        "{context_label}: unapproved start_execution/complete_step must fail: {payload}"
    );
    assert_eq!(
        error_code(payload),
        DENY_CODE,
        "{context_label}: must surface {DENY_CODE}: {payload}"
    );
}

// ── C-141-1：公共 seam 存在且命名/序列化冻结 ────────────────────────────────────────────────

#[test]
fn c141_1_approval_seam_is_host_bound_and_serde_frozen() {
    // (a) DispatchContext JSON 携带宿主审批（camelCase）并可读回字段。
    let ctx = context(
        Some(SESSION_A),
        Some(approval_json(SESSION_A, "plan-1", 1_700_000_000_000)),
    );
    let approval = ctx
        .plan_execution_approval
        .as_ref()
        .expect("planExecutionApproval must deserialize into Some(..)");
    assert_eq!(approval.session_id, SESSION_A);
    assert_eq!(approval.plan_id, "plan-1");
    assert_eq!(approval.expires_at_ms, 1_700_000_000_000);
    // camelCase JSON 键冻结。
    let encoded = serde_json::to_value(approval).expect("approval must serialize");
    assert_eq!(encoded["sessionId"], SESSION_A);
    assert_eq!(encoded["planId"], "plan-1");
    assert_eq!(encoded["expiresAtMs"], 1_700_000_000_000u64);

    // (b) 缺省为 None（不得隐式授予）。
    let plain = context(Some(SESSION_A), None);
    assert!(
        plain.plan_execution_approval.is_none(),
        "approval must default to None"
    );

    // (c) 生产可观察行为（**契约迁移**）：原断言经 `GovernedToolExecutor::set_context` /
    //     `context()` 这对访问器做往返；F2 已删除该共享上下文槽位（turn 事实改为 per-call
    //     经 `ToolExecutionContext.dispatch_context` 携带），访问器不复存在。故改为断言真正
    //     可观察的生产行为：per-call `dispatch_context` 必须抵达治理链的策略判定步
    //     （`ToolPolicyEvaluator::evaluate_with_context`），且两次调用互不污染。
    let recorder = Arc::new(TurnFactsRecorder::default());
    // 临时工作区：per-call 调用仍锚定在真实工作区（末尾统一清理）。
    let workspace = temp_workspace();
    let dispatcher = GovernedDispatcher::new(
        Arc::new(ToolRegistrySnapshot::builtin().expect("builtin registry must validate")),
        Arc::new(FakeClock::new(0)),
    );
    dispatcher.register_policy_evaluator(Arc::clone(&recorder) as Arc<dyn ToolPolicyEvaluator>);
    let scoped = GovernedToolExecutor::new(dispatcher);
    let probe = call(
        "workspace_list_files",
        json!({ "description": "C-141-1 per-call turn facts probe", "path": "." }),
    );
    let _ = scoped.execute_with_context(
        &probe,
        &ToolExecutionContext {
            session_id: Some(SESSION_A.to_string()),
            workspace_root: Some(workspace.clone()),
            dispatch_context: Some(ctx.clone()),
            ..Default::default()
        },
    );
    let _ = scoped.execute_with_context(
        &probe,
        &ToolExecutionContext {
            session_id: Some(SESSION_B.to_string()),
            workspace_root: Some(workspace.clone()),
            dispatch_context: Some(context(Some(SESSION_B), None)),
            ..Default::default()
        },
    );

    let observed = recorder.snapshot();
    assert_eq!(
        observed.len(),
        2,
        "每次调用都必须抵达策略判定步，否则 per-call turn 事实无从生效: {observed:?}"
    );
    assert_eq!(
        observed[0].approval,
        Some((SESSION_A.to_string(), "plan-1".to_string())),
        "第一次调用必须看到自己携带的宿主审批证据: {observed:?}"
    );
    assert_eq!(
        observed[0].session_id.as_deref(),
        Some(SESSION_A),
        "第一次调用的会话事实必须原样抵达判定步: {observed:?}"
    );
    assert_eq!(
        observed[1].approval, None,
        "第二次调用不得继承上一次调用的审批证据（共享槽位已删除，任何串扰都是回归）: {observed:?}"
    );
    assert_eq!(
        observed[1].session_id.as_deref(),
        Some(SESSION_B),
        "第二次调用的会话事实必须原样抵达判定步: {observed:?}"
    );

    // (d) TurnInput 同名宿主字段（JSON camelCase）反序列化后可见。
    let input: TurnInput = serde_json::from_value(json!({
        "message": "continue",
        "sessionId": SESSION_A,
        "planExecutionApproval": approval_json(SESSION_A, "plan-1", 1_700_000_000_000),
    }))
    .expect("TurnInput JSON must deserialize the host approval");
    assert_eq!(input.session_id.as_deref(), Some(SESSION_A));
    assert!(
        input.plan_execution_approval.is_some(),
        "TurnInput must carry the host approval (host-only, never model args)"
    );
    let no_approval: TurnInput = serde_json::from_value(json!({
        "message": "continue",
        "sessionId": SESSION_A,
    }))
    .expect("TurnInput without approval must still deserialize");
    assert!(no_approval.plan_execution_approval.is_none());

    let _ = fs::remove_dir_all(workspace);
}

// ── C-141-2：无审批 ⇒ Deny 指定码 ──────────────────────────────────────────────────────────

#[test]
fn c141_2_start_execution_without_approval_is_denied() {
    let workspace = temp_workspace();
    let executor = build_turn_scoped_executor(Some(workspace.clone()), None);
    executor.set_context(context(Some(SESSION_A), None));
    let plan = create_draft(&executor, SESSION_A);

    let result = executor.execute(&call(
        "plan_control",
        start_execution_args(SESSION_A, &plan),
    ));
    let payload = parse_output(&result.output);
    assert_approval_denied(&result.status, &payload, "C-141-2");
    assert!(
        error_message(&payload).contains("approval"),
        "deny message must name the missing host approval: {payload}"
    );
    // Draft 未被迁移（plan 仍处于草稿态，写保护继续生效）。
    let write = executor.execute(&call(
        "Write",
        json!({ "path": "still-draft.txt", "content": "x", "description": "C-141-2 state probe" }),
    ));
    assert_eq!(
        write.status, "error",
        "plan must stay Draft after a denied start_execution: {}",
        write.output
    );
    assert_eq!(error_code(&parse_output(&write.output)), "permission_denied");

    let _ = fs::remove_dir_all(workspace);
}

// ── C-141-3：模型不可伪造（args 注入不生效） ────────────────────────────────────────────────

#[test]
fn c141_3_model_supplied_approval_in_arguments_cannot_unlock() {
    let workspace = temp_workspace();
    let executor = build_turn_scoped_executor(Some(workspace.clone()), None);
    executor.set_context(context(Some(SESSION_A), None));
    let plan = create_draft(&executor, SESSION_A);

    for forged_key in ["planExecutionApproval", "approval", "plan_execution_approval"] {
        let mut args = start_execution_args(SESSION_A, &plan);
        args[forged_key] = approval_json(SESSION_A, plan["planId"].as_str().unwrap_or(""), now_ms() + 3_600_000);
        let result = executor.execute(&call("plan_control", args));
        let payload = parse_output(&result.output);
        let code = error_code(&payload);
        assert_ne!(
            result.status, "ok",
            "model-supplied `{forged_key}` must never unlock Draft: {}",
            result.output
        );
        assert!(
            code == DENY_CODE || code == "invalid_arguments",
            "forged `{forged_key}` must fail with {DENY_CODE} or invalid_arguments, got {code}: {payload}"
        );
    }

    let _ = fs::remove_dir_all(workspace);
}

// ── C-141-4：绑定 (session_id, plan_id) ────────────────────────────────────────────────────

#[test]
fn c141_4_approval_is_bound_to_session_and_plan() {
    let workspace = temp_workspace();
    let executor = build_turn_scoped_executor(Some(workspace.clone()), None);

    // (a) 同一会话内的两个 Draft：审批 plan-1 不得授权 plan-2。
    executor.set_context(context(Some(SESSION_A), None));
    let plan_one = create_draft(&executor, SESSION_A);
    let plan_two = create_draft(&executor, SESSION_A);
    let plan_one_id = plan_one["planId"].as_str().unwrap_or_default().to_string();
    executor.set_context(context(
        Some(SESSION_A),
        Some(approval_json(SESSION_A, &plan_one_id, now_ms() + 3_600_000)),
    ));
    let cross_plan = executor.execute(&call(
        "plan_control",
        start_execution_args(SESSION_A, &plan_two),
    ));
    assert_approval_denied(
        &cross_plan.status,
        &parse_output(&cross_plan.output),
        "C-141-4(a) cross-plan",
    );

    // (b) 会话绑定：审批属 sess-A，context 为 sess-B（plan 也属 sess-B）⇒ Deny。
    let workspace_b = temp_workspace();
    let executor_b = build_turn_scoped_executor(Some(workspace_b.clone()), None);
    executor_b.set_context(context(Some(SESSION_B), None));
    let plan_b = create_draft(&executor_b, SESSION_B);
    let plan_b_id = plan_b["planId"].as_str().unwrap_or_default().to_string();
    executor_b.set_context(context(
        Some(SESSION_B),
        Some(approval_json(SESSION_A, &plan_b_id, now_ms() + 3_600_000)),
    ));
    let cross_session = executor_b.execute(&call(
        "plan_control",
        start_execution_args(SESSION_B, &plan_b),
    ));
    assert_approval_denied(
        &cross_session.status,
        &parse_output(&cross_session.output),
        "C-141-4(b) cross-session",
    );

    let _ = fs::remove_dir_all(workspace);
    let _ = fs::remove_dir_all(workspace_b);
}

// ── C-141-5：TTL（now_ms < expires_at_ms 才有效） ─────────────────────────────────────────

#[test]
fn c141_5_approval_ttl_expiry_denies_and_future_expiry_unlocks() {
    let workspace = temp_workspace();
    let executor = build_turn_scoped_executor(Some(workspace.clone()), None);
    executor.set_context(context(Some(SESSION_A), None));
    let plan = create_draft(&executor, SESSION_A);
    let plan_id = plan["planId"].as_str().unwrap_or_default().to_string();

    // 过期：expires = now-1 ⇒ Deny 指定码（plan 仍 Draft）。
    executor.set_context(context(
        Some(SESSION_A),
        Some(approval_json(SESSION_A, &plan_id, now_ms().saturating_sub(1))),
    ));
    let expired = executor.execute(&call(
        "plan_control",
        start_execution_args(SESSION_A, &plan),
    ));
    assert_approval_denied(
        &expired.status,
        &parse_output(&expired.output),
        "C-141-5 expired",
    );

    // 未过期：expires = now + 1h ⇒ ok 且 lifecycle == executing。
    executor.set_context(context(
        Some(SESSION_A),
        Some(approval_json(SESSION_A, &plan_id, now_ms() + 3_600_000)),
    ));
    let started = executor.execute(&call(
        "plan_control",
        start_execution_args(SESSION_A, &plan),
    ));
    assert_eq!(
        started.status, "ok",
        "valid approval must unlock Draft→Executing: {}",
        started.output
    );
    assert_eq!(
        parse_output(&started.output)["lifecycle"],
        "executing",
        "approved start_execution must transition the plan"
    );

    let _ = fs::remove_dir_all(workspace);
}

// ── C-141-6：一次性（不得成为会话级通行证） ────────────────────────────────────────────────

#[test]
fn c141_6_approval_is_one_shot_per_plan_not_a_session_pass() {
    let workspace = temp_workspace();
    let executor = build_turn_scoped_executor(Some(workspace.clone()), None);
    executor.set_context(context(Some(SESSION_A), None));
    let plan_one = create_draft(&executor, SESSION_A);
    let plan_one_id = plan_one["planId"].as_str().unwrap_or_default().to_string();
    let approval = approval_json(SESSION_A, &plan_one_id, now_ms() + 3_600_000);

    // 首次：合法审批 ⇒ P1 迁移成功。
    executor.set_context(context(Some(SESSION_A), Some(approval.clone())));
    let first = executor.execute(&call(
        "plan_control",
        start_execution_args(SESSION_A, &plan_one),
    ));
    assert_eq!(
        first.status, "ok",
        "first approved start_execution must succeed: {}",
        first.output
    );
    assert_eq!(parse_output(&first.output)["lifecycle"], "executing");

    // 同一审批不得授权同会话的另一个 Draft（P2）。
    let plan_two = create_draft(&executor, SESSION_A);
    let second = executor.execute(&call(
        "plan_control",
        start_execution_args(SESSION_A, &plan_two),
    ));
    assert_approval_denied(
        &second.status,
        &parse_output(&second.output),
        "C-141-6 reuse on another plan",
    );

    let _ = fs::remove_dir_all(workspace);
}

// ── C-141-7：不削弱 PA-138（审批不放宽普通写） ─────────────────────────────────────────────

#[test]
fn c141_7_approval_does_not_weaken_the_draft_write_gate() {
    let workspace = temp_workspace();
    let executor = build_turn_scoped_executor(Some(workspace.clone()), None);
    executor.set_context(context(Some(SESSION_A), None));
    let plan = create_draft(&executor, SESSION_A);
    let plan_id = plan["planId"].as_str().unwrap_or_default().to_string();
    executor.set_context(context(
        Some(SESSION_A),
        Some(approval_json(SESSION_A, &plan_id, now_ms() + 3_600_000)),
    ));

    let write = executor.execute(&call(
        "Write",
        json!({ "path": "approved-but-draft.txt", "content": "x", "description": "C-141-7 probe" }),
    ));
    assert_eq!(
        write.status, "error",
        "a host approval must not unlock ordinary writes while Draft: {}",
        write.output
    );
    let payload = parse_output(&write.output);
    assert_eq!(error_code(&payload), "permission_denied", "{payload}");
    assert!(
        error_message(&payload).contains("plan_draft_gate"),
        "ordinary write deny must stay attributed to plan_draft_gate: {payload}"
    );
    assert!(!workspace.join("approved-but-draft.txt").exists());

    let _ = fs::remove_dir_all(workspace);
}

// ── C-141-8：complete_step 迁移同审批（Draft），Executing 不受影响 ──────────────────────────

#[test]
fn c141_8_complete_step_migration_requires_approval_only_in_draft() {
    let workspace = temp_workspace();
    let executor = build_turn_scoped_executor(Some(workspace.clone()), None);
    executor.set_context(context(Some(SESSION_A), None));
    let plan = create_draft(&executor, SESSION_A);
    let plan_id = plan["planId"].as_str().unwrap_or_default().to_string();

    // 无审批：Draft 上的 complete_step（第二条自解锁路径）必须 Deny 指定码。
    let unapproved = executor.execute(&call(
        "plan_control",
        complete_step_args(SESSION_A, &plan, 0),
    ));
    assert_approval_denied(
        &unapproved.status,
        &parse_output(&unapproved.output),
        "C-141-8 unapproved complete_step",
    );

    // 有审批：完成 step-1 ⇒ 迁移到 Executing。
    executor.set_context(context(
        Some(SESSION_A),
        Some(approval_json(SESSION_A, &plan_id, now_ms() + 3_600_000)),
    ));
    let approved = executor.execute(&call(
        "plan_control",
        complete_step_args(SESSION_A, &plan, 0),
    ));
    assert_eq!(
        approved.status, "ok",
        "approved complete_step must succeed: {}",
        approved.output
    );
    let executing = parse_output(&approved.output);
    assert_eq!(executing["lifecycle"], "executing");

    // Executing 状态的 complete_step 不再需要审批（仅迁移受影响）。
    // 注：两步 plan 完成第二步 ⇒ 既有 PlanStore 语义「全部 step 完成 ⇒ lifecycle=Completed」
    // （plan_state.rs:330，被 plan_state 冻结单测锁定）；本断言即锁定该终态。
    let next = executor.execute(&call(
        "plan_control",
        complete_step_args(SESSION_A, &executing, 1),
    ));
    assert_eq!(
        next.status, "ok",
        "complete_step on Executing must not require approval: {}",
        next.output
    );
    assert_eq!(parse_output(&next.output)["lifecycle"], "completed");

    let _ = fs::remove_dir_all(workspace);
}

// ── C-141-9：正向对照（合法审批后写入解锁；后续无需审批） ──────────────────────────────────

#[test]
fn c141_9_approved_start_execution_unlocks_writes() {
    let workspace = temp_workspace();
    let executor = build_turn_scoped_executor(Some(workspace.clone()), None);
    executor.set_context(context(Some(SESSION_A), None));
    let plan = create_draft(&executor, SESSION_A);
    let plan_id = plan["planId"].as_str().unwrap_or_default().to_string();

    executor.set_context(context(
        Some(SESSION_A),
        Some(approval_json(SESSION_A, &plan_id, now_ms() + 3_600_000)),
    ));
    let started = executor.execute(&call(
        "plan_control",
        start_execution_args(SESSION_A, &plan),
    ));
    assert_eq!(started.status, "ok", "{}", started.output);
    assert_eq!(parse_output(&started.output)["lifecycle"], "executing");

    // 非 Draft 后普通写工具放行（不得过度拦截）。
    let write = executor.execute(&call(
        "Write",
        json!({ "path": "approved-executing.txt", "content": "ok", "description": "C-141-9 probe" }),
    ));
    assert_eq!(
        write.status, "ok",
        "after the approved migration ordinary writes must be allowed: {}",
        write.output
    );
    assert!(workspace.join("approved-executing.txt").exists());

    let _ = fs::remove_dir_all(workspace);
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// T2-1 迁移垫片（测试本地，非生产 API）
//
// F2 已删除 `GovernedToolExecutor::set_context` 与其共享 `Mutex<DispatchContext>` 槽位：turn 级
// 治理事实（session/turn/授权模式/审批证据）改为**每次调用**经
// `ToolExecutionContext.dispatch_context` 传入。本文件的历史夹具以「先 set 再 execute」的形态
// 注入 turn 事实，故此处提供最小注入壳：每次 `execute` 都把当前 turn 事实作为 per-call
// 上下文交给**真实生产 adapter**，调用点与断言零改动。
// 生产已删除 `set_context`：此垫片仅维持本文件既有 13 条用例的因果，**不进入生产路径**
// （生产 adapter 由 `GovernedToolExecutor::new` 直接构造，本文件里它只是垫片的 inner）。
// 生产路径自身的并发隔离由 `tests/concurrent_turn_context_isolation_red.rs` 直接断言。
// ─────────────────────────────────────────────────────────────────────────────────────────────

struct TurnScopedExecutor {
    inner: GovernedToolExecutor,
    context: Mutex<DispatchContext>,
}

impl TurnScopedExecutor {
    fn new(inner: GovernedToolExecutor) -> Self {
        Self {
            inner,
            context: Mutex::new(DispatchContext::default()),
        }
    }

    /// 设置本轮 turn 事实（垫片语义等价于旧的 `set_context`，但不再有跨调用共享槽位）。
    fn set_context(&self, context: DispatchContext) {
        *self.context.lock().expect("turn facts poisoned") = context;
    }

    /// 读回当前 turn 事实（仅供旧夹具的往返断言使用）。
    fn context(&self) -> DispatchContext {
        self.context.lock().expect("turn facts poisoned").clone()
    }

    fn turn_exec_context(&self, base: &ToolExecutionContext) -> ToolExecutionContext {
        let mut scoped = base.clone();
        let turn_facts = self.context();
        if scoped.session_id.is_none() {
            scoped.session_id = turn_facts.session_id.clone();
        }
        scoped.dispatch_context = Some(turn_facts);
        scoped
    }
}

impl ToolExecutor for TurnScopedExecutor {
    fn execute(&self, call: &ToolCall) -> ToolResult {
        self.execute_with_context(call, &ToolExecutionContext::default())
    }

    fn execute_with_context(&self, call: &ToolCall, exec_context: &ToolExecutionContext) -> ToolResult {
        self.inner
            .execute_with_context(call, &self.turn_exec_context(exec_context))
    }
}

fn build_turn_scoped_executor(
    workspace_root: Option<PathBuf>,
    authorize_store: Option<Arc<pony_agent_core::agent::path_permission::AuthorizeStore>>,
) -> TurnScopedExecutor {
    TurnScopedExecutor::new(build_governed_executor(workspace_root, authorize_store))
}

/// C-141-1 (c) 观测器：记录每次调用抵达策略判定步时携带的 turn 事实
///（会话 id + 宿主审批证据）。这是生产口径下"per-call `dispatch_context` 是否真的送到
/// 治理链判定步且互不污染"的唯一可观察证据。
#[derive(Default)]
struct TurnFactsRecorder {
    observed: Mutex<Vec<ObservedTurnFacts>>,
}

#[derive(Clone, Debug)]
struct ObservedTurnFacts {
    session_id: Option<String>,
    approval: Option<(String, String)>,
}

impl TurnFactsRecorder {
    fn snapshot(&self) -> Vec<ObservedTurnFacts> {
        self.observed.lock().expect("turn facts recorder poisoned").clone()
    }
}

impl ToolPolicyEvaluator for TurnFactsRecorder {
    fn evaluate(
        &self,
        _descriptor: &ToolDescriptor,
        _origin: &pony_agent_core::agent::tool_runtime::InvocationOrigin,
        _final_arguments: &Value,
    ) -> PermissionDecision {
        PermissionDecision {
            verdict: PermissionVerdict::Deny,
            decision_source: "c141_1_turn_facts_recorder".to_string(),
            reason: Some("C-141-1 observer: refuse execution, observe per-call turn facts".to_string()),
        }
    }

    fn evaluate_with_context(
        &self,
        descriptor: &ToolDescriptor,
        origin: &pony_agent_core::agent::tool_runtime::InvocationOrigin,
        final_arguments: &Value,
        context: &DispatchContext,
    ) -> PermissionDecision {
        self.observed
            .lock()
            .expect("turn facts recorder poisoned")
            .push(ObservedTurnFacts {
                session_id: context.session_id.clone(),
                approval: context
                    .plan_execution_approval
                    .as_ref()
                    .map(|approval| (approval.session_id.clone(), approval.plan_id.clone())),
            });
        self.evaluate(descriptor, origin, final_arguments)
    }
}
