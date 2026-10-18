//! W36 / PA-138 红相契约：Plan 草稿态写门禁必须使用**运行时注入的会话 id**。
//!
//! 缺陷（wave-35 残留，形状由 ff76305 引入）：`GovernedPolicyEvaluator::evaluate` 没有
//! `DispatchContext`，`governed_executor.rs` 的门禁把 session 解析为
//! 「工具参数 `session_id` → 环境变量 → 常量 `"default"`」，并额外硬编码测试会话名
//! `"test-session-draft-gate"`。真实会话 id 由 runtime 经
//! `governed.set_context(DispatchContext { session_id: input.session_id, .. })` 注入
//! （runtime/mod.rs:299-300），而写工具的 schema 不接受 `session_id`
//! （`additionalProperties:false`）⇒ 生产调用永远匹配不到当前会话的 Draft Plan，
//! 写保护形同虚设（fail-open）。
//!
//! 本测试走**生产装配路径**（`build_governed_executor` + `set_context` + `execute`），
//! 必须在门禁真正读取注入会话后转绿；当前实现下写工具会返回 `ok`（红相）。

use pony_agent_core::agent::dispatcher::DispatchContext;
use pony_agent_core::agent::governed_executor::build_governed_executor;
use pony_agent_core::agent::tools::{ToolCall, ToolExecutor};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

const SESSION: &str = "wave36-prod-session-42";

fn temp_workspace() -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "pony-wave36-pa138-{}-{seq}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create pa138 test workspace");
    root
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

fn create_draft_plan(executor: &impl ToolExecutor) -> Value {
    let created = executor.execute(&call(
        "plan_control",
        json!({
            "op": "create",
            "session_id": SESSION,
            "description": "PA-138 draft-gate fixture plan",
            "payload": {
                "kind": "task",
                "summary": "wave36 PA-138 draft gate",
                "steps": [{ "name": "step-1", "summary": "draft a change" }]
            }
        }),
    ));
    assert_eq!(
        created.status, "ok",
        "Plan create must succeed under the injected session: {}",
        created.output
    );
    let plan = parse_output(&created.output);
    assert_eq!(
        plan["lifecycle"], "draft",
        "fixture plan must be Draft: {plan}"
    );
    plan
}

/// PA-138 (a)：非 `default` 的**真实注入会话**在 Draft 态调用写工具必须被
/// `plan_draft_gate` 拒绝。写工具参数里没有 `session_id`（生产 schema 也不允许），
/// 门禁只能从注入上下文取会话。
#[test]
fn draft_gate_blocks_write_for_runtime_injected_session() {
    let workspace = temp_workspace();
    let executor = build_governed_executor(Some(workspace.clone()), None);
    executor.set_context(DispatchContext {
        session_id: Some(SESSION.to_string()),
        ..Default::default()
    });

    let _plan = create_draft_plan(&executor);

    // Act：同一注入会话下调用写类工具（无 `session_id` 参数）。
    let write = executor.execute(&call(
        "Write",
        json!({
            "path": "draft-gate-write.txt",
            "content": "must not be written while Draft",
            "description": "PA-138 draft gate red-phase probe"
        }),
    ));

    // Assert：必须被草稿态写门禁拒绝，而不是落到 `default`/测试会话后放行。
    assert_eq!(
        write.status, "error",
        "Draft-state write must be denied for the injected session; got: {}",
        write.output
    );
    let denied = parse_output(&write.output);
    assert_eq!(
        denied["error"]["code"], "permission_denied",
        "expected permission_denied: {denied}"
    );
    let message = denied["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        message.contains("plan_draft_gate"),
        "deny must be attributed to plan_draft_gate (injected-session gate): {message}"
    );
    assert!(
        message.contains("Draft"),
        "deny reason must state the Draft lifecycle: {message}"
    );
    assert!(
        !workspace.join("draft-gate-write.txt").exists(),
        "denied write must not create the file"
    );

    let _ = fs::remove_dir_all(workspace);
}

/// PA-138 (b)：`plan_control` 自身必须继续豁免草稿态写门禁——它是解除草稿态写保护的
/// 唯一机制（`start_execution` = unlock write operations）。否则 merge / complete_step /
/// replace / start_execution 全被 Deny，plan 永远无法离开 Draft（自锁死回归）。
///
/// 本断言只锁「不被 plan_draft_gate 拒绝」；`start_execution` 的「审批后可达」语义
/// 由 PA-141 契约单独锁定（避免与本卡冲突）。
#[test]
fn draft_gate_keeps_plan_control_exempt() {
    let workspace = temp_workspace();
    let executor = build_governed_executor(Some(workspace.clone()), None);
    executor.set_context(DispatchContext {
        session_id: Some(SESSION.to_string()),
        ..Default::default()
    });

    let plan = create_draft_plan(&executor);
    let plan_id = plan["planId"].as_str().expect("planId").to_string();
    let revision = plan["revision"].as_u64().expect("revision");

    let merged = executor.execute(&call(
        "plan_control",
        json!({
            "op": "merge",
            "session_id": SESSION,
            "description": "PA-138 plan_control exemption probe",
            "plan_id": plan_id,
            "revision": revision,
            "step": { "name": "step-2", "summary": "merge while draft" }
        }),
    ));

    assert!(
        !merged.output.contains("plan_draft_gate"),
        "plan_control must stay exempt from the draft write gate (self-lock regression): {}",
        merged.output
    );
    assert_eq!(
        merged.status, "ok",
        "plan_control merge must remain reachable in Draft: {}",
        merged.output
    );
    let merged_plan = parse_output(&merged.output);
    assert_eq!(merged_plan["lifecycle"], "draft");
    assert_eq!(
        merged_plan["steps"].as_array().map(Vec::len),
        Some(2),
        "merge must append the step: {merged_plan}"
    );

    let _ = fs::remove_dir_all(workspace);
}

/// PA-138 / C-138-3 反向对照：门禁必须**按注入会话隔离**，不得过度拦截。
/// (i) Draft 在 `sess-A`、context 为 `sess-B` ⇒ 写工具 Allow（跨会话不误伤）；
/// (ii) `sess-A` 无 Draft 时写工具 Allow（无 Draft 不拦截）。
#[test]
fn draft_gate_does_not_cross_block_other_sessions() {
    let workspace = temp_workspace();
    let executor = build_governed_executor(Some(workspace.clone()), None);
    executor.set_context(DispatchContext {
        session_id: Some(SESSION.to_string()),
        ..Default::default()
    });
    let _plan = create_draft_plan(&executor);

    // (i) 切到另一个会话：sess-B 没有 Draft，写工具必须放行。
    executor.set_context(DispatchContext {
        session_id: Some("wave36-other-session-7".to_string()),
        ..Default::default()
    });
    let other_session_write = executor.execute(&call(
        "Write",
        json!({
            "path": "other-session.txt",
            "content": "allowed",
            "description": "C-138-3 cross-session control"
        }),
    ));
    assert_eq!(
        other_session_write.status, "ok",
        "a Draft owned by another session must not block this session: {}",
        other_session_write.output
    );

    // (ii) 新 executor（无任何 Draft）在同一注入会话下写工具必须放行。
    let workspace2 = temp_workspace();
    let clean_executor = build_governed_executor(Some(workspace2.clone()), None);
    clean_executor.set_context(DispatchContext {
        session_id: Some(SESSION.to_string()),
        ..Default::default()
    });
    let no_draft_write = clean_executor.execute(&call(
        "Write",
        json!({
            "path": "no-draft.txt",
            "content": "allowed",
            "description": "C-138-3 no-draft control"
        }),
    ));
    assert_eq!(
        no_draft_write.status, "ok",
        "without any Draft the write tool must stay allowed: {}",
        no_draft_write.output
    );

    let _ = fs::remove_dir_all(workspace);
    let _ = fs::remove_dir_all(workspace2);
}

/// PA-138 / C-138-4 fail-closed 扩展（Lead 2026-10-18 采纳）：
/// `context.session_id=None` 且 `PlanStore::has_any_draft_plan()` 为真 ⇒ Write/Execute Deny
/// `plan_draft_gate`；无任何 Draft ⇒ Allow。两分支都必须有断言。
#[test]
fn draft_gate_fails_closed_when_context_session_is_missing() {
    // 分支 1：存在任一 Draft（其属主会话不是当前 context）⇒ 无会话上下文也必须拒绝。
    let workspace = temp_workspace();
    let executor = build_governed_executor(Some(workspace.clone()), None);
    executor.set_context(DispatchContext {
        session_id: Some(SESSION.to_string()),
        ..Default::default()
    });
    let _plan = create_draft_plan(&executor);

    executor.set_context(DispatchContext::default()); // session_id = None
    let denied = executor.execute(&call(
        "Write",
        json!({
            "path": "no-context-draft.txt",
            "content": "must be denied",
            "description": "C-138-4 fail-closed probe"
        }),
    ));
    assert_eq!(
        denied.status, "error",
        "session-less context with any Draft must fail closed: {}",
        denied.output
    );
    let payload = parse_output(&denied.output);
    assert_eq!(payload["error"]["code"], "permission_denied", "{payload}");
    assert!(
        payload["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("plan_draft_gate"),
        "fail-closed deny must be attributed to plan_draft_gate: {payload}"
    );

    // 分支 2：无任何 Draft ⇒ 同一 session-less context 下写工具 Allow。
    let clean_workspace = temp_workspace();
    let clean_executor = build_governed_executor(Some(clean_workspace.clone()), None);
    clean_executor.set_context(DispatchContext::default());
    let allowed = clean_executor.execute(&call(
        "Write",
        json!({
            "path": "no-context-no-draft.txt",
            "content": "allowed",
            "description": "C-138-4 no-draft control"
        }),
    ));
    assert_eq!(
        allowed.status, "ok",
        "with no Draft at all the session-less write must stay allowed: {}",
        allowed.output
    );

    let _ = fs::remove_dir_all(workspace);
    let _ = fs::remove_dir_all(clean_workspace);
}
