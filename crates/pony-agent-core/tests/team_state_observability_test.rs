//! B1795535505312：队友运行态必须对模型可判读。
//!
//! 事故链（`~/job_copilot` 2026-11-24 实测，转登本仓 `backlog:B1795535505312`）：
//! `spawn_teammate` 返回登记快照 `active`、成功/失败/中断三态全塌缩成 `inactive`、
//! `list_agents(wait_ms)` 丢弃 `WaitAgentResult` ⇒ Lead 无法判定队友「在跑 / 跑完 / 跑挂 /
//! 被打断」，只能人催或把超时当进展。
//!
//! 判定口径与 `.agents/notes/implemented/bug-fix/2026-11-25-team-member-status-observability.md`
//! 一一对应：
//! 1. 派生回执是**回读的运行态**，并带 `delivery_state`；空 prompt 必须是 `not_started`。
//! 2. 引擎失败 ⇒ `failed` + `last_error` 文本；不得伪装成 `inactive`。
//! 3. `interrupt_agent` ⇒ `interrupted`；不得与「跑完了」同态。
//! 4. `is_driving` 跟随真实驱动线程（在途为 true，收口后为 false）。
//! 5. `list_agents(wait_ms)` 的 `timed_out` / `no_progress` 必须回传模型，不得丢弃。
//!
//! 隔离方式：子代理引擎槽是进程级 `OnceLock`（每进程只能注入一次），团队名册/收件箱也是
//! 进程级静态表 ⇒ 本文件用一个**行为可切换**的 Stub 引擎 + 进程内串行锁，让四个场景共享同一
//! 引擎槽且互不串扰（比「每用例一个引擎」更贴近真实单进程运行）。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use pony_agent_core::agent::orchestration::{
    self, interrupt_agent, list_agents, spawn_teammate_in_workspace_with_provider_as,
    InterruptAgentArgs, SpawnTeammateArgs, SpawnTeammateResult, TeammateMember, TeammateStatus,
    DELIVERY_STATE_DRIVING, DELIVERY_STATE_NOT_STARTED,
};
use pony_agent_core::agent::subagent_runtime::{
    set_subagent_engine, SubagentRunOutcome, SubagentTurnEngine,
};
use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolRouter};
use serde_json::Value;

/// 场景串行锁：同一时刻只允许一个场景改写引擎行为与团队名册。
static SCENARIO_LOCK: Mutex<()> = Mutex::new(());

fn scenario_guard() -> MutexGuard<'static, ()> {
    SCENARIO_LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

/// 失败开关：置位后 `run_turn` 一律返回 `subagent_busy`。
static ENGINE_FAILS: AtomicBool = AtomicBool::new(false);

/// 在途闸门：非 `None` 时 turn 会挂住直到放行，用于确定性制造「驱动在跑」窗口。
static ENGINE_GATE: Mutex<Option<Arc<(Mutex<bool>, Condvar)>>> = Mutex::new(None);

/// 行为可切换的 Stub 引擎（进程内唯一注入）。
struct SwitchableEngine;

impl SubagentTurnEngine for SwitchableEngine {
    fn run_turn(&self, _prompt: &str, _timeout_ms: u64) -> Result<SubagentRunOutcome, String> {
        let gate = ENGINE_GATE
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .clone();
        if let Some(gate) = gate {
            let (lock, condvar) = &*gate;
            let mut released = lock.lock().unwrap_or_else(|err| err.into_inner());
            let mut waited = Duration::ZERO;
            while !*released && waited < Duration::from_secs(5) {
                let (next, _timeout) = condvar
                    .wait_timeout(released, Duration::from_millis(20))
                    .unwrap_or_else(|err| err.into_inner());
                released = next;
                waited += Duration::from_millis(20);
            }
        }
        if ENGINE_FAILS.load(Ordering::SeqCst) {
            return Err("subagent_busy".to_string());
        }
        Ok(SubagentRunOutcome {
            final_text: "b1795535505312-stub-reply".to_string(),
            session_id: "b1795535505312-stub-session".to_string(),
        })
    }
}

/// 一次性引擎注入（重复注入的 Err 无妨：槽已被本文件占用）。
fn ensure_engine() {
    let _ = set_subagent_engine(Arc::new(SwitchableEngine) as Arc<dyn SubagentTurnEngine>);
    assert!(
        pony_agent_core::agent::tools::subagent_runtime_wired(),
        "前置失败：Stub 引擎注入后必须已接线"
    );
}

fn set_failing(failing: bool) {
    ENGINE_FAILS.store(failing, Ordering::SeqCst);
}

fn set_gate(gate: Option<Arc<(Mutex<bool>, Condvar)>>) {
    *ENGINE_GATE.lock().unwrap_or_else(|err| err.into_inner()) = gate;
}

fn release_gate(gate: &Arc<(Mutex<bool>, Condvar)>) {
    let (lock, condvar) = &**gate;
    *lock.lock().unwrap_or_else(|err| err.into_inner()) = true;
    condvar.notify_all();
}

fn unique_name(tag: &str) -> String {
    format!("{tag}-{}", std::process::id())
}

fn spawn_args(name: &str, prompt: &str) -> SpawnTeammateArgs {
    SpawnTeammateArgs {
        name: name.to_string(),
        role: Some("probe".to_string()),
        description: format!("{name} 运行态可见性探针"),
        prompt: prompt.to_string(),
        context: None,
        provider_id: None,
        model_id: None,
    }
}

fn spawn(name: &str, prompt: &str) -> SpawnTeammateResult {
    spawn_teammate_in_workspace_with_provider_as(
        spawn_args(name, prompt),
        None,
        None,
        &orchestration::TeamCaller::lead(),
    )
    .expect("派生必须成功")
}

fn member_of(target: &str) -> TeammateMember {
    list_agents()
        .expect("花名册必须可读")
        .agents
        .into_iter()
        .find(|member| member.target == target)
        .unwrap_or_else(|| panic!("成员 `{target}` 必须在册"))
}

fn wait_until(limit: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    condition()
}

/// T1：派生回执是运行态回读 + 派发阶段；空 prompt 不得声称已开始。
#[test]
fn t1_spawn_receipt_reports_live_state_and_delivery_state() {
    let _guard = scenario_guard();
    ensure_engine();
    set_failing(false);
    set_gate(None);

    let name = unique_name("state-live");
    let spawned = spawn(&name, "t1 initial prompt");
    let target = spawned.member.target.clone();

    // Assert 1（派发阶段）：非空 prompt + 引擎在位 ⇒ 必须声称已派发驱动。
    assert_eq!(
        spawned.delivery_state, DELIVERY_STATE_DRIVING,
        "T1 失败：非空 prompt 派生必须回执 delivery_state=driving（实际 {:?}）",
        spawned.delivery_state
    );

    // Assert 2（回读运行态）：不再是登记快照的 `active`。
    assert_ne!(
        spawned.member.status,
        TeammateStatus::Active,
        "T1 失败：派生回执仍是登记快照 `active`（未回读运行态）——这正是原事故的误判来源"
    );

    // Assert 3（收敛到可判读的空闲态）：收口后 `inactive` 且无脏 `last_error`。
    assert!(
        wait_until(Duration::from_secs(5), || {
            member_of(&target).status == TeammateStatus::Inactive
        }),
        "T1 失败：5s 内未收敛到 inactive（实际 {:?}）",
        member_of(&target).status
    );
    let settled = member_of(&target);
    assert!(
        settled.last_error.is_none(),
        "T1 失败：成功收口后 last_error 必须清空（实际 {:?}）",
        settled.last_error
    );
    assert!(
        !settled.is_driving,
        "T1 失败：驱动收口后仍标记 is_driving=true"
    );

    // Assert 4（空 prompt 不伪造进展）。
    let inert_name = unique_name("state-inert");
    let inert = spawn(&inert_name, "   ");
    assert_eq!(
        inert.delivery_state, DELIVERY_STATE_NOT_STARTED,
        "T1 失败：空 prompt 派生必须回执 delivery_state=not_started（实际 {:?}）",
        inert.delivery_state
    );
    assert_eq!(
        inert.member.status,
        TeammateStatus::Active,
        "T1 失败：空 prompt 派生后成员应保持空闲 active（实际 {:?}）",
        inert.member.status
    );
}

/// T2：引擎失败 ⇒ `failed` + 可判读错误文本；失败后新消息可重新拉起。
#[test]
fn t2_failed_turn_is_visible_as_failed_with_error_text() {
    let _guard = scenario_guard();
    ensure_engine();
    set_gate(None);
    set_failing(true);

    let name = unique_name("state-fail");
    let spawned = spawn(&name, "t2 trigger");
    let target = spawned.member.target.clone();

    assert!(
        wait_until(Duration::from_secs(5), || {
            member_of(&target).status == TeammateStatus::Failed
        }),
        "T2 失败：5s 内未落到 failed（实际 {:?}）——失败被伪装成了空闲",
        member_of(&target).status
    );
    let failed = member_of(&target);
    assert!(
        failed
            .last_error
            .as_deref()
            .is_some_and(|text| text.contains("subagent_busy")),
        "T2 失败：failed 必须带真实错误码文本（实际 {:?}）",
        failed.last_error
    );
    assert!(
        !failed.is_driving,
        "T2 失败：失败收口后仍标记 is_driving=true"
    );

    // 失败不粘死：新消息必须把成员拉回可驱动状态。
    set_failing(false);
    orchestration::enqueue_message(orchestration::SendMessageArgs {
        target: target.clone(),
        message: "t2 retry".to_string(),
        mode: Some(orchestration::MessageDeliveryMode::Queue),
    })
    .expect("T2 失败后入队必须成功");
    assert_eq!(
        member_of(&target).status,
        TeammateStatus::Active,
        "T2 失败：failed 成员收到新消息后必须回到 active（否则永久卡死）"
    );
}

/// T3：中断 ⇒ `interrupted`，与「跑完了」「跑挂了」都能区分；`is_driving` 跟随真实驱动线程。
#[test]
fn t3_interrupt_is_distinguishable_from_completion_and_failure() {
    let _guard = scenario_guard();
    ensure_engine();
    set_failing(false);
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    set_gate(Some(Arc::clone(&gate)));

    let name = unique_name("state-int");
    let spawned = spawn(&name, "t3 hold in flight");
    let target = spawned.member.target.clone();

    assert!(
        wait_until(Duration::from_secs(5), || member_of(&target).is_driving),
        "T3 前置失败：5s 内未观察到驱动线程持有该 target"
    );
    let interrupted = interrupt_agent(InterruptAgentArgs {
        target: target.clone(),
        reason: Some("t3 interrupt".to_string()),
    })
    .expect("T3 中断必须成功");
    assert_eq!(
        interrupted.new_status,
        TeammateStatus::Interrupted,
        "T3 失败：中断后状态必须是 interrupted（实际 {:?}）",
        interrupted.new_status
    );

    release_gate(&gate);
    set_gate(None);
    assert!(
        wait_until(Duration::from_secs(5), || !member_of(&target).is_driving),
        "T3 失败：5s 内驱动线程未退出（is_driving 恒 true）"
    );
    assert_eq!(
        member_of(&target).status,
        TeammateStatus::Interrupted,
        "T3 失败：中断收口后状态必须保持 interrupted（实际 {:?}）",
        member_of(&target).status
    );
}

/// T4：`list_agents(wait_ms)` 必须把 `timed_out` / `no_progress` 回传模型。
#[test]
fn t4_list_agents_surfaces_wait_result() {
    let _guard = scenario_guard();
    ensure_engine();
    set_failing(false);
    set_gate(None);

    let router = ToolRouter::new();
    let call = ToolCall {
        call_id: None,
        name: "list_agents".to_string(),
        arguments: serde_json::json!({ "wait_ms": 10_000u64 }),
        plan: None,
    };
    let result = router.execute_with_context(&call, &ToolExecutionContext::default());
    assert_eq!(
        result.status, "ok",
        "T4：list_agents 必须成功：{}",
        result.output
    );

    let parsed: Value = serde_json::from_str(&result.output).expect("T4 输出必须是合法 JSON");
    let wait = parsed
        .get("wait")
        .expect("T4 失败：list_agents 丢弃了 wait 结果（响应里没有 `wait` 字段）");
    assert!(
        wait.get("timed_out").is_some(),
        "T4 失败：wait 结果缺少 timed_out 判据：{wait}"
    );
    // 无活跃同伴 ⇒ 内核立即返回 no_progress（不阻塞），模型必须能看到原因。
    assert!(
        wait.get("no_progress").is_some(),
        "T4 失败：无活跃同伴时 wait 必须带 no_progress 原因（实际 {wait}）"
    );
}
