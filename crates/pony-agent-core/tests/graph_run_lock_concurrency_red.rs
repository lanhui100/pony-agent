//! L2-AT 对抗用例：`graph_runs` 锁重入自死锁的**确定性并发竞态**验证（无 sleep 概率等待）。
//!
//! 契约条目（冻结于 .dev-team/contract-matrix-deadlock.md）：
//! - **C1** `resolve_graph_run_submission_plan(run_id=None, session_id=非空)` 必须在有界时间内返回；
//! - **C2** `run_id=Some(不存在)` + 非空 session_id 同样必须有界返回。
//!
//! 对抗设计（红蓝对抗 - 确定性并发竞态）：
//! 1. 用 `std::sync::Barrier` 建立 N=8 线程的确定性同步起跑线，杜绝 `sleep` 概率等待；
//! 2. 主线程对每个线程的返回结果做 `mpsc::recv_timeout(5s)` 看门狗收敛，
//!    只要有任意一个调用未在有界时间内返回，立即断言失败（不会挂死测试进程）；
//! 3. 只有在收齐全部 N 个结果之后才 `join`，避免 `join` 本身成为无限阻塞点。
//!
//! 红相事实：修复前每个工作线程都会在 `.or_else -> resolve_graph_run_for_retrieval
//! -> load_active_graph_run_for_session` 上对同一把非重入 `std::sync::Mutex` 二次加锁，
//! 因此 8/8 线程全部无法返回，本用例必须**超时失败**。

use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Barrier, Once};
use std::thread;
use std::time::{Duration, Instant};

use pony_agent_core::agent::control_plane::{
    GraphRunSubmissionPlan, GraphRunSubmissionPlanQuery, HostControlPlane, HostControlPlaneBuilder,
};
use pony_agent_core::agent::graph::GraphRunStore;
use pony_agent_core::agent::runtime::AgentRuntime;

/// 并发线程数（对抗强度）。
const N: usize = 8;
/// 有界等待上限：与 .dev-team/nfr-baseline.json 的 `deadlock_watchdog_timeout_ms <= 5000` 对齐。
const WATCHDOG: Duration = Duration::from_secs(5);
/// 触发死锁的会话名（trim 后非空）。
const SESSION: &str = "red-phase-concurrency-session";

// ── 测试卫生：隔离生产数据目录 ──────────────────────────────────────────────────────────

/// 一次性重定向进程数据目录的初始化守卫。
static ISOLATE_PROCESS_DATA_DIR: Once = Once::new();

/// 集成测试（`tests/*.rs`）编译依赖 crate 时**不启用** `cfg(test)`，因此
/// `sqlite_session::default_sqlite_path()` 会走生产分支，指向用户真实
/// `~/.local/share/PonyAgent/sessions.db`（见 sqlite_session.rs:3370-3390）。
/// 本函数在**任何 HostControlPlane 构造之前**（由 Once 保证顺序）把
/// `dirs::data_local_dir()` / `dirs::home_dir()` 的解析基准重定向到本测试二进制
/// 独有的临时沙箱，确保集成测试绝不读写生产数据。
fn isolate_process_data_dir() {
    ISOLATE_PROCESS_DATA_DIR.call_once(|| {
        let unique = format!(
            "pony-agent-it-concurrency-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        );
        let sandbox = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&sandbox).expect("create isolated data dir");
        // Linux: data_local_dir() = $XDG_DATA_HOME | $HOME/.local/share
        std::env::set_var("XDG_DATA_HOME", &sandbox);
        // macOS / Unix 回退: home_dir() = $HOME
        std::env::set_var("HOME", &sandbox);
        // Windows: data_local_dir() = %LOCALAPPDATA%
        std::env::set_var("LOCALAPPDATA", &sandbox);
    });
}

fn build_control_plane() -> Arc<HostControlPlane> {
    isolate_process_data_dir();
    Arc::new(
        HostControlPlaneBuilder::new()
            .runtime(AgentRuntime::new())
            .graph_run_store(GraphRunStore::new())
            .build(),
    )
}

/// 确定性并发执行 `N` 次被测调用，并用单一 deadline 收敛全部返回值。
///
/// 返回每个线程的 `(index, plan)`；任一调用超时即 panic 断言失败。
fn run_concurrent(
    label: &str,
    run_id_factory: impl Fn(usize) -> Option<String> + Send + Sync + 'static,
) -> Vec<(usize, GraphRunSubmissionPlan)> {
    let control_plane = build_control_plane();
    let barrier = Arc::new(Barrier::new(N + 1));
    let run_id_factory = Arc::new(run_id_factory);
    let (sender, receiver) = mpsc::channel::<(usize, GraphRunSubmissionPlan)>();
    let started = Instant::now();

    let mut workers = Vec::with_capacity(N);
    for index in 0..N {
        let control_plane = Arc::clone(&control_plane);
        let barrier = Arc::clone(&barrier);
        let run_id_factory = Arc::clone(&run_id_factory);
        let sender = sender.clone();
        workers.push(thread::spawn(move || {
            // 起跑线：所有工作线程在此对齐后才同时发起调用（确定性并发，无 sleep）。
            barrier.wait();
            let plan = control_plane.resolve_graph_run_submission_plan(GraphRunSubmissionPlanQuery {
                session_id: Some(SESSION.to_string()),
                node_id: None,
                run_id: run_id_factory(index),
            });
            let _ = sender.send((index, plan));
        }));
    }
    drop(sender);

    // 主线程进入同一起跑线，令 N 个工作线程同时发起调用。
    // 工作线程在 barrier 之前的唯一语句是 barrier.wait()，不可能提前 panic。
    barrier.wait();

    let deadline = Instant::now() + WATCHDOG;
    let mut collected = Vec::with_capacity(N);
    while collected.len() < N {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .unwrap_or(Duration::ZERO);
        match receiver.recv_timeout(remaining) {
            Ok(item) => collected.push(item),
            Err(RecvTimeoutError::Timeout) => panic!(
                "[{label}] DEADLOCK (C1/C2): only {}/{} concurrent \
                 resolve_graph_run_submission_plan calls returned within {WATCHDOG:?}; the rest are \
                 still blocked on the non-reentrant `self.graph_runs` mutex",
                collected.len(),
                N
            ),
            Err(RecvTimeoutError::Disconnected) => panic!(
                "[{label}] result channel closed with only {}/{} plans received (worker panicked)",
                collected.len(),
                N
            ),
        }
    }

    // 收齐结果后才 join：此时每个工作线程都已发送结果，join 不可能无限阻塞。
    for worker in workers {
        worker.join().expect("concurrent resolve worker thread panicked");
    }

    // 冻结契约 §1：全部并发调用必须在有界窗口内返回（硬上限断言）。
    let elapsed = started.elapsed();
    assert!(
        elapsed < WATCHDOG,
        "[{label}] all {N} concurrent calls must return within {WATCHDOG:?}, took {elapsed:?}"
    );

    collected
}

fn assert_default_fallback(label: &str, index: usize, plan: &GraphRunSubmissionPlan) {
    assert_eq!(
        plan.command, "start_graph_run_stream",
        "[{label}] thread #{index}: 无 run 时必须回退 start_graph_run_stream"
    );
    assert_eq!(
        plan.run_id, None,
        "[{label}] thread #{index}: default 分支不得携带 run_id"
    );
    assert_eq!(
        plan.source, "default",
        "[{label}] thread #{index}: 无 run/checkpoint 时 source 必须是 default"
    );
}

#[test]
fn c1_submission_plan_bounded_eight_concurrent_null_run_id_calls() {
    // Arrange：8 线程全部使用前端进入会话页的入参形态（runId:null + 非空 session）。
    // Act
    let results = run_concurrent("C1 x8 concurrent run_id=None", |_index| None);

    // Assert
    assert_eq!(results.len(), N, "必须收齐全部 {N} 个并发结果");
    for (index, plan) in &results {
        assert_default_fallback("C1 x8", *index, plan);
    }
}

#[test]
fn c2_submission_plan_bounded_eight_concurrent_unknown_run_id_calls() {
    // Arrange：8 线程全部使用 run_id 指向不存在 run 的入参形态（必然走 `.or_else` 分支）。
    // Act
    let results = run_concurrent("C2 x8 concurrent run_id=Some(no-such-run)", |index| {
        Some(format!("no-such-run-{index}"))
    });

    // Assert
    assert_eq!(results.len(), N, "必须收齐全部 {N} 个并发结果");
    for (index, plan) in &results {
        assert_default_fallback("C2 x8", *index, plan);
    }
}
