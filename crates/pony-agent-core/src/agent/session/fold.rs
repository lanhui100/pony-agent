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
//! ## 阶段 2-B：这份 fold 就是**读路径的真源**
//!
//! `SqliteSessionBackend::load_store()`（运行时读路径）在索引原样视图之上叠加
//! [`merge_log_truth_into_sessions`]：有日志的会话以日志为准，无日志的会话保持索引
//! （存量 grandfather），墓碑从视图移除。**索引可删可建**由此兑现——删掉 `sessions`
//! 行之后会话照旧读得出来（`tests/session_read_from_log.rs` 的核心用例）。
//!
//! 两条必须写明的现状事实：
//!
//! 1. **写路径在阶段 2-D 之后只有一条内容通道**：`sessions.session_data` 对有日志的会话
//!    只写轻量索引行（`__index_only`），状态写（`save_store` / `commit_boundary`）不再产生
//!    完整内容。因此有日志的会话**日志无条件为准**——2-B 那条 `updated_at_ms` 定序已删除：
//!    轻量行的时间戳会随写路径前进，若仍按时间戳定序，日志会被「更新的空壳行」盖掉。
//! 2. **索引独有的投影产物**（`trace_migration_state` / `turn_trace_refs` / 光标节点快照）
//!    仍从索引行继承：日志没有承载它们的命令，丢掉等于凭空丢状态。轻量索引行不写
//!    `turn_trace_refs`（它随轮次增长），故这条继承在 2-D 之后只剩 `trace_migration_state`
//!    与 `workspace_id`（见 [`apply_index_projection`]）。
//!
//! 索引侧内容来源（2-D）：轻量行 + 索引自己的 `normalized_*` 投影由
//! `SqliteSessionBackend::fill_index_only_rows_from_index_projection` 补全，
//! **该补全不读日志**，所以「日志 vs 索引」的双读比对仍然有意义。
//!
//! 启动耗时保护：折叠总耗时超预算（[`DEFAULT_READ_FOLD_BUDGET`]）⇒ 记 `session_fold_slow`
//! 并整轮降级为读索引（不阻塞启动、不读半态），下轮再补。
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

use super::backend::{
    PersistCommand, SessionBackend, SessionMetaPatch, TraceTerminalPatch, TurnStateRecord,
};
use super::log_store::{SessionLogMeta, SessionLogStore};
use super::types::{
    HistoryBranch, HistoryCursor, HistoryNode, LongTermMemoryRecord, SessionState,
    TurnHistoryMessage, TurnTraceRecord,
};
use super::types::{DEFAULT_SESSION_SUMMARY, DEFAULT_SESSION_TITLE};
use crate::agent::hooks::{HistoryStateHookEvidence, HookTraceRecord, PersistedEffectEvidence};
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
    /// ADR 0102：provider transcript 的**追加**载荷。
    AppendProviderTranscript {
        session_id: String,
        messages: Vec<serde_json::Value>,
    },
    /// ADR 0102：记忆四件套（**整份替换**）+ `history_state_evidence`（**追加**）。
    UpdateSessionMemory {
        session_id: String,
        entries: Vec<LongTermMemoryRecord>,
        evidence: Vec<PersistedEffectEvidence>,
        hook_trace_records: Vec<HookTraceRecord>,
        history_state_evidence: Vec<HistoryStateHookEvidence>,
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
            "append_provider_transcript" => Self::AppendProviderTranscript {
                session_id: required_str(value, seq, kind, "sessionId")?.to_string(),
                messages: payload(value, seq, kind, "messages")?,
            },
            "update_session_memory" => Self::UpdateSessionMemory {
                session_id: required_str(value, seq, kind, "sessionId")?.to_string(),
                entries: payload(value, seq, kind, "entries")?,
                evidence: payload(value, seq, kind, "evidence")?,
                hook_trace_records: payload(value, seq, kind, "hookTraceRecords")?,
                history_state_evidence: payload(value, seq, kind, "historyStateEvidence")?,
            },
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
            | Self::AppendProviderTranscript { session_id, .. }
            | Self::UpdateSessionMemory { session_id, .. }
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
            // ADR 0102：transcript —— **追加**（与写路径 `store.rs` 的 `extend` 同语义：
            // 每轮把新增的 provider 消息接到尾部，历史消息一条都不得丢）。
            FoldCommand::AppendProviderTranscript { messages, .. } => {
                self.state().provider_native_transcript.extend(messages);
            }
            // ADR 0102：记忆 —— 三项**整份替换**（写路径就地重建它们），
            // `history_state_evidence` 则**追加**（写路径逐条累加）。两者语义不同，
            // 混用会让「同一 turn 重放两次」的记忆条数翻倍或清空。
            FoldCommand::UpdateSessionMemory {
                entries,
                evidence,
                hook_trace_records,
                history_state_evidence,
                ..
            } => {
                let state = self.state();
                state.long_term_memory_entries = entries;
                state.memory_write_evidence = evidence;
                state.memory_write_hook_trace_records = hook_trace_records;
                state.history_state_evidence.extend(history_state_evidence);
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
// ADR 0099 阶段 2-B：读路径切到日志真源
// ---------------------------------------------------------------------------

/// 读路径折叠的**耗时预算**（ADR 0099 阶段 2 门禁：启动装载 ≤800ms）。
///
/// 超预算不是错误，是**降级信号**：本轮保持索引视图（grandfather），记
/// `session_fold_slow` 日志，下轮再补。绝不因为「快了才可信」而在读路径上阻塞启动。
pub const DEFAULT_READ_FOLD_BUDGET: std::time::Duration = std::time::Duration::from_millis(800);

/// 读路径折叠报告（供调用方记录 `session_fold_slow` 与装载耗时）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReadFoldOutcome {
    /// 成功 fold 且**有已提交命令**的会话（日志真源侧）。
    pub folded: Vec<String>,
    /// 折叠出 `RemoveSession` 墓碑的会话（D3：读路径视其为不存在）。
    pub deleted: Vec<String>,
    /// fold / 读日志失败的会话（`(session_id, 原因)`）：**保留索引视图**，不丢会话。
    pub failed: Vec<(String, String)>,
    /// 因预算耗尽而**跳过折叠**的会话（`degraded()` 为真 ⇔ 非空）。
    pub skipped_due_to_budget: Vec<String>,
    /// 日志文件存在但没有任何已提交命令（只有 header）的会话：**不覆盖索引行**。
    pub empty_log: Vec<String>,
    /// 本轮折叠总耗时（毫秒）。
    pub elapsed_ms: u128,
}

impl ReadFoldOutcome {
    /// 是否触发了「超预算 ⇒ 降级为读索引」。
    pub fn degraded(&self) -> bool {
        !self.skipped_due_to_budget.is_empty()
    }
}

/// 读路径的**日志侧视图**：扫日志根，逐会话 fold 出 `SessionState`。
///
/// 与 [`rebuild_index`] 的区别：这里**只读**、不写索引、单会话失败不中断整轮
/// （该会话保留索引视图，等价于「下轮再补」）。
///
/// 超预算时返回**空视图**并把名单记进 `skipped_due_to_budget`——降级语义是「本轮按索引
/// 读」，不是「读一半日志 + 一半索引」：半态会让 `load_store` 的同一份结果在两次调用
/// 之间跳变，而启动路径只需要「可读且完整」。
pub fn fold_read_view(
    log_root: &Path,
    budget: std::time::Duration,
) -> (HashMap<String, SessionState>, ReadFoldOutcome) {
    let started = std::time::Instant::now();
    let store = SessionLogStore::new(log_root.to_path_buf());
    let mut outcome = ReadFoldOutcome::default();
    let metas = match store.list_sessions() {
        Ok(metas) => metas,
        Err(error) => {
            // 扫不动日志根（权限 / IO）⇒ 整轮降级为读索引，不阻塞启动。
            outcome.skipped_due_to_budget.push(format!("<log-root: {error}>"));
            return (HashMap::new(), outcome);
        }
    };
    // 预算为 0 ⇒ 立即降级（测试与「极端保护」入口用同一条判定，不做特殊分支）。
    let mut over_budget = budget.is_zero();
    let mut folded: HashMap<String, SessionState> = HashMap::new();
    for meta in metas {
        if over_budget || started.elapsed() > budget {
            over_budget = true;
            outcome.skipped_due_to_budget.push(meta.session_id.clone());
            continue;
        }
        let session_id = meta.session_id.clone();
        let events = match read_log_events(&store, &meta) {
            Ok(events) => events,
            Err(error) => {
                outcome
                    .failed
                    .push((session_id, format!("read log: {error}")));
                continue;
            }
        };
        if events.is_empty() {
            // 只有 header：日志还没有任何已提交命令。用它覆盖索引行会把真实历史读成空。
            outcome.empty_log.push(session_id);
            continue;
        }
        match fold_session(&events) {
            Ok(result) if result.deleted => outcome.deleted.push(session_id),
            Ok(result) => {
                let mut state = result.state;
                if state.conversation_id.is_empty() {
                    state.conversation_id = session_id.clone();
                }
                outcome.folded.push(session_id.clone());
                folded.insert(session_id, state);
            }
            Err(error) => outcome
                .failed
                .push((session_id, format!("fold: {error}"))),
        }
    }
    outcome.elapsed_ms = started.elapsed().as_millis();
    if outcome.degraded() {
        eprintln!(
            "[pony-agent][session] session_fold_slow: elapsed={}ms budget={}ms folded={} skipped={} — 降级为读索引（下轮再补）",
            outcome.elapsed_ms,
            budget.as_millis(),
            outcome.folded.len(),
            outcome.skipped_due_to_budget.len()
        );
        // 降级即整轮回落索引视图（见上方「半态」说明）。
        outcome.folded.clear();
        return (HashMap::new(), outcome);
    }
    (folded, outcome)
}

/// 把**日志真源**合并进索引视图（`load_store` 的读切换）。
///
/// 合并规则（ADR 0099 D1 / 阶段 2 / 2-D）：
/// 1. 有日志的会话 ⇒ **日志无条件为准**（同 id 覆盖索引行）。阶段 2-D 停写整包 blob 后，
///    索引行只剩索引字段（`__index_only` 轻量行），日志是唯一内容来源；原先那条
///    「索引 `updated_at_ms` 更新就保留索引」的过渡定序已删除——留着它会把轻量行
///    （内容为空、时间戳却随写路径前进）当成比日志更新，从而把会话读成空壳；
/// 2. 无日志的会话 ⇒ 保持索引视图（存量会话，grandfather，不得丢失）；
/// 3. 墓碑（`RemoveSession`）⇒ 从视图里移除（D3：删除不复活）；
/// 4. 折叠失败的会话 ⇒ 保持索引视图（不丢会话，下一轮再补）；
/// 5. fold 无法推导的索引侧字段（`INDEX_ONLY_STATE_FIELDS` + `workspace_id`）从索引行继承，
///    否则读切换会凭空丢掉「日志不承载」的真实状态。
pub fn merge_log_truth_into_sessions(
    log_root: &Path,
    sessions: &mut HashMap<String, SessionState>,
) -> ReadFoldOutcome {
    merge_log_truth_into_sessions_with_budget(log_root, sessions, DEFAULT_READ_FOLD_BUDGET)
}

/// 与 [`merge_log_truth_into_sessions`] 同语义，但预算由调用方给出（测试/维护入口用）。
pub fn merge_log_truth_into_sessions_with_budget(
    log_root: &Path,
    sessions: &mut HashMap<String, SessionState>,
    budget: std::time::Duration,
) -> ReadFoldOutcome {
    let (folded, outcome) = fold_read_view(log_root, budget);
    for (session_id, state) in folded {
        match sessions.get(&session_id) {
            None => {
                // 索引里没有这一行 ⇒ 日志是唯一来源（阶段 2-B 的核心兑现点）。
                sessions.insert(session_id, state);
            }
            Some(indexed) => {
                sessions.insert(session_id, apply_index_projection(indexed, state));
            }
        }
    }
    for session_id in &outcome.deleted {
        sessions.remove(session_id);
    }
    outcome
}

/// 把**索引独有的投影产物**套到日志 fold 出的状态上（日志里没有承载它们的命令）。
///
/// 1. `workspace_id` / `trace_migration_state`：见 [`INDEX_ONLY_STATE_FIELDS`]——日志里没有
///    任何命令携带它们，丢掉即凭空丢状态（`turn_trace_refs` 自 2-D 起**不再**继承，理由见
///    函数内注释）；
/// 2. 顶层 `turn_trace_history` 的**投影形状**：只显示**当前分支**（可见节点祖先链上的
///    turn）。阶段 2-D 之前这条过滤按索引行的 `turn_trace_refs` 做；停写 blob 后索引行
///    不带 refs，故改为从日志自己的 DAG（节点 `parent_node_id` + `turn_id`）推导——
///    不套这层投影就会把 fork 分支的 trace 泄漏到顶层显示里；
/// 3. `history` 的**可见分支视图**：由光标所指节点的快照派生（见函数内注释）。
fn apply_index_projection(indexed: &SessionState, mut state: SessionState) -> SessionState {
    if state.workspace_id.is_none() {
        state.workspace_id = indexed.workspace_id.clone();
    }
    state.trace_migration_state = indexed.trace_migration_state.clone();
    if state.turn_trace_refs.is_none() {
        // 字段继承保留（索引行是它唯一的载体）；但**不再用它过滤 trace**：ADR 0099
        // 阶段 2-D 停写 blob 后，这份 refs 可能来自陈旧整包行（或轻量行根本没有），
        // 拿它过滤会把日志这一轮新增的 trace 静默抹掉（2-A 记录过的既有缺口反过来咬到
        // 读路径）。顶层 trace 的「可见分支视图」改由**可见节点祖先链**推导（见本函数末尾）。
        state.turn_trace_refs = indexed.turn_trace_refs.clone();
    }
    // **可见视图投影**：`history` 的「当前分支视图」由光标所指节点的快照承载
    // （写路径在切分支 / 每轮同步时把整份可见视图写进 `UpdateHistoryNode` 的快照）。
    // fold 累加的 `messages` 是**全部分支的并集**，不套这层投影就会把 fork 分支的
    // 消息泄漏到顶层（`refresh_session_metadata` 还会据此把 turn_count 算错）。
    let visible_node_id = state
        .history_cursor
        .visible_node_id
        .clone()
        .or_else(|| state.history_cursor.branch_head_node_id.clone());
    // 日志侧「当前分支的 trace 集合」的等价推导：**可见节点祖先链**上的 turn。
    // 阶段 2-D 停写 blob 后索引行不再携带 `turn_trace_refs`（它随轮次增长），若只认 refs，
    // 重启后顶层 trace 会退化成「全部分支的并集」（fork 的 trace 泄漏）或被节点空快照抹掉。
    let branch_turn_ids: std::collections::BTreeSet<String> = visible_node_id
        .as_deref()
        .map(|node_id| ancestor_turn_ids(&state.history_nodes, node_id))
        .unwrap_or_default();
    let full_traces = state.turn_trace_history.clone();
    if let Some(node_id) = visible_node_id {
        let node = state
            .history_nodes
            .iter()
            .find(|node| node.node_id == node_id)
            .cloned();
        if let Some(node) = node {
            if !node.history.is_empty() {
                // 复用写路径同一份 hydrate 语义（同模块可见），不另造一套投影规则。
                hydrate_session_from_node(&mut state, &node);
            }
        }
    }
    if !branch_turn_ids.is_empty() {
        let visible_traces: Vec<TurnTraceRecord> = full_traces
            .iter()
            .filter(|trace| branch_turn_ids.contains(trace.turn_id.as_str()))
            .cloned()
            .collect();
        if !visible_traces.is_empty() {
            state.turn_trace_history = visible_traces;
        }
    }
    state
}

/// 节点快照 → 会话内存态（history / transcript / trace / memory / turn_count 全覆盖）。
///
/// 逐字段拷贝语义与写路径一致；`fold.rs::apply_index_projection` 与 `store` 的
/// 分支切换 / 恢复路径共用它，避免两套 hydrate 规则分叉。
pub(in crate::agent::session) fn hydrate_session_from_node(
    session: &mut SessionState,
    node: &HistoryNode,
) {
    session.title = node.title.clone();
    session.summary = node.summary.clone();
    session.history = node.history.clone();
    session.provider_native_transcript = node.provider_native_transcript.clone();
    session.turn_trace_history = node.turn_trace_history.clone();
    session.long_term_memory_entries = node.long_term_memory_entries.clone();
    session.memory_write_evidence = node.memory_write_evidence.clone();
    session.memory_write_hook_trace_records = node.memory_write_hook_trace_records.clone();
    session.turn_count = node.turn_count;
    session.last_referenced_file = node.last_referenced_file.clone();
}

/// 可见节点的**祖先链**上出现过的 turn id 集合（含自身）。
///
/// 用途见 [`apply_index_projection`]：日志里没有「会话级 trace 引用」这条命令，
/// 但节点命令带 `parent_node_id` + `turn_id`，因此「当前分支有哪些 turn」是可推导的。
/// 断环保护：`visited` 保证环状 `parent_node_id`（坏日志）不会死循环。
fn ancestor_turn_ids(nodes: &[HistoryNode], node_id: &str) -> std::collections::BTreeSet<String> {
    let mut by_id: HashMap<&str, &HistoryNode> = HashMap::new();
    for node in nodes {
        by_id.insert(node.node_id.as_str(), node);
    }
    let mut turns = std::collections::BTreeSet::new();
    let mut visited: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    let mut cursor = Some(node_id);
    while let Some(current) = cursor {
        if !visited.insert(current) {
            break;
        }
        let Some(node) = by_id.get(current) else {
            break;
        };
        if let Some(turn_id) = &node.turn_id {
            turns.insert(turn_id.clone());
        }
        cursor = node.parent_node_id.as_deref();
    }
    turns
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
    // 索引现有视图：**只加载一次**，且必须取**索引原样**（`load_index_store`），
    // 不能取运行时的 `load_store()`。
    //
    // ADR 0099 阶段 2-B 把运行时读路径切到了日志真源（`load_store` = 日志 fold ∪ 索引
    // grandfather），若拿它当比对基准，`mismatched` 会退化成「fold 与 fold 比」的恒零，
    // 双读比对（阶段 2 第 3 项）就此失去意义。故基准固定为索引原样视图；这也正是
    // 「索引可重建 / 可校验」这条承诺的观测面。
    let mut index_view: HashMap<String, SessionState> = match backend.load_index_store() {
        Some(store) => store.sessions_view().clone(),
        None => {
            // 读不出现有索引 ⇒ 比对基准缺失。重建本身仍可进行（真源不依赖索引），
            // 但必须显式说清楚「mismatched 这一列不可信」，不许静默当成零差异。
            eprintln!(
                "[pony-agent][session] rebuild_index: load_index_store() 返回 None，\
                 双读比对（mismatched）本轮不可用"
            );
            HashMap::new()
        }
    };
    // ADR 0099 阶段 2-D：停写 blob 后，有日志会话的索引行只有索引字段。若直接拿它当基准，
    // 每个会话都会被记成 mismatch（内容侧全空），比对列等于永久噪声。
    // 因此把轻量行按**索引自己的** `normalized_*` 投影补全（不读日志 ⇒ 仍是独立观察者），
    // 基准才回到「索引内容 vs 日志 fold」的正确语义。
    backend.fill_index_only_rows_from_index_projection(
        &mut index_view,
        &std::collections::HashSet::new(),
    );
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

// ---------------------------------------------------------------------------
// ADR 0102：索引侧投影写入（transcript 追加 / 记忆整份替换 + evidence 追加）
// ---------------------------------------------------------------------------
//
// 为什么放在 fold 模块而不是 `sqlite_session*.rs`：那两个文件已被行数棘轮卡死
// （`sqlite_session_commit.rs` 剩余额度 < 一个写函数的体积）。语义上也成立——本模块就是
// 「命令流 ↔ 状态投影」的家，这里补的是「命令 → 索引投影」的同一条映射。

/// 读-改-写 `normalized_sessions` 的某个 JSON 数组列（**追加**语义）。
fn append_json_column_tx<T: serde::Serialize + serde::de::DeserializeOwned + Clone>(
    tx: &rusqlite::Transaction<'_>, session_id: &str, column: &str, delta: &[T],
) -> Result<(), String> {
    let raw: Option<String> = tx
        .query_row(
            &format!("SELECT {column} FROM normalized_sessions WHERE session_id = ?1"),
            rusqlite::params![session_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .ok()
        .flatten();
    let mut all: Vec<T> = raw.as_deref().and_then(|v| serde_json::from_str(v).ok()).unwrap_or_default();
    all.extend(delta.iter().cloned());
    let encoded = serde_json::to_string(&all).map_err(|e| format!("encode {column}: {e}"))?;
    tx.execute(
        &format!("UPDATE normalized_sessions SET {column} = ?2 WHERE session_id = ?1"),
        rusqlite::params![session_id, encoded],
    )
    .map_err(|e| format!("update {column}: {e}"))?;
    Ok(())
}

/// ADR 0102：transcript **追加**进索引投影（与 fold 应用、`store.rs` 的 `extend` 同语义）。
pub(crate) fn append_provider_transcript_tx(
    tx: &rusqlite::Transaction<'_>, session_id: &str, messages: &[serde_json::Value],
) -> Result<(), String> {
    append_json_column_tx(tx, session_id, "provider_native_transcript_json", messages)
}

/// ADR 0102：记忆三项**整份替换** + `history_state_evidence` **追加**进索引投影。
///
/// 并列是刻意的：与命令定义、`Folder::apply` 一一对应；混用会让重放后的条数翻倍或清空。
pub(crate) fn update_session_memory_tx(
    tx: &rusqlite::Transaction<'_>, session_id: &str, entries: &[LongTermMemoryRecord],
    evidence: &[PersistedEffectEvidence], hook_trace_records: &[HookTraceRecord],
    history_state_evidence: &[HistoryStateHookEvidence],
) -> Result<(), String> {
    let memory = serde_json::json!({
        "longTermMemoryEntries": entries, "memoryWriteEvidence": evidence,
        "memoryWriteHookTraceRecords": hook_trace_records,
    });
    let encoded = serde_json::to_string(&memory).map_err(|e| format!("encode memory: {e}"))?;
    tx.execute(
        "UPDATE normalized_sessions SET memory_json = ?2 WHERE session_id = ?1",
        rusqlite::params![session_id, encoded],
    )
    .map_err(|e| format!("update session memory: {e}"))?;
    append_json_column_tx(tx, session_id, "history_state_evidence_json", history_state_evidence)
}

/// ADR 0102：把「内容类命令」写进索引投影（`apply_persist_command_tx` 的单一分派点）。
///
/// 只覆盖这两条命令；其它命令由调用方的**穷尽 match** 各自表态（这里不是兜底：
/// 调用点的主 match 仍无通配臂，新增变体会被编译器逼着表态）。非内容类命令传进来属于
/// 编程错误，返回 `Err` 而不是静默 no-op。
pub(crate) fn persist_session_content_tx(
    tx: &rusqlite::Transaction<'_>,
    command: &PersistCommand,
) -> Result<(), String> {
    match command {
        PersistCommand::AppendProviderTranscript { session_id, messages, .. } => {
            append_provider_transcript_tx(tx, session_id, messages)
        }
        PersistCommand::UpdateSessionMemory {
            session_id, entries, evidence, hook_trace_records, history_state_evidence, ..
        } => update_session_memory_tx(
            tx, session_id, entries, evidence, hook_trace_records, history_state_evidence,
        ),
        other => Err(format!("persist_session_content_tx 收到非内容类命令：{other:?}")),
    }
}

/// ADR 0102：把 **transcript 增量** 追加进命令批（**追加**语义）。
///
/// 与 `store.rs` 的 `staged_session.provider_native_transcript.extend(..)` 一一对应：
/// 命令里只放本批新增的 provider 消息，重放时接到尾部（历史消息不会被覆盖）。
pub(crate) fn append_transcript_command(
    commands: &mut Vec<PersistCommand>,
    session_id: &str,
    messages: &[serde_json::Value],
) {
    if messages.is_empty() {
        return;
    }
    commands.push(PersistCommand::AppendProviderTranscript {
        epoch: 1,
        session_id: session_id.to_string(),
        messages: messages.to_vec(),
    });
}

/// ADR 0102：记忆/证据**发生变化**时，把记忆投影追加进命令批。
///
/// 语义（与命令定义、`fold` 一一对应，不得混用）：
/// - `entries` / `evidence` / `hook_trace_records`：**整份替换**（写路径就地重建）；
/// - `history_state_evidence`：**追加**，故只发「本批新增的尾部」。
///
/// 尾部判据是 `after.starts_with(before)`：写路径只会 `extend`，前缀必然保持。若出现
/// 「证据被改写」（前缀不再匹配）这种当前写路径不可能产生的形态，则**不发该字段**并
/// 告警（宁可少一条投影，也不伪造一条语义相反的命令）。
pub(crate) fn append_memory_command_if_changed(
    commands: &mut Vec<PersistCommand>,
    session_id: &str,
    before: &SessionState,
    after: &SessionState,
) {
    let memory_changed = before.long_term_memory_entries != after.long_term_memory_entries
        || before.memory_write_evidence != after.memory_write_evidence
        || before.memory_write_hook_trace_records != after.memory_write_hook_trace_records;
    let evidence_appended = after.history_state_evidence.len() > before.history_state_evidence.len();
    let evidence_prefix_ok = after
        .history_state_evidence
        .starts_with(&before.history_state_evidence);
    if !memory_changed && !evidence_appended {
        return;
    }
    let evidence_delta = if evidence_appended && evidence_prefix_ok {
        after.history_state_evidence[before.history_state_evidence.len()..].to_vec()
    } else {
        if evidence_appended {
            eprintln!(
                "[pony-agent][session] history_state_evidence_prefix_mismatch: session={session_id}（证据不是追加形态，本命令不带 evidence）"
            );
        }
        Vec::new()
    };
    commands.push(PersistCommand::UpdateSessionMemory {
        epoch: 1,
        session_id: session_id.to_string(),
        entries: after.long_term_memory_entries.clone(),
        evidence: after.memory_write_evidence.clone(),
        hook_trace_records: after.memory_write_hook_trace_records.clone(),
        history_state_evidence: evidence_delta,
    });
}

/// 把**整份图投影**（trace 行 + 节点 + 分支 + 光标）作为命令流追加进真源日志。
///
/// ADR 0099 阶段 2-D：`ReplaceAll` 突变（fork / switch / checkout / restore / trace 全量重写）
/// 以前只写 blob + trace 表。停写 blob 之后，图状态若仍不落日志，重启会读回旧分支、
/// trace 变空 —— 那不是「SQLite 退为索引」，而是真数据丢失。故这些路径改走命令流。
///
/// 返回 `true` = 命令流已提交（调用方不必再走整包路径）；`false` = 回退全量写（不丢数据）。
pub(crate) fn persist_graph_projection_commands(
    backend: &dyn SessionBackend,
    session_id: &str,
    session: &SessionState,
    mutation: &super::backend::SessionTraceMutation,
) -> bool {
    const EPOCH: u64 = 1;
    let traces = match mutation {
        super::backend::SessionTraceMutation::ReplaceAll { traces } => traces.as_slice(),
        _ => &[],
    };
    let mut commands: Vec<PersistCommand> =
        Vec::with_capacity(traces.len() + session.history_nodes.len() + session.history_branches.len() + 1);
    for (trace_order, trace) in traces.iter().enumerate() {
        commands.push(PersistCommand::AppendTrace {
            epoch: EPOCH,
            session_id: session_id.to_string(),
            trace: trace.clone(),
            trace_order,
        });
    }
    for node in &session.history_nodes {
        commands.push(PersistCommand::UpdateHistoryNode {
            epoch: EPOCH,
            session_id: session_id.to_string(),
            node: node.clone(),
        });
    }
    for branch in &session.history_branches {
        commands.push(PersistCommand::UpdateBranch {
            epoch: EPOCH,
            session_id: session_id.to_string(),
            branch: branch.clone(),
        });
    }
    commands.push(PersistCommand::UpdateCursor {
        epoch: EPOCH,
        session_id: session_id.to_string(),
        cursor: session.history_cursor.clone(),
    });
    matches!(
        backend.persist_commands_batch(commands),
        super::backend::PersistCommandOutcome::Succeeded
    )
}

/// 会话级元数据的**真源追加**（ADR 0099 阶段 2-D）。
///
/// 标题 / 归档位 / 摘要 / 轮次 / 时间以前只随整包 blob 落盘。停写 blob 后索引行只剩索引
/// 字段、读路径又以日志为准 ⇒ 这些元数据必须同样进日志命令流，否则重命名 / 归档在重启后
/// 会读回旧值（真用户可见的回归）。后端不支持命令流（Memory / File）时返回 `false`，
/// 调用方仍靠整包写兜底。
pub(crate) fn persist_session_meta_command(
    backend: &dyn SessionBackend,
    session_id: &str,
    sessions: &HashMap<String, SessionState>,
) {
    let Some(session) = sessions.get(session_id) else {
        return;
    };
    let meta_patch = SessionMetaPatch {
        title: Some(session.title.clone()),
        title_override: Some(session.title_override.clone()),
        archived: Some(session.archived),
        summary: Some(session.summary.clone()),
        turn_count: Some(session.turn_count),
        last_referenced_file: session.last_referenced_file.clone(),
        updated_at_ms: Some(session.updated_at_ms),
    };
    let outcome = backend.persist_command(PersistCommand::UpdateSessionMeta {
        epoch: 1,
        session_id: session_id.to_string(),
        meta_patch,
    });
    if !matches!(outcome, super::backend::PersistCommandOutcome::Succeeded) {
        eprintln!(
            "[pony-agent][session] session_meta_command_not_persisted: session={session_id} outcome={outcome:?}（回退整包写）"
        );
    }
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
        // ADR 0102：transcript / 记忆 / evidence 也必须参与索引重建——否则 rebuild 出来的
    // 投影会把这几个字段丢掉（日志有、索引无 ⇒ 「索引可重建」不成立）。
    if !state.provider_native_transcript.is_empty() {
        commands.push(PersistCommand::AppendProviderTranscript {
            epoch,
            session_id: session_id.clone(),
            messages: state.provider_native_transcript.clone(),
        });
    }
    if !state.long_term_memory_entries.is_empty()
        || !state.memory_write_evidence.is_empty()
        || !state.memory_write_hook_trace_records.is_empty()
        || !state.history_state_evidence.is_empty()
    {
        commands.push(PersistCommand::UpdateSessionMemory {
            epoch,
            session_id: session_id.clone(),
            entries: state.long_term_memory_entries.clone(),
            evidence: state.memory_write_evidence.clone(),
            hook_trace_records: state.memory_write_hook_trace_records.clone(),
            history_state_evidence: state.history_state_evidence.clone(),
        });
    }
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
    use std::time::Duration;

    fn append_text_command(session_id: &str, role: &str, content: &str, ordinal: usize) -> PersistCommand {
        PersistCommand::AppendMessage {
            epoch: 1,
            session_id: session_id.to_string(),
            message: TurnHistoryMessage {
                role: role.to_string(),
                content: content.to_string(),
                ..Default::default()
            },
            ordinal,
        }
    }

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

    // -----------------------------------------------------------------------
    // ADR 0099 阶段 2-B：读路径日志真源合并
    // -----------------------------------------------------------------------

    fn temp_log_root(label: &str) -> std::path::PathBuf {
        let unique = format!(
            "pony-0099-2b-fold-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).expect("create temp log root");
        root
    }

    fn write_log(log_root: &Path, session_id: &str, commands: &[PersistCommand]) {
        let store = SessionLogStore::new(log_root.to_path_buf());
        let meta = SessionLogMeta::new(session_id, crate::agent::session::current_log_cwd());
        let mut writer = store.open_write(session_id, &meta).expect("open log writer");
        let values: Vec<Value> = commands
            .iter()
            .map(|command| command_to_value(command).expect("encode command"))
            .collect();
        writer
            .append_commands(1_760_000_000_000, &values)
            .expect("append commands");
    }

    fn index_state(session_id: &str, content: &str, updated_at_ms: u64) -> SessionState {
        SessionState {
            conversation_id: session_id.to_string(),
            title: "索引标题".to_string(),
            summary: String::new(),
            history: vec![TurnHistoryMessage {
                role: "user".to_string(),
                content: content.to_string(),
                ..Default::default()
            }],
            provider_native_transcript: Vec::new(),
            turn_trace_history: Vec::new(),
            trace_migration_state: crate::agent::session::TraceMigrationState::TraceTableAuthoritative,
            turn_trace_refs: Some(Vec::new()),
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 1,
            last_referenced_file: None,
            updated_at_ms,
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: HistoryCursor::default(),
            event_watermark: 0,
            last_commit_watermark: 0,
            workspace_id: Some("ws-index-only".to_string()),
            title_override: None,
            archived: false,
        }
    }

    #[test]
    fn read_path_merge_prefers_the_log_and_inherits_index_only_fields() {
        let root = temp_log_root("prefer_log");
        let session_id = "pa_2b_fold_prefer";
        write_log(
            &root,
            session_id,
            &[
                append_text_command(session_id, "user", "日志内容", 0),
                PersistCommand::UpdateSessionMeta {
                    epoch: 1,
                    session_id: session_id.to_string(),
                    meta_patch: SessionMetaPatch {
                        title: Some("日志标题".to_string()),
                        updated_at_ms: Some(1_760_000_000_000),
                        ..Default::default()
                    },
                },
            ],
        );
        let mut sessions = HashMap::new();
        sessions.insert(
            session_id.to_string(),
            index_state(session_id, "索引内容", 1_759_000_000_000),
        );

        let outcome = merge_log_truth_into_sessions(&root, &mut sessions);

        assert!(!outcome.degraded(), "预算充足时不得降级");
        assert_eq!(outcome.folded, vec![session_id.to_string()]);
        let merged = &sessions[session_id];
        assert_eq!(merged.title, "日志标题", "同 id 以日志为准");
        assert_eq!(merged.history.len(), 1);
        assert_eq!(merged.history[0].content, "日志内容");
        // 日志里没有承载它们的命令 ⇒ 从索引行继承（否则读切换会凭空丢状态）。
        assert_eq!(merged.workspace_id.as_deref(), Some("ws-index-only"));
        assert_eq!(
            merged.trace_migration_state,
            crate::agent::session::TraceMigrationState::TraceTableAuthoritative
        );
        assert_eq!(merged.turn_trace_refs, Some(Vec::new()));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ADR 0099 阶段 2-D：2-B 的「索引 `updated_at_ms` 更新就保留索引」过渡定序**已删除**。
    ///
    /// 用例方向随之反转（断言更强，不是放宽）：索引时间戳更新时，日志依旧无条件为准。
    /// 理由：停写 blob 后索引行只剩索引字段（内容为空），若仍按时间戳定序，一个「更新的
    /// 空壳行」会把会话读成空；而索引时间戳本来就是每条写路径都会前进的噪声。
    #[test]
    fn read_path_merge_prefers_the_log_even_when_the_index_timestamp_is_newer() {
        let root = temp_log_root("index_newer");
        let session_id = "pa_2b_fold_index_newer";
        write_log(
            &root,
            session_id,
            &[
                append_text_command(session_id, "user", "日志内容", 0),
                PersistCommand::UpdateSessionMeta {
                    epoch: 1,
                    session_id: session_id.to_string(),
                    meta_patch: SessionMetaPatch {
                        title: Some("日志标题".to_string()),
                        updated_at_ms: Some(1_760_000_000_000),
                        ..Default::default()
                    },
                },
            ],
        );
        // 索引侧时间戳更新（写路径每轮都会前进）⇒ 仍然是日志为准，不得读回索引里的旧内容。
        let mut sessions = HashMap::new();
        sessions.insert(
            session_id.to_string(),
            index_state(session_id, "更新的索引内容", 1_760_000_000_500),
        );

        let outcome = merge_log_truth_into_sessions(&root, &mut sessions);

        assert_eq!(outcome.folded, vec![session_id.to_string()], "日志侧确实 fold 成功");
        assert_eq!(
            sessions[session_id].history[0].content, "日志内容",
            "日志无条件为准：索引时间戳更新也不得把日志内容盖回去（2-D 删除过渡定序）"
        );
        assert_eq!(sessions[session_id].title, "日志标题");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_path_merge_with_zero_budget_degrades_to_the_index_view() {
        let root = temp_log_root("degrade");
        let session_id = "pa_2b_fold_degrade";
        write_log(
            &root,
            session_id,
            &[append_text_command(session_id, "user", "日志内容", 0)],
        );
        let mut sessions = HashMap::new();
        sessions.insert(session_id.to_string(), index_state(session_id, "索引内容", 1));

        let outcome = merge_log_truth_into_sessions_with_budget(&root, &mut sessions, Duration::ZERO);

        assert!(outcome.degraded(), "预算为 0 必须触发降级");
        assert_eq!(outcome.skipped_due_to_budget.len(), 1);
        assert!(outcome.folded.is_empty(), "降级 = 整轮回落索引，不读半态");
        assert_eq!(
            sessions[session_id].history[0].content, "索引内容",
            "降级后必须保持索引视图（不得被半折叠结果污染）"
        );
        assert_eq!(sessions.len(), 1, "降级不得丢会话");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_path_merge_drops_tombstones_and_keeps_unlogged_sessions() {
        let root = temp_log_root("tombstone");
        let tomb = "pa_2b_fold_tombstone";
        let index_only = "pa_2b_fold_index_only";
        write_log(
            &root,
            tomb,
            &[
                append_text_command(tomb, "user", "写了再删", 0),
                PersistCommand::RemoveSession {
                    epoch: 1,
                    session_id: tomb.to_string(),
                },
            ],
        );
        let mut sessions = HashMap::new();
        sessions.insert(tomb.to_string(), index_state(tomb, "残留索引行", 1));
        sessions.insert(index_only.to_string(), index_state(index_only, "索引内容", 1));

        let outcome = merge_log_truth_into_sessions(&root, &mut sessions);

        assert_eq!(outcome.deleted, vec![tomb.to_string()]);
        assert!(
            !sessions.contains_key(tomb),
            "日志墓碑必须让读路径视其不存在（D3：删除单调不可复活）"
        );
        assert!(
            sessions.contains_key(index_only),
            "无日志的存量会话必须原样保留（grandfather）"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn read_path_merge_does_not_let_a_header_only_log_wipe_the_index() {
        let root = temp_log_root("empty_log");
        let session_id = "pa_2b_fold_empty_log";
        // 只有 header（写者打开文件后崩溃）⇒ 日志还没有任何已提交命令。
        let store = SessionLogStore::new(root.clone());
        drop(
            store
                .open_write(
                    session_id,
                    &SessionLogMeta::new(session_id, crate::agent::session::current_log_cwd()),
                )
                .expect("create header-only log"),
        );
        let mut sessions = HashMap::new();
        sessions.insert(session_id.to_string(), index_state(session_id, "索引内容", 1));

        let outcome = merge_log_truth_into_sessions(&root, &mut sessions);

        assert_eq!(outcome.empty_log, vec![session_id.to_string()]);
        assert_eq!(
            sessions[session_id].history[0].content, "索引内容",
            "空日志不得把真实历史读成空会话"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
