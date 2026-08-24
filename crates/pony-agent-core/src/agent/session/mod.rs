mod backend;
mod file_backend;
mod store;
#[cfg(test)]
mod tests;
mod types;

pub use backend::{
    PersistCommand, PersistCommandOutcome, SeparateTraceTableMode, SessionBackend,
    SessionBackendMutationResult, SessionBackendTraceLoadResult, SessionMetaPatch,
    SessionTraceMutation, TraceMigrationState, TraceTerminalPatch, TurnStateRecord,
};
pub use file_backend::FileSessionBackend;
pub use store::{build_missing_run_control_audit_summary, PersistedStore, SessionStore};
pub use types::{
    collect_env_info, AttachmentAsset, AttachmentAssetQuery, AttachmentCleanupRequest,
    AttachmentCleanupResult, AttachmentLifecycleStatus, AttachmentReference, EnvironmentInfo,
    HistoryBranch, HistoryCheckoutMode, HistoryCheckoutStatus, HistoryCursor, HistoryCursorMode,
    HistoryNode, HistoryNodeKind, HistoryStateAuditActionSummary, HistoryStateAuditCurrentContext,
    HistoryStateAuditSummary, LongTermMemoryRecord, MessageStatus, RunControlAuditActionSummary,
    RunControlAuditCurrentContext, RunControlAuditSummary, SessionAttachment, SessionOverview,
    SessionSnapshot, SessionState, TraceTimelineEntry, TurnHistoryMessage, TurnTraceRecord,
    TurnTraceRef, WorkspaceRef, WorkspaceRefKind,
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
    session_state_for_backend,
};
#[cfg(test)]
use types::{
    DEFAULT_ATTACHMENT_RECLAIM_TTL_MS, DEFAULT_HISTORY_BRANCH_ID, DEFAULT_SESSION_SUMMARY,
    DEFAULT_SESSION_TITLE,
};
