//! B214：provider 重试事件（`llm/retry`）的持久化往返。
//!
//! 验收链（additive，不新增持久化通道）：
//! 真实重试事实 → 既有 turn 事件缓冲（`event_times_ms` 逐事件时刻）→ `turn_events` 落盘 →
//! 重载（新后端句柄）→ `TurnEvent` 反序列化 → 重试投影可读。
//!
//! 这里直接走 `SessionStore::persist_events_with_times`（与生产 buffered 通道的
//! `EventPersistFn` 汇入同一后端入口），从新句柄重载以证明「重启后可见」。

use std::path::PathBuf;

use pony_agent_core::agent::projection::Projection;
use pony_agent_core::agent::retry_projection::ProviderRetryProjectionState;
use pony_agent_core::agent::session::{
    SeparateTraceTableMode, SessionBackend, SessionStore,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;
use pony_agent_core::agent::turn_event::TurnEvent;

const SESSION_ID: &str = "b214-retry-roundtrip";
/// 与 `session::types::DEFAULT_HISTORY_BRANCH_ID` 同值（该常量对集成测试不可见）。
const BRANCH_ID: &str = "branch-main";

struct TempDb {
    path: PathBuf,
}

impl TempDb {
    fn new(label: &str) -> Self {
        let unique = format!(
            "pony-b214-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before unix epoch")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(format!("{unique}.db"));
        Self { path }
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let _ = std::fs::remove_file(self.path.with_extension("db-wal"));
        let _ = std::fs::remove_file(self.path.with_extension("db-shm"));
    }
}

fn retry_event(turn_id: &str, retry: u32, delay_ms: u64) -> TurnEvent {
    TurnEvent::LlmRetry {
        turn_id: turn_id.to_string(),
        request: "decision_stream".to_string(),
        scope: "decision".to_string(),
        failure_code: "rate_limited".to_string(),
        retry,
        max_attempts: 5,
        delay_ms,
    }
}

#[test]
fn llm_retry_event_round_trips_through_persistence_and_projection() {
    let temp = TempDb::new("retry-events");
    let turn_id = "turn-b214";
    let events = vec![
        TurnEvent::TurnStart {
            turn_id: turn_id.to_string(),
        },
        retry_event(turn_id, 1, 400),
        retry_event(turn_id, 2, 3_000),
    ];
    let event_times_ms = vec![1_700_000_000_000u64, 1_700_000_000_400, 1_700_000_003_400];

    // 写入：既有带逐事件时刻的落盘入口。
    {
        let store = SessionStore::with_backend(Box::new(SqliteSessionBackend::new_with_trace_mode(
            temp.path.clone(),
            SeparateTraceTableMode::Off,
        )));
        assert!(
            store.persist_events_with_times(
                SESSION_ID,
                turn_id,
                BRANCH_ID,
                events,
                event_times_ms.clone(),
            ),
            "llm/retry 事件必须能写入 turn_events"
        );
        // 集成测试需绕开 Windows 强制文件锁再开新句柄。
        store.release_log_writers_for_test();
    }

    // 落盘形态：事件名与逐事件产生时刻（复用 event_times_ms 语义）。
    {
        let conn = rusqlite::Connection::open(&temp.path).expect("open sqlite read-back");
        let mut stmt = conn
            .prepare(
                "SELECT event_type, created_at_ms FROM turn_events
                 WHERE session_id = ?1 ORDER BY seq",
            )
            .expect("prepare event query");
        let rows: Vec<(String, i64)> = stmt
            .query_map([SESSION_ID], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })
            .expect("query events")
            .map(|row| row.expect("event row"))
            .collect();
        assert_eq!(
            rows.iter().map(|(kind, _)| kind.as_str()).collect::<Vec<_>>(),
            vec!["turn/start", "llm/retry", "llm/retry"],
            "重试必须以独立事件类型落盘"
        );
        assert_eq!(
            rows.iter().map(|(_, at)| *at as u64).collect::<Vec<_>>(),
            event_times_ms,
            "created_at_ms 逐事件取 event_times_ms（不得整批塌缩）"
        );
    }

    // 重载：新后端句柄（模拟重启）读出事件并投影。
    let reader = SqliteSessionBackend::new_with_trace_mode(
        temp.path.clone(),
        SeparateTraceTableMode::Off,
    );
    let loaded = reader
        .load_turn_events_checked(SESSION_ID, None)
        .expect("reload events");
    assert_eq!(loaded.len(), 3, "重载后事件条数一致");

    let mut state = ProviderRetryProjectionState::init();
    for (seq, _branch, event) in &loaded {
        ProviderRetryProjectionState::apply(&mut state, *seq, event);
    }
    let retries = state.retries_for_turn(turn_id);
    assert_eq!(retries.len(), 2, "两次真实重试都可从重载事件投影出来");
    let latest = state.latest().expect("latest retry fact");
    assert_eq!(latest.retry, 2);
    assert_eq!(latest.max_attempts, 5, "客户端直接显示 2/5，无需 +1");
    assert_eq!(latest.delay_ms, 3_000, "客户端投影 3s 退避");
    assert_eq!(latest.failure_code, "rate_limited");
    assert_eq!(latest.request, "decision_stream");
    assert_eq!(latest.scope, "decision");
    assert_eq!(
        retries.iter().map(|fact| fact.seq).collect::<Vec<_>>(),
        {
            let mut seqs: Vec<u64> = loaded
                .iter()
                .filter(|(_, _, event)| matches!(event, TurnEvent::LlmRetry { .. }))
                .map(|(seq, _, _)| *seq)
                .collect();
            seqs.sort_unstable();
            seqs
        },
        "投影顺序与事件 seq 一致"
    );
}
