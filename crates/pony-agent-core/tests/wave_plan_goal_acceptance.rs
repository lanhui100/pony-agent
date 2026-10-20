//! Wave-1 冻结契约验收（Test Agent 冻结，实施者只读不可改）
//!
//! 契约唯一来源：`.dev-team/contract-wave-1.md`（C1/C2/C5 + 验收 1/2/3）与
//! `docs/architecture/plan-goal-contract.md`（§1/§2/§5 语义落点）。
//!
//! 红相原则：本文件在空桩阶段（`unimplemented!("wave-1")`）必须**编译通过**且**断言失败**；
//! 严禁 `#[ignore]`、严禁吞异常、严禁条件跳过。
//!
//! 进程级状态说明：`goal` 的 store 是进程级全局（`SESSION_GOALS` + sidecar 路径 env），
//! 因此所有触及目标 store / env 的用例统一经 `wave1_guard()` 串行，且各用例使用互不相同的
//! session id，避免跨用例污染。

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use pony_agent_core::agent::goal::{
    active_goal_for_session, create_goal, get_goal, goal_store_path, reset_goal_store_for_test,
    update_goal, CreateGoalArgs, GetGoalArgs, GoalActivation, UpdateGoalAction, UpdateGoalArgs,
};
use pony_agent_core::agent::graph::{
    GraphDecision, GraphDecisionKind, GraphEngine, GraphRun, GraphTurnHandoff,
};
use pony_agent_core::agent::planner::DefaultGraphPlanner;

/// env `PONY_AGENT_GOAL_STORE_PATH`（契约 §8 / C2）。
const GOAL_STORE_PATH_ENV: &str = "PONY_AGENT_GOAL_STORE_PATH";

/// 串行化所有会触碰进程级 goal store / env 的用例。
static WAVE1_EXECUTION_LOCK: Mutex<()> = Mutex::new(());

fn wave1_guard() -> std::sync::MutexGuard<'static, ()> {
    WAVE1_EXECUTION_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 构造全字段显式填充的 `GraphTurnHandoff`（turn 已正常收口：phase=ready，无 checkpoint 残留）。
fn completed_turn_handoff(
    engine: &GraphEngine,
    session_id: &str,
    assistant_message: &str,
) -> GraphTurnHandoff {
    GraphTurnHandoff {
        contract_version: engine.contract_version().to_string(),
        turn_id: Some("turn-wave1".to_string()),
        session_id: Some(session_id.to_string()),
        turn_phase: "ready".to_string(),
        checkpoint_status: None,
        checkpoint_phase: None,
        user_message: "继续把报告写完".to_string(),
        assistant_message: assistant_message.to_string(),
        session_summary: "已完成第三章，正在推进第四章".to_string(),
        conversation_id: session_id.to_string(),
        session_turn_count: 3,
        run_id: Some("run-wave1".to_string()),
        run_phase: Some("ready".to_string()),
        active_task_focus: None,
        acceptance_focus: None,
        closeout_focus: None,
        last_referenced_file: None,
        recent_attachment_asset_count: 0,
        long_term_memory_status: "idle".to_string(),
        long_term_memory_entry_count: 0,
        trace_step_count: 2,
        tool_activity_count: 1,
        provider_name: "test-provider".to_string(),
        provider_model: "test-model".to_string(),
    }
}

/// 断言决策为 `Continue`（armed 目标驱动续行）。
fn assert_continue(decision: &GraphDecision, case: &str) {
    assert_eq!(
        decision.kind,
        GraphDecisionKind::Continue,
        "契约 C1 违约（{case}）：armed 的 active 目标必须驱动 graph 判定 Continue，\
         实际 kind = {:?}, reason = {:?}, summary = {:?}",
        decision.kind,
        decision.reason,
        decision.summary
    );
}

/// 断言决策为 `WaitUser`。
fn assert_wait_user(decision: &GraphDecision, case: &str) {
    assert_eq!(
        decision.kind,
        GraphDecisionKind::WaitUser,
        "契约 C1 违约（{case}）：该场景必须判定 WaitUser，\
         实际 kind = {:?}, reason = {:?}, summary = {:?}",
        decision.kind,
        decision.reason,
        decision.summary
    );
}

/// 创建 armed/active 目标（objective 刻意不含任何续行关键词，见 `goal_supports_auto_continue` 词表）。
fn create_armed_goal(session_id: &str, objective: &str) -> pony_agent_core::agent::goal::GoalData {
    create_goal(CreateGoalArgs {
        session_id: Some(session_id.to_string()),
        objective: objective.to_string(),
        max_goal_rounds: Some(10),
    })
    .unwrap_or_else(|err| panic!("create_goal 必须成功（session={session_id}），实际 Err: {err}"))
    .goal
}

fn pause_goal(goal: &pony_agent_core::agent::goal::GoalData) {
    update_goal(UpdateGoalArgs {
        session_id: Some(goal.session_id.clone()),
        goal_id: goal.id.clone(),
        revision: goal.revision,
        action: UpdateGoalAction::Pause,
        objective: None,
        blocked_reason: None,
        max_goal_rounds: None,
    })
    .unwrap_or_else(|err| panic!("update_goal(Pause) 必须成功，实际 Err: {err}"));
}

// ===========================================================================
// 验收 2 / C1 —— 目标真值源 `active_goal_for_session`
// ===========================================================================

/// C1：`active_goal_for_session` 只认「同 session + phase=active + activation=Armed」。
#[test]
fn active_goal_for_session_matches_only_armed_active_goal_of_that_session() {
    // Arrange
    let _guard = wave1_guard();
    let session = "wave1-session-active-goal";
    let objective = "把报告写完";
    let created = create_armed_goal(session, objective);
    assert_eq!(
        created.activation,
        GoalActivation::Armed,
        "前置条件失败：新建目标必须是 Armed"
    );

    // Act：命中路径
    let hit = active_goal_for_session(session);

    // Assert
    let hit = hit.unwrap_or_else(|| {
        panic!("契约 C1 违约：session={session} 存在 armed/active 目标，active_goal_for_session 必须命中，实际 None")
    });
    assert_eq!(hit.id, created.id, "契约 C1 违约：命中目标的 id 必须与创建的一致");
    assert_eq!(
        hit.objective, objective,
        "契约 C1 违约：命中目标必须携带真实 objective，实际 = {:?}",
        hit.objective
    );
    assert_eq!(hit.phase, "active", "契约 C1 违约：命中目标 phase 必须为 active");
    assert_eq!(
        hit.activation,
        GoalActivation::Armed,
        "契约 C1 违约：命中目标 activation 必须为 Armed"
    );

    // Act + Assert：session 不匹配（禁止前缀 / 模糊匹配）
    assert!(
        active_goal_for_session("wave1-session-active-goal-other").is_none(),
        "契约 C1 违约：不同 session 不得命中该目标（必须精确匹配 session_id）"
    );
    assert!(
        active_goal_for_session("wave1-session-active").is_none(),
        "契约 C1 违约：session_id 前缀不得命中（禁止前缀匹配）"
    );

    // Act + Assert：不存在目标时返回 None（不得 panic、不得兜底到 default）
    assert!(
        active_goal_for_session("wave1-session-never-created").is_none(),
        "契约 C1 违约：无目标的 session 必须返回 None"
    );

    // Act + Assert：phase 不匹配（paused + disarmed）后不再命中
    pause_goal(&created);
    assert!(
        active_goal_for_session(session).is_none(),
        "契约 C1 违约：phase=paused / activation=disarmed 的目标不得被 active_goal_for_session 命中"
    );
}

// ===========================================================================
// 验收 2 / C1 —— planner 真值源（armed goal 优先于 run.goal 关键词）
// ===========================================================================

/// C1：planner 以 armed 目标的 objective 驱动 Continue（与关键词无关）；
/// 无目标时回退既有 run.goal 关键词启发（无关键词 → WaitUser）。
#[test]
fn planner_uses_armed_goal_as_truth_source_regardless_of_keywords() {
    // Arrange
    let _guard = wave1_guard();
    let session = "wave1-session-planner-truth";
    create_armed_goal(session, "把报告写完");

    let engine = GraphEngine::new("state-machine-v1");
    let run: GraphRun = engine.start_run("run-wave1-planner", "无关文本", Some(session));
    let handoff = completed_turn_handoff(&engine, session, "报告第三章已完成，接下来推进第四章。");

    // Act
    let decision =
        engine.decide_after_turn_with_planner(&run, &handoff, &DefaultGraphPlanner);

    // Assert：armed 目标命中 → Continue（objective 不含任何关键词）
    assert_continue(&decision, "armed goal 驱动续行，run.goal 与关键词必须无关");

    // Act + Assert：未建目标（新 session）且 run.goal 无关键词 → WaitUser（回退既有行为）
    let no_goal_session = "wave1-session-planner-no-goal";
    assert!(
        active_goal_for_session(no_goal_session).is_none(),
        "前置条件失败：{no_goal_session} 不应存在目标"
    );
    let fallback_run =
        engine.start_run("run-wave1-planner-fallback", "把报告写完", Some(no_goal_session));
    let fallback_handoff =
        completed_turn_handoff(&engine, no_goal_session, "报告第三章已完成，接下来推进第四章。");
    let fallback_decision = engine.decide_after_turn_with_planner(
        &fallback_run,
        &fallback_handoff,
        &DefaultGraphPlanner,
    );
    assert_wait_user(
        &fallback_decision,
        "无目标且 run.goal 无续行关键词时必须 WaitUser",
    );

    // Act + Assert：目标被 pause 后（disarmed）不得再驱动 Continue
    let paused_goal = create_armed_goal("wave1-session-planner-paused", "把报告写完");
    pause_goal(&paused_goal);
    let paused_run = engine.start_run(
        "run-wave1-planner-paused",
        "无关文本",
        Some("wave1-session-planner-paused"),
    );
    let paused_handoff = completed_turn_handoff(
        &engine,
        "wave1-session-planner-paused",
        "报告第三章已完成，接下来推进第四章。",
    );
    let paused_decision =
        engine.decide_after_turn_with_planner(&paused_run, &paused_handoff, &DefaultGraphPlanner);
    assert_wait_user(
        &paused_decision,
        "paused/disarmed 目标不得驱动 Continue",
    );
}

// ===========================================================================
// 验收 2 / C1 —— 问号启发式收紧（仅末尾问号拦截）
// ===========================================================================

/// C1：问号启发式只认「trim 后最后一个字符为 ?/？」，正文中间的 ? 不再拦截。
#[test]
fn question_heuristic_only_blocks_trailing_question_mark() {
    // Arrange
    let _guard = wave1_guard();
    let session = "wave1-session-question-heuristic";
    create_armed_goal(session, "把报告写完");
    let engine = GraphEngine::new("state-machine-v1");
    let run = engine.start_run("run-wave1-question", "无关文本", Some(session));

    // Act：问号出现在正文中间（代码/参数示例），不是提问
    let mid_question_handoff =
        completed_turn_handoff(&engine, session, "参数 query? 已修复，结果如下。");
    let mid_decision =
        engine.decide_after_turn_with_planner(&run, &mid_question_handoff, &DefaultGraphPlanner);

    // Assert
    assert_continue(
        &mid_decision,
        "正文中间出现 ? 不得判 WaitUser（仅末尾问号才拦截）",
    );

    // Act：末尾问号 → 明确在向用户提问
    let trailing_question_handoff =
        completed_turn_handoff(&engine, session, "还需要我继续吗？");
    let trailing_decision = engine.decide_after_turn_with_planner(
        &run,
        &trailing_question_handoff,
        &DefaultGraphPlanner,
    );

    // Assert
    assert_wait_user(
        &trailing_decision,
        "trim 后末尾为 ？时必须 WaitUser（即使存在 armed 目标）",
    );

    // Act + Assert：末尾为半角 ? 亦拦截
    let trailing_ascii_handoff = completed_turn_handoff(&engine, session, "May I proceed?");
    let trailing_ascii_decision = engine.decide_after_turn_with_planner(
        &run,
        &trailing_ascii_handoff,
        &DefaultGraphPlanner,
    );
    assert_wait_user(&trailing_ascii_decision, "末尾半角 ? 同样必须 WaitUser");
}

// ===========================================================================
// 验收 3 / C2 —— goal 持久化（跨 reset 读回 + 损坏 JSON 降级）
// ===========================================================================

/// env 与临时目录的兜底清理（panic 路径也生效）。
struct GoalStoreEnvCleanup {
    previous: Option<std::ffi::OsString>,
    dir: PathBuf,
    armed: bool,
}

impl Drop for GoalStoreEnvCleanup {
    fn drop(&mut self) {
        if self.armed {
            match self.previous.take() {
                Some(value) => std::env::set_var(GOAL_STORE_PATH_ENV, value),
                None => std::env::remove_var(GOAL_STORE_PATH_ENV),
            }
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}

fn unique_goal_store_dir() -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!(
        "pony-wave1-goal-store-{}-{}",
        std::process::id(),
        nanos
    ))
}

/// C2：`create_goal` → `reset_goal_store_for_test()` → `get_goal` 仍返回完整目标；
/// 损坏 JSON 降级为空且不 panic；路径真值来自 env `PONY_AGENT_GOAL_STORE_PATH`。
#[test]
fn goal_store_survives_reset_and_degrades_on_corrupt_json() {
    // Arrange
    let _guard = wave1_guard();
    let dir = unique_goal_store_dir();
    fs::create_dir_all(&dir)
        .unwrap_or_else(|err| panic!("创建临时目录失败 {}: {err}", dir.display()));
    let store_path = dir.join("goal_store.json");
    let mut cleanup = GoalStoreEnvCleanup {
        previous: std::env::var_os(GOAL_STORE_PATH_ENV),
        dir: dir.clone(),
        armed: true,
    };
    std::env::set_var(GOAL_STORE_PATH_ENV, &store_path);

    let session = "wave1-session-persistence";
    let objective = "把报告写完";

    // Assert：存储路径真值受 env 覆盖
    assert_eq!(
        goal_store_path(),
        store_path,
        "契约 C2 违约：goal_store_path() 必须取 env `{GOAL_STORE_PATH_ENV}` 覆盖值（期望 {}，实际 {}）",
        store_path.display(),
        goal_store_path().display()
    );

    // Act：创建目标后清空进程内缓存并重新加载（模拟重启）
    let created = create_goal(CreateGoalArgs {
        session_id: Some(session.to_string()),
        objective: objective.to_string(),
        max_goal_rounds: Some(7),
    })
    .unwrap_or_else(|err| panic!("create_goal 必须成功，实际 Err: {err}"))
    .goal;
    assert!(
        store_path.exists(),
        "契约 C2 违约：create_goal 成功后必须落盘 sidecar（期望文件 {} 存在）",
        store_path.display()
    );

    reset_goal_store_for_test();

    // Assert：跨 reset 读回完整目标
    let reloaded = get_goal(GetGoalArgs {
        session_id: Some(session.to_string()),
    })
    .unwrap_or_else(|err| panic!("契约 C2 违约：重启后 get_goal 不得返回 Err，实际 Err: {err}"));
    let reloaded = reloaded.goal.unwrap_or_else(|| {
        panic!(
            "契约 C2 违约：同一 sidecar 路径下重启后 get_goal 必须返回目标（路径 {}），实际 None",
            store_path.display()
        )
    });
    assert_eq!(reloaded.id, created.id, "契约 C2 违约：重启后目标 id 必须不变");
    assert_eq!(
        reloaded.objective, objective,
        "契约 C2 违约：重启后 objective 必须不变"
    );
    assert_eq!(reloaded.phase, "active", "契约 C2 违约：重启后 phase 必须不变");
    assert_eq!(
        reloaded.activation,
        GoalActivation::Armed,
        "契约 C2 违约：重启后 activation 必须不变"
    );
    assert_eq!(
        reloaded.revision, created.revision,
        "契约 C2 违约：重启后 revision 必须不变"
    );
    assert_eq!(
        reloaded.rounds_started, created.rounds_started,
        "契约 C2 违约：重启后 roundsStarted 必须不变"
    );
    assert_eq!(
        reloaded.max_goal_rounds, 7,
        "契约 C2 违约：重启后 maxGoalRounds 必须不变"
    );

    // Act：写入损坏 JSON 后重新加载
    fs::write(&store_path, b"{ \"broken\": not-json, ").unwrap_or_else(|err| {
        panic!("写入损坏 JSON 失败 {}: {err}", store_path.display())
    });
    reset_goal_store_for_test();

    // Assert：降级为空，不 panic、不 Err
    let degraded = get_goal(GetGoalArgs {
        session_id: Some(session.to_string()),
    })
    .unwrap_or_else(|err| {
        panic!("契约 C2 违约：损坏 sidecar 必须降级为空，get_goal 不得返回 Err，实际 Err: {err}")
    });
    assert!(
        degraded.goal.is_none(),
        "契约 C2 违约：损坏 JSON 必须降级为空目标（不得 panic、不得保留陈旧内存态），实际 Some({:?})",
        degraded.goal.as_ref().map(|goal| goal.id.clone())
    );

    // Cleanup：先摘 env 并回落到默认 store，再删除临时目录（避免污染同进程后续用例）
    std::env::remove_var(GOAL_STORE_PATH_ENV);
    reset_goal_store_for_test();
    cleanup.armed = false;
    if let Some(previous) = cleanup.previous.take() {
        std::env::set_var(GOAL_STORE_PATH_ENV, previous);
    }
    let _ = fs::remove_dir_all(&dir);
}

// ===========================================================================
// 验收 1 / C5 —— 提示词契约（Goal & planning policy / TodoWrite / 无 todo_write 残留）
// ===========================================================================

/// C5：`BASE_SYSTEM_PROMPT` 必须含 `Goal & planning policy` 段与模型可见工具名 `TodoWrite`，
/// 且不得残留 `todo_write` 文案。
#[test]
fn base_system_prompt_declares_goal_planning_policy_and_todowrite() {
    // Arrange
    let prompt = pony_agent_core::agent::context::base_system_prompt();

    // Act + Assert
    assert!(
        prompt.contains("Goal & planning policy"),
        "契约 C5 违约：BASE_SYSTEM_PROMPT 必须包含 `Goal & planning policy` 段（位于工程契约之后、输出规范之前）。当前长度 = {} 字节",
        prompt.len()
    );
    assert!(
        prompt.contains("TodoWrite"),
        "契约 C5 违约：BASE_SYSTEM_PROMPT 必须使用模型可见工具名 `TodoWrite` 建立计划。当前长度 = {} 字节",
        prompt.len()
    );
    assert!(
        !prompt.contains("todo_write"),
        "契约 C5 违约：BASE_SYSTEM_PROMPT 不得残留 `todo_write` 文案（全文工具名统一为 `TodoWrite`）"
    );
}
