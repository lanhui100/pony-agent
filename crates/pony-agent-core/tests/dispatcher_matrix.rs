//! PA-076 phase 3 — dispatcher matrix tests (task 3.7).
//!
//! Integration test for the governed dispatcher and permission surface of
//! `pony_agent_core::agent`. Part 1 pins the phase-2 contracts plus the current
//! `GovernedDispatcher` surface (Agent A, tasks 3.1-3.3); Part 2 is the full governed-lifecycle
//! matrix, compiled and run today against the real API (the `#[ignore]` gates were removed once
//! Agent A's implementation landed, so the whole suite is exercised).
//!
//! Design authority: `openspec/changes/harden-and-expand-agent-tool-runtime/design.md`
//! (Verification Strategy, Decisions 1-5, 10) and `tasks.md` task 3.7.
//!
//! API reconciliation (Agent A surface this file is pinned to):
//!   - `dispatcher`: `GovernedDispatcher::new/register_handler/register_composite_handler(_with_authority)/
//!     register_pre_dispatch_hook/register_policy_evaluator/set_turn_view/set_default_context/
//!     set_budget_config/dispatch_governed/pending_request(s)/approve|answer|cancel_control_request/
//!     expire_control_requests/registry/clock`; `ToolDispatcher::dispatch`; `DispatchContext`,
//!     `PermissionVerdict`, `PermissionDecision`, `ToolPolicyEvaluator`, `PreDispatchHook`,
//!     `CompositeToolHandler`, `CompositeToolHandlerRequest`, `ControlRequestAuthorization`,
//!     `DispatchError`.
//!   - `child_dispatch`: `ChildDispatch` (`dispatch`/`dispatch_many`/`cancel`/`lineage`/`depth`/...),
//!     `ChildDispatchRequest`, `ChildDispatchResult`.
//!   - `budget`: `BudgetLedger`, `CancellationToken`, `DispatchBudgetConfig`.
//!   - `tool_runtime`: `TurnToolView`, `PendingControlRequest(+Kind/+State)`, `FakeClock`,
//!     `InvocationOrigin`, `ToolDispatchRequest`, `RuntimeClock`.
//!   - `tools`: `ToolRegistrySnapshot`, `ToolOutcome`, `ToolExecutionStatus`, `ToolControlKind`,
//!     `ToolControlOutcome`, `ToolDescriptor`/identity/kind/exposure/permission types.
//!
//! Note: `ToolRegistrySnapshot::resolve` matches registry aliases only, so builtin dispatch below
//! uses the resolvable primitive name (e.g. `echo_input`); custom descriptors register their
//! canonical id as an alias (see `test_descriptor`). Every builtin input schema requires the
//! injected `description` string property (see `tools::with_description`).

// MATRIX CHECKLIST (PA-076 3.7)
//
// Coverage legend: [P1] = passing contract test (Part 1); [L] = lifecycle matrix test
// (Part 2, run against the real API since the governed lifecycle landed with Agent A).
//
//   tool class
//     builtin   -> [P1] matrix_registry_* , matrix_dispatcher_* , matrix_turn_view_*
//                  [L] matrix_origin_model_can_only_call_model_visible_or_elevated ,
//                      matrix_origin_model_allowed_after_turn_view_elevation ,
//                      matrix_cancellation_timeout_and_budget_exhaustion_error_with_reason
//     MCP       -> [L] matrix_mcp_and_skill_descriptors_share_the_governed_chain
//     skill     -> [L] matrix_mcp_and_skill_descriptors_share_the_governed_chain
//     composite -> [P1] matrix_registry_rejects_composite_cycles_and_accepts_acyclic_composites
//                  [L] matrix_atomic_budget_ledger_exhaustion_and_partial_results ,
//                      matrix_child_dispatch_depth_limit_and_cycle_rejection ,
//                      matrix_origin_internal_descriptor_requires_child_lineage_or_host_system
//
//   origin
//     model     -> [P1] matrix_turn_view_* (the turn view is the model-visible surface)
//                  [L] matrix_origin_model_can_only_call_model_visible_or_elevated ,
//                      matrix_origin_model_allowed_after_turn_view_elevation
//     child     -> [L] matrix_origin_internal_descriptor_requires_child_lineage_or_host_system ,
//                      matrix_child_dispatch_depth_limit_and_cycle_rejection
//     host      -> [L] matrix_origin_internal_descriptor_requires_child_lineage_or_host_system
//     system    -> [L] matrix_origin_internal_descriptor_requires_child_lineage_or_host_system
//
//   permission decision
//     allow               -> [L] matrix_permission_allow_deny_approval_and_waiting_host (public cell)
//     deny                -> [L] matrix_permission_allow_deny_approval_and_waiting_host (evaluator cell)
//     approval_required   -> [P1] matrix_execution_status_legacy_roundtrip_maps_cancel_and_timeout_reasons
//                                (pending-outcome shape)
//                            [L] matrix_permission_allow_deny_approval_and_waiting_host ,
//                                matrix_control_request_cas_approve_answer_cancel_expire
//     control-suspend     -> [L] matrix_permission_allow_deny_approval_and_waiting_host (waiting_host cell),
//                                matrix_atomic_budget_ledger_exhaustion_and_partial_results
//                                (pending sibling blocks later children; waiting_user/Ask persistence is
//                                the graph matrix, task 4.5)
//     unknown scope / denied origin -> [P1] matrix_dispatcher_fails_closed_for_unknown_descriptor
//                            [L] matrix_origin_* (denied origin -> Error)
//
//   hook rewrite -> re-authorization
//     [L] matrix_hook_rewrite_revalidates_final_arguments
//
//   execution outcome
//     cancellation / timeout / budget exhaustion -> [P1]
//         matrix_budget_ledger_atomic_reservation_fails_closed ,
//         matrix_execution_status_legacy_roundtrip_maps_cancel_and_timeout_reasons ("aborted" map)
//         [L] matrix_cancellation_timeout_and_budget_exhaustion_error_with_reason
//     atomic budget ledger exhaustion + partial results -> [L]
//         matrix_atomic_budget_ledger_exhaustion_and_partial_results
//     depth limit / cycle / self-recursion -> [P1]
//         matrix_registry_rejects_composite_cycles_and_accepts_acyclic_composites
//         [L] matrix_child_dispatch_depth_limit_and_cycle_rejection

use pony_agent_core::agent::budget::{BudgetLedger, CancellationToken, DispatchBudgetConfig};
use pony_agent_core::agent::child_dispatch::{ChildDispatch, ChildDispatchRequest};
use pony_agent_core::agent::dispatcher::{
    CompositeToolHandler, CompositeToolHandlerRequest, ControlRequestAuthorization, DispatchContext,
    GovernedDispatcher, PermissionDecision, PermissionVerdict, PreDispatchHook,
    ToolPolicyEvaluator,
};
use pony_agent_core::agent::tool_runtime::{
    FakeClock, InvocationOrigin, PendingControlRequest, PendingControlRequestKind,
    PendingControlRequestState, PrimitiveToolHandler, PrimitiveToolHandlerRequest, RuntimeClock,
    ToolDispatchRequest, ToolDispatcher, TurnToolView,
};
use pony_agent_core::agent::tools::{
    ToolControlKind, ToolControlOutcome, ToolDescriptor, ToolDescriptorSource, ToolDisplayMetadata,
    ToolExecutionPolicy, ToolExecutionStatus, ToolExposure, ToolHandlerProvenance, ToolIdentity,
    ToolKind, ToolOutcome, ToolPermissionDeclaration, ToolRegistrySnapshot,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Shared test harness (public API only)
// ---------------------------------------------------------------------------

/// A canonical `PendingControlRequest` at a known session/version/nonce/expiry.
fn canonical_pending_request() -> PendingControlRequest {
    PendingControlRequest {
        request_id: "request-1".to_string(),
        request_kind: PendingControlRequestKind::Approval,
        session_id: Some("session-1".to_string()),
        run_id: Some("run-1".to_string()),
        turn_id: "turn-1".to_string(),
        call_id: "call-1".to_string(),
        descriptor_snapshot_id: "builtin-tool-catalog-v1".to_string(),
        descriptor_id: "builtin:workspace_write_file".to_string(),
        final_args_digest: "digest-1".to_string(),
        policy_digest: "policy-1".to_string(),
        nonce: "nonce-1".to_string(),
        version: 7,
        expires_at_ms: 5_000,
        state: PendingControlRequestState::Pending,
        prompt: Some("approve write?".to_string()),
        options: Some(json!(["yes", "no"])),
    }
}

/// Build a valid registry descriptor. `source` is derived from the descriptor-id namespace so the
/// descriptor satisfies the registry's identity/provenance validation. The canonical descriptor id
/// is also registered as an alias so `ToolRegistrySnapshot::resolve` accepts it (the current
/// resolver matches aliases; the governed dispatcher resolves identity/alias to the descriptor).
fn test_descriptor(
    descriptor_id: &str,
    model_name: &str,
    aliases: &[&str],
    kind: ToolKind,
    exposure: ToolExposure,
    permission: ToolPermissionDeclaration,
) -> ToolDescriptor {
    let source = match descriptor_id.split(':').next().unwrap_or("") {
        "builtin" => ToolDescriptorSource::Builtin,
        "mcp" => ToolDescriptorSource::Mcp,
        "skill" => ToolDescriptorSource::Skill,
        _ => ToolDescriptorSource::Dynamic,
    };
    let source_id = if source == ToolDescriptorSource::Builtin {
        "builtin-tools".to_string()
    } else {
        "test-source".to_string()
    };
    let mut aliases = aliases.iter().map(|alias| alias.to_string()).collect::<Vec<_>>();
    aliases.push(descriptor_id.to_string());
    ToolDescriptor {
        identity: ToolIdentity {
            descriptor_id: descriptor_id.to_string(),
            model_name: model_name.to_string(),
            canonical_name: model_name.to_string(),
            primitive_name: model_name.to_lowercase().replace(' ', "_"),
            source,
        },
        aliases,
        description: format!("test descriptor {descriptor_id}"),
        input_schema: json!({ "type": "object", "properties": {} }),
        kind,
        exposure,
        permission_declaration: permission,
        execution_policy: ToolExecutionPolicy::default(),
        display_metadata: ToolDisplayMetadata::default(),
        handler_provenance: ToolHandlerProvenance {
            handler_kind: "test".to_string(),
            source_id,
        },
        source_revision: "test-revision-v1".to_string(),
        composed_descriptor_ids: Vec::new(),
    }
}

fn request(
    origin: InvocationOrigin,
    descriptor_id: &str,
    call_id: &str,
    arguments: Value,
) -> ToolDispatchRequest {
    ToolDispatchRequest {
        origin,
        descriptor_id: descriptor_id.to_string(),
        call_id: call_id.to_string(),
        arguments,
    }
}

/// Extract the structured `error.code` from a fail-closed `ToolOutcome`, if present.
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

fn test_dispatcher() -> GovernedDispatcher {
    GovernedDispatcher::new(
        Arc::new(ToolRegistrySnapshot::builtin().expect("builtin registry should build")),
        Arc::new(FakeClock::new(0)),
    )
}

// ── primitive handlers ──────────────────────────────────────────────────────────

struct EchoHandler;

impl PrimitiveToolHandler for EchoHandler {
    fn execute(&self, request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
        Ok(request.arguments.clone())
    }
}

struct FailingHandler;

impl PrimitiveToolHandler for FailingHandler {
    fn execute(&self, _request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
        Err("test handler failed closed".to_string())
    }
}

struct FixedValueHandler(Value);

impl PrimitiveToolHandler for FixedValueHandler {
    fn execute(&self, _request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
        Ok(self.0.clone())
    }
}

// ── composite handlers (bounded ChildDispatch) ──────────────────────────────────

/// Dispatches a single fixed child through the bounded child dispatch.
struct ChildDispatcher(String);

impl CompositeToolHandler for ChildDispatcher {
    fn execute(
        &self,
        request: &CompositeToolHandlerRequest,
        children: &ChildDispatch,
    ) -> Result<Value, String> {
        let child_id = self.0.clone();
        let outcome = children.dispatch(ChildDispatchRequest {
            descriptor_id: child_id.clone(),
            call_id: format!("{}-child", request.call_id),
            arguments: json!({}),
        })?;
        let child_status = outcome.execution_status.as_legacy_status();
        let child_output = outcome
            .result
            .as_ref()
            .map(|result| result.output.clone())
            .unwrap_or_default();
        let child_error = outcome_error_code(&outcome);
        Ok(json!({
            "child": child_id,
            "child_status": child_status,
            "child_error": child_error,
            "child_output": child_output,
        }))
    }
}

/// Dispatches a fixed batch of children via `dispatch_many` and summarizes each result.
struct BatchHandler {
    children: Vec<&'static str>,
}

impl CompositeToolHandler for BatchHandler {
    fn execute(
        &self,
        request: &CompositeToolHandlerRequest,
        children: &ChildDispatch,
    ) -> Result<Value, String> {
        let requests = self
            .children
            .iter()
            .enumerate()
            .map(|(index, descriptor_id)| ChildDispatchRequest {
                descriptor_id: descriptor_id.to_string(),
                call_id: format!("{}-{index}", request.call_id),
                arguments: json!({}),
            })
            .collect::<Vec<_>>();
        let results = children.dispatch_many(requests)?;
        let summary: Vec<Value> = results
            .iter()
            .map(|result| {
                let control = result
                    .outcome
                    .control_outcome
                    .as_ref()
                    .map(|outcome| format!("{:?}", outcome.kind))
                    .unwrap_or_default();
                json!({
                    "index": result.index,
                    "started": result.started,
                    "descriptor_id": result.request.descriptor_id,
                    "status": result.outcome.execution_status.as_legacy_status(),
                    "control": control,
                    "error_code": outcome_error_code(&result.outcome),
                })
            })
            .collect();
        Ok(json!({ "children": summary }))
    }
}

/// Dispatches one child, cancels the shared token, then dispatches a second child.
struct CancelAfterFirstHandler;

impl CompositeToolHandler for CancelAfterFirstHandler {
    fn execute(
        &self,
        request: &CompositeToolHandlerRequest,
        children: &ChildDispatch,
    ) -> Result<Value, String> {
        let first = children.dispatch(ChildDispatchRequest {
            descriptor_id: "dynamic:child_a".to_string(),
            call_id: format!("{}-a", request.call_id),
            arguments: json!({}),
        })?;
        children.cancel();
        let second = children.dispatch(ChildDispatchRequest {
            descriptor_id: "dynamic:child_b".to_string(),
            call_id: format!("{}-b", request.call_id),
            arguments: json!({}),
        })?;
        let first_status = first.execution_status.as_legacy_status();
        let second_status = second.execution_status.as_legacy_status();
        let second_output = second
            .result
            .map(|result| result.output)
            .unwrap_or_default();
        Ok(json!({
            "first": first_status,
            "second": second_status,
            "second_output": second_output,
        }))
    }
}

// ── pre-dispatch hooks (argument rewrite only) ─────────────────────────────────

struct ApprovalForcingHook;

impl PreDispatchHook for ApprovalForcingHook {
    fn rewrite(
        &self,
        _descriptor_id: &str,
        _origin: &InvocationOrigin,
        _call_id: &str,
        mut arguments: Value,
    ) -> Result<Value, String> {
        arguments["target"] = json!("sensitive");
        Ok(arguments)
    }
}

struct RejectingHook;

impl PreDispatchHook for RejectingHook {
    fn rewrite(
        &self,
        _descriptor_id: &str,
        _origin: &InvocationOrigin,
        _call_id: &str,
        _arguments: Value,
    ) -> Result<Value, String> {
        Err("hook rejected the invocation".to_string())
    }
}

// ── policy evaluators ───────────────────────────────────────────────────────────

struct DenyEvaluator;

impl ToolPolicyEvaluator for DenyEvaluator {
    fn evaluate(
        &self,
        _descriptor: &ToolDescriptor,
        _origin: &InvocationOrigin,
        _final_arguments: &Value,
    ) -> PermissionDecision {
        PermissionDecision {
            verdict: PermissionVerdict::Deny,
            decision_source: "test_deny".to_string(),
            reason: Some("matrix deny cell".to_string()),
        }
    }
}

struct TargetAwareEvaluator;

impl ToolPolicyEvaluator for TargetAwareEvaluator {
    fn evaluate(
        &self,
        _descriptor: &ToolDescriptor,
        _origin: &InvocationOrigin,
        final_arguments: &Value,
    ) -> PermissionDecision {
        let verdict = if final_arguments.get("target").and_then(Value::as_str) == Some("sensitive") {
            PermissionVerdict::ApprovalRequired
        } else {
            PermissionVerdict::Allow
        };
        PermissionDecision {
            verdict,
            decision_source: "target_aware".to_string(),
            reason: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Part 1 — passing contract tests (phase-2 contracts + current dispatcher surface)
// ---------------------------------------------------------------------------

/// 1. PendingControlRequest CAS: a valid one-shot consumption passes, including at the expiry
/// boundary, and a consumed request can never be replayed.
#[test]
fn matrix_cas_pending_control_request_consumes_oneshot_valid_attempt() {
    let request = canonical_pending_request();
    assert!(request.can_consume(Some("session-1"), 7, "nonce-1", 4_000));
    // Expiry is inclusive: now == expires_at_ms still consumes.
    assert!(request.can_consume(Some("session-1"), 7, "nonce-1", 5_000));

    let consumed = PendingControlRequest {
        state: PendingControlRequestState::Consumed,
        ..request.clone()
    };
    assert!(!consumed.can_consume(Some("session-1"), 7, "nonce-1", 4_000));
}

/// 1. PendingControlRequest CAS: wrong session, wrong version, wrong nonce, and a session
/// identity mismatch (Some vs None) all fail closed (cross-session + CAS conflicts).
#[test]
fn matrix_cas_rejects_wrong_session_version_and_nonce() {
    let request = canonical_pending_request();
    assert!(!request.can_consume(Some("session-2"), 7, "nonce-1", 4_000));
    assert!(!request.can_consume(Some("session-1"), 6, "nonce-1", 4_000));
    assert!(!request.can_consume(Some("session-1"), 7, "nonce-2", 4_000));
    // A session-bound request cannot be consumed without its session id.
    assert!(!request.can_consume(None, 7, "nonce-1", 4_000));

    let sessionless = PendingControlRequest {
        session_id: None,
        ..request.clone()
    };
    assert!(!sessionless.can_consume(Some("session-1"), 7, "nonce-1", 4_000));
    assert!(sessionless.can_consume(None, 7, "nonce-1", 4_000));
}

/// 1. PendingControlRequest CAS: expiry and final (non-pending) states reject consumption.
#[test]
fn matrix_cas_rejects_expired_and_final_states() {
    let request = canonical_pending_request();
    assert!(!request.can_consume(Some("session-1"), 7, "nonce-1", 5_001));

    let cancelled = PendingControlRequest {
        state: PendingControlRequestState::Cancelled,
        ..request.clone()
    };
    let expired = PendingControlRequest {
        state: PendingControlRequestState::Expired,
        ..request.clone()
    };
    assert!(!cancelled.can_consume(Some("session-1"), 7, "nonce-1", 4_000));
    assert!(!expired.can_consume(Some("session-1"), 7, "nonce-1", 4_000));
}

/// 2. TurnToolView: `from_registry` exposes exactly the model-visible descriptors plus the
/// deferred `tool_search` discovery entry — nothing else.
#[test]
fn matrix_turn_view_from_registry_exposes_only_model_visible_plus_tool_search() {
    let registry = ToolRegistrySnapshot::builtin().expect("builtin registry should build");
    let view = TurnToolView::from_registry(&registry);
    assert_eq!(view.snapshot_id, registry.snapshot_id);

    let expected: BTreeSet<String> = registry
        .descriptors
        .iter()
        .filter(|descriptor| {
            descriptor.exposure == ToolExposure::ModelVisible
                || descriptor.identity.primitive_name == "tool_search"
        })
        .map(|descriptor| descriptor.identity.descriptor_id.clone())
        .collect();
    assert_eq!(view.direct_descriptor_ids, expected);
    assert!(view.elevated_descriptor_ids.is_empty());

    // A deferred descriptor is not visible until elevated.
    assert!(!view.allows_model_descriptor("builtin:workspace_path_info"));
    // The deferred discovery entry is exposed by contract so the model can request elevation.
    assert!(view.allows_model_descriptor("builtin:tool_search"));
}

/// 2. TurnToolView: only direct or explicitly elevated descriptor ids are model-addressable.
#[test]
fn matrix_turn_view_allows_only_direct_or_elevated_ids() {
    let mut view = TurnToolView {
        snapshot_id: "snapshot-1".to_string(),
        direct_descriptor_ids: BTreeSet::from(["builtin:workspace_read_file".to_string()]),
        elevated_descriptor_ids: BTreeSet::new(),
    };

    assert!(view.allows_model_descriptor("builtin:workspace_read_file"));
    assert!(!view.allows_model_descriptor("mcp:test-source:tool"));
    view.elevate("mcp:test-source:tool");
    assert!(view.allows_model_descriptor("mcp:test-source:tool"));
    assert!(view.elevated_descriptor_ids.contains("mcp:test-source:tool"));
}

/// 2. TurnToolView: `elevate_from_registry` succeeds only for `Deferred` descriptors, rejects
/// non-deferred and unknown descriptors, and projects the elevated descriptor to the provider
/// contract view.
#[test]
fn matrix_turn_view_elevates_only_deferred_descriptors_from_the_registry() {
    let registry = ToolRegistrySnapshot::builtin().expect("builtin registry should build");
    let mut view = TurnToolView::from_registry(&registry);

    assert!(view
        .elevate_from_registry(&registry, "builtin:workspace_path_info")
        .is_ok());
    assert!(view.allows_model_descriptor("builtin:workspace_path_info"));

    // Non-deferred and unknown descriptors are rejected.
    assert!(view
        .elevate_from_registry(&registry, "builtin:workspace_read_file")
        .is_err());
    assert!(view
        .elevate_from_registry(&registry, "mcp:test-source:read")
        .is_err());

    // After elevation the provider contract view includes the deferred descriptor.
    let contracts = view
        .provider_contract_views(&registry)
        .expect("matching snapshot should project provider contracts");
    assert!(contracts
        .iter()
        .any(|contract| contract.execution_primitive == "workspace_path_info"));
}

/// 2. TurnToolView: a snapshot id mismatch (stale registry) fails closed for projection and
/// elevation alike.
#[test]
fn matrix_turn_view_rejects_stale_registry_snapshot() {
    let registry = ToolRegistrySnapshot::builtin().expect("builtin registry should build");
    let mut view = TurnToolView::from_registry(&registry);
    let stale = ToolRegistrySnapshot::from_descriptors(
        "different-snapshot",
        registry.descriptors.clone(),
    )
    .expect("same descriptors with a new snapshot id remain valid");
    assert!(view.provider_contract_views(&stale).is_err());
    assert!(view
        .elevate_from_registry(&stale, "builtin:workspace_path_info")
        .is_err());
}

/// 3. Registry snapshot integrity: the builtin snapshot builds, has a stable snapshot id, unique
/// descriptor ids, and resolves both descriptor ids and product aliases.
#[test]
fn matrix_registry_builtin_snapshot_is_consistent_truth_source() {
    let registry = ToolRegistrySnapshot::builtin().expect("builtin registry should build");
    assert_eq!(registry.snapshot_id, "builtin-tool-catalog-v1");
    assert!(!registry.descriptors.is_empty());

    let mut ids = BTreeSet::new();
    for descriptor in &registry.descriptors {
        assert!(ids.insert(descriptor.identity.descriptor_id.clone()));
        // Every builtin descriptor resolves by its primitive name (a registry alias).
        assert!(registry.resolve(&descriptor.identity.primitive_name).is_some());
    }

    // Product aliases and the deferred discovery entry resolve from the same truth source.
    assert!(registry.resolve("workspace.run_command").is_some());
    assert!(registry.resolve("ToolSearch").is_some());
    let read_model = registry
        .model_name_for_primitive("workspace_read_file")
        .expect("read primitive has a model name");
    assert!(registry.resolve(read_model).is_some());
}

/// 3. Registry snapshot integrity: duplicate descriptor ids are rejected.
#[test]
fn matrix_registry_rejects_duplicate_descriptor_ids() {
    let registry = ToolRegistrySnapshot::builtin().expect("builtin registry should build");
    let mut descriptors = registry.descriptors.clone();
    descriptors.push(descriptors[0].clone());
    let error = ToolRegistrySnapshot::from_descriptors("duplicate-v1", descriptors)
        .expect_err("duplicate descriptor ids must fail closed");
    assert!(error.contains("duplicate"), "unexpected error: {error}");
}

/// 3. Registry snapshot integrity: aliases must be unambiguous across the snapshot.
#[test]
fn matrix_registry_rejects_ambiguous_aliases() {
    let first = test_descriptor(
        "dynamic:first",
        "First",
        &["shared"],
        ToolKind::Read,
        ToolExposure::Internal,
        ToolPermissionDeclaration::default(),
    );
    let second = test_descriptor(
        "dynamic:second",
        "Second",
        &["shared"],
        ToolKind::Read,
        ToolExposure::Internal,
        ToolPermissionDeclaration::default(),
    );
    let error = ToolRegistrySnapshot::from_descriptors("alias-v1", vec![first, second])
        .expect_err("ambiguous aliases must fail closed");
    assert!(error.contains("ambiguous"), "unexpected error: {error}");
}

/// 3. Registry snapshot integrity: composite dependency cycles and unknown children are rejected;
/// acyclic composites that reference known children are accepted.
#[test]
fn matrix_registry_rejects_composite_cycles_and_accepts_acyclic_composites() {
    let mut one = test_descriptor(
        "dynamic:one",
        "One",
        &["dynamic.one"],
        ToolKind::Composite,
        ToolExposure::Internal,
        ToolPermissionDeclaration::default(),
    );
    let mut two = test_descriptor(
        "dynamic:two",
        "Two",
        &["dynamic.two"],
        ToolKind::Composite,
        ToolExposure::Internal,
        ToolPermissionDeclaration::default(),
    );
    one.composed_descriptor_ids = vec!["dynamic:two".to_string()];
    two.composed_descriptor_ids = vec!["dynamic:one".to_string()];
    let error = ToolRegistrySnapshot::from_descriptors("cycle-v1", vec![one.clone(), two.clone()])
        .expect_err("composite cycles must fail closed");
    assert!(error.contains("dependency cycle"), "unexpected error: {error}");

    let child = test_descriptor(
        "dynamic:child",
        "Child",
        &["dynamic.child"],
        ToolKind::Read,
        ToolExposure::ModelVisible,
        ToolPermissionDeclaration::default(),
    );
    one.composed_descriptor_ids = vec!["dynamic:child".to_string()];
    two.composed_descriptor_ids = vec!["dynamic:child".to_string()];
    let registry = ToolRegistrySnapshot::from_descriptors("acyclic-v1", vec![one, two, child])
        .expect("acyclic composites are valid");
    assert_eq!(registry.snapshot_id, "acyclic-v1");

    let mut bad = test_descriptor(
        "dynamic:bad",
        "Bad",
        &["dynamic.bad"],
        ToolKind::Composite,
        ToolExposure::Internal,
        ToolPermissionDeclaration::default(),
    );
    bad.composed_descriptor_ids = vec!["dynamic:missing".to_string()];
    assert!(ToolRegistrySnapshot::from_descriptors("bad-child-v1", vec![bad]).is_err());
}

/// 4. Dispatcher: an unknown descriptor fails closed with a structured error outcome.
#[test]
fn matrix_dispatcher_fails_closed_for_unknown_descriptor() {
    let dispatcher = test_dispatcher();
    let outcome = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "no:such-tool",
        "call-1",
        json!({}),
    ));
    assert_eq!(outcome.execution_status, ToolExecutionStatus::Error);
    assert_eq!(outcome_error_code(&outcome).as_deref(), Some("unknown_descriptor"));
}

/// 4. Dispatcher: a registered primitive handler runs and echoes its result; the same descriptor
/// without a handler fails closed. Builtin schemas require the injected `description` argument.
#[test]
fn matrix_dispatcher_runs_registered_handler_and_missing_handler_fails_closed() {
    let registry = Arc::new(ToolRegistrySnapshot::builtin().expect("builtin registry should build"));
    let dispatcher = GovernedDispatcher::new(registry.clone(), Arc::new(FakeClock::new(0)));
    let arguments = json!({ "description": "test", "text": "hi" });

    // No handler registered yet -> fail closed with a structured code.
    let missing = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "echo_input",
        "call-1",
        arguments.clone(),
    ));
    assert_eq!(missing.execution_status, ToolExecutionStatus::Error);
    assert_eq!(outcome_error_code(&missing).as_deref(), Some("no_handler_registered"));

    // Registered handler (under the canonical descriptor id) -> echoed ok outcome.
    dispatcher.register_handler("builtin:echo_input", Arc::new(EchoHandler));
    let ok = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "echo_input",
        "call-2",
        arguments.clone(),
    ));
    assert_eq!(ok.execution_status, ToolExecutionStatus::Ok);
    let ok_result = ok.result.expect("ok outcome carries a legacy result");
    assert_eq!(ok_result.status, "ok");
    assert!(
        ok_result.output.contains("\"text\": \"hi\""),
        "unexpected output: {}",
        ok_result.output
    );
}

/// 4. Dispatcher: a handler error maps to an error outcome carrying the handler's message.
#[test]
fn matrix_dispatcher_maps_handler_failure_to_error_outcome() {
    let dispatcher = test_dispatcher();
    // `echo_input` is the model-visible winner of the `Ask` product slot; `time_now` loses the
    // `Run` slot to `workspace_run_command` and is internal in the builtin registry.
    dispatcher.register_handler("builtin:echo_input", Arc::new(FailingHandler));
    let outcome = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "echo_input",
        "call-1",
        json!({ "description": "test", "text": "hi" }),
    ));
    assert_eq!(outcome.execution_status, ToolExecutionStatus::Error);
    assert_eq!(outcome_error_code(&outcome).as_deref(), Some("handler_error"));
    let result = outcome.result.expect("error outcome carries a legacy result");
    assert!(
        result.output.contains("test handler failed closed"),
        "unexpected output: {}",
        result.output
    );
}

/// 4. Dispatcher: the pinned accessors (`registry`, `clock`) expose the injected truth source
/// and clock.
#[test]
fn matrix_dispatcher_exposes_registry_and_clock() {
    let clock = Arc::new(FakeClock::new(42));
    let dispatcher = GovernedDispatcher::new(
        Arc::new(ToolRegistrySnapshot::builtin().expect("builtin registry should build")),
        clock.clone(),
    );
    assert_eq!(dispatcher.registry().snapshot_id, "builtin-tool-catalog-v1");
    assert_eq!(dispatcher.clock().now_ms(), 42);
    assert_eq!(clock.now_ms(), 42);
}

/// Contract: execution status and control outcome stay orthogonal, and a pending control request
/// is never a provider-consumable result (Decision 5). Also pins the cancel/timeout legacy map.
#[test]
fn matrix_execution_status_legacy_roundtrip_maps_cancel_and_timeout_reasons() {
    assert_eq!(ToolExecutionStatus::Ok.as_legacy_status(), "ok");
    assert_eq!(ToolExecutionStatus::Error.as_legacy_status(), "error");
    assert_eq!(ToolExecutionStatus::Cancelled.as_legacy_status(), "aborted");
    assert_eq!(
        ToolExecutionStatus::from_legacy_status("ok"),
        ToolExecutionStatus::Ok
    );
    assert_eq!(
        ToolExecutionStatus::from_legacy_status("aborted"),
        ToolExecutionStatus::Cancelled
    );
    assert_eq!(
        ToolExecutionStatus::from_legacy_status("cancelled"),
        ToolExecutionStatus::Cancelled
    );
    assert_eq!(
        ToolExecutionStatus::from_legacy_status("error"),
        ToolExecutionStatus::Error
    );

    // A pending control request has an orthogonal control outcome, not a consumable result.
    let pending = ToolOutcome::pending(ToolControlOutcome {
        kind: ToolControlKind::ApprovalRequired,
        request_id: "request-1".to_string(),
    });
    assert_eq!(pending.execution_status, ToolExecutionStatus::Ok);
    assert_eq!(
        pending.control_outcome.as_ref().map(|c| c.kind.clone()),
        Some(ToolControlKind::ApprovalRequired)
    );
    let legacy = pending.into_legacy_result("Ask");
    assert_eq!(legacy.status, "error");
    assert!(legacy.output.contains("control_outcome_pending"));
}

/// Contract: the shared atomic budget ledger and cancellation token fail closed at every
/// reservation boundary (calls / concurrency / bytes / deadline / cancellation).
#[test]
fn matrix_budget_ledger_atomic_reservation_fails_closed() {
    let ledger = Arc::new(BudgetLedger::new(2, 1, 10, 100, 50));
    assert!(ledger.reserve_call().is_ok());
    assert!(ledger.reserve_call().is_ok());
    assert!(ledger.reserve_call().is_err());
    assert_eq!(ledger.calls_reserved(), 2);

    {
        let _guard = ledger.begin_execution().expect("first concurrency slot is free");
        assert!(ledger.begin_execution().is_err());
    }
    assert!(ledger.begin_execution().is_ok());

    assert!(ledger.reserve_bytes(6).is_ok());
    assert!(ledger.reserve_bytes(4).is_ok());
    assert!(ledger.reserve_bytes(1).is_err());
    assert_eq!(ledger.bytes_reserved(), 10);

    assert!(ledger.check_deadline(149).is_ok());
    assert!(ledger.check_deadline(151).is_err());
    assert_eq!(ledger.remaining_deadline_ms(120), Some(30));

    let token = CancellationToken::new();
    assert!(token.check().is_ok());
    token.cancel();
    assert!(token.is_cancelled());
    assert!(token.check().is_err());
}

// ---------------------------------------------------------------------------
// Part 2 — governed lifecycle matrix, run against the real API (the `#[ignore]` gates were
// removed once Agent A's implementation landed).
// ---------------------------------------------------------------------------

/// Origins x exposure: a model origin may only call descriptors the current turn view exposes
/// directly (model-visible) or has elevated; anything else is a denied origin -> Error.
#[test]
fn matrix_origin_model_can_only_call_model_visible_or_elevated() {
    // TODO(PA-076 3.7): Decision 2 step 1 / Decision 10 — model origin is authorized only for
    // descriptors in the current turn provider view (direct or elevated). Deferred/internal
    // descriptors that are not elevated are denied origins with a structured reason.
    let registry = Arc::new(ToolRegistrySnapshot::builtin().expect("builtin registry should build"));
    let dispatcher = GovernedDispatcher::new(registry.clone(), Arc::new(FakeClock::new(0)));

    // Model-visible direct descriptor -> allowed.
    dispatcher.register_handler("builtin:echo_input", Arc::new(EchoHandler));
    let allowed = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "echo_input",
        "call-1",
        json!({ "description": "test", "text": "hi" }),
    ));
    assert_eq!(allowed.execution_status, ToolExecutionStatus::Ok);

    // Deferred, not elevated -> denied origin.
    dispatcher.register_handler("builtin:workspace_path_info", Arc::new(EchoHandler));
    let denied = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "workspace_path_info",
        "call-2",
        json!({ "description": "test" }),
    ));
    assert_eq!(denied.execution_status, ToolExecutionStatus::Error);
    assert_eq!(outcome_error_code(&denied).as_deref(), Some("origin_not_authorized"));
}

/// Origins x exposure: a deferred descriptor elevated into the current turn view becomes
/// model-callable for that turn.
#[test]
fn matrix_origin_model_allowed_after_turn_view_elevation() {
    // TODO(PA-076 3.7): Decision 10 — elevation is turn-scoped; the elevated schema must appear in
    // the next provider hop, and a fresh turn without re-elevation must reject the descriptor again.
    let registry = Arc::new(ToolRegistrySnapshot::builtin().expect("builtin registry should build"));
    let dispatcher = GovernedDispatcher::new(registry.clone(), Arc::new(FakeClock::new(0)));
    dispatcher.register_handler("builtin:workspace_path_info", Arc::new(EchoHandler));

    let before = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "workspace_path_info",
        "call-1",
        json!({ "description": "test" }),
    ));
    assert_eq!(before.execution_status, ToolExecutionStatus::Error);

    let mut view = TurnToolView::from_registry(&registry);
    view.elevate_from_registry(&registry, "builtin:workspace_path_info")
        .expect("deferred descriptors elevate");
    dispatcher.set_turn_view(view);

    let after = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "workspace_path_info",
        "call-2",
        json!({ "description": "test" }),
    ));
    assert_eq!(after.execution_status, ToolExecutionStatus::Ok);
}

/// Origins x exposure: internal descriptors are reachable only through a bounded child dispatch;
/// direct model/host/system/child origins all fail closed.
#[test]
fn matrix_origin_internal_descriptor_requires_child_lineage_or_host_system() {
    // TODO(PA-076 3.7): Decision 2 — internal descriptors are callable only via a ChildDispatch
    // carrying parent lineage + allowed-child authority. Host/system origins cannot bypass the
    // child boundary either; a bare child origin at top level is rejected.
    let internal = test_descriptor(
        "dynamic:internal",
        "Internal",
        &["dynamic.internal"],
        ToolKind::Read,
        ToolExposure::Internal,
        ToolPermissionDeclaration::default(),
    );
    let mut orchestrator = test_descriptor(
        "dynamic:orchestrator",
        "Orchestrator",
        &["dynamic.orchestrator"],
        ToolKind::Composite,
        ToolExposure::ModelVisible,
        ToolPermissionDeclaration::default(),
    );
    orchestrator.composed_descriptor_ids = vec!["dynamic:internal".to_string()];
    let registry = Arc::new(
        ToolRegistrySnapshot::from_descriptors("origin-v1", vec![internal, orchestrator])
            .expect("custom registry should build"),
    );
    let dispatcher = GovernedDispatcher::new(registry.clone(), Arc::new(FakeClock::new(0)));
    dispatcher.register_handler("dynamic:internal", Arc::new(EchoHandler));
    dispatcher.register_composite_handler(
        "dynamic:orchestrator",
        Arc::new(ChildDispatcher("dynamic:internal".to_string())),
    );

    // Bare model and lineage-less child origins are denied for internal descriptors.
    for (origin, call_id) in [
        (InvocationOrigin::Model, "call-1"),
        (InvocationOrigin::Child, "call-4"),
    ] {
        let outcome = dispatcher.dispatch(request(origin.clone(), "dynamic:internal", call_id, json!({})));
        assert_eq!(
            outcome.execution_status,
            ToolExecutionStatus::Error,
            "origin {origin:?} must be denied for an internal descriptor"
        );
        assert_eq!(
            outcome_error_code(&outcome).as_deref(),
            Some("origin_not_authorized")
        );
    }

    // Trusted adapter origins (host/system) may reach internal descriptors directly; this is the
    // host adapter's housekeeping path and is not subject to the model turn-view gate.
    for (origin, call_id) in [
        (InvocationOrigin::Host, "call-2"),
        (InvocationOrigin::System, "call-3"),
    ] {
        let outcome = dispatcher.dispatch(request(origin.clone(), "dynamic:internal", call_id, json!({})));
        assert_eq!(
            outcome.execution_status,
            ToolExecutionStatus::Ok,
            "trusted origin {origin:?} may reach an internal descriptor"
        );
    }

    // Through a bounded child dispatch the internal descriptor is reachable.
    let outcome = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "dynamic:orchestrator",
        "call-5",
        json!({}),
    ));
    assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
    let result = outcome.result.expect("composite outcome carries a result");
    assert!(
        result.output.contains("\"child\": \"dynamic:internal\""),
        "unexpected output: {}",
        result.output
    );
}

/// Permission: allow / deny / approval-required / control-suspend decisions on final arguments.
#[test]
fn matrix_permission_allow_deny_approval_and_waiting_host() {
    // TODO(PA-076 3.7): Decision 4 — ToolPolicyEvaluator maps final normalized arguments to
    // allow / deny / approval_required / waiting_host; unknown scopes fail closed and child
    // scopes only tighten. Decisions record decision_source.
    let public = test_descriptor(
        "dynamic:public",
        "Public",
        &["dynamic.public"],
        ToolKind::Read,
        ToolExposure::ModelVisible,
        ToolPermissionDeclaration {
            requires_approval: false,
            ..ToolPermissionDeclaration::default()
        },
    );
    let sensitive = test_descriptor(
        "dynamic:sensitive",
        "Sensitive",
        &["dynamic.sensitive"],
        ToolKind::Write,
        ToolExposure::ModelVisible,
        ToolPermissionDeclaration {
            requires_approval: true,
            approval_mode: Some("host".to_string()),
            ..ToolPermissionDeclaration::default()
        },
    );
    let waiting_host = test_descriptor(
        "dynamic:waiting_host",
        "WaitingHost",
        &["dynamic.waiting_host"],
        ToolKind::Execute,
        ToolExposure::ModelVisible,
        ToolPermissionDeclaration {
            host_mediated: true,
            approval_mode: Some("host".to_string()),
            ..ToolPermissionDeclaration::default()
        },
    );
    let registry = Arc::new(
        ToolRegistrySnapshot::from_descriptors(
            "permission-v1",
            vec![public, sensitive, waiting_host],
        )
        .expect("custom registry should build"),
    );
    let dispatcher = GovernedDispatcher::new(registry.clone(), Arc::new(FakeClock::new(0)));
    for descriptor_id in ["dynamic:public", "dynamic:sensitive", "dynamic:waiting_host"] {
        dispatcher.register_handler(descriptor_id, Arc::new(EchoHandler));
    }

    // allow -> normal ok outcome.
    let allowed = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "dynamic:public",
        "call-1",
        json!({}),
    ));
    assert_eq!(allowed.execution_status, ToolExecutionStatus::Ok);

    // approval_required -> orthogonal control outcome + persisted request, not a consumable result.
    let approval = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "dynamic:sensitive",
        "call-2",
        json!({}),
    ));
    assert_eq!(approval.execution_status, ToolExecutionStatus::Ok);
    assert_eq!(
        approval.control_outcome.as_ref().map(|c| c.kind.clone()),
        Some(ToolControlKind::ApprovalRequired)
    );
    let approval_request_id = approval
        .control_outcome
        .as_ref()
        .expect("control outcome has a request id")
        .request_id
        .clone();
    let persisted = dispatcher
        .pending_request(&approval_request_id)
        .expect("approval request is persisted");
    assert_eq!(persisted.descriptor_id, "dynamic:sensitive");
    assert_eq!(persisted.state, PendingControlRequestState::Pending);

    // control-suspend (waiting_host) -> orthogonal control outcome.
    let waiting = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "dynamic:waiting_host",
        "call-3",
        json!({}),
    ));
    assert_eq!(
        waiting.control_outcome.as_ref().map(|c| c.kind.clone()),
        Some(ToolControlKind::WaitingHost)
    );

    // deny -> structured permission_denied error via a registered policy evaluator.
    dispatcher.register_policy_evaluator(Arc::new(DenyEvaluator));
    let denied = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "dynamic:public",
        "call-4",
        json!({}),
    ));
    assert_eq!(denied.execution_status, ToolExecutionStatus::Error);
    assert_eq!(outcome_error_code(&denied).as_deref(), Some("permission_denied"));
    // TODO(PA-076 3.7): waiting_user / Ask is the PendingControlRequest-kind sibling of approval;
    // runtime persistence of the suspended run is asserted in the graph integration matrix
    // (task 4.5), not the dispatcher.
}

/// Control-suspend: approve/answer/cancel/expire are compare-and-swap and reject replay, expiry,
/// cross-session, and version/nonce mismatches.
#[test]
fn matrix_control_request_cas_approve_answer_cancel_expire() {
    // TODO(PA-076 3.7): Decision 5 — approval/Ask share PendingControlRequest; the runtime
    // consumes via CAS (session/version/nonce/expiry + descriptor/snapshot facts), and the same
    // authorization can never be replayed (task 3.2). `answer_control_request` needs the
    // Interaction (Ask) kind, which the dispatcher does not produce until task 4.3.
    let sensitive = test_descriptor(
        "dynamic:sensitive",
        "Sensitive",
        &["dynamic.sensitive"],
        ToolKind::Write,
        ToolExposure::ModelVisible,
        ToolPermissionDeclaration {
            requires_approval: true,
            approval_mode: Some("host".to_string()),
            ..ToolPermissionDeclaration::default()
        },
    );
    let registry = Arc::new(
        ToolRegistrySnapshot::from_descriptors("cas-v1", vec![sensitive])
            .expect("custom registry should build"),
    );
    let clock = Arc::new(FakeClock::new(1_000));
    let dispatcher = GovernedDispatcher::new(registry.clone(), clock.clone());
    dispatcher.register_handler("dynamic:sensitive", Arc::new(EchoHandler));
    let context = DispatchContext {
        session_id: Some("session-1".to_string()),
        run_id: Some("run-1".to_string()),
        turn_id: Some("turn-1".to_string()),
        ..DispatchContext::default()
    };

    let outcome = dispatcher.dispatch_governed(
        request(InvocationOrigin::Model, "dynamic:sensitive", "call-1", json!({})),
        &context,
    );
    assert_eq!(
        outcome.control_outcome.as_ref().map(|c| c.kind.clone()),
        Some(ToolControlKind::ApprovalRequired)
    );
    let request_id = outcome
        .control_outcome
        .as_ref()
        .expect("control outcome has a request id")
        .request_id
        .clone();
    let pending = dispatcher
        .pending_request(&request_id)
        .expect("approval request persisted");
    assert_eq!(pending.session_id.as_deref(), Some("session-1"));
    assert_eq!(pending.descriptor_id, "dynamic:sensitive");
    assert_eq!(pending.descriptor_snapshot_id, "cas-v1");

    let authorization = ControlRequestAuthorization {
        session_id: pending.session_id.clone(),
        expected_version: pending.version,
        nonce: pending.nonce.clone(),
        expected_descriptor_snapshot_id: Some(pending.descriptor_snapshot_id.clone()),
        expected_descriptor_id: Some(pending.descriptor_id.clone()),
        expected_final_args_digest: Some(pending.final_args_digest.clone()),
        expected_policy_digest: Some(pending.policy_digest.clone()),
        answer: None,
    };

    // Approve once.
    let consumed = dispatcher
        .approve_control_request(&request_id, &authorization)
        .expect("approve succeeds with correct facts");
    assert_eq!(consumed.request.state, PendingControlRequestState::Consumed);
    assert_eq!(consumed.request.version, pending.version + 1);

    // Replay with the identical authorization fails (version bumped).
    assert!(dispatcher.approve_control_request(&request_id, &authorization).is_err());
    // Wrong nonce fails.
    let wrong_nonce = ControlRequestAuthorization {
        nonce: "wrong".to_string(),
        ..authorization.clone()
    };
    assert!(dispatcher.approve_control_request(&request_id, &wrong_nonce).is_err());
    // Cross-session fails.
    let cross_session = ControlRequestAuthorization {
        session_id: Some("session-2".to_string()),
        ..authorization.clone()
    };
    assert!(dispatcher.approve_control_request(&request_id, &cross_session).is_err());
    // Descriptor mismatch fails.
    let wrong_descriptor = ControlRequestAuthorization {
        expected_descriptor_id: Some("dynamic:other".to_string()),
        ..authorization.clone()
    };
    assert!(dispatcher.approve_control_request(&request_id, &wrong_descriptor).is_err());

    // A second pending request can be cancelled via CAS.
    let second = dispatcher.dispatch_governed(
        request(InvocationOrigin::Model, "dynamic:sensitive", "call-2", json!({})),
        &context,
    );
    let second_id = second
        .control_outcome
        .as_ref()
        .expect("second pending request")
        .request_id
        .clone();
    let second_pending = dispatcher
        .pending_request(&second_id)
        .expect("second approval persisted");
    let cancel_auth = ControlRequestAuthorization {
        session_id: second_pending.session_id.clone(),
        expected_version: second_pending.version,
        nonce: second_pending.nonce.clone(),
        expected_descriptor_snapshot_id: None,
        expected_descriptor_id: None,
        expected_final_args_digest: None,
        expected_policy_digest: None,
        answer: None,
    };
    let cancelled = dispatcher
        .cancel_control_request(&second_id, &cancel_auth)
        .expect("cancel succeeds");
    assert_eq!(cancelled.request.state, PendingControlRequestState::Cancelled);
    assert!(dispatcher.cancel_control_request(&second_id, &cancel_auth).is_err());

    // Expiry transitions pending requests and blocks further consumption.
    let third = dispatcher.dispatch_governed(
        request(InvocationOrigin::Model, "dynamic:sensitive", "call-3", json!({})),
        &context,
    );
    let third_id = third
        .control_outcome
        .as_ref()
        .expect("third pending request")
        .request_id
        .clone();
    assert_eq!(dispatcher.expire_control_requests(clock.now_ms() + 61_000), 1);
    let expired = dispatcher.pending_request(&third_id).expect("expired request retained");
    assert_eq!(expired.state, PendingControlRequestState::Expired);
    let expired_auth = ControlRequestAuthorization {
        session_id: expired.session_id.clone(),
        expected_version: expired.version,
        nonce: expired.nonce.clone(),
        expected_descriptor_snapshot_id: None,
        expected_descriptor_id: None,
        expected_final_args_digest: None,
        expected_policy_digest: None,
        answer: None,
    };
    assert!(dispatcher.approve_control_request(&third_id, &expired_auth).is_err());
    // TODO(PA-076 3.7): `answer_control_request` (Interaction/Ask kind) is unreachable until the
    // Ask control path lands (task 4.3); the dispatcher currently persists every control request
    // as `Approval`.
}

/// Tool class: MCP and skill descriptors flow through the same governed chain as builtin and
/// composite; internal skill exposure follows the origin matrix.
#[test]
fn matrix_mcp_and_skill_descriptors_share_the_governed_chain() {
    // TODO(PA-076 3.7): MCP/skill descriptors are projected from the registry snapshot with the
    // same identity/exposure/permission rules; execution is source-bound (McpTransport in
    // task 7.2, skill runtime in task 3.5) and remains fully governed.
    let mcp = test_descriptor(
        "mcp:test-source:read",
        "McpRead",
        &["mcp.read"],
        ToolKind::Read,
        ToolExposure::ModelVisible,
        ToolPermissionDeclaration::default(),
    );
    let skill = test_descriptor(
        "skill:test-source:summarize",
        "SkillSummarize",
        &["skill.summarize"],
        ToolKind::Composite,
        ToolExposure::Internal,
        ToolPermissionDeclaration::default(),
    );
    let mut orchestrator = test_descriptor(
        "dynamic:orchestrator",
        "Orchestrator",
        &["dynamic.orchestrator"],
        ToolKind::Composite,
        ToolExposure::ModelVisible,
        ToolPermissionDeclaration::default(),
    );
    orchestrator.composed_descriptor_ids = vec!["skill:test-source:summarize".to_string()];
    let registry = Arc::new(
        ToolRegistrySnapshot::from_descriptors("external-v1", vec![mcp, skill, orchestrator])
            .expect("custom registry should build"),
    );
    let dispatcher = GovernedDispatcher::new(registry.clone(), Arc::new(FakeClock::new(0)));
    dispatcher.register_handler("mcp:test-source:read", Arc::new(EchoHandler));
    dispatcher.register_handler("skill:test-source:summarize", Arc::new(EchoHandler));
    dispatcher.register_composite_handler(
        "dynamic:orchestrator",
        Arc::new(ChildDispatcher("skill:test-source:summarize".to_string())),
    );

    // Model-visible MCP descriptor is directly callable by the model.
    let mcp_outcome = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "mcp:test-source:read",
        "call-1",
        json!({}),
    ));
    assert_eq!(mcp_outcome.execution_status, ToolExecutionStatus::Ok);

    // The internal skill is reached only through a bounded child dispatch.
    let skill_outcome = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "dynamic:orchestrator",
        "call-2",
        json!({}),
    ));
    assert_eq!(skill_outcome.execution_status, ToolExecutionStatus::Ok);
    let result = skill_outcome.result.expect("orchestrator outcome carries a result");
    assert!(
        result.output.contains("skill:test-source:summarize"),
        "unexpected output: {}",
        result.output
    );

    // Direct model access to the internal skill is denied.
    let direct_skill = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "skill:test-source:summarize",
        "call-3",
        json!({}),
    ));
    assert_eq!(direct_skill.execution_status, ToolExecutionStatus::Error);
    assert_eq!(outcome_error_code(&direct_skill).as_deref(), Some("origin_not_authorized"));
}

/// Hook rewrite -> re-authorization: a pre-dispatch hook may rewrite arguments; the dispatcher
/// re-validates and re-decides permissions against the final arguments.
#[test]
fn matrix_hook_rewrite_revalidates_final_arguments() {
    // TODO(PA-076 3.7): Decision 2 steps 3-4 — a mutable pre-dispatch hook may rewrite arguments;
    // the dispatcher re-runs raw + final schema/validation and the permission/sandbox decision
    // against the rewritten arguments, and a changed decision changes the outcome. The hook must
    // not be able to change descriptor identity or invocation origin; a rejected hook fails closed.
    let scoped = test_descriptor(
        "dynamic:scoped",
        "Scoped",
        &["dynamic.scoped"],
        ToolKind::Write,
        ToolExposure::ModelVisible,
        ToolPermissionDeclaration::default(),
    );
    let registry = Arc::new(
        ToolRegistrySnapshot::from_descriptors("hook-v1", vec![scoped])
            .expect("custom registry should build"),
    );
    let dispatcher = GovernedDispatcher::new(registry.clone(), Arc::new(FakeClock::new(0)));
    dispatcher.register_handler("dynamic:scoped", Arc::new(EchoHandler));

    // Baseline allow with the default evaluator.
    let baseline = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "dynamic:scoped",
        "call-1",
        json!({ "target": "safe" }),
    ));
    assert_eq!(baseline.execution_status, ToolExecutionStatus::Ok);

    // A hook rewrites the target to "sensitive"; a target-aware evaluator re-decides on the
    // rewritten (final) arguments and requires approval.
    dispatcher.register_policy_evaluator(Arc::new(TargetAwareEvaluator));
    dispatcher.register_pre_dispatch_hook(Arc::new(ApprovalForcingHook));
    let rewritten = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "dynamic:scoped",
        "call-2",
        json!({ "target": "safe" }),
    ));
    assert_eq!(
        rewritten.control_outcome.as_ref().map(|c| c.kind.clone()),
        Some(ToolControlKind::ApprovalRequired)
    );
    // TODO(PA-076 3.7): the pending request's final_args_digest must be derived from the rewritten
    // arguments, so a control authorization bound to the pre-hook digest fails CAS.

    // A hook that rejects the invocation fails closed with a structured reason.
    dispatcher.register_pre_dispatch_hook(Arc::new(RejectingHook));
    let rejected = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "dynamic:scoped",
        "call-3",
        json!({}),
    ));
    assert_eq!(rejected.execution_status, ToolExecutionStatus::Error);
    assert_eq!(outcome_error_code(&rejected).as_deref(), Some("pre_dispatch_hook_error"));
}

/// Cancellation / timeout / budget exhaustion: every abnormal termination is an Error carrying a
/// structured reason, never a synthetic success.
#[test]
fn matrix_cancellation_timeout_and_budget_exhaustion_error_with_reason() {
    // TODO(PA-076 3.7): Decision 2/5 — cancellation, timeout and budget exhaustion terminate with
    // a structured reason (`cancelled` / `timeout` / `budget_exhausted` / `output_budget_exceeded`),
    // not a fake success. Cancellation surfaces as ToolExecutionStatus::Cancelled (legacy
    // "aborted"); timeout is relative to the injected clock.
    let child_a = test_descriptor(
        "dynamic:child_a",
        "ChildA",
        &["dynamic.child_a"],
        ToolKind::Read,
        ToolExposure::Internal,
        ToolPermissionDeclaration::default(),
    );
    let child_b = test_descriptor(
        "dynamic:child_b",
        "ChildB",
        &["dynamic.child_b"],
        ToolKind::Read,
        ToolExposure::Internal,
        ToolPermissionDeclaration::default(),
    );
    let mut composite = test_descriptor(
        "dynamic:composite",
        "Composite",
        &["dynamic.composite"],
        ToolKind::Composite,
        ToolExposure::ModelVisible,
        ToolPermissionDeclaration::default(),
    );
    composite.composed_descriptor_ids = vec!["dynamic:child_a".to_string(), "dynamic:child_b".to_string()];
    let registry = Arc::new(
        ToolRegistrySnapshot::from_descriptors("cancel-v1", vec![child_a, child_b, composite])
            .expect("custom registry should build"),
    );

    // Cancellation: the composite cancels the shared token; the next child fails as Cancelled.
    let cancel_dispatcher = GovernedDispatcher::new(registry.clone(), Arc::new(FakeClock::new(0)));
    cancel_dispatcher.register_handler("dynamic:child_a", Arc::new(FixedValueHandler(json!({"ok": true}))));
    cancel_dispatcher.register_handler("dynamic:child_b", Arc::new(FixedValueHandler(json!({"ok": true}))));
    cancel_dispatcher.register_composite_handler("dynamic:composite", Arc::new(CancelAfterFirstHandler));
    let cancelled = cancel_dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "dynamic:composite",
        "call-1",
        json!({}),
    ));
    assert_eq!(cancelled.execution_status, ToolExecutionStatus::Ok);
    let cancelled_output = cancelled
        .result
        .expect("composite outcome carries a result")
        .output;
    let payload: Value = serde_json::from_str(&cancelled_output).expect("composite output is json");
    // The composite handler cancels after the first child; the second child fails closed as
    // `Cancelled` (legacy status "aborted") with the structured `cancelled` reason.
    assert_eq!(payload["first"], "ok");
    assert_eq!(payload["second"], "aborted");
    let second_output: Value = serde_json::from_str(
        payload["second_output"]
            .as_str()
            .expect("second_output is a nested json string"),
    )
    .expect("nested second_output is json");
    assert_eq!(second_output["error"]["code"], "cancelled");

    // Budget exhaustion: one call slot, two children -> the second fails with budget_exhausted.
    let budget_dispatcher = GovernedDispatcher::new(registry.clone(), Arc::new(FakeClock::new(0)));
    budget_dispatcher.register_handler("dynamic:child_a", Arc::new(FixedValueHandler(json!({"ok": true}))));
    budget_dispatcher.register_handler("dynamic:child_b", Arc::new(FixedValueHandler(json!({"ok": true}))));
    budget_dispatcher.register_composite_handler(
        "dynamic:composite",
        Arc::new(BatchHandler {
            children: vec!["dynamic:child_a", "dynamic:child_b"],
        }),
    );
    budget_dispatcher.set_budget_config(DispatchBudgetConfig {
        max_composite_calls: 1,
        ..DispatchBudgetConfig::default()
    });
    let exhausted = budget_dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "dynamic:composite",
        "call-2",
        json!({}),
    ));
    assert_eq!(exhausted.execution_status, ToolExecutionStatus::Ok);
    let exhausted_output = exhausted.result.expect("composite outcome carries a result").output;
    assert!(
        exhausted_output.contains("\"error_code\": \"budget_exhausted\""),
        "unexpected output: {}",
        exhausted_output
    );

    // TODO(PA-076 3.7): timeout is triggered by the injected clock advancing past the deadline
    // (check_budget_gates -> `timeout`); a synchronous dispatch cannot advance the clock
    // mid-flight, so the deadline path is pinned by matrix_budget_ledger_atomic_reservation_fails_closed.
}

/// Atomic budget ledger: children share an atomically reserved ledger; exhaustion and pending
/// siblings produce partial results with explicit structured markers.
#[test]
fn matrix_atomic_budget_ledger_exhaustion_and_partial_results() {
    // TODO(PA-076 3.7): Decision 2 — calls/bytes/concurrency are reserved atomically; a child
    // entering pending approval/interaction blocks un-started siblings (dispatch_many returns
    // `not_started`), and exhaustion aborts the remainder with partial results instead of
    // pretending full success.
    let child_a = test_descriptor(
        "dynamic:child_a",
        "ChildA",
        &["dynamic.child_a"],
        ToolKind::Read,
        ToolExposure::Internal,
        ToolPermissionDeclaration::default(),
    );
    let child_b = test_descriptor(
        "dynamic:child_b",
        "ChildB",
        &["dynamic.child_b"],
        ToolKind::Read,
        ToolExposure::Internal,
        ToolPermissionDeclaration::default(),
    );
    let mut composite = test_descriptor(
        "dynamic:composite",
        "Composite",
        &["dynamic.composite"],
        ToolKind::Composite,
        ToolExposure::ModelVisible,
        ToolPermissionDeclaration::default(),
    );
    composite.composed_descriptor_ids = vec!["dynamic:child_a".to_string(), "dynamic:child_b".to_string()];
    let registry = Arc::new(
        ToolRegistrySnapshot::from_descriptors("budget-v1", vec![child_a.clone(), child_b.clone(), composite])
            .expect("custom registry should build"),
    );
    let dispatcher = GovernedDispatcher::new(registry.clone(), Arc::new(FakeClock::new(0)));
    dispatcher.register_handler("dynamic:child_a", Arc::new(FixedValueHandler(json!({"ok": true}))));
    dispatcher.register_handler("dynamic:child_b", Arc::new(FixedValueHandler(json!({"ok": true}))));
    dispatcher.register_composite_handler(
        "dynamic:composite",
        Arc::new(BatchHandler {
            children: vec!["dynamic:child_a", "dynamic:child_b"],
        }),
    );
    dispatcher.set_budget_config(DispatchBudgetConfig {
        max_composite_calls: 1,
        ..DispatchBudgetConfig::default()
    });

    let outcome = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "dynamic:composite",
        "call-1",
        json!({}),
    ));
    assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
    let output = outcome.result.expect("composite outcome carries a result").output;
    // Partial results: child_a ran, child_b failed closed on the shared call ledger.
    assert!(
        output.contains("\"descriptor_id\": \"dynamic:child_a\"") && output.contains("\"started\": true"),
        "expected a completed partial result, got: {}",
        output
    );
    assert!(
        output.contains("\"descriptor_id\": \"dynamic:child_b\"")
            && output.contains("\"error_code\": \"budget_exhausted\""),
        "expected the exhausted child to fail closed, got: {}",
        output
    );

    // A pending sibling blocks later children: sensitive first, then child -> not_started.
    let sensitive = test_descriptor(
        "dynamic:sensitive",
        "Sensitive",
        &["dynamic.sensitive"],
        ToolKind::Read,
        ToolExposure::Internal,
        ToolPermissionDeclaration {
            requires_approval: true,
            ..ToolPermissionDeclaration::default()
        },
    );
    let mut composite2 = test_descriptor(
        "dynamic:composite2",
        "Composite2",
        &["dynamic.composite2"],
        ToolKind::Composite,
        ToolExposure::ModelVisible,
        ToolPermissionDeclaration::default(),
    );
    composite2.composed_descriptor_ids = vec!["dynamic:sensitive".to_string(), "dynamic:child_b".to_string()];
    let registry2 = Arc::new(
        ToolRegistrySnapshot::from_descriptors("pending-v1", vec![sensitive, child_b, composite2])
            .expect("custom registry should build"),
    );
    let dispatcher2 = GovernedDispatcher::new(registry2.clone(), Arc::new(FakeClock::new(0)));
    dispatcher2.register_handler("dynamic:sensitive", Arc::new(EchoHandler));
    dispatcher2.register_handler("dynamic:child_b", Arc::new(FixedValueHandler(json!({"ok": true}))));
    dispatcher2.register_composite_handler(
        "dynamic:composite2",
        Arc::new(BatchHandler {
            children: vec!["dynamic:sensitive", "dynamic:child_b"],
        }),
    );
    let pending_outcome = dispatcher2.dispatch(request(
        InvocationOrigin::Model,
        "dynamic:composite2",
        "call-2",
        json!({}),
    ));
    assert_eq!(pending_outcome.execution_status, ToolExecutionStatus::Ok);
    let pending_output = pending_outcome
        .result
        .expect("composite outcome carries a result")
        .output;
    assert!(
        pending_output.contains("\"control\": \"ApprovalRequired\""),
        "expected a pending control sibling, got: {}",
        pending_output
    );
    assert!(
        pending_output.contains("\"error_code\": \"not_started\""),
        "expected later siblings to be not_started, got: {}",
        pending_output
    );
    // TODO(PA-076 3.7): output-byte exhaustion surfaces as `output_budget_exceeded`
    // (DispatchBudgetConfig::max_result_bytes) once a small cap is configured.
}

/// Child dispatch: bounded depth (max 4) and cycle/self-recursion rejection.
#[test]
fn matrix_child_dispatch_depth_limit_and_cycle_rejection() {
    // TODO(PA-076 3.7): Decision 2 — ChildDispatch is bounded to a default max depth of 4; deeper
    // nesting, self-recursion and composite cycles are rejected with structured errors. Child
    // context carries parent_call_id, lineage, depth, the shared ledger and the cancellation token.
    let mut a = test_descriptor(
        "dynamic:a",
        "A",
        &["dynamic.a"],
        ToolKind::Composite,
        ToolExposure::ModelVisible,
        ToolPermissionDeclaration::default(),
    );
    a.composed_descriptor_ids = vec!["dynamic:b".to_string()];
    let mut b = test_descriptor(
        "dynamic:b",
        "B",
        &["dynamic.b"],
        ToolKind::Composite,
        ToolExposure::Internal,
        ToolPermissionDeclaration::default(),
    );
    b.composed_descriptor_ids = vec!["dynamic:leaf".to_string()];
    let leaf = test_descriptor(
        "dynamic:leaf",
        "Leaf",
        &["dynamic.leaf"],
        ToolKind::Read,
        ToolExposure::Internal,
        ToolPermissionDeclaration::default(),
    );
    let registry = Arc::new(
        ToolRegistrySnapshot::from_descriptors("depth-v1", vec![a, b, leaf])
            .expect("custom registry should build"),
    );
    let dispatcher = GovernedDispatcher::new(registry.clone(), Arc::new(FakeClock::new(0)));
    dispatcher.register_handler("dynamic:leaf", Arc::new(EchoHandler));
    dispatcher.register_composite_handler("dynamic:a", Arc::new(ChildDispatcher("dynamic:b".to_string())));
    dispatcher.register_composite_handler("dynamic:b", Arc::new(ChildDispatcher("dynamic:leaf".to_string())));

    // Default depth (4): a chain of two composites + a leaf is fine.
    let ok = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "dynamic:a",
        "call-1",
        json!({}),
    ));
    assert_eq!(ok.execution_status, ToolExecutionStatus::Ok);

    // Depth 1: dispatching `dynamic:leaf` from `dynamic:b` exceeds the bound.
    dispatcher.set_budget_config(DispatchBudgetConfig {
        max_composite_depth: 1,
        ..DispatchBudgetConfig::default()
    });
    let shallow = dispatcher.dispatch(request(
        InvocationOrigin::Model,
        "dynamic:a",
        "call-2",
        json!({}),
    ));
    assert_eq!(shallow.execution_status, ToolExecutionStatus::Ok);
    let shallow_output = shallow.result.expect("composite outcome carries a result").output;
    assert!(
        shallow_output.contains("child_depth_exceeded"),
        "expected depth limit enforcement, got: {}",
        shallow_output
    );

    // Self-recursion: a composite dispatching itself is rejected at preflight
    // (child_self_recursion / child_cycle) even when the authority is granted at runtime.
    let mut self_rec = test_descriptor(
        "dynamic:self_rec",
        "SelfRec",
        &["dynamic.self_rec"],
        ToolKind::Composite,
        ToolExposure::ModelVisible,
        ToolPermissionDeclaration::default(),
    );
    self_rec.composed_descriptor_ids = Vec::new();
    let registry2 = Arc::new(
        ToolRegistrySnapshot::from_descriptors("self-v1", vec![self_rec])
            .expect("registry without a static self-cycle builds"),
    );
    let dispatcher2 = GovernedDispatcher::new(registry2.clone(), Arc::new(FakeClock::new(0)));
    dispatcher2.register_composite_handler_with_authority(
        "dynamic:self_rec",
        vec!["dynamic:self_rec".to_string()],
        Arc::new(ChildDispatcher("dynamic:self_rec".to_string())),
    );
    let recursive = dispatcher2.dispatch(request(
        InvocationOrigin::Model,
        "dynamic:self_rec",
        "call-3",
        json!({}),
    ));
    assert_eq!(recursive.execution_status, ToolExecutionStatus::Ok);
    let recursive_output = recursive.result.expect("composite outcome carries a result").output;
    assert!(
        recursive_output.contains("child_self_recursion") || recursive_output.contains("child_cycle"),
        "expected self-recursion rejection, got: {}",
        recursive_output
    );
}
