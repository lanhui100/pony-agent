//! T8 验收：非真流式 provider 的 `turn:delta` 必须携带真实 `session_id`。
//!
//! 缺陷（`turn_flow.rs:1395` 起的 `stream_delta_chunks` 家族）：该 helper 历史上没有 session
//! 形参，事件一律 `sessionId: null`。前端在 "缺 sessionId 的流事件不得被当前会话吸收"
//! （T2-5）严格化之后，这类事件被整条丢弃 ⇒ **非真流式 provider**
//! （`provider.supports_true_streaming_decision() == false` → buffered/sync 决策路径）在
//! `turn:completed` 之前完全不渲染回复。
//!
//! 本文件钉住两条契约：
//!   1. `stream_text_chunks` / `stream_reasoning_chunks` 发出的每条 `turn:delta` 必须
//!      原样携带调用方给出的 `session_id`（不得固定 None）；
//!   2. 反向钉子：调用方显式给 `None` 时事件才是 None —— 证明"非空"是真实传递而非常量伪造。
//!
//! 注：修复前该 helper 没有 session 形参（本用例编译即红），修复后由
//! `turn_stream.rs` 的 buffered 分支传入 `input.session_id`。

use pony_agent_core::agent::runtime::TurnStreamEvent;
use pony_agent_core::agent::turn_flow::{
    stream_reasoning_chunks, stream_text_chunks, TurnEventSink,
};
use std::sync::Mutex;

#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<(String, TurnStreamEvent)>>,
}

impl RecordingSink {
    fn deltas(&self) -> Vec<TurnStreamEvent> {
        self.events
            .lock()
            .expect("recording sink poisoned")
            .iter()
            .filter(|(name, _)| name == "turn:delta")
            .map(|(_, payload)| payload.clone())
            .collect()
    }
}

impl TurnEventSink for RecordingSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        self.events
            .lock()
            .expect("recording sink poisoned")
            .push((name.to_string(), payload));
    }
}

/// 非真流式路径的正向契约：分块发出的每个 `turn:delta` 都必须带真实 session_id。
#[test]
fn buffered_delta_events_must_carry_the_real_session_id() {
    const SESSION: &str = "t8-session-buffered";
    const TURN: &str = "t8-turn-1";
    let started_at = std::time::Instant::now();

    let text_sink = RecordingSink::default();
    stream_text_chunks(
        &text_sink,
        TURN,
        Some(SESSION),
        "calling_model",
        "非真流式 provider 的整段回复必须能被前端渲染",
        &started_at,
        None,
        false,
    );

    let text_deltas = text_sink.deltas();
    assert!(
        !text_deltas.is_empty(),
        "T8：buffered 路径必须真的发出 turn:delta（否则用例空转）"
    );
    for (index, event) in text_deltas.iter().enumerate() {
        assert_eq!(
            event.session_id.as_deref(),
            Some(SESSION),
            "T8：第 {index} 条 turn:delta 必须携带真实 session_id（否则前端严格过滤会整条丢弃 ⇒ 该类 provider 不渲染回复），实际 {:?}",
            event.session_id
        );
    }

    let reasoning_sink = RecordingSink::default();
    stream_reasoning_chunks(
        &reasoning_sink,
        TURN,
        Some(SESSION),
        "calling_model",
        "先想一下再回答",
        &started_at,
        None,
        false,
    );
    let reasoning_deltas = reasoning_sink.deltas();
    assert!(
        !reasoning_deltas.is_empty(),
        "T8：reasoning 分块也必须真的发出 turn:delta"
    );
    for (index, event) in reasoning_deltas.iter().enumerate() {
        assert_eq!(
            event.session_id.as_deref(),
            Some(SESSION),
            "T8：reasoning 第 {index} 条 turn:delta 必须携带真实 session_id，实际 {:?}",
            event.session_id
        );
    }
}

/// 反向钉子：session 事实确实是**传进来的**，不是常量伪造。
#[test]
fn delta_session_id_is_the_caller_supplied_value_not_a_constant() {
    const TURN: &str = "t8-turn-2";
    let started_at = std::time::Instant::now();

    let sink = RecordingSink::default();
    stream_text_chunks(
        &sink,
        TURN,
        Some("t8-session-alpha"),
        "calling_model",
        "alpha",
        &started_at,
        None,
        false,
    );
    stream_text_chunks(
        &sink,
        TURN,
        Some("t8-session-beta"),
        "calling_model",
        "beta",
        &started_at,
        None,
        false,
    );

    let deltas = sink.deltas();
    let sessions = deltas
        .iter()
        .map(|event| event.session_id.clone())
        .collect::<Vec<_>>();
    assert!(
        sessions.contains(&Some("t8-session-alpha".to_string())),
        "T8：alpha 调用的 delta 必须带 alpha 的会话，实际 {sessions:?}"
    );
    assert!(
        sessions.contains(&Some("t8-session-beta".to_string())),
        "T8：beta 调用的 delta 必须带 beta 的会话（不得是常量/上一次调用的残留），实际 {sessions:?}"
    );
}
