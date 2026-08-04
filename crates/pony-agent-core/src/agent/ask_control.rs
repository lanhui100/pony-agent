//! Host-facing Ask (`Interaction`) control adapter (PA-076 phase 4, tasks 4.3-4.4).
//!
//! A thin adapter over a [`GovernedDispatcher`] for the Ask control path (design.md Decision 5).
//! `Ask` shares the [`PendingControlRequest`] lifecycle with approvals but is distinguished by
//! `PendingControlRequestKind::Interaction`; answering an Ask must never equal approving a tool.
//!
//! The module never reconstructs CAS facts by hand: [`ControlRequestAuthorization::for_request`]
//! derives a valid authorization from a pending request (session/version/nonce/descriptor/
//! digests), and every consume path delegates to the dispatcher's compare-and-swap machinery,
//! which rejects replay, expiry, cross-session, version/nonce, descriptor, and kind mismatches.

use crate::agent::dispatcher::{
    ControlRequestAuthorization, ControlRequestConsumed, GovernedDispatcher,
};
use crate::agent::tool_runtime::{PendingControlRequest, PendingControlRequestKind};
use serde_json::Value;

/// Snapshot of every pending Ask (`Interaction`) request currently awaiting a host answer.
pub fn list_pending_asks(dispatcher: &GovernedDispatcher) -> Vec<PendingControlRequest> {
    filter_interaction_requests(dispatcher.pending_requests())
}

/// Keep only `Interaction`-kind pending requests. Exposed separately so the filtering contract is
/// directly testable and reusable by host adapters.
pub(crate) fn filter_interaction_requests(
    requests: Vec<PendingControlRequest>,
) -> Vec<PendingControlRequest> {
    requests
        .into_iter()
        .filter(|request| request.request_kind == PendingControlRequestKind::Interaction)
        .collect()
}

/// Answer an Ask request. Delegates to `dispatcher.answer_control_request`, which enforces the
/// `Interaction` request kind, the session/version/nonce CAS facts, the descriptor snapshot/id,
/// and the final-arguments/policy digests.
pub fn answer_ask(
    dispatcher: &GovernedDispatcher,
    request_id: &str,
    authorization: &ControlRequestAuthorization,
) -> Result<ControlRequestConsumed, String> {
    dispatcher.answer_control_request(request_id, authorization)
}

/// Cancel an Ask request (any pending request kind may be cancelled).
pub fn cancel_ask(
    dispatcher: &GovernedDispatcher,
    request_id: &str,
    authorization: &ControlRequestAuthorization,
) -> Result<ControlRequestConsumed, String> {
    dispatcher.cancel_control_request(request_id, authorization)
}

/// Transition every expired pending request to `Expired`, returning the number transitioned.
/// Expiry is kind-agnostic: an unanswered Ask is just as dead as an unhandled approval.
pub fn expire_asks(dispatcher: &GovernedDispatcher, now_ms: u64) -> usize {
    dispatcher.expire_control_requests(now_ms)
}

impl ControlRequestAuthorization {
    /// Build a valid compare-and-swap authorization from a pending request so host adapters never
    /// reconstruct session/version/nonce/descriptor/digest facts by hand. `answer` carries the
    /// user's answer payload for `Interaction` requests.
    pub fn for_request(pending: &PendingControlRequest, answer: Option<Value>) -> Self {
        ControlRequestAuthorization {
            session_id: pending.session_id.clone(),
            expected_version: pending.version,
            nonce: pending.nonce.clone(),
            expected_descriptor_snapshot_id: Some(pending.descriptor_snapshot_id.clone()),
            expected_descriptor_id: Some(pending.descriptor_id.clone()),
            expected_final_args_digest: Some(pending.final_args_digest.clone()),
            expected_policy_digest: Some(pending.policy_digest.clone()),
            answer,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::dispatcher::DispatchContext;
    use crate::agent::tool_runtime::{
        FakeClock, InvocationOrigin, PendingControlRequestState, RuntimeClock, ToolDispatchRequest,
    };
    use crate::agent::tools::{
        ToolControlKind, ToolDescriptor, ToolDescriptorSource, ToolDisplayMetadata,
        ToolExecutionPolicy, ToolExposure, ToolHandlerProvenance, ToolIdentity, ToolKind,
        ToolPermissionDeclaration, ToolRegistrySnapshot,
    };
    use serde_json::json;
    use std::sync::Arc;

    fn pending_request(kind: PendingControlRequestKind, request_id: &str) -> PendingControlRequest {
        PendingControlRequest {
            request_id: request_id.to_string(),
            request_kind: kind,
            session_id: Some("session-1".to_string()),
            run_id: Some("run-1".to_string()),
            turn_id: "turn-1".to_string(),
            call_id: "call-1".to_string(),
            descriptor_snapshot_id: "snapshot-1".to_string(),
            descriptor_id: "builtin:ask".to_string(),
            final_args_digest: "args-digest".to_string(),
            policy_digest: "policy-digest".to_string(),
            nonce: format!("nonce-{request_id}"),
            version: 1,
            expires_at_ms: 10_000,
            state: PendingControlRequestState::Pending,
            prompt: Some("continue?".to_string()),
            options: Some(json!(["yes", "no"])),
        }
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
            aliases: vec![id.to_string()],
            description: String::new(),
            input_schema: json!({ "type": "object" }),
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

    fn approval_declaration() -> ToolPermissionDeclaration {
        let mut declaration = ToolPermissionDeclaration::default();
        declaration.requires_approval = true;
        declaration
    }

    /// Persist a control request through the real dispatcher by dispatching an approval-required
    /// descriptor. Today the dispatcher records every pending control request as
    /// `PendingControlRequestKind::Approval` — for both approval and host-mediation verdicts —
    /// so the Ask (`Interaction`) persistence is a phase-4 runtime wiring item (design.md
    /// Decision 5) rather than a dispatcher behavior we may change here.
    fn persist_request(dispatcher: &GovernedDispatcher) -> PendingControlRequest {
        let context = DispatchContext {
            session_id: Some("session-1".to_string()),
            ..Default::default()
        };
        let outcome = dispatcher.dispatch_governed(
            ToolDispatchRequest {
                origin: InvocationOrigin::Model,
                descriptor_id: "builtin:ask".to_string(),
                call_id: "call-1".to_string(),
                arguments: json!({ "text": "x" }),
            },
            &context,
        );
        let request_id = outcome
            .control_outcome
            .expect("control outcome")
            .request_id;
        dispatcher
            .pending_request(&request_id)
            .expect("persisted pending request")
    }

    // ── filtering ───────────────────────────────────────────────────────────────────────────

    #[test]
    fn filter_pending_asks_returns_only_interaction_requests() {
        let ask = pending_request(PendingControlRequestKind::Interaction, "ask-1");
        let approval = pending_request(PendingControlRequestKind::Approval, "approve-1");
        let filtered = filter_interaction_requests(vec![ask.clone(), approval.clone()]);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].request_id, "ask-1");
        assert_eq!(filtered[0].request_kind, PendingControlRequestKind::Interaction);
    }

    #[test]
    fn list_pending_asks_filters_the_real_dispatcher_store() {
        let dispatcher = dispatcher_for(registry(vec![descriptor_with_declaration(
            "builtin:ask",
            ToolKind::Interactive,
            ToolExposure::ModelVisible,
            approval_declaration(),
        )]));
        let pending = persist_request(&dispatcher);
        assert_eq!(pending.request_kind, PendingControlRequestKind::Approval);
        assert_eq!(dispatcher.pending_requests().len(), 1);
        // The Ask adapter surfaces only Interaction-kind requests. The dispatcher currently
        // persists Approval-kind requests for approval and host-mediation verdicts, so nothing
        // is listed until the phase-4 runtime persists Interaction for the Ask path.
        assert!(list_pending_asks(&dispatcher).is_empty());
    }

    // ── authorization builder ───────────────────────────────────────────────────────────────

    #[test]
    fn for_request_builds_an_authorization_that_consumes_the_pending_request() {
        let ask = pending_request(PendingControlRequestKind::Interaction, "ask-1");
        let authorization = ControlRequestAuthorization::for_request(&ask, Some(json!("continue")));
        assert_eq!(authorization.session_id.as_deref(), Some("session-1"));
        assert_eq!(authorization.expected_version, ask.version);
        assert_eq!(authorization.nonce, ask.nonce);
        assert_eq!(
            authorization.expected_descriptor_snapshot_id.as_deref(),
            Some("snapshot-1")
        );
        assert_eq!(
            authorization.expected_descriptor_id.as_deref(),
            Some("builtin:ask")
        );
        assert_eq!(
            authorization.expected_final_args_digest.as_deref(),
            Some("args-digest")
        );
        assert_eq!(
            authorization.expected_policy_digest.as_deref(),
            Some("policy-digest")
        );
        assert_eq!(authorization.answer, Some(json!("continue")));
        // The derived authorization satisfies the pending request's own CAS predicate.
        assert!(ask.can_consume(
            authorization.session_id.as_deref(),
            authorization.expected_version,
            &authorization.nonce,
            ask.expires_at_ms,
        ));
    }

    // ── answer / cancel / expire through the real dispatcher ────────────────────────────────

    #[test]
    fn answer_ask_rejects_cas_violations_on_the_real_dispatcher() {
        let dispatcher = dispatcher_for(registry(vec![descriptor_with_declaration(
            "builtin:ask",
            ToolKind::Interactive,
            ToolExposure::ModelVisible,
            approval_declaration(),
        )]));
        let pending = persist_request(&dispatcher);
        let request_id = pending.request_id.clone();

        let wrong_session = {
            let mut authorization = ControlRequestAuthorization::for_request(&pending, None);
            authorization.session_id = Some("session-2".to_string());
            authorization
        };
        assert!(
            answer_ask(&dispatcher, &request_id, &wrong_session).is_err(),
            "cross-session answer must fail closed"
        );

        let wrong_version = {
            let mut authorization = ControlRequestAuthorization::for_request(&pending, None);
            authorization.expected_version = pending.version + 1;
            authorization
        };
        assert!(answer_ask(&dispatcher, &request_id, &wrong_version).is_err());

        let wrong_nonce = {
            let mut authorization = ControlRequestAuthorization::for_request(&pending, None);
            authorization.nonce = "wrong-nonce".to_string();
            authorization
        };
        assert!(answer_ask(&dispatcher, &request_id, &wrong_nonce).is_err());

        // The request is still pending and untouched after the rejected attempts.
        assert_eq!(
            dispatcher.pending_request(&request_id).expect("stored").state,
            PendingControlRequestState::Pending
        );
    }

    #[test]
    fn answer_ask_rejects_approval_kind_requests() {
        let dispatcher = dispatcher_for(registry(vec![descriptor_with_declaration(
            "builtin:ask",
            ToolKind::Interactive,
            ToolExposure::ModelVisible,
            approval_declaration(),
        )]));
        let pending = persist_request(&dispatcher);
        let authorization =
            ControlRequestAuthorization::for_request(&pending, Some(json!("yes")));
        let error = answer_ask(&dispatcher, &pending.request_id, &authorization)
            .expect_err("answering an Approval request must be kind-rejected");
        assert!(error.contains("kind mismatch"), "{error}");
        assert_eq!(
            dispatcher.pending_request(&pending.request_id).expect("stored").state,
            PendingControlRequestState::Pending
        );
    }

    #[test]
    fn cancel_ask_cancels_and_cannot_be_replayed() {
        let dispatcher = dispatcher_for(registry(vec![descriptor_with_declaration(
            "builtin:ask",
            ToolKind::Interactive,
            ToolExposure::ModelVisible,
            approval_declaration(),
        )]));
        let pending = persist_request(&dispatcher);
        let authorization = ControlRequestAuthorization::for_request(&pending, None);
        let consumed =
            cancel_ask(&dispatcher, &pending.request_id, &authorization).expect("cancel");
        assert_eq!(consumed.request.state, PendingControlRequestState::Cancelled);
        assert!(cancel_ask(&dispatcher, &pending.request_id, &authorization).is_err());
    }

    #[test]
    fn expire_asks_transitions_expired_requests_and_returns_the_count() {
        let clock = Arc::new(FakeClock::new(1_000));
        let registry = registry(vec![descriptor_with_declaration(
            "builtin:ask",
            ToolKind::Interactive,
            ToolExposure::ModelVisible,
            approval_declaration(),
        )]);
        let dispatcher = GovernedDispatcher::new(registry, clock.clone() as Arc<dyn RuntimeClock>);
        let pending = persist_request(&dispatcher);

        assert_eq!(expire_asks(&dispatcher, 1_000), 0);
        clock.set_now_ms(pending.expires_at_ms + 1);
        assert_eq!(expire_asks(&dispatcher, pending.expires_at_ms + 1), 1);
        assert_eq!(
            dispatcher.pending_request(&pending.request_id).expect("stored").state,
            PendingControlRequestState::Expired
        );
        // Expired requests can no longer be answered or cancelled.
        let authorization = ControlRequestAuthorization::for_request(&pending, None);
        assert!(answer_ask(&dispatcher, &pending.request_id, &authorization).is_err());
        assert!(cancel_ask(&dispatcher, &pending.request_id, &authorization).is_err());
    }

    /// Full Ask happy path against the real dispatcher store: dispatch a host-mediated (Ask)
    /// descriptor, persist an Interaction-kind pending request, list it, answer it, and verify
    /// the CAS state transition. This was `#[ignore]`d until the dispatcher recorded `Interaction`
    /// for `WaitingHost` verdicts (design.md Decision 5); that wiring landed in the phase-4
    /// integration pass, so the test now runs and drives the real store end-to-end.
    #[test]
    fn answer_ask_consumes_an_interaction_request_via_the_real_dispatcher() {
        let mut declaration = ToolPermissionDeclaration::default();
        declaration.host_mediated = true;
        let dispatcher = dispatcher_for(registry(vec![descriptor_with_declaration(
            "builtin:ask",
            ToolKind::Interactive,
            ToolExposure::ModelVisible,
            declaration,
        )]));
        let context = DispatchContext {
            session_id: Some("session-1".to_string()),
            ..Default::default()
        };
        let outcome = dispatcher.dispatch_governed(
            ToolDispatchRequest {
                origin: InvocationOrigin::Model,
                descriptor_id: "builtin:ask".to_string(),
                call_id: "call-1".to_string(),
                arguments: json!({ "text": "x" }),
            },
            &context,
        );
        let control = outcome.control_outcome.expect("waiting host control outcome");
        assert_eq!(control.kind, ToolControlKind::WaitingHost);

        let asks = list_pending_asks(&dispatcher);
        assert_eq!(asks.len(), 1);
        let pending = asks.into_iter().next().expect("one pending ask");
        assert_eq!(pending.request_kind, PendingControlRequestKind::Interaction);

        let authorization =
            ControlRequestAuthorization::for_request(&pending, Some(json!("continue")));
        let consumed = answer_ask(&dispatcher, &pending.request_id, &authorization)
            .expect("answer consumes the ask");
        assert_eq!(consumed.request.state, PendingControlRequestState::Consumed);
        assert_eq!(consumed.answer, Some(json!("continue")));
    }
}
