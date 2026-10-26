//! 权限 fail-closed 契约（wave-19 冻结 → wave-30b 产品演进后按"不降鉴别力"重写）。
//!
//! **[事实声明] 本文件不测「模型 origin 写/执行默认需审批」——该默认自 wave-30b `ff76305`
//! 起是 `Allow`（`governed_default`）；今天的审批面由 工具规则(Allow/Ask/Deny) + Plan 草稿
//! 门禁 + 路径授权 承担，而不是全局 fail-closed 默认。** 读到这里请勿误以为"写/执行审批已被
//! 本文件覆盖"——本文件只覆盖下面三条真实存在的 fail-closed 保证。
//!
//! ## task-10 诊断：原两个用例为何 100% 稳定失败（非 flake）
//!
//! 原用例断言的是 wave-19 `309c254`（"fail-closed permission evaluator"）的语义：当时
//! `build_governed_executor` 在未设置 `PONY_LEGACY_POLICY_ALLOW_ALL` 时**不注册**策略评估器，
//! 于是命中 dispatcher 的默认 `DescriptorPolicyEvaluator`
//! （`dispatcher.rs:157-197`）——它把声明了 `WorkspaceWrite`/`WorkspaceExecute` scope 的内建
//! 工具一律抬升为 `ApprovalRequired`（phase-3 security review P1-2 的保守默认）。
//!
//! wave-30b `ff76305` 起产品改为**无条件注册** `GovernedPolicyEvaluator`
//! （`governed_executor.rs:431-437`），其兜底是 `Allow`（同文件 275-280，
//! `decision_source = "governed_default"`），同时 wave-19 的 `PONY_LEGACY_POLICY_ALLOW_ALL`
//! 开关被整体删除（今日只有 `wave22_execution_invariants_contract.rs` 还在 set，已是死配置）。
//! 也就是说「默认拒绝写/执行」这一契约已被产品**有意取代**：审批职责转移到
//! 工具级规则（Allow/Ask/Deny）+ Plan 草稿写门禁 + 路径授权，而非全局 fail-closed 默认。
//!
//! 后果（实测，非推理）：
//! - `builtin:workspace_write_file` 声明 `scopes={WorkspaceWrite}, requires_approval=false`
//!   → 空规则下 verdict=`Allow` → handler 直接执行；在全新临时目录里 `status="ok"`，
//!   原断言 `assert_eq!(status, "error")` 必红；
//! - 既然没有 `ApprovalRequired`，`ask_dispatcher.pending_requests()` 为空，
//!   原 `expect("must exist")` 必红。
//!
//! 结论：这是**测试断言过时（契约演进未跟进）**，不是 workspace/权限解析逻辑的缺陷。
//! Lead 已裁决 **B1：保持产品现状**（B2「恢复产品全局 fail-closed 默认」属 68 处调用面的
//! UX/安全姿态变更，另立议题，不在本文件夹带）。故本文件只断言今天真实存在且安全敏感的三条
//! fail-closed 保证。
//!
//! ## 断言不空转的证据（变异验证，实测后已还原）
//! - 把本文件注入的 `RuleAction::Ask` 改成 `Allow`（模拟"审批闸门被摘除"）→ 契约 2/3
//!   **立刻 FAILED**；
//! - 把 `build_governed_executor_with_plan_store(..., Some(plan_store))` 改成 `None`
//!   （模拟"草稿门禁消失"）→ 契约 1 **立刻 FAILED**。
//! 即三条断言都会随对应产品能力退化而变红，不是恒真断言。
use pony_agent_core::agent::dispatcher::GovernedDispatcher;
use pony_agent_core::agent::governed_executor::{
    build_governed_executor_with_plan_store, GovernedPolicyEvaluator,
};
use pony_agent_core::agent::plan_state::{PlanPayload, PlanStepSpec, PlanStore};
use pony_agent_core::agent::tool_runtime::{
    InvocationOrigin, PendingControlRequestKind, SystemClock, ToolDispatchRequest, ToolDispatcher,
};
use pony_agent_core::agent::tools::ToolRegistrySnapshot;
use pony_agent_core::agent::tools::{ToolCall, ToolExecutor};
use serde_json::json;
use std::fs;
use std::sync::Arc;

/// 构造一个注入了工具规则 `Ask` 的 dispatcher：`workspace_write_file` 必须走审批。
///
/// 为什么需要注入：`build_governed_executor` 的规则来自磁盘 `AppSettingsStore`，测试无法
/// 干净地写用户配置文件；而"审批流是否真的挂起 + CAS 消费是否正确"属于 dispatcher 契约，
/// 用显式注入的规则验证才能同时做到确定性（无磁盘/环境依赖）与跨平台一致。
fn dispatcher_with_ask_rule_for_write() -> Arc<GovernedDispatcher> {
    let registry =
        Arc::new(ToolRegistrySnapshot::builtin().expect("builtin registry must validate"));
    let dispatcher = Arc::new(GovernedDispatcher::new(registry, Arc::new(SystemClock)));
    dispatcher.register_policy_evaluator(Arc::new(GovernedPolicyEvaluator::new(
        vec![pony_agent_core::agent::app_settings::ToolRule {
            matcher: pony_agent_core::agent::app_settings::ToolRuleMatcher::Exact(
                "workspace_write_file".to_string(),
            ),
            action: pony_agent_core::agent::app_settings::RuleAction::Ask,
            reason: Some("workspace_write_file requires host approval".to_string()),
        }],
        None,
    )));
    dispatcher
}

/// 契约 1（取代原 `..._is_fail_closed_for_write_and_execute` 的"全局 fail-closed 默认"断言）：
/// Plan 仍处 `Draft` ⇒ 写工具被**拒绝**，且**没有任何文件落盘**。
///
/// 原断言错在哪：它要求"空规则下写工具默认 `error`"，该默认已由 wave-30b 取消（见文件头）；
/// 现在空规则下写工具是 Allow，故原断言永远红。
/// 新断言为什么仍能抓住回归：原断言只要 `status == "error"`（任何错误都算），本断言要求
/// `error.code == "permission_denied"`、文案含 `plan_draft_gate`，并额外证明拒绝发生在副作用
/// 之前（目标文件不存在）——比"只看状态字符串"鉴别力更强。
#[test]
fn test_draft_plan_gate_is_fail_closed_for_write_tools() {
    let dir = std::env::temp_dir().join(format!("pony-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).expect("create dir");

    let plan_store = PlanStore::new();
    plan_store
        .create(
            "session-draft-gate",
            PlanPayload {
                kind: "task".to_string(),
                summary: "draft plan blocks writes".to_string(),
                steps: vec![PlanStepSpec {
                    name: "step-1".to_string(),
                    summary: String::new(),
                }],
            },
        )
        .expect("create draft plan");

    let executor =
        build_governed_executor_with_plan_store(Some(dir.clone()), None, Some(plan_store));

    let call = ToolCall {
        call_id: Some("call-1".to_string()),
        name: "workspace_write_file".to_string(),
        arguments: json!({
            "path": "test.txt",
            "content": "hello",
            "description": "写入测试文件"
        }),
        plan: None,
    };

    let result = executor.execute(&call);
    assert_eq!(
        result.status, "error",
        "Draft 态写工具必须被拒绝: {}",
        result.output
    );
    let out: serde_json::Value = serde_json::from_str(&result.output).expect("valid json");
    assert_eq!(out["error"]["code"], "permission_denied", "{out}");
    assert!(
        out["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("plan_draft_gate"),
        "拒绝必须来自 Plan 草稿门禁（decision source 可追溯）: {out}"
    );
    assert!(
        !dir.join("test.txt").exists(),
        "fail-closed 必须在副作用之前生效：拒绝时不得落盘"
    );

    let _ = fs::remove_dir_all(dir);
}

/// 契约 2：`Ask` 规则 ⇒ 审批流真实挂起为 `Approval` 类型的 `PendingControlRequest`，
/// 并用 CAS version 批准成功。
///
/// 原断言错在哪：它依赖"默认评估器产生 ApprovalRequired"，该默认已随 wave-30b 取消。
/// 新断言为什么仍能抓住回归：原断言只要求"存在某个 call_id 匹配的 request"，本断言额外校验
/// 请求挂起（`control_outcome` 存在）、`request_kind == Approval`（不是 Interaction）、
/// 初始 CAS `version == 1`、批准后 `state == consumed` 且 `version == 2`、
/// 且挂起期间无副作用——任一项退化都会红。
#[test]
fn test_control_plane_approve_control_request_flow() {
    use pony_agent_core::agent::control_plane::HostControlPlaneBuilder;

    let dir = std::env::temp_dir().join(format!("pony-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).expect("create dir");

    let cp = HostControlPlaneBuilder::new()
        .ask_dispatcher(dispatcher_with_ask_rule_for_write())
        .build();

    let req = ToolDispatchRequest {
        origin: InvocationOrigin::Model,
        descriptor_id: "builtin:workspace_write_file".to_string(),
        call_id: "call-approve-1".to_string(),
        arguments: json!({
            "path": dir.join("test_appr.txt").display().to_string(),
            "content": "hello",
            "description": "写入待审批文件"
        }),
    };
    let outcome = cp.ask_dispatcher.dispatch(req);
    assert!(
        outcome.control_outcome.is_some(),
        "Ask 规则下 dispatch 必须挂起为控制请求，而不是直接执行: {outcome:?}"
    );

    let raw_requests = cp.ask_dispatcher.pending_requests();
    let approval_req = raw_requests
        .iter()
        .find(|r| r.call_id == "call-approve-1")
        .expect("must exist");
    assert_eq!(
        approval_req.request_kind,
        PendingControlRequestKind::Approval,
        "写工具审批必须是 Approval 类型（不是 Interaction）"
    );
    assert_eq!(
        approval_req.version, 1,
        "挂起请求的初始 CAS version 必须是 1"
    );

    let approve_res = cp.approve_control_request(&approval_req.request_id, approval_req.version);
    assert!(
        approve_res.is_ok(),
        "approve_control_request must succeed with CAS version: {:?}",
        approve_res
    );
    let consumed = approve_res.expect("approve ok");
    assert_eq!(
        consumed["request"]["state"],
        json!("consumed"),
        "批准后请求必须置 consumed: {consumed}"
    );
    assert_eq!(
        consumed["request"]["version"],
        json!(2),
        "CAS 消费必须推进 version: {consumed}"
    );
    // 审批只是消费控制请求；工具执行由运行时携带审批证据重新 dispatch。
    // 若 Ask 规则被忽略（回归成 Allow），handler 会直接跑并把文件写进 dir。
    assert!(
        !dir.join("test_appr.txt").exists(),
        "挂起/批准都不得代执行写工具（审批期间无副作用）"
    );

    let _ = fs::remove_dir_all(dir);
}

/// 契约 3：审批消费是 CAS —— 用已消费的 `version` 重放批准必须被拒绝。
///
/// 这是原用例缺失的维度：原断言只覆盖"正确 version 能批准"，无法抓住
/// "CAS 退化为无条件消费"这类回归（例如 version/state 校验被删）。
#[test]
fn test_approve_control_request_rejects_stale_version() {
    use pony_agent_core::agent::control_plane::HostControlPlaneBuilder;

    let dir = std::env::temp_dir().join(format!("pony-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).expect("create dir");

    let cp = HostControlPlaneBuilder::new()
        .ask_dispatcher(dispatcher_with_ask_rule_for_write())
        .build();

    let req = ToolDispatchRequest {
        origin: InvocationOrigin::Model,
        descriptor_id: "builtin:workspace_write_file".to_string(),
        call_id: "call-approve-2".to_string(),
        arguments: json!({
            "path": dir.join("test_stale.txt").display().to_string(),
            "content": "hello",
            "description": "写入待审批文件"
        }),
    };
    let _ = cp.ask_dispatcher.dispatch(req);
    let pending = cp
        .ask_dispatcher
        .pending_requests()
        .into_iter()
        .find(|r| r.call_id == "call-approve-2")
        .expect("must exist");

    // 第一次批准成功（version=1 → 2）。
    cp.approve_control_request(&pending.request_id, pending.version)
        .expect("first approve must succeed");
    // 重放同一个 version 必须失败（请求已 consumed、version 已推进）。
    let replay = cp.approve_control_request(&pending.request_id, pending.version);
    let err = replay.expect_err("重复消费同一 version 必须被 CAS 拒绝");
    assert!(
        err.contains("stale mutation rejected") || err.contains("cannot be consumed"),
        "拒绝必须来自 CAS 闸门（version/state 校验），而不是查找失败等旁路: {err}"
    );
    assert!(
        !dir.join("test_stale.txt").exists(),
        "批准/拒绝都不得代执行写工具（无副作用）"
    );

    let _ = fs::remove_dir_all(dir);
}
