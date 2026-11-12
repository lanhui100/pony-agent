//! ADR 0123 补充 v2（wave adr0123-supp-r2）验收测试：B2 / D1 契约。
//!
//! 契约真源：`.dev-team/contract-matrix-adr0123-supp-r2.md`
//! - B2-1：`failed_turn_checkpoint_text` 的「失败原因」行必须①在「已执行的工具步骤：」之前、
//!   ②换行/连续空白折叠为单空格、③截断至 ≤200 字符（超限加 `…`）；reason 空时与
//!   `interrupted_turn_checkpoint_text` 逐字一致。
//! - B2-2：折叠/截断不得改变 `[中断轮次检查点]` 前缀与「已执行的工具步骤：」段格式。
//! - D1-1/D1-2：两个中断注记常量迁至 `agent::session` 作为单一真源；
//!   `is_interrupt_placeholder_text` 只认「`[中断轮次检查点` 前缀」或「与常量整句相等」，
//!   裸子串匹配（正文引用）必须判 false。
//!
//! 零假测试：每个用例均有硬断言，禁止 `#[ignore]`、`unwrap_or`、吞异常。

use pony_agent_core::agent::session::{
    failed_turn_checkpoint_text, interrupted_turn_checkpoint_text, is_interrupt_placeholder_text,
    UPSTREAM_INTERRUPTED_NOTE, USER_CANCELLED_NOTE,
};

const REASON_PREFIX: &str = "失败原因：";
const TOOLS_HEADER: &str = "已执行的工具步骤：";

// ---------------------------------------------------------------------------
// T-B2a（B2-1 ①② + B2-2）：多行/HTML reason 折叠为单行，且位于工具段之前
// ---------------------------------------------------------------------------

#[test]
fn t_b2a_multiline_reason_folds_to_single_line_before_tool_steps() {
    // Arrange
    let prior_text = "正文";
    let tool_activities = vec!["Run 探测".to_string()];
    let raw_reason = "502 Bad Gateway\n<html>\n<body>";

    // Act
    let checkpoint = failed_turn_checkpoint_text(prior_text, &tool_activities, raw_reason)
        .expect("正文/工具步骤/失败原因皆非空时必须产出检查点文本");

    // Assert 1：前缀与工具段格式不变（B2-2 不变量）
    assert!(
        checkpoint.starts_with("[中断轮次检查点"),
        "检查点前缀必须保持 `[中断轮次检查点`，实际: {checkpoint:?}"
    );
    assert!(
        checkpoint.ends_with(&format!("{TOOLS_HEADER}\n- Run 探测")),
        "「已执行的工具步骤：」段格式不得被折叠/截断改写，实际: {checkpoint:?}"
    );

    // Assert 2：位置 —— 失败原因行在工具段之前（两侧都必须存在）
    let reason_pos = checkpoint
        .find(REASON_PREFIX)
        .expect("检查点必须包含 `失败原因：` 行");
    let tools_pos = checkpoint
        .find(TOOLS_HEADER)
        .expect("检查点必须包含 `已执行的工具步骤：` 段");
    assert!(
        reason_pos < tools_pos,
        "「失败原因」行必须位于工具步骤段之前，reason_pos={reason_pos}, tools_pos={tools_pos}, 实际: {checkpoint:?}"
    );

    // Assert 3：reason 部分不带换行（不含 '\n'），且折叠为单空格分隔
    let after_reason = checkpoint
        .split(REASON_PREFIX)
        .nth(1)
        .expect("检查点必须包含 `失败原因：` 行");
    let reason_line = after_reason
        .split('\n')
        .next()
        .expect("split 至少产出一段");
    assert!(
        !reason_line.contains('\n'),
        "失败原因行不得含换行，实际: {reason_line:?}"
    );
    assert!(
        reason_line.contains("502 Bad Gateway"),
        "折叠不得丢失 reason 首段，实际: {reason_line:?}"
    );
    assert!(
        reason_line.contains("<html>") && reason_line.contains("<body>"),
        "折叠不得丢失 reason 的后续行内容，实际: {reason_line:?}"
    );
    assert!(
        !checkpoint.contains("\n<html>") && !checkpoint.contains("\n<body>"),
        "reason 内部的换行必须被折叠，不得形成新的行，实际: {checkpoint:?}"
    );

    // Assert 4：整体行数 = 5（header / 正文 / 失败原因 / 工具段标题 / 一条工具行），
    // 多一行即说明 reason 的换行泄漏成了新行。
    let lines: Vec<&str> = checkpoint.lines().collect();
    assert_eq!(
        lines.len(),
        5,
        "检查点行数必须仍为 5（reason 折叠为单行），实际行: {lines:?}"
    );
    assert_eq!(
        lines[2],
        "失败原因：502 Bad Gateway <html> <body>",
        "失败原因行必须是单行折叠结果，实际: {:?}",
        lines[2]
    );
}

// ---------------------------------------------------------------------------
// T-B2b（B2-1 ③）：超长 reason 截断且带省略号
// ---------------------------------------------------------------------------

#[test]
fn t_b2b_oversized_reason_is_truncated_with_ellipsis() {
    // Arrange
    let raw_reason = "x".repeat(500);

    // Act
    let checkpoint = failed_turn_checkpoint_text("正文", &[], &raw_reason)
        .expect("正文与失败原因皆非空时必须产出检查点文本");

    // Assert 1：截断标记存在
    assert!(
        checkpoint.contains('…'),
        "超长 reason 必须被截断并追加 `…`，实际: {checkpoint:?}"
    );

    // Assert 2：reason 部分长度受限（契约 ≤200 字符；Lead 上限 <260 总长）
    let reason_part = checkpoint
        .split(REASON_PREFIX)
        .nth(1)
        .expect("检查点必须包含 `失败原因：` 行");
    assert!(
        reason_part.chars().count() <= 200,
        "截断后 reason 部分不得超过 200 字符，实际: {}",
        reason_part.chars().count()
    );
    assert!(
        reason_part.ends_with('…'),
        "截断后 reason 部分必须以 `…` 结尾，实际尾部: {:?}",
        reason_part.chars().rev().take(5).collect::<String>()
    );
    assert!(
        checkpoint.chars().count() < 260,
        "折叠截断后检查点总长必须 < 260 字符，实际: {}",
        checkpoint.chars().count()
    );
    assert!(
        checkpoint.starts_with("[中断轮次检查点"),
        "截断不得破坏检查点前缀，实际: {checkpoint:?}"
    );
}

// ---------------------------------------------------------------------------
// T-B2c（B2-1 回归）：reason 为空 ⇒ 与既有检查点逐字一致
// ---------------------------------------------------------------------------

#[test]
fn t_b2c_empty_reason_stays_verbatim_equal_to_legacy_checkpoint() {
    // Arrange
    let prior_text = "已做一半";
    let tool_activities = vec!["Run 探测".to_string()];
    let expected = interrupted_turn_checkpoint_text(prior_text, &tool_activities)
        .expect("既有实现必须为同样入参产出检查点文本");

    // Act + Assert：空串与纯空白（trim 后为空）都必须逐字一致
    for empty_reason in ["", "   ", "\n\t  \n"] {
        let actual = failed_turn_checkpoint_text(prior_text, &tool_activities, empty_reason);
        assert_eq!(
            actual.as_deref(),
            Some(expected.as_str()),
            "failure_reason={empty_reason:?}（trim 后为空）时必须逐字等于 interrupted_turn_checkpoint_text"
        );
        let actual_text = actual
            .as_deref()
            .expect("非空正文+工具步骤时 actual 必须为 Some");
        assert!(
            !actual_text.contains(REASON_PREFIX),
            "空失败原因不得留下空的 `失败原因：` 行，实际: {actual_text:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// T-D1a（D1-2 负向）：正文引用中断注记 ⇒ false（禁止裸子串匹配）
// ---------------------------------------------------------------------------

#[test]
fn t_d1a_body_text_quoting_interrupt_note_is_not_placeholder() {
    // Arrange：正文「引用」中断注记（含完整子串，但不是注记本身）
    let quoting_texts = [
        "（本轮由用户终止的原因见 trace）",
        "复盘：本轮上游调用中断，未产出最终回复；上面的工具往返是已执行的真实步骤。原因是网关 502。",
        "日志里出现 `[中断轮次检查点]` 字样，这里只是一句引用。",
    ];

    for text in quoting_texts {
        // Act
        let is_placeholder = is_interrupt_placeholder_text(text);

        // Assert
        assert!(
            !is_placeholder,
            "正文引用不得被判为中断注记（禁止裸子串匹配），实际判为 true: {text:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// T-D1b（D1-1/D1-2 正向）：检查点前缀与两个常量整句 ⇒ true
// ---------------------------------------------------------------------------

#[test]
fn t_d1b_checkpoint_prefix_and_note_constants_are_placeholders() {
    // Arrange
    let checkpoint_prefix_texts = [
        "[中断轮次检查点]",
        "[中断轮次检查点]\n已执行的工具步骤：",
        "[中断轮次检查点]\n失败原因：工具 `Run` 已连续失败 4 次",
    ];

    for text in checkpoint_prefix_texts {
        // Act + Assert
        assert!(
            is_interrupt_placeholder_text(text),
            "`[中断轮次检查点` 前缀必须判为中断注记，实际 false: {text:?}"
        );
    }

    // Assert：两个常量整句（单一真源）必须是可识别的中断注记
    assert!(
        is_interrupt_placeholder_text(UPSTREAM_INTERRUPTED_NOTE),
        "UPSTREAM_INTERRUPTED_NOTE 整句必须判为中断注记，实际常量: {UPSTREAM_INTERRUPTED_NOTE:?}"
    );
    assert!(
        is_interrupt_placeholder_text(USER_CANCELLED_NOTE),
        "USER_CANCELLED_NOTE 整句必须判为中断注记，实际常量: {USER_CANCELLED_NOTE:?}"
    );

    // Assert：常量语义必须仍是两类不同中断（防止常量被误写成同一句）
    assert_ne!(
        UPSTREAM_INTERRUPTED_NOTE, USER_CANCELLED_NOTE,
        "上游中断注记与用户终止注记必须是两个不同常量"
    );
    assert!(
        UPSTREAM_INTERRUPTED_NOTE.contains("上游调用中断"),
        "UPSTREAM_INTERRUPTED_NOTE 必须表达上游中断语义，实际: {UPSTREAM_INTERRUPTED_NOTE:?}"
    );
    assert!(
        USER_CANCELLED_NOTE.contains("由用户终止"),
        "USER_CANCELLED_NOTE 必须表达用户终止语义，实际: {USER_CANCELLED_NOTE:?}"
    );

    // Assert：带前后空白的常量整句（trim 后相等）仍判 true
    let padded = format!("  {USER_CANCELLED_NOTE}\n");
    assert!(
        is_interrupt_placeholder_text(&padded),
        "常量整句带空白包裹时必须判为中断注记（判定需先 trim），实际: {padded:?}"
    );
}

// ---------------------------------------------------------------------------
// T-D1c（回归）：普通业务回复 ⇒ false
// ---------------------------------------------------------------------------

#[test]
fn t_d1c_normal_business_reply_is_not_placeholder() {
    // Arrange
    let normal_texts = [
        "正常回复正文",
        "目录中有 3 个文件，已查看完毕。",
        "已完成工具调用并汇总：3 个端口在监听。",
    ];

    for text in normal_texts {
        // Act
        let is_placeholder = is_interrupt_placeholder_text(text);

        // Assert
        assert!(
            !is_placeholder,
            "普通业务正文不得被判为中断占位文本，实际判为 true: {text:?}"
        );
    }
}

// ===========================================================================
// R2-T2：A1（turn_id 历史腿）/ B1（重投影同源）验收用例
// ===========================================================================

use pony_agent_core::agent::projection::{fold_all, HistoryProjectionState};
use pony_agent_core::agent::session::{HistoryMessageKind, MessageStatus, SessionStore};
use pony_agent_core::agent::turn_event::{TurnEndReason, TurnEvent};

/// T-A1a（A1-1）：`append_turn_with_kind` 显式传 turn_id ⇒ user/assistant 两条历史消息
/// 都携带该 turn 身份（`turn_outcome_persisted` 历史腿的唯一真值源）；
/// 不传 turn_id 的老用法（`append_turn`）必须仍为 `None`（老会话读回行为不变）。
#[test]
fn t_a1a_history_messages_carry_explicit_turn_id() {
    // Arrange
    let mut store = SessionStore::memory_only();
    let session_id = "r2-a1a-session";

    // Act 1：显式 turn_id
    let with_turn_id = store.append_turn_with_kind(
        Some(session_id),
        "失败轮用户提问",
        "失败轮检查点正文",
        None,
        Vec::new(),
        Some(HistoryMessageKind::Interrupted),
        Some("turn-xyz"),
    );

    // Assert 1：两条消息都带 turn_id，且可派生出去重键 `{turn_id}-{role}`
    let history = &with_turn_id.history;
    assert!(
        history.len() >= 2,
        "追加一轮后历史至少含 user+assistant 两条，实际: {}",
        history.len()
    );
    let user_message = &history[history.len() - 2];
    let assistant_message = &history[history.len() - 1];
    assert_eq!(user_message.role, "user");
    assert_eq!(assistant_message.role, "assistant");
    assert_eq!(
        user_message.turn_id.as_deref(),
        Some("turn-xyz"),
        "user 历史消息必须携带本轮 turn_id（历史腿判据），实际: {:?}",
        user_message.turn_id
    );
    assert_eq!(
        assistant_message.turn_id.as_deref(),
        Some("turn-xyz"),
        "assistant 历史消息必须携带本轮 turn_id（历史腿判据），实际: {:?}",
        assistant_message.turn_id
    );
    assert_eq!(
        user_message.stable_id(),
        "turn-xyz-user",
        "去重键派生规则必须是 `{{turn_id}}-{{role}}`"
    );
    assert_eq!(
        assistant_message.stable_id(),
        "turn-xyz-assistant",
        "去重键派生规则必须是 `{{turn_id}}-{{role}}`"
    );
    assert_eq!(
        assistant_message.kind,
        Some(HistoryMessageKind::Interrupted),
        "失败/中断轮的合成种类必须落进历史（kind 与文本同判）"
    );

    // Act 2：老用法（不带轮次身份）
    let without_turn_id = store.append_turn(
        Some(session_id),
        "老用法用户提问",
        "老用法回答",
        None,
        Vec::new(),
    );

    // Assert 2：回归 —— 未传 turn_id 时消息仍为 None，且 stable_id 退化 unknown-{role}
    let legacy_history = &without_turn_id.history;
    let legacy_user = &legacy_history[legacy_history.len() - 2];
    let legacy_assistant = &legacy_history[legacy_history.len() - 1];
    assert_eq!(
        legacy_user.turn_id, None,
        "未传 turn_id 的老用法不得凭空获得轮次身份，实际: {:?}",
        legacy_user.turn_id
    );
    assert_eq!(
        legacy_assistant.turn_id, None,
        "未传 turn_id 的老用法不得凭空获得轮次身份，实际: {:?}",
        legacy_assistant.turn_id
    );
    assert_eq!(legacy_user.stable_id(), "unknown-user");
    assert_eq!(legacy_assistant.stable_id(), "unknown-assistant");
}

/// T-A1b（A1-1 幂等键语义）：同一 turn_id 连续两次直连 append 时，**store 层不做替換**
/// （幂等由运行时 gate `turn_outcome_persisted` 的历史腿保证，不在 store 去重）——
/// 实测口径：history 新增 4 条（2×user + 2×assistant）；同时断言去重键派生规则
/// （`{turn_id}-{role}`）使同键可被识别，这正是 gate/回填可据以对账的键。
#[test]
fn t_a1b_same_turn_id_append_semantics_and_dedupe_key() {
    // Arrange
    let mut store = SessionStore::memory_only();
    let session_id = "r2-a1b-session";
    let turn_id = "dup-turn";

    // Act：同一 turn_id 连续两次 append
    let first = store.append_turn_with_kind(
        Some(session_id),
        "第一次用户提问",
        "第一次回答",
        None,
        Vec::new(),
        None,
        Some(turn_id),
    );
    let second = store.append_turn_with_kind(
        Some(session_id),
        "第二次用户提问",
        "第二次回答",
        None,
        Vec::new(),
        None,
        Some(turn_id),
    );

    // Assert 1：第一次追加后该 turn 恰有 2 条（user+assistant）
    let first_turn_count = first
        .history
        .iter()
        .filter(|message| message.turn_id.as_deref() == Some(turn_id))
        .count();
    assert_eq!(
        first_turn_count, 2,
        "单次 append 必须为同一 turn_id 落下 user+assistant 两条，实际: {first_turn_count}"
    );

    // Assert 2：store 层语义是 append（不 REPLACE）—— 两次同 turn_id ⇒ 4 条并列
    let duplicated: Vec<&str> = second
        .history
        .iter()
        .filter(|message| message.turn_id.as_deref() == Some(turn_id))
        .map(|message| message.role.as_str())
        .collect();
    assert_eq!(
        duplicated,
        vec!["user", "assistant", "user", "assistant"],
        "store 层不做同 turn_id 去重（幂等由运行时 gate 的历史腿保证），实际顺序: {duplicated:?}"
    );

    // Assert 3：去重键派生 —— 同一 turn_id 的 (turn_id, role) 键必然撞键（gate 据此对账）
    let stable_ids: Vec<String> = second
        .history
        .iter()
        .filter(|message| message.turn_id.as_deref() == Some(turn_id))
        .map(|message| message.stable_id())
        .collect();
    assert_eq!(
        stable_ids,
        vec![
            "dup-turn-user".to_string(),
            "dup-turn-assistant".to_string(),
            "dup-turn-user".to_string(),
            "dup-turn-assistant".to_string()
        ],
        "去重键必须是 `{{turn_id}}-{{role}}` 派生（同 turn 重复写会撞出同一键），实际: {stable_ids:?}"
    );
}

/// 构造「失败轮」的事件序列：user/message + assistant/chunk + tool/call(+description)
/// + tool/result + turn/end{reason=error, error=Some(..)}。
fn failed_turn_events(turn_id: &str, failure_reason: Option<&str>) -> Vec<(u64, TurnEvent)> {
    vec![
        (1, TurnEvent::TurnStart { turn_id: turn_id.to_string() }),
        (
            2,
            TurnEvent::UserMessage {
                turn_id: turn_id.to_string(),
                text: "帮我探测端口".to_string(),
                attachments: Vec::new(),
            },
        ),
        (
            3,
            TurnEvent::AssistantChunk {
                turn_id: turn_id.to_string(),
                step: 0,
                text: "已做一半".to_string(),
            },
        ),
        (
            4,
            TurnEvent::ToolCall {
                turn_id: turn_id.to_string(),
                step: 0,
                call_id: "call_probe_1".to_string(),
                name: "Run".to_string(),
                arguments: "{\"command\":\"probe\"}".to_string(),
                started_at_ms: None,
                description: Some("探测端口".to_string()),
            },
        ),
        (
            5,
            TurnEvent::ToolResult {
                turn_id: turn_id.to_string(),
                step: 0,
                call_id: "call_probe_1".to_string(),
                result: Some("端口 8080 关闭".to_string()),
                error: None,
                status: None,
                duration_ms: None,
                artifacts: None,
                capability_invocation: None,
            },
        ),
        (
            6,
            TurnEvent::TurnEnd {
                turn_id: turn_id.to_string(),
                reason: TurnEndReason::Error,
                turn_duration_ms: Some(1200),
                error: failure_reason.map(str::to_string),
            },
        ),
    ]
}

/// T-B1a（B1-1）：事件重投影（`projection::fold_all`）产出的失败轮检查点文本必须与
/// 写路径**逐字同源**：经 `failed_turn_checkpoint_text`（含失败原因 + `name + description`
/// 工具行），且「失败原因」行位于「已执行的工具步骤：」之前；
/// 旧日志（`turn/end.error` 缺失）必须逐字退化为既有中断形态（无失败原因行）。
#[test]
fn t_b1a_reprojected_failed_turn_checkpoint_is_same_source_as_write_path() {
    // Arrange
    let turn_id = "turn-failed-1";
    let reason = "工具 `Run` 已连续失败 4 次";
    let events = failed_turn_events(turn_id, Some(reason));

    // Act
    let state = fold_all::<HistoryProjectionState, HistoryProjectionState>(&events);
    let messages = state.messages();

    // Assert 1：失败轮必须折出 user + 合成 assistant（不得只剩悬空 user）
    assert_eq!(
        messages.len(),
        2,
        "失败轮必须折出 user + assistant 两条消息，实际: {:?}",
        messages.iter().map(|m| (&m.role, &m.content)).collect::<Vec<_>>()
    );
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "帮我探测端口");
    assert_eq!(messages[1].role, "assistant");

    // Assert 2：结构化身份（kind/status）与写路径同形
    assert_eq!(
        messages[1].kind,
        Some(HistoryMessageKind::Interrupted),
        "上游失败轮的投影 kind 必须是 Interrupted，实际: {:?}",
        messages[1].kind
    );
    assert_eq!(
        messages[1].status,
        Some(MessageStatus::Error),
        "失败轮投影消息状态必须是 Error，实际: {:?}",
        messages[1].status
    );
    assert_eq!(
        messages[1].turn_id.as_deref(),
        Some(turn_id),
        "投影检查点消息必须携带 turn_id，实际: {:?}",
        messages[1].turn_id
    );

    // Assert 3：与写路径纯函数**逐字同源**（含 name + description 工具行）
    let expected = failed_turn_checkpoint_text("已做一半", &["Run 探测端口".to_string()], reason)
        .expect("非空入参必须产出检查点文本");
    assert_eq!(
        messages[1].content, expected,
        "重投影检查点文本必须与 failed_turn_checkpoint_text 逐字一致（单一真源）"
    );

    // Assert 4：契约要素 —— 失败原因存在、位于工具段之前、工具行含 description
    let text = &messages[1].content;
    assert!(
        text.starts_with("[中断轮次检查点"),
        "投影检查点前缀必须保持，实际: {text:?}"
    );
    assert!(
        text.contains(reason),
        "重投影必须保留失败原因（B1 核心缺陷：不得丢 reason），实际: {text:?}"
    );
    let reason_pos = text.find(REASON_PREFIX).expect("投影文本必须含 `失败原因：`");
    let tools_pos = text.find(TOOLS_HEADER).expect("投影文本必须含工具段");
    assert!(
        reason_pos < tools_pos,
        "投影文本中「失败原因」行必须在工具段之前，reason_pos={reason_pos}, tools_pos={tools_pos}, 实际: {text:?}"
    );
    assert!(
        text.contains("- Run 探测端口"),
        "工具行必须是 `name + description` 同形（写路径同源），实际: {text:?}"
    );

    // Assert 5：旧日志（turn/end.error 缺失）⇒ 逐字退化为既有中断形态，不造失败原因行
    let legacy_events = failed_turn_events(turn_id, None);
    let legacy_state = fold_all::<HistoryProjectionState, HistoryProjectionState>(&legacy_events);
    let legacy_messages = legacy_state.messages();
    assert_eq!(legacy_messages.len(), 2, "旧日志失败轮同样必须折出两条消息");
    let legacy_text = &legacy_messages[1].content;
    let legacy_expected =
        interrupted_turn_checkpoint_text("已做一半", &["Run 探测端口".to_string()])
            .expect("非空入参必须产出中断检查点文本");
    assert_eq!(
        legacy_text, &legacy_expected,
        "旧日志（无 error 键）必须逐字退化为 interrupted_turn_checkpoint_text 形态"
    );
    assert!(
        !legacy_text.contains(REASON_PREFIX),
        "旧日志不得凭空出现「失败原因：」行，实际: {legacy_text:?}"
    );
}
