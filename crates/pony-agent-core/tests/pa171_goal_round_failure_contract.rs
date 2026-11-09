//! PA-171 **C1 + C2 红相**验收：goal 续轮「连续失败止损」与「失败不烧轮次预算」。
//!
//! 契约真值源：`management/task-system/03_TASKS/PA-171-goal-round-failure-resilience.md`
//! §交付契约 C1 / C2（本文件由独立 Test Agent 冻结；Executor 只读，禁改）。
//!
//! 缺陷面（已取证）：`run_goal_rounds`（`bin/tui.rs`）只判 Esc / 8 轮 / 待作答请求，**不看上一轮
//! 成败**；`advance_goal_round`（`agent/goal.rs`）对失败轮照样 `rounds_started += 1`
//! ⇒ 一次上游 429 被"重放"成 16 轮（16 = 2×8），20 轮预算 74 秒烧光。
//!
//! ## 假设的可测面（`.dev-team/contract-matrix-pa171.md` 尚未生成，本文件自行锚定最小面）
//!
//! 契约矩阵落地前，本文件按任务卡 C1/C2 语义锚定如下**新增**公开面（`agent::goal`，
//! 属 `t-w1` 写域）。若矩阵给出不同签名，请 Lead 裁决后同步本文件的**调用点**，
//! 断言语义（连续 2 轮失败即停 / 失败轮不推进预算 / 预算耗尽状态行含原因）不得放宽：
//!
//! ```ignore
//! pub const MAX_CONSECUTIVE_GOAL_ROUND_FAILURES: u32 = 2;
//! #[derive(Debug, Clone, Copy, PartialEq, Eq)]
//! pub enum GoalRoundOutcome { Succeeded, Failed }
//! #[derive(Debug, Clone, PartialEq, Eq)]
//! pub enum GoalRoundStopReason {
//!     Cancelled,
//!     ConsecutiveFailures { failures: u32 },
//!     BudgetExhausted { rounds_started: u32, max_rounds: u32 },
//! }
//! pub fn goal_round_stop_reason(
//!     cancelled: bool,
//!     consecutive_failures: u32,
//!     rounds_started: u32,
//!     max_rounds: u32,
//! ) -> Option<GoalRoundStopReason>;              // Some ⇒ 停止新增 goal 轮（不静默 break）
//! pub fn advance_goal_round_for_outcome(
//!     goal_id: &str,
//!     outcome: GoalRoundOutcome,
//! ) -> Result<GoalData, String>;                 // 仅成功轮 rounds_started += 1
//! impl GoalRoundStopReason {
//!     pub fn visible_line(&self) -> String;      // TUI 可见状态行（含停止原因）
//! }
//! ```
//!
//! 红相预期：**本文件当前编译即红**（上述符号未落地）。这不是断言放松的理由——
//! 断言逐条写死在用例里，签名落地后必须直接绿，不允许改断言迁就实现。
//!
//! 铁律：每条用例真实断言；无 `#[ignore]`/skip/「只运行不报错」；不修改 `src/`。

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Mutex;

use pony_agent_core::agent::goal::{
    advance_goal_round_for_outcome, create_goal, goal_round_stop_reason, reset_goal_store_for_test,
    CreateGoalArgs, GoalRoundOutcome, GoalRoundStopReason, GOAL_STORE_PATH_ENV,
    MAX_CONSECUTIVE_GOAL_ROUND_FAILURES,
};

/// goal store 是**进程级全局单例**（`SESSION_GOALS` + sidecar 文件），
/// 与 `tests/pa109_goal_acceptance.rs` / `tests/wave_plan_goal_acceptance.rs` 同一处理：
/// 本文件用例串行化，避免并行覆盖同一 store。
static GOAL_STORE_TEST_LOCK: Mutex<()> = Mutex::new(());

fn goal_store_guard() -> std::sync::MutexGuard<'static, ()> {
    GOAL_STORE_TEST_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 临时 sidecar 路径 + env 覆盖的兜底清理（panic 路径同样生效，禁止污染后续运行）。
struct GoalStoreEnvGuard {
    previous: Option<OsString>,
    dir: PathBuf,
}

impl GoalStoreEnvGuard {
    /// Arrange：把 goal store 指到临时目录下的独立文件，并清空进程内缓存重新加载。
    fn isolated(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "pa171-goal-{tag}-{}-{nanos}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("PA-171：必须能创建 goal store 临时目录");
        let previous = std::env::var_os(GOAL_STORE_PATH_ENV);
        std::env::set_var(GOAL_STORE_PATH_ENV, dir.join("goal_store.json"));
        reset_goal_store_for_test();
        Self { previous, dir }
    }
}

impl Drop for GoalStoreEnvGuard {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(GOAL_STORE_PATH_ENV, value),
            None => std::env::remove_var(GOAL_STORE_PATH_ENV),
        }
        reset_goal_store_for_test();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn create_probe_goal(session: &str, max_rounds: u32) -> String {
    create_goal(CreateGoalArgs {
        session_id: Some(session.to_string()),
        objective: "PA-171 续轮失败止损探针".to_string(),
        max_goal_rounds: Some(max_rounds),
    })
    .expect("PA-171：Arrange 阶段 create_goal 必须成功")
    .goal
    .id
}

// ── C1：连续失败止损 ────────────────────────────────────────────────────────────────

/// C1 常量锁：任务卡写死「连续 **2** 轮 failed 即停止」。
#[test]
fn c1_consecutive_failure_limit_is_two_rounds() {
    assert_eq!(
        MAX_CONSECUTIVE_GOAL_ROUND_FAILURES, 2,
        "C1：连续失败止损阈值必须是 2 轮（任务卡契约），实际 {MAX_CONSECUTIVE_GOAL_ROUND_FAILURES}"
    );
}

/// C1 真值表：只有「连续失败 ≥ 2」才因失败止损；失败 0/1 轮必须继续。
#[test]
fn c1_goal_round_stop_truth_table() {
    const MAX: u32 = 20;

    // 1) 未取消、零失败、预算充足 ⇒ 继续（None）。
    assert_eq!(
        goal_round_stop_reason(false, 0, 0, MAX),
        None,
        "C1：未取消、0 轮失败时必须继续续轮"
    );

    // 2) 仅 1 轮失败 ⇒ 仍继续（止损阈值是 2，不得提前停）。
    assert_eq!(
        goal_round_stop_reason(false, 1, 3, MAX),
        None,
        "C1：只失败 1 轮时不得停止续轮（阈值 2）"
    );

    // 3) 连续 2 轮失败 ⇒ 必须停，且原因携带失败计数。
    assert!(
        matches!(
            goal_round_stop_reason(false, 2, 4, MAX),
            Some(GoalRoundStopReason::ConsecutiveFailures { failures: 2 })
        ),
        "C1：连续 2 轮 failed 必须停止续轮并给出失败原因，实际 {:?}",
        goal_round_stop_reason(false, 2, 4, MAX)
    );

    // 4) 连续 N（>2）轮失败 ⇒ 同样停，计数如实（不得截断成 2）。
    assert!(
        matches!(
            goal_round_stop_reason(false, 7, 4, MAX),
            Some(GoalRoundStopReason::ConsecutiveFailures { failures: 7 })
        ),
        "C1：连续 7 轮失败必须停止且失败计数如实，实际 {:?}",
        goal_round_stop_reason(false, 7, 4, MAX)
    );

    // 5) Esc 取消优先于一切计数。
    assert!(
        matches!(
            goal_round_stop_reason(true, 0, 0, MAX),
            Some(GoalRoundStopReason::Cancelled)
        ),
        "C1：未失败但用户已按 Esc 时必须停止"
    );
    assert!(
        matches!(
            goal_round_stop_reason(true, 3, 9, MAX),
            Some(GoalRoundStopReason::Cancelled)
        ),
        "C1：取消是最高优先级停止原因（失败计数不得掩盖 Cancelled）"
    );

    // 6) 预算用满 ⇒ 因预算停（原因携带 rounds_started / max_rounds）。
    assert!(
        matches!(
            goal_round_stop_reason(false, 0, 20, 20),
            Some(GoalRoundStopReason::BudgetExhausted {
                rounds_started: 20,
                max_rounds: 20
            })
        ),
        "C2：rounds_started 用满 max_rounds 时必须因预算停止，实际 {:?}",
        goal_round_stop_reason(false, 0, 20, 20)
    );
    assert!(
        matches!(
            goal_round_stop_reason(false, 0, 21, 20),
            Some(GoalRoundStopReason::BudgetExhausted { .. })
        ),
        "C2：越界 rounds_started 同样必须停止（不得继续起新轮）"
    );

    // 7) 两条停止条件同时成立 ⇒ 必须停（不锁具体变体，避免过度约束实现优先级）。
    assert!(
        goal_round_stop_reason(false, 2, 20, 20).is_some(),
        "C2：失败止损与预算耗尽同时命中时仍必须停止"
    );
}

/// C1 循环模拟（与 `tests/tui_batch2_regression.rs` N8(a) 同一「纯函数组合」式）：
/// 喂入 1 成功 + 3 失败序列 ⇒ 只新增 **1** 轮成功轮，第 3 条失败后立即止损。
#[test]
fn c1_loop_simulation_one_success_then_three_failures_starts_only_one_round() {
    // Arrange
    let _serial = goal_store_guard();
    let _env = GoalStoreEnvGuard::isolated("c1-loop");
    let goal_id = create_probe_goal("pa171-c1-loop", 20);
    let outcomes = [
        GoalRoundOutcome::Succeeded,
        GoalRoundOutcome::Failed,
        GoalRoundOutcome::Failed,
        GoalRoundOutcome::Failed,
    ];

    // Act：模拟 `run_goal_rounds` 的轮次边界（先判停止，再推进本轮）。
    let mut executed = 0usize;
    let mut consecutive_failures = 0u32;
    let mut rounds_started = 0u32;
    let mut stop_reason = None;
    for outcome in outcomes {
        if let Some(reason) =
            goal_round_stop_reason(false, consecutive_failures, rounds_started, 20)
        {
            stop_reason = Some(reason);
            break;
        }
        let goal = advance_goal_round_for_outcome(&goal_id, outcome)
            .expect("PA-171：轮次结果推进必须成功");
        rounds_started = goal.rounds_started;
        executed += 1;
        match outcome {
            GoalRoundOutcome::Failed => consecutive_failures += 1,
            GoalRoundOutcome::Succeeded => consecutive_failures = 0,
        }
    }

    // Assert
    assert_eq!(
        rounds_started, 1,
        "C1/C2：1 成功 + 3 失败序列只能新增 1 轮预算，实际 rounds_started={rounds_started}"
    );
    assert_eq!(
        executed, 3,
        "C1：连续 2 轮失败后必须止损——第 3 条失败执行完即停，第 4 条不得再开新轮，实际执行 {executed} 轮"
    );
    assert!(
        matches!(
            stop_reason,
            Some(GoalRoundStopReason::ConsecutiveFailures { failures: 2 })
        ),
        "C1：止损原因必须是「连续 2 轮失败」（不得静默 break），实际 {stop_reason:?}"
    );
}

// ── C2：失败不烧轮次预算 ────────────────────────────────────────────────────────────

/// C2：失败轮不得推进 `rounds_started`（失败轮照样 +1 是 16 轮烧光 20 轮预算的根因）。
#[test]
fn c2_failed_round_does_not_consume_round_budget() {
    // Arrange
    let _serial = goal_store_guard();
    let _env = GoalStoreEnvGuard::isolated("c2-failed");
    let goal_id = create_probe_goal("pa171-c2-failed", 20);

    // Act：连续 3 轮失败。
    for index in 1..=3u32 {
        let goal = advance_goal_round_for_outcome(&goal_id, GoalRoundOutcome::Failed)
            .expect("PA-171：失败轮推进调用本身不得报错");
        // Assert：失败轮不烧预算。
        assert_eq!(
            goal.rounds_started, 0,
            "C2：第 {index} 个失败轮不得推进 rounds_started（失败不烧预算），实际 {}",
            goal.rounds_started
        );
    }
}

/// C2 反向钉子：成功轮**必须**推进预算（防止"永不自增"式假修复）。
#[test]
fn c2_successful_round_still_consumes_round_budget() {
    // Arrange
    let _serial = goal_store_guard();
    let _env = GoalStoreEnvGuard::isolated("c2-success");
    let goal_id = create_probe_goal("pa171-c2-success", 20);

    // Act + Assert：两次成功 ⇒ 恰好 2 轮，逐轮递增。
    let first = advance_goal_round_for_outcome(&goal_id, GoalRoundOutcome::Succeeded)
        .expect("PA-171：成功轮推进必须成功");
    assert_eq!(first.rounds_started, 1, "C2：第 1 个成功轮必须消耗 1 轮预算");
    let second = advance_goal_round_for_outcome(&goal_id, GoalRoundOutcome::Succeeded)
        .expect("PA-171：成功轮推进必须成功");
    assert_eq!(
        second.rounds_started, 2,
        "C2：第 2 个成功轮必须继续消耗预算（成功轮计入 rounds_started）"
    );
}

/// C2：预算耗尽时 TUI 状态行必须写「已停止（原因）」，不得仍显示「进行中」。
#[test]
fn c2_budget_exhausted_visible_line_reports_stopped_with_reason() {
    // Arrange：预算 20 轮已用满。
    let reason = goal_round_stop_reason(false, 0, 20, 20)
        .expect("C2：预算用满必须返回停止原因");
    assert!(
        matches!(reason, GoalRoundStopReason::BudgetExhausted { .. }),
        "C2：预算用满的停止原因应为 BudgetExhausted，实际 {reason:?}"
    );

    // Act
    let line = reason.visible_line();

    // Assert
    assert!(
        line.contains("已停止"),
        "C2：预算耗尽的状态行必须明示「已停止」，实际 {line:?}"
    );
    assert!(
        !line.contains("进行中"),
        "C2：预算耗尽后不得仍显示「进行中」（历史缺陷：横幅只显示 \"进行中 [20/20]\"），实际 {line:?}"
    );
    assert!(
        line.contains("预算") || line.contains("轮次") || line.contains("上限"),
        "C2：状态行必须包含停止原因（预算/轮次/上限），实际 {line:?}"
    );
    assert!(
        line.contains("20"),
        "C2：状态行应给出可核对的轮次用量（20），实际 {line:?}"
    );
}

/// C1：失败止损的可见状态行同样必须含原因（不得静默 break）。
#[test]
fn c1_consecutive_failure_stop_line_mentions_failure_reason() {
    // Arrange
    let reason = goal_round_stop_reason(false, 2, 5, 20)
        .expect("C1：连续 2 轮失败必须返回停止原因");

    // Act
    let line = reason.visible_line();

    // Assert
    assert!(
        line.contains("已停止"),
        "C1：失败止损必须在状态行明示「已停止」，实际 {line:?}"
    );
    assert!(
        line.contains("失败"),
        "C1：状态行必须写明失败原因（失败/连续失败），实际 {line:?}"
    );
    assert!(
        !line.contains("进行中"),
        "C1：止损后不得仍显示「进行中」，实际 {line:?}"
    );
}
