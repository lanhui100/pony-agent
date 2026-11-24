//! B239：日志重建的逐事件时间语义（`flush_events.eventTimes` 解码，不得整批塌缩）。
//!
//! 现场：`log_command` 把逐事件产生时刻编码成 `eventTimes`（C6.2），但 fold 侧只解码
//! `events`、忽略 `eventTimes`，于是「从日志重建」得到的事件时间不可复原。
//! 契约：fold 出的重建记录逐事件携带 `created_at_ms`；旧日志缺键时回落该批日志行的
//! `ts`（批首时间），部分缺位回落本批第一个正数时间——绝不写 0、绝不整批塌缩到
//! flush 时刻。
//!
//! 本文件全部为真实断言；不读真实时钟、不 sleep、不吞异常。

use pony_agent_core::agent::session::{fold_session, LogEvent};

fn flush_line(
    seq: u64,
    ts: u64,
    session_id: &str,
    event_times_ms: Option<&[u64]>,
) -> String {
    let events = serde_json::json!([
        {"type": "turn_start", "turn_id": "b239-turn"},
        {"type": "user_message", "turn_id": "b239-turn", "text": "第一问"},
        {"type": "assistant_message", "turn_id": "b239-turn", "step": 0, "text": "第一答"}
    ]);
    let mut cmd = serde_json::json!({
        "type": "flush_events",
        "encodingVersion": 1,
        "epoch": 1,
        "sessionId": session_id,
        "turnId": "b239-turn",
        "branchId": "main",
        "events": events,
    });
    if let Some(times) = event_times_ms {
        cmd["eventTimes"] = serde_json::json!(times);
    }
    serde_json::json!({"seq": seq, "ts": ts, "cmd": cmd}).to_string()
}

fn fold_lines(lines: &[String]) -> pony_agent_core::agent::session::FoldedSession {
    let events: Vec<LogEvent> = lines
        .iter()
        .map(|line| LogEvent::from_json_line(line).expect("parse log line"))
        .collect();
    fold_session(&events).expect("fold session log")
}

/// 逐事件时间必须从 `eventTimes` 解码并回填到重建记录。
#[test]
fn log_rebuild_keeps_per_event_created_at_ms() {
    let session_id = "pa_b239_times";
    let times = [1_700_000_000_100u64, 1_700_000_000_450, 1_700_000_003_900];
    let folded = fold_lines(&[
        flush_line(1, 1_760_000_000_001, session_id, Some(&times)),
    ]);

    assert_eq!(folded.event_rows, 3);
    assert_eq!(
        folded
            .event_records
            .iter()
            .map(|record| record.created_at_ms)
            .collect::<Vec<_>>(),
        times,
        "重建记录的 created_at_ms 必须逐事件取 eventTimes（不得整批塌缩）"
    );
    assert!(
        folded
            .event_records
            .iter()
            .all(|record| record.turn_id == "b239-turn"),
        "重建记录必须携带事件自身的轮次身份"
    );
    assert_eq!(
        folded.event_records[2].event.type_name(),
        "assistant/message",
        "事件本体必须原样重建"
    );
}

/// 旧日志（无 `eventTimes` 键）：回落该批日志行的 `ts`（批首时间），保持兼容。
#[test]
fn log_rebuild_without_event_times_falls_back_to_batch_head() {
    let session_id = "pa_b239_legacy";
    let batch_ts = 1_760_000_000_007u64;
    let folded = fold_lines(&[
        flush_line(1, batch_ts, session_id, None),
    ]);

    assert_eq!(folded.event_rows, 3);
    assert_eq!(
        folded
            .event_records
            .iter()
            .map(|record| record.created_at_ms)
            .collect::<Vec<_>>(),
        vec![batch_ts; 3],
        "缺失 eventTimes 的旧日志必须回落到批首时间（同一批一致），不得写 0"
    );
}

/// `eventTimes` 短于事件数：缺失位回落批首（第一个正数时间），已有位保持原值。
#[test]
fn log_rebuild_partial_event_times_backfills_tail_with_batch_head() {
    let session_id = "pa_b239_partial";
    let folded = fold_lines(&[
        flush_line(
            1,
            1_760_000_000_009,
            session_id,
            Some(&[1_700_000_000_100]),
        ),
    ]);
    assert_eq!(
        folded
            .event_records
            .iter()
            .map(|record| record.created_at_ms)
            .collect::<Vec<_>>(),
        vec![1_700_000_000_100; 3],
        "缺位必须回落批首时间，不得留 0 或塌缩到日志 ts"
    );
}
