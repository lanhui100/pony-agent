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
//! - The dispatcher's conservative default (`DescriptorPolicyEvaluator`, which lifts any
//!   write/execute-scope builtin to `ApprovalRequired`) is intentionally overridden here: this
//!   seam registers a `GovernedPolicyEvaluator` built from the user's persisted tool rules
//!   (`AppSettingsStore::tool_rules`, wave-30b). Its fallback for an unmatched tool is `Allow`
//!   (`decision_source = "governed_default"`), i.e. model-origin writes/executes run without
//!   approval by default, reproducing the legacy `ToolRouter`'s direct-execution behavior.
//!   Real approval is therefore delegated to: (a) explicit `Ask`/`Deny` tool rules,
//!   (b) the Plan-draft write gate (`plan_draft_gate`), and (c) path authorization
//!   (`path_permission_requires_authorization` -> `ApprovalRequired`), plus the host-mediated
//!   `Ask` control path. Restoring a global fail-closed write/execute default would be a
//!   product/security-posture change, not a documentation change.
//! - Execute-scope tools (`workspace_run_command`) run behind the production sandbox
//!   assembly below: `NativeSandboxBackend` when available, else
//!   `HostApprovedUnsandboxedBackend` (DSH danger-full-access trust passthrough).
//!   Fail-closed (`sandbox_unavailable`/`sandbox_denied`) is still enforced by both the
//!   dispatcher preflight and the legacy handler when the backend rejects.
//! - Legacy tool error codes that are not in the dispatcher's known-code set surface as
//!   `handler_error`; status and message fidelity are preserved.

use pony_protocol::budget::DispatchBudgetConfig;
use crate::dispatcher::{
    DispatchContext, GovernedDispatcher, PermissionDecision, PermissionVerdict,
    ToolPolicyEvaluator,
};
use crate::dispatcher_composites::{
    register_governed_composites, GovernedToolExecutor,
};
use crate::document_conversion::ReadDocumentHandler;
use crate::image_artifact::ViewImageHandler;
use crate::exit_plan_mode::ExitPlanModeHandler;
use crate::sandbox::{HostApprovedUnsandboxedBackend, NativeSandboxBackend};
use crate::tool_runtime::{
    InvocationOrigin, PrimitiveToolHandler, PrimitiveToolHandlerRequest, RuntimeClock,
    SandboxAvailability, SandboxBackend, SystemClock,
};
use crate::tools::{
    ToolCall, ToolExecutionContext, ToolRegistrySnapshot, ToolRouter, TOOL_ASK_USER,
    TOOL_PLAN_CONTROL, TOOL_WORKSPACE_READ_DOCUMENT, TOOL_VIEW_IMAGE,
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
        descriptor: &crate::tools::ToolDescriptor,
        origin: &InvocationOrigin,
        final_arguments: &Value,
    ) -> PermissionDecision {
        GovernedPolicyEvaluator::default().evaluate(descriptor, origin, final_arguments)
    }
}

/// 治理型策略评估器：工具级规则（Allow / Ask / Deny）+ Plan 模式写/执行硬拦截门禁
/// （ADR 2026-11-27-plan-mode-dsh-alignment-and-task-count）。
///
/// Plan 状态已迁到进程级 `plan_mode` 注册表（会话级协作模式），本评估器不再持有步骤机。
#[derive(Clone, Debug, Default)]
pub struct GovernedPolicyEvaluator {
    pub rules: Vec<crate::app_settings::ToolRule>,
    pub authorize_store: Option<Arc<crate::path_permission::AuthorizeStore>>,
}

impl GovernedPolicyEvaluator {
    pub fn new(rules: Vec<crate::app_settings::ToolRule>) -> Self {
        Self { rules, authorize_store: None }
    }

    pub fn with_authorize_store(mut self, authorize_store: Arc<crate::path_permission::AuthorizeStore>) -> Self {
        self.authorize_store = Some(authorize_store);
        self
    }

    /// 裁定核心：`session_id` 是**运行时注入**的权威会话（`DispatchContext.session_id`）：
    /// 工具参数不是会话键（生产 schema 不允许，且可被模型控制）；`None` 表示本次 dispatch
    /// 未绑定会话上下文（按 fail-closed 语义交由只读闸门兜底）。
    ///
    /// `plan_execution_approval` 形参已**不再参与任何判定**：`Plan` 步骤机下线后，
    /// 解锁写权限的唯一路径是「用户批准 `ExitPlanMode` 提交的方案」或 `/plan off`
    /// （ADR 2026-11-27-plan-mode-dsh-alignment-and-task-count）。该形参保留是为了让
    /// `DispatchContext` 的既有字段与调用面零改动（线缆兼容），语义上已是惰性数据。
    fn decide_with_context(
        &self,
        descriptor: &crate::tools::ToolDescriptor,
        session_id: Option<&str>,
        _plan_execution_approval: Option<&crate::dispatcher::PlanExecutionApproval>,
        workspace_root: Option<&str>,
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
                    crate::app_settings::RuleAction::Allow => PermissionDecision {
                        verdict: PermissionVerdict::Allow,
                        decision_source: "tool_rule_allow".to_string(),
                        reason: rule.reason.clone(),
                    },
                    crate::app_settings::RuleAction::Ask => PermissionDecision {
                        verdict: PermissionVerdict::ApprovalRequired,
                        decision_source: "tool_rule_ask".to_string(),
                        reason: rule.reason.clone(),
                    },
                    crate::app_settings::RuleAction::Deny => PermissionDecision {
                        verdict: PermissionVerdict::Deny,
                        decision_source: "tool_rule_deny".to_string(),
                        reason: rule.reason.clone().or_else(|| {
                            Some(format!("Tool `{}` matched deny rule", descriptor.identity.model_name))
                        }),
                    },
                };
            }
        }

        // 2. Plan 模式门禁（ADR 2026-11-27-plan-mode-dsh-alignment-and-task-count，方案 1A）：
        // 会话处于 Plan 模式时**硬拦截写/执行类工具**，只放行只读检索、提问与退出审查工具。
        // 退出路径唯一：宿主 Ask 批准 `ExitPlanMode`（或宿主 `/plan off`）。
        // 判定与 read_only 闸门**同口径**——按权限事实（`kind` + descriptor 声明的 scopes），
        // 不看名字，否则外部工具（MCP/插件，`kind = External`）只要声明 `workspace_write`
        // 就能在 Plan 模式下绕过写保护（L3-B-C 实测过的同类漂移）。
        let is_write_or_exec = matches!(
            descriptor.kind,
            crate::tools::ToolKind::Write | crate::tools::ToolKind::Execute
        ) || descriptor.permission_declaration.scopes.iter().any(|scope| {
            matches!(
                scope,
                crate::tools::ToolPermissionScope::WorkspaceWrite
                    | crate::tools::ToolPermissionScope::WorkspaceExecute
            )
        });
        // 例外一：`ExitPlanMode`（primitive `plan_control`）自身必须可调用——它是解除本门禁的
        // 唯一机制（自锁死防护）。
        let is_exit_plan_mode = descriptor.identity.primitive_name == TOOL_PLAN_CONTROL;
        // 例外二：宿主 Ask（`WaitingHost`）在步骤 1 已提前返回，不参与写判定。
        if is_write_or_exec && !is_exit_plan_mode {
            // 会话判据唯一来源 = 运行期注入的 `DispatchContext.session_id`（模型实参不是会话键）。
            // 未绑定会话 ⇒ 无从证明该会话处于 Plan 模式，故按「未激活」处理并交由
            // read_only 闸门兜底，不回落默认/环境会话名（与 C-138-4 同向 fail-closed）。
            if let Some(session_id) = session_id {
                if crate::plan_mode::is_active(session_id) {
                    return PermissionDecision {
                        verdict: PermissionVerdict::Deny,
                        decision_source: "plan_mode_gate".to_string(),
                        reason: Some(
                            "Plan mode is active: write and execute tools are blocked until the user \
                             approves the plan through ExitPlanMode (or leaves with /plan off)."
                                .to_string(),
                        ),
                    };
                }
            }
        }

        // 3. 路径安全门禁（PA-080 / 审批联动）：
        // 若工具需要读取外部文件（如 Read / ReadDocument / ViewImage），检查路径是否位于工作区之外且未获得授权。
        // 未获得授权时返回 ApprovalRequired，触发宿主审批流（对话框），而不是直接报错。
        if let Some(auth_store) = &self.authorize_store {
            let prim = descriptor.identity.primitive_name.as_str();
            let model = descriptor.identity.model_name.as_str();
            let is_file_read_tool = prim == "workspace_read_file"
                || prim == "workspace_read_file_segment"
                || prim == "workspace_read_document"
                || prim == "view_image"
                || prim == "workspace_gather_context"
                || model == "Read"
                || model == "ReadDocument"
                || model == "ViewImage";
            if is_file_read_tool {
                let target_path = json_arguments.get("path")
                    .or_else(|| json_arguments.get("filePath"))
                    .or_else(|| json_arguments.get("file_path"))
                    .and_then(Value::as_str);
                if let Some(raw_path) = target_path {
                    let ws_root = workspace_root
                        .map(PathBuf::from)
                        .or_else(|| crate::workspace::compute_default_workspace_root())
                        .unwrap_or_else(|| PathBuf::from("."));
                    let tmp = ws_root.join(".tmp");
                    let checker = crate::path_permission::PathPermissionChecker::with_default();
                    if let Err(err) = checker.classify(
                        raw_path,
                        &ws_root,
                        &tmp,
                        auth_store,
                        crate::path_permission::PathPurpose::Read,
                    ) {
                        if err.code == crate::path_permission::PermissionErrorCode::RequiresAuthorization {
                            return PermissionDecision {
                                verdict: PermissionVerdict::ApprovalRequired,
                                decision_source: "path_permission_requires_authorization".to_string(),
                                reason: Some(err.message),
                            };
                        }
                    }
                }
            }
        }

        // 4. 外部来源尊重 `requires_approval`（ADR 0066 decision 5 / T2 裁决，fail-closed）。
        //
        // 口径：**只有**外部来源（MCP / 插件 / 动态工具，即 provenance 不是 `builtin-tools`）
        // 且 descriptor 显式声明 `requires_approval` 时才要求宿主审批；内建工具行为保持不变
        // （绝不能顺手把 45 个内建工具的审批面一起翻掉）。
        // 位置刻意放在最后一步之前：用户工具规则（Allow/Ask/Deny）、Plan 草稿门禁、路径授权
        // 的既有结果优先，本条只把"原本会默认放行"的外部工具提升为 Ask。
        if descriptor.handler_provenance.source_id != "builtin-tools"
            && descriptor.permission_declaration.requires_approval
        {
            return PermissionDecision {
                verdict: PermissionVerdict::ApprovalRequired,
                decision_source: "external_requires_approval".to_string(),
                reason: Some(format!(
                    "外部工具 `{}`（来源 `{}`）声明需要审批，调用前需要宿主授权。",
                    descriptor.identity.model_name, descriptor.handler_provenance.source_id
                )),
            };
        }

        // 5. 默认放行
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
        descriptor: &crate::tools::ToolDescriptor,
        _origin: &InvocationOrigin,
        final_arguments: &Value,
    ) -> PermissionDecision {
        // 上下文无关的旧 seam（既有单元/集成测试直接调用）：无注入会话事实，
        // 草稿门禁按 fail-closed 规则处理（C-138-4）。旧 seam 无宿主审批（None）。
        self.decide_with_context(descriptor, None, None, None, final_arguments)
    }

    fn evaluate_with_context(
        &self,
        descriptor: &crate::tools::ToolDescriptor,
        _origin: &InvocationOrigin,
        final_arguments: &Value,
        context: &DispatchContext,
    ) -> PermissionDecision {
        // 0. 检查前端传入的会话级授权模式 (tool_authorization_mode)
        if let Some(auth_mode) = context.tool_authorization_mode.as_deref() {
            // T7（执行闸门半）：判定走**语义分类** `ToolKind`。
            // ADR 0065 decision 5（PA-076 T1）：权限判定只读 descriptor 声明的权限事实
            // （`ToolPermissionFacts` = `kind` + typed `ToolPermissionDeclaration.scopes`），
            // **名称不参与权限判定**。此前的 `primitive_name ∈ {write,edit,bash,execute}` /
            // model_name 小写嗅探是真实漏洞：外部写工具（MCP / 插件）名字不在字面量表内，
            // 只读模式下会被直接放行（fail-open）；反过来，字面量判据也无法覆盖将来新增的
            // 写类 primitive。声明 workspace.write / workspace.execute 即视为写/执行类，
            // 与工具叫什么名字无关。
            let declares_write_or_exec = descriptor
                .permission_declaration
                .scopes
                .iter()
                .any(|scope| {
                    matches!(
                        scope,
                        crate::tools::ToolPermissionScope::WorkspaceWrite
                            | crate::tools::ToolPermissionScope::WorkspaceExecute
                    )
                });
            let is_write_or_exec = declares_write_or_exec
                || matches!(
                    descriptor.kind,
                    crate::tools::ToolKind::Write
                        | crate::tools::ToolKind::Execute
                );

            if auth_mode == "read_only" && is_write_or_exec {
                return PermissionDecision {
                    verdict: PermissionVerdict::Deny,
                    decision_source: "tool_authorization_mode_read_only".to_string(),
                    reason: Some("当前会话处于只读模式（工作区查看），禁止执行写入与执行类操作。".to_string()),
                };
            }
        }

        // PA-141：审批证据**仅**来自宿主注入的 `DispatchContext`；
        // `final_arguments`（模型可控）不参与审批判定（C-141-3）。
        self.decide_with_context(
            descriptor,
            context.session_id.as_deref(),
            context.plan_execution_approval.as_ref(),
            context.workspace_root.as_deref(),
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
        // wave-1 C5：会话 id 同样由 dispatch context 注入——TodoWrite/TodoList/create_goal/
        // get_goal/update_goal 在模型未显式给 session 参数时回退到当前会话。
        let execution_context = ToolExecutionContext {
            workspace_root: request
                .workspace_root
                .as_deref()
                .map(std::path::PathBuf::from),
            session_id: request.session_id.clone(),
            // wave-62 A1（C5）：**必须**透传取消令牌——此前 `Default::default()` 在这里把
            // turn 令牌丢掉，真实 app 的 `Run` 因此永不观测 stop（双路 L3 复现的 BLOCKING）。
            cancellation: request.cancellation.clone(),
            // W2：桥接层重建上下文时必须恢复宿主 provider 选择（线程局部作用域由
            // `GovernedToolExecutor::execute_with_context` 建立）。
            provider_selection: crate::dispatcher_composites::current_provider_selection(),
            ..Default::default()
        };
        let result = self.router.execute_with_context(
            &ToolCall {
                call_id: None,
                name: self.primitive.clone(),
                arguments: request.arguments.clone(),
                plan: None,
            },
            &execution_context,
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
    authorize_store: Option<Arc<crate::path_permission::AuthorizeStore>>,
) -> GovernedToolExecutor {
    build_governed_executor_full(workspace_root, authorize_store, None)
}

/// 生产沙箱装配的显式 override 入口（测试注入用）：`Some(backend)` 时直接注册该后端；
/// `None` 时走默认生产路径（Native 优先、fallback HostApprovedUnsandboxed）。
pub fn build_governed_executor_with_sandbox_backend(
    workspace_root: Option<PathBuf>,
    authorize_store: Option<Arc<crate::path_permission::AuthorizeStore>>,
    sandbox_override: Option<Arc<dyn SandboxBackend>>,
) -> GovernedToolExecutor {
    build_governed_executor_full(workspace_root, authorize_store, sandbox_override)
}

pub fn build_governed_executor_full(
    workspace_root: Option<PathBuf>,
    authorize_store: Option<Arc<crate::path_permission::AuthorizeStore>>,
    sandbox_override: Option<Arc<dyn SandboxBackend>>,
) -> GovernedToolExecutor {
    let registry = Arc::new(
        ToolRegistrySnapshot::builtin().expect("builtin registry must validate"),
    );
    let clock: Arc<dyn RuntimeClock> = Arc::new(SystemClock);
    let dispatcher = GovernedDispatcher::new(registry.clone(), clock);
    

    // 默认工作区分叉收敛：None 兜底取 compute_default_workspace_root()
    //（Windows Documents/pony_agent，Unix ~/pony_agent），dirs 解析失败才回退 cwd。
    let workspace = workspace_root.unwrap_or_else(|| {
        crate::workspace::compute_default_workspace_root()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
    });
    // PA-080：授权清单（如有）与 host control plane 共享同一 `Arc`；
    // 缺省（测试/legacy 构造）时工具判定使用空授权清单（workspace 外读一律 requires_authorization）。
    let shared_authorizations = authorize_store
        .unwrap_or_else(|| Arc::new(crate::path_permission::AuthorizeStore::new()));

    // W30b：加载并注册基于规则与真实共享 Plan 状态以及路径授权状态的策略评估器
    let settings = crate::app_settings::AppSettingsStore::new().load_view();
    dispatcher.register_policy_evaluator(Arc::new(
        GovernedPolicyEvaluator::new(settings.tool_rules)
            .with_authorize_store(Arc::clone(&shared_authorizations)),
    ));
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
    dispatcher.set_budget_config(DispatchBudgetConfig {
        unbounded_output_and_deadline: true,
        ..Default::default()
    });
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

    // ── Plan 模式退出审查（primitive `plan_control`，模型可见名 `ExitPlanMode`）：
    // 只登记 Markdown 方案并请求宿主审核，不持有步骤机（ADR 2026-11-27 方案 1A）。 ──
    let plan_handler = Arc::new(ExitPlanModeHandler::new());

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
    use crate::tool_runtime::PendingControlRequestKind;
    use crate::tools::ToolExecutor;
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

    /// T2-1 迁移辅助：替代已删除的 `GovernedToolExecutor::set_context`。
    ///
    /// turn 级事实现在随每次调用经 `ToolExecutionContext.dispatch_context` 显式传入，
    /// executor 不再持有共享槽位。本包装只把「事实 + executor」绑在一起，让既有断言语义
    /// 与 `executor.execute(&call)` 的调用形状保持不变。
    struct BoundGoverned<'a> {
        executor: &'a GovernedToolExecutor,
        context: crate::dispatcher::DispatchContext,
    }

    impl<'a> BoundGoverned<'a> {
        fn new(executor: &'a GovernedToolExecutor, context: crate::dispatcher::DispatchContext) -> Self {
            Self { executor, context }
        }

        fn execute(&self, call: &ToolCall) -> crate::tools::ToolResult {
            self.executor.execute_with_context(
                call,
                &ToolExecutionContext {
                    dispatch_context: Some(self.context.clone()),
                    ..Default::default()
                },
            )
        }

        fn dispatcher(&self) -> &GovernedDispatcher {
            self.executor.dispatcher()
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
        let turn_context = crate::dispatcher::DispatchContext {
            session_id: Some("session-ask".to_string()),
            run_id: Some("run-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            ..Default::default()
        }; let governed = BoundGoverned::new(&governed, turn_context);
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

    #[test]
    fn governed_executor_read_outside_workspace_requires_approval_and_persists_request() {
        let workspace = temp_workspace();
        let governed = build_governed_executor(Some(workspace.clone()), None);
        let turn_context = crate::dispatcher::DispatchContext {
            session_id: Some("session-approval".to_string()),
            run_id: Some("run-appr".to_string()),
            turn_id: Some("turn-appr".to_string()),
            workspace_root: Some(workspace.display().to_string()),
            ..Default::default()
        }; let governed = BoundGoverned::new(&governed, turn_context);

        // 创建一个真实存在的外部临时文件
        let ext_dir = temp_workspace();
        let ext_file = ext_dir.join("external_test_file.txt");
        std::fs::write(&ext_file, "external content").expect("write external");

        let read_call = same_call(
            "Read",
            json!({ "path": ext_file.display().to_string(), "description": "test" }),
        );
        let result = governed.execute(&read_call);
        assert_eq!(result.status, "error");
        assert!(
            result.output.contains("control_outcome_pending"),
            "expected control_outcome_pending for unauthorized external read, got: {}",
            result.output
        );

        let pending = governed.dispatcher().pending_requests();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].request_kind, PendingControlRequestKind::Approval);
        assert_eq!(pending[0].session_id.as_deref(), Some("session-approval"));
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
                "expectedHash": crate::tools::compute_content_sha256("hello\n"),
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
        use crate::sandbox::NoSandboxBackend;
        use crate::tool_runtime::SandboxBackend;
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
            fn execute(&self, _call: &ToolCall) -> crate::tools::ToolResult {
                crate::tools::ToolResult {
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

    // ── ADR 0065 decision 5：只读闸门读 descriptor 权限事实，不嗅探工具名 ────────────────

    fn gate_descriptor(
        model_name: &str,
        kind: crate::tools::ToolKind,
        scopes: &[crate::tools::ToolPermissionScope],
    ) -> crate::tools::ToolDescriptor {
        crate::tools::ToolDescriptor {
            identity: crate::tools::ToolIdentity {
                descriptor_id: format!("test:{model_name}"),
                model_name: model_name.to_string(),
                canonical_name: model_name.to_string(),
                primitive_name: model_name.to_string(),
                source: crate::tools::ToolDescriptorSource::Dynamic,
            },
            aliases: vec![model_name.to_string()],
            description: String::new(),
            input_schema: json!({ "type": "object" }),
            kind,
            exposure: crate::tools::ToolExposure::ModelVisible,
            permission_declaration: crate::tools::ToolPermissionDeclaration {
                scopes: scopes.iter().cloned().collect(),
                requires_approval: false,
                host_mediated: false,
                approval_mode: None,
            },
            execution_policy: crate::tools::ToolExecutionPolicy::default(),
            display_metadata: crate::tools::ToolDisplayMetadata::default(),
            handler_provenance: crate::tools::ToolHandlerProvenance {
                handler_kind: "test".to_string(),
                source_id: "external-test".to_string(),
            },
            source_revision: "test-v1".to_string(),
            composed_descriptor_ids: Vec::new(),
        }
    }

    /// 只读模式的真实漏洞回归：外部写工具的名字不含 write/edit/bash 字面量，
    /// 旧嗅探逻辑会直接放行（fail-open）。权限事实（声明的 workspace.write scope）才是指标。
    #[test]
    fn read_only_gate_denies_by_declared_permission_facts_not_by_tool_name() {
        use crate::tools::ToolKind;
        use crate::tools::ToolPermissionScope;

        let evaluator = GovernedPolicyEvaluator::default();
        let read_only_context = crate::dispatcher::DispatchContext {
            tool_authorization_mode: Some("read_only".to_string()),
            ..Default::default()
        };

        let external_writer = gate_descriptor(
            "mcp__demo__purge",
            ToolKind::External,
            &[ToolPermissionScope::WorkspaceWrite],
        );
        let denied = evaluator.evaluate_with_context(
            &external_writer,
            &InvocationOrigin::Model,
            &json!({}),
            &read_only_context,
        );
        assert_eq!(denied.verdict, PermissionVerdict::Deny);
        assert_eq!(
            denied.decision_source,
            "tool_authorization_mode_read_only",
            "an external write tool must be denied in read-only mode even though its name \
             contains no write/edit/bash literal"
        );

        let external_executor = gate_descriptor(
            "mcp__demo__run",
            ToolKind::External,
            &[ToolPermissionScope::WorkspaceExecute],
        );
        assert_eq!(
            evaluator
                .evaluate_with_context(
                    &external_executor,
                    &InvocationOrigin::Model,
                    &json!({}),
                    &read_only_context
                )
                .verdict,
            PermissionVerdict::Deny
        );

        // A write-kind descriptor is denied even when it declares no scope at all.
        let unnamed_writer = gate_descriptor("custom_mutator", ToolKind::Write, &[]);
        assert_eq!(
            evaluator
                .evaluate_with_context(
                    &unnamed_writer,
                    &InvocationOrigin::Model,
                    &json!({}),
                    &read_only_context
                )
                .verdict,
            PermissionVerdict::Deny
        );

        // Read-only facts stay allowed (the gate must not become a blanket deny).
        let reader = gate_descriptor(
            "mcp__demo__peek",
            ToolKind::Read,
            &[ToolPermissionScope::WorkspaceRead],
        );
        let allowed = evaluator.evaluate_with_context(
            &reader,
            &InvocationOrigin::Model,
            &json!({}),
            &read_only_context,
        );
        assert_eq!(allowed.verdict, PermissionVerdict::Allow);
    }
}
