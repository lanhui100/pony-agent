//! B222 历史重建按 turn_id 幂等（红相）。
//!
//! 现场：重启/投影为空时的历史重建会把**同一轮重复写入的 user 事件**折成两条 user 消息
//! （`store.rs:748` 的第二次 push + `turn_events` 里同一 turn 的两条 `user/message`），
//! 于是界面与请求携带里出现「同一句话问两遍」，模型也据此重复作答。
//!
//! 契约：折叠/重建必须按 `turn_id` 幂等——同一轮的多条 user 事件只保留**第一条**且不得
//! 连带丢掉该轮的 assistant；不同轮次各自保留一条（禁止过度去重）；同一事件流重复重建
//! 结果必须完全相同（幂等，不允许抖动）。
//!
//! 本文件全部为真实断言；不读真实时钟、不 sleep、不吞异常。

use pony_agent_core::agent::session::{history_from_turn_events, SessionStore, TurnHistoryMessage};
use pony_agent_core::agent::turn_event::{TurnEndReason, TurnEvent};

/// 一条「重复写入的同一轮」事件流：同一 `turn_id` 出现两条 user（seq=1 与 seq=2）。
fn duplicate_user_turn_events() -> Vec<(u64, TurnEvent)> {
    vec![
        (
            0,
            TurnEvent::TurnStart {
                turn_id: "t1".to_string(),
            },
        ),
        (
            1,
            TurnEvent::UserMessage {
                turn_id: "t1".to_string(),
                text: "一直收到报警".to_string(),
                attachments: Vec::new(),
            },
        ),
        (
            2,
            TurnEvent::UserMessage {
                turn_id: "t1".to_string(),
                text: "一直收到报警".to_string(),
                attachments: Vec::new(),
            },
        ),
        (
            3,
            TurnEvent::AssistantMessage {
                turn_id: "t1".to_string(),
                step: 0,
                text: "先看告警详情".to_string(),
                reasoning_content: None,
                usage: None,
                chunk_missing: None,
            },
        ),
        (
            4,
            TurnEvent::TurnEnd {
                turn_id: "t1".to_string(),
                reason: TurnEndReason::Completed,
                turn_duration_ms: Some(1_200),
                error: None,
            },
        ),
    ]
}

fn count_role(history: &[TurnHistoryMessage], role: &str) -> usize {
    history
        .iter()
        .filter(|message| message.role == role)
        .count()
}

/// 1. 同一轮重复 user 事件：只保留一条 user，且 assistant 不得被连带丢弃。
#[test]
fn b222_duplicate_user_events_deduped_by_turn_id() {
    // Arrange
    let events = duplicate_user_turn_events();

    // Act
    let history = history_from_turn_events(&events);

    // Assert 1：user 只保留一条。
    assert_eq!(
        count_role(&history, "user"),
        1,
        "同一 turn_id 的重复 user 事件必须按轮次身份去重，实际历史 = {:?}",
        history
            .iter()
            .map(|message| (message.role.as_str(), message.content.as_str()))
            .collect::<Vec<_>>()
    );

    // Assert 2：assistant 仍在（去重不得把本轮回复一起吞掉）。
    assert_eq!(
        count_role(&history, "assistant"),
        1,
        "去重只能作用于重复 user，assistant 必须保留，实际 = {:?}",
        history
            .iter()
            .map(|message| (message.role.as_str(), message.content.as_str()))
            .collect::<Vec<_>>()
    );
    let assistant = history
        .iter()
        .find(|message| message.role == "assistant")
        .expect("前提：事件流有 assistant/message");
    assert_eq!(
        assistant.content, "先看告警详情",
        "assistant 正文必须逐字保留，实际 = {:?}",
        assistant.content
    );
    assert_eq!(
        assistant.turn_id.as_deref(),
        Some("t1"),
        "轮次身份必须随消息保留，实际 = {:?}",
        assistant.turn_id
    );
}

/// 2. 不同轮次各一条 user：必须各留一条（禁止按内容/全局过度去重）。
#[test]
fn b222_distinct_turns_each_keep_one_user() {
    // Arrange：两个不同 turn，user 文本刻意相同（内容相同不等于同一条事件）。
    let events: Vec<(u64, TurnEvent)> = vec![
        (
            0,
            TurnEvent::TurnStart {
                turn_id: "t1".to_string(),
            },
        ),
        (
            1,
            TurnEvent::UserMessage {
                turn_id: "t1".to_string(),
                text: "一直收到报警".to_string(),
                attachments: Vec::new(),
            },
        ),
        (
            2,
            TurnEvent::TurnEnd {
                turn_id: "t1".to_string(),
                reason: TurnEndReason::Completed,
                turn_duration_ms: None,
                error: None,
            },
        ),
        (
            3,
            TurnEvent::TurnStart {
                turn_id: "t2".to_string(),
            },
        ),
        (
            4,
            TurnEvent::UserMessage {
                turn_id: "t2".to_string(),
                text: "一直收到报警".to_string(),
                attachments: Vec::new(),
            },
        ),
        (
            5,
            TurnEvent::TurnEnd {
                turn_id: "t2".to_string(),
                reason: TurnEndReason::Completed,
                turn_duration_ms: None,
                error: None,
            },
        ),
    ];

    // Act
    let history = history_from_turn_events(&events);

    // Assert
    assert_eq!(
        count_role(&history, "user"),
        2,
        "两个不同轮次必须各保留一条 user（按 turn_id 去重，不得按内容去重），实际 = {:?}",
        history
            .iter()
            .map(|message| (message.role.as_str(), message.content.as_str(), message.turn_id.clone()))
            .collect::<Vec<_>>()
    );
}

/// 3. 重建幂等：同一事件流连续重建两次，user 计数相同且均为 1。
#[test]
fn b222_rebuild_is_idempotent() {
    // Arrange
    let events = duplicate_user_turn_events();

    // Act
    let first = history_from_turn_events(&events);
    let second = history_from_turn_events(&events);

    // Assert
    let first_users = count_role(&first, "user");
    let second_users = count_role(&second, "user");
    assert_eq!(
        first_users, 1,
        "首次重建必须去重为 1 条 user，实际 = {first_users}"
    );
    assert_eq!(
        first_users, second_users,
        "同一事件流两次重建的 user 数必须相同（幂等），实际 {first_users} vs {second_users}"
    );
    assert_eq!(
        first.len(),
        second.len(),
        "两次重建的消息总数必须一致（幂等，不允许抖动）"
    );
}

/// 4. 普通单条 user 轮次：去重逻辑不得改动既有行为（内容逐字保留）。
#[test]
fn b222_single_user_unchanged() {
    // Arrange
    const TEXT: &str = "帮我查一下 8080 端口的占用进程";
    let events: Vec<(u64, TurnEvent)> = vec![
        (
            0,
            TurnEvent::TurnStart {
                turn_id: "t-single".to_string(),
            },
        ),
        (
            1,
            TurnEvent::UserMessage {
                turn_id: "t-single".to_string(),
                text: TEXT.to_string(),
                attachments: Vec::new(),
            },
        ),
        (
            2,
            TurnEvent::TurnEnd {
                turn_id: "t-single".to_string(),
                reason: TurnEndReason::Completed,
                turn_duration_ms: None,
                error: None,
            },
        ),
    ];

    // Act
    let history = history_from_turn_events(&events);

    // Assert
    assert_eq!(
        count_role(&history, "user"),
        1,
        "普通轮次必须恰好一条 user，实际 = {:?}",
        history
            .iter()
            .map(|message| (message.role.as_str(), message.content.as_str()))
            .collect::<Vec<_>>()
    );
    assert_eq!(
        history[0].content, TEXT,
        "既有单条 user 正文必须逐字保留，实际 = {:?}",
        history[0].content
    );
}

/// 5. store 级幂等：同一 turn_id 连续两次 append ⇒ user 与 assistant 各只留一条，且保留**首次**写入。
///
/// 真实重复源于写路径的第二次 push（`session/store.rs:748`）；本用例走公开
/// `SessionStore::memory_only()`（test-support 特性，dev-dependency 已启用）构造隔离后端。
#[test]
fn b222_store_append_same_turn_id_is_idempotent() {
    // Arrange：内存后端（不落盘、不污染真实会话库）；第二次写入内容刻意不同，
    // 用于区分「保留首次」与「后端覆盖成末次」两种实现。
    let mut store = SessionStore::memory_only();
    let session = Some("session-b222");

    // Act：同一 turn_id 连续写入两次（模拟重放/补写）。
    store
        .append_turn_fallible_with_kind(
            session,
            "一直收到报警",
            "先看告警详情",
            None,
            Vec::new(),
            None,
            Some("t1"),
        )
        .expect("第一次 append 必须成功");
    let snapshot = store
        .append_turn_fallible_with_kind(
            session,
            "一直收到报警（重放补写）",
            "重复回复（重放补写）",
            None,
            Vec::new(),
            None,
            Some("t1"),
        )
        .expect("第二次 append 必须成功（幂等写入不得报错）");

    // Assert 1：两条角色各只留一条。
    let history_dump = || {
        snapshot
            .history
            .iter()
            .map(|message| {
                (
                    message.role.as_str(),
                    message.content.as_str(),
                    message.turn_id.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        count_role(&snapshot.history, "user"),
        1,
        "同一 turn_id 连续两次 append 后 user 只应有一条，实际 = {:?}",
        history_dump()
    );
    assert_eq!(
        count_role(&snapshot.history, "assistant"),
        1,
        "同一 turn_id 的重复 append 也必须把 assistant 去重为一条，实际 = {:?}",
        history_dump()
    );

    // Assert 2：必须保留**首次**写入（去重不得变成「末次覆盖」）。
    let user = snapshot
        .history
        .iter()
        .find(|message| message.role == "user")
        .expect("前提：历史必须含 user 消息");
    assert_eq!(
        user.content, "一直收到报警",
        "重复 append 后必须保留首次写入的 user 正文，实际 = {:?}",
        user.content
    );
    let assistant = snapshot
        .history
        .iter()
        .find(|message| message.role == "assistant")
        .expect("前提：历史必须含 assistant 消息");
    assert_eq!(
        assistant.content, "先看告警详情",
        "重复 append 后必须保留首次写入的 assistant 正文，实际 = {:?}",
        assistant.content
    );
    assert_eq!(
        assistant.turn_id.as_deref(),
        Some("t1"),
        "轮次身份必须随消息保留，实际 = {:?}",
        assistant.turn_id
    );
}

/// 6. 守护：不同 turn_id 的两条 assistant 不得被合并（禁止过度去重）。
#[test]
fn b222_distinct_turns_assistants_not_merged() {
    // Arrange：两个不同轮次，assistant 文本刻意相同（内容相同 ≠ 同一轮）。
    let events: Vec<(u64, TurnEvent)> = vec![
        (
            0,
            TurnEvent::TurnStart {
                turn_id: "t1".to_string(),
            },
        ),
        (
            1,
            TurnEvent::UserMessage {
                turn_id: "t1".to_string(),
                text: "查一下端口".to_string(),
                attachments: Vec::new(),
            },
        ),
        (
            2,
            TurnEvent::AssistantMessage {
                turn_id: "t1".to_string(),
                step: 0,
                text: "已排查".to_string(),
                reasoning_content: None,
                usage: None,
                chunk_missing: None,
            },
        ),
        (
            3,
            TurnEvent::TurnEnd {
                turn_id: "t1".to_string(),
                reason: TurnEndReason::Completed,
                turn_duration_ms: None,
                error: None,
            },
        ),
        (
            4,
            TurnEvent::TurnStart {
                turn_id: "t2".to_string(),
            },
        ),
        (
            5,
            TurnEvent::UserMessage {
                turn_id: "t2".to_string(),
                text: "查一下端口".to_string(),
                attachments: Vec::new(),
            },
        ),
        (
            6,
            TurnEvent::AssistantMessage {
                turn_id: "t2".to_string(),
                step: 0,
                text: "已排查".to_string(),
                reasoning_content: None,
                usage: None,
                chunk_missing: None,
            },
        ),
        (
            7,
            TurnEvent::TurnEnd {
                turn_id: "t2".to_string(),
                reason: TurnEndReason::Completed,
                turn_duration_ms: None,
                error: None,
            },
        ),
    ];

    // Act
    let history = history_from_turn_events(&events);

    // Assert
    assert_eq!(
        count_role(&history, "assistant"),
        2,
        "不同 turn_id 的 assistant 必须各留一条（去重只认轮次身份），实际 = {:?}",
        history
            .iter()
            .map(|message| (
                message.role.as_str(),
                message.content.as_str(),
                message.turn_id.clone()
            ))
            .collect::<Vec<_>>()
    );
    let turn_ids: Vec<&str> = history
        .iter()
        .filter(|message| message.role == "assistant")
        .filter_map(|message| message.turn_id.as_deref())
        .collect();
    assert_eq!(
        turn_ids,
        vec!["t1", "t2"],
        "两条 assistant 必须分别归属 t1 / t2，实际 = {turn_ids:?}"
    );
}
