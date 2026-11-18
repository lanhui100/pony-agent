//! B235 / B238 回归（内存 store，不依赖真实磁盘）：
//!
//! - B235：宿主续轮落库后历史行**不再以 `user` 角色出现**（`host` 角色），
//!   且 `turn_count` / 标题 / `needs_ai_title` 不被污染。
//! - B238：B232 的「【挂起前用户指令（宿主补录）】+ 宿主续轮」组合正文（旧日志里仍是
//!   `user` 行）被派生侧识别为续轮：标题不再取段标、`continuation_only` 仍成立。
//! - 旧日志（无新字段/旧角色）回放不回归。

use super::host_resume::{HOST_HISTORY_ROLE, RESUME_INSTRUCTION_MARKER, RESUME_PROMPT_MARKER};
use super::store::{
    is_host_continuation_prompt, refresh_session_metadata, session_is_continuation_fragment,
    SessionStore, APPROVAL_APPROVED_CONTINUATION_PROMPT,
};
use super::types::{TurnHistoryMessage, DEFAULT_SESSION_TITLE};
use super::DEFAULT_SESSION_ID;

/// 现场原话（截自 `pa_01a1e905` 的 `turn-45f905c5` user/message 事件）。
const SITE_INSTRUCTION: &str = "本轮重点处理ACP相关事项包括以入账事项，dev team实施p0-p2以及入账的B164的修复。";

fn composed_continuation() -> String {
    super::host_resume::compose_host_continuation(SITE_INSTRUCTION, APPROVAL_APPROVED_CONTINUATION_PROMPT)
}

fn user_message(content: &str) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: "user".to_string(),
        content: content.to_string(),
        ..TurnHistoryMessage::default()
    }
}

fn assistant_message(content: &str) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: "assistant".to_string(),
        content: content.to_string(),
        ..TurnHistoryMessage::default()
    }
}

#[test]
fn host_continuation_predicate_recognizes_composed_body() {
    // B238：组合正文两个段标齐全 ⇒ 命中（旧日志里这条是 user 行）。
    assert!(is_host_continuation_prompt(&composed_continuation()));
    // 原指令内容任意（含段标字符之外的多行文本）仍按结构识别。
    assert!(is_host_continuation_prompt(&format!(
        "{RESUME_INSTRUCTION_MARKER}\n多行\n指令\n\n{RESUME_PROMPT_MARKER}\n继续"
    )));
    // 反例：只有原指令段标（缺宿主续轮段标）或段标只在中间 ⇒ 不是宿主续轮。
    assert!(!is_host_continuation_prompt(RESUME_INSTRUCTION_MARKER));
    assert!(!is_host_continuation_prompt(&format!(
        "用户说：{RESUME_INSTRUCTION_MARKER}\n{RESUME_PROMPT_MARKER}"
    )));
}

#[test]
fn b235_host_continuation_row_is_persisted_with_host_role_not_user() {
    let mut store = SessionStore::memory_only();
    let session_id = DEFAULT_SESSION_ID.to_string();
    store.append_turn(
        Some(&session_id),
        &composed_continuation(),
        "好的",
        None,
        Vec::new(),
    );

    let session = store.sessions.get(&session_id).expect("默认会话应存在");
    let row = session
        .history
        .iter()
        .find(|message| message.content.contains(SITE_INSTRUCTION))
        .expect("续轮消息必须落库");
    assert_eq!(
        row.role, HOST_HISTORY_ROLE,
        "B235：宿主续轮必须以 host 角色落库，实际 role={:?}",
        row.role
    );
    assert!(
        session
            .history
            .iter()
            .all(|message| !(message.role == "user"
                && message.content.contains(SITE_INSTRUCTION))),
        "B235：宿主续轮不得再占 user 槽，实际历史 = {:?}",
        session
            .history
            .iter()
            .map(|message| &message.role)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        session.turn_count, 0,
        "B238：宿主续轮不计入 turn_count，实际 = {}",
        session.turn_count
    );
    assert_eq!(
        session.title, DEFAULT_SESSION_TITLE,
        "B238：标题不得取宿主续轮正文（也不得取段标）"
    );
    // B238：只有宿主续轮的会话是碎片（不进 /resume）。
    assert!(
        session_is_continuation_fragment(session),
        "B238：continuation_only 必须成立（碎片会话不得进 /resume）"
    );
    assert!(
        !store.needs_ai_title(&session_id),
        "B238：宿主续轮不得触发 AI 标题（needs_ai_title 必须为 false）"
    );
}

#[test]
fn b238_legacy_user_composed_row_does_not_pollute_derivations() {
    // 旧日志：B232 写入的组合正文是 user 行（无新字段）——回放仍须派生正确。
    let mut store = SessionStore::memory_only();
    let session_id = DEFAULT_SESSION_ID.to_string();
    {
        let session = store.sessions.get_mut(&session_id).expect("默认会话应存在");
        session.turn_trace_history.clear();
        session.history = vec![
            user_message(&composed_continuation()),
            assistant_message("已按批准继续执行"),
        ];
        refresh_session_metadata(session, false);
    }
    let session = store.sessions.get(&session_id).expect("默认会话应存在");
    assert_eq!(
        session.title, DEFAULT_SESSION_TITLE,
        "B238：旧日志的组合行不得成为标题（段标/原指令都不得直出），实际 = {:?}",
        session.title
    );
    assert_eq!(session.turn_count, 0, "B238：旧日志组合行不计入 turn_count");
    assert!(
        session_is_continuation_fragment(session),
        "B238：旧日志只有组合行的会话必须判为 continuation_only"
    );
    assert!(
        !store.needs_ai_title(&session_id),
        "B238：旧日志组合行不得触发 AI 标题"
    );
}

#[test]
fn b238_legacy_real_user_row_still_derives_title_and_turn_count() {
    // 旧日志（普通真实用户轮）：新判据不得误伤 ⇒ 回归不破坏既有派生。
    let mut store = SessionStore::memory_only();
    let session_id = DEFAULT_SESSION_ID.to_string();
    {
        let session = store.sessions.get_mut(&session_id).expect("默认会话应存在");
        session.turn_trace_history.clear();
        session.history = vec![
            user_message("请修一下会话标题"),
            assistant_message("好的"),
        ];
        refresh_session_metadata(session, false);
    }
    let session = store.sessions.get(&session_id).expect("默认会话应存在");
    assert_eq!(session.title, "请修一下会话标题");
    assert_eq!(session.turn_count, 1);
    assert!(!session_is_continuation_fragment(session));
}
