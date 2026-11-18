//! B239：ACP 委派 flush 的并发去重（per-`(session,turn)` in-flight 幂等键）。
//!
//! 缺陷面：终态 flush 与 step/drain flush 共享进程级缓冲并各自执行
//! 「快照 → 提交 → 移除」。没有 in-flight 标记时，两路并发 flush 都能在任一方移除
//! 之前快照到同一批事件，于是同一批事件被提交两次（`turn_events` 重复行 + seq 分叉）。
//!
//! 本用例用 `Barrier` 让两路 flush 在同一时刻竞争，断言**恰好一路**提交整批，
//! 另一路看到空批返回 0；随后再 drain 仍为 0（幂等）。全部为真实断言，不读时钟、
//! 不 sleep。

use super::*;
use crate::agent::turn_event::TurnEvent;

use std::sync::Barrier;
use std::thread;

/// 两路并发 flush 不得重复提交同一批缓冲事件。
#[test]
fn concurrent_flushes_commit_each_buffered_event_exactly_once() {
    let _cp_lock = control_plane_test_lock();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("pony-b239-flush-dedup-{stamp}"));
    fs::create_dir_all(&dir).expect("create temp dir");

    let sessions = SessionStore::with_backend(Box::new(
        crate::agent::sqlite_session::SqliteSessionBackend::new_with_trace_mode(
            dir.join("sessions.db"),
            crate::agent::session::SeparateTraceTableMode::Off,
        ),
    ));
    let runtime = AgentRuntime::with_dependencies(
        sessions,
        Box::new(StaticResolver {
            selection: test_provider_selection("http://127.0.0.1:1/v1".to_string()),
        }),
        Box::new(ToolRouter::new()),
        Box::new(LocalTurnPlanner),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    let control_plane = HostControlPlane::with_runtime(runtime);
    let persist = control_plane.event_persist_channel();
    let flush = control_plane.event_flush_channel();

    let session_id = "b239-flush-dedup";
    let turn_id = "b239-turn-1";
    // 直接走本控制面通道，避免全局单槽被并行用例抢占。
    persist(
        session_id,
        turn_id,
        TurnEvent::TurnStart {
            turn_id: turn_id.to_string(),
        },
        false,
    );
    persist(
        session_id,
        turn_id,
        TurnEvent::UserMessage {
            turn_id: turn_id.to_string(),
            text: "并发 flush 只能提交一次".to_string(),
            attachments: Vec::new(),
        },
        false,
    );
    persist(
        session_id,
        turn_id,
        TurnEvent::AssistantMessage {
            turn_id: turn_id.to_string(),
            step: 0,
            text: "确认".to_string(),
            reasoning_content: None,
            usage: None,
            chunk_missing: None,
        },
        false,
    );

    // Act：两路 flush 在 Barrier 处对齐后同时竞争同一批事件。
    let barrier = Arc::new(Barrier::new(2));
    let mut handles = Vec::new();
    for _ in 0..2 {
        let flush = Arc::clone(&flush);
        let barrier = Arc::clone(&barrier);
        handles.push(thread::spawn(move || {
            barrier.wait();
            flush(session_id)
        }));
    }
    let mut committed: Vec<usize> = handles
        .into_iter()
        .map(|handle| {
            handle
                .join()
                .expect("flush thread panicked")
                .expect("flush must succeed")
        })
        .collect();
    committed.sort_unstable();

    // Assert 1：恰好一路提交整批（3 条），另一路看到 in-flight/空批返回 0。
    assert_eq!(
        committed,
        vec![0, 3],
        "两路并发 flush 不得重复提交同一批事件（允许的提交分布只有 [0,3]）"
    );

    // Assert 2：落库行数与事件数一致，没有重复行。
    let rows = control_plane
        .load_turn_events_checked(session_id, None)
        .expect("event table readable");
    assert_eq!(
        rows.iter()
            .map(|(_, _, event)| event.type_name())
            .collect::<Vec<_>>(),
        vec!["turn/start", "user/message", "assistant/message"],
        "同一批事件必须按原顺序只落盘一次"
    );

    // Assert 3：提交成功后缓冲已移除，再 drain 返回 0（幂等键不会留下可重提交残批）。
    assert_eq!(
        flush(session_id).expect("drain after commit"),
        0,
        "同一批事件提交后再次 drain 必须为空（不得二次提交）"
    );
    assert_eq!(
        control_plane
            .load_turn_events_checked(session_id, None)
            .expect("event table readable")
            .len(),
        3,
        "重复 drain 不得增加事件行"
    );

    let _ = fs::remove_dir_all(&dir);
}
