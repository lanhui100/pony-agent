//! PA-171 **遗留 #11 机械锚点**：goal 续轮租约（`agent::goal_lease`）+ 终态五臂归约
//! （`control_plane::turn_stream_outcome_from_terminal`）。
//!
//! 契约真值源：`.dev-team/contract-matrix-pa172.md` §7（v2 收窄版）+ PA-171 矩阵 §2.3 C1-T2 / §3.1 C1-A4。
//!
//! - §7.1：`GoalRoundLoopLease` 从 `bin/tui.rs` 抽到库模块，语义逐字不变：
//!   `Arc<Mutex<HashSet<String>>>` **按 `session_id` 分区**、`insert` 判互斥、RAII `Drop` 释放
//!   （含 panic 展开）；`try_acquire` 抢不到 ⇒ `None`。
//! - §7.2：归约真值表 10 例（决策五臂 × phase 五臂）。
//!
//! 本文件由独立 Test Agent 冻结；Executor 只读，禁改。
//!
//! 红相预期（wave-69 实现前）：`agent::goal_lease` 模块与 `pub turn_stream_outcome_from_terminal`
//! 都不存在 ⇒ 编译红（E0432/E0603），属冻结签名锚点的合法红相。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Barrier};

use pony_agent_core::agent::control_plane::{turn_stream_outcome_from_terminal, TurnStreamOutcome};
use pony_agent_core::agent::goal_lease::{
    new_goal_round_loop_lease_registry, GoalRoundLoopLease,
};
use pony_agent_core::agent::graph::GraphDecisionKind;

// ── §7.1：租约按会话分区 / 互斥 / Drop 释放 ──────────────────────────────────────

/// §7.1：**同一会话**第二次 `try_acquire` 必须返回 `None`（已有循环持有）。
#[test]
fn lease_blocks_second_acquire_in_same_session() {
    // Arrange
    let registry = new_goal_round_loop_lease_registry();
    let first = GoalRoundLoopLease::try_acquire(&registry, "session-a");

    // Act
    let second = GoalRoundLoopLease::try_acquire(&registry, "session-a");

    // Assert
    let first = first.expect("§7.1：首个小节必须能抢到租约");
    assert_eq!(
        first.session_id(),
        "session-a",
        "§7.1：`session_id()` 必须回读抢租时的会话 id"
    );
    assert!(
        second.is_none(),
        "§7.1：同会话第二次抢占必须返回 None（按 session 互斥），实际 Some"
    );
}

/// §7.1：**不同会话**各自可抢到租约，且互不阻塞。
#[test]
fn lease_allows_distinct_sessions_concurrently() {
    // Arrange
    let registry = new_goal_round_loop_lease_registry();

    // Act
    let session_a = GoalRoundLoopLease::try_acquire(&registry, "session-a");
    let session_b = GoalRoundLoopLease::try_acquire(&registry, "session-b");

    // Assert
    let session_a = session_a.expect("§7.1：会话 A 必须能抢到租约");
    let session_b = session_b.expect("§7.1：会话 B 必须能抢到租约（不得被 A 阻塞）");
    assert_eq!(session_a.session_id(), "session-a");
    assert_eq!(session_b.session_id(), "session-b");
}

/// §7.1：`Drop` 释放后**同一会话**可再次抢到（RAII 语义）。
#[test]
fn lease_released_after_drop() {
    // Arrange
    let registry = new_goal_round_loop_lease_registry();
    {
        let held = GoalRoundLoopLease::try_acquire(&registry, "session-drop")
            .expect("§7.1：首个小节必须能抢到租约");
        assert_eq!(held.session_id(), "session-drop");
    }

    // Act
    let reacquired = GoalRoundLoopLease::try_acquire(&registry, "session-drop");

    // Assert
    assert!(
        reacquired.is_some(),
        "§7.1：`Drop` 之后同会话必须能再次抢到租约，实际 None（说明 Drop 未 remove）"
    );
}

/// §7.1：**panic 展开**路径下租约必须释放（`catch_unwind` 内持有租约再 panic）。
#[test]
fn lease_released_on_panic_unwind() {
    // Arrange
    let registry = new_goal_round_loop_lease_registry();
    let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe({
        let registry = Arc::clone(&registry);
        move || {
            let held = GoalRoundLoopLease::try_acquire(&registry, "session-panic")
                .expect("§7.1：panic 场景下首个小节必须能抢到租约");
            assert_eq!(held.session_id(), "session-panic");
            panic!("PA-172 有意 panic：验证租约在展开路径释放");
        }
    }));

    // Act
    let reacquired = GoalRoundLoopLease::try_acquire(&registry, "session-panic");

    // Assert
    assert!(unwound.is_err(), "§7.1 前置：闭包必须真的 panic 展开");
    assert!(
        reacquired.is_some(),
        "§7.1：panic 展开必须触发 RAII `Drop` 释放租约，否则该会话续轮永久卡死，实际 None"
    );
}

/// L2-AT（对抗）：固定 4 线程 + `Barrier` 同步 + 固定 50 轮 ⇒ 每轮**恰好 1 个**成功。
///
/// 判据（禁止 `sleep` 概率等待）：`Barrier` 对齐起跑，且租约**持有跨过第二个 Barrier**
/// （所有线程都尝试过抢租约后才释放）⇒ 「恰好 1」是确定事实而非概率。
#[test]
fn lease_concurrent_acquire_yields_exactly_one() {
    // Arrange
    const THREADS: usize = 4;
    const ROUNDS: usize = 50;
    let registry = new_goal_round_loop_lease_registry();

    for round in 0..ROUNDS {
        let barrier = Arc::new(Barrier::new(THREADS));
        let successes = Arc::new(AtomicUsize::new(0));

        // Act
        let handles: Vec<std::thread::JoinHandle<()>> = (0..THREADS)
            .map(|_| {
                let registry = Arc::clone(&registry);
                let barrier = Arc::clone(&barrier);
                let successes = Arc::clone(&successes);
                std::thread::spawn(move || {
                    barrier.wait(); // 对齐起跑
                    let lease = GoalRoundLoopLease::try_acquire(&registry, "session-race");
                    if lease.is_some() {
                        successes.fetch_add(1, Ordering::SeqCst);
                    }
                    barrier.wait(); // 持租约跨过所有线程的抢占尝试
                    drop(lease);
                })
            })
            .collect();
        for handle in handles {
            handle
                .join()
                .unwrap_or_else(|_| panic!("L2-AT：第 {round} 轮的租约线程不得 panic"));
        }

        // Assert
        assert_eq!(
            successes.load(Ordering::SeqCst),
            1,
            "L2-AT：第 {round} 轮 {THREADS} 个线程同一会话抢占必须**恰好 1 个**成功，实际 {}",
            successes.load(Ordering::SeqCst)
        );
    }
}

// ── §7.2：终态归约真值表（决策五臂 × phase 五臂） ───────────────────────────────

/// 断言单条归约结果。
fn assert_outcome(
    label: &str,
    phase: &str,
    error: Option<&str>,
    decision: Option<GraphDecisionKind>,
    expected: TurnStreamOutcome,
) {
    let actual = turn_stream_outcome_from_terminal(
        phase,
        error.map(str::to_string),
        decision,
    );
    assert_eq!(
        actual, expected,
        "§7.2 真值表（{label}）：phase={phase:?} error={error:?} decision={decision:?} ⇒ 期望 {expected:?}，实际 {actual:?}"
    );
}

/// §7.2 真值表 10 例（逐行锚定）：
/// 决策五臂 `Fail⇒Failed{error}` / `Cancel⇒Cancelled` / `WaitUser⇒Suspended` /
/// `Pause⇒Cancelled` / `Continue|Complete⇒按 phase`；
/// phase 五臂 `failed⇒Failed` / `cancelled⇒Cancelled` / `suspended⇒Suspended` /
/// `completed⇒Completed` / 其它⇒`Unknown`。
#[test]
fn turn_stream_outcome_truth_table_is_frozen() {
    // 决策五臂（决策优先于 phase）
    assert_outcome(
        "决策臂 1：Fail",
        "completed",
        Some("upstream timeout"),
        Some(GraphDecisionKind::Fail),
        TurnStreamOutcome::Failed {
            error: Some("upstream timeout".to_string()),
        },
    );
    assert_outcome(
        "决策臂 2：Cancel",
        "failed",
        Some("ignored"),
        Some(GraphDecisionKind::Cancel),
        TurnStreamOutcome::Cancelled,
    );
    assert_outcome(
        "决策臂 3：WaitUser",
        "failed",
        None,
        Some(GraphDecisionKind::WaitUser),
        TurnStreamOutcome::Suspended,
    );
    // v3 更正（Lead 裁决）：`Pause ⇒ Cancelled`（HEAD 实现 `control_plane/mod.rs:705-710`
    // 「用户停止被折成 Pause」；矩阵 §7.2 早期版本误写 Suspended，已更正）。
    assert_outcome(
        "决策臂 4：Pause ⇒ Cancelled",
        "completed",
        None,
        Some(GraphDecisionKind::Pause),
        TurnStreamOutcome::Cancelled,
    );
    assert_outcome(
        "决策臂 5a：Continue 按 phase（cancelled）",
        "cancelled",
        None,
        Some(GraphDecisionKind::Continue),
        TurnStreamOutcome::Cancelled,
    );
    assert_outcome(
        "决策臂 5b：Complete 按 phase（completed）",
        "completed",
        None,
        Some(GraphDecisionKind::Complete),
        TurnStreamOutcome::Completed,
    );

    // phase 五臂（无决策真值）
    assert_outcome(
        "phase 臂 1：failed",
        "failed",
        Some("boom"),
        None,
        TurnStreamOutcome::Failed {
            error: Some("boom".to_string()),
        },
    );
    assert_outcome(
        "phase 臂 2：cancelled",
        "cancelled",
        None,
        None,
        TurnStreamOutcome::Cancelled,
    );
    assert_outcome(
        "phase 臂 3：suspended",
        "suspended",
        None,
        None,
        TurnStreamOutcome::Suspended,
    );
    assert_outcome(
        "phase 臂 4：completed",
        "completed",
        None,
        None,
        TurnStreamOutcome::Completed,
    );
    assert_outcome(
        "phase 臂 5：其它 ⇒ Unknown（不得当成功）",
        "planning",
        None,
        None,
        TurnStreamOutcome::Unknown,
    );
    assert_outcome(
        "phase 臂 5b：空 phase ⇒ Unknown",
        "",
        None,
        None,
        TurnStreamOutcome::Unknown,
    );
}
