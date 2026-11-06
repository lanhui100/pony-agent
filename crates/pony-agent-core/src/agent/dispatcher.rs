//! Governed tool dispatcher (PA-076 phase 3, tasks 3.1-3.3).
//!
//! `GovernedDispatcher` is the single governed execution entry. Every top-level and composite
//! child invocation walks the same fixed pipeline, in order:
//!
//!   1. origin authorization + exposure (model origins are bound to the current `TurnToolView`;
//!      internal descriptors are reachable only through a bounded `ChildDispatch`);
//!   2. raw schema validation of the incoming arguments;
//!   3. mutable pre-dispatch hooks (may rewrite arguments, never identity/origin);
//!   4. final validation/normalization plus the permission/sandbox decision
//!      (`allow` / `deny` / `approval_required` / `waiting_host`) with a recorded
//!      `decision_source`;
//!   5. atomic budget reservation (calls / concurrency / bytes / deadline / cancellation);
//!   6. handler execution (primitive handler, or composite handler with a restricted
//!      `ChildDispatch`);
//!   7. control outcome: `approval_required` / `waiting_host` persist a `PendingControlRequest`
//!      and return `ToolOutcome::pending(...)`; execution status and control outcome stay
//!      orthogonal;
//!   8. exactly-once lifecycle record per invocation (top-level and each child), decoupled from
//!      Tauri through the `DispatchLifecycleObserver` port.
//!
//! Everything fails closed: unknown descriptors, unauthorized origins, denied permissions,
//! exhausted budgets, and missing handlers produce structured error outcomes rather than panics.

use crate::agent::ask_persistence::{AskPersistenceStore, PersistedAsk};
use crate::agent::budget::{BudgetLedger, CancellationToken, DispatchBudgetConfig};
use crate::agent::child_dispatch::{
    check_budget_gates, ChildDispatch, ChildDispatchContext, ChildDispatchRequest,
    ChildDispatchRunner, PreflightedChild,
};
use crate::agent::tool_runtime::{
    InvocationOrigin, PendingControlRequest, PendingControlRequestKind, PendingControlRequestState,
    PrimitiveToolHandler, PrimitiveToolHandlerRequest, RuntimeClock, SandboxAvailability,
    SandboxBackend, SandboxRequest, ToolDispatcher, ToolDispatchRequest, TurnToolView,
};
use crate::agent::tools::{
    ToolControlKind, ToolControlOutcome, ToolDescriptor, ToolExecutionStatus, ToolExposure,
    ToolKind, ToolOutcome, ToolPermissionScope, ToolRegistrySnapshot, ToolResult,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Invocation context
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Session/run/turn/workspace facts bound to one governed invocation. Facts are optional so a
/// minimal `ToolDispatchRequest` (origin/descriptor/call/arguments) can still be dispatched.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DispatchContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    /// Whether an interactive host is currently available to answer control requests. When
    /// false, `approval_required` / `waiting_host` decisions fail closed with
    /// `interaction_unavailable` instead of persisting a pending request.
    #[serde(default = "default_host_control_available")]
    pub host_control_available: bool,
    /// W36 / PA-141（方案 b）：宿主注入的 Draft→Executing 审批证据。仅由宿主经
    /// `DispatchContext`（或 `TurnInput` 透传）携带；模型工具参数**不是**审批来源（C-141-3）。
    /// 绑定 `(session_id, plan_id)` 并带 TTL（`now_ms < expires_at_ms` 才有效，C-141-4/5）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_execution_approval: Option<PlanExecutionApproval>,
    /// 前端传入的授权模式：read_only | workspace_write | full_access
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_authorization_mode: Option<String>,
    /// wave-62 C2：本 turn 的取消令牌（运行时**句柄**，不进线缆 ⇒ `#[serde(skip)]`）。
    ///
    /// `Some` ⇒ dispatcher 闸门必须观测它（turn 被 stop ⇒ 本次调用 fail-closed 为
    /// `Cancelled`，沿用既有错误词表）；`None` ⇒ 行为与此前完全一致（向后兼容）。
    #[serde(skip)]
    pub turn_cancellation: Option<Arc<crate::agent::budget::CancellationToken>>,
}

/// 宿主审批证据（PA-141，C-141-1 serde 形状冻结：camelCase）。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanExecutionApproval {
    pub session_id: String,
    pub plan_id: String,
    pub expires_at_ms: u64,
}

fn default_host_control_available() -> bool {
    true
}

impl Default for DispatchContext {
    fn default() -> Self {
        Self {
            session_id: None,
            run_id: None,
            turn_id: None,
            workspace_root: None,
            host_control_available: true,
            plan_execution_approval: None,
            tool_authorization_mode: None,
            turn_cancellation: None,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Permission decisions
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionVerdict {
    Allow,
    Deny,
    ApprovalRequired,
    WaitingHost,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionDecision {
    pub verdict: PermissionVerdict,
    /// Provenance of the decision (for example `descriptor_declaration` or the name of a
    /// registered `ToolPolicyEvaluator`).
    pub decision_source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Per-invocation policy evaluator. The default implementation derives the decision from the
/// descriptor's typed `ToolPermissionDeclaration`; the runtime may register its own evaluator
/// (for example one that consults a host capability registry).
pub trait ToolPolicyEvaluator: Send + Sync {
    fn evaluate(
        &self,
        descriptor: &ToolDescriptor,
        origin: &InvocationOrigin,
        final_arguments: &Value,
    ) -> PermissionDecision;

    /// Context-aware evaluation (W36 / PA-138 public seam): the dispatcher calls this entry so a
    /// policy can key decisions on runtime-injected facts (`DispatchContext::session_id` and
    /// friends) instead of model-controlled tool arguments. The default delegates to the
    /// context-free [`Self::evaluate`], keeping every existing evaluator backward compatible.
    fn evaluate_with_context(
        &self,
        descriptor: &ToolDescriptor,
        origin: &InvocationOrigin,
        final_arguments: &Value,
        _context: &DispatchContext,
    ) -> PermissionDecision {
        self.evaluate(descriptor, origin, final_arguments)
    }
}

/// Default policy evaluator: `host_mediated` -> `waiting_host`, else `requires_approval` ->
/// `approval_required`, else — conservatively — a descriptor that declares a write/execute scope
/// is still `approval_required` unless a host registers a more specific evaluator. Only
/// read/discovery scopes default to `allow`. Unknown permission scopes cannot appear in the typed
/// `BTreeSet<ToolPermissionScope>`, so every scope is recognized by construction.
#[derive(Clone, Debug, Default)]
pub struct DescriptorPolicyEvaluator;

impl ToolPolicyEvaluator for DescriptorPolicyEvaluator {
    fn evaluate(
        &self,
        descriptor: &ToolDescriptor,
        _origin: &InvocationOrigin,
        _final_arguments: &Value,
    ) -> PermissionDecision {
        let declaration = &descriptor.permission_declaration;
        let verdict = if declaration.host_mediated {
            PermissionVerdict::WaitingHost
        } else if declaration.requires_approval {
            PermissionVerdict::ApprovalRequired
        } else if declaration.scopes.iter().any(|scope| {
            matches!(
                scope,
                ToolPermissionScope::WorkspaceWrite | ToolPermissionScope::WorkspaceExecute
            )
        }) {
            // Fail-closed default (phase-3 security review P1-2): a builtin that declares a
            // write/execute scope must not silently run with verdict Allow just because the
            // declaration's `requires_approval` flag is unset. The runtime-switch milestone
            // registers a real policy evaluator with accurate approval UX; until then this
            // default lifts such invocations to approval-required.
            PermissionVerdict::ApprovalRequired
        } else {
            PermissionVerdict::Allow
        };
        let conservative_lift = verdict == PermissionVerdict::ApprovalRequired
            && !declaration.requires_approval
            && !declaration.host_mediated;
        PermissionDecision {
            verdict,
            decision_source: "descriptor_declaration".to_string(),
            reason: conservative_lift.then(|| {
                "write/execute scope requires approval under the conservative default evaluator"
                    .to_string()
            }),
        }
    }
}

/// Conservative composite permission aggregation (design.md Decision 4). A composite's effective
/// permission surface is the union of its children's declared scopes plus the strongest
/// approval/host requirement among them. Every child's individual decision is preserved as
/// evidence so a parent summary never replaces per-child authorization at execution time: each
/// child is still authorized against its own final arguments by the dispatcher.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompositePermissionSummary {
    /// Union of every child scope plus any scope the composite declares itself.
    #[serde(default)]
    pub scopes: BTreeSet<ToolPermissionScope>,
    /// `true` when any child requires approval (or the composite declares it).
    pub requires_approval: bool,
    /// `true` when any child is host-mediated (or the composite declares it).
    pub host_mediated: bool,
    /// Per-child decision evidence, preserved in dispatch order.
    #[serde(default)]
    pub child_decisions: Vec<PermissionDecision>,
}

/// Aggregate child permission declarations/decisions conservatively. Scopes are unioned; if any
/// child's decision is `ApprovalRequired` the aggregate requires approval, and if any child is
/// `WaitingHost` the aggregate is host-mediated. Denied/allow children do not weaken the
/// aggregate. Child decisions are always preserved as evidence.
pub fn aggregate_composite_permission(
    declared_scopes: impl IntoIterator<Item = ToolPermissionScope>,
    decisions: &[PermissionDecision],
) -> CompositePermissionSummary {
    let mut summary = CompositePermissionSummary {
        scopes: declared_scopes.into_iter().collect(),
        requires_approval: false,
        host_mediated: false,
        child_decisions: decisions.to_vec(),
    };
    for decision in decisions {
        match decision.verdict {
            PermissionVerdict::ApprovalRequired => summary.requires_approval = true,
            PermissionVerdict::WaitingHost => summary.host_mediated = true,
            PermissionVerdict::Deny | PermissionVerdict::Allow => {}
        }
    }
    summary
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Mutable pre-dispatch hooks
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// A mutable pre-dispatch hook. It may rewrite the invocation arguments, but the signature makes
/// it impossible to change the descriptor identity or the invocation origin.
pub trait PreDispatchHook: Send + Sync {
    fn rewrite(
        &self,
        descriptor_id: &str,
        origin: &InvocationOrigin,
        call_id: &str,
        arguments: Value,
    ) -> Result<Value, String>;
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Composite handlers
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompositeToolHandlerRequest {
    pub descriptor_id: String,
    pub call_id: String,
    pub arguments: Value,
}

/// A composite handler implements domain behavior by orchestrating children through the bounded
/// `ChildDispatch`. It cannot reach the full dispatcher, the `ToolRouter`, or other handlers'
/// private methods.
pub trait CompositeToolHandler: Send + Sync {
    fn execute(
        &self,
        request: &CompositeToolHandlerRequest,
        children: &ChildDispatch,
    ) -> Result<Value, String>;
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Lifecycle observability
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchLifecyclePhase {
    Completed,
    Failed,
    Cancelled,
    ControlPending,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DispatchLifecycleRecord {
    pub descriptor_id: String,
    pub call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_call_id: Option<String>,
    pub origin: InvocationOrigin,
    pub depth: u32,
    pub phase: DispatchLifecyclePhase,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<PermissionVerdict>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_status: Option<ToolExecutionStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_outcome: Option<ToolControlOutcome>,
    #[serde(default)]
    pub final_args_digest: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_code: Option<String>,
    pub elapsed_ms: u64,
}

/// Exactly-once lifecycle observer port. The dispatcher emits exactly one record per top-level
/// invocation and one per child invocation; the runtime must not re-emit the same layer.
pub trait DispatchLifecycleObserver: Send + Sync {
    fn record(&self, event: &DispatchLifecycleRecord);
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Control request authorization
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Facts a host/runtime must present to answer, approve, or cancel a `PendingControlRequest`.
/// All consumption paths use compare-and-swap semantics on session/version/nonce/expiry and
/// reject replay, expiry, cross-session, descriptor, and source-version mismatches.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlRequestAuthorization {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub expected_version: u64,
    pub nonce: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_descriptor_snapshot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_descriptor_id: Option<String>,
    /// Optional final-arguments digest (design.md Decision 5). When present it must match the
    /// digest the pending request was bound to, so a resumed call can never diverge from what
    /// was authorized. Optional so existing approval flows stay backward compatible.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_final_args_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_policy_digest: Option<String>,
    /// Optional answer payload, used only by `answer_control_request`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlRequestConsumed {
    pub request: PendingControlRequest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<Value>,
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Dispatch errors
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Structured fail-closed dispatch error. `cancelled == true` maps to a `Cancelled` execution
/// status; everything else maps to `Error` with a structured `error.code`. The code is a `String`
/// so arbitrary structured codes from bridged legacy handlers (e.g. `not_found`,
/// `path_outside_workspace`) survive instead of collapsing to `handler_error`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DispatchError {
    pub code: String,
    pub message: String,
    pub cancelled: bool,
}

impl DispatchError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            cancelled: false,
        }
    }

    pub fn cancelled(message: impl Into<String>) -> Self {
        Self {
            code: "cancelled".to_string(),
            message: message.into(),
            cancelled: true,
        }
    }

    pub fn into_outcome(self, tool_name: &str) -> ToolOutcome {
        let status = if self.cancelled { "aborted" } else { "error" };
        ToolOutcome::from_legacy_result(ToolResult {
            tool_name: tool_name.to_string(),
            status: status.to_string(),
            output: json!({
                "ok": false,
                "error": {
                    // Stage1 双写（A1）：kind 与 code 并存且值一致，message 语义不变。
                    "code": self.code.clone(),
                    "kind": self.code,
                    "message": self.message,
                }
            })
            .to_string(),
            duration_ms: 0,
        })
    }
}

/// Build a fail-closed `DispatchError` from a handler `Err`. Composite and bridged handlers signal
/// structured errors as `code: message`; the code is preserved verbatim so consumers can branch on
/// `error.code` instead of everything collapsing to `handler_error` (phase-3 review P2-2). Only
/// well-formed `code: message` prefixes (alphanumeric/dash/underscore code) are lifted; everything
/// else falls back to the generic `handler_error`.
pub(crate) fn handler_failure(message: String) -> DispatchError {
    if let Some((code, rest)) = message.split_once(':') {
        let code = code.trim();
        let rest = rest.trim();
        let well_formed = !code.is_empty()
            && !rest.is_empty()
            && code
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
        if well_formed {
            // wave-62 C5：桥接层把 `Run` 的取消结果转成 `tool_cancelled: …` —— 它属于**取消**
            // 语义（legacy status = `aborted`），但**错误码必须原样保留** `tool_cancelled`
            // （不得被改写成 `cancelled`，也不得新增码）。
            if code == "tool_cancelled" {
                return DispatchError {
                    code: code.to_string(),
                    message: rest.to_string(),
                    cancelled: true,
                };
            }
            return DispatchError::new(code.to_string(), rest.to_string());
        }
    }
    DispatchError::new("handler_error", message)
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// GovernedDispatcher
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Concrete governed dispatcher. Owns the registry truth source, an injected clock, registered
/// handlers/hooks/observers, the current turn tool view, and the in-memory control-request store.
#[derive(Clone)]
pub struct GovernedDispatcher {
    inner: Arc<Inner>,
}

/// One atomically published **tool surface generation**: the registry snapshot, the primitive
/// handler table bound to it, and the turn tool view derived from that snapshot.
///
/// ADR 0065 decision 3: a generation is immutable once published and replaced wholesale, so no
/// observer can ever see "descriptor visible but its handler is not ready" (or the reverse:
/// a handler reachable whose descriptor vanished).
#[derive(Clone)]
pub(crate) struct ToolSurfaceGeneration {
    registry: Arc<ToolRegistrySnapshot>,
    handlers: BTreeMap<String, Arc<dyn PrimitiveToolHandler>>,
    turn_view: TurnToolView,
}

pub(crate) struct Inner {
    surface: RwLock<Arc<ToolSurfaceGeneration>>,
    clock: Arc<dyn RuntimeClock>,
    composite_handlers: Mutex<BTreeMap<String, Arc<dyn CompositeToolHandler>>>,
    composite_authority: Mutex<BTreeMap<String, BTreeSet<String>>>,
    pre_dispatch_hooks: Mutex<Vec<Arc<dyn PreDispatchHook>>>,
    policy_evaluator: RwLock<Arc<dyn ToolPolicyEvaluator>>,
    sandbox_backend: RwLock<Option<Arc<dyn SandboxBackend>>>,
    lifecycle_observers: Mutex<Vec<Arc<dyn DispatchLifecycleObserver>>>,
    pending_requests: Mutex<BTreeMap<String, PendingControlRequest>>,
    /// Durable Ask (Interaction) store (ADR 0074). `None` keeps the dispatcher memory-only
    /// (approval-only dispatchers and unit tests); the host installs the session-backed store
    /// through [`GovernedDispatcher::set_ask_store`] so pending Asks survive a restart.
    ask_store: RwLock<Option<Arc<dyn AskPersistenceStore>>>,
    default_context: RwLock<DispatchContext>,
    config: Mutex<DispatchBudgetConfig>,
    nonce_seq: AtomicU64,
}

impl Inner {
    /// The currently published tool surface generation (cheap `Arc` clone).
    fn surface(&self) -> Arc<ToolSurfaceGeneration> {
        Arc::clone(&self.surface.read().expect("dispatcher tool surface poisoned"))
    }

    /// Copy-on-write publish of the current generation. Every mutation of the surface (handler
    /// registration, turn-view replacement, atomic swap) goes through this one door.
    fn mutate_surface(&self, mutate: impl FnOnce(&mut ToolSurfaceGeneration)) {
        let mut guard = self
            .surface
            .write()
            .expect("dispatcher tool surface poisoned");
        let mut next = ToolSurfaceGeneration::clone(guard.as_ref());
        mutate(&mut next);
        *guard = Arc::new(next);
    }

    /// Publish a whole generation atomically.
    fn publish_surface(&self, next: ToolSurfaceGeneration) {
        *self
            .surface
            .write()
            .expect("dispatcher tool surface poisoned") = Arc::new(next);
    }
}

impl GovernedDispatcher {
    pub fn new(registry: Arc<ToolRegistrySnapshot>, clock: Arc<dyn RuntimeClock>) -> Self {
        let turn_view = registry.default_turn_tool_view();
        let inner = Arc::new(Inner {
            surface: RwLock::new(Arc::new(ToolSurfaceGeneration {
                registry: Arc::clone(&registry),
                handlers: BTreeMap::new(),
                turn_view,
            })),
            clock,
            composite_handlers: Mutex::new(BTreeMap::new()),
            composite_authority: Mutex::new(BTreeMap::new()),
            pre_dispatch_hooks: Mutex::new(Vec::new()),
            policy_evaluator: RwLock::new(Arc::new(DescriptorPolicyEvaluator)),
            sandbox_backend: RwLock::new(None),
            lifecycle_observers: Mutex::new(Vec::new()),
            pending_requests: Mutex::new(BTreeMap::new()),
            ask_store: RwLock::new(None),
            default_context: RwLock::new(DispatchContext::default()),
            config: Mutex::new(DispatchBudgetConfig::default()),
            nonce_seq: AtomicU64::new(1),
        });
        Self { inner }
    }

    /// Register a primitive handler under its canonical descriptor id (`builtin:<primitive>`).
    /// Unknown descriptor ids fail closed at dispatch.
    pub fn register_handler(
        &self,
        descriptor_id: impl Into<String>,
        handler: Arc<dyn PrimitiveToolHandler>,
    ) {
        let descriptor_id = descriptor_id.into();
        self.inner
            .mutate_surface(|surface| {
                surface.handlers.insert(descriptor_id, handler);
            });
    }

    /// ADR 0065 decision 3 — the atomic tool-surface swap seam (T2's prerequisite).
    ///
    /// Publishes **snapshot + primitive handler table + derived turn view as one unit**: either
    /// the whole generation becomes visible or none of it does, so an observer can never see a
    /// descriptor whose handler is not yet registered (that intermediate state is exactly what a
    /// two-step "set registry, then register handlers" API would expose to a concurrent dispatch).
    ///
    /// Contract (ADR 0065 decision 3, frozen as the outward-facing seam T2 builds on):
    /// - **整面替换 (wholesale replacement)**: `handlers` replaces the primitive handler table —
    ///   it is not a merge. The published generation swaps registry snapshot + handler table +
    ///   derived turn view together, so no entry of the previous surface (descriptor *or*
    ///   execution body) survives the swap.
    /// - `snapshot_id` must match `registry.snapshot_id` (both are the same fact; the argument is
    ///   the caller's assertion, re-checked against the snapshot it hands over).
    /// - **整面自洽校验 (whole-surface self-consistency)**: the pair is validated before anything
    ///   is published. Rejected with `Err` + the previous surface left fully intact when:
    ///   (a) a descriptor introduced by this swap has no handler — that would create exactly the
    ///   visible-but-unexecutable entry the atomic unit must make unobservable; (b) a descriptor
    ///   that already had a handler loses it — a swap may not silently amputate an execution body;
    ///   (c) a handler names a descriptor the new snapshot does not own — a stale/ghost binding;
    ///   (d) duplicate handlers for one descriptor. Descriptors that were already published
    ///   without a handler (builtin handlers registered separately after construction) are not
    ///   retroactively required to have one; dispatch keeps failing closed with
    ///   `no_handler_registered` for them.
    /// - Composite handlers/authority are not part of the swap (they are builtin-only, registered
    ///   at assembly time); external descriptors must not declare composites.
    pub fn swap_tool_surface(
        &self,
        snapshot_id: &str,
        registry: ToolRegistrySnapshot,
        handlers: Vec<(String, Arc<dyn PrimitiveToolHandler>)>,
    ) -> Result<(), String> {
        let snapshot_id = snapshot_id.trim();
        if snapshot_id.is_empty() {
            return Err("tool surface snapshot id cannot be empty".to_string());
        }
        if snapshot_id != registry.snapshot_id {
            return Err(format!(
                "tool surface snapshot id `{snapshot_id}` does not match registry snapshot `{}`",
                registry.snapshot_id
            ));
        }

        let mut handler_table: BTreeMap<String, Arc<dyn PrimitiveToolHandler>> = BTreeMap::new();
        for (descriptor_id, handler) in handlers {
            let descriptor_id = descriptor_id.trim().to_string();
            if descriptor_id.is_empty() {
                return Err("tool surface handler descriptor id cannot be empty".to_string());
            }
            if handler_table.insert(descriptor_id.clone(), handler).is_some() {
                return Err(format!(
                    "tool surface contains a duplicate handler for descriptor `{descriptor_id}`"
                ));
            }
        }

        // Atomicity precondition (ADR 0065 decision 3): the generation is only published when
        // descriptors and handlers agree for every entry this swap actually *introduces* —
        //   (a) a descriptor that is new relative to the current surface must arrive with its
        //       handler (otherwise the swap itself would create a visible-but-unexecutable
        //       entry: the exact torn state the atomic unit must make unobservable), and
        //   (b) a descriptor that already had a handler must keep one (a swap may not silently
        //       amputate the execution body of a retained, already-executable descriptor).
        //
        // Descriptors that were already published *without* a handler (e.g. a dispatcher whose
        // builtin handlers are registered separately after construction) are not retroactively
        // required to have one: that configuration predates the swap, and dispatch keeps failing
        // closed with `no_handler_registered`.
        //
        // A handler whose descriptor the snapshot does not own is a stale binding and is rejected
        // too. Nothing is published on rejection, so the previous surface stays fully intact.
        let current = self.inner.surface();
        let missing_handlers = registry
            .descriptors
            .iter()
            .map(|descriptor| descriptor.identity.descriptor_id.as_str())
            .filter(|descriptor_id| {
                if handler_table.contains_key(*descriptor_id) {
                    return false;
                }
                let previously_known = current
                    .registry
                    .descriptors
                    .iter()
                    .any(|descriptor| descriptor.identity.descriptor_id == *descriptor_id);
                !previously_known || current.handlers.contains_key(*descriptor_id)
            })
            .collect::<Vec<_>>();
        if !missing_handlers.is_empty() {
            return Err(format!(
                "tool surface swap rejected: descriptors without a handler: {}",
                missing_handlers.join(", ")
            ));
        }
        let unknown_handlers = handler_table
            .keys()
            .filter(|descriptor_id| {
                registry
                    .descriptors
                    .iter()
                    .all(|descriptor| &descriptor.identity.descriptor_id != *descriptor_id)
            })
            .cloned()
            .collect::<Vec<_>>();
        if !unknown_handlers.is_empty() {
            return Err(format!(
                "tool surface swap rejected: handlers without a descriptor in snapshot `{snapshot_id}`: {}",
                unknown_handlers.join(", ")
            ));
        }

        let registry = Arc::new(registry);
        let turn_view = registry.default_turn_tool_view();
        self.inner.publish_surface(ToolSurfaceGeneration {
            registry,
            handlers: handler_table,
            turn_view,
        });
        Ok(())
    }

    /// ADR 0065 decision 6: adopt an external [`ToolProvider`] as a governed tool source.
    ///
    /// The provider's descriptors join the registry snapshot atomically together with their
    /// `ExternalToolHandler` bindings (one `ToolSurfaceGeneration` publish), so the external
    /// descriptors become model-visible only at the same instant they become executable. Every
    /// external invocation then walks the full governed pipeline — origin authorization, schema
    /// validation, policy decision, budget reservation, exactly-once lifecycle record — because
    /// it is dispatched by `descriptor_id` through the same handler table as builtin tools.
    ///
    /// Re-registering the same `source_id` replaces that source **wholesale**: its previous
    /// descriptors and their handler bindings are pruned in the same atomic publish, so a reload
    /// can never leave a stale `Arc<dyn ToolProvider>` (and, for MCP, a live child process) bound
    /// to a descriptor the new snapshot no longer owns. The pruned bindings are dropped as part of
    /// the publish; `McpToolProvider`/`McpClient` release their child process in `Drop`.
    pub fn register_tool_provider(
        &self,
        provider: Arc<dyn crate::agent::tools::ToolProvider>,
    ) -> Result<(), String> {
        use crate::agent::tool_runtime::ExternalToolHandler;
        use crate::agent::tools::ToolDescriptorSource;

        let source_id = provider.source_id().trim().to_string();
        if source_id.is_empty() {
            return Err("tool provider source id cannot be empty".to_string());
        }
        if source_id == "builtin-tools" {
            return Err("tool provider cannot claim the builtin-tools source".to_string());
        }

        let descriptors = provider.list_tools();
        for descriptor in &descriptors {
            let descriptor_id = &descriptor.identity.descriptor_id;
            if descriptor.identity.source == ToolDescriptorSource::Builtin {
                return Err(format!(
                    "tool provider `{source_id}` cannot register builtin descriptor `{descriptor_id}`"
                ));
            }
            if descriptor.handler_provenance.source_id != source_id {
                return Err(format!(
                    "tool provider `{source_id}` returned descriptor `{descriptor_id}` owned by source `{}`",
                    descriptor.handler_provenance.source_id
                ));
            }
        }

        let current = self.inner.surface();
        let base_snapshot_id = current
            .registry
            .snapshot_id
            .split("::")
            .next()
            .unwrap_or(current.registry.snapshot_id.as_str())
            .to_string();
        let next_snapshot_id = format!("{base_snapshot_id}::{source_id}");
        // Registry-side validation (identity, namespace, provenance, reserved aliases, duplicate
        // ids/aliases) happens here — an invalid external descriptor never becomes visible.
        let next_registry = current
            .registry
            .replace_source(&source_id, next_snapshot_id.clone(), descriptors.clone())?;

        // L3-B3：同一 `source_id` 重注册 = **该来源整代替换**，不是叠加。
        // 先剪除该来源的旧 descriptor binding（handler 表），再装入新 binding；被剪除的
        // `Arc<dyn PrimitiveToolHandler>` 在此处释放 ⇒ `Arc<dyn ToolProvider>` 引用计数归零 ⇒
        // provider/子进程随之 Drop（MCP 重载不再永久持有上一代 server 子进程）。
        let replaced_source_ids = current
            .registry
            .descriptors
            .iter()
            .filter(|descriptor| {
                descriptor.handler_provenance.source_id == source_id
                    && descriptor.identity.source != ToolDescriptorSource::Builtin
            })
            .map(|descriptor| descriptor.identity.descriptor_id.clone())
            .collect::<Vec<_>>();

        let mut handlers = current.handlers.clone();
        for descriptor_id in &replaced_source_ids {
            handlers.remove(descriptor_id);
        }
        for descriptor in &descriptors {
            handlers.insert(
                descriptor.identity.descriptor_id.clone(),
                Arc::new(ExternalToolHandler::new(Arc::clone(&provider))),
            );
        }

        let next_registry = Arc::new(next_registry);
        let turn_view = next_registry.default_turn_tool_view();
        self.inner.publish_surface(ToolSurfaceGeneration {
            registry: next_registry,
            handlers,
            turn_view,
        });
        Ok(())
    }

    /// Register a composite handler. Children are limited to the descriptor's declared
    /// `composed_descriptor_ids` (or any authority granted via
    /// `register_composite_handler_with_authority`).
    pub fn register_composite_handler(
        &self,
        descriptor_id: impl Into<String>,
        handler: Arc<dyn CompositeToolHandler>,
    ) {
        self.inner
            .composite_handlers
            .lock()
            .expect("dispatcher composite handler registry poisoned")
            .insert(descriptor_id.into(), handler);
    }

    /// Register a composite handler together with an explicit allowed-child authority set. The
    /// runtime authority is unioned with the descriptor's declared `composed_descriptor_ids`.
    /// This is the only way to author dynamic composites whose children are not statically
    /// declared in the registry.
    pub fn register_composite_handler_with_authority(
        &self,
        descriptor_id: impl Into<String>,
        allowed_child_ids: Vec<String>,
        handler: Arc<dyn CompositeToolHandler>,
    ) {
        let descriptor_id = descriptor_id.into();
        self.inner
            .composite_handlers
            .lock()
            .expect("dispatcher composite handler registry poisoned")
            .insert(descriptor_id.clone(), handler);
        let authority = allowed_child_ids.into_iter().collect::<BTreeSet<_>>();
        self.inner
            .composite_authority
            .lock()
            .expect("dispatcher composite authority poisoned")
            .insert(descriptor_id, authority);
    }

    /// Register a mutable pre-dispatch hook. Hooks run in registration order for every
    /// invocation and may only rewrite arguments.
    pub fn register_pre_dispatch_hook(&self, hook: Arc<dyn PreDispatchHook>) {
        self.inner
            .pre_dispatch_hooks
            .lock()
            .expect("dispatcher hooks poisoned")
            .push(hook);
    }

    /// Replace the policy evaluator (defaults to `DescriptorPolicyEvaluator`).
    pub fn register_policy_evaluator(&self, evaluator: Arc<dyn ToolPolicyEvaluator>) {
        *self
            .inner
            .policy_evaluator
            .write()
            .expect("dispatcher policy evaluator poisoned") = evaluator;
    }

    /// Register a sandbox backend. Execute-scope descriptors are validated against it when
    /// present; when absent no sandbox check runs (the runtime decides its own enforcement).
    pub fn register_sandbox_backend(&self, backend: Arc<dyn SandboxBackend>) {
        *self
            .inner
            .sandbox_backend
            .write()
            .expect("dispatcher sandbox backend poisoned") = Some(backend);
    }

    /// Register an exactly-once lifecycle observer.
    pub fn register_lifecycle_observer(&self, observer: Arc<dyn DispatchLifecycleObserver>) {
        self.inner
            .lifecycle_observers
            .lock()
            .expect("dispatcher lifecycle observers poisoned")
            .push(observer);
    }

    /// Replace the current turn tool view. Model origins may only reach descriptors visible in
    /// this view; a snapshot mismatch with the registry fails closed at dispatch.
    ///
    /// The view is part of the published tool-surface generation, so replacing it is a
    /// copy-on-write publish of that generation (never a torn update).
    pub fn set_turn_view(&self, view: TurnToolView) {
        self.inner.mutate_surface(|surface| surface.turn_view = view);
    }

    pub fn turn_view(&self) -> TurnToolView {
        self.inner.surface().turn_view.clone()
    }

    pub fn set_default_context(&self, context: DispatchContext) {
        *self
            .inner
            .default_context
            .write()
            .expect("dispatcher default context poisoned") = context;
    }

    pub fn default_context(&self) -> DispatchContext {
        self.inner
            .default_context
            .read()
            .expect("dispatcher default context poisoned")
            .clone()
    }

    pub fn set_budget_config(&self, config: DispatchBudgetConfig) {
        *self
            .inner
            .config
            .lock()
            .expect("dispatcher config poisoned") = config;
    }

    /// Full governed dispatch with explicit context facts.
    pub fn dispatch_governed(&self, request: ToolDispatchRequest, context: &DispatchContext) -> ToolOutcome {
        let started = Instant::now();
        let pipeline = self.inner.top_level_dispatch(&request, context);
        let record = build_lifecycle_record(
            request.descriptor_id.clone(),
            request.call_id.clone(),
            None,
            request.origin.clone(),
            0,
            &pipeline.outcome,
            pipeline.decision.as_ref(),
            pipeline.final_args_digest.clone(),
            started.elapsed().as_millis() as u64,
        );
        self.inner.emit(&record);
        pipeline.outcome
    }

    /// Current registry truth source (the snapshot of the published tool-surface generation).
    ///
    /// Returns the `Arc` rather than a borrow because the surface can be swapped atomically
    /// (`swap_tool_surface` / `register_tool_provider`): a borrowed reference would have to be
    /// held across a publish, which the generation model deliberately forbids.
    pub fn registry(&self) -> Arc<ToolRegistrySnapshot> {
        Arc::clone(&self.inner.surface().registry)
    }

    /// Injected clock (expiry / decision timestamps).
    pub fn clock(&self) -> &dyn RuntimeClock {
        self.inner.clock.as_ref()
    }

    // ── control request store ──────────────────────────────────────────────────────────────

    /// Snapshot of every pending control request.
    pub fn pending_requests(&self) -> Vec<PendingControlRequest> {
        self.inner
            .pending_requests
            .lock()
            .expect("dispatcher pending requests poisoned")
            .values()
            .cloned()
            .collect()
    }

    pub fn pending_request(&self, request_id: &str) -> Option<PendingControlRequest> {
        self.inner
            .pending_requests
            .lock()
            .expect("dispatcher pending requests poisoned")
            .get(request_id)
            .cloned()
    }

    /// Consume a `PendingControlRequest` as a host approval. CAS semantics reject replay,
    /// expiry, cross-session, version/nonce, descriptor, and source-version mismatches.
    pub fn approve_control_request(
        &self,
        request_id: &str,
        authorization: &ControlRequestAuthorization,
    ) -> Result<ControlRequestConsumed, String> {
        self.inner.consume_control_request(
            request_id,
            authorization,
            Some(PendingControlRequestKind::Approval),
            PendingControlRequestState::Consumed,
        )
    }

    /// Consume a `PendingControlRequest` as an interaction answer (the `Ask` control path).
    pub fn answer_control_request(
        &self,
        request_id: &str,
        authorization: &ControlRequestAuthorization,
    ) -> Result<ControlRequestConsumed, String> {
        self.inner.consume_control_request(
            request_id,
            authorization,
            Some(PendingControlRequestKind::Interaction),
            PendingControlRequestState::Consumed,
        )
    }

    /// Consume a `PendingControlRequest` as a cancellation (any request kind).
    pub fn cancel_control_request(
        &self,
        request_id: &str,
        authorization: &ControlRequestAuthorization,
    ) -> Result<ControlRequestConsumed, String> {
        self.inner.consume_control_request(
            request_id,
            authorization,
            None,
            PendingControlRequestState::Cancelled,
        )
    }

    /// Transition every expired **Approval** request to `Expired`. Returns the number expired.
    ///
    /// ADR 0074: Ask (`Interaction`) requests are no longer expired by wall clock — an unanswered
    /// Ask is session state, reclaimed only by an explicit ignore (user), supersede (new user
    /// message) or session archive (GC). Approvals keep their TTL until their own wave.
    pub fn expire_control_requests(&self, now_ms: u64) -> usize {
        let mut store = self
            .inner
            .pending_requests
            .lock()
            .expect("dispatcher pending requests poisoned");
        let mut expired = 0;
        for request in store.values_mut() {
            if request.request_kind != PendingControlRequestKind::Approval {
                continue;
            }
            if request.state == PendingControlRequestState::Pending && now_ms > request.expires_at_ms
            {
                request.state = PendingControlRequestState::Expired;
                request.version = request.version.saturating_add(1);
                expired += 1;
            }
        }
        expired
    }

    // ── durable Ask store (ADR 0074) ───────────────────────────────────────────────────────

    /// Install the durable Ask store shared with the host control plane. From this point on every
    /// `Interaction` (Ask) control request is persisted on creation, and the host can list/recover
    /// it after a process restart. Approvals are untouched by this seam.
    pub fn set_ask_store(&self, store: Arc<dyn AskPersistenceStore>) {
        *self
            .inner
            .ask_store
            .write()
            .expect("dispatcher ask store poisoned") = Some(store);
    }

    /// The installed durable Ask store, when the host wired one.
    pub fn ask_store(&self) -> Option<Arc<dyn AskPersistenceStore>> {
        self.inner
            .ask_store
            .read()
            .expect("dispatcher ask store poisoned")
            .clone()
    }

    /// Mirror a persisted Ask record into the in-memory control-request store, so a record created
    /// by another instance (or before a restart) can flow through the same consume/resume paths.
    ///
    /// The descriptor/digest facts are not part of the durable record: the store is the CAS
    /// authority for a persisted Ask, and the mirror only exists so `graph_resume_ask` and the
    /// answer-landing paths can see a `Consumed`/`Cancelled` state. `state` therefore is the
    /// post-transition state the caller already committed to disk.
    pub fn hydrate_persisted_ask(
        &self,
        record: &PersistedAsk,
        state: PendingControlRequestState,
    ) -> PendingControlRequest {
        let hydration_marker = format!("persisted:{}", record.request_id);
        let hydrated = PendingControlRequest {
            request_id: record.request_id.clone(),
            request_kind: record.kind.clone(),
            session_id: Some(record.session_id.clone()),
            run_id: record.run_id.clone(),
            turn_id: record.turn_id.clone(),
            call_id: record.call_id.clone(),
            descriptor_snapshot_id: hydration_marker.clone(),
            descriptor_id: hydration_marker.clone(),
            final_args_digest: hydration_marker.clone(),
            policy_digest: hydration_marker,
            nonce: record.nonce.clone(),
            version: record.version,
            // No TTL for Ask: the deadline is never a product verdict (ADR 0074).
            expires_at_ms: u64::MAX,
            state,
            prompt: Some(record.prompt.clone()),
            options: Some(json!(record.options)),
        };
        self.inner
            .pending_requests
            .lock()
            .expect("dispatcher pending requests poisoned")
            .insert(record.request_id.clone(), hydrated.clone());
        hydrated
    }

    /// Collapse every still-pending Ask of a session into `Cancelled`.
    ///
    /// The in-memory mirror only carries the dispatcher's coarse state set, so a lifecycle
    /// disposition decided elsewhere (e.g. `Superseded` for a new user message, `Orphaned` for an
    /// archived session) is mirrored as the terminal `Cancelled` state — the persisted record keeps
    /// the precise disposition. Returns how many mirrors were closed.
    pub fn close_pending_asks_of_session(&self, session_id: &str) -> usize {
        let mut store = self
            .inner
            .pending_requests
            .lock()
            .expect("dispatcher pending requests poisoned");
        let mut closed = 0;
        for request in store.values_mut() {
            if request.request_kind != PendingControlRequestKind::Interaction {
                continue;
            }
            if request.session_id.as_deref() != Some(session_id) {
                continue;
            }
            if request.state != PendingControlRequestState::Pending {
                continue;
            }
            request.state = PendingControlRequestState::Cancelled;
            request.version = request.version.saturating_add(1);
            closed += 1;
        }
        closed
    }
}

impl ToolDispatcher for GovernedDispatcher {
    fn dispatch(&self, request: ToolDispatchRequest) -> ToolOutcome {
        let context = self.default_context();
        self.dispatch_governed(request, &context)
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Pipeline implementation
// ─────────────────────────────────────────────────────────────────────────────────────────────

struct PipelineOutcome {
    outcome: ToolOutcome,
    decision: Option<PermissionDecision>,
    final_args_digest: String,
}

impl Inner {
    fn top_level_dispatch(
        self: &Arc<Self>,
        request: &ToolDispatchRequest,
        context: &DispatchContext,
    ) -> PipelineOutcome {
        // L3-B1（ADR 0065 decision 3 / 单一真值）：**每次 top-level dispatch 在入口捕获一次
        // surface generation**，并把它贯穿 preflight / handler / 生命周期记录 / 子派发。
        // 任何中途重读 live slot 都会让一次 publish（`swap_tool_surface` / `register_tool_provider`）
        // 把一次调用撕成两代：「按 G1 授权、跑 G2 执行体」。
        let surface = self.surface();
        let Some(descriptor) = Self::resolve_descriptor_in(&surface, &request.descriptor_id) else {
            return PipelineOutcome {
                outcome: DispatchError::new(
                    "unknown_descriptor",
                    format!("unknown tool descriptor `{}`", request.descriptor_id),
                )
                .into_outcome(&request.descriptor_id),
                decision: None,
                final_args_digest: String::new(),
            };
        };

        // Step 1: origin authorization + exposure.
        if let Err(error) = self.authorize_origin(&request.origin, &descriptor, &surface) {
            return PipelineOutcome {
                outcome: error.into_outcome(&request.descriptor_id),
                decision: None,
                final_args_digest: String::new(),
            };
        }

        // Step 5: per-invocation budget ledger and cancellation token. Legacy-compatibility mode
        // disables the descriptor's output-byte/deadline caps entirely (0 = unlimited in the
        // ledger), reproducing the legacy `ToolRouter`'s behavior at the runtime switch.
        let config = self.config.lock().expect("dispatcher config poisoned").clone();
        let (max_bytes, max_duration_ms) = if config.unbounded_output_and_deadline {
            (0u64, 0u64)
        } else {
            (
                if config.max_result_bytes > 0 {
                    config.max_result_bytes
                } else {
                    descriptor.execution_policy.result_budget_bytes as u64
                },
                descriptor.execution_policy.default_timeout_ms,
            )
        };
        let ledger = Arc::new(BudgetLedger::new(
            config.max_composite_calls,
            config.max_concurrency,
            max_bytes,
            self.clock.now_ms(),
            max_duration_ms,
        ));
        // wave-62 A2（融合而非复用）：本调用**自有**取消令牌（composite 的 `children.cancel()`
        // 只 cancel 它，绝不 cancel turn 令牌毒化整轮）；turn 令牌另经 `context.turn_cancellation`
        // 在闸门/收口处**并集**判定。`turn_cancellation: None` 时行为与 wave-61 完全一致。
        let cancellation = Arc::new(CancellationToken::new());
        let turn_cancellation = context.turn_cancellation.clone();
        let turn_cancelled = || {
            turn_cancellation
                .as_ref()
                .is_some_and(|token| token.is_cancelled())
        };

        // Steps 2-4: raw validation, hooks, final validation, permission/sandbox decision.
        let ready = match self.preflight_descriptor(&request.origin, context, request, &descriptor) {
            Ok(ready) => ready,
            Err(error) => {
                return PipelineOutcome {
                    outcome: error.into_outcome(&request.descriptor_id),
                    decision: None,
                    final_args_digest: String::new(),
                }
            }
        };

        let outcome = match ready.decision.verdict {
            PermissionVerdict::Deny => deny_outcome(&request.descriptor_id, &ready),
            PermissionVerdict::ApprovalRequired | PermissionVerdict::WaitingHost => {
                match self.persist_pending_request(context, request, &ready, &surface) {
                    Ok(outcome) => outcome,
                    Err(error) => error.into_outcome(&request.descriptor_id),
                }
            }
            PermissionVerdict::Allow => {
                // Steps 5-6: gates, then handler execution.
                // wave-62 A2：闸门与收口都判「自有令牌 OR turn 令牌」。
                let gate = if turn_cancelled() {
                    Err(DispatchError::cancelled("tool invocation cancelled"))
                } else {
                    check_budget_gates(&cancellation, &ledger, self.clock.as_ref())
                };
                if let Err(error) = gate {
                    error.into_outcome(&request.descriptor_id)
                } else {
                    let started = Instant::now();
                    let handler_result =
                        self.run_handler(request, &ready, &ledger, &cancellation, context, &surface);
                    self.finalize_execution(
                        &descriptor,
                        request,
                        handler_result,
                        &ledger,
                        started,
                        turn_cancelled(),
                    )
                }
            }
        };

        PipelineOutcome {
            outcome,
            decision: Some(ready.decision.clone()),
            final_args_digest: ready.final_args_digest.clone(),
        }
    }

    fn authorize_origin(
        &self,
        origin: &InvocationOrigin,
        descriptor: &ToolDescriptor,
        surface: &ToolSurfaceGeneration,
    ) -> Result<(), DispatchError> {
        match origin {
            InvocationOrigin::Model => {
                if descriptor.exposure == ToolExposure::Internal {
                    return Err(DispatchError::new(
                        "origin_not_authorized",
                        format!(
                            "model origin cannot call internal descriptor `{}`",
                            descriptor.identity.descriptor_id
                        ),
                    ));
                }
                // Turn view and registry ship in the same generation, so this check is a
                // consistency invariant of the published surface (it can only trip if a caller
                // installed a hand-built view from another snapshot generation).
                let view = &surface.turn_view;
                if view.snapshot_id != surface.registry.snapshot_id {
                    return Err(DispatchError::new(
                        "turn_view_snapshot_mismatch",
                        format!(
                            "turn view snapshot `{}` does not match registry snapshot `{}`",
                            view.snapshot_id, surface.registry.snapshot_id
                        ),
                    ));
                }
                if !view.allows_model_descriptor(&descriptor.identity.descriptor_id) {
                    return Err(DispatchError::new(
                        "origin_not_authorized",
                        format!(
                            "descriptor `{}` is not visible in the current turn tool view",
                            descriptor.identity.descriptor_id
                        ),
                    ));
                }
                Ok(())
            }
            InvocationOrigin::Host | InvocationOrigin::System => {
                // Trusted adapter origins may reach any descriptor, including internal
                // descriptors and deferred (non-elevated) descriptors. Bare model and
                // lineage-less child origins cannot.
                Ok(())
            }
            InvocationOrigin::Child => Err(DispatchError::new(
                "origin_not_authorized",
                "child origin requires a bounded child dispatch context",
            )),
        }
    }

    /// Steps 2-4 of the pipeline: raw validation, mutable hooks, final validation, and the
    /// permission/sandbox decision over the final normalized arguments.
    fn preflight_descriptor(
        &self,
        origin: &InvocationOrigin,
        context: &DispatchContext,
        request: &ToolDispatchRequest,
        descriptor: &ToolDescriptor,
    ) -> Result<PreflightedChild, DispatchError> {
        validate_against_schema(&request.arguments, &descriptor.input_schema, "$", 0).map_err(
            |message| {
                DispatchError::new(
                    "invalid_arguments",
                    format!(
                        "arguments for `{}` are invalid: {message}",
                        descriptor.identity.descriptor_id
                    ),
                )
            },
        )?;

        let mut final_arguments = request.arguments.clone();
        let hooks = self
            .pre_dispatch_hooks
            .lock()
            .expect("dispatcher hooks poisoned")
            .clone();
        for hook in hooks {
            final_arguments = hook
                .rewrite(
                    &descriptor.identity.descriptor_id,
                    origin,
                    &request.call_id,
                    final_arguments,
                )
                .map_err(|message| {
                    DispatchError::new(
                        "pre_dispatch_hook_error",
                        format!(
                            "pre-dispatch hook rejected `{}`: {message}",
                            descriptor.identity.descriptor_id
                        ),
                    )
                })?;
        }

        validate_against_schema(&final_arguments, &descriptor.input_schema, "$", 0).map_err(
            |message| {
                DispatchError::new(
                    "invalid_arguments",
                    format!(
                        "hooked arguments for `{}` are invalid: {message}",
                        descriptor.identity.descriptor_id
                    ),
                )
            },
        )?;

        let evaluator = self
            .policy_evaluator
            .read()
            .expect("dispatcher policy evaluator poisoned")
            .clone();
        let decision =
            evaluator.evaluate_with_context(descriptor, origin, &final_arguments, context);

        // The sandbox gate applies only to executions the policy allows, so a denied or
        // approval-required invocation surfaces its real verdict instead of being masked by
        // sandbox availability. Absence of a registered backend fails closed (design.md
        // Decision 7: no sandbox backend => autonomous Execute is not allowed).
        if decision.verdict == PermissionVerdict::Allow && self.requires_sandbox(descriptor) {
            let backend = self
                .sandbox_backend
                .read()
                .expect("dispatcher sandbox backend poisoned")
                .clone();
            let backend = backend.as_deref().ok_or_else(|| {
                DispatchError::new(
                    "sandbox_unavailable",
                    format!(
                        "descriptor `{}` requires a sandbox but no sandbox backend is registered",
                        descriptor.identity.descriptor_id
                    ),
                )
            })?;
            match backend.availability() {
                SandboxAvailability::Unavailable => {
                    return Err(DispatchError::new(
                        "sandbox_unavailable",
                        format!(
                            "descriptor `{}` requires a sandbox that is unavailable",
                            descriptor.identity.descriptor_id
                        ),
                    ));
                }
                _ => {
                    let sandbox_request = SandboxRequest {
                        workspace_root: context
                            .workspace_root
                            .clone()
                            .unwrap_or_else(|| ".".to_string()),
                        allow_network: false,
                        environment_allowlist: Vec::new(),
                        isolate_environment: true,
                    };
                    if let Err(message) = backend.validate(&sandbox_request) {
                        return Err(DispatchError::new(
                            "sandbox_denied",
                            format!(
                                "sandbox rejected `{}`: {message}",
                                descriptor.identity.descriptor_id
                            ),
                        ));
                    }
                }
            }
        }

        let final_args_digest = stable_digest(&final_arguments);
        let policy_digest = stable_digest(&policy_payload(descriptor, &decision));
        Ok(PreflightedChild {
            descriptor: descriptor.clone(),
            final_arguments,
            decision,
            final_args_digest,
            policy_digest,
        })
    }

    fn requires_sandbox(&self, descriptor: &ToolDescriptor) -> bool {
        descriptor.kind == ToolKind::Execute
            || descriptor
                .permission_declaration
                .scopes
                .contains(&ToolPermissionScope::WorkspaceExecute)
    }

    /// Step 6: run the registered primitive handler or the composite handler with a restricted
    /// `ChildDispatch`.
    fn run_handler(
        self: &Arc<Self>,
        request: &ToolDispatchRequest,
        ready: &PreflightedChild,
        ledger: &Arc<BudgetLedger>,
        cancellation: &Arc<CancellationToken>,
        context: &DispatchContext,
        surface: &Arc<ToolSurfaceGeneration>,
    ) -> Result<Value, DispatchError> {
        let descriptor = &ready.descriptor;
        // 执行体必须来自**同一次捕获的代**，不得再读 live slot（L3-B1）。
        if let Some(handler) = surface
            .handlers
            .get(&descriptor.identity.descriptor_id)
            .cloned()
        {
            return handler
                .execute(&PrimitiveToolHandlerRequest {
                    descriptor_id: descriptor.identity.descriptor_id.clone(),
                    arguments: ready.final_arguments.clone(),
                    session_id: context.session_id.clone(),
                    // PA-080 P1-2：会话级 workspace root 透传，工具权限判定锚定会话 workspace。
                    workspace_root: context.workspace_root.clone(),
                    // wave-62 A1：把 turn 令牌交给 handler（桥接层再透传给工具上下文），
                    // 子进程类工具因此能在运行期观测 stop。
                    cancellation: context.turn_cancellation.clone(),
                })
                .map_err(handler_failure);
        }
        if let Some(composite_handler) = self.get_composite_handler(&descriptor.identity.descriptor_id)
        {
            let child_context = self.composite_child_context(
                request,
                descriptor,
                vec![descriptor.identity.descriptor_id.clone()],
                0,
                context,
                ledger,
                cancellation,
            );
            // The child runner is pinned to the surface generation this composite started in: its
            // `registry()` (read by composites for conservative permission aggregation) must agree
            // with the handlers the children are dispatched to.
            let child_dispatch = ChildDispatch::new(
                Arc::new(PinnedSurfaceRunner {
                    inner: Arc::clone(self),
                    surface: Arc::clone(surface),
                }) as Arc<dyn ChildDispatchRunner>,
                child_context,
            );
            let composite_request = CompositeToolHandlerRequest {
                descriptor_id: descriptor.identity.descriptor_id.clone(),
                call_id: request.call_id.clone(),
                arguments: ready.final_arguments.clone(),
            };
            return composite_handler
                .execute(&composite_request, &child_dispatch)
                .map_err(handler_failure);
        }
        Err(DispatchError::new(
            "no_handler_registered",
            format!(
                "no primitive handler or composite handler registered for `{}`",
                descriptor.identity.descriptor_id
            ),
        ))
    }

    fn composite_child_context(
        &self,
        request: &ToolDispatchRequest,
        descriptor: &ToolDescriptor,
        lineage: Vec<String>,
        depth: u32,
        context: &DispatchContext,
        ledger: &Arc<BudgetLedger>,
        cancellation: &Arc<CancellationToken>,
    ) -> ChildDispatchContext {
        ChildDispatchContext {
            parent_call_id: request.call_id.clone(),
            parent_descriptor_id: descriptor.identity.descriptor_id.clone(),
            lineage,
            depth,
            allowed_child_ids: self.allowed_children_for(descriptor),
            ledger: Arc::clone(ledger),
            cancellation: Arc::clone(cancellation),
            context: context.clone(),
            clock: Arc::clone(&self.clock),
            suspended: Arc::new(AtomicBool::new(false)),
        }
    }

    fn finalize_execution(
        &self,
        descriptor: &ToolDescriptor,
        request: &ToolDispatchRequest,
        handler_result: Result<Value, DispatchError>,
        ledger: &BudgetLedger,
        started: Instant,
        turn_cancelled: bool,
    ) -> ToolOutcome {
        // wave-62 C2/C5：**取消优先**。优先级：
        // ① handler 自己报告了取消（如 `Run` 的 `tool_cancelled`）⇒ **保真其错误码**，
        //    不覆盖成通用 `cancelled`（契约补遗 v2：码必须原样保留）；
        // ② 本轮 turn 令牌已取消 ⇒ fail-closed 为 `Cancelled`；
        // ③ 其余按原语义。
        match handler_result {
            Err(error) if error.cancelled => error.into_outcome(&request.descriptor_id),
            _ if turn_cancelled => {
                DispatchError::cancelled("tool invocation cancelled")
                    .into_outcome(&request.descriptor_id)
            }
            Ok(value) => {
                let output = render_output(&value);
                if let Err(message) = ledger.reserve_bytes(output.len() as u64) {
                    return DispatchError::new("output_budget_exceeded", message)
                        .into_outcome(&request.descriptor_id);
                }
                ToolOutcome::from_legacy_result(ToolResult {
                    tool_name: descriptor.identity.model_name.clone(),
                    status: "ok".to_string(),
                    output,
                    duration_ms: started.elapsed().as_millis() as u64,
                })
            }
            Err(error) => error.into_outcome(&request.descriptor_id),
        }
    }

    /// Resolve a raw invocation name **inside a pinned generation**. The registry snapshot matches
    /// aliases (product names, primitives, namespaced aliases); the dispatcher additionally matches
    /// the canonical `descriptor_id` itself so callers can invoke a tool by its stable identity.
    ///
    /// Returns an owned descriptor so the caller cannot accidentally keep borrowing the slot.
    fn resolve_descriptor_in(
        surface: &ToolSurfaceGeneration,
        raw_name: &str,
    ) -> Option<ToolDescriptor> {
        if let Some(descriptor) = surface.registry.resolve(raw_name) {
            return Some(descriptor.clone());
        }
        let trimmed = raw_name.trim();
        surface
            .registry
            .descriptors
            .iter()
            .find(|descriptor| descriptor.identity.descriptor_id == trimmed)
            .cloned()
    }


    fn get_composite_handler(&self, descriptor_id: &str) -> Option<Arc<dyn CompositeToolHandler>> {
        self.composite_handlers
            .lock()
            .expect("dispatcher composite handler registry poisoned")
            .get(descriptor_id)
            .cloned()
    }

    fn allowed_children_for(&self, descriptor: &ToolDescriptor) -> BTreeSet<String> {
        let mut allowed: BTreeSet<String> = descriptor
            .composed_descriptor_ids
            .iter()
            .cloned()
            .collect();
        if let Some(authority) = self
            .composite_authority
            .lock()
            .expect("dispatcher composite authority poisoned")
            .get(&descriptor.identity.descriptor_id)
        {
            allowed.extend(authority.iter().cloned());
        }
        allowed
    }

    /// Step 7: persist a `PendingControlRequest` and return a pending control outcome. Without
    /// an interactive host the invocation fails closed with `interaction_unavailable`.
    fn persist_pending_request(
        &self,
        context: &DispatchContext,
        request: &ToolDispatchRequest,
        ready: &PreflightedChild,
        surface: &ToolSurfaceGeneration,
    ) -> Result<ToolOutcome, DispatchError> {
        if !context.host_control_available {
            return Err(DispatchError::new(
                "interaction_unavailable",
                "no interactive host is available to answer this control request",
            ));
        }
        let (kind, prompt) = match ready.decision.verdict {
            PermissionVerdict::ApprovalRequired => (
                ToolControlKind::ApprovalRequired,
                format!(
                    "Tool `{}` ({}) requires approval. Final arguments digest: {}.",
                    ready.descriptor.identity.descriptor_id,
                    ready.descriptor.identity.model_name,
                    ready.final_args_digest
                ),
            ),
            PermissionVerdict::WaitingHost => (
                ToolControlKind::WaitingHost,
                // The Ask tool's own question is the user-facing prompt (phase-4 wiring P2-9):
                // surface `text`/`question`/`prompt` verbatim instead of a synthetic digest.
                ask_prompt_from_arguments(&ready.final_arguments).unwrap_or_else(|| {
                    format!(
                        "Tool `{}` ({}) requires host mediation. Final arguments digest: {}.",
                        ready.descriptor.identity.descriptor_id,
                        ready.descriptor.identity.model_name,
                        ready.final_args_digest
                    )
                }),
            ),
            _ => {
                return Err(DispatchError::new(
                    "internal",
                    "persist_pending_request called for a non-pending verdict",
                ))
            }
        };

        let now = self.clock.now_ms();
        let config = self.config.lock().expect("dispatcher config poisoned").clone();
        let request_id = format!("control-{:012}", next_control_request_seq());
        let nonce = format!("nonce-{:012}", self.nonce_seq.fetch_add(1, Ordering::Relaxed));
        // Design Decision 5: an Ask (`waiting_host`) roundtrip persists as an `Interaction` request
        // (answered via `answer_control_request`); an approval persists as `Approval`. The kind is
        // derived from the verdict so the two control paths stay distinct (phase-4 wiring).
        let request_kind = match ready.decision.verdict {
            PermissionVerdict::ApprovalRequired => PendingControlRequestKind::Approval,
            PermissionVerdict::WaitingHost => PendingControlRequestKind::Interaction,
            _ => {
                return Err(DispatchError::new(
                    "internal",
                    "persist_pending_request called for a non-pending verdict",
                ))
            }
        };
        let pending = PendingControlRequest {
            request_id: request_id.clone(),
            request_kind,
            session_id: context.session_id.clone(),
            run_id: context.run_id.clone(),
            turn_id: context.turn_id.clone().unwrap_or_else(|| "unknown".to_string()),
            call_id: request.call_id.clone(),
            descriptor_snapshot_id: surface.registry.snapshot_id.clone(),
            descriptor_id: ready.descriptor.identity.descriptor_id.clone(),
            final_args_digest: ready.final_args_digest.clone(),
            policy_digest: ready.policy_digest.clone(),
            nonce,
            version: 1,
            expires_at_ms: now.saturating_add(config.control_request_expiry_ms),
            state: PendingControlRequestState::Pending,
            prompt: Some(prompt),
            options: ask_options_from_arguments(&ready.final_arguments),
        };
        // ADR 0074: an Ask is persisted state, not process memory. Persist the full CAS facts
        // (`version`, `nonce`) so `list_pending` works after a restart and a second instance can
        // reject replays. A persistence failure is logged but does not fail the dispatch: the
        // Ask still reaches the host through the in-memory mirror, and the host read path
        // (`list_pending_asks`) reports the degraded store instead of pretending success.
        if pending.request_kind == PendingControlRequestKind::Interaction {
            let store = self
                .ask_store
                .read()
                .expect("dispatcher ask store poisoned")
                .clone();
            if let Some(store) = store {
                let record = persisted_ask_from_pending(&pending, now);
                if let Err(error) = store.upsert(&record) {
                    // Fail closed (contract §5/§7): an Ask that could not be persisted must not
                    // be announced as pending — the host would expose a question that no restart
                    // can recover. The caller sees a structured error instead.
                    eprintln!(
                        "[pony-agent] ask persistence: upsert of `{}` failed; failing the dispatch: \
                         {error}",
                        pending.request_id
                    );
                    return Err(DispatchError::new(
                        "ask_persist_failed",
                        format!(
                            "Ask `{}` could not be persisted to the session store: {error}",
                            pending.request_id
                        ),
                    ));
                }
            }
        }
        self.pending_requests
            .lock()
            .expect("dispatcher pending requests poisoned")
            .insert(request_id.clone(), pending);
        Ok(ToolOutcome::pending(ToolControlOutcome { kind, request_id }))
    }

    /// Compare-and-swap control-request consumption. Rejects replay, expiry, cross-session,
    /// version/nonce, descriptor, source-version, and kind mismatches, then bumps the version so
    /// the same authorization can never be replayed.
    fn consume_control_request(
        &self,
        request_id: &str,
        authorization: &ControlRequestAuthorization,
        expected_kind: Option<PendingControlRequestKind>,
        target_state: PendingControlRequestState,
    ) -> Result<ControlRequestConsumed, String> {
        let now = self.clock.now_ms();
        let mut store = self
            .pending_requests
            .lock()
            .expect("dispatcher pending requests poisoned");
        let request = store
            .get_mut(request_id)
            .ok_or_else(|| format!("unknown control request `{request_id}`"))?;
        if !request.can_consume(
            authorization.session_id.as_deref(),
            authorization.expected_version,
            &authorization.nonce,
            now,
        ) {
            return Err(format!(
                "control request `{request_id}` cannot be consumed: replay, expiry, cross-session, or version/nonce mismatch"
            ));
        }
        if let Some(expected_snapshot) = &authorization.expected_descriptor_snapshot_id {
            if request.descriptor_snapshot_id != *expected_snapshot {
                return Err(format!(
                    "control request `{request_id}` descriptor snapshot mismatch"
                ));
            }
        }
        if let Some(expected_descriptor) = &authorization.expected_descriptor_id {
            if request.descriptor_id != *expected_descriptor {
                return Err(format!("control request `{request_id}` descriptor mismatch"));
            }
        }
        if let Some(expected_digest) = &authorization.expected_final_args_digest {
            if request.final_args_digest != *expected_digest {
                return Err(format!(
                    "control request `{request_id}` final-arguments digest mismatch"
                ));
            }
        }
        if let Some(expected_digest) = &authorization.expected_policy_digest {
            if request.policy_digest != *expected_digest {
                return Err(format!("control request `{request_id}` policy digest mismatch"));
            }
        }
        if let Some(expected_kind) = expected_kind {
            if request.request_kind != expected_kind {
                return Err(format!("control request `{request_id}` kind mismatch"));
            }
        }
        request.state = target_state;
        request.version = request.version.saturating_add(1);
        let answer = authorization.answer.clone();
        Ok(ControlRequestConsumed {
            request: request.clone(),
            answer,
        })
    }

    fn emit(&self, record: &DispatchLifecycleRecord) {
        // Clone the observer list before iterating so a panicking or re-entrant observer cannot
        // poison the shared lock and brick every subsequent dispatch (phase-3 review P2-4).
        let observers = self
            .lifecycle_observers
            .lock()
            .expect("dispatcher lifecycle observers poisoned")
            .clone();
        for observer in observers {
            observer.record(record);
        }
    }
}

/// `ChildDispatchRunner` bound to one published tool-surface generation.
///
/// The trait's `registry()` hands out a `&ToolRegistrySnapshot` (composites read descriptor facts
/// for conservative permission aggregation), which a swappable slot cannot lend out safely. The
/// runner therefore owns the pinned `Arc` of the generation it was created for, so children
/// resolve descriptors and handlers from **the same** generation — never a mix of old registry and
/// new handlers.
pub(crate) struct PinnedSurfaceRunner {
    inner: Arc<Inner>,
    surface: Arc<ToolSurfaceGeneration>,
}

impl std::ops::Deref for PinnedSurfaceRunner {
    type Target = Inner;

    fn deref(&self) -> &Inner {
        self.inner.as_ref()
    }
}

impl PinnedSurfaceRunner {
    /// L3-B2：descriptor 解析必须用**钉住的代**，不能经 `Deref` 落到 `Inner` 的 live slot。
    ///
    /// 否则一次 publish 会让子调用「按新 descriptor 授权、跑钉住的旧 handler」——两个方向都是
    /// 撕裂。此 inherent 方法遮蔽 `Deref` 目标的同名方法。
    fn resolve_descriptor(&self, raw_name: &str) -> Option<ToolDescriptor> {
        Inner::resolve_descriptor_in(&self.surface, raw_name)
    }
}

impl ChildDispatchRunner for PinnedSurfaceRunner {
    fn preflight(
        &self,
        ctx: &ChildDispatchContext,
        request: &ChildDispatchRequest,
    ) -> Result<PreflightedChild, DispatchError> {
        let descriptor = self
            .resolve_descriptor(&request.descriptor_id)
            .ok_or_else(|| {
                DispatchError::new(
                    "unknown_descriptor",
                    format!("unknown tool descriptor `{}`", request.descriptor_id),
                )
            })?;
        let descriptor_id = descriptor.identity.descriptor_id.clone();
        if ctx.lineage.contains(&descriptor_id) {
            return Err(DispatchError::new(
                "child_cycle",
                format!(
                    "child descriptor `{descriptor_id}` is already present in the dispatch lineage; cycles are forbidden"
                ),
            ));
        }
        // Defense-in-depth: the parent descriptor is always the last element of the lineage
        // (pushed when the composite child context was created), so a self-dispatch is caught by
        // the `child_cycle` check above. This branch is retained as a redundant guard in case a
        // future context-construction change stops including the parent in the lineage.
        if descriptor_id == ctx.parent_descriptor_id {
            return Err(DispatchError::new(
                "child_self_recursion",
                format!("child descriptor `{descriptor_id}` may not dispatch itself"),
            ));
        }
        if !ctx.allowed_child_ids.contains(&descriptor_id) {
            return Err(DispatchError::new(
                "child_not_allowed",
                format!(
                    "descriptor `{descriptor_id}` is not an allowed child of `{}`",
                    ctx.parent_descriptor_id
                ),
            ));
        }
        let max_depth = self
            .config
            .lock()
            .expect("dispatcher config poisoned")
            .max_composite_depth;
        if ctx.depth.saturating_add(1) > max_depth {
            return Err(DispatchError::new(
                "child_depth_exceeded",
                format!(
                    "maximum child dispatch depth of {max_depth} exceeded at `{descriptor_id}`"
                ),
            ));
        }

        self.preflight_descriptor(
            &InvocationOrigin::Child,
            &ctx.context,
            &ToolDispatchRequest {
                origin: InvocationOrigin::Child,
                descriptor_id: request.descriptor_id.clone(),
                call_id: request.call_id.clone(),
                arguments: request.arguments.clone(),
            },
            &descriptor,
        )
    }

    fn execute_primitive(
        &self,
        ctx: &ChildDispatchContext,
        request: &ChildDispatchRequest,
        ready: PreflightedChild,
    ) -> ToolOutcome {
        if let Err(error) = check_budget_gates(&ctx.cancellation, &ctx.ledger, ctx.clock.as_ref())
        {
            return error.into_outcome(&request.descriptor_id);
        }
        if let Err(message) = ctx.ledger.reserve_call() {
            return DispatchError::new("budget_exhausted", message).into_outcome(&request.descriptor_id);
        }
        let started = Instant::now();
        let _concurrency = match ctx.ledger.begin_execution() {
            Ok(guard) => guard,
            Err(message) => {
                return DispatchError::new("budget_exhausted", message).into_outcome(&request.descriptor_id);
            }
        };
        // Pinned generation: the child must run against the handlers of the surface it was
        // authorized in, never a handler published after its authorization.
        let Some(handler) = self
            .surface
            .handlers
            .get(&ready.descriptor.identity.descriptor_id)
            .cloned()
        else {
            return DispatchError::new(
                "no_handler_registered",
                format!(
                    "no primitive handler registered for `{}`",
                    ready.descriptor.identity.descriptor_id
                ),
            )
            .into_outcome(&request.descriptor_id);
        };
        match handler.execute(&PrimitiveToolHandlerRequest {
            descriptor_id: ready.descriptor.identity.descriptor_id.clone(),
            arguments: ready.final_arguments.clone(),
            session_id: ctx.context.session_id.clone(),
            workspace_root: ctx.context.workspace_root.clone(),
            // wave-62 A1：同一桥接语义（turn 令牌随请求下传，不得在直派路径丢失）。
            cancellation: ctx.context.turn_cancellation.clone(),
        }) {
            Ok(value) => {
                let output = render_output(&value);
                if let Err(message) = ctx.ledger.reserve_bytes(output.len() as u64) {
                    return DispatchError::new("output_budget_exceeded", message)
                        .into_outcome(&request.descriptor_id);
                }
                ToolOutcome::from_legacy_result(ToolResult {
                    tool_name: ready.descriptor.identity.model_name.clone(),
                    status: "ok".to_string(),
                    output,
                    duration_ms: started.elapsed().as_millis() as u64,
                })
            }
            Err(message) => {
                DispatchError::new("handler_error", message).into_outcome(&request.descriptor_id)
            }
        }
    }

    fn persist_pending(
        &self,
        ctx: &ChildDispatchContext,
        request: &ChildDispatchRequest,
        ready: PreflightedChild,
    ) -> ToolOutcome {
        let top_request = ToolDispatchRequest {
            origin: InvocationOrigin::Child,
            descriptor_id: request.descriptor_id.clone(),
            call_id: request.call_id.clone(),
            arguments: ready.final_arguments.clone(),
        };
        match self.persist_pending_request(&ctx.context, &top_request, &ready, &self.surface) {
            Ok(outcome) => outcome,
            Err(error) => error.into_outcome(&request.descriptor_id),
        }
    }

    fn composite_handler(&self, descriptor_id: &str) -> Option<Arc<dyn CompositeToolHandler>> {
        Inner::get_composite_handler(self, descriptor_id)
    }

    fn allowed_children_for(&self, descriptor: &ToolDescriptor) -> BTreeSet<String> {
        Inner::allowed_children_for(self, descriptor)
    }

    fn registry(&self) -> &ToolRegistrySnapshot {
        // Pinned generation: never a mix of a new registry with old handlers.
        self.surface.registry.as_ref()
    }

    fn emit(&self, record: &DispatchLifecycleRecord) {
        Inner::emit(self, record);
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Shared helpers
// ─────────────────────────────────────────────────────────────────────────────────────────────

pub(crate) fn build_lifecycle_record(
    descriptor_id: String,
    call_id: String,
    parent_call_id: Option<String>,
    origin: InvocationOrigin,
    depth: u32,
    outcome: &ToolOutcome,
    decision: Option<&PermissionDecision>,
    final_args_digest: String,
    elapsed_ms: u64,
) -> DispatchLifecycleRecord {
    let phase = if outcome.control_outcome.is_some() {
        DispatchLifecyclePhase::ControlPending
    } else {
        match outcome.execution_status {
            ToolExecutionStatus::Ok => DispatchLifecyclePhase::Completed,
            ToolExecutionStatus::Cancelled => DispatchLifecyclePhase::Cancelled,
            ToolExecutionStatus::Error => DispatchLifecyclePhase::Failed,
        }
    };
    DispatchLifecycleRecord {
        descriptor_id,
        call_id,
        parent_call_id,
        origin,
        depth,
        phase,
        decision: decision.map(|item| item.verdict.clone()),
        decision_source: decision.map(|item| item.decision_source.clone()),
        execution_status: Some(outcome.execution_status.clone()),
        control_outcome: outcome.control_outcome.clone(),
        final_args_digest,
        error_code: outcome_error_code(outcome),
        elapsed_ms,
    }
}

fn outcome_error_code(outcome: &ToolOutcome) -> Option<String> {
    outcome
        .result
        .as_ref()
        .and_then(|result| serde_json::from_str::<Value>(&result.output).ok())
        .and_then(|value| {
            value
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str)
                .map(str::to_string)
        })
}

pub(crate) fn deny_outcome(tool_name: &str, ready: &PreflightedChild) -> ToolOutcome {
    let reason = ready
        .decision
        .reason
        .as_deref()
        .unwrap_or("declared policy forbids this invocation");
    // W36 / PA-141（C-141-6）：宿主审批闸门的 Deny 单独路由为
    // `plan_execution_approval_required`，不改变既有 `permission_denied` 语义。
    let code = if ready.decision.decision_source == "plan_execution_approval_gate" {
        "plan_execution_approval_required"
    } else {
        "permission_denied"
    };
    DispatchError::new(
        code,
        format!(
            "permission denied for `{}`: {reason} (decision source: {})",
            ready.descriptor.identity.descriptor_id, ready.decision.decision_source
        ),
    )
    .into_outcome(tool_name)
}

pub(crate) fn render_output(value: &Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

/// Surface the Ask tool's user-facing question verbatim (phase-4 wiring P2-9). Recognizes the
/// common argument keys `text`/`question`/`prompt`; falls back to `None` so the caller uses a
/// synthetic digest prompt.
fn ask_prompt_from_arguments(arguments: &Value) -> Option<String> {
    for key in ["text", "question", "prompt"] {
        if let Some(value) = arguments.get(key).and_then(Value::as_str) {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Extract an `options` array from Ask arguments if the model supplied one.
fn ask_options_from_arguments(arguments: &Value) -> Option<Value> {
    match arguments.get("options") {
        Some(Value::Array(items)) if !items.is_empty() => Some(Value::Array(items.clone())),
        _ => None,
    }
}

fn stable_digest(value: &Value) -> String {
    let canonical = serde_json::to_string(value).unwrap_or_default();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in canonical.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

fn policy_payload(descriptor: &ToolDescriptor, decision: &PermissionDecision) -> Value {
    let scopes = descriptor
        .permission_declaration
        .scopes
        .iter()
        .map(|scope| match scope {
            ToolPermissionScope::WorkspaceRead => "workspace.read",
            ToolPermissionScope::WorkspaceWrite => "workspace.write",
            ToolPermissionScope::WorkspaceExecute => "workspace.execute",
            ToolPermissionScope::CapabilityDiscovery => "capability.discovery",
        })
        .collect::<Vec<_>>();
    let verdict = match decision.verdict {
        PermissionVerdict::Allow => "allow",
        PermissionVerdict::Deny => "deny",
        PermissionVerdict::ApprovalRequired => "approval_required",
        PermissionVerdict::WaitingHost => "waiting_host",
    };
    json!({
        "scopes": scopes,
        "requires_approval": descriptor.permission_declaration.requires_approval,
        "host_mediated": descriptor.permission_declaration.host_mediated,
        "approval_mode": descriptor.permission_declaration.approval_mode,
        "decision_source": decision.decision_source,
        "verdict": verdict,
    })
}

/// Minimal JSON Schema validator sufficient for the descriptor input schemas: object type,
/// required properties, per-property type checks, array items, and `additionalProperties`.
/// Deliberately shallow (depth-capped) and fail closed.
fn validate_against_schema(value: &Value, schema: &Value, path: &str, depth: usize) -> Result<(), String> {
    if depth > 16 {
        return Err(format!(
            "argument schema validation exceeded maximum depth at `{path}`"
        ));
    }
    let Some(schema_object) = schema.as_object() else {
        // An absent or non-object schema imposes no constraints.
        return Ok(());
    };
    match schema_object.get("type").and_then(Value::as_str) {
        None => {
            if schema_object.contains_key("properties") {
                validate_object_arguments(value, schema, path, depth)
            } else {
                Ok(())
            }
        }
        Some("object") => validate_object_arguments(value, schema, path, depth),
        Some("string") => {
            if value.is_string() {
                Ok(())
            } else {
                Err(format!("argument at `{path}` must be a string"))
            }
        }
        // T2-A 三态契约（硬门，逻辑不动）：integer 分支只看类型——浮点/字符串/bool
        // 在此直接 `invalid_arguments`；负整数（`as_i64` 成功）过门进入 parser，
        // 由 `parse_start_line_arg` 判为 `Invalid` 并回退到 1 + 留痕（direct-call
        // 全类型同理，不过此门）。
        Some("integer") => {
            if value.as_i64().is_some() || value.as_u64().is_some() {
                Ok(())
            } else {
                Err(format!("argument at `{path}` must be an integer"))
            }
        }
        Some("number") => {
            if value.is_number() {
                Ok(())
            } else {
                Err(format!("argument at `{path}` must be a number"))
            }
        }
        Some("boolean") => {
            if value.is_boolean() {
                Ok(())
            } else {
                Err(format!("argument at `{path}` must be a boolean"))
            }
        }
        Some("array") => {
            if !value.is_array() {
                return Err(format!("argument at `{path}` must be an array"));
            }
            if let Some(items) = schema_object.get("items") {
                for (index, item) in value.as_array().expect("checked array").iter().enumerate() {
                    validate_against_schema(item, items, &format!("{path}[{index}]"), depth + 1)?;
                }
            }
            Ok(())
        }
        Some("null") => {
            if value.is_null() {
                Ok(())
            } else {
                Err(format!("argument at `{path}` must be null"))
            }
        }
        Some(other) => Err(format!(
            "unsupported argument schema type `{other}` at `{path}`"
        )),
    }
}

fn validate_object_arguments(
    value: &Value,
    schema: &Value,
    path: &str,
    depth: usize,
) -> Result<(), String> {
    let Some(object) = value.as_object() else {
        return Err(format!("arguments at `{path}` must be an object"));
    };
    if let Some(required) = schema.get("required").and_then(Value::as_array) {
        for entry in required {
            let name = entry
                .as_str()
                .ok_or_else(|| format!("schema `required` entry at `{path}` must be a string"))?;
            if object.get(name).map_or(true, Value::is_null) {
                return Err(format!("missing required argument `{name}` at `{path}`"));
            }
        }
    }
    let properties = schema.get("properties").and_then(Value::as_object);
    if let Some(properties) = properties {
        for (name, property_value) in object.iter() {
            if let Some(property_schema) = properties.get(name) {
                validate_against_schema(
                    property_value,
                    property_schema,
                    &format!("{path}.{name}"),
                    depth + 1,
                )?;
            }
        }
    }
    if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
        let Some(properties) = properties else {
            return Err(format!(
                "arguments at `{path}` contain properties but the schema forbids additional properties"
            ));
        };
        for name in object.keys() {
            if !properties.contains_key(name) {
                return Err(format!(
                    "unexpected argument `{name}` at `{path}` (schema forbids additional properties)"
                ));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::budget::DispatchBudgetConfig;
    use crate::agent::child_dispatch::ChildDispatchRequest;
    use crate::agent::tool_runtime::FakeClock;
    use crate::agent::tools::{
        ToolDescriptorSource, ToolDisplayMetadata, ToolExecutionPolicy, ToolHandlerProvenance,
        ToolIdentity, ToolPermissionDeclaration, ToolPermissionScope, ToolProvider,
        ToolProviderError,
    };
    use serde_json::json;

    // ── helpers ──────────────────────────────────────────────────────────────────────────────

    fn descriptor(
        id: &str,
        kind: ToolKind,
        exposure: ToolExposure,
    ) -> ToolDescriptor {
        descriptor_with_declaration(id, kind, exposure, ToolPermissionDeclaration::default())
    }

    fn descriptor_with_declaration(
        id: &str,
        kind: ToolKind,
        exposure: ToolExposure,
        declaration: ToolPermissionDeclaration,
    ) -> ToolDescriptor {
        ToolDescriptor {
            identity: ToolIdentity {
                descriptor_id: id.to_string(),
                model_name: id.to_string(),
                canonical_name: id.to_string(),
                primitive_name: id.to_string(),
                source: ToolDescriptorSource::Builtin,
            },
            aliases: vec![id.to_string(), format!("alias:{id}")],
            description: String::new(),
            input_schema: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
                "additionalProperties": false,
            }),
            kind,
            exposure,
            permission_declaration: declaration,
            execution_policy: ToolExecutionPolicy::default(),
            display_metadata: ToolDisplayMetadata::default(),
            handler_provenance: ToolHandlerProvenance {
                handler_kind: "test".to_string(),
                source_id: "builtin-tools".to_string(),
            },
            source_revision: "test-v1".to_string(),
            composed_descriptor_ids: Vec::new(),
        }
    }

    fn registry(descriptors: Vec<ToolDescriptor>) -> Arc<ToolRegistrySnapshot> {
        Arc::new(
            ToolRegistrySnapshot::from_descriptors("test-snapshot-1", descriptors)
                .expect("test registry must build"),
        )
    }

    fn dispatcher_for(registry: Arc<ToolRegistrySnapshot>) -> GovernedDispatcher {
        GovernedDispatcher::new(registry, Arc::new(FakeClock::new(1_000)))
    }

    fn echo_handler() -> Arc<dyn PrimitiveToolHandler> {
        #[derive(Clone)]
        struct Echo;
        impl PrimitiveToolHandler for Echo {
            fn execute(&self, request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
                Ok(request.arguments.clone())
            }
        }
        Arc::new(Echo)
    }

    fn dispatch(
        dispatcher: &GovernedDispatcher,
        origin: InvocationOrigin,
        descriptor_id: &str,
        call_id: &str,
        arguments: Value,
    ) -> ToolOutcome {
        dispatcher.dispatch(ToolDispatchRequest {
            origin,
            descriptor_id: descriptor_id.to_string(),
            call_id: call_id.to_string(),
            arguments,
        })
    }

    fn error_code(outcome: &ToolOutcome) -> Option<String> {
        outcome
            .result
            .as_ref()
            .and_then(|result| serde_json::from_str::<Value>(&result.output).ok())
            .and_then(|value| {
                value
                    .get("error")
                    .and_then(|error| error.get("code"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
    }

    fn approval_required_declaration() -> ToolPermissionDeclaration {
        let mut declaration = ToolPermissionDeclaration::default();
        declaration.requires_approval = true;
        declaration
    }

    // ── origin authorization + exposure ──────────────────────────────────────────────────────

    #[test]
    fn model_origin_cannot_call_internal_descriptor() {
        let registry = registry(vec![descriptor(
            "builtin:secret",
            ToolKind::Read,
            ToolExposure::Internal,
        )]);
        let dispatcher = dispatcher_for(registry);
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:secret",
            "call-1",
            json!({ "text": "x" }),
        );
        assert_eq!(
            outcome.execution_status,
            ToolExecutionStatus::Error
        );
        assert_eq!(error_code(&outcome).as_deref(), Some("origin_not_authorized"));
    }

    #[test]
    fn model_origin_cannot_call_deferred_descriptor_without_elevation() {
        let registry = registry(vec![descriptor(
            "builtin:deferred_tool",
            ToolKind::Read,
            ToolExposure::Deferred,
        )]);
        let dispatcher = dispatcher_for(registry);
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:deferred_tool",
            "call-1",
            json!({ "text": "x" }),
        );
        assert_eq!(error_code(&outcome).as_deref(), Some("origin_not_authorized"));
    }

    #[test]
    fn host_origin_can_reach_internal_descriptor() {
        let registry = registry(vec![descriptor(
            "builtin:secret",
            ToolKind::Read,
            ToolExposure::Internal,
        )]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_handler("builtin:secret", echo_handler());
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Host,
            "builtin:secret",
            "call-1",
            json!({ "text": "x" }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
    }

    #[test]
    fn bare_child_origin_is_rejected_without_a_child_dispatch_context() {
        let registry = registry(vec![descriptor(
            "builtin:read",
            ToolKind::Read,
            ToolExposure::ModelVisible,
        )]);
        let dispatcher = dispatcher_for(registry);
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Child,
            "builtin:read",
            "call-1",
            json!({ "text": "x" }),
        );
        assert_eq!(error_code(&outcome).as_deref(), Some("origin_not_authorized"));
    }

    #[test]
    fn descriptor_resolves_by_alias_and_by_canonical_descriptor_id() {
        let registry = registry(vec![descriptor(
            "builtin:echo",
            ToolKind::Read,
            ToolExposure::ModelVisible,
        )]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_handler("builtin:echo", echo_handler());

        // Alias resolution.
        let by_alias = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "alias:builtin:echo",
            "call-1",
            json!({ "text": "x" }),
        );
        assert_eq!(by_alias.execution_status, ToolExecutionStatus::Ok);

        // Canonical descriptor-id resolution (registry aliases do not contain `builtin:`).
        let by_id = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:echo",
            "call-2",
            json!({ "text": "x" }),
        );
        assert_eq!(by_id.execution_status, ToolExecutionStatus::Ok);
    }

    // ── validation + hooks ───────────────────────────────────────────────────────────────────

    #[test]
    fn schema_validation_rejects_missing_required_argument() {
        let registry = registry(vec![descriptor(
            "builtin:echo",
            ToolKind::Read,
            ToolExposure::ModelVisible,
        )]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_handler("builtin:echo", echo_handler());
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:echo",
            "call-1",
            json!({}),
        );
        assert_eq!(error_code(&outcome).as_deref(), Some("invalid_arguments"));
    }

    #[test]
    fn pre_dispatch_hook_rewrites_arguments_before_the_handler() {
        struct UpperHook;
        impl PreDispatchHook for UpperHook {
            fn rewrite(
                &self,
                _descriptor_id: &str,
                _origin: &InvocationOrigin,
                _call_id: &str,
                mut arguments: Value,
            ) -> Result<Value, String> {
                if let Some(text) = arguments
                    .get_mut("text")
                    .and_then(|value| value.as_str())
                {
                    arguments["text"] = Value::String(text.to_uppercase());
                }
                Ok(arguments)
            }
        }

        let registry = registry(vec![descriptor(
            "builtin:echo",
            ToolKind::Read,
            ToolExposure::ModelVisible,
        )]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_handler("builtin:echo", echo_handler());
        dispatcher.register_pre_dispatch_hook(Arc::new(UpperHook));
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:echo",
            "call-1",
            json!({ "text": "hello" }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        assert!(
            outcome
                .result
                .as_ref()
                .map(|result| result.output.contains("HELLO"))
                .unwrap_or(false),
            "handler must receive the rewritten (uppercased) arguments"
        );
    }

    #[test]
    fn pre_dispatch_hook_producing_invalid_arguments_is_revalidated() {
        struct StripTextHook;
        impl PreDispatchHook for StripTextHook {
            fn rewrite(
                &self,
                _descriptor_id: &str,
                _origin: &InvocationOrigin,
                _call_id: &str,
                mut arguments: Value,
            ) -> Result<Value, String> {
                if let Some(object) = arguments.as_object_mut() {
                    object.remove("text");
                }
                Ok(arguments)
            }
        }

        let registry = registry(vec![descriptor(
            "builtin:echo",
            ToolKind::Read,
            ToolExposure::ModelVisible,
        )]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_handler("builtin:echo", echo_handler());
        dispatcher.register_pre_dispatch_hook(Arc::new(StripTextHook));
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:echo",
            "call-1",
            json!({ "text": "hello" }),
        );
        assert_eq!(error_code(&outcome).as_deref(), Some("invalid_arguments"));
    }

    // ── permission decisions ────────────────────────────────────────────────────────────────

    #[test]
    fn custom_policy_evaluator_can_deny_an_invocation() {
        struct DenyAll;
        impl ToolPolicyEvaluator for DenyAll {
            fn evaluate(
                &self,
                _descriptor: &ToolDescriptor,
                _origin: &InvocationOrigin,
                _final_arguments: &Value,
            ) -> PermissionDecision {
                PermissionDecision {
                    verdict: PermissionVerdict::Deny,
                    decision_source: "test-denier".to_string(),
                    reason: Some("policy says no".to_string()),
                }
            }
        }

        let registry = registry(vec![descriptor(
            "builtin:echo",
            ToolKind::Read,
            ToolExposure::ModelVisible,
        )]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_handler("builtin:echo", echo_handler());
        dispatcher.register_policy_evaluator(Arc::new(DenyAll));
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:echo",
            "call-1",
            json!({ "text": "x" }),
        );
        assert_eq!(error_code(&outcome).as_deref(), Some("permission_denied"));
    }

    #[test]
    fn approval_required_persists_a_pending_request_bound_to_invocation_facts() {
        let registry = registry(vec![descriptor_with_declaration(
            "builtin:approve_me",
            ToolKind::Write,
            ToolExposure::ModelVisible,
            approval_required_declaration(),
        )]);
        let dispatcher = dispatcher_for(registry);
        let context = DispatchContext {
            session_id: Some("session-1".to_string()),
            run_id: Some("run-1".to_string()),
            turn_id: Some("turn-1".to_string()),
            ..Default::default()
        };
        let outcome = dispatcher.dispatch_governed(
            ToolDispatchRequest {
                origin: InvocationOrigin::Model,
                descriptor_id: "builtin:approve_me".to_string(),
                call_id: "call-1".to_string(),
                arguments: json!({ "text": "x" }),
            },
            &context,
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        let control = outcome
            .control_outcome
            .expect("approval-required dispatch returns a control outcome");
        assert_eq!(control.kind, ToolControlKind::ApprovalRequired);

        let requests = dispatcher.pending_requests();
        assert_eq!(requests.len(), 1);
        let pending = &requests[0];
        assert_eq!(pending.request_id, control.request_id);
        assert_eq!(pending.session_id.as_deref(), Some("session-1"));
        assert_eq!(pending.turn_id, "turn-1");
        assert_eq!(pending.call_id, "call-1");
        assert_eq!(pending.descriptor_snapshot_id, "test-snapshot-1");
        assert_eq!(pending.descriptor_id, "builtin:approve_me");
        assert!(!pending.final_args_digest.is_empty());
        assert!(!pending.policy_digest.is_empty());
        assert_eq!(pending.version, 1);
        assert_eq!(pending.state, PendingControlRequestState::Pending);
    }

    #[test]
    fn host_mediated_descriptor_maps_to_waiting_host_control_outcome() {
        let mut declaration = ToolPermissionDeclaration::default();
        declaration.host_mediated = true;
        let registry = registry(vec![descriptor_with_declaration(
            "builtin:host_mediated",
            ToolKind::Interactive,
            ToolExposure::ModelVisible,
            declaration,
        )]);
        let dispatcher = dispatcher_for(registry);
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:host_mediated",
            "call-1",
            json!({ "text": "x" }),
        );
        let control = outcome
            .control_outcome
            .expect("host-mediated dispatch returns a control outcome");
        assert_eq!(control.kind, ToolControlKind::WaitingHost);
    }

    #[test]
    fn no_interactive_host_fails_closed_without_persisting() {
        let registry = registry(vec![descriptor_with_declaration(
            "builtin:approve_me",
            ToolKind::Write,
            ToolExposure::ModelVisible,
            approval_required_declaration(),
        )]);
        let dispatcher = dispatcher_for(registry);
        let context = DispatchContext {
            session_id: Some("session-1".to_string()),
            host_control_available: false,
            ..Default::default()
        };
        let outcome = dispatcher.dispatch_governed(
            ToolDispatchRequest {
                origin: InvocationOrigin::Model,
                descriptor_id: "builtin:approve_me".to_string(),
                call_id: "call-1".to_string(),
                arguments: json!({ "text": "x" }),
            },
            &context,
        );
        assert_eq!(error_code(&outcome).as_deref(), Some("interaction_unavailable"));
        assert!(dispatcher.pending_requests().is_empty());
    }

    // ── control-request consumption (CAS) ────────────────────────────────────────────────────

    fn approval_request(dispatcher: &GovernedDispatcher) -> (String, PendingControlRequest) {
        let context = DispatchContext {
            session_id: Some("session-1".to_string()),
            ..Default::default()
        };
        let outcome = dispatcher.dispatch_governed(
            ToolDispatchRequest {
                origin: InvocationOrigin::Model,
                descriptor_id: "builtin:approve_me".to_string(),
                call_id: "call-1".to_string(),
                arguments: json!({ "text": "x" }),
            },
            &context,
        );
        let request_id = outcome
            .control_outcome
            .expect("approval outcome")
            .request_id;
        let pending = dispatcher
            .pending_request(&request_id)
            .expect("persisted pending request");
        (request_id, pending)
    }

    fn valid_authorization(pending: &PendingControlRequest) -> ControlRequestAuthorization {
        ControlRequestAuthorization {
            session_id: pending.session_id.clone(),
            expected_version: pending.version,
            nonce: pending.nonce.clone(),
            expected_descriptor_snapshot_id: Some(pending.descriptor_snapshot_id.clone()),
            expected_descriptor_id: Some(pending.descriptor_id.clone()),
            expected_final_args_digest: Some(pending.final_args_digest.clone()),
            expected_policy_digest: Some(pending.policy_digest.clone()),
            answer: None,
        }
    }

    #[test]
    fn approval_consumes_once_and_rejects_replay() {
        let registry = registry(vec![descriptor_with_declaration(
            "builtin:approve_me",
            ToolKind::Write,
            ToolExposure::ModelVisible,
            approval_required_declaration(),
        )]);
        let dispatcher = dispatcher_for(registry);
        let (request_id, pending) = approval_request(&dispatcher);
        let authorization = valid_authorization(&pending);

        let consumed = dispatcher
            .approve_control_request(&request_id, &authorization)
            .expect("first approval consumes the request");
        assert_eq!(consumed.request.state, PendingControlRequestState::Consumed);
        assert_eq!(
            dispatcher.pending_request(&request_id).expect("stored").state,
            PendingControlRequestState::Consumed
        );

        assert!(
            dispatcher
                .approve_control_request(&request_id, &authorization)
                .is_err(),
            "replaying the same authorization must be rejected"
        );
    }

    #[test]
    fn control_request_rejects_wrong_session_version_nonce_and_expiry() {
        let registry = registry(vec![descriptor_with_declaration(
            "builtin:approve_me",
            ToolKind::Write,
            ToolExposure::ModelVisible,
            approval_required_declaration(),
        )]);
        let clock = Arc::new(FakeClock::new(1_000));
        let dispatcher = GovernedDispatcher::new(registry, Arc::clone(&clock) as Arc<dyn RuntimeClock>);
        let (request_id, pending) = approval_request(&dispatcher);

        let wrong_session = {
            let mut authorization = valid_authorization(&pending);
            authorization.session_id = Some("session-2".to_string());
            authorization
        };
        assert!(dispatcher.approve_control_request(&request_id, &wrong_session).is_err());

        let wrong_version = {
            let mut authorization = valid_authorization(&pending);
            authorization.expected_version = pending.version + 1;
            authorization
        };
        assert!(dispatcher.approve_control_request(&request_id, &wrong_version).is_err());

        let wrong_nonce = {
            let mut authorization = valid_authorization(&pending);
            authorization.nonce = "wrong-nonce".to_string();
            authorization
        };
        assert!(dispatcher.approve_control_request(&request_id, &wrong_nonce).is_err());

        // Expiry: advance the clock past `expires_at_ms`, then both approve and expire fail.
        clock.set_now_ms(pending.expires_at_ms + 1);
        assert!(dispatcher.approve_control_request(&request_id, &valid_authorization(&pending)).is_err());
        assert_eq!(dispatcher.expire_control_requests(pending.expires_at_ms + 1), 1);
        assert_eq!(
            dispatcher.pending_request(&request_id).expect("stored").state,
            PendingControlRequestState::Expired
        );
    }

    #[test]
    fn interaction_answer_on_approval_request_is_kind_rejected() {
        let registry = registry(vec![descriptor_with_declaration(
            "builtin:approve_me",
            ToolKind::Write,
            ToolExposure::ModelVisible,
            approval_required_declaration(),
        )]);
        let dispatcher = dispatcher_for(registry);
        let (request_id, pending) = approval_request(&dispatcher);
        let authorization = valid_authorization(&pending);
        assert!(
            dispatcher
                .answer_control_request(&request_id, &authorization)
                .is_err(),
            "answering (Interaction kind) must be rejected for an Approval request"
        );
    }

    #[test]
    fn cancel_control_request_transitions_state_and_cannot_be_replayed() {
        let registry = registry(vec![descriptor_with_declaration(
            "builtin:approve_me",
            ToolKind::Write,
            ToolExposure::ModelVisible,
            approval_required_declaration(),
        )]);
        let dispatcher = dispatcher_for(registry);
        let (request_id, pending) = approval_request(&dispatcher);
        let authorization = valid_authorization(&pending);
        let consumed = dispatcher
            .cancel_control_request(&request_id, &authorization)
            .expect("cancel consumes the request");
        assert_eq!(consumed.request.state, PendingControlRequestState::Cancelled);
        assert!(dispatcher.cancel_control_request(&request_id, &authorization).is_err());
    }

    #[test]
    fn control_request_rejects_final_args_digest_mismatch() {
        let registry = registry(vec![descriptor_with_declaration(
            "builtin:approve_me",
            ToolKind::Write,
            ToolExposure::ModelVisible,
            approval_required_declaration(),
        )]);
        let dispatcher = dispatcher_for(registry);
        let (request_id, pending) = approval_request(&dispatcher);
        let mut authorization = valid_authorization(&pending);
        authorization.expected_final_args_digest = Some("different-digest".to_string());
        assert!(
            dispatcher.approve_control_request(&request_id, &authorization).is_err(),
            "a resumed call whose arguments no longer match the approved digest must fail closed"
        );

        let mut policy_mismatch = valid_authorization(&pending);
        policy_mismatch.expected_policy_digest = Some("different-policy".to_string());
        assert!(
            dispatcher.approve_control_request(&request_id, &policy_mismatch).is_err(),
            "policy digest mismatch must fail closed"
        );
    }

    // ── lifecycle observability ──────────────────────────────────────────────────────────────

    #[test]
    fn lifecycle_observer_receives_exactly_one_record_per_top_level_dispatch() {
        struct RecordingObserver(Mutex<Vec<DispatchLifecycleRecord>>);
        impl DispatchLifecycleObserver for RecordingObserver {
            fn record(&self, event: &DispatchLifecycleRecord) {
                self.0.lock().expect("observer lock poisoned").push(event.clone());
            }
        }

        let registry = registry(vec![descriptor(
            "builtin:echo",
            ToolKind::Read,
            ToolExposure::ModelVisible,
        )]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_handler("builtin:echo", echo_handler());
        let observer = Arc::new(RecordingObserver(Mutex::new(Vec::new())));
        dispatcher.register_lifecycle_observer(Arc::clone(&observer) as Arc<dyn DispatchLifecycleObserver>);

        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:echo",
            "call-1",
            json!({ "text": "x" }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);

        let records = observer.0.lock().expect("observer lock poisoned");
        assert_eq!(records.len(), 1, "exactly one lifecycle record per top-level dispatch");
        assert_eq!(records[0].descriptor_id, "builtin:echo");
        assert_eq!(records[0].call_id, "call-1");
        assert_eq!(records[0].depth, 0);
        assert_eq!(records[0].phase, DispatchLifecyclePhase::Completed);
        assert_eq!(records[0].decision_source.as_deref(), Some("descriptor_declaration"));
    }

    // ── composite / child dispatch ───────────────────────────────────────────────────────────

    struct DelegatingComposite {
        child: String,
    }

    impl CompositeToolHandler for DelegatingComposite {
        fn execute(
            &self,
            _request: &CompositeToolHandlerRequest,
            children: &ChildDispatch,
        ) -> Result<Value, String> {
            let outcome = children.dispatch(ChildDispatchRequest {
                descriptor_id: self.child.clone(),
                call_id: "child-1".to_string(),
                arguments: json!({ "text": "child" }),
            })?;
            Ok(json!({
                "status": outcome.execution_status.as_legacy_status(),
                "code": outcome
                    .result
                    .as_ref()
                    .and_then(|result| serde_json::from_str::<Value>(&result.output).ok())
                    .and_then(|value| {
                        value
                            .get("error")
                            .and_then(|error| error.get("code"))
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    }),
            }))
        }
    }

    #[test]
    fn composite_child_rejects_self_recursion() {
        let registry = registry(vec![descriptor(
            "builtin:composite",
            ToolKind::Composite,
            ToolExposure::ModelVisible,
        )]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_composite_handler_with_authority(
            "builtin:composite",
            vec!["builtin:composite".to_string()],
            Arc::new(DelegatingComposite {
                child: "builtin:composite".to_string(),
            }),
        );
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:composite",
            "call-1",
            json!({ "text": "x" }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        let payload: Value = outcome
            .result
            .and_then(|result| serde_json::from_str(&result.output).ok())
            .expect("composite result json");
        assert_eq!(payload["status"], "error");
        // The parent composite is always present in its own dispatch lineage, so self-recursion
        // is folded into the lineage cycle check before the dedicated self-recursion branch.
        assert_eq!(payload["code"], "child_cycle");
    }

    #[test]
    fn composite_child_within_allowed_authority_executes() {
        let registry = registry(vec![
            descriptor("builtin:composite", ToolKind::Composite, ToolExposure::ModelVisible),
            descriptor("builtin:leaf", ToolKind::Read, ToolExposure::Internal),
        ]);
        let dispatcher = dispatcher_for(registry);
        // Authority allows only `builtin:leaf`; the handler dispatches `builtin:leaf`.
        dispatcher.register_composite_handler_with_authority(
            "builtin:composite",
            vec!["builtin:leaf".to_string()],
            Arc::new(DelegatingComposite {
                child: "builtin:leaf".to_string(),
            }),
        );
        dispatcher.register_handler("builtin:leaf", echo_handler());
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:composite",
            "call-1",
            json!({ "text": "x" }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        let payload: Value = outcome
            .result
            .and_then(|result| serde_json::from_str(&result.output).ok())
            .expect("composite result json");
        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["code"], Value::Null);
    }

    #[test]
    fn composite_child_rejects_descriptor_outside_allowed_authority() {
        let registry = registry(vec![
            descriptor("builtin:composite", ToolKind::Composite, ToolExposure::ModelVisible),
            descriptor("builtin:leaf", ToolKind::Read, ToolExposure::Internal),
            descriptor("builtin:other", ToolKind::Read, ToolExposure::Internal),
        ]);
        let dispatcher = dispatcher_for(registry);
        // Authority allows only `builtin:leaf`, but the handler dispatches `builtin:other`.
        dispatcher.register_composite_handler_with_authority(
            "builtin:composite",
            vec!["builtin:leaf".to_string()],
            Arc::new(DelegatingComposite {
                child: "builtin:other".to_string(),
            }),
        );
        dispatcher.register_handler("builtin:other", echo_handler());
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:composite",
            "call-1",
            json!({ "text": "x" }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        let payload: Value = outcome
            .result
            .and_then(|result| serde_json::from_str(&result.output).ok())
            .expect("composite result json");
        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "child_not_allowed");
    }

    #[test]
    fn child_dispatch_enforces_max_depth() {
        let registry = registry(vec![
            descriptor("builtin:composite", ToolKind::Composite, ToolExposure::ModelVisible),
            descriptor("builtin:leaf", ToolKind::Read, ToolExposure::Internal),
        ]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.set_budget_config(DispatchBudgetConfig {
            max_composite_depth: 0,
            ..Default::default()
        });
        dispatcher.register_composite_handler_with_authority(
            "builtin:composite",
            vec!["builtin:leaf".to_string()],
            Arc::new(DelegatingComposite {
                child: "builtin:leaf".to_string(),
            }),
        );
        dispatcher.register_handler("builtin:leaf", echo_handler());
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:composite",
            "call-1",
            json!({ "text": "x" }),
        );
        let payload: Value = outcome
            .result
            .and_then(|result| serde_json::from_str(&result.output).ok())
            .expect("composite result json");
        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "child_depth_exceeded");
    }

    #[test]
    fn child_dispatch_cycle_is_rejected_across_two_composites() {
        struct DispatchOther {
            child: String,
        }
        impl CompositeToolHandler for DispatchOther {
            fn execute(
                &self,
                _request: &CompositeToolHandlerRequest,
                children: &ChildDispatch,
            ) -> Result<Value, String> {
                children
                    .dispatch(ChildDispatchRequest {
                        descriptor_id: self.child.clone(),
                        call_id: "child-1".to_string(),
                        arguments: json!({}),
                    })
                    .map(|outcome| {
                        json!({ "status": outcome.execution_status.as_legacy_status() })
                    })
            }
        }

        let registry = registry(vec![
            descriptor("builtin:a", ToolKind::Composite, ToolExposure::ModelVisible),
            descriptor("builtin:b", ToolKind::Composite, ToolExposure::ModelVisible),
        ]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_composite_handler_with_authority(
            "builtin:a",
            vec!["builtin:b".to_string()],
            Arc::new(DispatchOther { child: "builtin:b".to_string() }),
        );
        dispatcher.register_composite_handler_with_authority(
            "builtin:b",
            vec!["builtin:a".to_string()],
            Arc::new(DispatchOther { child: "builtin:a".to_string() }),
        );
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:a",
            "call-1",
            json!({ "text": "x" }),
        );
        // A -> B -> A is a lineage cycle; the final A child fails closed.
        let payload: Value = outcome
            .result
            .and_then(|result| serde_json::from_str(&result.output).ok())
            .expect("composite result json");
        assert_eq!(payload["status"], "error");
    }

    #[test]
    fn child_budget_exhaustion_fails_closed() {
        struct ManyChildren {
            count: u64,
        }
        impl CompositeToolHandler for ManyChildren {
            fn execute(
                &self,
                _request: &CompositeToolHandlerRequest,
                children: &ChildDispatch,
            ) -> Result<Value, String> {
                let mut results = Vec::new();
                for index in 0..self.count {
                    let outcome = children.dispatch(ChildDispatchRequest {
                        descriptor_id: "builtin:leaf".to_string(),
                        call_id: format!("child-{index}"),
                        arguments: json!({ "text": "child" }),
                    })?;
                    results.push(outcome.execution_status.as_legacy_status().to_string());
                }
                Ok(json!({ "results": results }))
            }
        }

        let registry = registry(vec![
            descriptor("builtin:composite", ToolKind::Composite, ToolExposure::ModelVisible),
            descriptor("builtin:leaf", ToolKind::Read, ToolExposure::Internal),
        ]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.set_budget_config(DispatchBudgetConfig {
            max_composite_calls: 1,
            ..Default::default()
        });
        dispatcher.register_composite_handler_with_authority(
            "builtin:composite",
            vec!["builtin:leaf".to_string()],
            Arc::new(ManyChildren { count: 2 }),
        );
        dispatcher.register_handler("builtin:leaf", echo_handler());
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:composite",
            "call-1",
            json!({ "text": "x" }),
        );
        let payload: Value = outcome
            .result
            .and_then(|result| serde_json::from_str(&result.output).ok())
            .expect("composite result json");
        let results = payload["results"].as_array().expect("results array");
        assert_eq!(results[0], "ok");
        assert_eq!(results[1], "error");
    }

    #[test]
    fn child_cancellation_fails_closed_as_cancelled() {
        struct CancelThenDispatch;
        impl CompositeToolHandler for CancelThenDispatch {
            fn execute(
                &self,
                _request: &CompositeToolHandlerRequest,
                children: &ChildDispatch,
            ) -> Result<Value, String> {
                children.cancel();
                let outcome = children.dispatch(ChildDispatchRequest {
                    descriptor_id: "builtin:leaf".to_string(),
                    call_id: "child-1".to_string(),
                    arguments: json!({ "text": "child" }),
                })?;
                Ok(json!({
                    "status": outcome.execution_status.as_legacy_status(),
                }))
            }
        }

        let registry = registry(vec![
            descriptor("builtin:composite", ToolKind::Composite, ToolExposure::ModelVisible),
            descriptor("builtin:leaf", ToolKind::Read, ToolExposure::Internal),
        ]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_composite_handler_with_authority(
            "builtin:composite",
            vec!["builtin:leaf".to_string()],
            Arc::new(CancelThenDispatch),
        );
        dispatcher.register_handler("builtin:leaf", echo_handler());
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:composite",
            "call-1",
            json!({ "text": "x" }),
        );
        let payload: Value = outcome
            .result
            .and_then(|result| serde_json::from_str(&result.output).ok())
            .expect("composite result json");
        assert_eq!(payload["status"], "aborted");
    }

    #[test]
    fn dispatch_many_stops_unstarted_siblings_after_a_pending_control_child() {
        struct BatchWithApproval;
        impl CompositeToolHandler for BatchWithApproval {
            fn execute(
                &self,
                _request: &CompositeToolHandlerRequest,
                children: &ChildDispatch,
            ) -> Result<Value, String> {
                let results = children.dispatch_many(vec![
                    ChildDispatchRequest {
                        descriptor_id: "builtin:approve_me".to_string(),
                        call_id: "child-0".to_string(),
                        arguments: json!({ "text": "approve" }),
                    },
                    ChildDispatchRequest {
                        descriptor_id: "builtin:leaf".to_string(),
                        call_id: "child-1".to_string(),
                        arguments: json!({ "text": "leaf" }),
                    },
                ])?;
                Ok(json!({
                    "started": results.iter().map(|result| result.started).collect::<Vec<_>>(),
                    "statuses": results.iter().map(|result| result.outcome.execution_status.as_legacy_status()).collect::<Vec<_>>(),
                }))
            }
        }

        let mut approval = ToolPermissionDeclaration::default();
        approval.requires_approval = true;
        let registry = registry(vec![
            descriptor_with_declaration(
                "builtin:approve_me",
                ToolKind::Write,
                ToolExposure::ModelVisible,
                approval,
            ),
            descriptor("builtin:composite", ToolKind::Composite, ToolExposure::ModelVisible),
            descriptor("builtin:leaf", ToolKind::Read, ToolExposure::Internal),
        ]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_composite_handler_with_authority(
            "builtin:composite",
            vec!["builtin:approve_me".to_string(), "builtin:leaf".to_string()],
            Arc::new(BatchWithApproval),
        );
        dispatcher.register_handler("builtin:leaf", echo_handler());
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:composite",
            "call-1",
            json!({ "text": "x" }),
        );
        let payload: Value = outcome
            .result
            .and_then(|result| serde_json::from_str(&result.output).ok())
            .expect("composite result json");
        // First child entered a pending control state, so the second sibling never started.
        assert_eq!(payload["started"], json!([false, false]));
        assert_eq!(payload["statuses"][0], "ok");
        assert_eq!(payload["statuses"][1], "aborted");
    }

    #[test]
    fn execute_descriptor_without_sandbox_backend_fails_closed() {
        let registry = registry(vec![descriptor(
            "builtin:exec",
            ToolKind::Execute,
            ToolExposure::ModelVisible,
        )]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_handler("builtin:exec", echo_handler());
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:exec",
            "call-1",
            json!({ "text": "x" }),
        );
        // Execute scope + no registered sandbox backend => fail closed (phase-3 review P1-1),
        // never silently run unsandboxed.
        assert_eq!(error_code(&outcome).as_deref(), Some("sandbox_unavailable"));
    }

    #[test]
    fn single_child_dispatch_stops_siblings_after_a_pending_control_child() {
        struct SequentialWithApproval;
        impl CompositeToolHandler for SequentialWithApproval {
            fn execute(
                &self,
                _request: &CompositeToolHandlerRequest,
                children: &ChildDispatch,
            ) -> Result<Value, String> {
                let first = children.dispatch(ChildDispatchRequest {
                    descriptor_id: "builtin:approve_me".to_string(),
                    call_id: "child-0".to_string(),
                    arguments: json!({ "text": "approve" }),
                })?;
                let second = children.dispatch(ChildDispatchRequest {
                    descriptor_id: "builtin:leaf".to_string(),
                    call_id: "child-1".to_string(),
                    arguments: json!({ "text": "leaf" }),
                })?;
                Ok(json!({
                    "first": first.execution_status.as_legacy_status(),
                    "second": second.execution_status.as_legacy_status(),
                }))
            }
        }

        let mut approval = ToolPermissionDeclaration::default();
        approval.requires_approval = true;
        let registry = registry(vec![
            descriptor_with_declaration(
                "builtin:approve_me",
                ToolKind::Write,
                ToolExposure::ModelVisible,
                approval,
            ),
            descriptor("builtin:composite", ToolKind::Composite, ToolExposure::ModelVisible),
            descriptor("builtin:leaf", ToolKind::Read, ToolExposure::Internal),
        ]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_composite_handler_with_authority(
            "builtin:composite",
            vec![
                "builtin:approve_me".to_string(),
                "builtin:leaf".to_string(),
            ],
            Arc::new(SequentialWithApproval),
        );
        dispatcher.register_handler("builtin:leaf", echo_handler());
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "builtin:composite",
            "call-1",
            json!({ "text": "x" }),
        );
        let payload: Value = outcome
            .result
            .and_then(|result| serde_json::from_str(&result.output).ok())
            .expect("composite result json");
        // The pending approval suspends the context; the second single-dispatch sibling never
        // starts (phase-3 review P1-3).
        assert_eq!(payload["first"], "ok");
        assert_eq!(payload["second"], "aborted");
    }

    #[test]
    fn workspace_scope_and_final_args_are_bound_into_the_pending_request_digests() {
        let mut declaration = approval_required_declaration();
        declaration.scopes.insert(ToolPermissionScope::WorkspaceWrite);
        let registry = registry(vec![descriptor_with_declaration(
            "builtin:writer",
            ToolKind::Write,
            ToolExposure::ModelVisible,
            declaration,
        )]);
        let dispatcher = dispatcher_for(registry);
        let context = DispatchContext {
            session_id: Some("session-1".to_string()),
            workspace_root: Some("C:\\workspace".to_string()),
            ..Default::default()
        };
        let outcome = dispatcher.dispatch_governed(
            ToolDispatchRequest {
                origin: InvocationOrigin::Model,
                descriptor_id: "builtin:writer".to_string(),
                call_id: "call-1".to_string(),
                arguments: json!({ "text": "x" }),
            },
            &context,
        );
        assert_eq!(
            outcome.control_outcome.as_ref().map(|control| control.kind.clone()),
            Some(ToolControlKind::ApprovalRequired)
        );
        let pending = &dispatcher.pending_requests()[0];
        assert!(!pending.policy_digest.is_empty());
        assert_eq!(pending.final_args_digest, stable_digest(&json!({ "text": "x" })));
    }

    // ── composite permission aggregation (3.6) ───────────────────────────────────────────────

    #[test]
    fn aggregate_composite_permission_unions_scopes_and_lifts_to_strongest_approval() {
        let decisions = vec![
            PermissionDecision {
                verdict: PermissionVerdict::Allow,
                decision_source: "descriptor_declaration".to_string(),
                reason: None,
            },
            PermissionDecision {
                verdict: PermissionVerdict::ApprovalRequired,
                decision_source: "descriptor_declaration".to_string(),
                reason: None,
            },
            PermissionDecision {
                verdict: PermissionVerdict::WaitingHost,
                decision_source: "host_policy".to_string(),
                reason: None,
            },
        ];
        let summary = aggregate_composite_permission(
            [
                ToolPermissionScope::WorkspaceRead,
                ToolPermissionScope::CapabilityDiscovery,
            ],
            &decisions,
        );
        assert_eq!(
            summary.scopes,
            BTreeSet::from([
                ToolPermissionScope::WorkspaceRead,
                ToolPermissionScope::CapabilityDiscovery,
            ])
        );
        assert!(summary.requires_approval, "any approval-required child lifts the aggregate");
        assert!(summary.host_mediated, "any waiting-host child lifts the aggregate");
        assert_eq!(summary.child_decisions.len(), 3, "per-child evidence is preserved");
    }

    #[test]
    fn unbounded_legacy_budget_config_does_not_cap_output_bytes_or_deadline() {
        struct BigOutput;
        impl PrimitiveToolHandler for BigOutput {
            fn execute(&self, _request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
                Ok(Value::String("x".repeat(200 * 1024)))
            }
        }
        let registry = registry(vec![descriptor(
            "builtin:big",
            ToolKind::Read,
            ToolExposure::ModelVisible,
        )]);
        // Legacy-compatible mode (runtime switch default) must NOT cap output bytes or deadline.
        let unbounded = dispatcher_for(registry.clone());
        unbounded.register_handler("builtin:big", Arc::new(BigOutput));
        unbounded.set_budget_config(DispatchBudgetConfig {
            unbounded_output_and_deadline: true,
            ..Default::default()
        });
        let ok = dispatch(
            &unbounded,
            InvocationOrigin::Model,
            "builtin:big",
            "call-1",
            json!({ "text": "x" }),
        );
        assert_eq!(
            ok.execution_status,
            ToolExecutionStatus::Ok,
            "legacy-compatible mode must not apply the descriptor's 120 KiB result budget"
        );

        // Without the flag, the descriptor's result budget applies and fails closed.
        let capped = dispatcher_for(registry);
        capped.register_handler("builtin:big", Arc::new(BigOutput));
        let rejected = dispatch(
            &capped,
            InvocationOrigin::Model,
            "builtin:big",
            "call-2",
            json!({ "text": "x" }),
        );
        assert_eq!(
            error_code(&rejected).as_deref(),
            Some("output_budget_exceeded")
        );
    }

    #[test]
    fn handler_failure_preserves_arbitrary_structured_codes() {
        // Arbitrary bridged-handler codes survive verbatim (not collapsing to `handler_error`).
        let error = handler_failure("not_found: 文件不存在".to_string());
        assert_eq!(error.code, "not_found");
        assert_eq!(error.message, "文件不存在");
        assert!(!error.cancelled);

        // Non-structured / colon-free messages fall back to the generic code.
        let plain = handler_failure("some plain error".to_string());
        assert_eq!(plain.code, "handler_error");

        // Empty or non-identifier code prefix is not lifted.
        let colon_only = handler_failure(": nothing before the colon".to_string());
        assert_eq!(colon_only.code, "handler_error");
    }

    #[test]
    fn aggregate_composite_permission_is_not_weakened_by_allowed_or_denied_children() {
        let decisions = vec![
            PermissionDecision {
                verdict: PermissionVerdict::Allow,
                decision_source: "descriptor_declaration".to_string(),
                reason: None,
            },
            PermissionDecision {
                verdict: PermissionVerdict::Deny,
                decision_source: "test-denier".to_string(),
                reason: Some("no".to_string()),
            },
        ];
        let summary = aggregate_composite_permission(std::iter::empty(), &decisions);
        assert!(!summary.requires_approval);
        assert!(!summary.host_mediated);
        assert_eq!(summary.child_decisions.len(), 2);
        assert_eq!(summary.child_decisions[1].verdict, PermissionVerdict::Deny);
    }

    // ── T2-A：startLine 硬门契约（只加测试，不改逻辑） ─────────────────────────────
    // integer 分支只看类型：浮点/字符串/bool 在此直接拒（`invalid_arguments`）；
    // 负整数/零（`as_i64` 成功）放行进入 parser，由 `parse_start_line_arg` 判
    // `Invalid`（负数）或 `Valid(1)`（零 clamp）并留痕/回退。
    #[test]
    fn integer_gate_rejects_float_string_bool_but_passes_negative_and_zero() {
        let schema = json!({
            "type": "object",
            "properties": { "startLine": { "type": "integer" } },
        });
        for rejected in [
            json!({ "startLine": 80.5 }),
            json!({ "startLine": "80" }),
            json!({ "startLine": true }),
        ] {
            let error = validate_against_schema(&rejected, &schema, "$", 0)
                .expect_err("float/string/bool startLine must be rejected at the gate");
            assert!(
                error.contains("must be an integer"),
                "unexpected gate message: {error}"
            );
        }
        for passed in [json!({ "startLine": -5 }), json!({ "startLine": 0 })] {
            validate_against_schema(&passed, &schema, "$", 0)
                .expect("negative/zero integers must pass the gate into the parser");
        }
    }

    // ── ADR 0065：工具面原子换入（swap_tool_surface）与外部工具通道（ToolProvider） ──────────

    /// An `mcp__demo__echo`-shaped external descriptor (source `demo`, reserved-alias free).
    fn external_descriptor(tool: &str, exposure: ToolExposure) -> ToolDescriptor {
        let descriptor_id = format!("mcp:demo:{tool}");
        let model_name = format!("mcp__demo__{tool}");
        ToolDescriptor {
            identity: ToolIdentity {
                descriptor_id: descriptor_id.clone(),
                model_name: model_name.clone(),
                canonical_name: model_name.clone(),
                primitive_name: model_name.clone(),
                source: ToolDescriptorSource::Mcp,
            },
            aliases: vec![descriptor_id, model_name],
            description: "external test tool".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": { "text": { "type": "string" } },
                "required": ["text"],
                "additionalProperties": false,
            }),
            kind: ToolKind::External,
            exposure,
            permission_declaration: ToolPermissionDeclaration::default(),
            execution_policy: ToolExecutionPolicy::default(),
            display_metadata: ToolDisplayMetadata::default(),
            handler_provenance: ToolHandlerProvenance {
                handler_kind: "external".to_string(),
                source_id: "demo".to_string(),
            },
            source_revision: "demo-v1".to_string(),
            composed_descriptor_ids: Vec::new(),
        }
    }

    struct StubProvider {
        source_id: String,
        descriptors: Vec<ToolDescriptor>,
        result: Result<Value, ToolProviderError>,
    }

    impl StubProvider {
        fn new(descriptors: Vec<ToolDescriptor>, result: Result<Value, ToolProviderError>) -> Self {
            Self {
                source_id: "demo".to_string(),
                descriptors,
                result,
            }
        }
    }

    impl ToolProvider for StubProvider {
        fn source_id(&self) -> &str {
            &self.source_id
        }

        fn list_tools(&self) -> Vec<ToolDescriptor> {
            self.descriptors.clone()
        }

        fn call(
            &self,
            _descriptor_id: &str,
            _arguments: Value,
        ) -> Result<Value, ToolProviderError> {
            self.result.clone()
        }
    }

    #[test]
    fn swap_tool_surface_publishes_registry_and_handlers_as_one_unit() {
        let first = registry(vec![descriptor(
            "builtin:first",
            ToolKind::Read,
            ToolExposure::ModelVisible,
        )]);
        let dispatcher = dispatcher_for(Arc::clone(&first));
        dispatcher.register_handler("builtin:first", echo_handler());
        assert_eq!(
            dispatch(
                &dispatcher,
                InvocationOrigin::Model,
                "builtin:first",
                "call-1",
                json!({ "text": "one" })
            )
            .execution_status,
            ToolExecutionStatus::Ok
        );

        // Swap to a generation that no longer contains `builtin:first` and whose handler table
        // only covers `builtin:second`: the old handler must not survive the swap.
        let second = ToolRegistrySnapshot::from_descriptors(
            "test-snapshot-2",
            vec![descriptor(
                "builtin:second",
                ToolKind::Read,
                ToolExposure::ModelVisible,
            )],
        )
        .expect("second snapshot must build");
        dispatcher
            .swap_tool_surface(
                "test-snapshot-2",
                second,
                vec![("builtin:second".to_string(), echo_handler())],
            )
            .expect("consistent surface must swap");

        assert_eq!(dispatcher.registry().snapshot_id, "test-snapshot-2");
        assert_eq!(dispatcher.turn_view().snapshot_id, "test-snapshot-2");
        assert_eq!(
            error_code(&dispatch(
                &dispatcher,
                InvocationOrigin::Model,
                "builtin:first",
                "call-2",
                json!({ "text": "one" })
            ))
            .as_deref(),
            Some("unknown_descriptor"),
            "a descriptor from the replaced generation must disappear with its handler table"
        );
        assert_eq!(
            dispatch(
                &dispatcher,
                InvocationOrigin::Model,
                "builtin:second",
                "call-3",
                json!({ "text": "two" })
            )
            .execution_status,
            ToolExecutionStatus::Ok
        );

        // Swapping in a descriptor without a handler is rejected before publishing: the torn
        // "visible but unexecutable" state is exactly what the atomic swap must make impossible,
        // and a rejected swap leaves the previous surface intact.
        let third = ToolRegistrySnapshot::from_descriptors(
            "test-snapshot-3",
            vec![
                descriptor("builtin:second", ToolKind::Read, ToolExposure::ModelVisible),
                descriptor("builtin:third", ToolKind::Read, ToolExposure::ModelVisible),
            ],
        )
        .expect("third snapshot must build");
        assert!(
            dispatcher
                .swap_tool_surface(
                    "test-snapshot-3",
                    third.clone(),
                    vec![("builtin:second".to_string(), echo_handler())],
                )
                .is_err(),
            "a visible descriptor without a handler must reject the swap"
        );
        assert_eq!(
            dispatcher.registry().snapshot_id,
            "test-snapshot-2",
            "a rejected swap must not publish a partial surface"
        );

        // The same rules reject a handler table that names a descriptor the snapshot lacks.
        assert!(dispatcher
            .swap_tool_surface(
                "test-snapshot-3",
                third.clone(),
                vec![
                    ("builtin:second".to_string(), echo_handler()),
                    ("builtin:third".to_string(), echo_handler()),
                    ("builtin:ghost".to_string(), echo_handler()),
                ],
            )
            .is_err());
        assert_eq!(dispatcher.registry().snapshot_id, "test-snapshot-2");

        // The caller's snapshot id is re-checked against the snapshot it hands over.
        let mismatched = ToolRegistrySnapshot::from_descriptors(
            "test-snapshot-4",
            vec![descriptor(
                "builtin:fourth",
                ToolKind::Read,
                ToolExposure::ModelVisible,
            )],
        )
        .expect("fourth snapshot must build");
        assert!(dispatcher
            .swap_tool_surface("test-snapshot-5", mismatched, Vec::new())
            .is_err());
        assert_eq!(dispatcher.registry().snapshot_id, "test-snapshot-2");

        // A consistent generation still swaps, and a descriptor without a handler that is simply
        // absent from the surface keeps failing closed at dispatch (registry never fabricates one).
        dispatcher
            .swap_tool_surface(
                "test-snapshot-3",
                third,
                vec![
                    ("builtin:second".to_string(), echo_handler()),
                    ("builtin:third".to_string(), echo_handler()),
                ],
            )
            .expect("a complete generation must swap");
        assert_eq!(
            dispatch(
                &dispatcher,
                InvocationOrigin::Model,
                "builtin:third",
                "call-5",
                json!({ "text": "three" })
            )
            .execution_status,
            ToolExecutionStatus::Ok
        );
    }

    #[test]
    fn register_tool_provider_runs_external_descriptors_through_the_governed_pipeline() {
        let dispatcher = dispatcher_for(registry(vec![descriptor(
            "builtin:first",
            ToolKind::Read,
            ToolExposure::ModelVisible,
        )]));
        dispatcher.register_handler("builtin:first", echo_handler());

        let provider = Arc::new(StubProvider::new(
            vec![external_descriptor("echo", ToolExposure::ModelVisible)],
            Ok(json!({ "external": "ok" })),
        ));
        dispatcher
            .register_tool_provider(provider)
            .expect("valid external descriptors must register");

        // The external descriptor is model-visible and executable in the same generation.
        assert_eq!(
            dispatcher.registry().snapshot_id,
            "test-snapshot-1::demo",
            "registering a source re-versions the snapshot"
        );
        assert!(dispatcher
            .registry()
            .resolve("mcp__demo__echo")
            .is_some());
        let outcome = dispatch(
            &dispatcher,
            InvocationOrigin::Model,
            "mcp__demo__echo",
            "call-1",
            json!({ "text": "hi" }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        assert!(
            outcome
                .result
                .as_ref()
                .expect("ok outcome carries a result")
                .output
                .contains("external"),
            "the provider result must be returned verbatim"
        );
        // Builtin handlers survive a provider registration.
        assert_eq!(
            dispatch(
                &dispatcher,
                InvocationOrigin::Model,
                "builtin:first",
                "call-2",
                json!({ "text": "one" })
            )
            .execution_status,
            ToolExecutionStatus::Ok
        );

        // A structured provider failure keeps its source-specific error code.
        let failing = Arc::new(StubProvider::new(
            vec![external_descriptor("flaky", ToolExposure::ModelVisible)],
            Err(ToolProviderError::new(
                "mcp_timeout",
                "server did not answer in time",
            )),
        ));
        dispatcher
            .register_tool_provider(failing)
            .expect("a second source registration must succeed");
        assert_eq!(
            error_code(&dispatch(
                &dispatcher,
                InvocationOrigin::Model,
                "mcp__demo__flaky",
                "call-3",
                json!({ "text": "hi" })
            ))
            .as_deref(),
            Some("mcp_timeout"),
            "external failures must stay distinguishable per source"
        );

        // An external provider cannot smuggle in a builtin descriptor.
        let mut smuggled = external_descriptor("smuggle", ToolExposure::ModelVisible);
        smuggled.identity = ToolIdentity {
            descriptor_id: "builtin:smuggle".to_string(),
            model_name: "smuggle".to_string(),
            canonical_name: "smuggle".to_string(),
            primitive_name: "smuggle".to_string(),
            source: ToolDescriptorSource::Builtin,
        };
        let bad = Arc::new(StubProvider::new(vec![smuggled], Ok(json!({}))));
        assert!(
            dispatcher.register_tool_provider(bad).is_err(),
            "external providers must not register builtin descriptors"
        );
    }
}

/// Project a live `PendingControlRequest` onto the durable Ask record (ADR 0074).
///
/// `created_at_ms` is the dispatch clock's wall clock, used for display/ordering only — the record
/// deliberately carries no deadline.
fn persisted_ask_from_pending(
    pending: &PendingControlRequest,
    created_at_ms: u64,
) -> PersistedAsk {
    PersistedAsk {
        request_id: pending.request_id.clone(),
        version: pending.version,
        nonce: pending.nonce.clone(),
        session_id: pending
            .session_id
            .clone()
            .unwrap_or_else(|| "default".to_string()),
        run_id: pending.run_id.clone(),
        turn_id: pending.turn_id.clone(),
        call_id: pending.call_id.clone(),
        kind: pending.request_kind.clone(),
        prompt: pending.prompt.clone().unwrap_or_default(),
        options: persisted_ask_options(pending.options.as_ref()),
        state: crate::agent::ask_persistence::AskLifecycleState::Pending,
        created_at_ms,
        answered_at_ms: None,
    }
}

/// Persisted Ask options are the production `string[]` schema; a `{label, description}` array is
/// flattened to its labels so the durable record keeps one shape.
fn persisted_ask_options(options: Option<&Value>) -> Vec<String> {
    match options {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(text) => Some(text.clone()),
                Value::Object(map) => map
                    .get("label")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Process-unique, collision-resistant control-request sequence.
///
/// The request id is the **persisted** Ask identity (`pending_asks.request_id` is the CAS key, ADR
/// 0074), so it must be unique across processes and across dispatcher instances in one process. A
/// plain per-dispatcher counter restarts at 0 (TUI + desktop, parent/child dispatchers, or parallel
/// tests sharing one `sessions.db`), which would let one session's answer CAS land on another
/// session's Ask. The seed mixes the wall clock with the pid; the global counter keeps ids unique
/// within the process. Shape stays `control-<digits>` (opaque to hosts).
fn next_control_request_seq() -> u64 {
    static GLOBAL_SEQ: AtomicU64 = AtomicU64::new(0);
    static SEED: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    const MODULUS: u64 = 1_000_000_000_000;
    let seed = *SEED.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0);
        (nanos ^ ((std::process::id() as u64).rotate_left(32))) % MODULUS
    });
    (seed + GLOBAL_SEQ.fetch_add(1, Ordering::Relaxed)) % MODULUS
}
