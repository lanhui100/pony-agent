use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

/// Pipeline contract version.
pub const PIPELINE_CONTRACT_VERSION: &str = "lifecycle-pipeline-v1";

/// Canonical execution lifecycle phase for pipeline interceptors.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PipelinePhase {
    /// Before turn execution starts.
    TurnStart,
    /// Before prompt context compilation.
    BeforeContextBuild,
    /// After prompt context compilation.
    AfterContextBuild,
    /// Before calling the provider model.
    BeforeModelCall,
    /// After model response is received.
    AfterModelCall,
    /// Before executing a tool call.
    BeforeToolExec,
    /// After executing a tool call.
    AfterToolExec,
    /// Before finalizing and saving turn outputs.
    TurnFinalize,
    /// When an unhandled error or panic occurs.
    OnError,
}

/// Pipeline intercept decision returned by middleware.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum InterceptDecision {
    /// Allow the lifecycle pipeline to proceed to the next middleware or core execution.
    Pass,
    /// Block the lifecycle pipeline immediately with a deterministic reason.
    Block {
        reason: String,
        code: Option<String>,
    },
    /// Mutate the contextual payload before proceeding.
    Mutate {
        patch: Value,
    },
    /// Halt execution pending asynchronous human or external approval.
    ApprovalRequired {
        request_id: String,
        prompt: String,
        context: Option<Value>,
        timeout: Option<Duration>,
    },
}

impl InterceptDecision {
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, Self::Block { .. })
    }

    pub fn is_mutate(&self) -> bool {
        matches!(self, Self::Mutate { .. })
    }

    pub fn is_approval_required(&self) -> bool {
        matches!(self, Self::ApprovalRequired { .. })
    }
}

/// Read-only snapshot of interception telemetry.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InterceptTrace {
    pub middleware_name: String,
    pub phase: PipelinePhase,
    pub duration_ms: u64,
    pub decision: String,
    pub details: Option<String>,
}

/// Context passed into pipeline middleware during interception.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PipelineContext {
    pub session_id: String,
    pub turn_id: String,
    pub phase: PipelinePhase,
    pub payload: Value,
    pub metadata: Value,
    pub traces: Vec<InterceptTrace>,
}

impl PipelineContext {
    pub fn new(
        session_id: impl Into<String>,
        turn_id: impl Into<String>,
        phase: PipelinePhase,
        payload: Value,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            turn_id: turn_id.into(),
            phase,
            payload,
            metadata: Value::Object(Default::default()),
            traces: Vec::new(),
        }
    }
}

/// Continuation handle representing the remainder of the onion middleware stack.
pub type NextFn<'a> = Box<
    dyn FnOnce(&'a mut PipelineContext) -> Result<InterceptDecision, PipelineError> + 'a,
>;

/// Error type emitted by pipeline middleware or pipeline execution engine.
#[derive(Debug)]
pub enum PipelineError {
    Timeout(Duration),
    RecursionLimitExceeded(String),
    MiddlewareExecutionFailed { name: String, message: String },
    InvalidMutation(String),
    Internal(String),
}

impl std::fmt::Display for PipelineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Timeout(d) => write!(f, "Pipeline middleware execution timed out after {:?}", d),
            Self::RecursionLimitExceeded(s) => write!(f, "Pipeline cycle or recursion limit exceeded: {}", s),
            Self::MiddlewareExecutionFailed { name, message } => {
                write!(f, "Middleware `{}` failed: {}", name, message)
            }
            Self::InvalidMutation(s) => write!(f, "Mutation payload invalid: {}", s),
            Self::Internal(s) => write!(f, "Internal pipeline error: {}", s),
        }
    }
}

impl std::error::Error for PipelineError {}

/// Lifecycle Onion Middleware Trait contract.
///
/// Implementations must be pure contracts in this crate without hardcoded business logic.
pub trait PipelineMiddleware: Send + Sync {
    /// Human-readable identifier for this middleware.
    fn name(&self) -> &str;

    /// The phases in which this middleware participates.
    fn supported_phases(&self) -> &[PipelinePhase];

    /// Intercept and wrap execution around `next`.
    fn handle<'a>(
        &self,
        cx: &'a mut PipelineContext,
        next: NextFn<'a>,
    ) -> Result<InterceptDecision, PipelineError>;
}

/// Pure contract stub for a pipeline orchestrator / chain.
pub trait Pipeline: Send + Sync {
    /// Register a middleware component onto the pipeline stack.
    fn use_middleware(&mut self, middleware: Arc<dyn PipelineMiddleware>);

    /// Execute the pipeline stack for a given phase and context.
    fn execute(&self, cx: &mut PipelineContext) -> Result<InterceptDecision, PipelineError>;
}

/// A pure no-op stub implementation of `PipelineMiddleware`.
pub struct NoopPipelineMiddleware {
    name: String,
}

impl NoopPipelineMiddleware {
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl PipelineMiddleware for NoopPipelineMiddleware {
    fn name(&self) -> &str {
        &self.name
    }

    fn supported_phases(&self) -> &[PipelinePhase] {
        static ALL_PHASES: &[PipelinePhase] = &[
            PipelinePhase::TurnStart,
            PipelinePhase::BeforeContextBuild,
            PipelinePhase::AfterContextBuild,
            PipelinePhase::BeforeModelCall,
            PipelinePhase::AfterModelCall,
            PipelinePhase::BeforeToolExec,
            PipelinePhase::AfterToolExec,
            PipelinePhase::TurnFinalize,
            PipelinePhase::OnError,
        ];
        ALL_PHASES
    }

    fn handle<'a>(
        &self,
        cx: &'a mut PipelineContext,
        next: NextFn<'a>,
    ) -> Result<InterceptDecision, PipelineError> {
        next(cx)
    }
}

/// A robust Onion-model execution engine for `Pipeline`.
#[derive(Clone, Default)]
pub struct PipelineStub {
    middlewares: Vec<Arc<dyn PipelineMiddleware>>,
    max_depth: usize,
    default_timeout: Option<Duration>,
}

impl PipelineStub {
    pub fn new() -> Self {
        Self {
            middlewares: Vec::new(),
            max_depth: 64,
            default_timeout: Some(Duration::from_millis(25)),
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.default_timeout = Some(timeout);
        self
    }

    pub fn with_max_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
        self
    }
}

impl Pipeline for PipelineStub {
    fn use_middleware(&mut self, middleware: Arc<dyn PipelineMiddleware>) {
        self.middlewares.push(middleware);
    }

    fn execute(&self, cx: &mut PipelineContext) -> Result<InterceptDecision, PipelineError> {
        let matching: Vec<Arc<dyn PipelineMiddleware>> = self
            .middlewares
            .iter()
            .filter(|m| m.supported_phases().contains(&cx.phase))
            .cloned()
            .collect();

        if matching.is_empty() {
            return Ok(InterceptDecision::Pass);
        }

        // Apply timeout if configured
        let timeout_opt = self.default_timeout;
        let start_time = std::time::Instant::now();

        fn run_chain<'a>(
            index: usize,
            middlewares: &'a [Arc<dyn PipelineMiddleware>],
            cx: &'a mut PipelineContext,
            depth: usize,
            max_depth: usize,
            timeout: Option<Duration>,
            start_time: std::time::Instant,
        ) -> Result<InterceptDecision, PipelineError> {
            if depth > max_depth {
                return Err(PipelineError::RecursionLimitExceeded(format!(
                    "Pipeline stack depth {} exceeded maximum allowed {}",
                    depth, max_depth
                )));
            }

            if let Some(limit) = timeout {
                if start_time.elapsed() >= limit {
                    return Err(PipelineError::Timeout(limit));
                }
            }

            if index >= middlewares.len() {
                return Ok(InterceptDecision::Pass);
            }

            let current = &middlewares[index];

            let next: NextFn<'a> = Box::new(move |next_cx| {
                run_chain(
                    index + 1,
                    middlewares,
                    next_cx,
                    depth + 1,
                    max_depth,
                    timeout,
                    start_time,
                )
            });

            let res = current.handle(cx, next);

            if let Some(limit) = timeout {
                if start_time.elapsed() >= limit && res.is_ok() {
                    return Err(PipelineError::Timeout(limit));
                }
            }

            res
        }

        run_chain(
            0,
            &matching,
            cx,
            0,
            self.max_depth,
            timeout_opt,
            start_time,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipeline_noop_contract() {
        let mut pipeline = PipelineStub::new();
        pipeline.use_middleware(Arc::new(NoopPipelineMiddleware::new("noop")));

        let mut cx = PipelineContext::new("s1", "t1", PipelinePhase::TurnStart, Value::Null);
        let decision = pipeline.execute(&mut cx).expect("execute ok");
        assert_eq!(decision, InterceptDecision::Pass);
        assert!(decision.is_pass());
    }

    #[test]
    fn test_decision_variants() {
        let block = InterceptDecision::Block {
            reason: "blocked by policy".to_string(),
            code: Some("E_BLOCKED".to_string()),
        };
        assert!(block.is_blocked());

        let mutate = InterceptDecision::Mutate {
            patch: serde_json::json!({"arg": "rewritten"}),
        };
        assert!(mutate.is_mutate());

        let approval = InterceptDecision::ApprovalRequired {
            request_id: "req-1".to_string(),
            prompt: "Allow tool run?".to_string(),
            context: None,
            timeout: Some(Duration::from_secs(30)),
        };
        assert!(approval.is_approval_required());
    }
}
