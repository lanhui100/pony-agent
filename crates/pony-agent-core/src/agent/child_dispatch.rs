//! Bounded child dispatch context (PA-076 phase 3, task 3.3).
//!
//! Composite handlers never receive the full dispatcher or router. They only receive a
//! `ChildDispatch`, which carries parent lineage, the allowed-child authority set, a shared
//! atomically reserved budget ledger, a cancellation token, and the remaining deadline. Child
//! dispatch enforces cycle/self-recursion rejection and a maximum depth before any handler runs.

use crate::agent::budget::{BudgetLedger, CancellationToken};
use crate::agent::dispatcher::{
    build_lifecycle_record, deny_outcome, handler_failure, render_output, CompositeToolHandler,
    CompositeToolHandlerRequest, DispatchContext, DispatchError, DispatchLifecycleRecord,
    PermissionDecision, PermissionVerdict,
};
use crate::agent::tool_runtime::{InvocationOrigin, RuntimeClock};
use crate::agent::tools::{ToolDescriptor, ToolOutcome, ToolRegistrySnapshot, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Hard cap on the number of children one `dispatch_many` call accepts, so a host-registered
/// composite cannot spawn unbounded threads before the shared budget rejects overflow
/// (phase-3 review P2). Composites with their own tighter caps (e.g. `workspace_batch` at 24)
/// are unaffected.
pub const MAX_DISPATCH_MANY_REQUESTS: usize = 256;

/// A single child invocation request issued by a composite handler.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildDispatchRequest {
    pub descriptor_id: String,
    pub call_id: String,
    pub arguments: Value,
}

/// The outcome of one child invocation inside a `dispatch_many` batch.
#[derive(Clone, Debug)]
pub struct ChildDispatchResult {
    pub index: usize,
    pub request: ChildDispatchRequest,
    pub outcome: ToolOutcome,
    /// `true` when the child reached execution; `false` when it was rejected during preflight,
    /// persisted as a control request, denied, or never started because an earlier sibling
    /// entered a pending control state.
    pub started: bool,
}

/// Facts bound to one bounded child-dispatch context.
#[derive(Clone)]
pub struct ChildDispatchContext {
    /// Call id of the composite invocation that owns this context.
    pub parent_call_id: String,
    /// Descriptor id of the composite invocation that owns this context.
    pub parent_descriptor_id: String,
    /// Descriptor ids from the top-level invocation down to and including the owner composite.
    pub lineage: Vec<String>,
    /// Depth of the owner composite; children dispatched through this context execute at
    /// `depth + 1`.
    pub depth: u32,
    /// Descriptor ids this composite is allowed to dispatch as children.
    pub allowed_child_ids: BTreeSet<String>,
    /// Shared atomic budget ledger (calls / concurrency / bytes / deadline).
    pub ledger: Arc<BudgetLedger>,
    /// Shared cancellation token.
    pub cancellation: Arc<CancellationToken>,
    /// Session/run/turn/workspace facts inherited from the top-level invocation.
    pub context: DispatchContext,
    /// Injected clock used for deadline evaluation.
    pub clock: Arc<dyn RuntimeClock>,
    /// Set once any child through this context enters a pending control state. Once set, every
    /// subsequent sibling dispatch returns `not_started` — design.md Decision 2: no un-started
    /// sibling may run while an approval/interaction is outstanding (phase-3 review P1-3).
    pub suspended: Arc<AtomicBool>,
}

impl std::fmt::Debug for ChildDispatchContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ChildDispatchContext")
            .field("parent_call_id", &self.parent_call_id)
            .field("parent_descriptor_id", &self.parent_descriptor_id)
            .field("lineage", &self.lineage)
            .field("depth", &self.depth)
            .field("allowed_child_ids", &self.allowed_child_ids)
            .field("ledger", &self.ledger)
            .field("cancellation", &self.cancellation)
            .field("context", &self.context)
            .field("clock", &"<RuntimeClock>")
            .field("suspended", &self.suspended)
            .finish()
    }
}

/// Result of the authorization half of the child pipeline, produced before any handler runs.
#[derive(Clone, Debug)]
pub(crate) struct PreflightedChild {
    pub descriptor: ToolDescriptor,
    pub final_arguments: Value,
    pub decision: PermissionDecision,
    pub final_args_digest: String,
    pub policy_digest: String,
}

/// The restricted execution port exposed to a `CompositeToolHandler`.
#[derive(Clone)]
pub struct ChildDispatch {
    runner: Arc<dyn ChildDispatchRunner>,
    context: ChildDispatchContext,
}

impl ChildDispatch {
    pub(crate) fn new(
        runner: Arc<dyn ChildDispatchRunner>,
        context: ChildDispatchContext,
    ) -> Self {
        Self { runner, context }
    }

    /// Dispatch one child. Returns the child `ToolOutcome`; dispatch-level failures are already
    /// encoded as `Error`/`Cancelled` outcomes, so `Err` is reserved for internal errors. Once
    /// any sibling through this context has entered a pending control state, subsequent children
    /// return a `not_started` outcome (no un-started sibling may run while approval is pending).
    pub fn dispatch(&self, request: ChildDispatchRequest) -> Result<ToolOutcome, String> {
        let started = Instant::now();
        if self.context.suspended.load(Ordering::Acquire) {
            let outcome = not_started_outcome(&request);
            self.emit_child_record(&request, &outcome, None, started);
            return Ok(outcome);
        }
        match self.runner.preflight(&self.context, &request) {
            Err(error) => {
                let outcome = error.into_outcome(&request.descriptor_id);
                self.emit_child_record(&request, &outcome, None, started);
                Ok(outcome)
            }
            Ok(ready) => {
                let outcome = invoke_child_shared(&self.runner, &self.context, &request, ready.clone());
                self.emit_child_record(&request, &outcome, Some(&ready), started);
                Ok(outcome)
            }
        }
    }

    /// Dispatch a batch of children. Every child completes resolution + final-argument
    /// authorization + permission decision before any handler runs. Once a child enters a
    /// pending control state, un-started siblings are not started (they return a `not_started`
    /// `Cancelled` outcome). Other errors fail only the offending child.
    pub fn dispatch_many(
        &self,
        requests: Vec<ChildDispatchRequest>,
    ) -> Result<Vec<ChildDispatchResult>, String> {
        if requests.len() > MAX_DISPATCH_MANY_REQUESTS {
            return Err(format!(
                "dispatch_many received {} children, exceeding the cap of {MAX_DISPATCH_MANY_REQUESTS}",
                requests.len()
            ));
        }
        let count = requests.len();
        let mut slots: Vec<Option<ChildDispatchResult>> = (0..count).map(|_| None).collect();

        // Pass 1: sequential preflight for every child, stopping preflight once a pending
        // control child is found so later siblings are never started.
        let mut first_pending: Option<usize> = None;
        let mut preflights: Vec<Option<Result<PreflightedChild, DispatchError>>> =
            Vec::with_capacity(count);
        for request in &requests {
            if first_pending.is_some() {
                preflights.push(None);
                continue;
            }
            match self.runner.preflight(&self.context, request) {
                Ok(ready) => {
                    if matches!(
                        ready.decision.verdict,
                        PermissionVerdict::ApprovalRequired | PermissionVerdict::WaitingHost
                    ) {
                        first_pending = Some(preflights.len());
                        self.context.suspended.store(true, Ordering::Release);
                    }
                    preflights.push(Some(Ok(ready)));
                }
                Err(error) => preflights.push(Some(Err(error))),
            }
        }

        let mut launch: Vec<(usize, ChildDispatchRequest, PreflightedChild)> = Vec::new();
        for (index, request) in requests.into_iter().enumerate() {
            let started = Instant::now();
            let slot = match preflights.get(index).cloned().flatten() {
                None => {
                    let outcome = not_started_outcome(&request);
                    ChildDispatchResult {
                        index,
                        request,
                        outcome,
                        started: false,
                    }
                }
                Some(Err(error)) => {
                    let outcome = error.into_outcome(&request.descriptor_id);
                    self.emit_child_record(&request, &outcome, None, started);
                    ChildDispatchResult {
                        index,
                        request,
                        outcome,
                        started: false,
                    }
                }
                Some(Ok(ready)) => {
                    if matches!(
                        ready.decision.verdict,
                        PermissionVerdict::ApprovalRequired | PermissionVerdict::WaitingHost
                    ) {
                        let outcome =
                            invoke_child_shared(&self.runner, &self.context, &request, ready.clone());
                        self.emit_child_record(&request, &outcome, Some(&ready), started);
                        ChildDispatchResult {
                            index,
                            request,
                            outcome,
                            started: false,
                        }
                    } else if ready.decision.verdict == PermissionVerdict::Deny {
                        let outcome =
                            invoke_child_shared(&self.runner, &self.context, &request, ready.clone());
                        self.emit_child_record(&request, &outcome, Some(&ready), started);
                        ChildDispatchResult {
                            index,
                            request,
                            outcome,
                            started: false,
                        }
                    } else {
                        launch.push((index, request, ready));
                        continue;
                    }
                }
            };
            slots[index] = Some(slot);
        }

        // Pass 2: launch the approved children concurrently. The shared ledger keeps call and
        // concurrency reservations atomic, so oversubscription fails closed per child.
        if !launch.is_empty() {
            let runner = Arc::clone(&self.runner);
            let context = self.context.clone();
            std::thread::scope(|scope| {
                let mut handles = Vec::with_capacity(launch.len());
                for (index, request, ready) in launch {
                    let runner = Arc::clone(&runner);
                    let context = context.clone();
                    let request_for_thread = request.clone();
                    handles.push((
                        index,
                        request,
                        scope.spawn(move || {
                            let started = Instant::now();
                            let outcome =
                                invoke_child_shared(&runner, &context, &request_for_thread, ready.clone());
                            let record = build_child_record(&request_for_thread, &outcome, Some(&ready), &context, started);
                            runner.emit(&record);
                            (index, request_for_thread, outcome)
                        }),
                    ));
                }
                for (index, request, handle) in handles {
                    // The closure owns a clone, so the original request stays available here and a
                    // panicking child still reports its real identity (phase-3 review P3).
                    let (index, request, outcome) = handle.join().unwrap_or_else(|_| {
                        (index, request, child_panic_outcome())
                    });
                    slots[index] = Some(ChildDispatchResult {
                        index,
                        request,
                        outcome,
                        started: true,
                    });
                }
            });
        }

        Ok(slots
            .into_iter()
            .map(|slot| slot.expect("every dispatch_many slot is filled"))
            .collect())
    }

    /// Cancel the shared cancellation token; every subsequent child fails closed as `Cancelled`.
    pub fn cancel(&self) {
        self.context.cancellation.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.context.cancellation.is_cancelled()
    }

    /// Remaining time until the shared deadline, or `None` when there is no duration limit.
    pub fn remaining_deadline_ms(&self) -> Option<u64> {
        self.context
            .ledger
            .remaining_deadline_ms(self.context.clock.now_ms())
    }

    pub fn parent_call_id(&self) -> &str {
        &self.context.parent_call_id
    }

    pub fn parent_descriptor_id(&self) -> &str {
        &self.context.parent_descriptor_id
    }

    /// Depth at which children dispatched through this context will execute.
    pub fn depth(&self) -> u32 {
        self.context.depth + 1
    }

    pub fn lineage(&self) -> Vec<String> {
        self.context.lineage.clone()
    }

    /// Read-only registry truth source, so a composite can derive its children's declared
    /// permission scopes for conservative aggregation (phase-3 review P2-3). Read access only —
    /// it cannot be used to dispatch un-authorized children.
    pub fn registry(&self) -> &ToolRegistrySnapshot {
        self.runner.registry()
    }

    pub fn allowed_child_ids(&self) -> BTreeSet<String> {
        self.context.allowed_child_ids.clone()
    }

    fn emit_child_record(
        &self,
        request: &ChildDispatchRequest,
        outcome: &ToolOutcome,
        ready: Option<&PreflightedChild>,
        started: Instant,
    ) {
        let record = build_child_record(request, outcome, ready, &self.context, started);
        self.runner.emit(&record);
    }
}

/// Shared decision + execution dispatch used by both the single and batch child paths.
fn invoke_child_shared(
    runner: &Arc<dyn ChildDispatchRunner>,
    context: &ChildDispatchContext,
    request: &ChildDispatchRequest,
    ready: PreflightedChild,
) -> ToolOutcome {
    match ready.decision.verdict {
        PermissionVerdict::Deny => deny_outcome(&request.descriptor_id, &ready),
        PermissionVerdict::ApprovalRequired | PermissionVerdict::WaitingHost => {
            // A pending control child suspends every un-started sibling in this context.
            context.suspended.store(true, Ordering::Release);
            runner.persist_pending(context, request, ready)
        }
        PermissionVerdict::Allow => {
            if runner
                .composite_handler(&ready.descriptor.identity.descriptor_id)
                .is_some()
            {
                execute_composite_shared(runner, context, request, ready)
            } else {
                runner.execute_primitive(context, request, ready)
            }
        }
    }
}

/// Run a composite child handler with a freshly bounded child context.
fn execute_composite_shared(
    runner: &Arc<dyn ChildDispatchRunner>,
    context: &ChildDispatchContext,
    request: &ChildDispatchRequest,
    ready: PreflightedChild,
) -> ToolOutcome {
    if let Err(error) = check_budget_gates(
        &context.cancellation,
        &context.ledger,
        context.clock.as_ref(),
    ) {
        return error.into_outcome(&request.descriptor_id);
    }
    if let Err(message) = context.ledger.reserve_call() {
        return DispatchError::new("budget_exhausted", message).into_outcome(&request.descriptor_id);
    }
    let started = Instant::now();
    let _concurrency = match context.ledger.begin_execution() {
        Ok(guard) => guard,
        Err(message) => {
            return DispatchError::new("budget_exhausted", message).into_outcome(&request.descriptor_id);
        }
    };
    let Some(handler) = runner.composite_handler(&ready.descriptor.identity.descriptor_id) else {
        return DispatchError::new(
            "no_handler_registered",
            format!(
                "no composite handler registered for `{}`",
                ready.descriptor.identity.descriptor_id
            ),
        )
        .into_outcome(&request.descriptor_id);
    };

    let mut lineage = context.lineage.clone();
    lineage.push(ready.descriptor.identity.descriptor_id.clone());
    let child_context = ChildDispatchContext {
        parent_call_id: request.call_id.clone(),
        parent_descriptor_id: ready.descriptor.identity.descriptor_id.clone(),
        lineage,
        depth: context.depth + 1,
        allowed_child_ids: runner.allowed_children_for(&ready.descriptor),
        ledger: Arc::clone(&context.ledger),
        cancellation: Arc::clone(&context.cancellation),
        context: context.context.clone(),
        clock: Arc::clone(&context.clock),
        suspended: Arc::new(AtomicBool::new(false)),
    };
    let child_dispatch = ChildDispatch::new(Arc::clone(runner), child_context);
    let composite_request = CompositeToolHandlerRequest {
        descriptor_id: ready.descriptor.identity.descriptor_id.clone(),
        call_id: request.call_id.clone(),
        arguments: ready.final_arguments.clone(),
    };

    match handler.execute(&composite_request, &child_dispatch) {
        Ok(value) => {
            let output = render_output(&value);
            if let Err(message) = context.ledger.reserve_bytes(output.len() as u64) {
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
            handler_failure(message).into_outcome(&request.descriptor_id)
        }
    }
}

/// Check the cancellation token and shared deadline before a child runs.
pub(crate) fn check_budget_gates(
    cancellation: &CancellationToken,
    ledger: &BudgetLedger,
    clock: &dyn RuntimeClock,
) -> Result<(), DispatchError> {
    cancellation
        .check()
        .map_err(|_| DispatchError::cancelled("tool invocation cancelled"))?;
    ledger
        .check_deadline(clock.now_ms())
        .map_err(|message| DispatchError::new("timeout", message))?;
    Ok(())
}

pub(crate) fn build_child_record(
    request: &ChildDispatchRequest,
    outcome: &ToolOutcome,
    ready: Option<&PreflightedChild>,
    context: &ChildDispatchContext,
    started: Instant,
) -> DispatchLifecycleRecord {
    build_lifecycle_record(
        ready.map(|item| item.descriptor.identity.descriptor_id.clone())
            .unwrap_or_else(|| request.descriptor_id.clone()),
        request.call_id.clone(),
        Some(context.parent_call_id.clone()),
        InvocationOrigin::Child,
        context.depth + 1,
        outcome,
        ready.map(|item| item.decision.clone()).as_ref(),
        ready.map(|item| item.final_args_digest.clone()).unwrap_or_default(),
        started.elapsed().as_millis() as u64,
    )
}

fn not_started_outcome(request: &ChildDispatchRequest) -> ToolOutcome {
    ToolOutcome::from_legacy_result(ToolResult {
        tool_name: request.descriptor_id.clone(),
        status: "aborted".to_string(),
        output: json!({
            "ok": false,
            "error": {
                "code": "not_started",
                "message": "child not started because an earlier sibling entered a pending control state",
            }
        })
        .to_string(),
        duration_ms: 0,
    })
}

fn child_panic_outcome() -> ToolOutcome {
    ToolOutcome::from_legacy_result(ToolResult {
        tool_name: "unknown".to_string(),
        status: "error".to_string(),
        output: json!({
            "ok": false,
            "error": {
                "code": "child_panic",
                "message": "child handler panicked during concurrent dispatch",
            }
        })
        .to_string(),
        duration_ms: 0,
    })
}

/// The restricted execution port implemented by the governed dispatcher. Composite handlers can
/// never reach this trait directly; they only hold a `ChildDispatch`.
pub(crate) trait ChildDispatchRunner: Send + Sync {
    fn preflight(
        &self,
        ctx: &ChildDispatchContext,
        request: &ChildDispatchRequest,
    ) -> Result<PreflightedChild, DispatchError>;

    fn execute_primitive(
        &self,
        ctx: &ChildDispatchContext,
        request: &ChildDispatchRequest,
        ready: PreflightedChild,
    ) -> ToolOutcome;

    fn persist_pending(
        &self,
        ctx: &ChildDispatchContext,
        request: &ChildDispatchRequest,
        ready: PreflightedChild,
    ) -> ToolOutcome;

    fn composite_handler(&self, descriptor_id: &str) -> Option<Arc<dyn CompositeToolHandler>>;

    fn allowed_children_for(&self, descriptor: &ToolDescriptor) -> BTreeSet<String>;

    fn registry(&self) -> &ToolRegistrySnapshot;

    fn emit(&self, record: &DispatchLifecycleRecord);
}
