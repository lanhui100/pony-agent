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
use crate::agent::dispatcher::ControlRequestAuthorization;
use crate::agent::graph::GraphAskWaitBinding;
use crate::agent::plan_state::{Plan, PlanPayload, PlanStepSpec};
use serde_json::{json, Value};

impl HostControlPlane {
    // ── Ask control surface (design.md Decision 5) ────────────────────────────────────────

    /// Snapshot of every pending `Interaction` (Ask) request, optionally filtered by session.
    pub fn list_pending_asks(&self, session_id: Option<&str>) -> Value {
        let requests = ask_control::list_pending_asks(&self.ask_dispatcher);
        let filtered = requests
            .into_iter()
            .filter(|request| {
                session_id
                    .map(|sid| request.session_id.as_deref() == Some(sid))
                    .unwrap_or(true)
            })
            .filter_map(|request| serde_json::to_value(&request).ok())
            .collect::<Vec<Value>>();
        Value::Array(filtered)
    }

    /// Answer an Ask request by stable `request_id` with a compare-and-swap `expected_version`.
    /// A stale version (the request was already consumed, cancelled, or expired) fails closed
    /// before the dispatcher CAS runs.
    pub fn answer_ask(
        &self,
        request_id: &str,
        expected_version: u64,
        answer: Value,
    ) -> Result<Value, String> {
        let pending = self
            .ask_dispatcher
            .pending_request(request_id)
            .ok_or_else(|| format!("unknown control request `{request_id}`"))?;
        require_current_version(&pending, expected_version)?;
        let authorization = ControlRequestAuthorization::for_request(&pending, Some(answer));
        let consumed = ask_control::answer_ask(&self.ask_dispatcher, request_id, &authorization)?;
        serde_json::to_value(&consumed)
            .map_err(|error| format!("failed to serialize answer result: {error}"))
    }

    /// Cancel an Ask request by stable `request_id` with a compare-and-swap `expected_version`.
    pub fn cancel_ask(
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
        let consumed = ask_control::cancel_ask(&self.ask_dispatcher, request_id, &authorization)?;
        serde_json::to_value(&consumed)
            .map_err(|error| format!("failed to serialize cancel result: {error}"))
    }

    /// Transition every expired pending request to `Expired`, returning the count. Defaults to
    /// the dispatcher's injected clock when `now_ms` is not supplied.
    pub fn expire_asks(&self, now_ms: Option<u64>) -> Value {
        let now = now_ms.unwrap_or_else(|| self.ask_dispatcher.clock().now_ms());
        let expired = ask_control::expire_asks(&self.ask_dispatcher, now);
        json!({ "expired": expired, "nowMs": now })
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
