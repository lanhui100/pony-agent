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
// ADR 0099 阶段 3-A：会话目录包（`manifest.json` + `events.jsonl` + `attachments/`）导出/导入。
pub mod bundle;
// PA-174 P1：投影侧全空时从 durable `turn_events` 重建历史（纯函数）。
mod history;
// ADR 0099 阶段 3-B：存量会话日志回填（grandfather 转正，方案 A）。
pub mod log_migrate;
mod store;
mod transcript_sanitize;
// 启动装载期的有界修复（空壳会话派生字段保护 + trace 投影缓存预算）。
pub(crate) mod startup_repair;
// P1 启动提速：`pony resume` 目标解析的廉价 meta 查询（单条只读 SELECT，不做整库装载）。
pub mod startup_meta;
#[cfg(test)]
mod tests;
// file-size 收口：新增用例不再堆进 `tests.rs`（内存后端最小闭环先行拆出）。
#[cfg(test)]
mod memory_backend_tests;
// ADR 0092 回归：goal 续轮文案识别（单独成文件，避免撑大已触棘轮的 `tests.rs`）。
#[cfg(test)]
mod host_continuation_goal_round_tests;
// 历史治理门禁：追加语义 / 压缩双写 / 压缩可重放 / 事件物化（同上，避免撑大 `tests.rs`）。
#[cfg(test)]
mod history_compaction_tests;
mod types;
// 中断/失败轮次的闭合真值（补齐优先于截断；事故 pa_01a1ce16）：纯函数出口。
pub mod interrupted_turn;
pub mod usage;
pub mod workspace_snapshot;
// 「撤回」：轮次级文件回滚快照（写工具落盘前的 pre-image，契约见模块文档）。
pub mod rewind_files;
// 「撤回」：活动链定位 + 空会话 checkpoint 根补齐（ADR 0113）。
mod rewind_store;

// ADR 0099 阶段 1-A：仅 additive 导出。
pub use log_store::{
    encode_session_id_segment, normalize_cwd_segment, read_header, read_header_source,
    read_header_source_strict, HeaderClass, LogReader, LogStoreError, LogWriter,
    NewSessionLogEvent, SessionLogEvent, SessionLogHeader, SessionLogMeta, SessionLogSource,
    SessionLogStore, SESSION_LOG_FILE_NAME, SESSION_LOG_FORMAT_VERSION,
};

// ADR 0099 阶段 3-B：additive 导出（迁移器 / 报告 / 比对豁免清单）。
pub use log_migrate::{
    migrate_grandfathered_sessions, migrated_session_ids, synthesize_log_events, MigrateOptions,
    MigrateReport, MIGRATION_PARITY_EXEMPTIONS,
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

// ADR 0099 阶段 3-A：additive 导出（CLI 与验收测试共用同一套打包/校验口径）。
pub use bundle::{
    export_session, import_bundle, BundleError, BundleFileEntry, BundleFormat, BundleManifest,
    BundleSource, ExportReport, ImportOptions, ImportReport, BUNDLE_ATTACHMENTS_DIR_NAME,
    BUNDLE_EVENTS_FILE_NAME, BUNDLE_FORMAT, BUNDLE_MANIFEST_FILE_NAME, BUNDLE_VERSION,
};

pub use usage::{
    last_context_input_tokens, session_usage_totals, turn_usage_totals,
    turn_usage_totals_from_snapshot, SessionUsageLedger, SessionUsageTotals,
};
pub use workspace_snapshot::WorkspaceSnapshotManager;
// 「撤回」文件快照：写工具（抓取）与撤回命令（恢复）共用同一入口。
pub use rewind_files::{
    default_rewind_root, RewindFileStore, RewindRestoreOutcome, MAX_SNAPSHOT_BYTES,
    SNAPSHOT_ROOT_ENV,
};
pub use rewind_store::RewindPlan;

// PA-174 P1：投影侧全空（`__index_only` + 零 `normalized_*` 行）时，装载侧用它与
// `turn_events` 重建历史，避免重启后历史为零（空壳）。
pub use history::history_from_turn_events;
// PA-174 B1：按活动血缘过滤的重建入口（装载侧专用，不对外承诺）。
pub(crate) use history::history_from_branch_events;

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
pub use startup_meta::{
    load_session_meta_rows, resolve_from_meta_rows, resolve_resume_target, SessionMetaRow,
    NO_RESUMABLE_SESSION_MESSAGE,
};
pub use store::{
    build_missing_run_control_audit_summary, ensure_history_graph, failed_turn_checkpoint_text,
    interrupted_turn_checkpoint_text, is_interrupt_placeholder_text, is_host_continuation_prompt,
    session_is_continuation_fragment, PersistedStore, SessionStore,
    APPROVAL_APPROVED_CONTINUATION_PROMPT, APPROVAL_DENIED_CONTINUATION_PROMPT,
    ASK_RESUME_CONTINUATION_PROMPT, UPSTREAM_INTERRUPTED_NOTE, USER_CANCELLED_NOTE,
};
pub use types::{
    collect_env_info, AttachmentAsset, AttachmentAssetQuery, AttachmentCleanupRequest,
    AttachmentCleanupResult, AttachmentLifecycleStatus, AttachmentReference, EnvironmentInfo,
    HistoryBranch, HistoryCheckoutMode, HistoryCheckoutStatus, HistoryCursor, HistoryCursorMode,
    CANCELLED_TURN_SENTINEL, HistoryMessageKind, HistoryNode, HistoryNodeKind, HistoryStateAuditActionSummary, HistoryStateAuditCurrentContext,
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
/// 历史**紧急上限**（条消息）：写路径、投影侧、上下文组装共用同一常量。
///
/// 单一口径是刻意的——任一侧自己留一个副本就是一条新的隐式滑窗。
pub(crate) use types::HISTORY_EMERGENCY_LIMIT;
