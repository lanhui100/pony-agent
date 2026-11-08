mod backend;
mod file_backend;
mod id;
// ADR 0099 阶段 1-A：会话日志文件真源。
mod log_store;
// ADR 0099 阶段 1-B：命令流编码 + 日志写入接线（真源优先，SQLite 作索引）。
mod log_command;
mod log_sink;
// ADR 0099 阶段 2-A：日志命令流 → SessionState 的 fold 重建器 + SQLite 索引重建。
pub(crate) mod fold;
mod store;
#[cfg(test)]
mod tests;
// ADR 0092 回归：goal 续轮文案识别（单独成文件，避免撑大已触棘轮的 `tests.rs`）。
#[cfg(test)]
mod host_continuation_goal_round_tests;
mod types;
pub mod usage;
pub mod workspace_snapshot;

// ADR 0099 阶段 1-A：仅 additive 导出。
pub use log_store::{
    encode_session_id_segment, normalize_cwd_segment, read_header, HeaderClass, LogReader,
    LogStoreError, LogWriter, NewSessionLogEvent, SessionLogEvent, SessionLogHeader, SessionLogMeta,
    SessionLogStore, SESSION_LOG_FILE_NAME, SESSION_LOG_FORMAT_VERSION,
};

// ADR 0099 阶段 1-B：additive 导出（`sqlite_session` 接线与验收测试共用同一路径解析）。
pub use log_sink::{
    current_log_cwd, session_log_path_for_db, session_log_root_for_db, SessionLogSink,
    SESSION_LOG_ROOT_DIR_NAME,
};

// ADR 0099 阶段 2-A：additive 导出（fold 重建器 / 索引重建 / 逐字段对拍口径）。
pub use fold::{
    fold_session, read_log_events, read_session_state, rebuild_index, rebuild_index_at_root,
    session_parity_diff, session_parity_issues, FoldError, FoldedSession, LogEvent, RebuildOptions,
    RebuildReport, INDEX_ONLY_STATE_FIELDS,
};

pub use usage::{
    last_context_input_tokens, session_usage_totals, turn_usage_totals,
    turn_usage_totals_from_snapshot, SessionUsageLedger, SessionUsageTotals,
};
pub use workspace_snapshot::WorkspaceSnapshotManager;

pub use backend::{
    BackfillReport, CommitOutcome, PersistCommand, PersistCommandOutcome, SeparateTraceTableMode,
    SessionBackend, SessionBackendMutationResult, SessionBackendTraceLoadResult, SessionMetaPatch,
    SessionTraceMutation, TraceMigrationState, TraceTerminalPatch, TurnStateRecord,
};
pub use file_backend::FileSessionBackend;
pub use id::{
    classify_session_id, is_canonical_session_id, is_main_session_id, new_session_id,
    new_session_id_unless, SessionIdKind, MAX_UNIQUE_ATTEMPTS, SESSION_ID_PREFIX,
};
pub use store::{
    build_missing_run_control_audit_summary, interrupted_turn_checkpoint_text,
    is_host_continuation_prompt, session_is_continuation_fragment, PersistedStore, SessionStore,
    APPROVAL_APPROVED_CONTINUATION_PROMPT, APPROVAL_DENIED_CONTINUATION_PROMPT,
    ASK_RESUME_CONTINUATION_PROMPT,
};
pub use types::{
    collect_env_info, AttachmentAsset, AttachmentAssetQuery, AttachmentCleanupRequest,
    AttachmentCleanupResult, AttachmentLifecycleStatus, AttachmentReference, EnvironmentInfo,
    HistoryBranch, HistoryCheckoutMode, HistoryCheckoutStatus, HistoryCursor, HistoryCursorMode,
    HistoryNode, HistoryNodeKind, HistoryStateAuditActionSummary, HistoryStateAuditCurrentContext,
    HistoryStateAuditSummary, LongTermMemoryRecord, MessageStatus, RunControlAuditActionSummary,
    RunControlAuditCurrentContext, RunControlAuditSummary, SessionAttachment, SessionError,
    SessionOverview, SessionSnapshot, SessionState, SessionVersion, SessionVersionMap,
    TraceTimelineEntry, TurnHistoryMessage, TurnTraceRecord, TurnTraceRef, WorkspaceRef,
    WorkspaceRefKind,
};
// 生产路径仅用 DEFAULT_SESSION_ID；两个附件类型仅 control_plane 测试模块经
// 本模块命名空间引用（见下方测试供给区同款模式），故按测试构建条件导出。
pub(crate) use types::DEFAULT_SESSION_ID;
#[cfg(test)]
pub(crate) use types::{AttachmentAssetMap, SessionAttachmentIndex};

// 测试供给区：tests.rs 的 use super::*; 依赖本模块命名空间提供这些名字
#[cfg(test)]
use crate::agent::hooks::{
    HistoryStateHookEnvelope, HistoryStateHookExecutor, HistoryStateHookPoint,
    HookPatchOperationKind, HookPatchTarget, HookResultKind, HookStructuredResult, HookTraceRecord,
    MemoryWriteHookEnvelope, MemoryWriteHookExecutor,
};
#[cfg(test)]
use crate::agent::input::TurnInputImage;
#[cfg(test)]
use crate::agent::telemetry::{ProviderCallCacheRecord, TurnToolActivity, TurnTraceStep};
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::path::PathBuf;
#[cfg(test)]
use store::{
    attachment_asset_id, collect_trace_union, enrich_history_from_traces, fold_session_views,
    load_store_from_path, materialize_last_turn_messages, now_timestamp_ms,
    refresh_session_metadata, session_state_for_backend,
};
#[cfg(test)]
use types::{
    DEFAULT_ATTACHMENT_RECLAIM_TTL_MS, DEFAULT_HISTORY_BRANCH_ID, DEFAULT_SESSION_SUMMARY,
    DEFAULT_SESSION_TITLE,
};
