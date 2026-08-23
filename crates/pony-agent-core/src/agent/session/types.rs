use super::backend::TraceMigrationState;
use super::store::{commit_history_node_from_live_state, default_session_title};
use crate::agent::hooks::{
    merge_patch_results, HistoryStateCommandKind, HistoryStateCursorSummary,
    HistoryStateHookEnvelope, HistoryStateHookEvidence, HistoryStateHookExecutor,
    HistoryStateHookPoint, HookPatchConflictPolicy, HookPatchOperationKind, HookPatchTarget,
    HookResultKind, HookStructuredResult, HookTraceRecord, MemoryWriteHookEnvelope,
    MemoryWriteHookExecutor, MemoryWriteHookPoint, MemoryWriteIntentRecord, MemoryWriteOperation,
    MemoryWriteTarget, NoopHistoryStateHookExecutor, NoopMemoryWriteHookExecutor,
    PersistedEffectEvidence,
};
use crate::agent::input::TurnInputImage;
use crate::agent::provider::BuildContextObservation;
use crate::agent::telemetry::{ProviderCallCacheRecord, TurnToolActivity, TurnTraceStep};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub(crate) const DEFAULT_SESSION_ID: &str = "local-dev-session";
pub(super) const DEFAULT_SESSION_SUMMARY: &str = "Pony Agent 本地开发会话";
pub(super) const DEFAULT_HISTORY_LIMIT: usize = 24;
pub(super) const DEFAULT_SESSION_TITLE: &str = "\u{65B0}\u{5BF9}\u{8BDD}";
pub(super) const TITLE_MAX_CHARS: usize = 28;
pub(super) const DEFAULT_ATTACHMENT_RECLAIM_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;
pub(super) const DEFAULT_HISTORY_BRANCH_ID: &str = "branch-main";

pub(super) type SessionMap = HashMap<String, SessionState>;
pub(crate) type AttachmentAssetMap = HashMap<String, AttachmentAsset>;
pub(crate) type SessionAttachmentIndex = HashMap<String, Vec<String>>;
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryNodeKind {
    #[default]
    TurnCommitted,
    TurnCancelled,
    RunPaused,
    Checkpoint,
    ManualSnapshot,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
    #[default]
    Done,
    Error,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryCursorMode {
    #[default]
    Live,
    Historical,
    HistoricalDirty,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceRefKind {
    #[default]
    None,
    GitCommit,
    PatchSet,
    HostSnapshot,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryCheckoutMode {
    #[default]
    TranscriptOnly,
    TranscriptAndWorkspace,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryCheckoutStatus {
    #[default]
    NotRequested,
    Applied,
    DegradedToTranscriptOnly,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceRef {
    #[serde(default)]
    pub kind: WorkspaceRefKind,
    pub locator: Option<String>,
    #[serde(default)]
    pub rollback_capable: bool,
}

/// 轻量 trace 引用（PA-088）：持久化层用引用替代节点内嵌的完整 trace。
/// `None`（字段缺失）= legacy/旧数据（可用内嵌 trace 兜底）；
/// `Some([])` = authoritative 且确实无 trace（禁止兜底）；
/// `Some(v)` = 有序引用（materialize 时按此顺序组装）。
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnTraceRef {
    pub turn_id: String,
    pub updated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryNode {
    pub node_id: String,
    pub session_id: String,
    pub parent_node_id: Option<String>,
    pub branch_id: String,
    pub forked_from_node_id: Option<String>,
    #[serde(default)]
    pub kind: HistoryNodeKind,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub workspace_ref: WorkspaceRef,
    pub summary: String,
    pub title: String,
    #[serde(default)]
    pub history: Vec<TurnHistoryMessage>,
    #[serde(default)]
    pub provider_native_transcript: Vec<Value>,
    #[serde(default)]
    pub turn_trace_history: Vec<TurnTraceRecord>,
    /// PA-088：节点归属的 turn（前端 checkpoint/回滚依赖，替代从 trace 末条推导）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    /// PA-088：持久化时生成的轻量 trace 引用（节点不再内嵌完整 trace）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_trace_refs: Option<Vec<TurnTraceRef>>,
    /// PA-093：节点覆盖的事件区间（引用化）。`None` = legacy（内嵌快照兜底）；
    /// `Some((start, end))` = 该节点状态可由事件流 seq [start, end] 折叠重建
    /// （新节点不再内嵌 history/transcript/trace 快照，checkout 时水位回退）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_seq_range: Option<(u64, u64)>,
    #[serde(default)]
    pub long_term_memory_entries: Vec<LongTermMemoryRecord>,
    #[serde(default)]
    pub memory_write_evidence: Vec<PersistedEffectEvidence>,
    #[serde(default)]
    pub memory_write_hook_trace_records: Vec<HookTraceRecord>,
    #[serde(default)]
    pub turn_count: usize,
    pub last_referenced_file: Option<String>,
    #[serde(default)]
    pub created_at_ms: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryBranch {
    pub branch_id: String,
    pub session_id: String,
    pub base_node_id: Option<String>,
    pub head_node_id: Option<String>,
    pub forked_from_branch_id: Option<String>,
    pub forked_from_node_id: Option<String>,
    pub label: String,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default)]
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryCursor {
    #[serde(default)]
    pub session_id: String,
    pub visible_node_id: Option<String>,
    pub active_branch_id: Option<String>,
    pub branch_head_node_id: Option<String>,
    pub workspace_node_id: Option<String>,
    #[serde(default)]
    pub cursor_version: u64,
    /// PA-093：当前事件水位（已落盘事件总数，seq 0-based）。与 cursor_version
    /// 并存：cursor_version 保持旧语义（前端兼容），event_watermark 供前端
    /// 展示/后续水位校验（wire 新增字段，旧客户端不传则跳过校验）。
    #[serde(default)]
    pub event_watermark: u64,
    #[serde(default)]
    pub mode: HistoryCursorMode,
    #[serde(default)]
    pub checkout_mode: HistoryCheckoutMode,
    #[serde(default)]
    pub checkout_status: HistoryCheckoutStatus,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TurnHistoryMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentReference>,

    // PA-059: 新增元数据字段（均为 Option，向后兼容）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<MessageStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

impl TurnHistoryMessage {
    /// 稳定消息标识符，所有消费者（桌面/TUI/CLI/HTTP）可用。
    /// 格式："{turn_id}-{role}"，turn_id 缺失时 fallback "unknown-{role}"。
    /// 注意：方法不参与序列化，仅内存投影。
    pub fn stable_id(&self) -> String {
        format!(
            "{}-{}",
            self.turn_id.as_deref().unwrap_or("unknown"),
            self.role
        )
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentReference {
    pub id: String,
    #[serde(default)]
    pub asset_id: String,
    pub name: Option<String>,
    pub mime_type: String,
    pub relative_path: String,
    pub size_bytes: u64,
    pub created_at_ms: u64,
}

pub type SessionAttachment = AttachmentReference;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentLifecycleStatus {
    #[default]
    Active,
    MissingPayload,
    Expired,
    Reclaimable,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentAsset {
    pub id: String,
    pub session_id: String,
    pub name: Option<String>,
    pub mime_type: String,
    pub relative_path: String,
    pub size_bytes: u64,
    pub created_at_ms: u64,
    #[serde(default)]
    pub status: AttachmentLifecycleStatus,
    #[serde(default)]
    pub reference_count: usize,
    pub last_referenced_at_ms: Option<u64>,
    pub expires_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentAssetQuery {
    pub session_id: Option<String>,
    pub mime_type: Option<String>,
    pub name_contains: Option<String>,
    pub created_after_ms: Option<u64>,
    pub created_before_ms: Option<u64>,
    #[serde(default)]
    pub statuses: Vec<AttachmentLifecycleStatus>,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentCleanupRequest {
    pub session_id: Option<String>,
    pub expire_before_ms: Option<u64>,
    #[serde(default)]
    pub include_reclaimable: bool,
    #[serde(default)]
    pub include_expired: bool,
    pub limit: Option<usize>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentCleanupResult {
    pub removed_asset_ids: Vec<String>,
    pub removed_file_count: usize,
    pub removed_catalog_count: usize,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LongTermMemoryRecord {
    pub kind: String,
    pub content: String,
    #[serde(default)]
    pub source: String,
    #[serde(default)]
    pub updated_at_ms: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionState {
    pub conversation_id: String,
    #[serde(default = "default_session_title")]
    pub title: String,
    pub summary: String,
    pub history: Vec<TurnHistoryMessage>,
    #[serde(default)]
    pub provider_native_transcript: Vec<Value>,
    #[serde(default)]
    pub turn_trace_history: Vec<TurnTraceRecord>,
    #[serde(default)]
    pub trace_migration_state: TraceMigrationState,
    /// PA-088：当前可见分支的顶层 trace 引用（WriteSeparate+Authoritative 持久化用）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_trace_refs: Option<Vec<TurnTraceRef>>,
    #[serde(default)]
    pub long_term_memory_entries: Vec<LongTermMemoryRecord>,
    #[serde(default)]
    pub memory_write_evidence: Vec<PersistedEffectEvidence>,
    #[serde(default)]
    pub memory_write_hook_trace_records: Vec<HookTraceRecord>,
    #[serde(default)]
    pub history_state_evidence: Vec<HistoryStateHookEvidence>,
    pub turn_count: usize,
    pub last_referenced_file: Option<String>,
    #[serde(default)]
    pub updated_at_ms: u64,
    #[serde(default)]
    pub history_nodes: Vec<HistoryNode>,
    #[serde(default)]
    pub history_branches: Vec<HistoryBranch>,
    #[serde(default)]
    pub history_cursor: HistoryCursor,
    /// PA-093：会话事件水位（已落盘事件总数；turn 终态 flush 后由控制平面更新）。
    /// commit_history_node_from_live_state 以之为引用化区间的起点。
    #[serde(default)]
    pub event_watermark: u64,
    /// PA-093：最近一次节点提交时的水位（引用化区间起点；commit 时记录，
    /// finalize 时与 event_watermark 形成区间 [last_commit_watermark, new - 1]）。
    #[serde(default)]
    pub last_commit_watermark: u64,
    /// Workspace 归属（PA-079）：None → 投影为默认 workspace。serde default 兼容旧数据。
    #[serde(default)]
    pub workspace_id: Option<String>,
}
/// Runtime environment information captured at session snapshot build time.
/// Injected into the model context so the agent understands its execution environment.
///
/// Fields are collected without spawning subprocesses — reads env vars,
/// filesystem state (.git/HEAD), and compile-time constants only.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentInfo {
    /// Absolute working directory path.
    pub cwd: String,
    /// Platform identifier (e.g. "windows", "linux", "macos").
    pub platform: String,
    /// Shell executable name or path, if detectable.
    pub shell: Option<String>,
    /// Operating system version string, if available.
    pub os_version: Option<String>,
    /// Current date in ISO 8601 format (YYYY-MM-DD).
    pub current_date: String,
    /// Timezone abbreviation (e.g. "CST", "UTC").
    pub timezone: Option<String>,
    /// Whether the working directory is inside a git repository.
    pub is_git_repo: bool,
    /// Current git branch name, if detectable.
    pub git_branch: Option<String>,
}

/// Collects environment information at the current point in time.
/// Non-blocking: reads env vars, filesystem, and compile-time constants only.
pub fn collect_env_info() -> EnvironmentInfo {
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let now = chrono::Local::now();
    let is_git_repo = {
        let git_dir = Path::new(&cwd).join(".git");
        git_dir.is_dir() || git_dir.is_file()
    };
    let git_branch = if is_git_repo {
        read_git_branch(&cwd)
    } else {
        None
    };

    EnvironmentInfo {
        cwd,
        platform: std::env::consts::OS.to_string(),
        shell: std::env::var("SHELL")
            .ok()
            .or_else(|| std::env::var("ComSpec").ok()),
        os_version: None,
        current_date: now.format("%Y-%m-%d").to_string(),
        timezone: Some(now.format("%Z").to_string()),
        is_git_repo,
        git_branch,
    }
}

/// Reads git branch name from `.git/HEAD` without spawning a git process.
fn read_git_branch(workspace_root: &str) -> Option<String> {
    let head_path = Path::new(workspace_root).join(".git").join("HEAD");
    let content = fs::read_to_string(head_path).ok()?;
    let trimmed = content.trim();
    // "ref: refs/heads/main" → "main"
    if let Some(ref_path) = trimmed.strip_prefix("ref: refs/heads/") {
        Some(ref_path.trim().to_string())
    } else {
        // Detached HEAD — use commit hash prefix
        Some(trimmed.chars().take(7).collect())
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub conversation_id: String,
    pub title: String,
    pub summary: String,
    pub history: Vec<TurnHistoryMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachment_assets: Vec<AttachmentAsset>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_native_transcript: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turn_trace_history: Vec<TurnTraceRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub long_term_memory_entries: Vec<LongTermMemoryRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_write_evidence: Vec<PersistedEffectEvidence>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub memory_write_hook_trace_records: Vec<HookTraceRecord>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history_state_evidence: Vec<HistoryStateHookEvidence>,
    pub history_state_audit_summary: HistoryStateAuditSummary,
    pub run_control_audit_summary: RunControlAuditSummary,
    pub turn_count: usize,
    pub last_referenced_file: Option<String>,
    pub updated_at_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history_nodes: Vec<HistoryNode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history_branches: Vec<HistoryBranch>,
    #[serde(default)]
    pub history_cursor: HistoryCursor,
    pub resolved_node_id: Option<String>,
    pub latest_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_info: Option<EnvironmentInfo>,
    /// Workspace 归属投影（PA-079）：None → 默认 workspace。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<String>,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStateAuditActionSummary {
    pub status: String,
    pub source_family: String,
    pub command_kind: Option<String>,
    pub boundary: Option<String>,
    pub result_kind: Option<String>,
    pub summary: String,
    pub elapsed_ms: Option<u64>,
    pub blocked: bool,
    pub degraded: bool,
    pub evidence_id: Option<String>,
    pub observed_at_ms: Option<u64>,
    pub requested_node_id: Option<String>,
    pub requested_branch_id: Option<String>,
    pub resolved_node_id: Option<String>,
    pub resolved_branch_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStateAuditCurrentContext {
    pub mode: String,
    pub visible_node_id: Option<String>,
    pub active_branch_id: Option<String>,
    pub branch_head_node_id: Option<String>,
    pub workspace_node_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryStateAuditSummary {
    pub action: HistoryStateAuditActionSummary,
    pub current_context: HistoryStateAuditCurrentContext,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunControlAuditActionSummary {
    pub status: String,
    pub source_family: String,
    pub command_kind: Option<String>,
    pub boundary: Option<String>,
    pub result_kind: Option<String>,
    pub summary: String,
    pub target_summary: String,
    pub elapsed_ms: Option<u64>,
    pub blocked: bool,
    pub degraded: bool,
    pub evidence_id: Option<String>,
    pub observed_at_ms: Option<u64>,
    pub run_id: Option<String>,
    pub turn_id: Option<String>,
    pub checkpoint_turn_id: Option<String>,
    pub checkpoint_kind: Option<String>,
    pub recovery_mode: Option<String>,
    pub projected_command: Option<String>,
    pub degradation_reason: Option<String>,
    pub request_summary: Option<String>,
    pub start_reason: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunControlAuditCurrentContext {
    pub phase: String,
    pub checkpoint_status: String,
    pub active_run_id: Option<String>,
    pub checkpoint_kind: Option<String>,
    pub checkpoint_recovery_mode: Option<String>,
    pub submission_plan_command: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RunControlAuditSummary {
    pub action_evidence_summary: RunControlAuditActionSummary,
    pub current_context_projection: RunControlAuditCurrentContext,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOverview {
    pub conversation_id: String,
    pub title: String,
    pub summary: String,
    pub turn_count: usize,
    pub last_referenced_file: Option<String>,
    pub updated_at_ms: u64,
    /// Workspace 归属投影（PA-079）：None → 默认 workspace。
    pub workspace_id: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TraceTimelineEntry {
    pub id: String,
    pub kind: String,
    pub label: String,
    pub state: String,
    pub sequence: u64,
    pub provider_requested_name: Option<String>,
    pub provider_name: Option<String>,
    pub provider_protocol: Option<String>,
    pub provider_model: Option<String>,
    pub provider_source: Option<String>,
    pub provider_mode: Option<String>,
    pub build_context_observation: Option<BuildContextObservation>,
    /// PA-094：大字段外置引用（`bco:<turn_id>:<seq>`）。事件折叠重建的
    /// build_context 条目只带引用（全量 payload 按需加载）；legacy 内嵌数据
    /// 保留 `build_context_observation` 读取兼容。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_context_observation_ref: Option<String>,
    #[serde(default)]
    pub tool_activities: Vec<TurnToolActivity>,
    pub text: Option<String>,
    pub reasoning_content: Option<String>,
    pub fallback_reason: Option<String>,
    pub error: Option<String>,
    pub input_tokens: Option<u64>,
    pub cache_hit_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    pub turn_duration_ms: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnTraceRecord {
    pub turn_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(default)]
    pub event_type: Option<String>,
    #[serde(default)]
    pub event_version: Option<String>,
    #[serde(default)]
    pub sequence: Option<u64>,
    #[serde(default)]
    pub emitted_at_ms: Option<u64>,
    pub title: String,
    pub phase: String,
    #[serde(default)]
    pub trace_steps: Vec<TurnTraceStep>,
    #[serde(default)]
    pub trace_timeline: Vec<TraceTimelineEntry>,
    #[serde(default)]
    pub tool_activities: Vec<TurnToolActivity>,
    #[serde(default)]
    pub provider_call_records: Vec<ProviderCallCacheRecord>,
    #[serde(default)]
    pub hook_trace_records: Vec<HookTraceRecord>,
    pub provider_requested_name: Option<String>,
    pub provider_name: Option<String>,
    pub provider_protocol: Option<String>,
    pub provider_model: Option<String>,
    pub provider_source: Option<String>,
    pub provider_mode: Option<String>,
    pub build_context_observation: Option<BuildContextObservation>,
    /// PA-094：大字段外置引用（`bco:<turn_id>:<seq>`）。新数据走引用（事件折叠
    /// 产物），legacy 内嵌数据保留 `build_context_observation` 读取兼容。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build_context_observation_ref: Option<String>,
    pub session_summary: Option<String>,
    pub fallback_reason: Option<String>,
    pub error: Option<String>,
    pub input_tokens: Option<u64>,
    pub cache_hit_input_tokens: Option<u64>,
    pub reasoning_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    pub turn_duration_ms: Option<u64>,
    #[serde(default)]
    pub updated_at: u64,
}
