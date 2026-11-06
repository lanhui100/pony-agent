//! ask_plan_commands: Ask (`Interaction`) and Plan host control-plane surface (PA-076 task 4.4).
//!
//! These are the host-agnostic read/write adapters the Tauri command layer calls. Ask
//! list/answer/cancel/expire delegate to `ask_control` (which drives the dispatcher's
//! compare-and-swap control-request store); Plan create/replace/merge/complete-step/list/get
//! delegate to `plan_state::PlanStore`; the graph Ask wait bind/resume surface wires a run's
//! `wait_user` suspension to a `PendingControlRequest` (design.md Decision 5). Everything returns
//! serializable `serde_json::Value` projections so no Tauri type leaks into core.
use super::*;
use crate::agent::ask_control;
use crate::agent::ask_persistence::{AskLifecycleState, PersistedAsk};
use crate::agent::dispatcher::ControlRequestAuthorization;
use crate::agent::graph::{GraphAskResumeInjection, GraphAskWaitBinding};
use crate::agent::plan_state::{Plan, PlanPayload, PlanStepSpec};
use crate::agent::tool_runtime::PendingControlRequestState;
use serde_json::{json, Value};

impl HostControlPlane {
    // ── Ask control surface (design.md Decision 5 / ADR 0074) ────────────────────────────

    /// Snapshot of pending `Interaction` (Ask) requests **inside one session scope**.
    ///
    /// Contract §5/§4: the session scope is mandatory. An unscoped call returns an empty array
    /// (`ask_scope_required` is logged) instead of leaking other sessions' `requestId`/`version`;
    /// the projection never carries `nonce` (a server-held CAS credential). The durable store is
    /// the only source — a request that exists only in a process-local mirror is not "pending".
    pub fn list_pending_asks(&self, session_id: Option<&str>) -> Value {
        let Some(session_id) = session_id.map(str::trim).filter(|scope| !scope.is_empty()) else {
            eprintln!(
                "[pony-agent] ask_list_pending: ask_scope_required (unscoped listing is fail-closed)"
            );
            return Value::Array(Vec::new());
        };
        let Some(store) = self.ask_store.as_ref() else {
            eprintln!(
                "[pony-agent] ask_list_pending: ask_scope_required (host has no durable Ask store)"
            );
            return Value::Array(Vec::new());
        };
        match store.load_pending(session_id) {
            Ok(records) => Value::Array(
                records
                    .iter()
                    .filter(|record| record.session_id == session_id)
                    .map(persisted_ask_projection)
                    .collect(),
            ),
            Err(error) => {
                // Fail closed: an unreadable store must not silently fall back to process memory.
                eprintln!(
                    "[pony-agent] ask_list_pending: read failed for `{session_id}`: {error}"
                );
                Value::Array(Vec::new())
            }
        }
    }

    /// Legacy unscoped entry: **always fails closed** (`ask_scope_required`).
    ///
    /// Contract §4: answering requires a session scope so a record can never be consumed from
    /// another session's context. Kept only for API compatibility; hosts call
    /// [`Self::answer_ask_in_session`]. It never touches the record or the dispatcher mirror.
    pub fn answer_ask(
        &self,
        request_id: &str,
        _expected_version: u64,
        _answer: Value,
    ) -> Result<Value, String> {
        self.unscoped_ask_error("answering", request_id)
    }

    /// Answer an Ask inside an explicit session scope (contract §4 four-tuple CAS).
    ///
    /// `nonce` is **server-held**: it is read from the durable record, never supplied by the
    /// caller. The CAS is `(session_id, request_id, version, nonce, state)` and is committed on
    /// disk first; a missing durable record is `ask_not_found` (no memory-mirror fallback).
    pub fn answer_ask_in_session(
        &self,
        session_id: &str,
        request_id: &str,
        expected_version: u64,
        answer: Value,
    ) -> Result<Value, String> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(format!(
                "ask_scope_required: answering `{request_id}` requires a non-empty session scope"
            ));
        }
        let store = self.ask_store.as_ref().ok_or_else(|| {
            "ask_scope_required: host has no durable Ask store; refusing memory-only answer"
                .to_string()
        })?;
        let record = store
            .load_ask(session_id, request_id)?
            .filter(|record| record.session_id == session_id)
            .ok_or_else(|| {
                format!("ask_not_found: no persisted Ask `{request_id}` in session `{session_id}`")
            })?;
        let updated = store.transition_scoped(
            session_id,
            request_id,
            expected_version,
            &record.nonce,
            AskLifecycleState::Pending,
            AskLifecycleState::Answered,
        )?;
        let mut pre_transition = updated.clone();
        pre_transition.version = expected_version;
        let mirror = match self
            .ask_dispatcher
            .pending_request(request_id)
            .filter(|pending| pending.session_id.as_deref() == Some(session_id))
        {
            Some(pending) => pending,
            None => self
                .ask_dispatcher
                .hydrate_persisted_ask(&pre_transition, PendingControlRequestState::Pending),
        };
        let authorization = ControlRequestAuthorization::for_request(&mirror, Some(answer.clone()));
        let consumed = match ask_control::answer_ask(&self.ask_dispatcher, request_id, &authorization)
        {
            Ok(consumed) => consumed,
            Err(error) => {
                // Durable-first compensation (T8): the record is already `Answered` on disk, so a
                // failing dispatcher CAS must not be reported as "nothing happened". Make the
                // mirror state truthful and surface the divergence instead of a bogus success.
                self.ask_dispatcher
                    .hydrate_persisted_ask(&updated, PendingControlRequestState::Consumed);
                eprintln!(
                    "[pony-agent] ask answer: durable commit succeeded but dispatcher mirror sync \
                     failed for `{request_id}`: {error}"
                );
                return Err(format!(
                    "ask_mirror_sync_failed: answer was persisted ({:?}) but the runtime mirror \
                     rejected the CAS: {error}",
                    updated.state
                ));
            }
        };
        self.land_answered_ask(
            &updated.session_id,
            &updated.request_id,
            updated.run_id.as_deref(),
            &updated.turn_id,
            &updated.call_id,
            expected_version,
            &answer,
        );
        let mut value = serde_json::to_value(&consumed)
            .map_err(|error| format!("failed to serialize answer result: {error}"))?;
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "state".to_string(),
                json!(consumed.request.state.clone()),
            );
            object.insert("lifecycleState".to_string(), json!("answered"));
            object.insert("answeredAtMs".to_string(), json!(updated.answered_at_ms));
            object.insert("sessionId".to_string(), json!(updated.session_id));
        }
        Ok(value)
    }

    /// Approve an Approval control request by stable `request_id` with a compare-and-swap `expected_version`.
    pub fn approve_control_request(
        &self,
        request_id: &str,
        expected_version: u64,
    ) -> Result<Value, String> {
        let pending = self
            .ask_dispatcher
            .pending_request(request_id)
            .ok_or_else(|| format!("unknown control request `{request_id}`"))?;
        require_current_version(&pending, expected_version)?;
        let authorization = ControlRequestAuthorization::for_request(&pending, None);
        let consumed = self
            .ask_dispatcher
            .approve_control_request(request_id, &authorization)?;
        serde_json::to_value(&consumed)
            .map_err(|error| format!("failed to serialize approve result: {error}"))
    }

    /// Legacy unscoped entry: **always fails closed** (`ask_scope_required`).
    pub fn cancel_ask(&self, request_id: &str, _expected_version: u64) -> Result<Value, String> {
        self.unscoped_ask_error("cancelling", request_id)
    }

    /// Cancel (explicit ignore) an Ask inside an explicit session scope.
    ///
    /// Same four-tuple CAS as [`Self::answer_ask_in_session`]; the durable record moves to
    /// `Cancelled` (never `Superseded`/`Orphaned`).
    pub fn cancel_ask_in_session(
        &self,
        session_id: &str,
        request_id: &str,
        expected_version: u64,
    ) -> Result<Value, String> {
        let session_id = session_id.trim();
        if session_id.is_empty() {
            return Err(format!(
                "ask_scope_required: cancelling `{request_id}` requires a non-empty session scope"
            ));
        }
        let store = self.ask_store.as_ref().ok_or_else(|| {
            "ask_scope_required: host has no durable Ask store; refusing memory-only cancel"
                .to_string()
        })?;
        let record = store
            .load_ask(session_id, request_id)?
            .filter(|record| record.session_id == session_id)
            .ok_or_else(|| {
                format!("ask_not_found: no persisted Ask `{request_id}` in session `{session_id}`")
            })?;
        let updated = store.transition_scoped(
            session_id,
            request_id,
            expected_version,
            &record.nonce,
            AskLifecycleState::Pending,
            AskLifecycleState::Cancelled,
        )?;
        let mut pre_transition = updated.clone();
        pre_transition.version = expected_version;
        let mirror = match self
            .ask_dispatcher
            .pending_request(request_id)
            .filter(|pending| pending.session_id.as_deref() == Some(session_id))
        {
            Some(pending) => pending,
            None => self
                .ask_dispatcher
                .hydrate_persisted_ask(&pre_transition, PendingControlRequestState::Pending),
        };
        let authorization = ControlRequestAuthorization::for_request(&mirror, None);
        let consumed = match ask_control::cancel_ask(&self.ask_dispatcher, request_id, &authorization)
        {
            Ok(consumed) => consumed,
            Err(error) => {
                self.ask_dispatcher
                    .hydrate_persisted_ask(&updated, PendingControlRequestState::Cancelled);
                eprintln!(
                    "[pony-agent] ask cancel: durable commit succeeded but dispatcher mirror sync \
                     failed for `{request_id}`: {error}"
                );
                return Err(format!(
                    "ask_mirror_sync_failed: cancel was persisted ({:?}) but the runtime mirror \
                     rejected the CAS: {error}",
                    updated.state
                ));
            }
        };
        let mut value = serde_json::to_value(&consumed)
            .map_err(|error| format!("failed to serialize cancel result: {error}"))?;
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "state".to_string(),
                json!(consumed.request.state.clone()),
            );
            object.insert("lifecycleState".to_string(), json!("cancelled"));
            object.insert("sessionId".to_string(), json!(updated.session_id));
        }
        Ok(value)
    }

    /// Answer landing (ADR 0074 §5): with a graph ask-wait binding the client's
    /// `graph_resume_ask` resumes the run; **without** one the answer is queued as a one-shot
    /// session injection that the next turn consumes via `take_pending_ask_injection`, so an
    /// answer is never silently dropped.
    #[allow(clippy::too_many_arguments)]
    fn land_answered_ask(
        &self,
        session_id: &str,
        request_id: &str,
        run_id: Option<&str>,
        turn_id: &str,
        call_id: &str,
        answered_version: u64,
        answer: &Value,
    ) {
        let mut graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
            eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
            e.into_inner()
        });
        let bound = run_id
            .map(|run_id| {
                self.graph_runner
                    .list_ask_waits(&graph_runs, run_id)
                    .iter()
                    .any(|wait| wait.request_id == request_id)
            })
            .unwrap_or(false);
        if bound {
            return;
        }
        let injection = GraphAskResumeInjection {
            request_id: request_id.to_string(),
            run_id: run_id.unwrap_or_default().to_string(),
            turn_id: turn_id.to_string(),
            call_id: call_id.to_string(),
            tool_name: "builtin:ask_user".to_string(),
            assistant_transcript: Value::Null,
            terminal_result: json!({
                "toolCallId": call_id,
                "toolName": "builtin:ask_user",
                "status": "ok",
                "output": {
                    "ok": true,
                    "requestId": request_id,
                    "version": answered_version,
                    "kind": "waiting_user",
                    "answer": answer,
                },
            }),
        };
        graph_runs.queue_ask_injection_for_session(session_id, injection);
        eprintln!(
            "[pony-agent] ask answer landed: source=session session={session_id} request={request_id} call={call_id}"
        );
    }

    /// Stable fail-closed error for an unscoped legacy mutation.
    ///
    /// Contract §4: a session scope is mandatory. To keep the rejection diagnosable the durable
    /// store is consulted **read-only**: a record owned by some session yields
    /// `ask_state_conflict` (plus the `ask_scope_required` hint), no record at all yields
    /// `ask_not_found`. No mutation and no mirror consumption ever happens on this path.
    fn unscoped_ask_error(&self, action: &str, request_id: &str) -> Result<Value, String> {
        let scope_hint = format!(
            "ask_scope_required: {action} `{request_id}` requires a session scope (use the \
             *_in_session entry point)"
        );
        match self.ask_store.as_ref() {
            None => Err(scope_hint),
            Some(store) => match store.find_ask(request_id) {
                Ok(Some(record)) => Err(format!(
                    "ask_state_conflict: Ask `{request_id}` belongs to session `{}` and cannot be \
                     {action} outside that scope; {scope_hint}",
                    record.session_id
                )),
                Ok(None) => Err(format!(
                    "ask_not_found: no persisted Ask `{request_id}`; {scope_hint}"
                )),
                Err(error) => Err(format!("{scope_hint}; durable lookup failed: {error}")),
            },
        }
    }

    /// Retired as a product verdict — this is now **GC only** (ADR 0074 §5).
    ///
    /// Time alone never reclaims a pending Ask: only archived/deleted sessions produce `Orphaned`
    /// records. `now_ms` defaults to the dispatcher clock and is passed to the GC pass as
    /// bookkeeping only. `expired` still reports Approval TTL reconciliation (unchanged non-goal).
    pub fn expire_asks(&self, now_ms: Option<u64>) -> Value {
        let now = now_ms.unwrap_or_else(|| self.ask_dispatcher.clock().now_ms());
        let archived = self.archived_session_ids();
        let orphaned = self.gc_orphan_asks(&archived, now);
        let expired = ask_control::expire_asks(&self.ask_dispatcher, now);
        json!({ "expired": expired, "orphaned": orphaned, "nowMs": now })
    }

    /// Record that a user message entered a session: every still-pending Ask of that session is
    /// superseded. Called from the host turn entry points (plain + graph, streaming + non-streaming)
    /// before the turn runs, so the message that makes an older question moot retires it.
    pub(crate) fn note_user_message_entry(&self, input: &crate::agent::runtime::TurnInput) {
        if input.message.trim().is_empty() {
            return;
        }
        let session_id = input
            .session_id
            .clone()
            .unwrap_or_else(|| crate::agent::session::DEFAULT_SESSION_ID.to_string());
        self.supersede_pending_asks_for_session(&session_id);
    }

    /// A new user message entered the session: every still-pending Ask of that session becomes
    /// `Superseded` (ADR 0074 §3). Conversation order, not the wall clock, is the staleness signal.
    pub fn supersede_pending_asks_for_session(&self, session_id: &str) -> usize {
        let at_ms = self.ask_dispatcher.clock().now_ms();
        let mut superseded = 0;
        if let Some(store) = self.ask_store.as_ref() {
            match store.supersede_pending_on_user_message(session_id, at_ms) {
                Ok(moved) => superseded = moved,
                Err(error) => eprintln!(
                    "[pony-agent] supersede_pending_asks_for_session `{session_id}` failed: {error}"
                ),
            }
        }
        self.ask_dispatcher
            .close_pending_asks_of_session(session_id);
        superseded
    }

    /// GC pass for archived/deleted sessions: their pending Asks become `Orphaned` (never
    /// `Expired`). Returns how many records were reclaimed.
    pub(crate) fn gc_orphan_asks(&self, session_ids: &[String], now_ms: u64) -> usize {
        if session_ids.is_empty() {
            return 0;
        }
        let mut orphaned = 0;
        if let Some(store) = self.ask_store.as_ref() {
            match store.gc_orphans(session_ids, now_ms) {
                Ok(moved) => orphaned = moved,
                Err(error) => eprintln!("[pony-agent] gc_orphan_asks failed: {error}"),
            }
        }
        for session_id in session_ids {
            self.ask_dispatcher
                .close_pending_asks_of_session(session_id);
        }
        orphaned
    }

    /// Ids of sessions currently archived (the only source of `Orphaned`).
    fn archived_session_ids(&self) -> Vec<String> {
        self.sessions_rwlock
            .read()
            .unwrap_or_else(|e| {
                eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
                e.into_inner()
            })
            .list_sessions()
            .into_iter()
            .filter(|overview| overview.archived)
            .map(|overview| overview.conversation_id)
            .collect()
    }

    // ── Plan control surface (design.md Decision 6) ───────────────────────────────────────

    /// Create a session-owned `Draft` plan from a `PlanPayload` projection.
    pub fn plan_create(&self, session_id: &str, payload: Value) -> Result<Value, String> {
        let payload: PlanPayload = serde_json::from_value(payload)
            .map_err(|error| format!("invalid_request: malformed plan payload: {error}"))?;
        let plan = self
            .plan_store
            .create(session_id, payload)
            .map_err(|error| error.to_message())?;
        plan_value(&plan)
    }

    /// Replace a plan's content, preserving its `plan_id`, with a compare-and-swap `revision`.
    pub fn plan_replace(
        &self,
        session_id: &str,
        plan_id: &str,
        revision: u64,
        payload: Value,
    ) -> Result<Value, String> {
        let payload: PlanPayload = serde_json::from_value(payload)
            .map_err(|error| format!("invalid_request: malformed plan payload: {error}"))?;
        let plan = self
            .plan_store
            .replace(session_id, plan_id, revision, payload)
            .map_err(|error| error.to_message())?;
        plan_value(&plan)
    }

    /// Append a single step to a plan, preserving every existing `step_id`.
    pub fn plan_merge(
        &self,
        session_id: &str,
        plan_id: &str,
        revision: u64,
        step: Value,
    ) -> Result<Value, String> {
        let step: PlanStepSpec = serde_json::from_value(step)
            .map_err(|error| format!("invalid_request: malformed plan step: {error}"))?;
        let plan = self
            .plan_store
            .merge(session_id, plan_id, revision, step)
            .map_err(|error| error.to_message())?;
        plan_value(&plan)
    }

    /// Mark a plan step completed; the first completion moves the plan to `Executing` and the
    /// last moves it to `Completed`.
    pub fn plan_complete_step(
        &self,
        session_id: &str,
        plan_id: &str,
        revision: u64,
        step_id: &str,
    ) -> Result<Value, String> {
        let plan = self
            .plan_store
            .complete_step(session_id, plan_id, revision, step_id)
            .map_err(|error| error.to_message())?;
        plan_value(&plan)
    }

    /// Explicitly approve execution of a Draft plan from host, transitioning it to `Executing`.
    pub fn plan_execution_approve(
        &self,
        session_id: &str,
        plan_id: &str,
        revision: u64,
    ) -> Result<Value, String> {
        let plan = self
            .plan_store
            .start_execution(session_id, plan_id, revision)
            .map_err(|error| error.to_message())?;
        plan_value(&plan)
    }

    /// Every plan owned by `session_id`, in creation order.
    pub fn plan_list(&self, session_id: &str) -> Value {
        let plans = self.plan_store.plans_for_session(session_id);
        Value::Array(
            plans
                .iter()
                .filter_map(|plan| serde_json::to_value(plan).ok())
                .collect(),
        )
    }

    /// Read a single plan by stable id (fails closed on cross-session or unknown plan).
    pub fn plan_get(&self, session_id: &str, plan_id: &str) -> Result<Value, String> {
        let plan = self
            .plan_store
            .plan(session_id, plan_id)
            .map_err(|error| error.to_message())?;
        plan_value(&plan)
    }

    // ── graph Ask wait surface (design.md Decision 5) ─────────────────────────────────────

    /// Snapshot of every Ask wait currently bound to a graph run.
    pub fn graph_list_ask_waits(&self, run_id: &str) -> Value {
        let graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
            eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
            e.into_inner()
        });
        let waits = self.graph_runner.list_ask_waits(&graph_runs, run_id);
        serde_json::to_value(&waits).unwrap_or_else(|_| Value::Array(Vec::new()))
    }

    /// Bind an Ask `PendingControlRequest` to a run's `wait_user` suspension.
    pub fn graph_bind_ask_wait(
        &self,
        run_id: &str,
        binding: GraphAskWaitBinding,
    ) -> Result<Value, String> {
        let mut graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
            eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
            e.into_inner()
        });
        let run = self
            .graph_runner
            .bind_ask_wait(&mut graph_runs, run_id, binding)?;
        serde_json::to_value(&run)
            .map_err(|error| format!("failed to serialize bound graph run: {error}"))
    }

    /// Resolve a bound Ask wait and inject exactly one terminal tool result for the original
    /// call id, moving the run back to `Ready`. A stale version is rejected. The dispatcher's
    /// pending request must already have been CAS-consumed (through `answer_ask` / `cancel_ask`):
    /// resuming a request that is still `Pending` would let an arbitrary answer bypass the single
    /// consumption entry point (phase-4 P1-2).
    pub fn graph_resume_ask(
        &self,
        run_id: &str,
        request_id: &str,
        expected_version: u64,
        answer: Value,
    ) -> Result<Value, String> {
        let pending = self
            .ask_dispatcher
            .pending_request(request_id)
            .ok_or_else(|| format!("unknown control request `{request_id}`"))?;
        if pending.state != crate::agent::tool_runtime::PendingControlRequestState::Consumed {
            return Err(format!(
                "Ask `{request_id}` cannot resume: the pending control request has not been \
                 CAS-consumed (state {:?}); answer or cancel it first.",
                pending.state
            ));
        }
        let mut graph_runs = self.graph_runs.lock().unwrap_or_else(|e| {
            eprintln!("[pony-agent] graph run lock poisoned: {e}, recovering");
            e.into_inner()
        });
        // ADR 0074: an answered Ask with no graph wait binding does not resume a run — the answer
        // already landed through the session-scoped injection (`answer_ask`). Report the landing
        // instead of surfacing a host-internal failure to the client.
        let bound = self
            .graph_runner
            .list_ask_waits(&graph_runs, run_id)
            .iter()
            .any(|wait| wait.request_id == request_id);
        if !bound {
            let landed = pending
                .session_id
                .as_deref()
                .map(|session_id| {
                    graph_runs
                        .peek_ask_injection_for_session(session_id)
                        .map(|injection| injection.request_id == request_id)
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            if landed {
                return Ok(json!({
                    "requestId": request_id,
                    "runId": run_id,
                    "resumed": false,
                    "landed": "session_injection",
                }));
            }
        }
        let outcome = self.graph_runner.resume_ask_wait(
            &mut graph_runs,
            run_id,
            request_id,
            expected_version,
            answer,
        )?;
        serde_json::to_value(&outcome)
            .map_err(|error| format!("failed to serialize ask resume outcome: {error}"))
    }
}

/// Fail-closed stale-version gate used by every Ask mutation. The pending request's version is
/// the CAS truth; a caller presenting any other version cannot be the version the binding was
/// created with.
fn require_current_version(pending: &crate::agent::tool_runtime::PendingControlRequest, expected_version: u64) -> Result<(), String> {
    if pending.version != expected_version {
        return Err(format!(
            "Ask `{}` expected version {expected_version} but current version is {}; stale mutation rejected.",
            pending.request_id, pending.version
        ));
    }
    Ok(())
}

fn plan_value(plan: &Plan) -> Result<Value, String> {
    serde_json::to_value(plan)
        .map_err(|error| format!("handler_error: plan projection serialization failed: {error}"))
}

/// Wire projection of a durable Ask record for the host panel.
///
/// Mirrors the frozen `PersistedAsk` camelCase shape and additionally carries `requestKind` (the
/// key every host adapter — TUI + desktop — already reads) plus the lifecycle fields, so a panel
/// rebuilt from disk renders exactly like one built from the in-memory mirror.
fn persisted_ask_projection(record: &PersistedAsk) -> Value {
    let kind = serde_json::to_value(&record.kind).unwrap_or_else(|_| json!("interaction"));
    json!({
        "requestId": record.request_id,
        "version": record.version,
        "sessionId": record.session_id,
        "runId": record.run_id,
        "turnId": record.turn_id,
        "callId": record.call_id,
        "requestKind": kind,
        "kind": kind,
        "prompt": record.prompt,
        "options": record.options,
        "state": record.state,
        "lifecycleState": record.state,
        "createdAtMs": record.created_at_ms,
        "answeredAtMs": record.answered_at_ms,
        "persisted": true,
    })
}
