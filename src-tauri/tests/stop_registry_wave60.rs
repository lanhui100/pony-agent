//! wave-60 C6 黑盒验收（独立测试者，红相先写）：tauri 任务注册表按 `turn_id` 隔离。
//!
//! 契约真值源：`.dev-team/contracts/wave-60-interruptible-stop.md` §C6。
//! 判据：
//! - 注册表 key 从 `session_id` 改为 `turn_id`；**同会话两轮并发**时，停止 A **不得** abort B 的句柄；
//! - `stop_turn` 的副作用顺序固定为 ① `request_stop`（含 C2 事件）→ ② 取消该 turn 的取消令牌 →
//!   ③ abort 该 turn 句柄（仅 L2 兜底）；顺序反转视为 FAIL（顺序判据由 core/tauri 侧集成测试覆盖，
//!   本文件覆盖可观测的注册表隔离语义）。
//!
//! 依赖的**冻结签名**（Lead 裁决，T3 必须交付）：
//! `pub mod turn_task_registry;` + `TurnTaskRegistry::{with_max_concurrent, register(turn_id, handle),
//! abort_turn(&self, turn_id) -> bool, unregister(turn_id)}`。
//!
//! 纪律：真实断言、AAA、禁止 `#[ignore]`/吞异常；不修改 `src-tauri/src/`。

use pony_agent_lib::turn_task_registry::TurnTaskRegistry;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 显式条件轮询（禁止裸 sleep 盲等）。
fn wait_until<F: FnMut() -> bool>(mut predicate: F, timeout_ms: u64) -> bool {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        if predicate() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// 永不结束的任务句柄（模拟 `spawn_blocking` 里卡住的 turn）。
fn spawn_pending_handle() -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        std::future::pending::<()>().await;
    })
}

/// 持续干活的任务句柄（用于观测「B 未被 A 连带 abort」）。
fn spawn_counting_handle(counter: Arc<AtomicUsize>) -> tauri::async_runtime::JoinHandle<()> {
    tauri::async_runtime::spawn(async move {
        loop {
            counter.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
        }
    })
}

/// C6：同会话两轮并发 —— `abort_turn(A)` 不得 abort B 的句柄。
#[test]
fn c6_stopping_turn_a_does_not_abort_turn_b_in_same_session() {
    // Arrange：同一 session 下两个 turn 各自注册
    let registry = TurnTaskRegistry::with_max_concurrent(4);
    let counter_b = Arc::new(AtomicUsize::new(0));
    registry
        .register("turn-a".to_string(), spawn_pending_handle())
        .expect("C6：turn A 注册必须成功");
    registry
        .register("turn-b".to_string(), spawn_counting_handle(Arc::clone(&counter_b)))
        .expect("C6：turn B 注册必须成功");
    assert!(
        wait_until(|| counter_b.load(Ordering::SeqCst) > 0, 2_000),
        "C6：前置条件——turn B 的句柄必须在跑"
    );

    // Act：只 abort A
    let aborted_a = registry.abort_turn("turn-a");
    let observed_before = counter_b.load(Ordering::SeqCst);
    let b_still_running = wait_until(
        || counter_b.load(Ordering::SeqCst) > observed_before,
        500,
    );

    // Assert
    assert!(aborted_a, "C6：abort_turn(A) 必须命中该 turn 的表项并返回 true");
    assert!(
        b_still_running,
        "C6：同会话停止 A 不得 abort B —— B 的句柄必须继续推进（观测到计数停在 {observed_before}）"
    );
    assert!(
        registry.abort_turn("turn-b"),
        "C6：B 必须仍在注册表内（若被 A 连带 abort/移除，此断言即红）"
    );
}

/// C6：注册表按 `turn_id` 键 —— 两次 register 各自独立，abort 后即出表（幂等 false）。
#[test]
fn c6_registry_is_keyed_by_turn_id_and_abort_is_single_use() {
    // Arrange
    let registry = TurnTaskRegistry::with_max_concurrent(4);
    registry
        .register("turn-keyed-1".to_string(), spawn_pending_handle())
        .expect("C6：注册 turn-1 必须成功");
    registry
        .register("turn-keyed-2".to_string(), spawn_pending_handle())
        .expect("C6：同会话第二个 turn 必须能独立注册（key 是 turn_id，不是 session_id）");

    // Act
    let first_abort = registry.abort_turn("turn-keyed-1");
    let second_abort = registry.abort_turn("turn-keyed-1");
    let unknown_abort = registry.abort_turn("turn-does-not-exist");

    // Assert
    assert!(first_abort, "C6：首次 abort 必须命中");
    assert!(
        !second_abort,
        "C6：同一 turn 第二次 abort 必须返回 false（表项已回收，幂等）"
    );
    assert!(!unknown_abort, "C6：未知 turn_id 的 abort 必须返回 false");
    assert!(
        registry.abort_turn("turn-keyed-2"),
        "C6：同会话另一 turn 不受影响，必须仍可 abort"
    );
}

/// C6：`unregister` 只回收指定 turn 的表项。
#[test]
fn c6_unregister_releases_only_the_named_turn() {
    // Arrange
    let registry = TurnTaskRegistry::with_max_concurrent(4);
    registry
        .register("turn-unregister-a".to_string(), spawn_pending_handle())
        .expect("C6：注册 A 必须成功");
    registry
        .register("turn-unregister-b".to_string(), spawn_pending_handle())
        .expect("C6：注册 B 必须成功");

    // Act
    registry.unregister("turn-unregister-a");
    let a_after_unregister = registry.abort_turn("turn-unregister-a");
    let b_after_unregister = registry.abort_turn("turn-unregister-b");

    // Assert
    assert!(
        !a_after_unregister,
        "C6：unregister 后该 turn 的行必须已回收"
    );
    assert!(
        b_after_unregister,
        "C6：unregister 不得误删其它 turn（B 必须仍可 abort）"
    );
}
