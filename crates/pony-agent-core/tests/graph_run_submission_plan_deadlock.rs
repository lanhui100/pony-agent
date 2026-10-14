//! L2-T 红相验收测试：`resolve_graph_run_submission_plan` 的 `graph_runs` 锁重入自死锁。
//!
//! 覆盖契约条目（冻结于 .dev-team/contract-matrix-deadlock.md）：
//! - **C1** `resolve_graph_run_submission_plan(run_id=None, session_id=非空)` 必须在有界时间内返回；
//! - **C2** `run_id=Some(不存在)` + 非空 session_id 同样必须有界返回；
//! - **C3** 存在 unresolved ask waits 时自愈为 `start_graph_run_stream` + `run_id=None`；
//! - **C4** 终态 checkpoint/run → `start_graph_run_stream`；非终态 ready/waiting_user →
//!   `continue_graph_run_stream`；paused/resumable → `resume_graph_run_stream`。
//!
//! 红相事实（修复前）：`history_commands.rs:428-447` 在持有 `self.graph_runs`
//! （非重入 `std::sync::Mutex`）时经 `.or_else` 调用 `resolve_graph_run_for_retrieval`
//! → `graph_projection.rs:396 load_active_graph_run_for_session`
//! → `graph_projection.rs:383 self.graph_runs.lock()` → 同线程自死锁。
//!
//! 因此 C1/C2 用例在本文件下必须**有界超时失败**，而不是挂死测试进程：
//! 每一个可能走 `.or_else` 分支的调用都被 `std::thread` + `mpsc::recv_timeout(5s)`
//! 看门狗包裹，超时即 `panic!` 断言失败。

use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Once};
use std::thread;
use std::time::{Duration, Instant};

use pony_agent_core::agent::control_plane::{
    GraphRunSubmissionPlan, GraphRunSubmissionPlanQuery, HostControlPlane, HostControlPlaneBuilder,
};
use pony_agent_core::agent::execution_control::ExecutionControlRegistry;
use pony_agent_core::agent::graph::{
    GraphAskWaitBinding, GraphRun, GraphRunPhase, GraphRunStore, GraphRunner,
};
use pony_agent_core::agent::runtime::AgentRuntime;

/// 有界等待上限：与 .dev-team/nfr-baseline.json 的 `deadlock_watchdog_timeout_ms <= 5000` 对齐。
const WATCHDOG: Duration = Duration::from_secs(5);

/// 触发死锁的会话名（trim 后非空）。
const SESSION: &str = "red-phase-deadlock-session";

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
            "pony-agent-it-deadlock-{}-{}",
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

// ── Arrange 辅助 ────────────────────────────────────────────────────────────────────────

fn build_control_plane() -> Arc<HostControlPlane> {
    isolate_process_data_dir();
    Arc::new(
        HostControlPlaneBuilder::new()
            .runtime(AgentRuntime::new())
            .graph_run_store(GraphRunStore::new())
            .build(),
    )
}

fn build_control_plane_with(
    store: GraphRunStore,
    execution_control: ExecutionControlRegistry,
) -> Arc<HostControlPlane> {
    isolate_process_data_dir();
    Arc::new(
        HostControlPlaneBuilder::new()
            .runtime(AgentRuntime::new())
            .graph_run_store(store)
            .execution_control(execution_control)
            .build(),
    )
}

fn make_run(run_id: &str, session_id: &str, phase: GraphRunPhase) -> GraphRun {
    GraphRun {
        id: run_id.to_string(),
        goal: format!("red-phase goal for {run_id}"),
        session_id: Some(session_id.to_string()),
        phase,
        steps: Vec::new(),
        active_turn_id: None,
        last_completed_turn_id: None,
        stop_reason: None,
        last_handoff: None,
        resume_count: 0,
        last_decision: None,
        control_boundary_evidence: Vec::new(),
        created_at_ms: 1_700_000_000_000,
        updated_at_ms: 1_700_000_000_000,
    }
}

fn store_with_run(run: GraphRun) -> GraphRunStore {
    let mut store = GraphRunStore::new();
    let _lifecycle = GraphRunner::new().start_run(&mut store, run);
    store
}

/// 把 `turn_id` 注册成 `checkpoint_kind = "recovery"` 的恢复检查点，并让注册表按
/// `phase`/`status` 投影出 `submission_command`（`update` 内部会调用
/// `refresh_execution_checkpoint_projection`）。
fn register_recovery_checkpoint(
    execution_control: &ExecutionControlRegistry,
    turn_id: &str,
    run_id: &str,
    phase: &str,
    status: &str,
) {
    execution_control.register_turn(turn_id, Some(SESSION), Some(run_id));
    execution_control.update(turn_id, |checkpoint| {
        checkpoint.checkpoint_kind = "recovery".to_string();
        checkpoint.recovery_mode = "persisted_effect".to_string();
        checkpoint.run_id = Some(run_id.to_string());
        checkpoint.phase = phase.to_string();
        checkpoint.status = status.to_string();
        checkpoint.resumable = true;
        checkpoint.replayable = true;
    });
}

// ── Act 看门狗 ──────────────────────────────────────────────────────────────────────────

/// 在工作线程里执行被测调用，并在有界时间内收取结果。
///
/// 超时即断言失败（绝不让测试进程永久挂起）：泄漏的工作线程只阻塞在它自己那份
/// `HostControlPlane` 的互斥量上，不影响同进程内其它用例（每个用例独占一份控制面）。
fn resolve_with_watchdog(
    label: &str,
    control_plane: Arc<HostControlPlane>,
    query: GraphRunSubmissionPlanQuery,
) -> GraphRunSubmissionPlan {
    let (sender, receiver) = mpsc::channel::<GraphRunSubmissionPlan>();
    let started = Instant::now();
    let worker = thread::spawn(move || {
        let plan = control_plane.resolve_graph_run_submission_plan(query);
        let _ = sender.send(plan);
    });

    match receiver.recv_timeout(WATCHDOG) {
        Ok(plan) => {
            worker
                .join()
                .expect("resolve worker thread panicked after producing a plan");
            let elapsed = started.elapsed();
            // 冻结契约 §1：有界返回硬上限断言（deadlock_watchdog_timeout_ms = 5000）。
            assert!(
                elapsed < WATCHDOG,
                "[{label}] elapsed {elapsed:?} must be < {WATCHDOG:?}"
            );
            plan
        }
        Err(RecvTimeoutError::Timeout) => panic!(
            "[{label}] DEADLOCK (C1/C2): resolve_graph_run_submission_plan did not return within \
             {WATCHDOG:?}; the worker thread is still blocked on the non-reentrant `self.graph_runs` \
             mutex (self-deadlock via resolve_graph_run_for_retrieval -> \
             load_active_graph_run_for_session)"
        ),
        Err(RecvTimeoutError::Disconnected) => panic!(
            "[{label}] resolve worker thread terminated without sending a plan (panic before send)"
        ),
    }
}

// ── C1 / C2：有界返回 ──────────────────────────────────────────────────────────────────

#[test]
fn c1_submission_plan_bounded_null_run_id_with_nonempty_session() {
    // Arrange：前端进入会话页的典型入参（runId:null + 非空 session）。
    let control_plane = build_control_plane();

    // Act
    let plan = resolve_with_watchdog(
        "C1 run_id=None + session=Some(non-empty)",
        control_plane,
        GraphRunSubmissionPlanQuery {
            session_id: Some(SESSION.to_string()),
            node_id: None,
            run_id: None,
        },
    );

    // Assert：无 run / 无 checkpoint 时必须落到 default 分支。
    assert_eq!(
        plan.command, "start_graph_run_stream",
        "C1: 无 run 时必须回退 start_graph_run_stream"
    );
    assert_eq!(plan.run_id, None, "C1: default 分支不得携带 run_id");
    assert_eq!(plan.source, "default", "C1: 无 run/checkpoint 时 source 必须是 default");
    assert_eq!(plan.hook_point, "submission_plan_resolved");
}

#[test]
fn c2_submission_plan_bounded_unknown_run_id_with_nonempty_session() {
    // Arrange：run_id 指向不存在的 run → 必然走 `.or_else` 二次加锁分支。
    let control_plane = build_control_plane();

    // Act
    let plan = resolve_with_watchdog(
        "C2 run_id=Some(no-such-run) + session=Some(non-empty)",
        control_plane,
        GraphRunSubmissionPlanQuery {
            session_id: Some(SESSION.to_string()),
            node_id: None,
            run_id: Some("no-such-run".to_string()),
        },
    );

    // Assert
    assert_eq!(
        plan.command, "start_graph_run_stream",
        "C2: run_id 未命中且无 active run 时必须回退 start_graph_run_stream"
    );
    assert_eq!(plan.run_id, None, "C2: 未命中 run 时不得携带 run_id");
    assert_eq!(plan.source, "default", "C2: 未命中 run/checkpoint 时 source 必须是 default");
}

#[test]
fn c1_submission_plan_bounded_regression_null_run_id_and_null_session() {
    // Arrange：对照组——session_id 为 None 时 load_active_graph_run_for_session 会在加锁前
    // early return，因此修复前也应通过（证明看门狗本身不会误报死锁）。
    let control_plane = build_control_plane();

    // Act
    let plan = resolve_with_watchdog(
        "C1 control run_id=None + session=None",
        control_plane,
        GraphRunSubmissionPlanQuery {
            session_id: None,
            node_id: None,
            run_id: None,
        },
    );

    // Assert
    assert_eq!(plan.command, "start_graph_run_stream");
    assert_eq!(plan.run_id, None);
    assert_eq!(plan.source, "default");
}

#[test]
fn c1_submission_plan_bounded_null_run_id_with_active_non_terminal_run() {
    // Arrange：契约矩阵 C1 第 2 行——session 存在活跃非终态 run 且无 checkpoint 时，
    // fallback 语义必须保留：有界返回且继续该活跃 run（这正是缺陷本体所在的 `.or_else` 分支）。
    const RUN: &str = "run-red-c1-active-ready";
    let control_plane =
        build_control_plane_with(store_with_run(make_run(RUN, SESSION, GraphRunPhase::Ready)), ExecutionControlRegistry::new());

    // Act
    let plan = resolve_with_watchdog(
        "C1 run_id=None + active non-terminal run",
        control_plane,
        GraphRunSubmissionPlanQuery {
            session_id: Some(SESSION.to_string()),
            node_id: None,
            run_id: None,
        },
    );

    // Assert
    assert_eq!(
        plan.command, "continue_graph_run_stream",
        "C1: 存在活跃非终态 run 时必须继续该 run，不得回退 start"
    );
    assert_eq!(
        plan.run_id.as_deref(),
        Some(RUN),
        "C1: fallback 必须解析出活跃 run 的 id（锁作用域收敛后语义不得丢失）"
    );
    assert_eq!(
        plan.source, "graph_run_reconciled",
        "C1: 活跃 run + 投影恢复 checkpoint 的协调来源必须是 graph_run_reconciled"
    );
}

#[test]
fn c2_submission_plan_bounded_unknown_run_id_with_null_session() {
    // Arrange：契约矩阵 C2 第 2 行——run_id 不存在且 session_id=None，不触发 fallback。
    let control_plane = build_control_plane();

    // Act
    let plan = resolve_with_watchdog(
        "C2 run_id=Some(no-such-run) + session=None",
        control_plane,
        GraphRunSubmissionPlanQuery {
            session_id: None,
            node_id: None,
            run_id: Some("no-such-run".to_string()),
        },
    );

    // Assert
    assert_eq!(plan.command, "start_graph_run_stream");
    assert_eq!(plan.run_id, None);
    assert_eq!(plan.source, "default");
}

// ── C3：unresolved ask wait 自愈语义 ───────────────────────────────────────────────────

#[test]
fn c3_unresolved_ask_wait_self_heals_to_start_graph_run_stream() {
    // Arrange：构造一个带 unresolved ask wait 的 run（公开 API：GraphRunner::bind_ask_wait）。
    const RUN: &str = "run-red-c3-ask-unresolved";
    let runner = GraphRunner::new();
    let mut store = GraphRunStore::new();
    let _lifecycle = runner.start_run(&mut store, make_run(RUN, SESSION, GraphRunPhase::Ready));

    let bound = runner
        .bind_ask_wait(
            &mut store,
            RUN,
            GraphAskWaitBinding {
                request_id: "req-red-c3-ask-1".to_string(),
                expected_version: 1,
                run_id: RUN.to_string(),
                turn_id: "turn-red-c3-ask-1".to_string(),
                session_id: Some(SESSION.to_string()),
                call_id: "call-red-c3-ask-1".to_string(),
                tool_name: "ask_user_question".to_string(),
                assistant_transcript: serde_json::json!({}),
                created_at_ms: 1_700_000_000_100,
            },
        )
        .expect("bind_ask_wait must accept a non-terminal run");
    assert_eq!(
        bound.phase,
        GraphRunPhase::WaitingUser,
        "绑定 ask wait 必须把 run 挂起到 waiting_user"
    );
    assert_eq!(
        runner.list_ask_waits(&store, RUN).len(),
        1,
        "ask wait 必须可被观测到（前置条件校验，避免假测试）"
    );

    let control_plane = build_control_plane_with(store, ExecutionControlRegistry::new());

    // Act
    let plan = resolve_with_watchdog(
        "C3 unresolved ask wait",
        control_plane,
        GraphRunSubmissionPlanQuery {
            session_id: Some(SESSION.to_string()),
            node_id: None,
            run_id: Some(RUN.to_string()),
        },
    );

    // Assert：unresolved ask wait 存在时不得继续流式续跑，必须自愈为全新 start。
    assert_eq!(
        plan.command, "start_graph_run_stream",
        "C3: 存在 unresolved ask waits 时必须自愈为 start_graph_run_stream"
    );
    assert_eq!(plan.run_id, None, "C3: 自愈结果不得复用被挂起的 run_id");
}

// ── C4：终态 / 非终态 checkpoint 与 run 的命令映射 ─────────────────────────────────────

#[test]
fn c4_ready_recovery_checkpoint_maps_to_continue_graph_run_stream() {
    // Arrange
    const RUN: &str = "run-red-c4-ready";
    let execution_control = ExecutionControlRegistry::new();
    register_recovery_checkpoint(&execution_control, "turn-red-c4-ready", RUN, "ready", "completed");
    let checkpoint = execution_control
        .load_checkpoint(None, Some(SESSION))
        .expect("recovery checkpoint must be registered");
    assert_eq!(
        checkpoint.submission_command.as_deref(),
        Some("continue_graph_run_stream"),
        "前置条件：ready 恢复检查点必须投影出 continue_graph_run_stream"
    );
    let control_plane = build_control_plane_with(
        store_with_run(make_run(RUN, SESSION, GraphRunPhase::Ready)),
        execution_control,
    );

    // Act
    let plan = resolve_with_watchdog(
        "C4 ready",
        control_plane,
        GraphRunSubmissionPlanQuery {
            session_id: Some(SESSION.to_string()),
            node_id: None,
            run_id: Some(RUN.to_string()),
        },
    );

    // Assert
    assert_eq!(plan.command, "continue_graph_run_stream");
    assert_eq!(plan.run_id.as_deref(), Some(RUN));
    assert_eq!(plan.source, "graph_run_reconciled");
}

#[test]
fn c4_waiting_user_recovery_checkpoint_maps_to_continue_graph_run_stream() {
    // Arrange
    const RUN: &str = "run-red-c4-waiting-user";
    let execution_control = ExecutionControlRegistry::new();
    register_recovery_checkpoint(
        &execution_control,
        "turn-red-c4-waiting-user",
        RUN,
        "waiting_user",
        "completed",
    );
    let checkpoint = execution_control
        .load_checkpoint(None, Some(SESSION))
        .expect("recovery checkpoint must be registered");
    assert_eq!(
        checkpoint.submission_command.as_deref(),
        Some("continue_graph_run_stream"),
        "前置条件：waiting_user 恢复检查点必须投影出 continue_graph_run_stream"
    );
    let control_plane = build_control_plane_with(
        store_with_run(make_run(RUN, SESSION, GraphRunPhase::WaitingUser)),
        execution_control,
    );

    // Act
    let plan = resolve_with_watchdog(
        "C4 waiting_user",
        control_plane,
        GraphRunSubmissionPlanQuery {
            session_id: Some(SESSION.to_string()),
            node_id: None,
            run_id: Some(RUN.to_string()),
        },
    );

    // Assert
    assert_eq!(plan.command, "continue_graph_run_stream");
    assert_eq!(plan.run_id.as_deref(), Some(RUN));
}

#[test]
fn c4_submission_plan_paused_resumable_maps_to_resume_graph_run_stream() {
    // Arrange
    const RUN: &str = "run-red-c4-paused";
    let execution_control = ExecutionControlRegistry::new();
    register_recovery_checkpoint(&execution_control, "turn-red-c4-paused", RUN, "paused", "ready");
    let checkpoint = execution_control
        .load_checkpoint(None, Some(SESSION))
        .expect("recovery checkpoint must be registered");
    assert_eq!(
        checkpoint.submission_command.as_deref(),
        Some("resume_graph_run_stream"),
        "前置条件：paused 恢复检查点必须投影出 resume_graph_run_stream"
    );
    let control_plane = build_control_plane_with(
        store_with_run(make_run(RUN, SESSION, GraphRunPhase::Paused)),
        execution_control,
    );

    // Act
    let plan = resolve_with_watchdog(
        "C4 paused",
        control_plane,
        GraphRunSubmissionPlanQuery {
            session_id: Some(SESSION.to_string()),
            node_id: None,
            run_id: Some(RUN.to_string()),
        },
    );

    // Assert
    assert_eq!(plan.command, "resume_graph_run_stream");
    assert_eq!(plan.run_id.as_deref(), Some(RUN));
}

#[test]
fn c4_submission_plan_paused_with_active_turn_downgrades_to_start_graph_run_stream() {
    // Arrange：契约矩阵 C4/G7 降级分支——Paused 且 active_turn_id != None 时既不可 resume
    // 也不可 continue，协调器必须降级为 start_graph_run_stream + run_id=None。
    const RUN: &str = "run-red-c4-paused-active";
    let execution_control = ExecutionControlRegistry::new();
    register_recovery_checkpoint(
        &execution_control,
        "turn-red-c4-paused-active",
        RUN,
        "paused",
        "ready",
    );
    let mut run = make_run(RUN, SESSION, GraphRunPhase::Paused);
    run.active_turn_id = Some("turn-red-c4-paused-active-inflight".to_string());
    let control_plane = build_control_plane_with(store_with_run(run), execution_control);

    // Act
    let plan = resolve_with_watchdog(
        "C4 paused + active turn",
        control_plane,
        GraphRunSubmissionPlanQuery {
            session_id: Some(SESSION.to_string()),
            node_id: None,
            run_id: Some(RUN.to_string()),
        },
    );

    // Assert
    assert_eq!(
        plan.command, "start_graph_run_stream",
        "C4: Paused 且存在活跃 turn 时必须降级为 start_graph_run_stream"
    );
    assert_eq!(plan.run_id, None, "C4: 降级分支不得复用不可 resume 的 run_id");
}

#[test]
fn c4_completed_run_maps_to_start_graph_run_stream() {
    // Arrange：终态 checkpoint 的投影命令是 continue，必须由 run 终态语义收敛为 start。
    const RUN: &str = "run-red-c4-completed";
    let execution_control = ExecutionControlRegistry::new();
    register_recovery_checkpoint(
        &execution_control,
        "turn-red-c4-completed",
        RUN,
        "completed",
        "completed",
    );
    let checkpoint = execution_control
        .load_checkpoint(None, Some(SESSION))
        .expect("recovery checkpoint must be registered");
    assert_eq!(
        checkpoint.submission_command.as_deref(),
        Some("continue_graph_run_stream"),
        "前置条件：completed 检查点投影为 continue，需由终态 run 再收敛"
    );
    let control_plane = build_control_plane_with(
        store_with_run(make_run(RUN, SESSION, GraphRunPhase::Completed)),
        execution_control,
    );

    // Act
    let plan = resolve_with_watchdog(
        "C4 completed",
        control_plane,
        GraphRunSubmissionPlanQuery {
            session_id: Some(SESSION.to_string()),
            node_id: None,
            run_id: Some(RUN.to_string()),
        },
    );

    // Assert
    assert_eq!(
        plan.command, "start_graph_run_stream",
        "C4: 终态 run 必须回退为 start_graph_run_stream"
    );
    assert_eq!(plan.run_id, None, "C4: 终态 run 不得被复用为继续执行的 run_id");
}

#[test]
fn c4_failed_run_maps_to_start_graph_run_stream() {
    // Arrange：failed/cancelled 检查点不投影 submission_command，走 run 终态分支。
    const RUN: &str = "run-red-c4-failed";
    let execution_control = ExecutionControlRegistry::new();
    register_recovery_checkpoint(&execution_control, "turn-red-c4-failed", RUN, "failed", "failed");
    let checkpoint = execution_control
        .load_checkpoint(None, Some(SESSION))
        .expect("recovery checkpoint must be registered");
    assert_eq!(
        checkpoint.submission_command, None,
        "前置条件：failed 检查点不投影 submission_command"
    );
    let control_plane = build_control_plane_with(
        store_with_run(make_run(RUN, SESSION, GraphRunPhase::Failed)),
        execution_control,
    );

    // Act
    let plan = resolve_with_watchdog(
        "C4 failed",
        control_plane,
        GraphRunSubmissionPlanQuery {
            session_id: Some(SESSION.to_string()),
            node_id: None,
            run_id: Some(RUN.to_string()),
        },
    );

    // Assert
    assert_eq!(plan.command, "start_graph_run_stream");
    assert_eq!(plan.run_id, None);
}

#[test]
fn c4_cancelled_run_maps_to_start_graph_run_stream() {
    // Arrange
    const RUN: &str = "run-red-c4-cancelled";
    let execution_control = ExecutionControlRegistry::new();
    register_recovery_checkpoint(
        &execution_control,
        "turn-red-c4-cancelled",
        RUN,
        "cancelled",
        "cancelled",
    );
    let checkpoint = execution_control
        .load_checkpoint(None, Some(SESSION))
        .expect("recovery checkpoint must be registered");
    assert_eq!(
        checkpoint.submission_command, None,
        "前置条件：cancelled 检查点不投影 submission_command"
    );
    let control_plane = build_control_plane_with(
        store_with_run(make_run(RUN, SESSION, GraphRunPhase::Cancelled)),
        execution_control,
    );

    // Act
    let plan = resolve_with_watchdog(
        "C4 cancelled",
        control_plane,
        GraphRunSubmissionPlanQuery {
            session_id: Some(SESSION.to_string()),
            node_id: None,
            run_id: Some(RUN.to_string()),
        },
    );

    // Assert
    assert_eq!(plan.command, "start_graph_run_stream");
    assert_eq!(plan.run_id, None);
}
