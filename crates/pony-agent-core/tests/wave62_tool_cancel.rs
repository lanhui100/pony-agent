//! wave-62 工具级取消（wave-60 A9 清偿）黑盒验收 —— 红相先写。
//!
//! 契约真值源：`.dev-team/contracts/wave-62-tool-cancel.md` §1（C1–C4 唯一判据来源）。
//!
//! 覆盖：
//! - C1：每轮一个取消令牌（register_turn 创建 / 被接受的 stop 才 cancel / 未知 turn ⇒ None）；
//! - C2：dispatcher 观测 turn 令牌（注入"阻塞到取消"的测试工具；`None` 时行为不变）；
//! - C3：`workspace_run_command` 在取消后**真正终止子进程**（≤1000ms 返回 aborted + `tool_cancelled`，
//!   且被取消的命令不得继续执行留下副作用）；
//! - C4：回归边界（不改 wave-60 六个验收测试；本文件只新增，不触碰既有断言）。
//!
//! 纪律（test-expert L2）：真实断言、AAA、命名带判据号；禁止 `#[ignore]` / catch 吞异常 / 空断言；
//! 所有可能阻塞的用例都**自带超时上界**（`recv_timeout`），**永不挂死**。

use pony_agent_core::agent::budget::CancellationToken;
use pony_agent_core::agent::dispatcher::{DispatchContext, GovernedDispatcher};
use pony_agent_core::agent::execution_control::ExecutionControlRegistry;
use pony_agent_core::agent::sandbox::TestSandboxBackend;
use pony_agent_core::agent::stop_escalation::StopReason;
use pony_agent_core::agent::tool_runtime::{
    FakeClock, InvocationOrigin, PrimitiveToolHandler, PrimitiveToolHandlerRequest,
    ToolDispatchRequest,
};
use pony_agent_core::agent::tools::{
    ToolCall, ToolDescriptor, ToolDescriptorSource, ToolDisplayMetadata, ToolExecutionPolicy,
    ToolExecutionStatus, ToolExecutionContext, ToolExposure, ToolHandlerProvenance, ToolIdentity,
    ToolKind, ToolOutcome, ToolPermissionDeclaration, ToolRegistrySnapshot, ToolRouter,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// C3 契约硬上界：cancel → run_command 返回 aborted。
const TURN_CANCEL_EXIT_BOUND_MS: u64 = 1_000;
/// 契约 C3：子进程轮询间隔上界（NFR `turn_cancel_poll_ms`）。
const TURN_CANCEL_POLL_MS: u64 = 100;

fn temp_workspace(tag: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "pony-wave62-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&path).expect("create temp workspace");
    path
}

fn dispatch_context(turn_cancellation: Option<Arc<CancellationToken>>) -> DispatchContext {
    DispatchContext {
        session_id: Some("session-wave62".to_string()),
        run_id: None,
        turn_id: Some("turn-wave62".to_string()),
        workspace_root: None,
        host_control_available: true,
        plan_execution_approval: None,
        tool_authorization_mode: None,
        turn_cancellation,
    }
}

fn request(origin: InvocationOrigin, descriptor_id: &str, call_id: &str) -> ToolDispatchRequest {
    ToolDispatchRequest {
        origin,
        descriptor_id: descriptor_id.to_string(),
        call_id: call_id.to_string(),
        arguments: json!({ "description": "wave62 test" }),
    }
}

fn test_descriptor(descriptor_id: &str, model_name: &str) -> ToolDescriptor {
    ToolDescriptor {
        identity: ToolIdentity {
            descriptor_id: descriptor_id.to_string(),
            model_name: model_name.to_string(),
            canonical_name: model_name.to_string(),
            primitive_name: model_name.to_lowercase().replace(' ', "_"),
            source: ToolDescriptorSource::Dynamic,
            },
        aliases: vec![descriptor_id.to_string()],
        description: format!("wave62 test descriptor {descriptor_id}"),
        input_schema: json!({ "type": "object", "properties": {} }),
        kind: ToolKind::Read,
        exposure: ToolExposure::ModelVisible,
        permission_declaration: ToolPermissionDeclaration::default(),
        execution_policy: ToolExecutionPolicy::default(),
        display_metadata: ToolDisplayMetadata::default(),
        handler_provenance: ToolHandlerProvenance {
            handler_kind: "test".to_string(),
            source_id: "wave62-test-source".to_string(),
        },
        source_revision: "wave62-test-v1".to_string(),
        composed_descriptor_ids: Vec::new(),
    }
}

fn dispatcher_with(descriptor_id: &str) -> Arc<GovernedDispatcher> {
    let registry = Arc::new(
        ToolRegistrySnapshot::from_descriptors(
            "wave62-test-v1",
            vec![test_descriptor(descriptor_id, "Wave62Blocking")],
        )
        .expect("wave62 test registry should build"),
    );
    Arc::new(GovernedDispatcher::new(registry, Arc::new(FakeClock::new(0))))
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

/// C2 测试工具：阻塞到 turn 令牌被 cancel（有 3s 自身上界，绝不永久阻塞）；
/// 一旦开始执行就通过 `started` 通道通知测试线程（确定性握手，不用盲等）。
struct BlockingUntilCancelledHandler {
    token: Arc<CancellationToken>,
    started: mpsc::Sender<()>,
}

impl PrimitiveToolHandler for BlockingUntilCancelledHandler {
    fn execute(&self, _request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
        let _ = self.started.send(());
        let deadline = Instant::now() + Duration::from_secs(3);
        while !self.token.is_cancelled() {
            if Instant::now() >= deadline {
                return Err("wave62_test_handler_never_cancelled".to_string());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Ok(json!({ "ok": true, "observed_cancel": true }))
    }
}

/// C2 回归工具：立即返回值（用于 `turn_cancellation: None` 的向后兼容判据）。
struct FixedValueHandler(Value);

impl PrimitiveToolHandler for FixedValueHandler {
    fn execute(&self, _request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
        Ok(self.0.clone())
    }
}

// ---------------------------------------------------------------------------
// C1 每轮一个取消令牌
// ---------------------------------------------------------------------------

/// C1：`register_turn` 后该 turn 有令牌且未处于已取消态。
#[test]
fn c1_register_turn_exposes_a_live_cancellation_token() {
    // Arrange
    let registry = ExecutionControlRegistry::new();
    registry.register_turn("turn-c1-live", Some("session-c1"), None);

    // Act
    let token = registry.cancellation_token("turn-c1-live");

    // Assert
    let token = token.expect("C1：register_turn 后必须能取到该 turn 的取消令牌（accessor 返回 Some）");
    assert!(
        !token.is_cancelled(),
        "C1：新注册 turn 的令牌不得处于已取消态"
    );
}

/// C1：`request_stop` 被接受 ⇒ cancel 该 turn 令牌（且是同一实例，accessor 复取仍为已取消）。
#[test]
fn c1_accepted_stop_cancels_the_turn_token() {
    // Arrange
    let registry = ExecutionControlRegistry::new();
    registry.register_turn("turn-c1-accepted", Some("session-c1"), None);
    let token = registry
        .cancellation_token("turn-c1-accepted")
        .expect("C1：前置条件——令牌必须存在");
    assert!(!token.is_cancelled(), "C1：前置条件——停止前不得已取消");

    // Act
    let response = registry.request_stop("turn-c1-accepted", StopReason::User);

    // Assert
    assert!(response.accepted, "C1：运行中 turn 的 stop 必须被接受");
    assert!(
        token.is_cancelled(),
        "C1：被接受的 stop 必须 cancel 该 turn 的令牌"
    );
    let reacquired = registry
        .cancellation_token("turn-c1-accepted")
        .expect("C1：停止后 accessor 仍必须可用");
    assert!(
        reacquired.is_cancelled(),
        "C1：令牌的取消态必须对后续 accessor 可见（同一 turn 一个令牌）"
    );
}

/// C1：未被接受的 stop（未知 turn / 已终态）**不得** cancel 任何令牌。
#[test]
fn c1_rejected_stop_never_cancels_a_token() {
    // Arrange
    let registry = ExecutionControlRegistry::new();
    registry.register_turn("turn-c1-terminal", Some("session-c1"), None);
    registry.update("turn-c1-terminal", |checkpoint| {
        checkpoint.status = "completed".to_string();
    });
    let terminal_token = registry
        .cancellation_token("turn-c1-terminal")
        .expect("C1：前置条件——已终态 turn 的令牌仍应可取");

    // Act 1：未知 turn
    let unknown = registry.request_stop("turn-c1-missing", StopReason::User);
    // Act 2：已终态 turn
    let terminal = registry.request_stop("turn-c1-terminal", StopReason::User);

    // Assert
    assert!(!unknown.accepted, "C1：未知 turn 的 stop 不得被接受");
    assert_eq!(unknown.state, "not_found");
    assert!(
        registry.cancellation_token("turn-c1-missing").is_none(),
        "C1：未知 turn ⇒ cancellation_token 必须返回 None"
    );
    assert!(!terminal.accepted, "C1：已终态 turn 的 stop 不得被接受");
    assert_eq!(terminal.state, "already_terminal");
    assert!(
        !terminal_token.is_cancelled(),
        "C1：未被接受的 stop 不得 cancel 该 turn 令牌"
    );
}

// ---------------------------------------------------------------------------
// C2 dispatcher 观测 turn 令牌
// ---------------------------------------------------------------------------

/// C2：`DispatchContext.turn_cancellation = Some(token)` ⇒ token cancel 后本次调用 fail-closed 为
/// `Cancelled`（沿用既有词表 `cancelled`/`tool_cancelled`，legacy status `aborted`）。
#[test]
fn c2_dispatcher_observes_turn_cancellation_token() {
    // Arrange
    let token = Arc::new(CancellationToken::new());
    let dispatcher = dispatcher_with("dynamic:wave62_blocking");
    let (started_tx, started_rx) = mpsc::channel::<()>();
    dispatcher.register_handler(
        "dynamic:wave62_blocking",
        Arc::new(BlockingUntilCancelledHandler {
            token: Arc::clone(&token),
            started: started_tx,
        }),
    );
    let context = dispatch_context(Some(Arc::clone(&token)));
    let (outcome_tx, outcome_rx) = mpsc::channel::<ToolOutcome>();
    let dispatch_thread = Arc::clone(&dispatcher);
    let started_at = Instant::now();
    std::thread::spawn(move || {
        let outcome = dispatch_thread.dispatch_governed(
            request(InvocationOrigin::Model, "dynamic:wave62_blocking", "call-c2"),
            &context,
        );
        let _ = outcome_tx.send(outcome);
    });

    // Act：等 handler 真正开始（若实现选择「闸门前置拦截」，则不会开始，属契约允许路径），
    // 然后取消 turn 令牌——不使用盲等，且上界 500ms 避免死等。
    let handler_started = started_rx.recv_timeout(Duration::from_millis(500)).is_ok();
    let cancelled_at = Instant::now();
    token.cancel();
    let outcome = outcome_rx
        .recv_timeout(Duration::from_millis(3_000))
        .expect("C2：turn 令牌 cancel 后 dispatch 必须返回（3s 观测上界；超时=未观测令牌）");
    let exit_ms = cancelled_at.elapsed().as_millis() as u64;

    // Assert
    assert_eq!(
        outcome.execution_status,
        ToolExecutionStatus::Cancelled,
        "C2：turn 令牌已 cancel ⇒ 本次调用必须 fail-closed 为 Cancelled（handler_started={handler_started}, 退出 {}ms）",
        exit_ms
    );
    let legacy = outcome
        .result
        .as_ref()
        .expect("C2：取消结果必须携带 legacy 结果（status 词表）");
    assert_eq!(
        legacy.status, "aborted",
        "C2：legacy 词表必须沿用 `aborted`（ToolExecutionStatus::Cancelled ↔ aborted）"
    );
    let code = outcome_error_code(&outcome);
    assert!(
        matches!(code.as_deref(), Some("cancelled") | Some("tool_cancelled")),
        "C2：必须沿用既有取消错误码词表（cancelled / tool_cancelled），不得新造码；实际 = {code:?}"
    );
    assert!(
        started_at.elapsed() < Duration::from_secs(5),
        "C2：取消判据必须在秒级内完成，实际 {:?}",
        started_at.elapsed()
    );
}

/// C2：`turn_cancellation: None` ⇒ 行为完全不变（正常完成，status `ok`）。
#[test]
fn c2_none_turn_cancellation_preserves_existing_behaviour() {
    // Arrange
    let dispatcher = dispatcher_with("dynamic:wave62_echo");
    dispatcher.register_handler(
        "dynamic:wave62_echo",
        Arc::new(FixedValueHandler(json!({ "ok": true }))),
    );

    // Act
    let outcome = dispatcher.dispatch_governed(
        request(InvocationOrigin::Model, "dynamic:wave62_echo", "call-c2-none"),
        &dispatch_context(None),
    );

    // Assert
    assert_eq!(
        outcome.execution_status,
        ToolExecutionStatus::Ok,
        "C2 向后兼容：未注入 turn 令牌时既有行为必须不变（正常完成）"
    );
    assert_eq!(
        outcome_error_code(&outcome),
        None,
        "C2 向后兼容：正常完成不得出现错误码"
    );
}

// ---------------------------------------------------------------------------
// C3 子进程类工具在取消时被真正终止
// ---------------------------------------------------------------------------

fn run_command_call(command: String) -> ToolCall {
    ToolCall {
        call_id: Some("call-c3".to_string()),
        name: "workspace_run_command".to_string(),
        arguments: json!({ "command": command, "timeoutMs": 10_000 }),
        plan: None,
    }
}

/// C3：`sleep` 类长命令在 turn 令牌 cancel 后 ≤1000ms 返回 `aborted`/`tool_cancelled`，
/// 且**无残留子进程**（被取消的命令不得在其后写入副作用文件）。
#[test]
fn c3_run_command_is_terminated_when_turn_is_cancelled() {
    // Arrange
    let workspace = temp_workspace("cancel");
    let marker = workspace.join("c3-leaked-side-effect.txt");
    let command = if cfg!(windows) {
        format!(
            "ping -n 4 127.0.0.1 > nul & echo leaked > \"{}\"",
            marker.display()
        )
    } else {
        // 1.2s 后写副作用文件：若取消未真正杀进程，marker 会被写出（可观测证据）。
        format!("sleep 1.2; echo leaked > '{}'", marker.display())
    };
    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_sandbox_backend(TestSandboxBackend::available());
    let token = Arc::new(CancellationToken::new());
    let context = ToolExecutionContext {
        workspace_root: Some(workspace.clone()),
        cancellation: Some(Arc::clone(&token)),
        ..Default::default()
    };
    let call = run_command_call(command);
    let (_result_tx, result_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = router.execute_with_context(&call, &context);
        let _ = _result_tx.send(result);
    });

    // Act：命令真正开始执行后（契约 C3 用 200ms 量级）再取消。
    std::thread::sleep(Duration::from_millis(200));
    let cancelled_at = Instant::now();
    token.cancel();
    let tool_result = result_rx
        .recv_timeout(Duration::from_millis(TURN_CANCEL_EXIT_BOUND_MS + 2_000))
        .expect("C3：cancel 后 run_command 必须返回（观测上界 3s；超时=未轮询取消令牌）");
    let exit_ms = cancelled_at.elapsed().as_millis() as u64;

    // Assert 1（语义优先）：取消后必须返回 aborted，而不是等命令自然结束
    assert_eq!(
        tool_result.status, "aborted",
        "C3：被取消的 run_command 必须返回 status=\"aborted\"（现状：不轮询令牌 ⇒ 命令自然跑完），         实际 status={:?}、cancel→返回 {}ms、output={}",
        tool_result.status, exit_ms, tool_result.output
    );
    let payload = serde_json::from_str::<Value>(&tool_result.output).expect("C3：run_command 输出必须为 JSON");
    let code = payload
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert_eq!(
        code, "tool_cancelled",
        "C3：错误码必须沿用既有 `tool_cancelled`（不得新增码），实际 = {code:?}"
    );

    // Assert 3：时延上界（cancel → 返回 ≤1000ms，NFR `turn_cancel_exit_bound_ms`）
    assert!(
        exit_ms <= TURN_CANCEL_EXIT_BOUND_MS,
        "C3：cancel → run_command 返回必须 ≤{TURN_CANCEL_EXIT_BOUND_MS}ms（轮询上界 {TURN_CANCEL_POLL_MS}ms），实际 {exit_ms}ms"
    );

    // Assert 2：子进程已真正退出 —— 等过命令原本要写副作用文件的时刻，marker 必须不存在
    std::thread::sleep(Duration::from_millis(1_400));
    assert!(
        !marker.exists(),
        "C3：取消必须真正终止子进程（否则 1.2s 后会把副作用写进 {:?}）",
        marker
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

/// C3 回归：未取消时 run_command 行为不变（短命令正常完成）。
#[test]
fn c3_run_command_completes_normally_without_cancellation() {
    // Arrange
    let workspace = temp_workspace("normal");
    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_sandbox_backend(TestSandboxBackend::available());
    let token = Arc::new(CancellationToken::new());
    let context = ToolExecutionContext {
        workspace_root: Some(workspace.clone()),
        cancellation: Some(Arc::clone(&token)),
        ..Default::default()
    };

    // Act
    let result = router.execute_with_context(&run_command_call("echo hi".to_string()), &context);

    // Assert
    assert_eq!(
        result.status, "ok",
        "C3 回归：未取消的命令必须正常完成，实际 status={:?} output={}",
        result.status, result.output
    );
    assert!(
        result.output.contains("hi"),
        "C3 回归：命令输出必须回传，实际 = {}",
        result.output
    );
    assert!(
        !token.is_cancelled(),
        "C3 回归：正常完成路径不得自我置位取消"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}
