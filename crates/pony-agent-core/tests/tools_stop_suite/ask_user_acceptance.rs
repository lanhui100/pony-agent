//! PA-114: ask_user 工具（后端一等公民）验收测试 — 红相（red phase）
//!
//! 契约依据：`.dev-team/contract-matrix-ask-user.md`（Lead 冻结）B1-1 ~ B3-4。
//!
//! 红相说明：`ask_user` 尚未实现（当前产品面 `Ask` 仍由 legacy `echo_input` 占据），
//! 因此本文件大部分断言在红相时期必须失败；但每个引用的符号都必须是*已存在*的 pub API，
//! 文件必须能编译（引用符号真实存在），失败仅来自断言/运行时行为。
//!
//! 注意：`canonical_tool_name` 是 pub(crate)，集成测试不可直接使用；映射类断言统一通过
//! `ToolRegistrySnapshot::builtin()` 的 `resolve()`/`descriptors`/`provider_contract_views()`
//! 观察产品面。

use pony_agent_core::agent::ask_control::{answer_ask, list_pending_asks};
use pony_agent_core::agent::dispatcher::{
    ControlRequestAuthorization, DispatchContext, GovernedDispatcher, PermissionVerdict,
    ToolPolicyEvaluator,
};
use pony_agent_core::agent::governed_executor::LegacyCompatiblePolicyEvaluator;
use pony_agent_core::agent::tool_runtime::{
    FakeClock, InvocationOrigin, PendingControlRequestKind, PendingControlRequestState,
    RuntimeClock, ToolDispatchRequest,
};
use pony_agent_core::agent::tools::{
    builtin_tools, model_visible_tool_name, tool_display_metadata_for_name, ToolControlKind,
    ToolDescriptor, ToolExposure, ToolKind, ToolRegistrySnapshot,
};
use serde_json::{json, Value};
use std::sync::Arc;

/// 契约约定的 ask_user 一等公民 descriptor id。
const ASK_DESCRIPTOR_ID: &str = "builtin:ask_user";

// ─────────────────────────────────────────────────────────────────────────────
// 测试基建（均为已存在 pub API 之上的最小封装）
// ─────────────────────────────────────────────────────────────────────────────

fn builtin_registry() -> ToolRegistrySnapshot {
    ToolRegistrySnapshot::builtin().expect("builtin tool registry must construct cleanly")
}

/// 从 builtin 注册表取 `builtin:ask_user` descriptor。红相时期该 descriptor 不存在，
/// expect 直接 panic（红相证据）。
fn ask_user_descriptor(registry: &ToolRegistrySnapshot) -> &ToolDescriptor {
    registry
        .descriptors
        .iter()
        .find(|descriptor| descriptor.identity.descriptor_id == ASK_DESCRIPTOR_ID)
        .expect("builtin registry must contain descriptor `builtin:ask_user`")
}

/// 带 LegacyCompatiblePolicyEvaluator 的真实 dispatcher（B3 受管路径共用）。
fn legacy_dispatcher() -> (Arc<ToolRegistrySnapshot>, Arc<dyn RuntimeClock>, GovernedDispatcher) {
    let registry = Arc::new(builtin_registry());
    let clock: Arc<dyn RuntimeClock> = Arc::new(FakeClock::new(1_000));
    let dispatcher = GovernedDispatcher::new(Arc::clone(&registry), Arc::clone(&clock));
    dispatcher.register_policy_evaluator(Arc::new(LegacyCompatiblePolicyEvaluator));
    (registry, clock, dispatcher)
}

fn ask_dispatch_context() -> DispatchContext {
    DispatchContext {
        session_id: Some("session-1".to_string()),
        run_id: Some("run-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        host_control_available: true,
        ..Default::default()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// B1 builtin 注册
// ─────────────────────────────────────────────────────────────────────────────

/// B1-1：`builtin_tools()` 包含 `name == "ask_user"` 定义；schema 为 object，
/// properties 含 question(string, required)、options(array<string>, optional)，
/// additionalProperties=false；`defaultAnswer` / `timeoutMs` 为死参数（dispatcher 与
/// 前端均不消费），必须不存在以免误导模型。
#[test]
fn test_b1_1_ask_user_builtin_definition_and_schema() {
    let definitions = builtin_tools();
    let ask = definitions
        .iter()
        .find(|definition| definition.name == "ask_user")
        .expect("builtin_tools() must include an `ask_user` definition");

    let schema = &ask.input_schema;
    assert_eq!(schema["type"], json!("object"), "input schema must be an object");
    let properties = &schema["properties"];
    assert_eq!(properties["question"]["type"], json!("string"));
    // `description` 由 builtin schema 的 with_description() 注入，因此 required 含
    // question + description；断言只锁定 question 必须、其余可选参数不得必填。
    let required = schema["required"]
        .as_array()
        .expect("required must be an array");
    let required_strs = required
        .iter()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    assert!(
        required_strs.contains(&"question"),
        "question must be a required property"
    );
    assert!(!required_strs.contains(&"options"));
    assert!(!required_strs.contains(&"defaultAnswer"));
    assert!(!required_strs.contains(&"timeoutMs"));
    assert_eq!(properties["options"]["type"], json!("array"));
    assert_eq!(properties["options"]["items"]["type"], json!("string"));
    // PA-114 修复第二轮（Lead 裁决 B）：defaultAnswer/timeoutMs 是死参数，schema 不得再暴露。
    assert!(
        properties.get("defaultAnswer").is_none(),
        "dead param `defaultAnswer` must not appear in the ask_user schema"
    );
    assert!(
        properties.get("timeoutMs").is_none(),
        "dead param `timeoutMs` must not appear in the ask_user schema"
    );
    assert_eq!(
        schema["additionalProperties"],
        json!(false),
        "unknown argument keys must be rejected"
    );
}

/// B1-2：`ToolRegistrySnapshot::builtin()` 构建成功；descriptor
/// `builtin:ask_user`：primitive_name == "ask_user"、model_name == "Ask"、
/// kind == interactive、exposure == modelVisible、host_mediated == true、
/// requires_approval == false。
#[test]
fn test_b1_2_ask_user_builtin_descriptor_shape() {
    let registry = builtin_registry();
    let ask = ask_user_descriptor(&registry);

    assert_eq!(ask.identity.descriptor_id, "builtin:ask_user");
    assert_eq!(ask.identity.primitive_name, "ask_user");
    assert_eq!(ask.identity.model_name, "Ask");
    assert_eq!(ask.kind, ToolKind::Interactive);
    assert_eq!(ask.exposure, ToolExposure::ModelVisible);
    assert!(
        ask.permission_declaration.host_mediated,
        "ask_user must be host-mediated (WaitingHost), not approval"
    );
    assert!(
        !ask.permission_declaration.requires_approval,
        "ask_user must not require a separate approval step"
    );
}

/// B1-3：模型面 `provider_contract_views()`（builtin 表面）含 name "Ask"
/// （execution_primitive == "ask_user"），其 schema 含 `options`；
/// `echo_input` 不再 model-visible（走入 Internal）。
#[test]
fn test_b1_3_model_surface_wins_to_ask_user() {
    let registry = builtin_registry();

    let provider_views = registry.provider_contract_views();
    let ask_view = provider_views
        .iter()
        .find(|view| view.name == "Ask")
        .expect("model surface must include product tool `Ask`");
    assert_eq!(
        ask_view.execution_primitive, "ask_user",
        "product name `Ask` must be won by the ask_user primitive"
    );
    assert!(
        ask_view
            .input_schema
            .get("properties")
            .and_then(|properties| properties.get("options"))
            .is_some(),
        "model-visible `Ask` schema must expose `options`"
    );

    let echo = registry
        .descriptors
        .iter()
        .find(|descriptor| descriptor.identity.primitive_name == "echo_input")
        .expect("legacy echo_input descriptor must be retained");
    assert_eq!(
        echo.exposure,
        ToolExposure::Internal,
        "echo_input must no longer be model-visible once ask_user owns `Ask`"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// B2 映射注册
// ─────────────────────────────────────────────────────────────────────────────

/// B2-1：产品名 `"Ask"` 解析到 `ask_user`；`"ask_user"` 亦命中；
/// `"echo_input"` 仍解析 echo_input 自身（兼容不回归）。
#[test]
fn test_b2_1_name_resolution_maps_ask_to_ask_user() {
    let registry = builtin_registry();

    let by_product = registry
        .resolve("Ask")
        .expect("product name `Ask` must resolve");
    assert_eq!(
        by_product.identity.descriptor_id, "builtin:ask_user",
        "`Ask` must resolve to the ask_user descriptor"
    );

    let by_primitive = registry
        .resolve("ask_user")
        .expect("primitive name `ask_user` must resolve");
    assert_eq!(by_primitive.identity.descriptor_id, "builtin:ask_user");

    let by_echo = registry
        .resolve("echo_input")
        .expect("legacy `echo_input` must keep resolving");
    assert_eq!(
        by_echo.identity.descriptor_id, "builtin:echo_input",
        "legacy echo_input self-resolution must not regress"
    );
}

/// B2-2：`model_visible_tool_name("ask_user") == "Ask"`；display metadata 中文名 "提问"。
#[test]
fn test_b2_2_model_visible_name_and_display_metadata() {
    assert_eq!(model_visible_tool_name("ask_user"), "Ask");

    let display = tool_display_metadata_for_name("ask_user");
    assert_eq!(
        display.display_name_zh.as_deref(),
        Some("提问"),
        "ask_user display metadata must carry the Chinese name `提问`"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// B3 受管执行路径（governed dispatcher 端到端）
// ─────────────────────────────────────────────────────────────────────────────

/// B3-1：`LegacyCompatiblePolicyEvaluator::evaluate` 对 `builtin:ask_user` descriptor
/// 返回 `PermissionVerdict::WaitingHost`。
#[test]
fn test_b3_1_legacy_policy_evaluator_waiting_host() {
    let registry = builtin_registry();
    let ask = ask_user_descriptor(&registry);

    let evaluator = LegacyCompatiblePolicyEvaluator;
    let decision = evaluator.evaluate(ask, &InvocationOrigin::Model, &json!({}));
    assert_eq!(
        decision.verdict,
        PermissionVerdict::WaitingHost,
        "LegacyCompatiblePolicyEvaluator must keep the old Ask verdict for ask_user"
    );
}

/// B3-2：`dispatch_governed`（descriptor `builtin:ask_user`，
/// arguments=`{"question":"继续？","options":["是","否"],"description":"确认"}`，
/// host_control_available: true）→ `ToolControlKind::WaitingHost`；持久化 Interaction
/// PendingControlRequest：prompt == "继续？"、options == ["是","否"]、
/// descriptor_id == "builtin:ask_user"、state == pending。
#[test]
fn test_b3_2_dispatch_persists_interaction_ask() {
    let (_registry, _clock, dispatcher) = legacy_dispatcher();

    let outcome = dispatcher.dispatch_governed(
        ToolDispatchRequest {
            origin: InvocationOrigin::Model,
            descriptor_id: ASK_DESCRIPTOR_ID.to_string(),
            call_id: "call-1".to_string(),
            arguments: json!({
                "question": "继续？",
                "options": ["是", "否"],
                "description": "确认"
            }),
        },
        &ask_dispatch_context(),
    );
    let control = outcome
        .control_outcome
        .expect("asking must surface a host-mediation control outcome");
    assert_eq!(control.kind, ToolControlKind::WaitingHost);

    let asks = list_pending_asks(&dispatcher);
    assert_eq!(asks.len(), 1, "exactly one pending Interaction request");
    let pending = &asks[0];
    assert_eq!(pending.request_kind, PendingControlRequestKind::Interaction);
    assert_eq!(pending.prompt.as_deref(), Some("继续？"));
    assert_eq!(pending.options, Some(json!(["是", "否"])));
    assert_eq!(pending.descriptor_id, "builtin:ask_user");
    assert_eq!(pending.state, PendingControlRequestState::Pending);
}

/// B3-3：`list_pending_asks(&dispatcher)` 可见该请求；用
/// `ControlRequestAuthorization::for_request(&pending, Some(json!("是")))` 调 `answer_ask`
/// 成功；consumed request state == `Consumed`，answer == json!("是")。
#[test]
fn test_b3_3_answer_ask_consumes_with_cas() {
    let (_registry, _clock, dispatcher) = legacy_dispatcher();

    let outcome = dispatcher.dispatch_governed(
        ToolDispatchRequest {
            origin: InvocationOrigin::Model,
            descriptor_id: ASK_DESCRIPTOR_ID.to_string(),
            call_id: "call-1".to_string(),
            arguments: json!({
                "question": "继续？",
                "options": ["是", "否"],
                "description": "确认"
            }),
        },
        &ask_dispatch_context(),
    );
    outcome
        .control_outcome
        .expect("waiting host control outcome required");

    let asks = list_pending_asks(&dispatcher);
    assert_eq!(asks.len(), 1, "pending ask must be visible via list_pending_asks");
    let pending = &asks[0];

    let authorization =
        ControlRequestAuthorization::for_request(pending, Some(json!("是")));
    let consumed = answer_ask(&dispatcher, &pending.request_id, &authorization)
        .expect("answer_ask must consume the pending interaction request");
    assert_eq!(consumed.request.state, PendingControlRequestState::Consumed);
    assert_eq!(consumed.answer, Some(json!("是")));
}

/// B3-4：兼容不回归：以 `Ask`（model_name）/`echo_input`（legacy）dispatch 仍得到
/// `WaitingHost`（LegacyCompatiblePolicyEvaluator 旧判定保留）。
///
/// 红相与绿相两阶段下传入参数的 schema 不同（red: echo_input 仅收 `text`；
/// green: ask_user 收 `question`），因此 `Ask` 名字路径在 evaluator 层做判定断言
/// （两端 model_name == "Ask" 均命中旧判定），`echo_input` legacy primitive 走真实
/// System-origin dispatch（schema 不变，两阶段均可验证旧判定保留）。
#[test]
fn test_b3_4_compat_ask_and_echo_input_still_wait_for_host() {
    let (registry, _clock, dispatcher) = legacy_dispatcher();

    // 产品名 `Ask`（当前解析到 `Ask` winner，两阶段下 evaluator 必须仍判 WaitingHost）。
    let by_product = registry
        .resolve("Ask")
        .expect("product name `Ask` must keep resolving");
    let evaluator = LegacyCompatiblePolicyEvaluator;
    assert_eq!(
        evaluator
            .evaluate(by_product, &InvocationOrigin::Model, &json!({}))
            .verdict,
        PermissionVerdict::WaitingHost,
        "legacy verdict for product name `Ask` must be preserved"
    );

    // legacy primitive `echo_input`（System origin，可触达 internal descriptor）。
    let by_echo = dispatcher.dispatch_governed(
        ToolDispatchRequest {
            origin: InvocationOrigin::System,
            descriptor_id: "echo_input".to_string(),
            call_id: "call-echo".to_string(),
            arguments: json!({ "text": "x", "description": "兼容回显验证" }),
        },
        &ask_dispatch_context(),
    );
    assert_eq!(
        by_echo.control_outcome.expect("echo_input must wait for host").kind,
        ToolControlKind::WaitingHost,
        "legacy primitive `echo_input` must keep the WaitingHost verdict"
    );
}