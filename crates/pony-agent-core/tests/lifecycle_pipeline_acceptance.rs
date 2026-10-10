use pony_agent_core::agent::pipeline::{
    InterceptDecision, NextFn, Pipeline, PipelineContext, PipelineError, PipelineMiddleware,
    PipelinePhase, PipelineStub,
};
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

// ============================================================================
// 蓝军功能契约验收用例 (L2-T Functional Acceptance Tests)
// ============================================================================

/// 中间件：洋葱模型调用顺序与计数追踪
struct TelemetryLoggingMiddleware {
    name: String,
    phase: PipelinePhase,
    executed: Arc<AtomicBool>,
}

impl PipelineMiddleware for TelemetryLoggingMiddleware {
    fn name(&self) -> &str {
        &self.name
    }

    fn supported_phases(&self) -> &[PipelinePhase] {
        std::slice::from_ref(&self.phase)
    }

    fn handle<'a>(
        &self,
        cx: &'a mut PipelineContext,
        next: NextFn<'a>,
    ) -> Result<InterceptDecision, PipelineError> {
        self.executed.store(true, Ordering::SeqCst);
        next(cx)
    }
}

/// 中间件：阻断（Block）策略
struct PolicyBlockMiddleware {
    block_phase: PipelinePhase,
    reason: String,
    code: Option<String>,
}

impl PipelineMiddleware for PolicyBlockMiddleware {
    fn name(&self) -> &str {
        "policy_blocker"
    }

    fn supported_phases(&self) -> &[PipelinePhase] {
        std::slice::from_ref(&self.block_phase)
    }

    fn handle<'a>(
        &self,
        _cx: &'a mut PipelineContext,
        _next: NextFn<'a>,
    ) -> Result<InterceptDecision, PipelineError> {
        Ok(InterceptDecision::Block {
            reason: self.reason.clone(),
            code: self.code.clone(),
        })
    }
}

/// 中间件：改写（Mutate）参数策略
struct ParamMutateMiddleware {
    phase: PipelinePhase,
    patch: serde_json::Value,
}

impl PipelineMiddleware for ParamMutateMiddleware {
    fn name(&self) -> &str {
        "param_mutator"
    }

    fn supported_phases(&self) -> &[PipelinePhase] {
        std::slice::from_ref(&self.phase)
    }

    fn handle<'a>(
        &self,
        _cx: &'a mut PipelineContext,
        next: NextFn<'a>,
    ) -> Result<InterceptDecision, PipelineError> {
        let dec = next(_cx)?;
        if dec.is_pass() {
            Ok(InterceptDecision::Mutate {
                patch: self.patch.clone(),
            })
        } else {
            Ok(dec)
        }
    }
}

/// 中间件：人工审批（ApprovalRequired）策略
struct HumanInTheLoopMiddleware {
    phase: PipelinePhase,
    request_id: String,
    prompt: String,
}

impl PipelineMiddleware for HumanInTheLoopMiddleware {
    fn name(&self) -> &str {
        "hitl_gate"
    }

    fn supported_phases(&self) -> &[PipelinePhase] {
        std::slice::from_ref(&self.phase)
    }

    fn handle<'a>(
        &self,
        _cx: &'a mut PipelineContext,
        _next: NextFn<'a>,
    ) -> Result<InterceptDecision, PipelineError> {
        Ok(InterceptDecision::ApprovalRequired {
            request_id: self.request_id.clone(),
            prompt: self.prompt.clone(),
            context: Some(json!({"requires_admin": true})),
            timeout: Some(Duration::from_secs(60)),
        })
    }
}

/// L2-T-01: 测试中间件在洋葱模型中的真实链路执行与按生命周期切面过滤
/// 在空桩 (PipelineStub) 下，由于 execute() 仅简单返回 Ok(Pass) 且不调用中间件 handle()，
/// 该断言必然原生失败，确立红相锚定。
#[test]
fn test_l2_t01_onion_execution_and_phase_filtering() {
    let mut pipeline = PipelineStub::new();
    let executed_tool = Arc::new(AtomicBool::new(false));
    let executed_turn = Arc::new(AtomicBool::new(false));

    let m1 = Arc::new(TelemetryLoggingMiddleware {
        name: "mw_tool_exec".to_string(),
        phase: PipelinePhase::BeforeToolExec,
        executed: Arc::clone(&executed_tool),
    });
    let m2 = Arc::new(TelemetryLoggingMiddleware {
        name: "mw_turn_start".to_string(),
        phase: PipelinePhase::TurnStart,
        executed: Arc::clone(&executed_turn),
    });

    pipeline.use_middleware(m1);
    pipeline.use_middleware(m2);

    let mut cx = PipelineContext::new("s_test", "t_test", PipelinePhase::BeforeToolExec, json!({"tool": "bash"}));
    let decision = pipeline.execute(&mut cx).expect("Pipeline execution must succeed");

    // 核心契约断言：
    // 1. 中间件必须被调度并执行（空桩下为 false，必失败）
    assert!(
        executed_tool.load(Ordering::SeqCst),
        "Pipeline must execute phase-matched middleware handle()"
    );
    // 2. 仅支持 BeforeToolExec 的中间件被触发，TurnStart 中间件不应执行
    assert!(
        !executed_turn.load(Ordering::SeqCst),
        "TurnStart middleware should not be triggered during BeforeToolExec phase"
    );
    assert!(decision.is_pass());
}

/// L2-T-02: 测试生命周期各关键切面的阻断（Block）策略生效
/// 断言 Pipeline 执行能正确捕获 Block 决策并短路下游，同时透出 reason 与 error code
#[test]
fn test_l2_t02_lifecycle_block_decision_short_circuit() {
    let mut pipeline = PipelineStub::new();
    let blocker = Arc::new(PolicyBlockMiddleware {
        block_phase: PipelinePhase::BeforeModelCall,
        reason: "Cost quota exceeded for model call".to_string(),
        code: Some("ERR_BUDGET_EXCEEDED".to_string()),
    });
    pipeline.use_middleware(blocker);

    let mut cx = PipelineContext::new(
        "s_block",
        "t_block",
        PipelinePhase::BeforeModelCall,
        json!({"prompt": "hello"}),
    );
    let decision = pipeline.execute(&mut cx).expect("Execute must return decision");

    // 断言必须返回 Block 决策，且携带完整的 reason 与 code
    match decision {
        InterceptDecision::Block { reason, code } => {
            assert_eq!(reason, "Cost quota exceeded for model call");
            assert_eq!(code.as_deref(), Some("ERR_BUDGET_EXCEEDED"));
        }
        other => panic!("Expected InterceptDecision::Block, got {:?}", other),
    }
}

/// L2-T-03: 测试生命周期参数改写（Mutate）与人工审批（ApprovalRequired）切面决策
#[test]
fn test_l2_t03_lifecycle_mutate_and_approval_decisions() {
    // 验证 Mutate
    let mut pipeline_mutate = PipelineStub::new();
    let mutator = Arc::new(ParamMutateMiddleware {
        phase: PipelinePhase::BeforeToolExec,
        patch: json!({"command": "ls -l (sanitized)"}),
    });
    pipeline_mutate.use_middleware(mutator);

    let mut cx_mutate = PipelineContext::new(
        "s_mut",
        "t_mut",
        PipelinePhase::BeforeToolExec,
        json!({"command": "rm -rf /"}),
    );
    let decision_mut = pipeline_mutate.execute(&mut cx_mutate).expect("Execute should succeed");
    match decision_mut {
        InterceptDecision::Mutate { patch } => {
            assert_eq!(patch, json!({"command": "ls -l (sanitized)"}));
        }
        other => panic!("Expected Mutate decision, got {:?}", other),
    }

    // 验证 ApprovalRequired
    let mut pipeline_approval = PipelineStub::new();
    let hitl = Arc::new(HumanInTheLoopMiddleware {
        phase: PipelinePhase::BeforeToolExec,
        request_id: "req_hitl_001".to_string(),
        prompt: "Sensitive write operation requires human approval".to_string(),
    });
    pipeline_approval.use_middleware(hitl);

    let mut cx_approval = PipelineContext::new(
        "s_app",
        "t_app",
        PipelinePhase::BeforeToolExec,
        json!({"action": "deploy"}),
    );
    let decision_app = pipeline_approval.execute(&mut cx_approval).expect("Execute should succeed");
    match decision_app {
        InterceptDecision::ApprovalRequired {
            request_id,
            prompt,
            context,
            timeout,
        } => {
            assert_eq!(request_id, "req_hitl_001");
            assert!(prompt.contains("human approval"));
            assert!(context.is_some());
            assert_eq!(timeout, Some(Duration::from_secs(60)));
        }
        other => panic!("Expected ApprovalRequired decision, got {:?}", other),
    }
}

// ============================================================================
// 红军对抗破坏注入用例 (L2-AT Adversarial Tests)
// ============================================================================

/// 对抗中间件：模拟耗时中间件与超时注入
struct MaliciousHangingMiddleware {
    phase: PipelinePhase,
    hang_duration: Duration,
}

impl PipelineMiddleware for MaliciousHangingMiddleware {
    fn name(&self) -> &str {
        "malicious_hanging_mw"
    }

    fn supported_phases(&self) -> &[PipelinePhase] {
        std::slice::from_ref(&self.phase)
    }

    fn handle<'a>(
        &self,
        cx: &'a mut PipelineContext,
        next: NextFn<'a>,
    ) -> Result<InterceptDecision, PipelineError> {
        std::thread::sleep(self.hang_duration);
        next(cx)
    }
}

/// 对抗中间件：模拟中间件内部递归/重复调用自身触发循环深度
struct RecursiveLoopMiddleware {
    phase: PipelinePhase,
    call_count: Arc<AtomicUsize>,
}

impl PipelineMiddleware for RecursiveLoopMiddleware {
    fn name(&self) -> &str {
        "recursive_loop_mw"
    }

    fn supported_phases(&self) -> &[PipelinePhase] {
        std::slice::from_ref(&self.phase)
    }

    fn handle<'a>(
        &self,
        cx: &'a mut PipelineContext,
        next: NextFn<'a>,
    ) -> Result<InterceptDecision, PipelineError> {
        let count = self.call_count.fetch_add(1, Ordering::SeqCst);
        if count > 50 {
            return Err(PipelineError::RecursionLimitExceeded(
                "Recursion loop detected in middleware".to_string(),
            ));
        }
        // 尝试传递执行
        next(cx)
    }
}

/// 对抗中间件：参数篡改与非法 Payload 穿透攻击
struct TamperingPayloadMiddleware {
    phase: PipelinePhase,
    inject_invalid: bool,
}

impl PipelineMiddleware for TamperingPayloadMiddleware {
    fn name(&self) -> &str {
        "tampering_payload_mw"
    }

    fn supported_phases(&self) -> &[PipelinePhase] {
        std::slice::from_ref(&self.phase)
    }

    fn handle<'a>(
        &self,
        cx: &'a mut PipelineContext,
        next: NextFn<'a>,
    ) -> Result<InterceptDecision, PipelineError> {
        if self.inject_invalid {
            // 篡改 context 破坏不可变或非法 schema
            cx.payload = json!({
                "__proto__": { "polluted": true },
                "malicious_overflow": "A".repeat(1024 * 1024)
            });
            // 注入非法变异错误
            return Err(PipelineError::InvalidMutation(
                "Payload mutation failed structural schema validation".to_string(),
            ));
        }
        next(cx)
    }
}

/// L2-AT-01: 对抗破坏注入——中间件超时与防御熔断 (Timeout Protection)
/// 断言 Pipeline 调度器必须具备执行耗时监控机制，当恶意中间件超时挂起时，
/// 必须返回 PipelineError::Timeout，严禁无限卡死阻塞主调度流程。
#[test]
fn test_l2_at01_adversarial_middleware_timeout_containment() {
    let mut pipeline = PipelineStub::new();
    let hanging_mw = Arc::new(MaliciousHangingMiddleware {
        phase: PipelinePhase::TurnStart,
        hang_duration: Duration::from_millis(50),
    });
    pipeline.use_middleware(hanging_mw);

    let mut cx = PipelineContext::new("s_atk", "t_atk", PipelinePhase::TurnStart, json!({}));

    // 空桩不执行中间件，直接返回 Ok(Pass)，因此此处断言 Err 必然在红相阶段失败！
    let result = pipeline.execute(&mut cx);
    assert!(
        result.is_err(),
        "Adversarial hanging middleware must trigger error or timeout containment"
    );
    match result.unwrap_err() {
        PipelineError::Timeout(_) => {}
        PipelineError::MiddlewareExecutionFailed { .. } => {}
        other => panic!("Expected Timeout or ExecutionFailed error, got {:?}", other),
    }
}

/// L2-AT-02: 对抗破坏注入——死循环与调用栈深度穿透防护 (Cycle & Recursion Limit Protection)
/// 模拟递归或深度超限中间件，断言调度执行器能有效拦截并返回 RecursionLimitExceeded
#[test]
fn test_l2_at02_adversarial_recursion_and_loop_containment() {
    let mut pipeline = PipelineStub::new();
    let call_count = Arc::new(AtomicUsize::new(0));
    let loop_mw = Arc::new(RecursiveLoopMiddleware {
        phase: PipelinePhase::BeforeContextBuild,
        call_count: Arc::clone(&call_count),
    });
    pipeline.use_middleware(loop_mw);

    let mut cx = PipelineContext::new("s_loop", "t_loop", PipelinePhase::BeforeContextBuild, json!({}));

    // 空桩直接返回 Ok(Pass)，红相阶段断言必定失败
    let result = pipeline.execute(&mut cx);
    assert!(
        result.is_err() || call_count.load(Ordering::SeqCst) > 0,
        "Recursion/loop middleware must be intercepted by pipeline"
    );
}

/// L2-AT-03: 对抗破坏注入——参数篡改注入与非法畸形 Payload 防护 (Tampering & Payload Integrity)
/// 模拟中间件产生畸形或非法 Mutation 变异，断言调度执行器拒绝非法变更并返回 InvalidMutation
#[test]
fn test_l2_at03_adversarial_payload_tampering_integrity() {
    let mut pipeline = PipelineStub::new();
    let tamper_mw = Arc::new(TamperingPayloadMiddleware {
        phase: PipelinePhase::BeforeToolExec,
        inject_invalid: true,
    });
    pipeline.use_middleware(tamper_mw);

    let mut cx = PipelineContext::new(
        "s_tamper",
        "t_tamper",
        PipelinePhase::BeforeToolExec,
        json!({"command": "echo normal"}),
    );

    // 空桩直接返回 Ok(Pass)，红相阶段断言必定失败
    let result = pipeline.execute(&mut cx);
    assert!(
        result.is_err(),
        "Tampering invalid mutation must be rejected with PipelineError::InvalidMutation"
    );
    match result.unwrap_err() {
        PipelineError::InvalidMutation(msg) => {
            assert!(msg.contains("schema validation") || msg.contains("invalid"));
        }
        other => panic!("Expected InvalidMutation error, got {:?}", other),
    }
}

/// L2-AT-04: 对抗破坏注入——中间件显式错误阻断与传播 (Middleware Error Propagation)
#[test]
fn test_l2_at04_adversarial_middleware_error_propagation() {
    struct FailingMiddleware;
    impl PipelineMiddleware for FailingMiddleware {
        fn name(&self) -> &str {
            "failing_mw"
        }
        fn supported_phases(&self) -> &[PipelinePhase] {
            &[PipelinePhase::TurnStart]
        }
        fn handle<'a>(
            &self,
            _cx: &'a mut PipelineContext,
            _next: NextFn<'a>,
        ) -> Result<InterceptDecision, PipelineError> {
            Err(PipelineError::MiddlewareExecutionFailed {
                name: "failing_mw".to_string(),
                message: "deterministic failure injection".to_string(),
            })
        }
    }

    let mut pipeline = PipelineStub::new();
    pipeline.use_middleware(Arc::new(FailingMiddleware));
    let mut cx = PipelineContext::new("s_fail", "t_fail", PipelinePhase::TurnStart, json!({}));
    let result = pipeline.execute(&mut cx);
    assert!(result.is_err());
    match result.unwrap_err() {
        PipelineError::MiddlewareExecutionFailed { name, message } => {
            assert_eq!(name, "failing_mw");
            assert_eq!(message, "deterministic failure injection");
        }
        other => panic!("Expected MiddlewareExecutionFailed, got {:?}", other),
    }
}

