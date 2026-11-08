//! ADR 0099 **阶段 2-A**：`fold` 状态重建器 + SQLite 索引重建。
//!
//! 阶段 1 已把**写路径真源**切到每会话一份 append-only 日志文件
//! （`<db 目录>/session-logs/--<cwd>--/<session-id>/session.jsonl`），日志行是
//! `PersistCommand` 的**命令流**而非状态快照。本模块交付其读侧对偶：
//!
//! ```text
//! 日志命令流（LogEvent） --fold_session()--> SessionState --rebuild_index()--> SQLite 索引
//! ```
//!
//! ## 三条硬约束（阶段 2-A 验收口径）
//!
//! 1. **纯函数**：[`fold_session`] 不读盘、不写库、**不读时钟**、不碰任何全局状态。
//!    输入相同事件序列必得相同 `SessionState`；所有时间戳只来自日志里的 `ts` /
//!    命令 payload（`updatedAtMs` 等），绝不调用 `now()`。因此可 off-thread、可重放、
//!    可对拍。
//! 2. **与索引侧逐字段对齐**：fold 的落位/去重/upsert 语义逐条对齐
//!    `sqlite_session.rs::apply_persist_command_tx`（live 写路径的索引语义）：
//!    - `AppendMessage`：`message_id = {turn_id}-{role}`（`turn_id` 缺失时
//!      `msg-{ordinal}-{role}`）去重（`INSERT OR REPLACE`），最终 history 按 `ordinal`
//!      升序（同 ordinal 按到达顺序）；
//!    - `UpdateSessionMeta`：`Option<Option<T>>` 三态（`None` 不改 / `Some(None)` 清空 /
//!      `Some(Some(v))` 赋值），其余字段 `COALESCE` 语义（`None` 保持原值）；
//!    - `UpdateHistoryNode` / `UpdateCursor` / `UpdateBranch` / `UpsertTurn`：upsert；
//!    - 会话级派生字段（`event_watermark` / `last_commit_watermark`）取自 cursor 行，
//!      与 `load_store` 的取值口径一致。
//! 3. **未知/坏命令 fail loud**：任何不认识的 `cmd.type`、坏信封、缺字段、会话归属
//!    不一致一律返回 [`FoldError`]，**绝不静默跳过**（静默跳过 = 假绿）。
//!
//! ## 为什么 fold 的结果不是裸 `SessionState`
//!
//! `RemoveSession` 的折叠结果是「已删除」，而 `SessionState` 没有墓碑字段
//! （`types.rs` 不在本阶段写域内，也不该为读侧投影加字段）。故 [`FoldedSession`]
//! 把「状态 + 是否已删除」一并返回：删除语义是第一等结果，不是错误、也不该被猜。
//!
//! ## 索引重建的两条安全边界
//!
//! - [`rebuild_index`] **只覆盖「有日志文件」的会话**；索引里有、日志里没有的会话
//!   （存量库的绝大多数）**原封不动**，只计入 `skipped_no_log`。这一条是硬约束：
//!   把「没有日志」误当成「空会话」会一次性清空真实历史。
//! - `dry_run` 时不写库（CLI 入口用 `mode=ro` 打开，物理上零写入）。

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use serde::de::DeserializeOwned;
use serde_json::Value;

use super::backend::{PersistCommand, SessionBackend, SessionMetaPatch, TraceTerminalPatch, TurnStateRecord};
use super::log_store::{SessionLogMeta, SessionLogStore};
use super::types::{
    HistoryBranch, HistoryCursor, HistoryNode, SessionState, TurnHistoryMessage, TurnTraceRecord,
};
use super::types::{DEFAULT_SESSION_SUMMARY, DEFAULT_SESSION_TITLE};
use crate::agent::hooks::HookTraceRecord;
use crate::agent::sqlite_session::SqliteSessionBackend;
use crate::agent::turn_event::TurnEvent;

// ---------------------------------------------------------------------------
// 错误
// ---------------------------------------------------------------------------

/// fold 失败：**每一条都是显式失败**，没有「跳过坏行继续」的旁路。
#[derive(Debug)]
pub enum FoldError {
    /// 信封不是合法 JSON / 缺 `seq`、`cmd`。
    MalformedEnvelope { reason: String },
    /// `seq` 不再单调（日志损坏 / 拼接了别的文件）——继续 fold 会把乱序当真相。
    NonMonotonicSeq { previous: u64, found: u64 },
    /// 不认识的 `cmd.type`：**必须报错**，静默跳过等于伪造结果。
    UnknownCommand { seq: u64, kind: String },
    /// 认识的命令但字段缺失 / 类型不对。
    MalformedCommand {
        seq: u64,
        kind: String,
        reason: String,
    },
    /// 一个日志文件里出现了别的会话的命令（跨会话污染）。
    SessionMismatch {
        seq: u64,
        expected: String,
        found: String,
    },
    /// 会话日志不存在（或 header 不是 `current`，无法 fold）。
    SessionNotFound { session_id: String, reason: String },
    /// 读日志文件失败。
    Io { context: String, source: std::io::Error },
}

impl std::fmt::Display for FoldError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MalformedEnvelope { reason } => write!(formatter, "malformed log envelope: {reason}"),
            Self::NonMonotonicSeq { previous, found } => write!(
                formatter,
                "log seq is not monotonic: found {found} after {previous}"
            ),
            Self::UnknownCommand { seq, kind } => {
                write!(formatter, "unknown persist command `{kind}` at seq {seq}")
            }
            Self::MalformedCommand { seq, kind, reason } => {
                write!(formatter, "malformed `{kind}` command at seq {seq}: {reason}")
            }
            Self::SessionMismatch {
                seq,
                expected,
                found,
            } => write!(
                formatter,
                "command at seq {seq} belongs to session `{found}`, expected `{expected}`"
            ),
            Self::SessionNotFound {
                session_id,
                reason,
            } => write!(formatter, "session log not found for `{session_id}`: {reason}"),
            Self::Io { context, source } => write!(formatter, "{context}: {source}"),
        }
    }
}

impl std::error::Error for FoldError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// 日志事件（命令信封）
// ---------------------------------------------------------------------------

/// 日志里的一条**命令信封**：`{"seq":N,"ts":ms,"cmd":{...}}`。
///
/// `cmd` 保持为不透明 JSON：解码放在 [`fold_session`] 里做，这样「坏命令」与
/// 「未知命令」都变成 [`FoldError`] 而不是读侧的静默丢弃。
#[derive(Clone, Debug, PartialEq)]
pub struct LogEvent {
    /// 该文件内单调的序号（写者分配，从 0 起连续）。
    pub seq: u64,
    /// 事件时间戳（Unix 毫秒），由写者落盘时写入。
    pub ts: i64,
    /// `PersistCommand` 的自描述 JSON（含 `type` 标签）。
    pub cmd: Value,
}

impl LogEvent {
    /// 从单行 JSON 解析信封。`seq` / `cmd` 缺失即错（头部行由调用方跳过）。
    pub fn from_json_line(line: &str) -> Result<Self, FoldError> {
        let value: Value = serde_json::from_str(line).map_err(|error| FoldError::MalformedEnvelope {
            reason: format!("not valid JSON ({error}): {line}"),
        })?;
        let seq = value
            .get("seq")
            .and_then(Value::as_u64)
            .ok_or_else(|| FoldError::MalformedEnvelope {
                reason: format!("missing integer `seq`: {line}"),
            })?;
        let cmd = value
            .get("cmd")
            .cloned()
            .ok_or_else(|| FoldError::MalformedEnvelope {
                reason: format!("missing `cmd` (command envelope expected): {line}"),
            })?;
        Ok(Self {
            seq,
            ts: value.get("ts").and_then(Value::as_i64).unwrap_or(0),
            cmd,
        })
    }
}

// ---------------------------------------------------------------------------
// 读日志（不越过撕裂尾行）
// ---------------------------------------------------------------------------

/// 读某会话日志文件的**全部命令信封**（跳过 header 行）。
///
/// 撕裂尾行语义与 [`super::log_store::LogReader`] 完全一致：没有换行结束符的最后
/// 一行、或解析失败的最后一行 ⇒ 该行及之后都不是已提交内容，直接丢弃（不 panic、
/// 不报错）。命令信封与 `SessionLogEvent` 是两种已提交行形态，`LogReader` 只认后者，
/// 故这里按信封形态读。
pub fn read_log_events(
    store: &SessionLogStore,
    meta: &SessionLogMeta,
) -> Result<Vec<LogEvent>, FoldError> {
    let path = store
        .log_path(&meta.session_id, &meta.cwd)
        .map_err(|error| FoldError::SessionNotFound {
            session_id: meta.session_id.clone(),
            reason: error.to_string(),
        })?;
    read_log_events_from_path(&path)
}

fn read_log_events_from_path(path: &Path) -> Result<Vec<LogEvent>, FoldError> {
    let raw = std::fs::read_to_string(path).map_err(|error| FoldError::Io {
        context: format!("read session log `{}`", path.display()),
        source: error,
    })?;
    let mut events = Vec::new();
    let mut previous_seq: Option<u64> = None;
    for (index, line) in raw.split_inclusive('\n').enumerate() {
        // header 行（第 0 行）不是命令信封；但它必须**确实是** session header，
        // 否则说明这不是会话日志（例如把 `events.jsonl` 之类的东西放进来了）。
        if index == 0 {
            let trimmed = line.trim();
            let is_header = serde_json::from_str::<Value>(trimmed)
                .map(|value| value.get("type").and_then(Value::as_str) == Some("session"))
                .unwrap_or(false);
            if !is_header {
                return Err(FoldError::MalformedEnvelope {
                    reason: format!(
                        "`{}` does not start with a session header: {trimmed}",
                        path.display()
                    ),
                });
            }
            continue;
        }
        // 撕裂尾行：没有换行结束符 ⇒ 该行及之后都不是已提交内容。
        if !line.ends_with('\n') {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let event = match LogEvent::from_json_line(trimmed) {
            Ok(event) => event,
            // 解析失败：与 LogReader 同口径——视为撕裂尾行，丢弃该行及之后。
            Err(_) => break,
        };
        match previous_seq {
            Some(previous) if event.seq <= previous => {
                return Err(FoldError::NonMonotonicSeq {
                    previous,
                    found: event.seq,
                })
            }
            _ => previous_seq = Some(event.seq),
        }
        events.push(event);
    }
    Ok(events)
}

// ---------------------------------------------------------------------------
// fold 结果
// ---------------------------------------------------------------------------

/// fold 的产物：可折叠出的会话状态 + 墓碑标记。
#[derive(Clone, Debug)]
pub struct FoldedSession {
    /// 重建出的会话状态（`deleted` 为真时是墓碑之前的最后状态，仅供诊断）。
    pub state: SessionState,
    /// 出现 `RemoveSession` ⇒ 该会话已被删除（D3 墓碑语义：fold 视其为不存在）。
    pub deleted: bool,
    /// `FlushEvents` 折叠出的事件行数（`turn_events` 不属于 `SessionState`，
    /// 这里只保留计数供重建报告/校验，不假装它是状态的一部分）。
    pub event_rows: usize,
    /// `UpsertTurn` 折叠出的 turn 行（`normalized_turns` 的投影源，按 turn_id upsert）。
    pub turns: Vec<TurnStateRecord>,
}

// ---------------------------------------------------------------------------
// 命令解码
// ---------------------------------------------------------------------------

/// 解码后的命令（只保留折叠需要的字段；字段名与 `log_command.rs` 的编码逐一对齐）。
enum FoldCommand {
    AppendMessage {
        session_id: String,
        message: TurnHistoryMessage,
        ordinal: usize,
    },
    UpsertTurn {
        session_id: String,
        turn: TurnStateRecord,
    },
    AppendTrace {
        session_id: String,
        trace: TurnTraceRecord,
        trace_order: usize,
    },
    UpdateTraceTerminal {
        session_id: String,
        turn_id: String,
        terminal_patch: TraceTerminalPatch,
    },
    AppendHookRecords {
        session_id: String,
        turn_id: String,
        hook_trace_records: Vec<HookTraceRecord>,
        updated_at: u64,
    },
    UpdateHistoryNode {
        session_id: String,
        node: HistoryNode,
    },
    UpdateCursor {
        session_id: String,
        cursor: HistoryCursor,
    },
    UpdateBranch {
        session_id: String,
        branch: HistoryBranch,
    },
    UpdateSessionMeta {
        session_id: String,
        meta_patch: SessionMetaPatch,
    },
    RemoveSession {
        session_id: String,
    },
    FlushEvents {
        session_id: String,
        events: Vec<TurnEvent>,
    },
    /// 全局元数据（无会话归属）：**不属于**会话状态，fold 中按 N/A 跳过。
    PublishMetadata,
}

fn malformed(seq: u64, kind: &str, reason: impl Into<String>) -> FoldError {
    FoldError::MalformedCommand {
        seq,
        kind: kind.to_string(),
        reason: reason.into(),
    }
}

fn required_str<'a>(value: &'a Value, seq: u64, kind: &str, field: &str) -> Result<&'a str, FoldError> {
    value.get(field).and_then(Value::as_str).ok_or_else(|| {
        malformed(
            seq,
            kind,
            format!("missing string field `{field}`: {value}"),
        )
    })
}

fn optional_str(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_string)
}

fn optional_u64(value: &Value, field: &str) -> Option<u64> {
    value.get(field).and_then(Value::as_u64)
}

fn required_u64(value: &Value, seq: u64, kind: &str, field: &str) -> Result<u64, FoldError> {
    optional_u64(value, field).ok_or_else(|| {
        malformed(
            seq,
            kind,
            format!("missing unsigned integer field `{field}`: {value}"),
        )
    })
}

fn payload<T: DeserializeOwned>(
    value: &Value,
    seq: u64,
    kind: &str,
    field: &str,
) -> Result<T, FoldError> {
    let raw = value
        .get(field)
        .ok_or_else(|| malformed(seq, kind, format!("missing payload field `{field}`")))?;
    serde_json::from_value(raw.clone())
        .map_err(|error| malformed(seq, kind, format!("field `{field}`: {error}")))
}

/// `TurnStateRecord`（`backend.rs` 未实现 `Deserialize`）的解码。
fn decode_turn(value: &Value, seq: u64, kind: &str) -> Result<TurnStateRecord, FoldError> {
    Ok(TurnStateRecord {
        turn_id: required_str(value, seq, kind, "turnId")?.to_string(),
        ordinal: optional_u64(value, "ordinal").unwrap_or(0) as usize,
        phase: optional_str(value, "phase"),
        status: optional_str(value, "status"),
        user_message_id: optional_str(value, "userMessageId"),
        assistant_message_id: optional_str(value, "assistantMessageId"),
        started_at_ms: optional_u64(value, "startedAtMs"),
        completed_at_ms: optional_u64(value, "completedAtMs"),
        updated_at_ms: optional_u64(value, "updatedAtMs").unwrap_or(0),
    })
}

/// `TraceTerminalPatch`（`backend.rs` 未实现 `Deserialize`）的解码。
fn decode_terminal_patch(
    value: &Value,
    seq: u64,
    kind: &str,
) -> Result<TraceTerminalPatch, FoldError> {
    Ok(TraceTerminalPatch {
        event_id: optional_str(value, "eventId"),
        event_type: optional_str(value, "eventType"),
        event_version: optional_str(value, "eventVersion"),
        sequence: optional_u64(value, "sequence"),
        emitted_at_ms: optional_u64(value, "emittedAtMs"),
        updated_at: required_u64(value, seq, kind, "updatedAt")?,
        phase: optional_str(value, "phase"),
        event_watermark: optional_u64(value, "eventWatermark"),
    })
}

/// `SessionMetaPatch`（`backend.rs` 未实现 `Deserialize`）的解码。
///
/// `title_override` 的**三态**由 `titleOverrideSet` + `titleOverride` 两个字段承载
/// （与 `log_command.rs::session_meta_patch_to_value` 的编码一一对应）：
/// `titleOverrideSet=false` ⇒ `None`（不改）；`true` + `null` ⇒ `Some(None)`（清空）；
/// `true` + 字符串 ⇒ `Some(Some(v))`。
fn decode_meta_patch(
    value: &Value,
    seq: u64,
    kind: &str,
) -> Result<SessionMetaPatch, FoldError> {
    let title_override_set = value
        .get("titleOverrideSet")
        .and_then(Value::as_bool)
        .ok_or_else(|| malformed(seq, kind, "missing boolean field `titleOverrideSet`"))?;
    let title_override = if title_override_set {
        Some(optional_str(value, "titleOverride"))
    } else {
        None
    };
    Ok(SessionMetaPatch {
        title: optional_str(value, "title"),
        title_override,
        archived: value.get("archived").and_then(Value::as_bool),
        summary: optional_str(value, "summary"),
        turn_count: optional_u64(value, "turnCount").map(|count| count as usize),
        last_referenced_file: optional_str(value, "lastReferencedFile"),
        updated_at_ms: optional_u64(value, "updatedAtMs"),
    })
}

impl FoldCommand {
    fn decode(event: &LogEvent) -> Result<Self, FoldError> {
        let seq = event.seq;
        let kind = event.cmd.get("type").and_then(Value::as_str).ok_or({
            FoldError::MalformedEnvelope {
                reason: format!("`cmd` at seq {seq} has no string `type` tag"),
            }
        })?;
        let value = &event.cmd;
        let command = match kind {
            "append_message" => Self::AppendMessage {
                session_id: required_str(value, seq, kind, "sessionId")?.to_string(),
                message: payload(value, seq, kind, "message")?,
                ordinal: required_u64(value, seq, kind, "ordinal")? as usize,
            },
            "upsert_turn" => {
                let turn_value = value
                    .get("turn")
                    .ok_or_else(|| malformed(seq, kind, "missing payload field `turn`"))?;
                Self::UpsertTurn {
                    session_id: required_str(value, seq, kind, "sessionId")?.to_string(),
                    turn: decode_turn(turn_value, seq, kind)?,
                }
            }
            "append_trace" => Self::AppendTrace {
                session_id: required_str(value, seq, kind, "sessionId")?.to_string(),
                trace: payload(value, seq, kind, "trace")?,
                trace_order: required_u64(value, seq, kind, "traceOrder")? as usize,
            },
            "update_trace_terminal" => {
                let patch_value = value
                    .get("terminalPatch")
                    .ok_or_else(|| malformed(seq, kind, "missing payload field `terminalPatch`"))?;
                Self::UpdateTraceTerminal {
                    session_id: required_str(value, seq, kind, "sessionId")?.to_string(),
                    turn_id: required_str(value, seq, kind, "turnId")?.to_string(),
                    terminal_patch: decode_terminal_patch(patch_value, seq, kind)?,
                }
            }
            "append_hook_records" => Self::AppendHookRecords {
                session_id: required_str(value, seq, kind, "sessionId")?.to_string(),
                turn_id: required_str(value, seq, kind, "turnId")?.to_string(),
                hook_trace_records: payload(value, seq, kind, "hookTraceRecords")?,
                updated_at: required_u64(value, seq, kind, "updatedAt")?,
            },
            "update_history_node" => Self::UpdateHistoryNode {
                session_id: required_str(value, seq, kind, "sessionId")?.to_string(),
                node: payload(value, seq, kind, "node")?,
            },
            "update_cursor" => Self::UpdateCursor {
                session_id: required_str(value, seq, kind, "sessionId")?.to_string(),
                cursor: payload(value, seq, kind, "cursor")?,
            },
            "update_branch" => Self::UpdateBranch {
                session_id: required_str(value, seq, kind, "sessionId")?.to_string(),
                branch: payload(value, seq, kind, "branch")?,
            },
            "update_session_meta" => {
                let patch_value = value
                    .get("metaPatch")
                    .ok_or_else(|| malformed(seq, kind, "missing payload field `metaPatch`"))?;
                Self::UpdateSessionMeta {
                    session_id: required_str(value, seq, kind, "sessionId")?.to_string(),
                    meta_patch: decode_meta_patch(patch_value, seq, kind)?,
                }
            }
            "remove_session" => Self::RemoveSession {
                session_id: required_str(value, seq, kind, "sessionId")?.to_string(),
            },
            "flush_events" => Self::FlushEvents {
                session_id: required_str(value, seq, kind, "sessionId")?.to_string(),
                events: payload(value, seq, kind, "events")?,
            },
            "publish_metadata" => Self::PublishMetadata,
            other => {
                return Err(FoldError::UnknownCommand {
                    seq,
                    kind: other.to_string(),
                })
            }
        };
        Ok(command)
    }

    fn session_id(&self) -> Option<&str> {
        match self {
            Self::AppendMessage { session_id, .. }
            | Self::UpsertTurn { session_id, .. }
            | Self::AppendTrace { session_id, .. }
            | Self::UpdateTraceTerminal { session_id, .. }
            | Self::AppendHookRecords { session_id, .. }
            | Self::UpdateHistoryNode { session_id, .. }
            | Self::UpdateCursor { session_id, .. }
            | Self::UpdateBranch { session_id, .. }
            | Self::UpdateSessionMeta { session_id, .. }
            | Self::RemoveSession { session_id }
            | Self::FlushEvents { session_id, .. } => Some(session_id),
            Self::PublishMetadata => None,
        }
    }
}

// ---------------------------------------------------------------------------
// fold（纯函数）
// ---------------------------------------------------------------------------

/// 一条消息的折叠中间态（`message_id` 是去重键，`sequence` 只用于同 ordinal 的稳定排序）。
struct MessageRow {
    message_id: String,
    ordinal: usize,
    sequence: usize,
    message: TurnHistoryMessage,
}

/// 一条 trace 的折叠中间态（`turn_id` 是去重键，排序口径对齐索引的
/// `ORDER BY trace_order ASC, updated_at_ms ASC, turn_id ASC`）。
struct TraceRow {
    turn_id: String,
    trace_order: usize,
    trace: TurnTraceRecord,
}

/// 折叠累加器（全部字段都是内存状态：不读盘、不读时钟、不读全局）。
#[derive(Default)]
struct Folder {
    session_id: Option<String>,
    deleted: bool,
    state: Option<SessionState>,
    messages: Vec<MessageRow>,
    traces: Vec<TraceRow>,
    turns: Vec<TurnStateRecord>,
    nodes: Vec<HistoryNode>,
    branches: Vec<HistoryBranch>,
    cursor: Option<HistoryCursor>,
    event_rows: usize,
    sequence: usize,
}

impl Folder {
    fn state(&mut self) -> &mut SessionState {
        self.state.get_or_insert_with(|| SessionState {
            conversation_id: String::new(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            summary: DEFAULT_SESSION_SUMMARY.to_string(),
            history: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: Vec::new(),
            trace_migration_state: Default::default(),
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 0,
            last_referenced_file: None,
            updated_at_ms: 0,
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: HistoryCursor::default(),
            event_watermark: 0,
            last_commit_watermark: 0,
            workspace_id: None,
            title_override: None,
            archived: false,
        })
    }

    fn claim_session(&mut self, seq: u64, session_id: &str) -> Result<(), FoldError> {
        match &self.session_id {
            None => {
                self.session_id = Some(session_id.to_string());
                self.state().conversation_id = session_id.to_string();
                Ok(())
            }
            Some(expected) if expected == session_id => Ok(()),
            Some(expected) => Err(FoldError::SessionMismatch {
                seq,
                expected: expected.clone(),
                found: session_id.to_string(),
            }),
        }
    }

    fn apply(&mut self, seq: u64, command: FoldCommand) -> Result<(), FoldError> {
        if let Some(session_id) = command.session_id() {
            self.claim_session(seq, session_id)?;
        }
        match command {
            FoldCommand::AppendMessage {
                message, ordinal, ..
            } => {
                // 与索引侧 `INSERT OR REPLACE INTO normalized_messages` 同一条去重键。
                let message_id = match &message.turn_id {
                    Some(turn_id) if !turn_id.is_empty() => format!("{turn_id}-{}", message.role),
                    _ => format!("msg-{ordinal}-{}", message.role),
                };
                if let Some(existing) = self
                    .messages
                    .iter_mut()
                    .find(|row| row.message_id == message_id)
                {
                    existing.ordinal = ordinal;
                    existing.message = message;
                } else {
                    self.sequence += 1;
                    self.messages.push(MessageRow {
                        message_id,
                        ordinal,
                        sequence: self.sequence,
                        message,
                    });
                }
            }
            FoldCommand::UpsertTurn { turn, .. } => {
                // `INSERT OR REPLACE`：删旧插新 ⇒ 位置移到末尾（与索引行序一致）。
                self.turns.retain(|existing| existing.turn_id != turn.turn_id);
                self.turns.push(turn);
            }
            FoldCommand::AppendTrace {
                trace, trace_order, ..
            } => {
                let turn_id = trace.turn_id.clone();
                self.traces.retain(|row| row.turn_id != turn_id);
                self.traces.push(TraceRow {
                    turn_id,
                    trace_order,
                    trace,
                });
            }
            FoldCommand::UpdateTraceTerminal {
                turn_id,
                terminal_patch,
                ..
            } => {
                // 与索引侧 `UPDATE … WHERE session_id = ?1 AND turn_id = ?2` 同口径：
                // 行不存在时是 no-op（不凭空造 trace 行）；存在的行按补丁三态覆盖。
                if let Some(row) = self.traces.iter_mut().find(|row| row.turn_id == turn_id) {
                    let trace = &mut row.trace;
                    if let Some(event_id) = terminal_patch.event_id {
                        trace.event_id = Some(event_id);
                    }
                    if let Some(event_type) = terminal_patch.event_type {
                        trace.event_type = Some(event_type);
                    }
                    if let Some(event_version) = terminal_patch.event_version {
                        trace.event_version = Some(event_version);
                    }
                    if let Some(sequence) = terminal_patch.sequence {
                        trace.sequence = Some(sequence);
                    }
                    if let Some(emitted_at_ms) = terminal_patch.emitted_at_ms {
                        trace.emitted_at_ms = Some(emitted_at_ms);
                    }
                    if let Some(phase) = terminal_patch.phase {
                        trace.phase = phase;
                    }
                    trace.updated_at = terminal_patch.updated_at;
                }
            }
            FoldCommand::AppendHookRecords {
                turn_id,
                hook_trace_records,
                updated_at,
                ..
            } => {
                // 命令名即语义：**追加**（不是替换）。索引侧把 hook 记录塞进
                // `extension_json` / legacy `trace_data` 补丁里，读侧 schema 不同；
                // fold 按命令流的语义追加，属两边口径的已知差异（见模块文档）。
                if let Some(row) = self.traces.iter_mut().find(|row| row.turn_id == turn_id) {
                    row.trace
                        .hook_trace_records
                        .extend(hook_trace_records.into_iter());
                    row.trace.updated_at = updated_at;
                }
            }
            FoldCommand::UpdateHistoryNode { node, .. } => {
                self.nodes.retain(|existing| existing.node_id != node.node_id);
                self.nodes.push(node);
            }
            FoldCommand::UpdateCursor { cursor, .. } => {
                self.cursor = Some(cursor);
            }
            FoldCommand::UpdateBranch { branch, .. } => {
                self.branches
                    .retain(|existing| existing.branch_id != branch.branch_id);
                self.branches.push(branch);
            }
            FoldCommand::UpdateSessionMeta { meta_patch, .. } => {
                let state = self.state();
                // patch 语义：`None` = 不改（索引侧 COALESCE），`Some(None)` = 清空。
                if let Some(title) = meta_patch.title {
                    state.title = title;
                }
                if let Some(title_override) = meta_patch.title_override {
                    state.title_override = title_override;
                }
                if let Some(archived) = meta_patch.archived {
                    state.archived = archived;
                }
                if let Some(summary) = meta_patch.summary {
                    state.summary = summary;
                }
                if let Some(turn_count) = meta_patch.turn_count {
                    state.turn_count = turn_count;
                }
                if let Some(last_referenced_file) = meta_patch.last_referenced_file {
                    state.last_referenced_file = Some(last_referenced_file);
                }
                if let Some(updated_at_ms) = meta_patch.updated_at_ms {
                    state.updated_at_ms = updated_at_ms;
                }
            }
            FoldCommand::RemoveSession { .. } => {
                // D3 墓碑：fold 结果为「已删除」。删除是幂等的终态。
                self.deleted = true;
            }
            FoldCommand::FlushEvents { events, .. } => {
                // `turn_events` 是独立表的派生视图，不属于 `SessionState`；
                // 这里只折叠计数（重建报告/校验用），不假装它是会话状态的一部分。
                self.event_rows += events.len();
            }
            FoldCommand::PublishMetadata => {
                // 全局元数据（key/value，无会话归属）不属于会话状态，也不该出现在
                // 会话日志里（写侧 `command_session_id` 返回 None 时跳过）；若出现，
                // fold 按 N/A 处理，不污染会话状态、也不报错。
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<FoldedSession, FoldError> {
        let session_id = self.session_id.clone().unwrap_or_default();
        let mut messages = std::mem::take(&mut self.messages);
        messages.sort_by_key(|row| (row.ordinal, row.sequence));
        let history: Vec<TurnHistoryMessage> = messages.into_iter().map(|row| row.message).collect();

        let mut traces = std::mem::take(&mut self.traces);
        traces.sort_by(|left, right| {
            left.trace_order
                .cmp(&right.trace_order)
                .then(left.trace.updated_at.cmp(&right.trace.updated_at))
                .then(left.turn_id.cmp(&right.turn_id))
        });
        let turn_trace_history: Vec<TurnTraceRecord> =
            traces.into_iter().map(|row| row.trace).collect();
        let by_turn: HashMap<&str, &TurnTraceRecord> = turn_trace_history
            .iter()
            .map(|trace| (trace.turn_id.as_str(), trace))
            .collect();

        // 节点 trace 从 trace 表按 refs materialize（与两个 loader 同一条口径）。
        let mut nodes = std::mem::take(&mut self.nodes);
        for node in &mut nodes {
            node.session_id = session_id.clone();
            if let Some(refs) = &node.turn_trace_refs {
                if refs.is_empty() {
                    node.turn_trace_history.clear();
                } else {
                    node.turn_trace_history = refs
                        .iter()
                        .filter_map(|reference| {
                            by_turn.get(reference.turn_id.as_str()).map(|t| (*t).clone())
                        })
                        .collect();
                }
            }
        }
        let mut branches = std::mem::take(&mut self.branches);
        for branch in &mut branches {
            branch.session_id = session_id.clone();
        }

        // 先取走 cursor（`state()` 借 `self` 期间不能再动其它字段）再填会话级水位。
        let cursor = self.cursor.take();
        let state = self.state();
        state.conversation_id = session_id.clone();
        state.history = history;
        state.turn_trace_history = turn_trace_history;
        state.history_nodes = nodes;
        state.history_branches = branches;
        if let Some(mut cursor) = cursor {
            // 索引侧 cursor 行的 `session_id` 取自行键，不信任存储值；fold 同口径。
            cursor.session_id = session_id.clone();
            state.event_watermark = cursor.event_watermark;
            state.last_commit_watermark = cursor.event_watermark;
            state.history_cursor = cursor;
        }
        Ok(FoldedSession {
            state: self.state.take().expect("state initialized above"),
            deleted: self.deleted,
            event_rows: self.event_rows,
            turns: self.turns,
        })
    }
}

/// **纯函数**：把会话日志的命令流折叠成 [`SessionState`]。
///
/// - 不读盘 / 不写库 / **不读时钟** / 无全局状态：相同 `events` 必得相同结果；
/// - 未知或坏命令 ⇒ [`FoldError`]（绝不静默跳过）；
/// - `RemoveSession` ⇒ `deleted = true`（D3 墓碑）。
pub fn fold_session(events: &[LogEvent]) -> Result<FoldedSession, FoldError> {
    let mut folder = Folder::default();
    let mut previous_seq: Option<u64> = None;
    for event in events {
        if let Some(previous) = previous_seq {
            if event.seq <= previous {
                return Err(FoldError::NonMonotonicSeq {
                    previous,
                    found: event.seq,
                });
            }
        }
        previous_seq = Some(event.seq);
        let command = FoldCommand::decode(event)?;
        folder.apply(event.seq, command)?;
    }
    folder.finish()
}

/// 读某会话日志并 fold（`会话日志 → SessionState`）。
///
/// 会话 id 的权威来源是 header（文件自描述）；日志里的命令若指向别的会话 ⇒
/// [`FoldError::SessionMismatch`]（跨会话污染不猜、不放过）。
pub fn read_session_state(
    store: &SessionLogStore,
    session_id: &str,
) -> Result<FoldedSession, FoldError> {
    let metas = store.list_sessions().map_err(|error| FoldError::SessionNotFound {
        session_id: session_id.to_string(),
        reason: error.to_string(),
    })?;
    let meta = metas
        .into_iter()
        .find(|meta| meta.session_id == session_id)
        .ok_or_else(|| FoldError::SessionNotFound {
            session_id: session_id.to_string(),
            reason: "no `current` session log header in the log root".to_string(),
        })?;
    let events = read_log_events(store, &meta)?;
    let mut folded = fold_session(&events)?;
    if folded.state.conversation_id.is_empty() {
        // 只有 header、没有任何命令的会话：状态为空，会话 id 取 header。
        folded.state.conversation_id = session_id.to_string();
    } else if folded.state.conversation_id != session_id {
        return Err(FoldError::SessionMismatch {
            seq: 0,
            expected: session_id.to_string(),
            found: folded.state.conversation_id,
        });
    }
    Ok(folded)
}

// ---------------------------------------------------------------------------
// 逐字段对拍（测试与 rebuild 的 mismatched 判定共用同一口径）
// ---------------------------------------------------------------------------

/// 索引侧**独有**（命令流无法推导）的字段：`fold` 与索引在这两项上不比对。
///
/// - `trace_migration_state` / `turn_trace_refs` 是写路径按 `SeparateTraceTableMode`
///   决定的**投影形态**，日志里没有任何命令携带它们（不是状态，是存储配置的产物）。
///
/// 除这两项外，本函数比对 `SessionState` 的**每一个字段**。
pub const INDEX_ONLY_STATE_FIELDS: [&str; 2] = ["trace_migration_state", "turn_trace_refs"];

/// 逐字段对拍：返回**全部**差异（空 = 完全一致）。
///
/// 字段是逐个显式列出的（不是整体 JSON 比对）：任何一侧新增字段都会在这份清单上
/// 显式缺项，而不是被「整体相等」蒙过去。`trace_migration_state` / `turn_trace_refs`
/// 见 [`INDEX_ONLY_STATE_FIELDS`]。
pub fn session_parity_issues(folded: &SessionState, indexed: &SessionState) -> Vec<String> {
    let mut issues = Vec::new();
    let mut check = |field: &str, left: String, right: String| {
        if left != right {
            issues.push(format!("{field}: fold={left} index={right}"));
        }
    };
    check(
        "conversation_id",
        folded.conversation_id.clone(),
        indexed.conversation_id.clone(),
    );
    check("title", folded.title.clone(), indexed.title.clone());
    check("summary", folded.summary.clone(), indexed.summary.clone());
    check(
        "turn_count",
        folded.turn_count.to_string(),
        indexed.turn_count.to_string(),
    );
    check(
        "last_referenced_file",
        format!("{:?}", folded.last_referenced_file),
        format!("{:?}", indexed.last_referenced_file),
    );
    check(
        "updated_at_ms",
        folded.updated_at_ms.to_string(),
        indexed.updated_at_ms.to_string(),
    );
    check(
        "title_override",
        format!("{:?}", folded.title_override),
        format!("{:?}", indexed.title_override),
    );
    check(
        "archived",
        folded.archived.to_string(),
        indexed.archived.to_string(),
    );
    check(
        "workspace_id",
        format!("{:?}", folded.workspace_id),
        format!("{:?}", indexed.workspace_id),
    );
    check(
        "event_watermark",
        folded.event_watermark.to_string(),
        indexed.event_watermark.to_string(),
    );
    check(
        "last_commit_watermark",
        folded.last_commit_watermark.to_string(),
        indexed.last_commit_watermark.to_string(),
    );
    check(
        "provider_native_transcript",
        json_of(&folded.provider_native_transcript),
        json_of(&indexed.provider_native_transcript),
    );
    check(
        "long_term_memory_entries",
        json_of(&folded.long_term_memory_entries),
        json_of(&indexed.long_term_memory_entries),
    );
    check(
        "memory_write_evidence",
        json_of(&folded.memory_write_evidence),
        json_of(&indexed.memory_write_evidence),
    );
    check(
        "memory_write_hook_trace_records",
        json_of(&folded.memory_write_hook_trace_records),
        json_of(&indexed.memory_write_hook_trace_records),
    );
    check(
        "history_state_evidence",
        json_of(&folded.history_state_evidence),
        json_of(&indexed.history_state_evidence),
    );
    check(
        "history.len",
        folded.history.len().to_string(),
        indexed.history.len().to_string(),
    );
    check("history", json_of(&folded.history), json_of(&indexed.history));
    check(
        "turn_trace_history.len",
        folded.turn_trace_history.len().to_string(),
        indexed.turn_trace_history.len().to_string(),
    );
    check(
        "turn_trace_history",
        json_of(&folded.turn_trace_history),
        json_of(&indexed.turn_trace_history),
    );
    check(
        "history_nodes.len",
        folded.history_nodes.len().to_string(),
        indexed.history_nodes.len().to_string(),
    );
    check(
        "history_nodes",
        json_of(&folded.history_nodes),
        json_of(&indexed.history_nodes),
    );
    check(
        "history_branches",
        json_of(&folded.history_branches),
        json_of(&indexed.history_branches),
    );
    check(
        "history_cursor",
        json_of(&folded.history_cursor),
        json_of(&indexed.history_cursor),
    );
    issues
}

/// 逐字段对拍的单串结果（`None` = 完全一致）。
pub fn session_parity_diff(folded: &SessionState, indexed: &SessionState) -> Option<String> {
    let issues = session_parity_issues(folded, indexed);
    if issues.is_empty() {
        None
    } else {
        Some(issues.join("; "))
    }
}

fn json_of<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|error| format!("<unserializable: {error}>"))
}

// ---------------------------------------------------------------------------
// 索引重建
// ---------------------------------------------------------------------------

/// 索引重建选项。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RebuildOptions {
    /// 只统计、不写库（CLI `--dry-run`；调用方应同时以只读方式打开库）。
    pub dry_run: bool,
}

impl RebuildOptions {
    /// 预演（零写入）。
    pub fn dry_run() -> Self {
        Self { dry_run: true }
    }

    /// 真正写回索引。
    pub fn apply() -> Self {
        Self { dry_run: false }
    }
}

/// 索引重建报告（`ponycli sessions rebuild-index` 的输出契约）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RebuildReport {
    /// 扫到的**有日志文件**的会话数（header 为 `current` 的会话目录）。
    pub scanned: usize,
    /// 已用 fold 结果覆盖索引的会话（`dry_run` 时是「将要重建」的集合）。
    pub rebuilt: Vec<String>,
    /// 索引里有、日志里没有的会话数——**这些会话原封不动**（存量库的主体）。
    pub skipped_no_log: usize,
    /// 日志里有 `RemoveSession` 墓碑、因此不重建的会话（D3：删除不复活）。
    pub skipped_deleted: Vec<String>,
    /// fold 或写索引失败的会话（`(session_id, 原因)`）；**不中断**其余会话。
    pub failed: Vec<(String, String)>,
    /// 重建前「fold 结果 ≠ 索引现有状态」的会话（ADF 阶段 2 双读比对的落地点）。
    pub mismatched: Vec<String>,
}

impl RebuildReport {
    /// 汇总一行，供 CLI / 日志使用。
    pub fn summary_line(&self, dry_run: bool) -> String {
        format!(
            "mode={} scanned={} rebuilt={} skipped_no_log={} skipped_deleted={} failed={} mismatched={}",
            if dry_run { "dry-run" } else { "apply" },
            self.scanned,
            self.rebuilt.len(),
            self.skipped_no_log,
            self.skipped_deleted.len(),
            self.failed.len(),
            self.mismatched.len()
        )
    }
}

/// **从会话日志重建 SQLite 索引**（ADR 0099 阶段 2 第 2 项）。
///
/// 硬约束（顺序有意义）：
/// 1. 只扫描 `session-logs/`（真源）里的会话；**索引里没有日志的会话一律不动**，
///    只计入 `skipped_no_log`——把「没有日志」当「空会话」会一次性清空真实历史；
/// 2. 每个会话一次事务内**覆盖**自己的索引行（先删该会话行、再按 fold 结果重放）；
/// 3. 墓碑（`RemoveSession`）不重建：删除单调不可复活（D3）；
/// 4. 幂等：连跑两次结果一致（覆盖语义 + 纯函数 fold）；
/// 5. `dry_run` ⇒ 只统计，绝不写库。
pub fn rebuild_index(
    backend: &SqliteSessionBackend,
    options: RebuildOptions,
) -> Result<RebuildReport, String> {
    rebuild_index_at_root(backend, &backend.session_log_root(), options)
}

/// 与 [`rebuild_index`] 同语义，但日志根目录由调用方指定（测试可指向临时目录）。
pub fn rebuild_index_at_root(
    backend: &SqliteSessionBackend,
    log_root: &Path,
    options: RebuildOptions,
) -> Result<RebuildReport, String> {
    let store = SessionLogStore::new(log_root.to_path_buf());
    // 索引现有视图：**只加载一次**，且与读路径同一条分派（`load_store` 内部按
    // `storage.normalized.v1.phase` 在 blob / 规范化投影之间切换）。双读比对必须
    // 面对「产品实际读到的那份视图」，而不是另造一份口径。
    let index_view: HashMap<String, SessionState> = match backend.load_store() {
        Some(store) => store.sessions_view().clone(),
        None => {
            // 读不出现有索引 ⇒ 比对基准缺失。重建本身仍可进行（真源不依赖索引），
            // 但必须显式说清楚「mismatched 这一列不可信」，不许静默当成零差异。
            eprintln!(
                "[pony-agent][session] rebuild_index: load_store() 返回 None，\
                 双读比对（mismatched）本轮不可用"
            );
            HashMap::new()
        }
    };
    let metas = store
        .list_sessions()
        .map_err(|error| format!("scan session log root `{}`: {error}", log_root.display()))?;
    let mut report = RebuildReport {
        scanned: metas.len(),
        ..Default::default()
    };
    let mut logged: BTreeSet<String> = BTreeSet::new();
    for meta in metas {
        let session_id = meta.session_id.clone();
        logged.insert(session_id.clone());
        let events = match read_log_events(&store, &meta) {
            Ok(events) => events,
            Err(error) => {
                report
                    .failed
                    .push((session_id, format!("read log: {error}")));
                continue;
            }
        };
        let folded = match fold_session(&events) {
            Ok(folded) => folded,
            Err(error) => {
                report
                    .failed
                    .push((session_id, format!("fold: {error}")));
                continue;
            }
        };
        if folded.deleted {
            // D3 墓碑：不重建（删除单调不可复活）。
            report.skipped_deleted.push(session_id);
            continue;
        }
        // 双读比对（阶段 2 第 3 项的落地点）：重建前先记「fold vs 现有索引」的差异。
        if let Some(current) = index_view.get(&session_state_id(&folded.state)) {
            if session_parity_diff(&folded.state, current).is_some() {
                report.mismatched.push(session_id.clone());
            }
        }
        if !options.dry_run {
            if let Err(error) = backend.rebuild_folded_session_index(&folded.state) {
                report
                    .failed
                    .push((session_id, format!("write index: {error}")));
                continue;
            }
        }
        report.rebuilt.push(session_id);
    }
    // 无日志会话：索引里有、真源里没有 ⇒ 原封不动（这是存量库的绝大多数会话）。
    let index_ids = backend.session_index_ids()?;
    report.skipped_no_log = index_ids
        .iter()
        .filter(|session_id| !logged.contains(*session_id))
        .count();
    eprintln!(
        "[pony-agent][session] rebuild_index_summary: {}",
        report.summary_line(options.dry_run)
    );
    Ok(report)
}

/// fold 结果的会话键（`conversation_id` 即索引行键）。
fn session_state_id(state: &SessionState) -> String {
    state.conversation_id.clone()
}

/// 把 fold 结果摊平成 `PersistCommand` 序列（索引重建的重放输入）。
///
/// 复用 live 写路径的索引语义（`apply_persist_command_tx`）而不是另写一套 SQL：
/// 两边必须走同一套落位/去重/upsert 规则，否则「重建」会再造一份口径。
pub(crate) fn canonical_commands(state: &SessionState, epoch: u64) -> Vec<PersistCommand> {
    let session_id = state.conversation_id.clone();
    let mut commands = Vec::new();
    for (ordinal, message) in state.history.iter().enumerate() {
        commands.push(PersistCommand::AppendMessage {
            epoch,
            session_id: session_id.clone(),
            message: message.clone(),
            ordinal,
        });
    }
    commands.push(PersistCommand::UpdateSessionMeta {
        epoch,
        session_id: session_id.clone(),
        meta_patch: SessionMetaPatch {
            title: Some(state.title.clone()),
            title_override: Some(state.title_override.clone()),
            archived: Some(state.archived),
            summary: Some(state.summary.clone()),
            turn_count: Some(state.turn_count),
            last_referenced_file: state.last_referenced_file.clone(),
            updated_at_ms: Some(state.updated_at_ms),
        },
    });
    for (trace_order, trace) in state.turn_trace_history.iter().enumerate() {
        commands.push(PersistCommand::AppendTrace {
            epoch,
            session_id: session_id.clone(),
            trace: trace.clone(),
            trace_order,
        });
    }
    for node in &state.history_nodes {
        commands.push(PersistCommand::UpdateHistoryNode {
            epoch,
            session_id: session_id.clone(),
            node: node.clone(),
        });
    }
    commands.push(PersistCommand::UpdateCursor {
        epoch,
        session_id: session_id.clone(),
        cursor: state.history_cursor.clone(),
    });
    for branch in &state.history_branches {
        commands.push(PersistCommand::UpdateBranch {
            epoch,
            session_id: session_id.clone(),
            branch: branch.clone(),
        });
    }
    commands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::session::log_command::command_to_value;
    use crate::agent::session::PersistCommand;
    use serde_json::json;

    fn envelope(seq: u64, command: &PersistCommand) -> LogEvent {
        LogEvent {
            seq,
            ts: 1_760_000_000_000 + seq as i64,
            cmd: command_to_value(command).expect("encode command"),
        }
    }

    fn append_text(seq: u64, session_id: &str, role: &str, content: &str, ordinal: usize) -> LogEvent {
        envelope(
            seq,
            &PersistCommand::AppendMessage {
                epoch: 1,
                session_id: session_id.to_string(),
                message: TurnHistoryMessage {
                    role: role.to_string(),
                    content: content.to_string(),
                    ..Default::default()
                },
                ordinal,
            },
        )
    }

    fn meta_command(seq: u64, session_id: &str, patch: SessionMetaPatch) -> LogEvent {
        envelope(
            seq,
            &PersistCommand::UpdateSessionMeta {
                epoch: 1,
                session_id: session_id.to_string(),
                meta_patch: patch,
            },
        )
    }

    #[test]
    fn fold_is_deterministic_and_never_reads_the_clock() {
        let session_id = "pa_fold_pure";
        let events = vec![
            append_text(0, session_id, "user", "u1", 0),
            append_text(1, session_id, "assistant", "a1", 1),
        ];
        let first = fold_session(&events).expect("fold");
        let second = fold_session(&events).expect("fold again");
        assert_eq!(
            json_of(&first.state),
            json_of(&second.state),
            "纯函数：相同事件序列必须得到完全相同的状态"
        );
        // 所有时间戳都来自命令 payload：两次 fold 之间没有任何「当前时刻」泄漏。
        assert_eq!(first.state.updated_at_ms, 0);
        assert_eq!(first.state.history.len(), 2);
        assert_eq!(first.state.conversation_id, session_id);
    }

    #[test]
    fn unknown_command_is_a_hard_error_not_a_silent_skip() {
        let events = vec![LogEvent {
            seq: 0,
            ts: 1,
            cmd: json!({"type": "invented_command", "sessionId": "pa_x"}),
        }];
        let error = fold_session(&events).expect_err("unknown command must fail loud");
        match error {
            FoldError::UnknownCommand { seq, kind } => {
                assert_eq!(seq, 0);
                assert_eq!(kind, "invented_command");
            }
            other => panic!("expected UnknownCommand, got {other:?}"),
        }
    }

    #[test]
    fn malformed_envelope_and_non_monotonic_seq_fail_loud() {
        assert!(matches!(
            LogEvent::from_json_line("not json"),
            Err(FoldError::MalformedEnvelope { .. })
        ));
        assert!(matches!(
            LogEvent::from_json_line("{\"ts\":1}"),
            Err(FoldError::MalformedEnvelope { .. })
        ));
        // SessionLogEvent 形态（{"seq","type","ts","payload"}）不是命令信封 ⇒ 报错。
        assert!(matches!(
            LogEvent::from_json_line("{\"seq\":0,\"type\":\"x\",\"ts\":1,\"payload\":{}}"),
            Err(FoldError::MalformedEnvelope { .. })
        ));

        let events = vec![
            append_text(3, "pa_x", "user", "u", 0),
            append_text(2, "pa_x", "assistant", "a", 1),
        ];
        assert!(matches!(
            fold_session(&events),
            Err(FoldError::NonMonotonicSeq {
                previous: 3,
                found: 2
            })
        ));
    }

    #[test]
    fn meta_patch_three_state_is_preserved() {
        let session_id = "pa_fold_meta";
        // 1) Some(Some(v)) 赋值
        let set = fold_session(&[meta_command(
            0,
            session_id,
            SessionMetaPatch {
                title_override: Some(Some("自定义".to_string())),
                ..Default::default()
            },
        )])
        .expect("fold set");
        assert_eq!(set.state.title_override.as_deref(), Some("自定义"));
        // 2) None 不改（保持上一条命令的值）
        let untouched = fold_session(&[
            meta_command(
                0,
                session_id,
                SessionMetaPatch {
                    title_override: Some(Some("自定义".to_string())),
                    ..Default::default()
                },
            ),
            meta_command(
                1,
                session_id,
                SessionMetaPatch {
                    title: Some("派生标题".to_string()),
                    ..Default::default()
                },
            ),
        ])
        .expect("fold untouched");
        assert_eq!(
            untouched.state.title_override.as_deref(),
            Some("自定义"),
            "None 表示「本次不改这个字段」，不能把已有 override 冲掉"
        );
        assert_eq!(untouched.state.title, "派生标题");
        // 3) Some(None) 清空
        let cleared = fold_session(&[
            meta_command(
                0,
                session_id,
                SessionMetaPatch {
                    title_override: Some(Some("自定义".to_string())),
                    ..Default::default()
                },
            ),
            meta_command(
                1,
                session_id,
                SessionMetaPatch {
                    title_override: Some(None),
                    ..Default::default()
                },
            ),
        ])
        .expect("fold cleared");
        assert_eq!(cleared.state.title_override, None, "Some(None) 必须清空 override");
    }

    #[test]
    fn append_message_dedupes_on_turn_id_and_role() {
        let session_id = "pa_fold_dedupe";
        let message = |content: &str, ordinal: usize| PersistCommand::AppendMessage {
            epoch: 1,
            session_id: session_id.to_string(),
            message: TurnHistoryMessage {
                role: "user".to_string(),
                content: content.to_string(),
                turn_id: Some("turn-1".to_string()),
                ..Default::default()
            },
            ordinal,
        };
        let events = vec![
            envelope(0, &message("first", 0)),
            envelope(1, &message("rewritten", 0)),
        ];
        let folded = fold_session(&events).expect("fold");
        assert_eq!(
            folded.state.history.len(),
            1,
            "message_id = {{turn_id}}-{{role}} 相同 ⇒ 覆盖而不是并列两条"
        );
        assert_eq!(folded.state.history[0].content, "rewritten");
    }

    #[test]
    fn cross_session_command_is_rejected() {
        let events = vec![
            append_text(0, "pa_a", "user", "u", 0),
            append_text(1, "pa_b", "assistant", "a", 1),
        ];
        assert!(matches!(
            fold_session(&events),
            Err(FoldError::SessionMismatch { .. })
        ));
    }

    #[test]
    fn remove_session_marks_the_fold_result_deleted() {
        let events = vec![
            append_text(0, "pa_gone", "user", "u", 0),
            envelope(
                1,
                &PersistCommand::RemoveSession {
                    epoch: 1,
                    session_id: "pa_gone".to_string(),
                },
            ),
        ];
        let folded = fold_session(&events).expect("fold");
        assert!(folded.deleted, "RemoveSession 必须折叠成「已删除」");
    }

    #[test]
    fn every_persist_command_variant_decodes() {
        // 编码器的字段一个不漏（未知/坏字段由 fold 的严格解码兜住）。
        let commands = vec![
            append_text(0, "pa_all", "user", "u", 0),
            envelope(
                1,
                &PersistCommand::UpsertTurn {
                    epoch: 1,
                    session_id: "pa_all".to_string(),
                    turn: TurnStateRecord {
                        turn_id: "turn-1".to_string(),
                        ordinal: 0,
                        phase: Some("completed".to_string()),
                        status: None,
                        user_message_id: Some("turn-1-user".to_string()),
                        assistant_message_id: None,
                        started_at_ms: Some(1),
                        completed_at_ms: Some(2),
                        updated_at_ms: 3,
                    },
                },
            ),
            envelope(
                2,
                &PersistCommand::UpdateCursor {
                    epoch: 1,
                    session_id: "pa_all".to_string(),
                    cursor: HistoryCursor {
                        event_watermark: 7,
                        ..Default::default()
                    },
                },
            ),
            envelope(
                3,
                &PersistCommand::FlushEvents {
                    epoch: 1,
                    session_id: "pa_all".to_string(),
                    turn_id: "turn-1".to_string(),
                    branch_id: "main".to_string(),
                    events: Vec::new(),
                },
            ),
            envelope(
                4,
                &PersistCommand::PublishMetadata {
                    epoch: 1,
                    key: "k".to_string(),
                    value: "v".to_string(),
                },
            ),
        ];
        let folded = fold_session(&commands).expect("every variant decodes");
        assert_eq!(folded.turns.len(), 1);
        assert_eq!(folded.turns[0].phase.as_deref(), Some("completed"));
        assert_eq!(folded.state.event_watermark, 7);
        assert_eq!(folded.state.last_commit_watermark, 7);
        assert_eq!(folded.event_rows, 0);
    }
}
