// 历史治理门禁：追加语义 / 压缩双写 / 压缩可重放 / 事件物化
// （单独成文件，避免继续撑大已触棘轮的 `session/tests.rs`；与
// `host_continuation_goal_round_tests.rs` 同一取舍）。
//
// 四个关注点是同一条链：上下文体积靠**压缩**控制而不是靠丢历史；压缩必须同时改写两条
// 记忆载体（history + native transcript），且必须留下**可重放**的事实（`history/squash`
// 事件）——只写 blob/索引时，「压到哪」只有派生视图知道，索引投影一丢（自愈路径
// 「投影空 → 按事件重建历史」）就把被压缩的旧轮次整段复活，压缩等于白做。

use super::*;
use crate::agent::turn_event::TurnEvent;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// 记录 `FlushEvents` 的后端替身：事件夹具可变（先建 store 才能问出活动分支）。
struct RecordingEventBackend {
    attachment_root: PathBuf,
    events: Arc<Mutex<Vec<(u64, String, TurnEvent)>>>,
    flushed: Arc<Mutex<Vec<TurnEvent>>>,
}

impl SessionBackend for RecordingEventBackend {
    fn load_store(&self) -> Option<PersistedStore> {
        None
    }
    fn save_store(&self, _store: &PersistedStore) -> Result<(), String> {
        Ok(())
    }
    fn attachment_root(&self) -> Option<PathBuf> {
        Some(self.attachment_root.clone())
    }
    fn supports_turn_events(&self) -> bool {
        true
    }
    fn load_turn_events(
        &self,
        _session_id: &str,
        _up_to_seq: Option<u64>,
    ) -> Vec<(u64, String, TurnEvent)> {
        self.events
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }
    fn persist_command(&self, command: PersistCommand) -> PersistCommandOutcome {
        if let PersistCommand::FlushEvents { events, .. } = command {
            self.flushed
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .extend(events);
        }
        PersistCommandOutcome::Succeeded
    }
}

fn history_message(role: &str, content: &str) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: role.to_string(),
        content: content.to_string(),
        ..Default::default()
    }
}

/// 门禁 ①：压缩必须**可重放**——落一条 `history/squash` 事件，且由它折叠出的历史
/// 与进程内压缩结果一致（摘要置顶 + 保留窗口）。
///
/// 反例（本修复前的真实缺陷）：只写 blob/索引 ⇒ 索引投影一丢，被压缩的旧轮次整段复活。
#[test]
fn commit_compaction_appends_replayable_history_squash_event() {
    let dir = {
        let raw = std::env::temp_dir().join(format!("compaction-squash-{}", std::process::id()));
        raw.canonicalize().unwrap_or(raw)
    };
    let event_log: Arc<Mutex<Vec<(u64, String, TurnEvent)>>> = Arc::new(Mutex::new(Vec::new()));
    let flushed: Arc<Mutex<Vec<TurnEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let mut store = SessionStore::with_backend(Box::new(RecordingEventBackend {
        attachment_root: dir.clone(),
        events: Arc::clone(&event_log),
        flushed: Arc::clone(&flushed),
    }));
    let session_id = "compaction-squash";
    store.ensure_session(session_id);
    // 事件必须落在会话的**活动分支**上，否则血缘过滤会把它们全部跳过（折叠为空，
    // 边界退化为 0）。
    let branch = store
        .session_active_branch_id(session_id)
        .unwrap_or_else(|| "branch-main".to_string());
    let events: Vec<(u64, String, TurnEvent)> = vec![
        (
            1,
            branch.clone(),
            TurnEvent::UserMessage {
                turn_id: "t1".to_string(),
                text: "旧问题".to_string(),
                attachments: Vec::new(),
            },
        ),
        (
            2,
            branch.clone(),
            TurnEvent::AssistantMessage {
                turn_id: "t1".to_string(),
                step: 0,
                text: "旧回答".to_string(),
                reasoning_content: None,
                usage: None,
                chunk_missing: None,
            },
        ),
        (
            3,
            branch.clone(),
            TurnEvent::UserMessage {
                turn_id: "t2".to_string(),
                text: "保留问题".to_string(),
                attachments: Vec::new(),
            },
        ),
        (
            4,
            branch,
            TurnEvent::AssistantMessage {
                turn_id: "t2".to_string(),
                step: 0,
                text: "保留回答".to_string(),
                reasoning_content: None,
                usage: None,
                chunk_missing: None,
            },
        ),
    ];
    *event_log.lock().unwrap_or_else(|error| error.into_inner()) = events.clone();

    let summary = "[会话摘要] t1 已讨论";
    let compacted_history = vec![
        history_message("user", summary),
        history_message("user", "保留问题"),
        history_message("assistant", "保留回答"),
    ];
    store.commit_compaction(
        Some(session_id),
        compacted_history,
        Vec::new(),
        summary.to_string(),
    );

    let flushed = flushed
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let (base_seq, summary_message) = flushed
        .iter()
        .find_map(|event| match event {
            TurnEvent::HistorySquash {
                base_seq,
                summary_message,
            } => Some((*base_seq, summary_message.clone())),
            _ => None,
        })
        .expect("压缩必须落 history/squash 事件（否则按事件重建会复活被压缩历史）");
    assert_eq!(
        base_seq, 2,
        "边界必须是最后一个被压缩消息的 seq（1/2 被压掉，3/4 保留）"
    );
    assert_eq!(summary_message, summary);

    // 端到端：把落盘事件（含 squash）重新折叠，必须得到与进程内一致的压缩视图。
    let mut replay: Vec<(u64, TurnEvent)> = events
        .into_iter()
        .map(|(seq, _branch, event)| (seq, event))
        .collect();
    replay.push((
        5,
        TurnEvent::HistorySquash {
            base_seq,
            summary_message,
        },
    ));
    let rebuilt = history_from_turn_events(&replay);
    let contents: Vec<&str> = rebuilt
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(
        contents,
        vec![summary, "保留问题", "保留回答"],
        "重放结果必须与压缩后的历史同序同内容（摘要置顶 + 保留窗口）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// 门禁：历史**不再按轮数滑窗**。
///
/// 曾经 `DEFAULT_HISTORY_LIMIT = 24` 会在每轮成功提交后丢掉最老消息 ⇒ 前缀每轮都变，
/// provider 前缀缓存（ADR 0007 一等指标）每轮全灭。现在只有紧急上限兜底，正常会话
/// 逐轮**只追加**。
#[test]
fn history_is_append_only_below_emergency_limit() {
    let mut store = SessionStore::memory_only();
    for index in 0..30 {
        store.append_turn(
            Some("append-only"),
            &format!("问题 {index}"),
            &format!("回答 {index}"),
            None,
            Vec::new(),
        );
    }

    let snapshot = store.snapshot(Some("append-only"), &[]);
    assert_eq!(
        snapshot.history.len(),
        60,
        "30 轮 = 60 条，一条都不许因为「窗口」被丢掉"
    );
    assert_eq!(snapshot.history[0].content, "问题 0", "最老一轮必须还在");
}

/// 门禁：压缩**同时**改写两条记忆载体。
///
/// native-tool-flow 会话的上下文只看 `provider_native_transcript`：只压 `history`
/// 等于没压（下一请求照旧把全部工具往返发上去）。
#[test]
fn commit_compaction_rewrites_history_and_transcript_together() {
    let mut store = SessionStore::memory_only();
    let transcript_turn = |label: &str| vec![serde_json::json!({"role": "user", "content": label})];
    store.append_turn(
        Some("compact-dual"),
        "q1",
        "a1",
        Some(transcript_turn("q1")),
        Vec::new(),
    );
    store.append_turn(
        Some("compact-dual"),
        "q2",
        "a2",
        Some(transcript_turn("q2")),
        Vec::new(),
    );

    let compacted_history = vec![TurnHistoryMessage {
        role: "user".to_string(),
        content: "summary".to_string(),
        ..Default::default()
    }];
    store.commit_compaction(
        Some("compact-dual"),
        compacted_history,
        transcript_turn("q2"),
        "summary".to_string(),
    );

    let snapshot = store.snapshot(Some("compact-dual"), &[]);
    assert_eq!(snapshot.history.len(), 1);
    assert_eq!(snapshot.history[0].content, "summary");
    assert_eq!(
        snapshot.provider_native_transcript.len(),
        1,
        "transcript 必须与 history 同步裁剪，否则压缩对 native 会话无效"
    );
    assert_eq!(snapshot.provider_native_transcript[0]["content"], "q2");
}

#[test]
fn materialize_last_turn_messages_picks_latest_turn() {
    use crate::agent::turn_event::{TurnEndReason, TurnEvent};
    let events = |events: Vec<TurnEvent>| {
        events
            .into_iter()
            .enumerate()
            .map(|(seq, event)| (seq as u64, "main".to_string(), event))
            .collect::<Vec<_>>()
    };
    // 两个 turn：物化必须取最近一个。
    let two_turns = events(vec![
        TurnEvent::TurnStart {
            turn_id: "t1".into(),
        },
        TurnEvent::UserMessage {
            turn_id: "t1".into(),
            text: "第一问".into(),
            attachments: Vec::new(),
        },
        TurnEvent::AssistantMessage {
            turn_id: "t1".into(),
            step: 0,
            text: "第一答".into(),
            reasoning_content: None,
            usage: None,
            chunk_missing: None,
        },
        TurnEvent::TurnEnd {
            turn_id: "t1".into(),
            reason: TurnEndReason::Completed,
            turn_duration_ms: Some(10),
        },
        TurnEvent::TurnStart {
            turn_id: "t2".into(),
        },
        TurnEvent::UserMessage {
            turn_id: "t2".into(),
            text: "第二问".into(),
            attachments: Vec::new(),
        },
        TurnEvent::AssistantMessage {
            turn_id: "t2".into(),
            step: 0,
            text: "第二答".into(),
            reasoning_content: Some("推理R".into()),
            usage: None,
            chunk_missing: None,
        },
    ]);
    let (user, assistant) = materialize_last_turn_messages(&two_turns, 0).expect("materialized");
    assert_eq!(user.as_deref(), Some("第二问"));
    assert_eq!(
        assistant
            .as_ref()
            .map(|(text, reasoning)| (text.as_str(), reasoning.as_deref())),
        Some(("第二答", Some("推理R")))
    );

    // failed turn：仅 user/message——assistant 为 None（调用方补占位）。
    let failed_turn = events(vec![
        TurnEvent::TurnStart {
            turn_id: "t3".into(),
        },
        TurnEvent::UserMessage {
            turn_id: "t3".into(),
            text: "失败问".into(),
            attachments: Vec::new(),
        },
        TurnEvent::TurnEnd {
            turn_id: "t3".into(),
            reason: TurnEndReason::Error,
            turn_duration_ms: None,
        },
    ]);
    let (user, assistant) = materialize_last_turn_messages(&failed_turn, 0).expect("materialized");
    assert_eq!(user.as_deref(), Some("失败问"));
    assert!(assistant.is_none(), "failed turn has no assistant message");

    // 无 turn 归属事件 → None（整体回退）。
    assert!(materialize_last_turn_messages(&[], 0).is_none());
}
