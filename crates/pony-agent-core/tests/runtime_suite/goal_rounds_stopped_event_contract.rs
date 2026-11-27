//! 契约 C3（O6 领域事实）红相验收：`GoalEvent::RoundsStopped` 的广播面。
//!
//! 契约真值源：`.dev-team/plan-goal-gate-v1.md` §0 C3（本文件由独立 Test Agent 冻结，
//! Executor 只读，禁改断言迁就实现）。
//!
//! 要防的回归（`docs/analysis/goal-ask-blocking-audit-2026-11-26.md` P1-3）：
//! 桌面端只有 `goal://changed` / `goal://round`（`status:"started"`）/ `goal://cleared`，
//! **没有「续轮停止」事件面** ⇒ 目标停下时 UI 零信号，用户只看到「这一轮结束了」。
//!
//! 冻结面：
//! ```ignore
//! pub enum GoalEvent { /* 既有三者 */ RoundsStopped {
//!     session_id: String, goal_id: String, reason_code: String, visible_line: String,
//!     rounds_started: u32, max_rounds: u32, consecutive_failures: u32,
//! } }
//! pub fn notify_goal_rounds_stopped(session_id: &str, goal_id: &str, reason: &GoalRoundStopReason,
//!     consecutive_failures: u32, rounds_started: u32, max_rounds: u32);
//! ```
//!
//! 铁律：每条用例真实断言；无 `#[ignore]` / skip / 同义反复断言；不修改 `src/`。

use std::sync::{Arc, Mutex};

use pony_agent_core::agent::goal::{
    install_goal_event_observer, notify_goal_rounds_stopped, reset_goal_event_observer_for_test,
    GoalEvent, GoalRoundStopReason,
};

/// 收集观察者收到的全部事件（进程级观察者只允许一个，故用例串行）。
type Captured = Arc<Mutex<Vec<GoalEvent>>>;

/// 观察者守卫：Drop 时无条件复位进程级观察者。
///
/// 为什么不用裸 `reset_goal_event_observer_for_test()` 收尾：断言失败会 `panic!` 展开，
/// 若清理写在用例末尾就不执行 ⇒ 收集器泄漏到同进程后续用例（跨用例串话、随机红）。
struct ObserverGuard {
    captured: Captured,
}

impl ObserverGuard {
    /// Arrange：安装收集器（覆盖任何残留观察者，保证本用例只看到自己的事件）。
    fn install() -> Self {
        let captured: Captured = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&captured);
        install_goal_event_observer(Arc::new(move |event: &GoalEvent| {
            sink.lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(event.clone());
        }));
        Self { captured }
    }
}

impl Drop for ObserverGuard {
    fn drop(&mut self) {
        reset_goal_event_observer_for_test();
    }
}

/// 取出唯一一条 `RoundsStopped` 事件（其余事件视为违约噪声）。
fn take_single_rounds_stopped(captured: &Captured) -> GoalEvent {
    let events = captured
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    assert_eq!(
        events.len(),
        1,
        "契约 C3 违约：一次 notify_goal_rounds_stopped 必须只广播一条事件，实际 {} 条",
        events.len()
    );
    match &events[0] {
        GoalEvent::RoundsStopped { .. } => events[0].clone(),
        other => panic!("契约 C3 违约：必须广播 RoundsStopped，实际 {other:?}"),
    }
}

/// `notify_goal_rounds_stopped(TurnSuspended)` ⇒ `RoundsStopped`，且
/// `reason_code == "turn_suspended"`、`visible_line == reason.visible_line()`。
///
/// `visible_line` 必须与领域侧同源（不得让宿主各自拼字面量），否则「暂停等你作答」
/// 与「首轮失败」会在 UI 与日志里打架。
#[test]
fn c3_rounds_stopped_event_carries_reason_code_and_visible_line() {
    // Arrange
    let _guard = crate::env_lock::lock();
    let observer = ObserverGuard::install();
    let captured = Arc::clone(&observer.captured);
    let reason = GoalRoundStopReason::TurnSuspended;
    let session_id = "o6-rounds-stopped-session";
    let goal_id = "o6-rounds-stopped-goal";

    // Act
    notify_goal_rounds_stopped(session_id, goal_id, &reason, 0, 4, 20);

    // Assert
    let event = take_single_rounds_stopped(&captured);
    match event {
        GoalEvent::RoundsStopped {
            session_id: got_session,
            goal_id: got_goal,
            reason_code,
            visible_line,
            rounds_started,
            max_rounds,
            consecutive_failures,
        } => {
            assert_eq!(got_session, session_id, "session_id 必须原样透传");
            assert_eq!(got_goal, goal_id, "goal_id 必须原样透传");
            assert_eq!(
                reason_code,
                reason.code(),
                "reason_code 必须是领域侧 code()（TurnSuspended ⇒ turn_suspended）"
            );
            assert_eq!(reason_code, "turn_suspended", "reason_code 字面量必须稳定");
            assert_eq!(
                visible_line,
                reason.visible_line(),
                "visible_line 必须与 reason.visible_line() 完全一致（单一文案源）"
            );
            assert_eq!(rounds_started, 4, "rounds_started 必须原样透传");
            assert_eq!(max_rounds, 20, "max_rounds 必须原样透传");
            assert_eq!(consecutive_failures, 0, "consecutive_failures 必须原样透传");
        }
        other => panic!("契约 C3 违约：必须广播 RoundsStopped，实际 {other:?}"),
    }

    // Teardown 由 `ObserverGuard::drop` 承担（断言 panic 时同样复位观察者）。
}

/// 非挂起原因同样必须原样透传 `code()` 与 `visible_line()`（防「只对 TurnSuspended 特判」）。
#[test]
fn c3_rounds_stopped_event_reflects_other_reasons() {
    // Arrange
    let _guard = crate::env_lock::lock();
    let observer = ObserverGuard::install();
    let captured = Arc::clone(&observer.captured);
    let reason = GoalRoundStopReason::ConsecutiveFailures { failures: 2 };

    // Act
    notify_goal_rounds_stopped("o6-session-failures", "o6-goal-failures", &reason, 2, 6, 20);

    // Assert
    match take_single_rounds_stopped(&captured) {
        GoalEvent::RoundsStopped {
            reason_code,
            visible_line,
            consecutive_failures,
            ..
        } => {
            assert_eq!(
                reason_code, "consecutive_failures",
                "连续失败止损的 reason_code 必须稳定"
            );
            assert_eq!(visible_line, reason.visible_line(), "visible_line 必须同源");
            assert_eq!(consecutive_failures, 2, "consecutive_failures 必须透传真实计数");
        }
        other => panic!("契约 C3 违约：必须广播 RoundsStopped，实际 {other:?}"),
    }

    // Teardown 由 `ObserverGuard::drop` 承担。
}
