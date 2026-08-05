//! Session-owned, revisioned `Plan` control state (PA-076 phase 4, task 4.1).
//!
//! Design.md Decision 6: a `Plan` is a *state control* — it never executes arbitrary child
//! calls. A plan is owned by a session, has a stable `plan_id`, a `revision`, a kind/summary,
//! ordered steps (each with a stable `step_id`, name, summary, and status), and a lifecycle
//! (`Draft` / `Executing` / `Completed` / `Aborted`).
//!
//! Every mutation is compare-and-swap on the plan revision:
//!
//! - a stale `expected_revision` fails with [`PlanError::StaleRevision`];
//! - an unknown plan or step fails with [`PlanError::NotFound`];
//! - an operation from a session that does not own the plan fails with
//!   [`PlanError::CrossSession`];
//! - an illegal lifecycle transition — including completing an already-completed step or
//!   mutating a `Completed`/`Aborted` plan — fails with [`PlanError::IllegalTransition`].
//!
//! [`PlanControlHandler`] adapts a [`PlanStore`] to `PrimitiveToolHandler` so the Plan tool's
//! input schema is exactly `{ create | replace | merge | complete_step }` operations. There is no
//! generic `calls` array and no child execution. `abort` remains a store-level API so the
//! runtime/graph can terminate a plan directly without exposing extra tool surface.

use crate::agent::tool_runtime::{PrimitiveToolHandler, PrimitiveToolHandlerRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Lifecycle of a session-owned plan.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanLifecycleState {
    Draft,
    Executing,
    Completed,
    Aborted,
}

/// Status of an individual plan step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// A single ordered step of a plan. `step_id` is stable for the lifetime of the step; the store
/// assigns it on `create`/`replace`/`merge` and never reuses or rewrites it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStep {
    pub step_id: String,
    pub name: String,
    #[serde(default)]
    pub summary: String,
    pub status: PlanStepStatus,
}

/// A session-owned, revisioned plan. Serializes directly to the plan's JSON projection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub plan_id: String,
    pub session_id: String,
    /// Bumped on every successful mutation; every mutation is CAS on this value.
    pub revision: u64,
    pub kind: String,
    #[serde(default)]
    pub summary: String,
    pub lifecycle: PlanLifecycleState,
    pub steps: Vec<PlanStep>,
}

/// Creation/replacement payload. `steps` are step *specs*; the store assigns stable `step_id`s.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanPayload {
    pub kind: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub steps: Vec<PlanStepSpec>,
}

/// Step description supplied by `create`/`replace`/`merge`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStepSpec {
    pub name: String,
    #[serde(default)]
    pub summary: String,
}

/// Fail-closed plan control errors. `code()` returns the stable error code; `to_message()`
/// formats it as `code: detail` so the dispatcher's `handler_failure` lifts it into a structured
/// `error.code` value instead of collapsing to `handler_error`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    NotFound(String),
    StaleRevision {
        plan_id: String,
        expected_revision: u64,
        actual_revision: u64,
    },
    CrossSession {
        plan_id: String,
        session_id: String,
    },
    IllegalTransition(String),
    /// No authoritative session was injected by the dispatch context. Plan ownership must come
    /// from the dispatcher, never from model-supplied arguments (PA-076 phase-7 review P1-2).
    MissingSession,
    /// The model-supplied `session_id` conflicts with the dispatcher-injected session. The
    /// injected session is authoritative; a self-claimed key is a cross-session forgery attempt.
    SessionClaimMismatch {
        claimed: String,
        authoritative: String,
    },
}

impl PlanError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound(_) => "not_found",
            Self::StaleRevision { .. } => "stale_revision",
            Self::CrossSession { .. } => "cross_session",
            Self::IllegalTransition(_) => "illegal_transition",
            Self::MissingSession => "missing_session",
            Self::SessionClaimMismatch { .. } => "session_claim_mismatch",
        }
    }

    pub fn detail(&self) -> String {
        match self {
            Self::NotFound(detail) => detail.clone(),
            Self::StaleRevision {
                plan_id,
                expected_revision,
                actual_revision,
            } => format!(
                "plan `{plan_id}` expected revision {expected_revision} but current revision is {actual_revision}"
            ),
            Self::CrossSession { plan_id, session_id } => format!(
                "plan `{plan_id}` is owned by session `{session_id}`"
            ),
            Self::IllegalTransition(detail) => detail.clone(),
            Self::MissingSession => format!(
                "no authoritative session was injected by the dispatch context; plan ownership must come from the dispatcher, not model-supplied arguments"
            ),
            Self::SessionClaimMismatch {
                claimed,
                authoritative,
            } => format!(
                "model-supplied session `{claimed}` conflicts with the dispatcher-injected session `{authoritative}`"
            ),
        }
    }

    /// `code: detail` message suitable for the dispatcher's structured error lifting.
    pub fn to_message(&self) -> String {
        format!("{}: {}", self.code(), self.detail())
    }
}

#[derive(Debug, Default)]
struct PlanStoreInner {
    /// session_id -> plan_id -> plan
    plans: BTreeMap<String, BTreeMap<String, Plan>>,
}

/// Session-keyed store of revisioned plans. `create`/`replace`/`merge`/`complete_step`/`abort`
/// all take an explicit session so a caller can never operate on a plan owned by another session
/// without the store failing closed with [`PlanError::CrossSession`].
#[derive(Clone, Debug)]
pub struct PlanStore {
    inner: Arc<Mutex<PlanStoreInner>>,
    plan_seq: Arc<AtomicU64>,
    step_seq: Arc<AtomicU64>,
}

impl Default for PlanStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PlanStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(PlanStoreInner::default())),
            plan_seq: Arc::new(AtomicU64::new(0)),
            step_seq: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Create a `Draft` plan owned by `session_id`. The returned plan has `revision == 1` and
    /// stable `plan_id`/`step_id`s.
    pub fn create(&self, session_id: &str, payload: PlanPayload) -> Result<Plan, PlanError> {
        let mut inner = self.inner.lock().expect("plan store poisoned");
        let plan_id = self.next_plan_seq();
        let steps = payload
            .steps
            .into_iter()
            .map(|spec| PlanStep {
                step_id: self.next_step_seq(),
                name: spec.name,
                summary: spec.summary,
                status: PlanStepStatus::Pending,
            })
            .collect();
        let plan = Plan {
            plan_id,
            session_id: session_id.to_string(),
            revision: 1,
            kind: payload.kind,
            summary: payload.summary,
            lifecycle: PlanLifecycleState::Draft,
            steps,
        };
        inner
            .plans
            .entry(session_id.to_string())
            .or_default()
            .insert(plan.plan_id.clone(), plan.clone());
        Ok(plan)
    }

    /// Read a plan by stable id.
    pub fn plan(&self, session_id: &str, plan_id: &str) -> Result<Plan, PlanError> {
        let inner = self.inner.lock().expect("plan store poisoned");
        self.resolve(&inner, session_id, plan_id).cloned()
    }

    /// Every plan owned by `session_id`, in creation order.
    pub fn plans_for_session(&self, session_id: &str) -> Vec<Plan> {
        let inner = self.inner.lock().expect("plan store poisoned");
        inner
            .plans
            .get(session_id)
            .into_iter()
            .flat_map(|plans| plans.values().cloned())
            .collect()
    }

    /// Replace a plan's kind/summary/steps, preserving its `plan_id`. Steps are rebuilt with new
    /// stable ids and reset to `Pending`. Allowed while the plan is `Draft` or `Executing`.
    pub fn replace(
        &self,
        session_id: &str,
        plan_id: &str,
        expected_revision: u64,
        payload: PlanPayload,
    ) -> Result<Plan, PlanError> {
        let mut inner = self.inner.lock().expect("plan store poisoned");
        let plan = self.locate_mut(&mut inner, session_id, plan_id)?;
        self.require_revision(plan, expected_revision)?;
        self.require_active(plan)?;
        plan.kind = payload.kind;
        plan.summary = payload.summary;
        plan.steps = payload
            .steps
            .into_iter()
            .map(|spec| PlanStep {
                step_id: self.next_step_seq(),
                name: spec.name,
                summary: spec.summary,
                status: PlanStepStatus::Pending,
            })
            .collect();
        plan.revision = plan.revision.saturating_add(1);
        Ok(plan.clone())
    }

    /// Append a single step to the plan, preserving every existing `step_id`.
    pub fn merge(
        &self,
        session_id: &str,
        plan_id: &str,
        expected_revision: u64,
        step: PlanStepSpec,
    ) -> Result<Plan, PlanError> {
        let mut inner = self.inner.lock().expect("plan store poisoned");
        let plan = self.locate_mut(&mut inner, session_id, plan_id)?;
        self.require_revision(plan, expected_revision)?;
        self.require_active(plan)?;
        plan.steps.push(PlanStep {
            step_id: self.next_step_seq(),
            name: step.name,
            summary: step.summary,
            status: PlanStepStatus::Pending,
        });
        plan.revision = plan.revision.saturating_add(1);
        Ok(plan.clone())
    }

    /// Mark a step `Completed`. Completing the first step moves the plan from `Draft` to
    /// `Executing`; completing the last step moves it to `Completed`. Completing an
    /// already-completed step (duplicate) or a step of a `Completed`/`Aborted` plan is an
    /// [`PlanError::IllegalTransition`].
    pub fn complete_step(
        &self,
        session_id: &str,
        plan_id: &str,
        expected_revision: u64,
        step_id: &str,
    ) -> Result<Plan, PlanError> {
        let mut inner = self.inner.lock().expect("plan store poisoned");
        let plan = self.locate_mut(&mut inner, session_id, plan_id)?;
        self.require_revision(plan, expected_revision)?;
        self.require_active(plan)?;
        let step = plan
            .steps
            .iter_mut()
            .find(|step| step.step_id == step_id)
            .ok_or_else(|| {
                PlanError::NotFound(format!("plan `{plan_id}` has no step `{step_id}`"))
            })?;
        if step.status == PlanStepStatus::Completed {
            return Err(PlanError::IllegalTransition(format!(
                "step `{step_id}` is already completed; a step may complete only once"
            )));
        }
        step.status = PlanStepStatus::Completed;
        if plan.lifecycle == PlanLifecycleState::Draft {
            plan.lifecycle = PlanLifecycleState::Executing;
        }
        if plan.steps.iter().all(|step| step.status == PlanStepStatus::Completed) {
            plan.lifecycle = PlanLifecycleState::Completed;
        }
        plan.revision = plan.revision.saturating_add(1);
        Ok(plan.clone())
    }

    /// Abort a `Draft` or `Executing` plan. Aborting an already-`Completed`/`Aborted` plan is an
    /// [`PlanError::IllegalTransition`].
    pub fn abort(
        &self,
        session_id: &str,
        plan_id: &str,
        expected_revision: u64,
    ) -> Result<Plan, PlanError> {
        let mut inner = self.inner.lock().expect("plan store poisoned");
        let plan = self.locate_mut(&mut inner, session_id, plan_id)?;
        self.require_revision(plan, expected_revision)?;
        if matches!(
            plan.lifecycle,
            PlanLifecycleState::Completed | PlanLifecycleState::Aborted
        ) {
            return Err(PlanError::IllegalTransition(format!(
                "plan `{plan_id}` is already {lifecycle:?}; it cannot be aborted again",
                lifecycle = plan.lifecycle
            )));
        }
        plan.lifecycle = PlanLifecycleState::Aborted;
        plan.revision = plan.revision.saturating_add(1);
        Ok(plan.clone())
    }

    fn next_plan_seq(&self) -> String {
        let seq = self.plan_seq.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        format!("plan-{seq}")
    }

    fn next_step_seq(&self) -> String {
        let seq = self.step_seq.fetch_add(1, Ordering::Relaxed).saturating_add(1);
        format!("step-{seq}")
    }

    fn resolve<'a>(
        &self,
        inner: &'a PlanStoreInner,
        session_id: &str,
        plan_id: &str,
    ) -> Result<&'a Plan, PlanError> {
        match inner.plans.get(session_id).and_then(|plans| plans.get(plan_id)) {
            Some(plan) => Ok(plan),
            None => Err(Self::ownership_error(inner, session_id, plan_id)),
        }
    }

    fn locate_mut<'a>(
        &self,
        inner: &'a mut PlanStoreInner,
        session_id: &str,
        plan_id: &str,
    ) -> Result<&'a mut Plan, PlanError> {
        // Determine ownership failure before taking the mutable borrow so the error path never
        // coexists with a live `&mut` derived from `inner`.
        let present = inner
            .plans
            .get(session_id)
            .is_some_and(|plans| plans.contains_key(plan_id));
        if !present {
            return Err(Self::ownership_error(inner, session_id, plan_id));
        }
        Ok(inner
            .plans
            .get_mut(session_id)
            .and_then(|plans| plans.get_mut(plan_id))
            .expect("plan presence verified above"))
    }

    fn ownership_error(
        inner: &PlanStoreInner,
        session_id: &str,
        plan_id: &str,
    ) -> PlanError {
        let cross_session = inner
            .plans
            .iter()
            .any(|(owner, plans)| owner != session_id && plans.contains_key(plan_id));
        if cross_session {
            PlanError::CrossSession {
                plan_id: plan_id.to_string(),
                session_id: session_id.to_string(),
            }
        } else {
            PlanError::NotFound(format!("no plan `{plan_id}` for session `{session_id}`"))
        }
    }

    fn require_revision(&self, plan: &Plan, expected_revision: u64) -> Result<(), PlanError> {
        if plan.revision != expected_revision {
            return Err(PlanError::StaleRevision {
                plan_id: plan.plan_id.clone(),
                expected_revision,
                actual_revision: plan.revision,
            });
        }
        Ok(())
    }

    fn require_active(&self, plan: &Plan) -> Result<(), PlanError> {
        if matches!(
            plan.lifecycle,
            PlanLifecycleState::Completed | PlanLifecycleState::Aborted
        ) {
            return Err(PlanError::IllegalTransition(format!(
                "plan `{}` is {lifecycle:?}; content mutations are no longer allowed",
                plan.plan_id,
                lifecycle = plan.lifecycle
            )));
        }
        Ok(())
    }
}

/// One Plan-tool operation. The schema is exactly `{ create | replace | merge | complete_step }`
/// — there is deliberately no generic `calls` array and no child execution.
#[derive(Debug, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum PlanControlOperation {
    Create { session_id: String, payload: PlanPayload },
    Replace {
        session_id: String,
        plan_id: String,
        revision: u64,
        payload: PlanPayload,
    },
    Merge {
        session_id: String,
        plan_id: String,
        revision: u64,
        step: PlanStepSpec,
    },
    CompleteStep {
        session_id: String,
        plan_id: String,
        revision: u64,
        step_id: String,
    },
}

/// Host-agnostic `PrimitiveToolHandler` that maps Plan-tool arguments into [`PlanStore`]
/// operations and returns the updated plan's JSON projection.
#[derive(Clone, Debug)]
pub struct PlanControlHandler {
    store: PlanStore,
}

impl PlanControlHandler {
    pub fn new(store: PlanStore) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &PlanStore {
        &self.store
    }

    /// Execute a Plan-tool operation under an authoritative session. `session_id` must be the
    /// dispatch-context-injected session (`PrimitiveToolHandlerRequest.session_id`); a `None` here
    /// fails closed with [`PlanError::MissingSession`]. The model-supplied `session_id` embedded in
    /// the operation is validated against the injected session and never trusted as the ownership
    /// key (PA-076 phase-7 review P1-2).
    pub fn execute_operation(
        &self,
        session_id: Option<&str>,
        operation: PlanControlOperation,
    ) -> Result<Plan, PlanError> {
        let Some(session_id) = session_id else {
            return Err(PlanError::MissingSession);
        };
        let operation = self.authorize_operation(session_id, operation)?;
        match operation {
            PlanControlOperation::Create { session_id, payload } => {
                self.store.create(&session_id, payload)
            }
            PlanControlOperation::Replace {
                session_id,
                plan_id,
                revision,
                payload,
            } => self.store.replace(&session_id, &plan_id, revision, payload),
            PlanControlOperation::Merge {
                session_id,
                plan_id,
                revision,
                step,
            } => self.store.merge(&session_id, &plan_id, revision, step),
            PlanControlOperation::CompleteStep {
                session_id,
                plan_id,
                revision,
                step_id,
            } => self.store.complete_step(&session_id, &plan_id, revision, &step_id),
        }
    }

    /// Validate the model-supplied `session_id` against the authoritative injected session and
    /// rewrite the operation so every store call is keyed by the injected session. A self-claimed
    /// key that differs from the injected session is a cross-session forgery attempt and fails
    /// closed with [`PlanError::SessionClaimMismatch`].
    fn authorize_operation(
        &self,
        authoritative: &str,
        operation: PlanControlOperation,
    ) -> Result<PlanControlOperation, PlanError> {
        match operation {
            PlanControlOperation::Create { session_id, payload } => {
                self.require_claimed_session(authoritative, &session_id)?;
                Ok(PlanControlOperation::Create {
                    session_id: authoritative.to_string(),
                    payload,
                })
            }
            PlanControlOperation::Replace {
                session_id,
                plan_id,
                revision,
                payload,
            } => {
                self.require_claimed_session(authoritative, &session_id)?;
                Ok(PlanControlOperation::Replace {
                    session_id: authoritative.to_string(),
                    plan_id,
                    revision,
                    payload,
                })
            }
            PlanControlOperation::Merge {
                session_id,
                plan_id,
                revision,
                step,
            } => {
                self.require_claimed_session(authoritative, &session_id)?;
                Ok(PlanControlOperation::Merge {
                    session_id: authoritative.to_string(),
                    plan_id,
                    revision,
                    step,
                })
            }
            PlanControlOperation::CompleteStep {
                session_id,
                plan_id,
                revision,
                step_id,
            } => {
                self.require_claimed_session(authoritative, &session_id)?;
                Ok(PlanControlOperation::CompleteStep {
                    session_id: authoritative.to_string(),
                    plan_id,
                    revision,
                    step_id,
                })
            }
        }
    }

    fn require_claimed_session(&self, authoritative: &str, claimed: &str) -> Result<(), PlanError> {
        if claimed != authoritative {
            return Err(PlanError::SessionClaimMismatch {
                claimed: claimed.to_string(),
                authoritative: authoritative.to_string(),
            });
        }
        Ok(())
    }
}

impl PrimitiveToolHandler for PlanControlHandler {
    fn execute(&self, request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
        let operation: PlanControlOperation =
            serde_json::from_value(request.arguments.clone()).map_err(|error| {
                format!("invalid_request: malformed plan control arguments: {error}")
            })?;
        let plan = self
            .execute_operation(request.session_id.as_deref(), operation)
            .map_err(|error| error.to_message())?;
        serde_json::to_value(&plan)
            .map_err(|error| format!("handler_error: plan projection serialization failed: {error}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload(kind: &str, steps: &[(&str, &str)]) -> PlanPayload {
        PlanPayload {
            kind: kind.to_string(),
            summary: format!("{kind} summary"),
            steps: steps
                .iter()
                .map(|(name, summary)| PlanStepSpec {
                    name: name.to_string(),
                    summary: summary.to_string(),
                })
                .collect(),
        }
    }

    fn handler_request(arguments: Value) -> PrimitiveToolHandlerRequest {
        handler_request_for_session(arguments, Some("session-1"))
    }

    fn handler_request_for_session(
        arguments: Value,
        session_id: Option<&str>,
    ) -> PrimitiveToolHandlerRequest {
        PrimitiveToolHandlerRequest {
            descriptor_id: "builtin:plan_control".to_string(),
            arguments,
            session_id: session_id.map(ToString::to_string),
        }
    }

    #[test]
    fn create_builds_a_draft_plan_with_stable_ids_and_revision_one() {
        let store = PlanStore::new();
        let plan = store
            .create("session-1", payload("implement", &[("a", "sa"), ("b", "sb")]))
            .expect("create");
        assert_eq!(plan.plan_id, "plan-1");
        assert_eq!(plan.session_id, "session-1");
        assert_eq!(plan.revision, 1);
        assert_eq!(plan.lifecycle, PlanLifecycleState::Draft);
        assert_eq!(plan.steps.len(), 2);
        assert_eq!(plan.steps[0].step_id, "step-1");
        assert_eq!(plan.steps[0].status, PlanStepStatus::Pending);
        assert_eq!(plan.steps[1].step_id, "step-2");

        // Stable read-back and per-session projection.
        assert_eq!(store.plan("session-1", "plan-1").expect("plan"), plan);
        assert_eq!(store.plans_for_session("session-1").len(), 1);
        assert!(store.plans_for_session("session-2").is_empty());
    }

    #[test]
    fn replace_rebuilds_content_and_bumps_revision_keeping_plan_id() {
        let store = PlanStore::new();
        let created = store
            .create("session-1", payload("implement", &[("a", "sa")]))
            .expect("create");
        let replaced = store
            .replace(
                "session-1",
                &created.plan_id,
                1,
                payload("refactor", &[("x", "sx"), ("y", "sy")]),
            )
            .expect("replace");
        assert_eq!(replaced.plan_id, created.plan_id);
        assert_eq!(replaced.revision, 2);
        assert_eq!(replaced.kind, "refactor");
        assert_eq!(replaced.lifecycle, PlanLifecycleState::Draft);
        assert_eq!(replaced.steps.len(), 2);
        assert!(replaced.steps.iter().all(|s| s.status == PlanStepStatus::Pending));
    }

    #[test]
    fn merge_appends_a_step_and_preserves_existing_step_ids() {
        let store = PlanStore::new();
        let created = store
            .create("session-1", payload("implement", &[("a", "sa")]))
            .expect("create");
        let merged = store
            .merge(
                "session-1",
                &created.plan_id,
                1,
                PlanStepSpec {
                    name: "b".to_string(),
                    summary: "sb".to_string(),
                },
            )
            .expect("merge");
        assert_eq!(merged.revision, 2);
        assert_eq!(merged.steps.len(), 2);
        assert_eq!(merged.steps[0].step_id, created.steps[0].step_id);
        assert_eq!(merged.steps[1].name, "b");
        assert_eq!(merged.steps[1].status, PlanStepStatus::Pending);
    }

    #[test]
    fn complete_step_advances_lifecycle_and_completes_the_plan_when_all_steps_are_done() {
        let store = PlanStore::new();
        let created = store
            .create("session-1", payload("implement", &[("a", "sa"), ("b", "sb")]))
            .expect("create");
        let first = store
            .complete_step("session-1", &created.plan_id, 1, &created.steps[0].step_id)
            .expect("complete a");
        assert_eq!(first.revision, 2);
        assert_eq!(first.lifecycle, PlanLifecycleState::Executing);
        assert_eq!(first.steps[0].status, PlanStepStatus::Completed);
        assert_eq!(first.steps[1].status, PlanStepStatus::Pending);

        let second = store
            .complete_step("session-1", &created.plan_id, 2, &created.steps[1].step_id)
            .expect("complete b");
        assert_eq!(second.revision, 3);
        assert_eq!(second.lifecycle, PlanLifecycleState::Completed);
        assert!(second.steps.iter().all(|s| s.status == PlanStepStatus::Completed));
    }

    #[test]
    fn complete_step_rejects_duplicate_completion() {
        let store = PlanStore::new();
        // Two steps so completing the first leaves the plan `Executing` (not auto-completed);
        // the duplicate-completion guard on the step itself must fire.
        let created = store
            .create("session-1", payload("implement", &[("a", "sa"), ("b", "sb")]))
            .expect("create");
        store
            .complete_step("session-1", &created.plan_id, 1, &created.steps[0].step_id)
            .expect("first completion");
        let error = store
            .complete_step("session-1", &created.plan_id, 2, &created.steps[0].step_id)
            .expect_err("duplicate completion must fail closed");
        assert_eq!(error.code(), "illegal_transition");
        assert_eq!(
            error,
            PlanError::IllegalTransition(format!(
                "step `{}` is already completed; a step may complete only once",
                created.steps[0].step_id
            ))
        );
    }

    #[test]
    fn complete_step_unknown_step_is_not_found() {
        let store = PlanStore::new();
        let created = store
            .create("session-1", payload("implement", &[("a", "sa")]))
            .expect("create");
        let error = store
            .complete_step("session-1", &created.plan_id, 1, "step-999")
            .expect_err("unknown step must fail closed");
        assert_eq!(error.code(), "not_found");
    }

    #[test]
    fn stale_revision_is_rejected_on_every_mutation() {
        let store = PlanStore::new();
        let created = store
            .create("session-1", payload("implement", &[("a", "sa")]))
            .expect("create");
        let error = store
            .replace("session-1", &created.plan_id, 99, payload("x", &[]))
            .expect_err("stale replace");
        assert_eq!(error.code(), "stale_revision");
        assert_eq!(
            error,
            PlanError::StaleRevision {
                plan_id: created.plan_id.clone(),
                expected_revision: 99,
                actual_revision: 1,
            }
        );
        assert!(store
            .merge(
                "session-1",
                &created.plan_id,
                2,
                PlanStepSpec { name: "b".into(), summary: String::new() },
            )
            .is_err());
        assert!(store
            .complete_step("session-1", &created.plan_id, 0, &created.steps[0].step_id)
            .is_err());
        assert!(store.abort("session-1", &created.plan_id, 5).is_err());
    }

    #[test]
    fn cross_session_operations_fail() {
        let store = PlanStore::new();
        let created = store
            .create("session-1", payload("implement", &[("a", "sa")]))
            .expect("create");
        assert_eq!(
            store.plan("session-2", &created.plan_id).expect_err("cross-session read").code(),
            "cross_session"
        );
        assert_eq!(
            store
                .replace("session-2", &created.plan_id, 1, payload("x", &[]))
                .expect_err("cross-session replace")
                .code(),
            "cross_session"
        );
        assert_eq!(
            store
                .complete_step("session-2", &created.plan_id, 1, &created.steps[0].step_id)
                .expect_err("cross-session complete")
                .code(),
            "cross_session"
        );
        assert_eq!(
            store
                .abort("session-2", &created.plan_id, 1)
                .expect_err("cross-session abort")
                .code(),
            "cross_session"
        );
        // A sibling session can create its own independent plan.
        let sibling = store.create("session-2", payload("other", &[])).expect("create");
        assert_eq!(sibling.session_id, "session-2");
    }

    #[test]
    fn unknown_plan_is_not_found() {
        let store = PlanStore::new();
        assert_eq!(
            store.plan("session-1", "plan-999").expect_err("unknown plan").code(),
            "not_found"
        );
        assert_eq!(
            store
                .abort("session-1", "plan-999", 1)
                .expect_err("unknown abort")
                .code(),
            "not_found"
        );
    }

    #[test]
    fn abort_transitions_and_terminates_the_plan() {
        let store = PlanStore::new();
        let created = store
            .create("session-1", payload("implement", &[("a", "sa")]))
            .expect("create");
        let aborted = store
            .abort("session-1", &created.plan_id, 1)
            .expect("abort");
        assert_eq!(aborted.lifecycle, PlanLifecycleState::Aborted);
        assert_eq!(aborted.revision, 2);
        assert_eq!(
            store
                .abort("session-1", &created.plan_id, 2)
                .expect_err("double abort")
                .code(),
            "illegal_transition"
        );
        assert_eq!(
            store
                .replace("session-1", &created.plan_id, 2, payload("x", &[]))
                .expect_err("replace after abort")
                .code(),
            "illegal_transition"
        );
        assert_eq!(
            store
                .merge(
                    "session-1",
                    &created.plan_id,
                    2,
                    PlanStepSpec { name: "x".into(), summary: String::new() },
                )
                .expect_err("merge after abort")
                .code(),
            "illegal_transition"
        );
    }

    #[test]
    fn completed_plan_rejects_content_mutations() {
        let store = PlanStore::new();
        let created = store
            .create("session-1", payload("implement", &[("a", "sa")]))
            .expect("create");
        let completed = store
            .complete_step("session-1", &created.plan_id, 1, &created.steps[0].step_id)
            .expect("complete");
        assert_eq!(completed.lifecycle, PlanLifecycleState::Completed);
        assert_eq!(
            store
                .replace("session-1", &created.plan_id, 2, payload("x", &[]))
                .expect_err("replace after completion")
                .code(),
            "illegal_transition"
        );
        assert_eq!(
            store
                .merge(
                    "session-1",
                    &created.plan_id,
                    2,
                    PlanStepSpec { name: "x".into(), summary: String::new() },
                )
                .expect_err("merge after completion")
                .code(),
            "illegal_transition"
        );
        assert_eq!(
            store
                .complete_step("session-1", &created.plan_id, 2, &created.steps[0].step_id)
                .expect_err("complete after completion")
                .code(),
            "illegal_transition"
        );
    }

    // ── PlanControlHandler ──────────────────────────────────────────────────────────────────

    #[test]
    fn handler_maps_create_replace_merge_and_complete_step_arguments() {
        let store = PlanStore::new();
        let handler = PlanControlHandler::new(store);

        let created: Value = handler
            .execute(&handler_request(json!({
                "op": "create",
                "session_id": "session-1",
                "payload": {
                    "kind": "implement",
                    "summary": "Implement the feature",
                    "steps": [{ "name": "a", "summary": "sa" }]
                }
            })))
            .expect("create via handler");
        assert_eq!(created["planId"], "plan-1");
        assert_eq!(created["revision"], 1);
        assert_eq!(created["lifecycle"], "draft");
        assert_eq!(created["steps"][0]["name"], "a");
        assert_eq!(created["steps"][0]["status"], "pending");

        let merged: Value = handler
            .execute(&handler_request(json!({
                "op": "merge",
                "session_id": "session-1",
                "plan_id": "plan-1",
                "revision": 1,
                "step": { "name": "b", "summary": "sb" }
            })))
            .expect("merge via handler");
        assert_eq!(merged["revision"], 2);
        assert_eq!(merged["steps"][1]["name"], "b");

        let step_id = merged["steps"][0]["stepId"].as_str().expect("step id").to_string();
        let completed: Value = handler
            .execute(&handler_request(json!({
                "op": "complete_step",
                "session_id": "session-1",
                "plan_id": "plan-1",
                "revision": 2,
                "step_id": step_id
            })))
            .expect("complete via handler");
        assert_eq!(completed["steps"][0]["status"], "completed");
        assert_eq!(completed["lifecycle"], "executing");

        let replaced: Value = handler
            .execute(&handler_request(json!({
                "op": "replace",
                "session_id": "session-1",
                "plan_id": "plan-1",
                "revision": 3,
                "payload": { "kind": "refactor", "summary": "Refactor", "steps": [] }
            })))
            .expect("replace via handler");
        assert_eq!(replaced["planId"], "plan-1");
        assert_eq!(replaced["kind"], "refactor");
        assert_eq!(replaced["revision"], 4);
    }

    #[test]
    fn handler_rejects_malformed_arguments_and_stale_revisions_with_structured_messages() {
        let store = PlanStore::new();
        let handler = PlanControlHandler::new(store);

        // Missing required payload field.
        let malformed = handler
            .execute(&handler_request(json!({ "op": "create", "session_id": "session-1" })))
            .expect_err("malformed create");
        assert!(malformed.starts_with("invalid_request:"), "{malformed}");

        // A generic `calls` array is rejected: the Plan schema has no child execution.
        let generic_calls = handler
            .execute(&handler_request(json!({
                "op": "execute",
                "session_id": "session-1",
                "calls": [{ "name": "workspace_write_file" }]
            })))
            .expect_err("generic calls must be rejected");
        assert!(generic_calls.starts_with("invalid_request:"), "{generic_calls}");

        handler
            .execute(&handler_request(json!({
                "op": "create",
                "session_id": "session-1",
                "payload": { "kind": "k", "summary": "s", "steps": [] }
            })))
            .expect("create");

        // Stale revision surfaces as a `code: detail` message the dispatcher lifts structurally.
        let stale = handler
            .execute(&handler_request(json!({
                "op": "replace",
                "session_id": "session-1",
                "plan_id": "plan-1",
                "revision": 99,
                "payload": { "kind": "k", "summary": "s", "steps": [] }
            })))
            .expect_err("stale replace via handler");
        assert!(stale.starts_with("stale_revision:"), "{stale}");
    }

    #[test]
    fn handler_rejects_model_session_that_conflicts_with_injected_session() {
        let store = PlanStore::new();
        let handler = PlanControlHandler::new(store);

        // The model claims session-2 but the dispatch context injected session-1; the injected
        // session is authoritative and the self-claimed key is a forgery attempt.
        let error = handler
            .execute(&handler_request_for_session(
                json!({
                    "op": "create",
                    "session_id": "session-2",
                    "payload": { "kind": "implement", "summary": "s", "steps": [] }
                }),
                Some("session-1"),
            ))
            .expect_err("conflicting session claim must fail closed");
        assert!(error.starts_with("session_claim_mismatch:"), "{error}");

        // A later operation with a matching claimed session still works under the same injected
        // session.
        handler
            .execute(&handler_request_for_session(
                json!({
                    "op": "create",
                    "session_id": "session-1",
                    "payload": { "kind": "implement", "summary": "s", "steps": [] }
                }),
                Some("session-1"),
            ))
            .expect("matching claimed session succeeds");
    }

    #[test]
    fn handler_fails_closed_when_no_session_is_injected_even_if_model_claims_one() {
        let store = PlanStore::new();
        let handler = PlanControlHandler::new(store);

        // No dispatch-context session; the model's self-claimed session must not grant ownership.
        let error = handler
            .execute(&handler_request_for_session(
                json!({
                    "op": "create",
                    "session_id": "session-1",
                    "payload": { "kind": "implement", "summary": "s", "steps": [] }
                }),
                None,
            ))
            .expect_err("missing injected session must fail closed");
        assert!(error.starts_with("missing_session:"), "{error}");
    }

    #[test]
    fn handler_keys_ownership_by_injected_session_and_blocks_cross_session_access() {
        let store = PlanStore::new();
        let handler = PlanControlHandler::new(store);

        // The plan is created under the injected session-1.
        let created: Value = handler
            .execute(&handler_request_for_session(
                json!({
                    "op": "create",
                    "session_id": "session-1",
                    "payload": {
                        "kind": "implement",
                        "summary": "s",
                        "steps": [{ "name": "a", "summary": "sa" }]
                    }
                }),
                Some("session-1"),
            ))
            .expect("create under injected session-1");
        assert_eq!(created["sessionId"], "session-1");

        // A sibling session (injected session-2, matching its own claim) cannot reach the plan
        // owned by session-1.
        let cross = handler
            .execute(&handler_request_for_session(
                json!({
                    "op": "replace",
                    "session_id": "session-2",
                    "plan_id": "plan-1",
                    "revision": 1,
                    "payload": { "kind": "x", "summary": "y", "steps": [] }
                }),
                Some("session-2"),
            ))
            .expect_err("cross-session replace via injected session");
        assert!(cross.starts_with("cross_session:"), "{cross}");

        // The owner session can still read its own plan.
        let plan = handler
            .store()
            .plan("session-1", "plan-1")
            .expect("owner read");
        assert_eq!(plan.session_id, "session-1");
    }
}
