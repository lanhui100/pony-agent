//! 红相验收：并发 turn 的授权上下文隔离（T2-1）。
//!
//! 缺陷：`GovernedToolExecutor` 用一个共享 `Mutex<DispatchContext>` 保存“当前调用上下文”，
//! 每个 turn 开始由运行时整块覆盖（`set_context`）。并发会话下后写者覆盖先写者，
//! 于是 A 会话的工具调用会带着 B 会话的 `session_id` / `turn_id` / `tool_authorization_mode`
//! 进入治理链（权限判定、控制请求落库、跨会话守卫全部按错误的会话事实决策）。
//!
//! 观测缝：`ToolPolicyEvaluator::evaluate_with_context` 是公开注入点，dispatcher 会把**本次调用**
//! 实际使用的 `DispatchContext` 传给它（dispatcher.rs:968-969）。用录制型 evaluator 记录
//! 每次调用看到的会话事实，即可判定“上下文是否互相污染”，无需真实进程、无 sleep。
//!
//! 契约要求（实现可自由选择 API 形状：per-turn 上下文对象 / 显式传入 / 从
//! `ToolExecutionContext.session_id` 派生）：**每次工具调用只能看到自己这次 turn 的会话事实**。
//! 若实现改动了 `set_context` / `ToolExecutor` 公开签名，本文件只需同步调用形状，断言语义不变。

use pony_agent_core::agent::dispatcher::{
    DispatchContext, GovernedDispatcher, PermissionDecision, PermissionVerdict, ToolPolicyEvaluator,
};
use pony_agent_core::agent::dispatcher_composites::GovernedToolExecutor;
use pony_agent_core::agent::tool_runtime::{FakeClock, InvocationOrigin};
use pony_agent_core::agent::tools::{
    ToolCall, ToolDescriptor, ToolExecutionContext, ToolExecutor, ToolRegistrySnapshot,
};
use serde_json::{json, Value};
use std::sync::{Arc, Barrier, Mutex};

/// 一次工具调用实际看到的会话事实（按调用方埋入的 marker 归档）。
#[derive(Clone, Debug)]
struct ObservedContext {
    marker: String,
    session_id: Option<String>,
    turn_id: Option<String>,
    tool_authorization_mode: Option<String>,
}

/// 录制型策略评估器：只记录上下文，然后一律 Deny。
/// 用 Deny 是为了让管线在“权限判定”这一步停下——本用例不关心执行结果，
/// 只关心判定所依据的会话事实，因此不需要注册 handler、不依赖任何工具实现。
#[derive(Default)]
struct ContextRecorder {
    observed: Mutex<Vec<ObservedContext>>,
}

impl ContextRecorder {
    fn snapshot(&self) -> Vec<ObservedContext> {
        self.observed
            .lock()
            .expect("recorder poisoned")
            .clone()
    }

    fn record(&self, marker: String, context: &DispatchContext) {
        self.observed
            .lock()
            .expect("recorder poisoned")
            .push(ObservedContext {
                marker,
                session_id: context.session_id.clone(),
                turn_id: context.turn_id.clone(),
                tool_authorization_mode: context.tool_authorization_mode.clone(),
            });
    }
}

impl ToolPolicyEvaluator for ContextRecorder {
    fn evaluate(
        &self,
        _descriptor: &ToolDescriptor,
        _origin: &InvocationOrigin,
        _final_arguments: &Value,
    ) -> PermissionDecision {
        PermissionDecision {
            verdict: PermissionVerdict::Deny,
            decision_source: "t2_concurrent_context_recorder".to_string(),
            reason: Some("T2-1 recorder: refuse execution, context observation only".to_string()),
        }
    }

    fn evaluate_with_context(
        &self,
        _descriptor: &ToolDescriptor,
        _origin: &InvocationOrigin,
        final_arguments: &Value,
        context: &DispatchContext,
    ) -> PermissionDecision {
        // marker 走 description：所有内置工具都要求 description，且它不会被改写。
        let marker = final_arguments
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("<missing-marker>")
            .to_string();
        self.record(marker, context);
        self.evaluate(_descriptor, _origin, final_arguments)
    }
}

fn context(session: &str, turn: &str, mode: &str) -> DispatchContext {
    DispatchContext {
        session_id: Some(session.to_string()),
        turn_id: Some(turn.to_string()),
        tool_authorization_mode: Some(mode.to_string()),
        ..Default::default()
    }
}

/// `workspace_list_files` 是常驻模型可见的只读工具，schema 只要求 `description`
/// （其余 `path` / `limit` 可选，`additionalProperties: false`），args 可直接过校验。
fn call(call_id: &str) -> ToolCall {
    ToolCall {
        call_id: Some(call_id.to_string()),
        name: "workspace_list_files".to_string(),
        arguments: json!({ "description": call_id, "path": "." }),
        plan: None,
    }
}

fn build_executor(recorder: Arc<ContextRecorder>) -> GovernedToolExecutor {
    let dispatcher = GovernedDispatcher::new(
        Arc::new(ToolRegistrySnapshot::builtin().expect("builtin registry must build")),
        Arc::new(FakeClock::new(0)),
    );
    dispatcher.register_policy_evaluator(recorder);
    GovernedToolExecutor::new(dispatcher)
}

fn assert_isolated(observed: &[ObservedContext], marker: &str, session: &str, turn: &str, mode: &str) {
    let hit = observed
        .iter()
        .find(|entry| entry.marker == marker)
        .unwrap_or_else(|| {
            panic!(
                "T2-1：未观测到 marker={marker} 的调用上下文；观测到的 marker={:?}（说明该调用没有走到策略判定，用例失效）",
                observed.iter().map(|entry| entry.marker.clone()).collect::<Vec<_>>()
            )
        });
    assert_eq!(
        hit.session_id.as_deref(),
        Some(session),
        "T2-1：会话 {session} 的调用必须带着自己的 session_id，实际 {:?}（并发下上下文被另一 turn 覆盖）",
        hit.session_id
    );
    assert_eq!(
        hit.turn_id.as_deref(),
        Some(turn),
        "T2-1：会话 {session} 的调用必须带着自己的 turn_id，实际 {:?}",
        hit.turn_id
    );
    assert_eq!(
        hit.tool_authorization_mode.as_deref(),
        Some(mode),
        "T2-1：会话 {session} 的调用必须带着自己的授权模式 {mode}，实际 {:?}（授权模式串档会造成越权/误拒）",
        hit.tool_authorization_mode
    );
}

/// 确定性并发：双线程 + Barrier。两个 turn 都先写入自己的上下文，再同时发起调用。
/// 现状：共享槽位被后写者覆盖 ⇒ 至少一个 turn 看到另一 turn 的会话事实。
/// 修复后：每次调用只看到自己的上下文。
#[test]
fn concurrent_turns_must_not_share_the_authorization_context_slot() {
    let recorder = Arc::new(ContextRecorder::default());
    let executor = Arc::new(build_executor(Arc::clone(&recorder)));
    // 2 个参与线程 + 主线程不参与；两轮 barrier：①双方写完上下文 ②双方调用完成
    let barrier = Arc::new(Barrier::new(2));

    let turns = [
        ("t2-session-A", "t2-turn-A", "read_only"),
        ("t2-session-B", "t2-turn-B", "full_access"),
    ];

    let mut handles = Vec::new();
    for (index, (session, turn, mode)) in turns.into_iter().enumerate() {
        let executor = Arc::clone(&executor);
        let barrier = Arc::clone(&barrier);
        let session = session.to_string();
        let turn = turn.to_string();
        let mode = mode.to_string();
        handles.push(std::thread::spawn(move || {
            // ① 先让两个 turn 都把自己的调用参数（含 turn 事实）准备好，再同时发起调用（固定顺序，无 sleep）
            let marker = format!("t2-call-{index}-{session}");
            let exec_context = ToolExecutionContext {
                session_id: Some(session.clone()),
                dispatch_context: Some(context(&session, &turn, &mode)),
                workspace_root: None,
                ..Default::default()
            };
            barrier.wait();
            let _ = executor.execute_with_context(&call(&marker), &exec_context);
            // ② 双方调用都结束后再让主线程继续，避免提前 join 造成的时序噪声
            barrier.wait();
        }));
    }
    for handle in handles {
        handle.join().expect("worker thread panicked");
    }

    let observed = recorder.snapshot();
    assert_eq!(
        observed.len(),
        2,
        "T2-1：两个并发 turn 都应各触发一次策略判定，实际 {} 次：{observed:?}",
        observed.len()
    );
    assert_isolated(&observed, "t2-call-0-t2-session-A", "t2-session-A", "t2-turn-A", "read_only");
    assert_isolated(&observed, "t2-call-1-t2-session-B", "t2-session-B", "t2-turn-B", "full_access");
}

/// 上一 turn 的上下文不得成为后续调用的“环境默认值”：
/// 一个从未设置过上下文的调用必须看到空会话，而不是继承上一个 turn 的会话事实。
#[test]
fn context_must_not_carry_over_to_the_next_turn() {
    let recorder = Arc::new(ContextRecorder::default());
    let executor = Arc::new(build_executor(Arc::clone(&recorder)));

    // turn-1：会话 A 的只读 turn（turn 事实只随本次调用传入，不存在共享槽位）
    let turn_one_context = ToolExecutionContext {
        session_id: Some("t2-carry-session-A".to_string()),
        dispatch_context: Some(context("t2-carry-session-A", "t2-carry-turn-1", "read_only")),
        ..Default::default()
    };
    let _ = executor.execute_with_context(&call("t2-carry-call-1"), &turn_one_context);

    // turn-2：新 executor 语义上的“干净调用”——调用方没有提供任何会话事实
    let _ = executor.execute_with_context(&call("t2-carry-call-2"), &ToolExecutionContext::default());

    let observed = recorder.snapshot();
    let second = observed
        .iter()
        .find(|entry| entry.marker == "t2-carry-call-2")
        .expect("T2-1：第二次调用必须走到策略判定");
    assert_eq!(
        second.session_id, None,
        "T2-1：没有会话事实的调用不得继承上一 turn 的 session_id，实际 {:?}",
        second.session_id
    );
    assert_eq!(
        second.turn_id, None,
        "T2-1：没有会话事实的调用不得继承上一 turn 的 turn_id，实际 {:?}",
        second.turn_id
    );
    assert_eq!(
        second.tool_authorization_mode, None,
        "T2-1：没有会话事实的调用不得继承上一 turn 的授权模式，实际 {:?}（fail-open 授权面）",
        second.tool_authorization_mode
    );
}
