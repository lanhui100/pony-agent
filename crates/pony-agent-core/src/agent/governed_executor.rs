//! Production seam for adopting the governed dispatcher as the runtime's tool-execution engine
//! (PA-076 runtime-switch milestone).
//!
//! `build_governed_executor` constructs a `GovernedToolExecutor` over the full builtin tool
//! surface: every builtin primitive is bridged to its legacy `ToolRouter` implementation, and the
//! governed `workspace_batch` / `workspace_gather_context` composites are registered with the same
//! read-only child authority as the legacy migration gate. The runtime can switch its tool engine
//! from a bare `ToolRouter` to this executor without changing its own surface.
//!
//! Migration notes (deliberate, documented):
//! - A `LegacyCompatiblePolicyEvaluator` reproduces the legacy runtime's direct-execution
//!   behavior (every builtin allowed; the legacy `ToolRouter` applied its own tool-level risk
//!   checks). The dispatcher's conservative default is intentionally overridden here so the switch
//!   does not change today's tool-visible behavior. Real approval semantics land with the
//!   permission-contract work (phase 4 / task 3.6 closeout).
//! - Execute-scope tools (`workspace_run_command`) run behind the production sandbox
//!   assembly below: `NativeSandboxBackend` when available, else
//!   `HostApprovedUnsandboxedBackend` (DSH danger-full-access trust passthrough).
//!   Fail-closed (`sandbox_unavailable`/`sandbox_denied`) is still enforced by both the
//!   dispatcher preflight and the legacy handler when the backend rejects.
//! - Legacy tool error codes that are not in the dispatcher's known-code set surface as
//!   `handler_error`; status and message fidelity are preserved.

use crate::agent::budget::DispatchBudgetConfig;
use crate::agent::dispatcher::{
    DispatchContext, GovernedDispatcher, PermissionDecision, PermissionVerdict,
    ToolPolicyEvaluator,
};
use crate::agent::dispatcher_composites::{
    register_governed_composites, GovernedToolExecutor,
};
use crate::agent::document_conversion::ReadDocumentHandler;
use crate::agent::image_artifact::ViewImageHandler;
use crate::agent::plan_state::{PlanLifecycleState, PlanControlHandler, PlanStore};
use crate::agent::sandbox::{HostApprovedUnsandboxedBackend, NativeSandboxBackend};
use crate::agent::tool_runtime::{
    InvocationOrigin, PrimitiveToolHandler, PrimitiveToolHandlerRequest, RuntimeClock,
    SandboxAvailability, SandboxBackend, SystemClock,
};
use crate::agent::tools::{
    ToolCall, ToolRegistrySnapshot, ToolRouter, TOOL_ASK_USER, TOOL_PLAN_CONTROL,
    TOOL_WORKSPACE_READ_DOCUMENT, TOOL_VIEW_IMAGE,
};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;

/// Read-only primitive surface shared by the migration gate and the governed composite authority.
/// Mirrors the legacy task-3.4 allowlist in `tools.rs` (kept in sync by contract).
const READ_ONLY_PRIMITIVES: &[&str] = &[
    "workspace_list_files",
    "workspace_read_file",
    "workspace_read_file_segment",
    "workspace_read_document",
    "workspace_path_info",
    "workspace_search_text",
    "workspace_glob_files",
];

const COMPOSITE_PRIMITIVES: &[&str] = &["workspace_batch", "workspace_gather_context"];

/// 保持旧单元/集成测试零修改向后兼容。
#[derive(Clone, Debug, Default)]
pub struct LegacyCompatiblePolicyEvaluator;

impl ToolPolicyEvaluator for LegacyCompatiblePolicyEvaluator {
    fn evaluate(
        &self,
        descriptor: &crate::agent::tools::ToolDescriptor,
        origin: &InvocationOrigin,
        final_arguments: &Value,
    ) -> PermissionDecision {
        GovernedPolicyEvaluator::default().evaluate(descriptor, origin, final_arguments)
    }
}

/// W30b：治理型策略评估器，支持工具级规则（Allow / Ask / Deny）及 Plan 草稿态写保护门禁。
#[derive(Clone, Debug, Default)]
pub struct GovernedPolicyEvaluator {
    pub rules: Vec<crate::agent::app_settings::ToolRule>,
    pub plan_store: Option<Arc<crate::agent::plan_state::PlanStore>>,
}

impl GovernedPolicyEvaluator {
    pub fn new(
        rules: Vec<crate::agent::app_settings::ToolRule>,
        plan_store: Option<Arc<crate::agent::plan_state::PlanStore>>,
    ) -> Self {
        Self { rules, plan_store }
    }

    /// PA-141（方案 b）扩展裁定核心：在 `session_id` 之外额外接收宿主注入的
    /// `plan_execution_approval`（仅来自 `DispatchContext`，模型工具参数不是审批来源）。
    /// `session_id` 是**运行时注入**的权威会话（`DispatchContext.session_id`）：工具参数
    /// 不是会话键（生产 schema 不允许，且可被模型控制）；`None` 表示本次 dispatch 未绑定
    /// 会话上下文（按 C-138-4 fail-closed 语义处理）。
    fn decide_with_context(
        &self,
        descriptor: &crate::agent::tools::ToolDescriptor,
        session_id: Option<&str>,
        plan_execution_approval: Option<&crate::agent::dispatcher::PlanExecutionApproval>,
        json_arguments: &Value,
    ) -> PermissionDecision {
        let is_ask = descriptor.identity.model_name == "Ask"
            || descriptor.identity.primitive_name == "echo_input"
            || descriptor.identity.primitive_name == TOOL_ASK_USER;
        if is_ask {
            return PermissionDecision {
                verdict: PermissionVerdict::WaitingHost,
                decision_source: "ask_descriptor".to_string(),
                reason: None,
            };
        }

        // 1. 工具级规则匹配（优先级高于默认判定与 Plan 门禁）
        for rule in &self.rules {
            if rule.matcher.matches(&descriptor.identity.model_name)
                || rule.matcher.matches(&descriptor.identity.primitive_name)
                || rule.matcher.matches(&descriptor.identity.descriptor_id)
            {
                return match rule.action {
                    crate::agent::app_settings::RuleAction::Allow => PermissionDecision {
                        verdict: PermissionVerdict::Allow,
                        decision_source: "tool_rule_allow".to_string(),
                        reason: rule.reason.clone(),
                    },
                    crate::agent::app_settings::RuleAction::Ask => PermissionDecision {
                        verdict: PermissionVerdict::ApprovalRequired,
                        decision_source: "tool_rule_ask".to_string(),
                        reason: rule.reason.clone(),
                    },
                    crate::agent::app_settings::RuleAction::Deny => PermissionDecision {
                        verdict: PermissionVerdict::Deny,
                        decision_source: "tool_rule_deny".to_string(),
                        reason: rule.reason.clone().or_else(|| {
                            Some(format!("Tool `{}` matched deny rule", descriptor.identity.model_name))
                        }),
                    },
                };
            }
        }

        // 2. Plan 相位门禁：如果存在 Draft 态 Plan，则写类/执行类工具被拒绝
        // 例外:`plan_control` 自身豁免本门禁——它是解除草稿态写保护的唯一机制
        // (`plan_state.rs` 中 `start_execution` = "unlock write operations")。否则
        // merge / complete_step / replace / start_execution 全被 Deny,plan 永远无法
        // 离开 Draft(自锁死)。豁免按 canonical primitive 名判定,不影响其它写类描述符。
        if let Some(plan_store) = &self.plan_store {
            let is_write_or_exec = matches!(
                descriptor.kind,
                crate::agent::tools::ToolKind::Write | crate::agent::tools::ToolKind::Execute
            );
            if is_write_or_exec && descriptor.identity.primitive_name == TOOL_PLAN_CONTROL {
                // ── W36 / PA-141（方案 b，C-141-2..8）：宿主审批闸门 ─────────────────────
                // 触发面：`start_execution`，或目标 plan 仍处 Draft 的 `complete_step`
                //（complete_step 是第二条 Draft→Executing 迁移路径）。仅该迁移面需要审批；
                // 其余 plan_control op（create/merge/replace/abort…）与 Executing 态的
                // complete_step 不受影响（C-141-8）。审批证据只来自宿主注入的
                // DispatchContext，模型工具参数不是来源（C-141-3）。
                let requires_execution_approval = || -> Option<bool> {
                    let op = json_arguments
                        .get("op")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    let session =
                        json_arguments.get("session_id").and_then(Value::as_str)?;
                    let plan_id = json_arguments.get("plan_id").and_then(Value::as_str)?;
                    match op {
                        "start_execution" => Some(true),
                        "complete_step" => {
                            let plan = plan_store.plan(session, plan_id).ok()?;
                            Some(plan.lifecycle == PlanLifecycleState::Draft)
                        }
                        _ => Some(false),
                    }
                }();
                if requires_execution_approval == Some(true) {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_millis() as u64)
                        .unwrap_or(u64::MAX);
                    let valid = plan_execution_approval
                        .map(|approval| {
                            approval.session_id == session_id.unwrap_or_default()
                                && json_arguments.get("plan_id").and_then(Value::as_str)
                                    == Some(approval.plan_id.as_str())
                                && now_ms < approval.expires_at_ms
                        })
                        .unwrap_or(false);
                    if !valid {
                        return PermissionDecision {
                            verdict: PermissionVerdict::Deny,
                            decision_source: "plan_execution_approval_gate".to_string(),
                            reason: Some(
                                "host plan execution approval is required to transition a Draft plan to Executing (missing, mismatched, or expired)."
                                    .to_string(),
                            ),
                        };
                    }
                }
                // 其余 plan_control op 豁免草稿写门禁（既有 PA-138 语义不变）。
            } else if is_write_or_exec {
                // PA-138：唯一会话判据 = 运行期注入的 `DispatchContext.session_id`。
                // 未绑定会话上下文（None）时**不得**回退到默认/环境/测试会话名，而是走
                // fail-closed 判定：PlanStore 存在任一 Draft ⇒ 拒绝（C-138-4）。
                let draft_gate_tripped = match session_id {
                    Some(session_id) => plan_store.has_active_draft_plan(session_id),
                    None => plan_store.has_any_draft_plan(),
                };
                if draft_gate_tripped {
                    return PermissionDecision {
                        verdict: PermissionVerdict::Deny,
                        decision_source: "plan_draft_gate".to_string(),
                        reason: Some(
                            "Plan is currently in Draft state; write and execute operations are blocked."
                                .to_string(),
                        ),
                    };
                }
            }
        }

        // 3. 默认放行
        PermissionDecision {
            verdict: PermissionVerdict::Allow,
            decision_source: "governed_default".to_string(),
            reason: None,
        }
    }
}

impl ToolPolicyEvaluator for GovernedPolicyEvaluator {
    fn evaluate(
        &self,
        descriptor: &crate::agent::tools::ToolDescriptor,
        _origin: &InvocationOrigin,
        final_arguments: &Value,
    ) -> PermissionDecision {
        // 上下文无关的旧 seam（既有单元/集成测试直接调用）：无注入会话事实，
        // 草稿门禁按 fail-closed 规则处理（C-138-4）。旧 seam 无宿主审批（None）。
        self.decide_with_context(descriptor, None, None, final_arguments)
    }

    fn evaluate_with_context(
        &self,
        descriptor: &crate::agent::tools::ToolDescriptor,
        _origin: &InvocationOrigin,
        final_arguments: &Value,
        context: &DispatchContext,
    ) -> PermissionDecision {
        // PA-141：审批证据**仅**来自宿主注入的 `DispatchContext`；
        // `final_arguments`（模型可控）不参与审批判定（C-141-3）。
        self.decide_with_context(
            descriptor,
            context.session_id.as_deref(),
            context.plan_execution_approval.as_ref(),
            final_arguments,
        )
    }
}

/// Bridges one builtin primitive to the legacy `ToolRouter` implementation, preserving the exact
/// `ToolResult.output` byte-for-byte on success (returned as a string `Value` so the dispatcher's
/// `render_output` passes it through unchanged).
struct RouterPrimitiveHandler {
    router: Arc<ToolRouter>,
    primitive: String,
}

impl PrimitiveToolHandler for RouterPrimitiveHandler {
    fn execute(&self, request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
        // PA-080 P1-2：会话级 workspace root 透传给 ToolRouter 的权限判定。
        let workspace_root = request
            .workspace_root
            .as_deref()
            .map(std::path::Path::new);
        let result = self
            .router
            .execute_with_workspace_root(
                &ToolCall {
                    call_id: None,
                    name: self.primitive.clone(),
                    arguments: request.arguments.clone(),
                    plan: None,
                },
                workspace_root,
            );
        if result.status == "ok" {
            Ok(Value::String(result.output))
        } else {
            Err(legacy_error_message(&result.output))
        }
    }
}

/// Build a fail-closed `code: message` handler error from a legacy `ToolResult.output`, preferring
/// the structured `error.code`/`error.message` when present so the dispatcher's `handler_failure`
/// can lift a known code.
fn legacy_error_message(output: &str) -> String {
    if let Ok(value) = serde_json::from_str::<Value>(output) {
        if let Some(error) = value.get("error") {
            let code = error
                .get("code")
                .and_then(Value::as_str)
                .unwrap_or("handler_error");
            let message = error
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or(output);
            return format!("{code}: {message}");
        }
    }
    format!("handler_error: {output}")
}

/// Build a governed executor over the full builtin tool surface, ready to be adopted as the
/// runtime's tool-execution engine. `workspace_root` seeds the `ToolRouter` implementations and the
/// dispatcher's workspace facts. `authorize_store`（PA-080）与 host control plane 共享同一
/// `Arc`：`authorize_path` 的授权变更即时对工具读判定生效。
pub fn build_governed_executor(
    workspace_root: Option<PathBuf>,
    authorize_store: Option<Arc<crate::agent::path_permission::AuthorizeStore>>,
) -> GovernedToolExecutor {
    build_governed_executor_with_sandbox_backend(workspace_root, authorize_store, None)
}

/// 生产沙箱装配的显式 override 入口（测试注入用）：`Some(backend)` 时直接注册该后端；
/// `None` 时走默认生产路径（Native 优先、fallback HostApprovedUnsandboxed）。
pub fn build_governed_executor_with_sandbox_backend(
    workspace_root: Option<PathBuf>,
    authorize_store: Option<Arc<crate::agent::path_permission::AuthorizeStore>>,
    sandbox_override: Option<Arc<dyn SandboxBackend>>,
) -> GovernedToolExecutor {
    let registry = Arc::new(
        ToolRegistrySnapshot::builtin().expect("builtin registry must validate"),
    );
    let clock: Arc<dyn RuntimeClock> = Arc::new(SystemClock);
    let dispatcher = GovernedDispatcher::new(registry.clone(), clock);
    
    // Each runtime owns one PlanStore; session isolation is enforced by the store's
    // CrossSession guard on every operation (PA-076 Decision 6). ──
    let shared_plan_store = Arc::new(PlanStore::new());

    // W30b：加载并注册基于规则与真实共享 Plan 状态的策略评估器
    let settings = crate::agent::app_settings::AppSettingsStore::new().load_view();
    dispatcher.register_policy_evaluator(Arc::new(GovernedPolicyEvaluator::new(
        settings.tool_rules,
        Some(Arc::clone(&shared_plan_store)),
    )));
    // Legacy-compatible budgets: the switch must reproduce the legacy ToolRouter's lack of
    // tool-level output-byte/deadline caps (phase-4..7 review P1-2), not silently regress large
    // reads/batches. The governed path still enforces cancellation + child-call budgets.
    dispatcher.set_budget_config(DispatchBudgetConfig {
        unbounded_output_and_deadline: true,
        ..Default::default()
    });
    // 生产链路沙箱装配（design Decision 7 / DSH danger-full-access 信任透传语义）：
    // Execute-scope（workspace_run_command）有 dispatcher 预检 + legacy handler 两层门禁，
    // 两层必须装配同一决策的后端，否则 dispatcher 放行后 handler 仍 fail-closed。
    // 优先级：显式 override > Native 可用 > HostApproved。
    // override（测试注入）只替换 dispatcher 预检层；handler 层沿用生产默认，保证 Run 可执行。
    let native_probe = NativeSandboxBackend::detect();
    let native_available = native_probe.availability() == SandboxAvailability::Available;
    match &sandbox_override {
        Some(backend) => dispatcher.register_sandbox_backend(Arc::clone(backend)),
        None => {
            if native_available {
                dispatcher.register_sandbox_backend(Arc::new(native_probe.clone()));
            } else {
                dispatcher.register_sandbox_backend(Arc::new(
                    HostApprovedUnsandboxedBackend::new(),
                ));
            }
        }
    }

    // 默认工作区分叉收敛：None 兜底取 compute_default_workspace_root()
    //（Windows Documents/pony_agent，Unix ~/pony_agent），dirs 解析失败才回退 cwd。
    let workspace = workspace_root.unwrap_or_else(|| {
        crate::agent::workspace::compute_default_workspace_root()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    });
    // PA-080：授权清单（如有）与 host control plane 共享同一 `Arc`；
    // 缺省（测试/legacy 构造）时工具判定使用空授权清单（workspace 外读一律 requires_authorization）。
    let shared_authorizations = authorize_store
        .unwrap_or_else(|| Arc::new(crate::agent::path_permission::AuthorizeStore::new()));
    // handler 层（legacy ToolRouter.Run）与 dispatcher 预检层装配同一决策的后端：
    // 否则预检放行后 handler 仍以 NoSandboxBackend 报 sandbox_denied。
    let router = if native_available {
        ToolRouter::with_workspace_root(workspace.clone())
            .with_authorize_store(Arc::clone(&shared_authorizations))
            .with_sandbox_backend(native_probe)
    } else {
        ToolRouter::with_workspace_root(workspace.clone())
            .with_authorize_store(Arc::clone(&shared_authorizations))
            .with_sandbox_backend(HostApprovedUnsandboxedBackend::new())
    };
    let router = Arc::new(router);

    // ── Plan state control: session-owned, revisioned create/replace/merge/complete-step.
    // Each runtime owns one PlanStore; session isolation is enforced by the store's
    // CrossSession guard on every operation (PA-076 Decision 6). ──
    let plan_handler = Arc::new(PlanControlHandler::new((*shared_plan_store).clone()));

    // ── view_image: reference-based artifact with default include_bytes=false (design Decision 11).
    // Whether the encoded bytes reach a model is a provider modality choice. ──
    let view_image_handler = Arc::new(
        ViewImageHandler::new(workspace.clone()).with_authorizations(Arc::clone(&shared_authorizations)),
    );

    // ── workspace_read_document: local office document → Markdown via anydoc, workspace-scoped,
    // with explicit output/input byte budgets and truncated evidence. ──
    let read_document_handler = Arc::new(
        ReadDocumentHandler::new(workspace.clone()).with_authorizations(Arc::clone(&shared_authorizations)),
    );

    // Register every builtin primitive handler (composites are registered separately below).
    for descriptor in &registry.descriptors {
        let primitive = descriptor.identity.primitive_name.clone();
        if COMPOSITE_PRIMITIVES.contains(&primitive.as_str()) {
            continue;
        }
        // ── MCP resource read + ToolSearch boundary decision (PA-076 P2-6): these descriptors
        // stay in the capability registry execution path; their builtin entries are discovery
        // metadata only. Deliberately NOT registering governed handlers — doing so would create
        // a forbidden double execution path (governed dispatcher vs. runtime capability registry).
        // See tools.rs builtin_tools() comments and runtime/tool_exec.rs. ──
        if primitive == TOOL_PLAN_CONTROL {
            dispatcher.register_handler(
                descriptor.identity.descriptor_id.clone(),
                Arc::clone(&plan_handler) as Arc<dyn PrimitiveToolHandler>,
            );
            continue;
        }
        if primitive == TOOL_VIEW_IMAGE {
            dispatcher.register_handler(
                descriptor.identity.descriptor_id.clone(),
                Arc::clone(&view_image_handler) as Arc<dyn PrimitiveToolHandler>,
            );
            continue;
        }
        if primitive == TOOL_WORKSPACE_READ_DOCUMENT {
            dispatcher.register_handler(
                descriptor.identity.descriptor_id.clone(),
                Arc::clone(&read_document_handler) as Arc<dyn PrimitiveToolHandler>,
            );
            continue;
        }
        dispatcher.register_handler(
            descriptor.identity.descriptor_id.clone(),
            Arc::new(RouterPrimitiveHandler {
                router: Arc::clone(&router),
                primitive,
            }),
        );
    }

    // Register the governed composites with the read-only child authority matching the legacy gate.
    let authority = READ_ONLY_PRIMITIVES
        .iter()
        .map(|primitive| format!("builtin:{primitive}"))
        .collect::<Vec<_>>();
    register_governed_composites(&dispatcher, authority.clone(), authority);

    GovernedToolExecutor::new(dispatcher)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tool_runtime::PendingControlRequestKind;
    use crate::agent::tools::ToolExecutor;
    use serde_json::json;
    use std::fs;

    static TEMP_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn temp_workspace() -> PathBuf {
        // Unique per call so parallel tests in one process never share/race a fixture directory.
        let seq = TEMP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = { let raw = std::env::temp_dir().join(format!(
            "pony-governed-executor-test-{}-{seq}",
            std::process::id()
        )); raw.canonicalize().unwrap_or(raw) };
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub")).expect("create test workspace");
        root
    }

    fn same_call(name: &str, arguments: Value) -> ToolCall {
        ToolCall {
            call_id: None,
            name: name.to_string(),
            arguments,
            plan: None,
        }
    }

    #[test]
    fn governed_executor_matches_legacy_router_output_for_model_visible_tools() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.rs"), "fn main() {}\n").expect("write fixture");
        fs::write(workspace.join("sub/mod.rs"), "pub fn helper() {}\n").expect("write fixture");

        let legacy = ToolRouter::with_workspace_root(workspace.clone());
        let governed = build_governed_executor(Some(workspace.clone()), None);

        // The model-visible product surface (what the runtime actually dispatches): List, Search,
        // Glob. Read/Write/Edit are covered separately; Run is sandbox-gated; Ask is host-mediated
        // (covered by its own assertion below — no longer a legacy echo).
        let cases = vec![
            same_call("List", json!({ "path": ".", "description": "test" })),
            same_call(
                "Search",
                json!({ "query": "fn", "path": ".", "description": "test" }),
            ),
            same_call("Glob", json!({ "pattern": "**/*.rs", "description": "test" })),
        ];

        for call in cases {
            let legacy_result = legacy.execute(&call);
            let governed_result = governed.execute(&call);
            assert_eq!(
                governed_result.status, legacy_result.status,
                "status mismatch for `{}`: {}",
                call.name,
                governed_result.output
            );
            // Search/Glob outputs now carry `durationMs` (non-deterministic wall time); strip it
            // before the byte-for-byte comparison so only deterministic fields are compared.
            assert_eq!(
                normalize_output(&governed_result.output),
                normalize_output(&legacy_result.output),
                "output mismatch for `{}`",
                call.name
            );
        }
    }

    #[test]
    fn governed_executor_ask_is_host_mediated_and_persists_the_question() {
        let workspace = temp_workspace();
        let governed = build_governed_executor(Some(workspace), None);
        governed.set_context(crate::agent::dispatcher::DispatchContext {
            session_id: Some("session-ask".to_string()),
            run_id: Some("run-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            ..Default::default()
        });
        let result = governed.execute(&same_call(
            "Ask",
            json!({ "question": "继续吗？", "description": "test" }),
        ));
        // Pending control outcome surfaces as the legacy `control_outcome_pending` marker, which
        // the runtime detects to suspend the run (design Decision 5, P1-1 wiring).
        assert_eq!(result.status, "error");
        assert!(
            result.output.contains("control_outcome_pending"),
            "expected pending control marker, got: {}",
            result.output
        );
        // The persisted request is an Interaction kind bound to the real session, with the
        // model's question surfaced verbatim (P2-9).
        let pending = governed.dispatcher().pending_requests();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].request_kind, PendingControlRequestKind::Interaction);
        assert_eq!(pending[0].session_id.as_deref(), Some("session-ask"));
        assert_eq!(pending[0].turn_id, "turn-1");
        assert_eq!(pending[0].prompt.as_deref(), Some("继续吗？"));
    }

    /// Parse a tool output and drop the non-deterministic `durationMs` field for fidelity
    /// comparisons.
    fn normalize_output(output: &str) -> Value {
        if let Ok(mut value) = serde_json::from_str::<Value>(output) {
            if let Some(object) = value.as_object_mut() {
                object.remove("durationMs");
            }
            value
        } else {
            Value::String(output.to_string())
        }
    }

    #[test]
    fn governed_executor_matches_legacy_router_for_write_and_edit() {
        let workspace = temp_workspace();
        let legacy = ToolRouter::with_workspace_root(workspace.clone());
        let governed = build_governed_executor(Some(workspace.clone()), None);

        let write = same_call(
            "Write",
            json!({ "path": "out.txt", "content": "hello\n", "description": "test" }),
        );
        // Reset the file before each run so `overwroteExisting` is identical.
        fs::remove_file(workspace.join("out.txt")).ok();
        let legacy_write = legacy.execute(&write);
        assert_eq!(legacy_write.status, "ok");
        fs::remove_file(workspace.join("out.txt")).ok();
        let governed_write = governed.execute(&write.clone());
        assert_eq!(
            governed_write.status, "ok",
            "governed write failed: {}",
            governed_write.output
        );
        assert_eq!(governed_write.output, legacy_write.output);

        let edit = same_call(
            "Edit",
            json!({
                "path": "out.txt",
                "oldText": "hello",
                "newText": "world",
                "description": "test",
                // wave-22 冻结的读前写不变量:未传 expectedHash ⇒ fs_not_observed。
                "expectedHash": crate::agent::tools::compute_content_sha256("hello\n"),
            }),
        );
        // Re-create the fixture before each edit run (edit requires the target to exist).
        fs::write(workspace.join("out.txt"), "hello\n").expect("write fixture");
        let legacy_edit = legacy.execute(&edit);
        assert_eq!(
            legacy_edit.status, "ok",
            "legacy edit failed: {}",
            legacy_edit.output
        );
        fs::write(workspace.join("out.txt"), "hello\n").expect("write fixture");
        let governed_edit = governed.execute(&edit.clone());
        assert_eq!(
            governed_edit.status, "ok",
            "governed edit failed: {}",
            governed_edit.output
        );
        assert_eq!(governed_edit.output, legacy_edit.output);
    }

    #[test]
    fn governed_executor_registers_production_sandbox_backend_for_run() {
        use crate::agent::sandbox::NoSandboxBackend;
        use crate::agent::tool_runtime::SandboxBackend;
        let workspace = temp_workspace();
        // 默认生产路径必须已注册后端：不再报 "no sandbox backend is registered" 指纹。
        let governed = build_governed_executor(Some(workspace), None);
        let result = governed.execute(&same_call(
            "Run",
            json!({ "command": "echo hi", "description": "test" }),
        ));
        assert!(
            !result.output.contains("no sandbox backend is registered"),
            "production path must register a sandbox backend, got: {}",
            result.output
        );
        // 显式 override 入口（测试注入）：NoSandboxBackend 恢复 fail-closed 预检。
        let governed_override = build_governed_executor_with_sandbox_backend(
            Some(temp_workspace()),
            None,
            Some(Arc::new(NoSandboxBackend) as Arc<dyn SandboxBackend>),
        );
        let denied = governed_override.execute(&same_call(
            "Run",
            json!({ "command": "echo hi", "description": "test" }),
        ));
        assert_eq!(denied.status, "error");
        assert!(
            denied.output.contains("sandbox_unavailable"),
            "override backend must gate execute scope, got: {}",
            denied.output
        );
    }

    #[test]
    fn governed_executor_exposes_governed_dispatcher_through_trait_downcast() {
        let workspace = temp_workspace();
        let executor: Box<dyn ToolExecutor> =
            Box::new(build_governed_executor(Some(workspace), None));
        // PA-076 P1-1: the runtime reaches the shared dispatcher via `ToolExecutor::as_any`.
        let governed = executor
            .as_any()
            .and_then(|any| any.downcast_ref::<GovernedToolExecutor>())
            .expect("default governed executor must downcast to GovernedToolExecutor");
        assert!(governed.dispatcher().registry().descriptors.len() > 0);
        // A non-governed executor exposes nothing (backward-compatible default).
        struct PlainExecutor;
        impl ToolExecutor for PlainExecutor {
            fn execute(&self, _call: &ToolCall) -> crate::agent::tools::ToolResult {
                crate::agent::tools::ToolResult {
                    tool_name: "plain".to_string(),
                    status: "ok".to_string(),
                    output: "{}".to_string(),
                    duration_ms: 0,
                }
            }
        }
        assert!(PlainExecutor.as_any().is_none());
    }

    #[test]
    fn governed_executor_runs_read_only_batch_children_through_child_dispatch() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.rs"), "fn main() {}\n").expect("write fixture");
        let governed = build_governed_executor(Some(workspace), None);

        let result = governed.execute(&same_call(
            "BatchExecute",
            json!({
                "calls": [
                    { "name": "workspace_read_file", "arguments": { "path": "demo.rs", "description": "test" } },
                    { "name": "workspace_path_info", "arguments": { "path": "demo.rs", "description": "test" } },
                ],
                "description": "test",
            }),
        ));
        assert_eq!(result.status, "ok");
        let payload: Value = serde_json::from_str(&result.output).expect("batch output json");
        assert_eq!(payload["ok"], true);
        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["successCount"], 2);
        assert_eq!(payload["results"].as_array().map(Vec::len), Some(2));
    }

    // ── PA-076 P2-6: plan_control via governed dispatch ──────────────────────────────────────

    #[test]
    fn governed_executor_dispatches_plan_control_create_replace_merge_and_complete_step() {
        let workspace = temp_workspace();
        let governed = build_governed_executor(Some(workspace.clone()), None);
        governed.set_context(crate::agent::dispatcher::DispatchContext {
            session_id: Some("session-1".to_string()),
            run_id: Some("run-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            // W36 / PA-141：Draft→Executing 迁移（complete_step 首步）需要宿主审批证据。
            plan_execution_approval: Some(crate::agent::dispatcher::PlanExecutionApproval {
                session_id: "session-1".to_string(),
                plan_id: "plan-1".to_string(),
                expires_at_ms: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64 + 3_600_000)
                    .unwrap_or(u64::MAX),
            }),
            ..Default::default()
        });

        // Create a draft plan with two steps.
        let created = governed.execute(&same_call(
            "Plan",
            json!({
                "op": "create",
                "session_id": "session-1",
                "payload": {
                    "kind": "implement",
                    "summary": "Implement feature",
                    "steps": [
                        { "name": "draft", "summary": "Write spec" },
                        { "name": "code", "summary": "Implement" }
                    ]
                },
                "description": "test",
            }),
        ));
        assert_eq!(created.status, "ok", "create failed: {}", created.output);
        let created_value: Value =
            serde_json::from_str(&created.output).expect("plan output json");
        assert_eq!(created_value["planId"], "plan-1");
        assert_eq!(created_value["revision"], 1);
        assert_eq!(created_value["lifecycle"], "draft");
        assert_eq!(created_value["steps"].as_array().map(Vec::len), Some(2));

        // Merge a third step, preserving existing step ids.
        let merged = governed.execute(&same_call(
            "Plan",
            json!({
                "op": "merge",
                "session_id": "session-1",
                "plan_id": "plan-1",
                "revision": 1,
                "step": { "name": "test", "summary": "Add tests" },
                "description": "test",
            }),
        ));
        assert_eq!(merged.status, "ok", "merge failed: {}", merged.output);
        let merged_value: Value = serde_json::from_str(&merged.output).expect("plan output json");
        assert_eq!(merged_value["revision"], 2);
        assert_eq!(merged_value["steps"].as_array().map(Vec::len), Some(3));

        // Complete the first step → lifecycle advances to Executing.
        let step_id = merged_value["steps"][0]["stepId"]
            .as_str()
            .expect("step id")
            .to_string();
        let completed = governed.execute(&same_call(
            "Plan",
            json!({
                "op": "complete_step",
                "session_id": "session-1",
                "plan_id": "plan-1",
                "revision": 2,
                "step_id": step_id,
                "description": "test",
            }),
        ));
        assert_eq!(
            completed.status, "ok",
            "complete_step failed: {}",
            completed.output
        );
        let completed_value: Value =
            serde_json::from_str(&completed.output).expect("plan output json");
        assert_eq!(completed_value["steps"][0]["status"], "completed");
        assert_eq!(completed_value["lifecycle"], "executing");

        // Replace the plan content, keeping the same plan_id.
        let replaced = governed.execute(&same_call(
            "Plan",
            json!({
                "op": "replace",
                "session_id": "session-1",
                "plan_id": "plan-1",
                "revision": 3,
                "payload": { "kind": "refactor", "summary": "Refactor after review", "steps": [] },
                "description": "test",
            }),
        ));
        assert_eq!(
            replaced.status, "ok",
            "replace failed: {}",
            replaced.output
        );
        let replaced_value: Value =
            serde_json::from_str(&replaced.output).expect("plan output json");
        assert_eq!(replaced_value["planId"], "plan-1");
        assert_eq!(replaced_value["kind"], "refactor");
        assert_eq!(replaced_value["revision"], 4);
    }

    #[test]
    fn governed_executor_plan_control_stale_revision_and_missing_op_fail_closed() {
        let workspace = temp_workspace();
        let governed = build_governed_executor(Some(workspace.clone()), None);
        governed.set_context(crate::agent::dispatcher::DispatchContext {
            session_id: Some("session-1".to_string()),
            ..Default::default()
        });

        // Set up a plan so a stale revision can be attempted.
        let created = governed.execute(&same_call(
            "Plan",
            json!({
                "op": "create",
                "session_id": "session-1",
                "payload": { "kind": "k", "summary": "s", "steps": [{ "name": "a", "summary": "sa" }] },
                "description": "test",
            }),
        ));
        assert_eq!(created.status, "ok", "plan setup failed: {}", created.output);

        let stale = governed.execute(&same_call(
            "Plan",
            json!({
                "op": "replace",
                "session_id": "session-1",
                "plan_id": "plan-1",
                "revision": 99,
                "payload": { "kind": "x", "summary": "y", "steps": [] },
                "description": "test",
            }),
        ));
        assert_eq!(stale.status, "error");
        assert!(stale.output.contains("stale_revision"), "{}", stale.output);

        // Missing `op` is rejected by schema validation.
        let no_op = governed.execute(&same_call(
            "Plan",
            json!({
                "session_id": "session-1",
                "description": "test",
            }),
        ));
        assert_eq!(no_op.status, "error");
        assert!(
            no_op.output.contains("missing required argument `op`"),
            "{}",
            no_op.output
        );
    }

    // ── PA-076 P2-6: view_image via governed dispatch ────────────────────────────────────────

    fn write_minimal_png(path: &std::path::Path, width: u32, height: u32) {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        bytes.extend_from_slice(&[0, 0, 0, 13]);
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 0, 0]); // CRC placeholder
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(b"IEND");
        std::fs::write(path, bytes).expect("png fixture should write");
    }

    #[test]
    fn governed_executor_view_image_reference_artifact_default_omits_bytes() {
        let workspace = temp_workspace();
        write_minimal_png(&workspace.join("photo.png"), 320, 240);

        let governed = build_governed_executor(Some(workspace.clone()), None);
        let result = governed.execute(&same_call(
            "ViewImage",
            json!({ "path": "photo.png", "description": "test" }),
        ));
        assert_eq!(
            result.status, "ok",
            "view_image failed: {}",
            result.output
        );
        let value: Value =
            serde_json::from_str(&result.output).expect("view_image output json");
        assert_eq!(value["mimeType"], "image/png");
        assert_eq!(value["width"], 320);
        assert_eq!(value["height"], 240);
        assert_eq!(value["bytes"], Value::Null, "default include_bytes=false");
        assert_eq!(value["truncated"], false);
    }

    #[test]
    fn governed_executor_view_image_include_bytes_embeds_capped_payload() {
        let workspace = temp_workspace();
        write_minimal_png(&workspace.join("photo.png"), 64, 64);
        let full_len =
            std::fs::metadata(&workspace.join("photo.png")).unwrap().len();

        let governed = build_governed_executor(Some(workspace.clone()), None);
        // includeBytes=true with a small maxBytes → capped, truncated.
        let result = governed.execute(&same_call(
            "ViewImage",
            json!({
                "path": "photo.png",
                "includeBytes": true,
                "maxBytes": 16,
                "description": "test",
            }),
        ));
        assert_eq!(result.status, "ok", "capped view_image failed: {}", result.output);
        let value: Value =
            serde_json::from_str(&result.output).expect("view_image output json");
        assert_eq!(value["truncated"], true);
        assert_eq!(value["bytesLen"], full_len);
        // bytes are base64-encoded in the serialization; verify non-null.
        assert!(!value["bytes"].is_null());
    }

    #[test]
    fn governed_executor_view_image_missing_path_and_unknown_format_fail_closed() {
        let workspace = temp_workspace();
        let governed = build_governed_executor(Some(workspace.clone()), None);

        let missing = governed.execute(&same_call(
            "ViewImage",
            json!({ "description": "test" }),
        ));
        assert_eq!(missing.status, "error");
        assert!(
            missing.output.contains("missing required argument `path`"),
            "{}",
            missing.output
        );

        // A .txt file with valid PNG magic is rejected by extension check.
        std::fs::write(workspace.join("notes.txt"), b"not an image").expect("write");
        let unknown = governed.execute(&same_call(
            "ViewImage",
            json!({ "path": "notes.txt", "description": "test" }),
        ));
        assert_eq!(unknown.status, "error");
        assert!(
            unknown.output.contains("unsupported image extension"),
            "{}",
            unknown.output
        );
    }
}
