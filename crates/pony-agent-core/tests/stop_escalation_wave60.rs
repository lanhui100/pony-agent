//! wave-60 C2 / C3(派发) / C5 黑盒验收（独立测试者，红相先写）。
//!
//! 契约真值源：`.dev-team/contracts/wave-60-interruptible-stop.md` §2 C2 / C3 / C5。
//!
//! 覆盖：
//! - C2：`stop_turn` 被接受时**同步**派发**恰好一条** `turn:stop_requested`（未接受不派发）；
//! - C3：监督者派发终态时携带的 `StopReason` 必须与该 turn 的 checkpoint `stop_reason` 一致；
//! - C5：可注入 `stop_grace_ms`/`stop_poll_tick_ms`（测试用 ≤50ms）；循环先终止 ⇒ **零派发**；
//!   未注册发射器 ⇒ 只落盘 checkpoint 且**不 panic**；监督者自身**不重复**派发。
//!
//! 纪律：真实断言、AAA、命名带判据号；不使用 `#[ignore]`、不吞异常、不以 Exit 0 代替断言。
//!
//! 依赖的**冻结签名**（protocol-agent 回执 + Lead 裁决）：
//! - `ExecutionControlRegistry::request_stop(&self, turn_id: &str, stop_reason: StopReason)`
//! - `StopTurnCommand { turn_id: String, stop_reason: Option<StopReason> }`
//! - `HostControlPlane::register_turn_event_sink(&self, turn_id: &str, sink: Arc<dyn TurnEventSink>)`
//! - `StopEscalation::with_registry(config, ExecutionControlRegistry)`

use pony_agent_core::agent::control_plane::{
    ExecutionCheckpointQuery, HostControlPlane, HostControlPlaneBuilder, StopTurnCommand,
};
use pony_agent_core::agent::execution_control::ExecutionControlRegistry;
use pony_agent_core::agent::runtime::TurnStreamEvent;
use pony_agent_core::agent::stop_escalation::{
    StopEscalation, StopEscalationConfig, StopReason, TerminalEmitter,
};
use pony_agent_core::agent::turn_flow::TurnEventSink;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 记录型 sink：捕获 `(name, payload)`。
#[derive(Default)]
struct RecordingSink {
    captured: Mutex<Vec<(String, TurnStreamEvent)>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self::default()
    }

    fn names(&self) -> Vec<String> {
        self.captured
            .lock()
            .expect("recording sink lock poisoned")
            .iter()
            .map(|(name, _)| name.clone())
            .collect()
    }

    fn events_named(&self, name: &str) -> Vec<TurnStreamEvent> {
        self.captured
            .lock()
            .expect("recording sink lock poisoned")
            .iter()
            .filter(|(captured, _)| captured == name)
            .map(|(_, payload)| payload.clone())
            .collect()
    }
}

impl TurnEventSink for RecordingSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.captured
            .lock()
            .expect("recording sink lock poisoned")
            .push((name.to_string(), payload));
    }
}

/// C5 的假终端发射器：只计数与留痕，不接触任何表面。
#[derive(Default)]
struct FakeEmitter {
    calls: AtomicUsize,
    records: Mutex<Vec<(String, Option<String>, StopReason)>>,
}

impl FakeEmitter {
    fn new() -> Self {
        Self::default()
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn records(&self) -> Vec<(String, Option<String>, StopReason)> {
        self.records
            .lock()
            .expect("fake emitter lock poisoned")
            .clone()
    }
}

impl TerminalEmitter for FakeEmitter {
    fn emit_cancelled(&self, turn_id: &str, session_id: Option<&str>, stop_reason: StopReason) {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.records
            .lock()
            .expect("fake emitter lock poisoned")
            .push((
                turn_id.to_string(),
                session_id.map(str::to_string),
                stop_reason,
            ));
    }
}

/// 显式条件轮询（禁止裸 sleep 盲等）：谓词为真即返回 true，超时返回 false。
fn wait_until<F: FnMut() -> bool>(mut predicate: F, timeout_ms: u64) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if predicate() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn control_plane_with_registry() -> (HostControlPlane, ExecutionControlRegistry) {
    let registry = ExecutionControlRegistry::new();
    let control_plane = HostControlPlaneBuilder::new()
        .execution_control(registry.clone())
        .build();
    (control_plane, registry)
}

fn checkpoint_of(
    control_plane: &HostControlPlane,
    turn_id: &str,
) -> pony_agent_core::agent::execution_control::ExecutionCheckpoint {
    control_plane
        .load_execution_checkpoint(ExecutionCheckpointQuery {
            turn_id: Some(turn_id.to_string()),
            session_id: None,
        })
        .expect("被监督的 turn 必须有 checkpoint")
}

// ---------------------------------------------------------------------------
// C1 显式 reason（需要 `request_stop(turn_id, StopReason)` 第 2 参）
// ---------------------------------------------------------------------------

/// C1：显式 reason="watchdog" ⇒ accepted/stopping/stopReason="watchdog"（并落盘 checkpoint）。
#[test]
fn c1_stop_turn_carries_explicit_watchdog_reason() {
    // Arrange
    let registry = ExecutionControlRegistry::new();
    registry.register_turn("turn-c1-watchdog", Some("session-c1"), None);

    // Act
    let response = registry.request_stop("turn-c1-watchdog", StopReason::Watchdog);

    // Assert
    assert!(response.accepted, "C1：运行中 turn 的 watchdog 停止请求必须被接受");
    assert_eq!(response.state, "stopping");
    assert_eq!(
        response.stop_reason.as_deref(),
        Some("watchdog"),
        "C1：显式 reason 必须原样回带（不得被缺省 user 覆盖）"
    );
    let checkpoint = registry
        .load_checkpoint(Some("turn-c1-watchdog"), None)
        .expect("C1：注册过的 turn 必须有 checkpoint");
    assert_eq!(
        checkpoint.stop_reason.as_deref(),
        Some("watchdog"),
        "C1：显式 reason 必须与 stop_requested_at_ms 一并落盘"
    );
    assert!(
        checkpoint.stop_requested_at_ms.is_some(),
        "C1：reason 与 stop_requested_at_ms 必须同写"
    );
}

/// C1：首次记录的原因不得被随后的 stop 改写（幂等）。
#[test]
fn c1_second_stop_does_not_override_recorded_reason() {
    // Arrange
    let registry = ExecutionControlRegistry::new();
    registry.register_turn("turn-c1-reason-sticky", Some("session-c1"), None);

    // Act
    let first = registry.request_stop("turn-c1-reason-sticky", StopReason::Parent);
    let second = registry.request_stop("turn-c1-reason-sticky", StopReason::Watchdog);

    // Assert
    assert!(first.accepted, "C1：首次 stop 必须被接受");
    assert!(!second.accepted, "C1：第 2 次 stop 必须被拒绝（幂等）");
    let checkpoint = registry
        .load_checkpoint(Some("turn-c1-reason-sticky"), None)
        .expect("C1：注册过的 turn 必须有 checkpoint");
    assert_eq!(
        checkpoint.stop_reason.as_deref(),
        Some("parent"),
        "C1：首次记录的原因不得被随后的 stop 改写"
    );
}

// ---------------------------------------------------------------------------
// C2 立即应答事件
// ---------------------------------------------------------------------------

/// C2：被接受的 stop 同步派发**恰好一条** `turn:stop_requested`，且载荷完整、不改终态归属。
#[test]
fn c2_accepted_stop_emits_exactly_one_stop_requested() {
    // Arrange
    let (control_plane, registry) = control_plane_with_registry();
    registry.register_turn("turn-c2-ack", Some("session-c2"), None);
    let sink = Arc::new(RecordingSink::new());
    control_plane.register_turn_event_sink("turn-c2-ack", sink.clone());

    // Act
    let response = control_plane.stop_turn(StopTurnCommand {
        turn_id: "turn-c2-ack".to_string(),
        stop_reason: None,
    });

    // Assert
    assert!(response.accepted, "C2：运行中 turn 的 stop 必须被接受");
    assert_eq!(response.state, "stopping");
    assert_eq!(response.stop_reason.as_deref(), Some("user"));

    let stop_requested = sink.events_named("turn:stop_requested");
    assert_eq!(
        stop_requested.len(),
        1,
        "C2：被接受的 stop 必须**同步**派发恰好一条 turn:stop_requested，实际全部事件 = {:?}",
        sink.names()
    );
    let payload = &stop_requested[0];
    assert_eq!(payload.turn_id, "turn-c2-ack", "C2：payload.turnId 必填");
    assert_eq!(
        payload.session_id.as_deref(),
        Some("session-c2"),
        "C2：checkpoint 有 session_id 时必须带上"
    );
    assert_eq!(payload.kind, "stop_requested", "C2：payload.kind 固定");
    assert_eq!(
        payload.phase.as_deref(),
        Some("stopping"),
        "C2：payload.phase 必须为 Some(\"stopping\")"
    );
    assert_eq!(
        payload.stop_reason.as_deref(),
        Some("user"),
        "C2：payload.stopReason 必须回带该轮原因"
    );
    assert!(
        payload.emitted_at_ms.is_some(),
        "C2：payload 必须携带 requestedAtMs（wire emittedAtMs）"
    );
    assert_eq!(
        sink.names(),
        vec!["turn:stop_requested".to_string()],
        "C2：stop 被接受时**只**派发立即应答事件，不得顺带改终态归属（终态仍只有 completed/failed/cancelled 三选一）"
    );
}

/// C2：显式 watchdog 原因的立即应答事件必须原样携带。
#[test]
fn c2_stop_requested_payload_carries_explicit_reason() {
    // Arrange
    let (control_plane, registry) = control_plane_with_registry();
    registry.register_turn("turn-c2-watchdog", Some("session-c2"), None);
    let sink = Arc::new(RecordingSink::new());
    control_plane.register_turn_event_sink("turn-c2-watchdog", sink.clone());

    // Act
    let response = control_plane.stop_turn(StopTurnCommand {
        turn_id: "turn-c2-watchdog".to_string(),
        stop_reason: Some(StopReason::Watchdog),
    });

    // Assert
    assert!(response.accepted);
    assert_eq!(response.stop_reason.as_deref(), Some("watchdog"));
    let stop_requested = sink.events_named("turn:stop_requested");
    assert_eq!(stop_requested.len(), 1);
    assert_eq!(
        stop_requested[0].stop_reason.as_deref(),
        Some("watchdog"),
        "C2：立即应答事件的原因必须与 StopTurnCommand 一致"
    );
}

/// C2：未被接受的 stop（未知 turn / 已终态 / 重复 stop）一律**不**派发立即应答事件。
#[test]
fn c2_rejected_stop_emits_no_stop_requested() {
    // Arrange
    let (control_plane, registry) = control_plane_with_registry();
    registry.register_turn("turn-c2-rejected", Some("session-c2"), None);
    let terminal_turn = "turn-c2-terminal";
    registry.register_turn(terminal_turn, Some("session-c2"), None);
    registry.update(terminal_turn, |checkpoint| {
        checkpoint.status = "completed".to_string();
    });
    let sink = Arc::new(RecordingSink::new());
    control_plane.register_turn_event_sink("turn-c2-rejected", sink.clone());
    control_plane.register_turn_event_sink(terminal_turn, sink.clone());

    // Act 1：未知 turn
    let unknown = control_plane.stop_turn(StopTurnCommand {
        turn_id: "turn-c2-missing".to_string(),
        stop_reason: None,
    });
    // Act 2：已终态 turn
    let terminal = control_plane.stop_turn(StopTurnCommand {
        turn_id: terminal_turn.to_string(),
        stop_reason: None,
    });
    // Act 3：首次接受 + 重复 stop
    let first = control_plane.stop_turn(StopTurnCommand {
        turn_id: "turn-c2-rejected".to_string(),
        stop_reason: None,
    });
    let repeat = control_plane.stop_turn(StopTurnCommand {
        turn_id: "turn-c2-rejected".to_string(),
        stop_reason: None,
    });

    // Assert
    assert!(!unknown.accepted, "C2：未知 turn 不得被接受");
    assert_eq!(unknown.state, "not_found");
    assert!(!terminal.accepted, "C2：已终态 turn 不得被接受");
    assert_eq!(terminal.state, "already_terminal");
    assert!(first.accepted, "C2：首次 stop 必须被接受");
    assert!(!repeat.accepted, "C2：重复 stop 必须被拒绝");
    assert_eq!(repeat.state, "stopping");
    assert_eq!(
        sink.events_named("turn:stop_requested").len(),
        1,
        "C2：4 次调用中只有 1 次被接受 ⇒ 立即应答事件必须恰好 1 条，实际 {:?}",
        sink.names()
    );
}

// ---------------------------------------------------------------------------
// C3 终态 stop_reason 与 checkpoint 一致（派发面）
// ---------------------------------------------------------------------------

/// C3：监督者派发终态时携带的 stop_reason 必须等于该 turn 的 checkpoint stop_reason。
#[test]
fn c3_escalated_terminal_dispatch_matches_checkpoint_stop_reason() {
    // Arrange
    let (_control_plane, registry) = control_plane_with_registry();
    registry.register_turn("turn-c3-escalate", Some("session-c3"), None);
    let emitter = Arc::new(FakeEmitter::new());
    let escalation =
        StopEscalation::with_registry(StopEscalationConfig::for_test(50, 5), registry.clone());
    escalation.register_emitter("turn-c3-escalate", emitter.clone());

    // Act
    let response = registry.request_stop("turn-c3-escalate", StopReason::Hook);
    assert!(response.accepted, "C3：前置 stop 必须被接受");
    escalation.on_stop_accepted("turn-c3-escalate", Some("session-c3"), StopReason::Hook);
    let dispatched = wait_until(|| emitter.call_count() == 1, 1_000);

    // Assert
    assert!(
        dispatched,
        "C3：宽限期内 checkpoint 仍为 running ⇒ 监督者必须派发一条 turn:cancelled，实际 {} 条",
        emitter.call_count()
    );
    let checkpoint = registry
        .load_checkpoint(Some("turn-c3-escalate"), None)
        .expect("C3：被监督的 turn 必须有 checkpoint");
    let records = emitter.records();
    let recorded_reason = records[0].2;
    assert_eq!(
        Some(recorded_reason),
        checkpoint
            .stop_reason
            .as_deref()
            .and_then(|value| value.parse::<StopReason>().ok()),
        "C3：终态派发的 stopReason（{}, 期望 wire = checkpoint.stop_reason = {:?}）必须与该轮 checkpoint 一致",
        recorded_reason.as_str(),
        checkpoint.stop_reason
    );
    assert_eq!(
        recorded_reason.as_str(),
        "hook",
        "C3：wire 值必须是 contract §1 的 snake_case 词表"
    );
}

// ---------------------------------------------------------------------------
// C5 宽限期强制收口
// ---------------------------------------------------------------------------

/// C5：注入 grace=50ms/tick=5ms —— 循环未终止时监督者幂等收口：checkpoint 置 cancelled + 恰好一条终态。
#[test]
fn c5_grace_escalation_cancels_running_checkpoint_and_dispatches_exactly_once() {
    // Arrange
    let (_control_plane, registry) = control_plane_with_registry();
    registry.register_turn("turn-c5-grace", Some("session-c5"), None);
    let emitter = Arc::new(FakeEmitter::new());
    let escalation =
        StopEscalation::with_registry(StopEscalationConfig::for_test(50, 5), registry.clone());
    escalation.register_emitter("turn-c5-grace", emitter.clone());

    // Act
    let response = registry.request_stop("turn-c5-grace", StopReason::User);
    assert!(response.accepted, "C5：前置 stop 必须被接受");
    escalation.on_stop_accepted("turn-c5-grace", Some("session-c5"), StopReason::User);
    let escalated = wait_until(|| emitter.call_count() >= 1, 1_000);
    // 观察窗口：再等 3 个 tick 量级，确认监督者不会重复派发（显式条件轮询，非盲等）
    let duplicated = wait_until(|| emitter.call_count() > 1, 60);

    // Assert
    assert!(
        escalated,
        "C5：注入 grace=50ms 后，仍在 running 的 checkpoint 必须被监督者收口（实际 0 条终态派发）"
    );
    assert!(
        !duplicated,
        "C5：监督者自身每 turn 至多派发一次终态，实际 {} 条",
        emitter.call_count()
    );
    assert_eq!(
        emitter.call_count(),
        1,
        "C5：恰好一条 turn:cancelled（表面层门禁去重不在 core 职责内，但监督者自身不得重复）"
    );
    let checkpoint = checkpoint_of(&_control_plane, "turn-c5-grace");
    assert_eq!(
        checkpoint.status, "cancelled",
        "C5：宽限期收口必须把 checkpoint 置 cancelled"
    );
    assert_eq!(
        checkpoint.stop_reason.as_deref(),
        Some("user"),
        "C5：收口时 checkpoint 必须保留 stop_reason"
    );
    let records = emitter.records();
    assert_eq!(
        records[0].0, "turn-c5-grace",
        "C5：终态必须归属被停止的 turn"
    );
    assert_eq!(
        records[0].1.as_deref(),
        Some("session-c5"),
        "C5：终态必须带上与会话一致的 session_id"
    );
}

/// C5：宽限期内循环已自行终止 ⇒ 监督者**必须不派发**任何事件，且不得改写已落地终态。
#[test]
fn c5_no_dispatch_when_loop_terminated_within_grace() {
    // Arrange
    let (_control_plane, registry) = control_plane_with_registry();
    registry.register_turn("turn-c5-done", Some("session-c5"), None);
    registry.update("turn-c5-done", |checkpoint| {
        checkpoint.status = "completed".to_string();
        checkpoint.stop_reason = None;
    });
    let emitter = Arc::new(FakeEmitter::new());
    let escalation =
        StopEscalation::with_registry(StopEscalationConfig::for_test(50, 5), registry.clone());
    escalation.register_emitter("turn-c5-done", emitter.clone());

    // Act
    escalation.on_stop_accepted("turn-c5-done", Some("session-c5"), StopReason::User);
    let dispatched = wait_until(|| emitter.call_count() > 0, 200);

    // Assert
    assert!(
        !dispatched,
        "C5：宽限期内循环已终止（checkpoint 非 running）⇒ 监督者必须零派发，实际 {} 条",
        emitter.call_count()
    );
    let checkpoint = checkpoint_of(&_control_plane, "turn-c5-done");
    assert_eq!(
        checkpoint.status, "completed",
        "C5：监督者不得把已完成的 turn 改写成 cancelled"
    );
    assert_eq!(
        checkpoint.stop_reason, None,
        "C5：已完成 turn 的 stop_reason 必须保持 None（C3：completed/failed 不带原因）"
    );
}

/// C5：未注册终端发射器（CLI 无 sink 场景）⇒ 只落盘 checkpoint，**不得 panic**。
#[test]
fn c5_missing_emitter_persists_checkpoint_without_panic() {
    // Arrange
    let (_control_plane, registry) = control_plane_with_registry();
    registry.register_turn("turn-c5-no-emitter", Some("session-c5"), None);
    let escalation =
        StopEscalation::with_registry(StopEscalationConfig::for_test(30, 5), registry.clone());

    // Act
    let response = registry.request_stop("turn-c5-no-emitter", StopReason::Watchdog);
    assert!(response.accepted);
    escalation.on_stop_accepted("turn-c5-no-emitter", Some("session-c5"), StopReason::Watchdog);
    // 未注册发射器时不得 panic：本用例能执行到断言即证明未 panic（panic 会直接判红）。
    let settled = wait_until(
        || {
            registry
                .load_checkpoint(Some("turn-c5-no-emitter"), None)
                .map(|checkpoint| checkpoint.status == "cancelled")
                .unwrap_or(false)
        },
        1_000,
    );

    // Assert
    assert!(
        settled,
        "C5：未注册发射器时监督者仍必须落盘 checkpoint=cancelled（不得 panic、不得跳过收口）"
    );
    let checkpoint = checkpoint_of(&_control_plane, "turn-c5-no-emitter");
    assert_eq!(
        checkpoint.stop_reason.as_deref(),
        Some("watchdog"),
        "C5：落盘必须保留 stop_reason"
    );
}
