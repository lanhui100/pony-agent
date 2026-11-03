//! wave-60「可中断停止」黑盒验收（独立测试者，红相先写）。
//!
//! 契约真值源：`.dev-team/contracts/wave-60-interruptible-stop.md` §2。
//! 本文件覆盖：C1（stop 应答语义 / 封闭 state 集合 / 幂等 / checkpoint 落盘结转）、
//! C3（终态 stop_reason 的 wire 面与「非取消终态不带原因」）。
//!
//! 纪律（test-expert L2）：
//! - 每个用例 ≥1 个真实断言，AAA 结构，命名带判据号；
//! - 禁止 `#[ignore]` / `catch` 吞异常 / 空断言 / 以「命令 Exit 0」代替断言；
//! - 只使用 core 的**公开 API**（不读实现、不加 `mod` 声明到 `src/`）。
//!
//! 本文件按**冻结签名** `ExecutionControlRegistry::request_stop(&self, turn_id: &str, stop_reason: StopReason)` 编写，
//! 以便红相失败信息是「断言不成立」而不是「编译错误」——
//! 现状实测：`state="running"`、`stop_reason=None`，正是 C1 要求被点名的缺陷。
//! 需要注册面/监督面的判据（C2 立即应答、C5 宽限期、显式 reason 的 checkpoint 落盘）
//! 落在 `stop_escalation_wave60.rs`；两文件判据互补、互不重复。

use pony_agent_core::agent::execution_control::{ExecutionControlRegistry, StopTurnResponse};
use pony_agent_core::agent::stop_escalation::StopReason;
use pony_agent_core::agent::runtime::TurnStreamEvent;
use pony_agent_core::agent::turn_flow::{emit_stream_failed, TurnEventSink};
use std::sync::Mutex;

/// C1 §2：`state` 的封闭取值集合（`"running"` 已废弃）。
const CLOSED_STATE_SET: [&str; 3] = ["stopping", "not_found", "already_terminal"];

/// 记录型 sink：捕获 `(name, payload)`，用于断言事件条数与载荷字段。
#[derive(Default)]
struct RecordingSink {
    captured: Mutex<Vec<(String, TurnStreamEvent)>>,
}

impl RecordingSink {
    fn new() -> Self {
        Self::default()
    }

    // wave-61-team CI 修复（2026-11-03）：该辅助方法在 wave-60 收口后已无调用方，
    // 而 CI 的 Windows 测试步骤带 `-D warnings` ⇒ `-D dead-code` 使整个测试二进制编译失败。
    // 仅抑制 lint、不改任何断言（冻结用例语义不变）；如需删除 helper 属独立技术债清理。
    #[allow(dead_code)]
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
            .filter(|(captured_name, _)| captured_name == name)
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

fn running_registry(turn_id: &str, session_id: Option<&str>) -> ExecutionControlRegistry {
    let registry = ExecutionControlRegistry::new();
    registry.register_turn(turn_id, session_id, None);
    registry
}

fn assert_state_in_closed_set(response: &StopTurnResponse, scenario: &str) {
    assert!(
        CLOSED_STATE_SET.contains(&response.state.as_str()),
        "{scenario}：wave-60 C1 规定 state 取值必须落在封闭集合 {CLOSED_STATE_SET:?}（\"running\" 已废弃），实际 = {:?}",
        response.state
    );
}

/// C1：运行中 turn 的缺省停止请求 ⇒ accepted/stopping/stopReason="user"。
#[test]
fn c1_stop_turn_accepts_running_turn_as_stopping() {
    // Arrange
    let registry = running_registry("turn-c1-accept", Some("session-c1"));

    // Act
    let response = registry.request_stop("turn-c1-accept", StopReason::User);

    // Assert
    assert_eq!(response.turn_id, "turn-c1-accept");
    assert!(response.accepted, "C1：运行中 turn 的停止请求必须被接受");
    assert_eq!(
        response.state, "stopping",
        "C1：被接受时 state 必须是 \"stopping\"（旧 \"running\" 废弃）"
    );
    assert_eq!(
        response.stop_reason.as_deref(),
        Some("user"),
        "C1：reason 缺省必须回带 stopReason=\"user\""
    );
    assert_state_in_closed_set(&response, "运行中-accepted");
}

/// C1：未知 turn_id ⇒ accepted=false / state="not_found" / 无 reason。
#[test]
fn c1_stop_turn_unknown_turn_is_not_found() {
    // Arrange
    let registry = running_registry("turn-c1-existing", Some("session-c1"));

    // Act
    let response = registry.request_stop("turn-c1-does-not-exist", StopReason::User);

    // Assert
    assert!(!response.accepted, "C1：未知 turn 不得被接受");
    assert_eq!(response.state, "not_found");
    assert_eq!(
        response.stop_reason, None,
        "C1：未被接受的停止请求不得回带 stopReason"
    );
    assert_state_in_closed_set(&response, "未知 turn");
}

/// C1：已终态 turn（cancelled/completed/failed 三选一）⇒ already_terminal。
#[test]
fn c1_stop_turn_terminal_turn_is_already_terminal() {
    for terminal_status in ["cancelled", "completed", "failed"] {
        // Arrange
        let turn_id = format!("turn-c1-terminal-{terminal_status}");
        let registry = running_registry(&turn_id, Some("session-c1"));
        registry.update(&turn_id, |checkpoint| {
            checkpoint.status = terminal_status.to_string();
        });

        // Act
        let response = registry.request_stop(&turn_id, StopReason::User);

        // Assert
        assert!(
            !response.accepted,
            "C1：已终态 turn（{terminal_status}）不得再被接受"
        );
        assert_eq!(
            response.state, "already_terminal",
            "C1：已终态 turn（{terminal_status}）必须报 already_terminal"
        );
        assert_state_in_closed_set(&response, terminal_status);
    }
}

/// C1：同一 turn 连续两次 stop ⇒ 第 1 次 accepted，第 2 次 accepted=false/state="stopping"（幂等）。
#[test]
fn c1_repeated_stop_is_idempotent_and_reports_stopping() {
    // Arrange
    let registry = running_registry("turn-c1-idempotent", Some("session-c1"));

    // Act
    let first = registry.request_stop("turn-c1-idempotent", StopReason::User);
    let second = registry.request_stop("turn-c1-idempotent", StopReason::User);

    // Assert
    assert!(first.accepted, "C1：第 1 次 stop 必须被接受");
    assert_eq!(first.state, "stopping");
    assert!(
        !second.accepted,
        "C1：第 2 次 stop 不得重复接受（幂等，不重复派发）"
    );
    assert_eq!(
        second.state, "stopping",
        "C1：第 2 次 stop 的 state 必须是 \"stopping\"（不是 already_terminal）"
    );
    assert_state_in_closed_set(&first, "幂等-第1次");
    assert_state_in_closed_set(&second, "幂等-第2次");
}

/// C1：checkpoint 同时落盘 stop_requested_at_ms 与 stop_reason，且重注册结转。
#[test]
fn c1_checkpoint_persists_stop_reason_and_carries_over_reregistration() {
    // Arrange
    let registry = running_registry("turn-c1-checkpoint", Some("session-c1"));

    // Act
    let response = registry.request_stop("turn-c1-checkpoint", StopReason::User);
    let checkpoint = registry
        .load_checkpoint(Some("turn-c1-checkpoint"), None)
        .expect("C1：注册过的 turn 必须有 checkpoint");
    registry.register_turn("turn-c1-checkpoint", Some("session-c1"), None);
    let reregistered = registry
        .load_checkpoint(Some("turn-c1-checkpoint"), None)
        .expect("C1：重注册后 checkpoint 必须仍可读");

    // Assert
    assert!(response.accepted);
    assert!(
        checkpoint.stop_requested_at_ms.is_some(),
        "C1：被接受的 stop 必须落盘 stop_requested_at_ms"
    );
    assert_eq!(
        checkpoint.stop_reason.as_deref(),
        Some("user"),
        "C1：被接受的 stop 必须同时落盘 stop_reason=user"
    );
    assert_eq!(
        reregistered.stop_requested_at_ms,
        checkpoint.stop_requested_at_ms,
        "C1：register_turn 重注册必须结转 stop_requested_at_ms"
    );
    assert_eq!(
        reregistered.stop_reason.as_deref(),
        Some("user"),
        "C1：register_turn 重注册必须结转 stop_reason"
    );
}

/// C3（wire 兼容）：stop_reason 以**追加**字段 `stopReason` 下发；缺省（None）不得下发虚假原因。
///
/// 说明：本用例只锁 C3 的 wire 面（serde camelCase + 追加语义），不绑定
/// `emit_stream_cancelled` 的形参列表（该函数在本波会增加 stop_reason 入参，
/// 形参形状归 Executor，判据不因形参变化而失效）。
#[test]
fn c3_cancelled_event_wire_carries_stop_reason() {
    // Arrange：任一终态事件载荷作为基线（failed 的 stop_reason 恒为 None）
    let sink = RecordingSink::new();
    emit_stream_failed(
        &sink,
        "turn-c3-wire".to_string(),
        None,
        Vec::new(),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        "stopped_by_user".to_string(),
        Some("session-c3".to_string()),
    );
    let baseline = sink
        .events_named("turn:failed")
        .into_iter()
        .next()
        .expect("C3：前置条件——必须有一条终态事件载荷");

    // Act + Assert：携带原因时 wire 键为 camelCase `stopReason`
    let mut payload = baseline.clone();
    payload.kind = "cancelled".to_string();
    payload.phase = Some("cancelled".to_string());
    payload.stop_reason = Some("user".to_string());
    let with_reason = serde_json::to_value(&payload).expect("C3：TurnStreamEvent 必须可序列化");
    assert_eq!(
        with_reason.get("stopReason").and_then(|value| value.as_str()),
        Some("user"),
        "C3：stop_reason=user 必须以 camelCase wire 键 stopReason 下发（追加字段，旧消费者不读即不变）"
    );

    // Act + Assert：无原因（completed/failed）不得下发虚假值
    payload.stop_reason = None;
    let without_reason = serde_json::to_value(&payload).expect("C3：TurnStreamEvent 必须可序列化");
    assert!(
        matches!(
            without_reason.get("stopReason"),
            None | Some(serde_json::Value::Null)
        ),
        "C3：无 stop_reason 时不得下发虚假原因值，实际 {:?}",
        without_reason.get("stopReason")
    );
}

/// C3：`turn:failed`（非取消终态）不得携带 stopReason。
#[test]
fn c3_failed_terminal_event_carries_no_stop_reason() {
    // Arrange
    let sink = RecordingSink::new();

    // Act
    emit_stream_failed(
        &sink,
        "turn-c3-failed".to_string(),
        None,
        Vec::new(),
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        "provider_error".to_string(),
        Some("session-c3".to_string()),
    );

    // Assert
    let failed = sink.events_named("turn:failed");
    assert_eq!(failed.len(), 1, "C3：必须恰好派发一条 turn:failed");
    assert_eq!(
        failed[0].stop_reason, None,
        "C3：turn:failed 的 stopReason 必须为 None（只有 cancelled 承载停止原因）"
    );
}
