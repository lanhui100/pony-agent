//! ADR 0099 阶段 1-B：`PersistCommand` → **日志命令流**的编码层。
//!
//! 日志文件是写路径的真源，它保存的不是「快照」而是**命令流**：
//!
//! ```text
//! {"seq":0,"ts":1762500000000,"cmd":{"type":"append_message","epoch":1,…}}
//! ```
//!
//! 每个 `cmd` 自带 `type` 标签（`PersistCommand` 变体的 `snake_case` 名），字段名沿用
//! 本 crate 既有的 JSON 约定（camelCase，与 `TurnHistoryMessage` / `TurnTraceRecord`
//! 等 `#[serde(rename_all = "camelCase")]` 类型一致），因此日志可被后续阶段的 fold
//! 投影无歧义地读回。
//!
//! 为什么不直接给 `PersistCommand` 加 `#[derive(Serialize)]`：`backend.rs` 不在本阶段
//! 的写域内（见 task-5），且命令里混有未实现 `Serialize` 的记录类型
//! （`TurnStateRecord` / `TraceTerminalPatch` / `SessionMetaPatch`）。本模块用一层显式
//! 编码把它们逐字段落盘：**字段一个不漏**（真源不允许静默丢字段），未实现的类型也
//! 不例外。

use serde_json::{json, Value};

use super::backend::{PersistCommand, SessionMetaPatch, TraceTerminalPatch, TurnStateRecord};
use super::types::{HistoryBranch, HistoryCursor, HistoryNode, TurnHistoryMessage, TurnTraceRecord};
use crate::agent::hooks::HookTraceRecord;
use crate::agent::turn_event::TurnEvent;

/// 日志命令信封里的 `cmd` 编码版本（与 `SESSION_LOG_FORMAT_VERSION` 正交：
/// 前者管单行命令编码，后者管文件格式）。
pub const SESSION_LOG_COMMAND_ENCODING_VERSION: u64 = 1;

/// 命令所属会话 id。
///
/// `PublishMetadata` 是**全局元数据**（`key`/`value`，无会话归属），日志按会话分文件，
/// 无处安放 → 返回 `None`（这类命令不进日志，也不假装属于某个会话）。
pub fn command_session_id(command: &PersistCommand) -> Option<&str> {
    match command {
        PersistCommand::AppendMessage { session_id, .. }
        | PersistCommand::UpsertTurn { session_id, .. }
        | PersistCommand::AppendTrace { session_id, .. }
        | PersistCommand::UpdateTraceTerminal { session_id, .. }
        | PersistCommand::AppendHookRecords { session_id, .. }
        | PersistCommand::UpdateHistoryNode { session_id, .. }
        | PersistCommand::UpdateCursor { session_id, .. }
        | PersistCommand::UpdateBranch { session_id, .. }
        | PersistCommand::UpdateSessionMeta { session_id, .. }
        | PersistCommand::RemoveSession { session_id, .. }
        | PersistCommand::FlushEvents { session_id, .. } => Some(session_id.as_str()),
        PersistCommand::PublishMetadata { .. } => None,
    }
}

/// 把一条持久化命令编码成自描述 JSON（`{"type": "<variant>", …}`）。
pub fn command_to_value(command: &PersistCommand) -> Result<Value, String> {
    let encoded = match command {
        PersistCommand::AppendMessage {
            epoch,
            session_id,
            message,
            ordinal,
        } => json!({
            "type": "append_message",
            "encodingVersion": SESSION_LOG_COMMAND_ENCODING_VERSION,
            "epoch": epoch,
            "sessionId": session_id,
            "message": serialize(message)?,
            "ordinal": ordinal,
        }),
        PersistCommand::UpsertTurn {
            epoch,
            session_id,
            turn,
        } => json!({
            "type": "upsert_turn",
            "encodingVersion": SESSION_LOG_COMMAND_ENCODING_VERSION,
            "epoch": epoch,
            "sessionId": session_id,
            "turn": turn_state_to_value(turn),
        }),
        PersistCommand::AppendTrace {
            epoch,
            session_id,
            trace,
            trace_order,
        } => json!({
            "type": "append_trace",
            "encodingVersion": SESSION_LOG_COMMAND_ENCODING_VERSION,
            "epoch": epoch,
            "sessionId": session_id,
            "trace": serialize(trace)?,
            "traceOrder": trace_order,
        }),
        PersistCommand::UpdateTraceTerminal {
            epoch,
            session_id,
            turn_id,
            terminal_patch,
        } => json!({
            "type": "update_trace_terminal",
            "encodingVersion": SESSION_LOG_COMMAND_ENCODING_VERSION,
            "epoch": epoch,
            "sessionId": session_id,
            "turnId": turn_id,
            "terminalPatch": terminal_patch_to_value(terminal_patch),
        }),
        PersistCommand::AppendHookRecords {
            epoch,
            session_id,
            turn_id,
            hook_trace_records,
            updated_at,
        } => json!({
            "type": "append_hook_records",
            "encodingVersion": SESSION_LOG_COMMAND_ENCODING_VERSION,
            "epoch": epoch,
            "sessionId": session_id,
            "turnId": turn_id,
            "hookTraceRecords": serialize(hook_trace_records)?,
            "updatedAt": updated_at,
        }),
        PersistCommand::UpdateHistoryNode {
            epoch,
            session_id,
            node,
        } => json!({
            "type": "update_history_node",
            "encodingVersion": SESSION_LOG_COMMAND_ENCODING_VERSION,
            "epoch": epoch,
            "sessionId": session_id,
            "node": serialize(node)?,
        }),
        PersistCommand::UpdateCursor {
            epoch,
            session_id,
            cursor,
        } => json!({
            "type": "update_cursor",
            "encodingVersion": SESSION_LOG_COMMAND_ENCODING_VERSION,
            "epoch": epoch,
            "sessionId": session_id,
            "cursor": serialize(cursor)?,
        }),
        PersistCommand::UpdateBranch {
            epoch,
            session_id,
            branch,
        } => json!({
            "type": "update_branch",
            "encodingVersion": SESSION_LOG_COMMAND_ENCODING_VERSION,
            "epoch": epoch,
            "sessionId": session_id,
            "branch": serialize(branch)?,
        }),
        PersistCommand::UpdateSessionMeta {
            epoch,
            session_id,
            meta_patch,
        } => json!({
            "type": "update_session_meta",
            "encodingVersion": SESSION_LOG_COMMAND_ENCODING_VERSION,
            "epoch": epoch,
            "sessionId": session_id,
            "metaPatch": session_meta_patch_to_value(meta_patch),
        }),
        PersistCommand::RemoveSession { epoch, session_id } => json!({
            "type": "remove_session",
            "encodingVersion": SESSION_LOG_COMMAND_ENCODING_VERSION,
            "epoch": epoch,
            "sessionId": session_id,
        }),
        PersistCommand::FlushEvents {
            epoch,
            session_id,
            turn_id,
            branch_id,
            events,
        } => json!({
            "type": "flush_events",
            "encodingVersion": SESSION_LOG_COMMAND_ENCODING_VERSION,
            "epoch": epoch,
            "sessionId": session_id,
            "turnId": turn_id,
            "branchId": branch_id,
            "events": serialize(events)?,
        }),
        PersistCommand::PublishMetadata { epoch, key, value } => json!({
            "type": "publish_metadata",
            "encodingVersion": SESSION_LOG_COMMAND_ENCODING_VERSION,
            "epoch": epoch,
            "key": key,
            "value": value,
        }),
    };
    Ok(encoded)
}

fn serialize<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|error| format!("encode command payload: {error}"))
}

/// `TurnStateRecord`（`backend.rs`，未实现 `Serialize`）→ JSON。
fn turn_state_to_value(turn: &TurnStateRecord) -> Value {
    json!({
        "turnId": turn.turn_id,
        "ordinal": turn.ordinal,
        "phase": turn.phase,
        "status": turn.status,
        "userMessageId": turn.user_message_id,
        "assistantMessageId": turn.assistant_message_id,
        "startedAtMs": turn.started_at_ms,
        "completedAtMs": turn.completed_at_ms,
        "updatedAtMs": turn.updated_at_ms,
    })
}

/// `TraceTerminalPatch`（`backend.rs`，未实现 `Serialize`）→ JSON。
fn terminal_patch_to_value(patch: &TraceTerminalPatch) -> Value {
    json!({
        "eventId": patch.event_id,
        "eventType": patch.event_type,
        "eventVersion": patch.event_version,
        "sequence": patch.sequence,
        "emittedAtMs": patch.emitted_at_ms,
        "updatedAt": patch.updated_at,
        "phase": patch.phase,
        "eventWatermark": patch.event_watermark,
    })
}

/// `SessionMetaPatch`（`backend.rs`，未实现 `Serialize`）→ JSON。
///
/// `title_override` 是 `Option<Option<String>>`：「未提供补丁」与「显式清空标题」是
/// 两种不同语义，普通 JSON `null` 表达不了。故落成两个字段：
/// `titleOverrideSet`（本次是否要改这个字段）+ `titleOverride`（改成什么，`null` = 清空）。
fn session_meta_patch_to_value(patch: &SessionMetaPatch) -> Value {
    let (title_override_set, title_override) = match &patch.title_override {
        None => (false, Value::Null),
        Some(None) => (true, Value::Null),
        Some(Some(title)) => (true, Value::String(title.clone())),
    };
    json!({
        "title": patch.title,
        "titleOverrideSet": title_override_set,
        "titleOverride": title_override,
        "archived": patch.archived,
        "summary": patch.summary,
        "turnCount": patch.turn_count,
        "lastReferencedFile": patch.last_referenced_file,
        "updatedAtMs": patch.updated_at_ms,
    })
}

// 这些 `use` 只服务类型完整性检查：编码以泛型 `Serialize` 走 serde，若上述类型
// 的 `Serialize` 实现被移除，本模块会在编译期而不是运行期报错。
const _: fn() = || {
    fn assert_serialize<T: serde::Serialize>() {}
    assert_serialize::<TurnHistoryMessage>();
    assert_serialize::<TurnTraceRecord>();
    assert_serialize::<HistoryNode>();
    assert_serialize::<HistoryCursor>();
    assert_serialize::<HistoryBranch>();
    assert_serialize::<HookTraceRecord>();
    assert_serialize::<TurnEvent>();
};

#[cfg(test)]
mod tests {
    use super::*;

    fn message() -> TurnHistoryMessage {
        TurnHistoryMessage {
            role: "user".to_string(),
            content: "hi".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn append_message_encodes_type_tag_and_payload_without_loss() {
        let value = command_to_value(&PersistCommand::AppendMessage {
            epoch: 1,
            session_id: "pa_x".to_string(),
            message: message(),
            ordinal: 7,
        })
        .expect("encode");
        assert_eq!(value["type"], json!("append_message"));
        assert_eq!(value["sessionId"], json!("pa_x"));
        assert_eq!(value["epoch"], json!(1));
        assert_eq!(value["ordinal"], json!(7));
        // payload 走 serde（camelCase 契约由 `TurnHistoryMessage` 自身保证）。
        assert_eq!(value["message"]["role"], json!("user"));
        assert_eq!(value["message"]["content"], json!("hi"));
    }

    #[test]
    fn session_meta_patch_distinguishes_absent_and_explicit_clear() {
        let absent = command_to_value(&PersistCommand::UpdateSessionMeta {
            epoch: 1,
            session_id: "pa_x".to_string(),
            meta_patch: SessionMetaPatch::default(),
        })
        .expect("encode");
        assert_eq!(absent["metaPatch"]["titleOverrideSet"], json!(false));

        let cleared = command_to_value(&PersistCommand::UpdateSessionMeta {
            epoch: 1,
            session_id: "pa_x".to_string(),
            meta_patch: SessionMetaPatch {
                title_override: Some(None),
                ..Default::default()
            },
        })
        .expect("encode");
        assert_eq!(cleared["metaPatch"]["titleOverrideSet"], json!(true));
        assert!(cleared["metaPatch"]["titleOverride"].is_null());

        let set = command_to_value(&PersistCommand::UpdateSessionMeta {
            epoch: 1,
            session_id: "pa_x".to_string(),
            meta_patch: SessionMetaPatch {
                title_override: Some(Some("t".to_string())),
                ..Default::default()
            },
        })
        .expect("encode");
        assert_eq!(set["metaPatch"]["titleOverride"], json!("t"));
    }

    #[test]
    fn publish_metadata_has_no_session_owner() {
        let command = PersistCommand::PublishMetadata {
            epoch: 1,
            key: "k".to_string(),
            value: "v".to_string(),
        };
        assert!(command_session_id(&command).is_none());
        assert_eq!(
            command_to_value(&command).expect("encode")["type"],
            json!("publish_metadata")
        );
    }

    #[test]
    fn every_variant_carries_a_type_tag() {
        let commands = vec![
            PersistCommand::RemoveSession {
                epoch: 1,
                session_id: "pa_x".to_string(),
            },
            PersistCommand::UpdateCursor {
                epoch: 1,
                session_id: "pa_x".to_string(),
                cursor: HistoryCursor::default(),
            },
            PersistCommand::FlushEvents {
                epoch: 1,
                session_id: "pa_x".to_string(),
                turn_id: "t1".to_string(),
                branch_id: "main".to_string(),
                events: Vec::new(),
            },
            PersistCommand::UpdateTraceTerminal {
                epoch: 1,
                session_id: "pa_x".to_string(),
                turn_id: "t1".to_string(),
                terminal_patch: TraceTerminalPatch {
                    event_id: None,
                    event_type: None,
                    event_version: None,
                    sequence: Some(3),
                    emitted_at_ms: None,
                    updated_at: 9,
                    phase: Some("done".to_string()),
                    event_watermark: None,
                },
            },
        ];
        for command in commands {
            let value = command_to_value(&command).expect("encode");
            assert!(
                value["type"].as_str().is_some_and(|tag| !tag.is_empty()),
                "variant must carry a non-empty type tag"
            );
            assert_eq!(value["sessionId"], json!("pa_x"));
        }
    }
}
