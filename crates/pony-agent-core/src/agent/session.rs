use crate::agent::capability_bridge::{McpSourceSnapshot, SkillSourceSnapshot};
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

const DEFAULT_SESSION_ID: &str = "local-dev-session";
const DEFAULT_SESSION_SUMMARY: &str = "Pony Agent 本地开发会话";
const DEFAULT_HISTORY_LIMIT: usize = 24;
const DEFAULT_SESSION_TITLE: &str = "\u{65B0}\u{5BF9}\u{8BDD}";
const TITLE_MAX_CHARS: usize = 28;
const DEFAULT_ATTACHMENT_RECLAIM_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;
const DEFAULT_HISTORY_BRANCH_ID: &str = "branch-main";

type SessionMap = HashMap<String, SessionState>;
type AttachmentAssetMap = HashMap<String, AttachmentAsset>;
type SessionAttachmentIndex = HashMap<String, Vec<String>>;

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

fn bump_cursor_version(cursor: &mut HistoryCursor) {
    cursor.cursor_version = cursor.cursor_version.saturating_add(1);
}

fn reject_stale_cursor_version(
    cursor: &HistoryCursor,
    expected_cursor_version: Option<u64>,
) -> Result<(), String> {
    let Some(expected) = expected_cursor_version else {
        return Ok(());
    };

    if expected == cursor.cursor_version {
        return Ok(());
    }

    Err(format!(
        "history cursor conflict: expected revision {}, actual revision {}",
        expected, cursor.cursor_version
    ))
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TraceMigrationState {
    #[default]
    LegacyBlob,
    DualWrite,
    TraceTableAuthoritative,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SeparateTraceTableMode {
    #[default]
    Off,
    DualWrite,
    WriteSeparate,
}

#[derive(Clone, Debug)]
pub enum SessionBackendTraceLoadResult {
    Unsupported,
    Loaded(Vec<TurnTraceRecord>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionBackendMutationResult {
    Unsupported,
    NotFound,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug)]
pub enum SessionTraceMutation {
    ReplaceAll {
        traces: Vec<TurnTraceRecord>,
    },
    UpsertOne {
        trace: TurnTraceRecord,
        trace_order: usize,
    },
    UpdateTerminalEvent {
        turn_id: String,
        event_id: Option<String>,
        event_type: Option<String>,
        event_version: Option<String>,
        sequence: Option<u64>,
        emitted_at_ms: Option<u64>,
        updated_at: u64,
    },
    AppendHookRecords {
        turn_id: String,
        hook_trace_records: Vec<HookTraceRecord>,
        updated_at: u64,
    },
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

/// PA-089 阶段 3：规范化双写命令（spec 3.2/3.7 定稿）。
/// 每个命令由 SqliteSessionBackend 在**统一事务**内同时写 blob（旧 sessions 表）
/// 与 normalized_* 表。`epoch` 用于迁移 barrier——旧 epoch 命令在 materialize 后拒绝。
#[derive(Clone, Debug)]
pub enum PersistCommand {
    AppendMessage {
        epoch: u64,
        session_id: String,
        message: TurnHistoryMessage,
        ordinal: usize,
    },
    UpsertTurn {
        epoch: u64,
        session_id: String,
        turn: TurnStateRecord,
    },
    AppendTrace {
        epoch: u64,
        session_id: String,
        trace: TurnTraceRecord,
        trace_order: usize,
    },
    UpdateTraceTerminal {
        epoch: u64,
        session_id: String,
        turn_id: String,
        terminal_patch: TraceTerminalPatch,
    },
    AppendHookRecords {
        epoch: u64,
        session_id: String,
        turn_id: String,
        hook_trace_records: Vec<HookTraceRecord>,
        updated_at: u64,
    },
    UpdateHistoryNode {
        epoch: u64,
        session_id: String,
        node: HistoryNode,
    },
    UpdateCursor {
        epoch: u64,
        session_id: String,
        cursor: HistoryCursor,
    },
    UpdateSessionMeta {
        epoch: u64,
        session_id: String,
        meta_patch: SessionMetaPatch,
    },
    RemoveSession {
        epoch: u64,
        session_id: String,
    },
    /// PA-091：turn 终态事件批次落盘（与快照写入同事务）。
    FlushEvents {
        epoch: u64,
        session_id: String,
        turn_id: String,
        /// PA-093：事件归属分支（阶段 3 起分支可见性推导的基石；legacy 数据 'main'）。
        branch_id: String,
        events: Vec<crate::agent::turn_event::TurnEvent>,
    },
    PublishMetadata {
        epoch: u64,
        key: String,
        value: String,
    },
}

/// turn 的规范化记录（normalized_turns 行）。
#[derive(Clone, Debug)]
pub struct TurnStateRecord {
    pub turn_id: String,
    pub ordinal: usize,
    pub phase: Option<String>,
    pub status: Option<String>,
    pub user_message_id: Option<String>,
    pub assistant_message_id: Option<String>,
    pub started_at_ms: Option<u64>,
    pub completed_at_ms: Option<u64>,
    pub updated_at_ms: u64,
}

/// trace 终态补丁（normalized_turn_traces 更新）。
#[derive(Clone, Debug)]
pub struct TraceTerminalPatch {
    pub event_id: Option<String>,
    pub event_type: Option<String>,
    pub event_version: Option<String>,
    pub sequence: Option<u64>,
    pub emitted_at_ms: Option<u64>,
    pub updated_at: u64,
    pub phase: Option<String>,
}

/// 会话元数据补丁（normalized_sessions 更新）。
#[derive(Clone, Debug)]
pub struct SessionMetaPatch {
    pub title: Option<String>,
    pub summary: Option<String>,
    pub turn_count: Option<usize>,
    pub last_referenced_file: Option<String>,
    pub updated_at_ms: Option<u64>,
}

/// 命令执行结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PersistCommandOutcome {
    Succeeded,
    Unsupported,
    /// 旧 epoch 命令被拒绝（materialize barrier 后）。
    StaleEpoch,
    Failed,
}

impl PersistCommand {
    /// 命令所属 epoch（迁移 barrier 用）。
    pub fn epoch(&self) -> u64 {
        match self {
            PersistCommand::AppendMessage { epoch, .. }
            | PersistCommand::UpsertTurn { epoch, .. }
            | PersistCommand::AppendTrace { epoch, .. }
            | PersistCommand::UpdateTraceTerminal { epoch, .. }
            | PersistCommand::AppendHookRecords { epoch, .. }
            | PersistCommand::UpdateHistoryNode { epoch, .. }
            | PersistCommand::UpdateCursor { epoch, .. }
            | PersistCommand::UpdateSessionMeta { epoch, .. }
            | PersistCommand::RemoveSession { epoch, .. }
            | PersistCommand::FlushEvents { epoch, .. }
            | PersistCommand::PublishMetadata { epoch, .. } => *epoch,
        }
    }
}

pub trait SessionBackend: Send + Sync {
    fn load_store(&self) -> Option<PersistedStore>;
    fn save_store(&self, store: &PersistedStore);
    fn trace_storage_mode(&self) -> SeparateTraceTableMode {
        SeparateTraceTableMode::Off
    }
    /// PA-089 阶段 3：规范化双写命令（blob + normalized_* 表统一事务）。
    /// 默认 Unsupported——File/Memory backend 无需实现；SqliteSessionBackend 实现。
    fn persist_command(&self, _command: PersistCommand) -> PersistCommandOutcome {
        PersistCommandOutcome::Unsupported
    }
    fn upsert_session(&self, _session_id: &str, _session: &SessionState) -> bool {
        false
    }
    fn remove_session(
        &self,
        _session_id: &str,
        _attachment_assets: &AttachmentAssetMap,
        _session_attachment_index: &SessionAttachmentIndex,
        _mcp_source_snapshots: &HashMap<String, McpSourceSnapshot>,
        _skill_source_snapshots: &HashMap<String, SkillSourceSnapshot>,
    ) -> bool {
        false
    }
    fn load_session_traces(&self, _session_id: &str) -> SessionBackendTraceLoadResult {
        SessionBackendTraceLoadResult::Unsupported
    }
    /// PA-093：读取会话事件流 `(seq, branch_id, event)`（seq 升序，可截断）。
    /// 默认返回空（File/Memory backend 不支持——引用化节点在非 SQLite 后端下
    /// checkout 退化为空视图并告警；生产路径恒为 SqliteSessionBackend）。
    fn load_turn_events(
        &self,
        _session_id: &str,
        _up_to_seq: Option<u64>,
    ) -> Vec<(u64, String, crate::agent::turn_event::TurnEvent)> {
        Vec::new()
    }
    /// PA-093：当前事件水位（已落盘事件总数）。默认 0（非 SQLite 后端无事件流）。
    fn load_event_watermark(&self, _session_id: &str) -> u64 {
        0
    }
    fn replace_session_traces(
        &self,
        _session_id: &str,
        _traces: &[TurnTraceRecord],
    ) -> SessionBackendMutationResult {
        SessionBackendMutationResult::Unsupported
    }
    fn upsert_turn_trace(
        &self,
        _session_id: &str,
        _trace: &TurnTraceRecord,
        _trace_order: usize,
    ) -> SessionBackendMutationResult {
        SessionBackendMutationResult::Unsupported
    }
    fn update_turn_trace_terminal_event(
        &self,
        _session_id: &str,
        _turn_id: &str,
        _event_id: Option<&str>,
        _event_type: Option<&str>,
        _event_version: Option<&str>,
        _sequence: Option<u64>,
        _emitted_at_ms: Option<u64>,
        _updated_at: u64,
    ) -> SessionBackendMutationResult {
        SessionBackendMutationResult::Unsupported
    }
    fn append_turn_trace_hook_records(
        &self,
        _session_id: &str,
        _turn_id: &str,
        _hook_trace_records: &[HookTraceRecord],
        _updated_at: u64,
    ) -> SessionBackendMutationResult {
        SessionBackendMutationResult::Unsupported
    }
    fn persist_session_with_trace_mutation(
        &self,
        _session_id: &str,
        _session: &SessionState,
        _mutation: SessionTraceMutation,
    ) -> SessionBackendMutationResult {
        SessionBackendMutationResult::Unsupported
    }
    fn attachment_root(&self) -> Option<PathBuf>;
}

pub struct SessionStore {
    sessions: SessionMap,
    attachment_assets: AttachmentAssetMap,
    session_attachment_index: SessionAttachmentIndex,
    mcp_source_snapshots: HashMap<String, McpSourceSnapshot>,
    skill_source_snapshots: HashMap<String, SkillSourceSnapshot>,
    /// Workspace 注册表（PA-079）。
    workspaces: Vec<crate::agent::workspace::WorkspaceRecord>,
    /// 路径读授权清单（PA-080）：内存 `AuthorizeStore`，持久化经 PersistedStore。
    path_authorizations: Arc<crate::agent::path_permission::AuthorizeStore>,
    backend: Box<dyn SessionBackend>,
    attachment_root: PathBuf,
    memory_write_hook_executor: Arc<dyn MemoryWriteHookExecutor>,
    history_state_hook_executor: Arc<dyn HistoryStateHookExecutor>,
    /// PA-093：节点 → 投影状态缓存 `(end_seq, history, trace)`。节点 commit 后其
    /// 事件区间不再追加 → 缓存天然有效（无需失效协议）；重启后由重折叠重建。
    /// 内存态，不参与持久化。
    projection_cache: HashMap<String, (u64, Vec<TurnHistoryMessage>, Vec<TurnTraceRecord>)>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedStore {
    pub(crate) sessions: SessionMap,
    #[serde(default)]
    pub(crate) attachment_assets: AttachmentAssetMap,
    #[serde(default)]
    pub(crate) session_attachment_index: SessionAttachmentIndex,
    #[serde(default)]
    pub(crate) mcp_source_snapshots: HashMap<String, McpSourceSnapshot>,
    #[serde(default)]
    pub(crate) skill_source_snapshots: HashMap<String, SkillSourceSnapshot>,
    /// Workspace 注册表（PA-079），随 full-store 持久化（SQLite store_metadata key=workspaces）。
    #[serde(default)]
    pub(crate) workspaces: Vec<crate::agent::workspace::WorkspaceRecord>,
    /// 路径读授权清单（PA-080），随 full-store 持久化（SQLite store_metadata
    /// key=`path_authorizations.v1`）；重启后生效。
    #[serde(default)]
    pub(crate) path_authorizations: Vec<crate::agent::path_permission::AuthorizedPathEntry>,
}

pub struct FileSessionBackend {
    storage_path: PathBuf,
}

#[cfg(test)]
pub struct MemorySessionBackend {
    attachment_root: PathBuf,
}

impl SessionStore {
    pub fn new() -> Self {
        use super::sqlite_session::{default_sqlite_path, SqliteSessionBackend};

        let sqlite_path = default_sqlite_path();
        // Ensure the parent directory exists for SQLite
        if let Some(parent) = sqlite_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let backend: Box<dyn SessionBackend> = Box::new(SqliteSessionBackend::new_with_trace_mode(
            sqlite_path,
            SeparateTraceTableMode::WriteSeparate,
        ));
        Self::with_backend(backend)
    }

    #[cfg(test)]
    pub fn memory_only() -> Self {
        Self::with_backend(Box::new(MemorySessionBackend {
            attachment_root: unique_test_session_dir("pony-agent-memory-attachments"),
        }))
    }

    pub fn with_backend(backend: Box<dyn SessionBackend>) -> Self {
        let attachment_root = backend
            .attachment_root()
            .unwrap_or_else(default_attachment_root);
        let persisted = backend.load_store().unwrap_or_default();
        let mut sessions = if persisted.sessions.is_empty() {
            default_sessions()
        } else {
            persisted.sessions
        };
        let mut attachment_assets = persisted.attachment_assets;
        let mut session_attachment_index = persisted.session_attachment_index;
        let mcp_source_snapshots = persisted.mcp_source_snapshots;
        let skill_source_snapshots = persisted.skill_source_snapshots;
        let mut should_save = false;
        // Workspace 注册表：加载 + 确保默认 workspace 始终存在（PA-079）。
        let mut workspaces = persisted.workspaces;
        if !crate::agent::workspace::default_workspace_exists(&workspaces) {
            let raw_default_root = std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .display()
                .to_string();
            // P2-6：默认 root 也过规范化（canonicalize + 去 \\?\ 前缀），与 create 路径一致，
            // 避免与 create_workspace_entry 存储形式不一致导致 PA-080 前缀比较失配。
            let default_root = crate::agent::workspace::normalize_workspace_root(&raw_default_root)
                .unwrap_or(raw_default_root);
            workspaces.push(crate::agent::workspace::WorkspaceRecord {
                id: crate::agent::workspace::DEFAULT_WORKSPACE_ID.to_string(),
                name: "默认工作区".to_string(),
                root_path: default_root,
            });
            should_save = true;
        }
        for session in sessions.values_mut() {
            refresh_session_metadata(session, false);
            if session.updated_at_ms == 0 {
                session.updated_at_ms = now_timestamp_ms();
            }
            if sanitize_attachment_references(session) {
                should_save = true;
            }
            if backfill_attachment_reference_assets(session) {
                should_save = true;
            }
        }
        let rebuilt_assets = rebuild_attachment_assets_from_sessions(&sessions, &attachment_assets);
        let rebuilt_index = rebuild_session_attachment_index(&sessions);
        if attachment_assets != rebuilt_assets {
            attachment_assets = rebuilt_assets;
            should_save = true;
        }
        if session_attachment_index != rebuilt_index {
            session_attachment_index = rebuilt_index;
            should_save = true;
        }
        let store = Self {
            sessions,
            attachment_assets,
            session_attachment_index,
            mcp_source_snapshots,
            skill_source_snapshots,
            workspaces,
            path_authorizations: Arc::new(
                crate::agent::path_permission::AuthorizeStore::from_entries(
                    persisted.path_authorizations,
                    None,
                ),
            ),
            backend,
            attachment_root,
            memory_write_hook_executor: Arc::new(NoopMemoryWriteHookExecutor),
            history_state_hook_executor: Arc::new(NoopHistoryStateHookExecutor),
            projection_cache: HashMap::new(),
        };
        if should_save {
            store.save_to_backend();
        }
        store
    }

    #[allow(dead_code)]
    pub fn snapshot(
        &mut self,
        session_id: Option<&str>,
        fallback_history: &[TurnHistoryMessage],
    ) -> SessionSnapshot {
        self.snapshot_at(session_id, None, fallback_history)
    }

    pub fn snapshot_at(
        &mut self,
        session_id: Option<&str>,
        node_id: Option<&str>,
        fallback_history: &[TurnHistoryMessage],
    ) -> SessionSnapshot {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID);
        let mut should_save = false;
        let mut should_refresh_catalog = false;
        {
            let session = self.ensure_session(session_key);
            if sanitize_provider_native_transcript(session) {
                should_save = true;
            }
            if session.history.is_empty() && !fallback_history.is_empty() {
                session.history = fallback_history.to_vec();
                refresh_session_metadata(session, false);
                should_save = true;
                should_refresh_catalog = true;
            }
            if ensure_history_graph(session) {
                should_save = true;
            }
        }
        if should_refresh_catalog {
            self.refresh_attachment_catalog();
        }
        let snapshot = self.snapshot_for_session_at(session_key, node_id);

        if should_save {
            self.save_to_backend();
        }

        snapshot
    }

    /// 只读快照：不创建 session、不修正数据、不触发任何落盘。
    /// 供可观测性/展示类查询使用（读锁即可），避免 trace 面板等
    /// 查询路径占用写锁或意外触发全量写库而阻塞主对话执行路径。
    pub fn snapshot_at_readonly(
        &self,
        session_id: Option<&str>,
        node_id: Option<&str>,
        fallback_history: &[TurnHistoryMessage],
    ) -> SessionSnapshot {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID);
        let Some(session) = self.sessions.get(session_key) else {
            return default_snapshot_for_session(session_key, node_id);
        };
        let attachment_assets = attachment_assets_for_query(
            &self.sessions,
            &self.attachment_assets,
            &self.session_attachment_index,
            &self.attachment_root,
            &AttachmentAssetQuery {
                session_id: Some(session_key.to_string()),
                ..AttachmentAssetQuery::default()
            },
            now_timestamp_ms(),
        );
        let mut snapshot = snapshot_from_state(session, attachment_assets, node_id);
        if snapshot.history.is_empty() && !fallback_history.is_empty() {
            snapshot.history = fallback_history.to_vec();
        }
        snapshot
    }

    pub fn append_turn(
        &mut self,
        session_id: Option<&str>,
        user_message: &str,
        assistant_message: &str,
        provider_native_transcript: Option<Vec<Value>>,
        attachments: Vec<SessionAttachment>,
    ) -> SessionSnapshot {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let memory_write_hook_executor = Arc::clone(&self.memory_write_hook_executor);
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
            prepare_session_for_new_turn(session);
            session.history.push(TurnHistoryMessage {
                role: "user".to_string(),
                content: user_message.to_string(),
                attachments,
                ..Default::default()
            });
            session.history.push(TurnHistoryMessage {
                role: "assistant".to_string(),
                content: assistant_message.to_string(),
                attachments: Vec::new(),
                ..Default::default()
            });

            if session.history.len() > DEFAULT_HISTORY_LIMIT {
                let keep_from = session.history.len() - DEFAULT_HISTORY_LIMIT;
                session.history.drain(..keep_from);
            }

            if let Some(messages) = provider_native_transcript {
                session.provider_native_transcript.extend(messages);
            }

            update_long_term_memory_from_user_message(
                session,
                user_message,
                memory_write_hook_executor.as_ref(),
            );
            refresh_session_metadata(session, true);
            commit_history_node_from_live_state(session, HistoryNodeKind::TurnCommitted, None);
        }
        self.refresh_attachment_catalog();
        let snapshot = self.snapshot_for_session(&session_key);

        self.save_to_backend();
        snapshot
    }

    pub fn append_failed_turn(
        &mut self,
        session_id: Option<&str>,
        user_message: &str,
        assistant_message: &str,
        mut trace: TurnTraceRecord,
    ) -> SessionSnapshot {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
            prepare_session_for_new_turn(session);
            session.history.push(TurnHistoryMessage {
                role: "user".to_string(),
                content: user_message.to_string(),
                attachments: Vec::new(),
                ..Default::default()
            });
            session.history.push(TurnHistoryMessage {
                role: "assistant".to_string(),
                content: assistant_message.to_string(),
                attachments: Vec::new(),
                ..Default::default()
            });
            trace.updated_at = now_timestamp_ms();
            session.turn_trace_history.push(trace);
            if session.history.len() > DEFAULT_HISTORY_LIMIT {
                let keep_from = session.history.len() - DEFAULT_HISTORY_LIMIT;
                session.history.drain(..keep_from);
            }
            if session.turn_trace_history.len() > DEFAULT_HISTORY_LIMIT {
                let keep_from = session.turn_trace_history.len() - DEFAULT_HISTORY_LIMIT;
                session.turn_trace_history = session.turn_trace_history[keep_from..].to_vec();
            }
            refresh_session_metadata(session, true);
            commit_history_node_from_live_state(
                session,
                classify_turn_node_kind(assistant_message),
                None,
            );
        }

        let snapshot = self.snapshot_for_session(&session_key);
        self.persist_session_and_trace_change(
            &session_key,
            SessionTraceMutation::ReplaceAll {
                traces: snapshot.turn_trace_history.clone(),
            },
        );
        snapshot
    }

    /// 替换会话的历史记录（用于上下文压缩后更新历史）
    pub fn replace_session_history(
        &mut self,
        session_id: Option<&str>,
        new_history: Vec<TurnHistoryMessage>,
    ) {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        {
            let session = self.ensure_session(&session_key);
            session.history = new_history;
            refresh_session_metadata(session, true);
            commit_history_node_from_live_state(session, HistoryNodeKind::TurnCommitted, None);
        }
        self.save_to_backend();
    }

    /// 仅做内存更新并返回待持久化的 trace 变更（SQLite 落库由调用方决定，
    /// 主对话热路径应通过后台队列异步落库，避免可观测性写盘阻塞主对话）。
    pub fn record_turn_trace_in_memory(
        &mut self,
        session_id: Option<&str>,
        mut trace: TurnTraceRecord,
    ) -> (String, SessionTraceMutation) {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
            trace.updated_at = now_timestamp_ms();

            if let Some(existing) = session
                .turn_trace_history
                .iter_mut()
                .find(|item| item.turn_id == trace.turn_id)
            {
                *existing = trace.clone();
            } else {
                session.turn_trace_history.push(trace.clone());
            }

            if session.turn_trace_history.len() > DEFAULT_HISTORY_LIMIT {
                let keep_from = session.turn_trace_history.len() - DEFAULT_HISTORY_LIMIT;
                session.turn_trace_history = session.turn_trace_history[keep_from..].to_vec();
            }

            refresh_session_metadata(session, true);
            sync_latest_history_node(session, Some(trace.turn_id.clone()));
        }
        let snapshot = self.snapshot_for_session(&session_key);
        let trace_order = snapshot
            .turn_trace_history
            .iter()
            .position(|item| item.turn_id == trace.turn_id)
            .unwrap_or(snapshot.turn_trace_history.len().saturating_sub(1));
        (
            session_key,
            SessionTraceMutation::UpsertOne {
                trace: trace.clone(),
                trace_order,
            },
        )
    }

    pub fn record_turn_trace(
        &mut self,
        session_id: Option<&str>,
        trace: TurnTraceRecord,
    ) -> SessionSnapshot {
        let (session_key, mutation) = self.record_turn_trace_in_memory(session_id, trace);
        let snapshot = self.snapshot_for_session(&session_key);
        self.persist_session_and_trace_change(&session_key, mutation);
        snapshot
    }

    /// 后台 trace 持久化 worker 的落库入口：与 persist_session_and_trace_change
    /// 行为一致（含失败回退），但只被后台线程调用，不阻塞主对话执行路径。
    pub fn persist_trace_mutation_from_worker(
        &mut self,
        session_id: &str,
        mutation: SessionTraceMutation,
    ) {
        self.persist_session_and_trace_change(session_id, mutation);
    }

    /// 仅做内存更新并返回待持久化的 trace 变更（供异步落库使用）。
    pub fn annotate_turn_trace_terminal_event_in_memory(
        &mut self,
        session_id: Option<&str>,
        turn_id: &str,
        event_id: Option<String>,
        event_type: Option<String>,
        event_version: Option<String>,
        sequence: Option<u64>,
        emitted_at_ms: Option<u64>,
    ) -> Option<(String, SessionTraceMutation)> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let persisted_event_id = event_id.clone();
        let persisted_event_type = event_type.clone();
        let persisted_event_version = event_version.clone();
        {
            let session = self.sessions.get_mut(&session_key)?;
            let trace = session
                .turn_trace_history
                .iter_mut()
                .find(|item| item.turn_id == turn_id)?;
            trace.session_id = Some(session_key.clone());
            trace.event_id = event_id;
            trace.event_type = event_type;
            trace.event_version = event_version;
            trace.sequence = sequence;
            trace.emitted_at_ms = emitted_at_ms;
            trace.updated_at = now_timestamp_ms();
            refresh_session_metadata(session, true);
        }
        let updated_at = self
            .sessions
            .get(&session_key)
            .and_then(|session| {
                session
                    .turn_trace_history
                    .iter()
                    .find(|item| item.turn_id == turn_id)
                    .map(|trace| trace.updated_at)
            })
            .unwrap_or_default();
        Some((
            session_key,
            SessionTraceMutation::UpdateTerminalEvent {
                turn_id: turn_id.to_string(),
                event_id: persisted_event_id,
                event_type: persisted_event_type,
                event_version: persisted_event_version,
                sequence,
                emitted_at_ms,
                updated_at,
            },
        ))
    }

    pub fn annotate_turn_trace_terminal_event(
        &mut self,
        session_id: Option<&str>,
        turn_id: &str,
        event_id: Option<String>,
        event_type: Option<String>,
        event_version: Option<String>,
        sequence: Option<u64>,
        emitted_at_ms: Option<u64>,
    ) -> Option<SessionSnapshot> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let persisted_event_id = event_id.clone();
        let persisted_event_type = event_type.clone();
        let persisted_event_version = event_version.clone();
        {
            let session = self.sessions.get_mut(&session_key)?;
            let trace = session
                .turn_trace_history
                .iter_mut()
                .find(|item| item.turn_id == turn_id)?;
            trace.session_id = Some(session_key.clone());
            trace.event_id = event_id;
            trace.event_type = event_type;
            trace.event_version = event_version;
            trace.sequence = sequence;
            trace.emitted_at_ms = emitted_at_ms;
            trace.updated_at = now_timestamp_ms();
            refresh_session_metadata(session, true);
        }
        let updated_at = self
            .sessions
            .get(&session_key)
            .and_then(|session| {
                session
                    .turn_trace_history
                    .iter()
                    .find(|item| item.turn_id == turn_id)
                    .map(|trace| trace.updated_at)
            })
            .unwrap_or_default();
        let snapshot = self.snapshot_for_session(&session_key);
        self.persist_session_and_trace_change(
            &session_key,
            SessionTraceMutation::UpdateTerminalEvent {
                turn_id: turn_id.to_string(),
                event_id: persisted_event_id,
                event_type: persisted_event_type,
                event_version: persisted_event_version,
                sequence,
                emitted_at_ms,
                updated_at,
            },
        );
        Some(snapshot)
    }

    /// 仅做内存更新并返回待持久化的 trace 变更（供异步落库使用）。
    /// hook 记录为空或无匹配 trace 时返回 None，表示无需持久化。
    pub fn append_turn_trace_hook_records_in_memory(
        &mut self,
        session_id: Option<&str>,
        turn_id: &str,
        hook_trace_records: Vec<HookTraceRecord>,
    ) -> Option<(String, SessionTraceMutation)> {
        if hook_trace_records.is_empty() {
            return None;
        }

        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let persisted_hook_trace_records = hook_trace_records.clone();
        {
            let session = self.sessions.get_mut(&session_key)?;
            let trace = session
                .turn_trace_history
                .iter_mut()
                .find(|item| item.turn_id == turn_id)?;
            trace.session_id = Some(session_key.clone());
            trace.hook_trace_records.extend(hook_trace_records);
            trace.updated_at = now_timestamp_ms();
            refresh_session_metadata(session, true);
            sync_latest_history_node(session, Some(turn_id.to_string()));
        }
        let updated_at = self
            .sessions
            .get(&session_key)
            .and_then(|session| {
                session
                    .turn_trace_history
                    .iter()
                    .find(|item| item.turn_id == turn_id)
                    .map(|trace| trace.updated_at)
            })
            .unwrap_or_default();
        Some((
            session_key,
            SessionTraceMutation::AppendHookRecords {
                turn_id: turn_id.to_string(),
                hook_trace_records: persisted_hook_trace_records,
                updated_at,
            },
        ))
    }

    pub fn append_turn_trace_hook_records(
        &mut self,
        session_id: Option<&str>,
        turn_id: &str,
        hook_trace_records: Vec<HookTraceRecord>,
    ) -> Option<SessionSnapshot> {
        if hook_trace_records.is_empty() {
            return Some(self.snapshot(session_id, &[]));
        }

        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let persisted_hook_trace_records = hook_trace_records.clone();
        {
            let session = self.sessions.get_mut(&session_key)?;
            let trace = session
                .turn_trace_history
                .iter_mut()
                .find(|item| item.turn_id == turn_id)?;
            trace.session_id = Some(session_key.clone());
            trace.hook_trace_records.extend(hook_trace_records);
            trace.updated_at = now_timestamp_ms();
            refresh_session_metadata(session, true);
            sync_latest_history_node(session, Some(turn_id.to_string()));
        }
        let updated_at = self
            .sessions
            .get(&session_key)
            .and_then(|session| {
                session
                    .turn_trace_history
                    .iter()
                    .find(|item| item.turn_id == turn_id)
                    .map(|trace| trace.updated_at)
            })
            .unwrap_or_default();
        let snapshot = self.snapshot_for_session(&session_key);
        self.persist_session_and_trace_change(
            &session_key,
            SessionTraceMutation::AppendHookRecords {
                turn_id: turn_id.to_string(),
                hook_trace_records: persisted_hook_trace_records,
                updated_at,
            },
        );
        Some(snapshot)
    }

    pub fn persist_mcp_source_snapshot(&mut self, snapshot: McpSourceSnapshot) {
        self.mcp_source_snapshots
            .insert(snapshot.source.source_id.clone(), snapshot);
        self.save_to_backend();
    }

    pub fn persist_skill_source_snapshot(&mut self, snapshot: SkillSourceSnapshot) {
        self.skill_source_snapshots
            .insert(snapshot.source.source_id.clone(), snapshot);
        self.save_to_backend();
    }

    pub fn list_persisted_mcp_source_snapshots(&self) -> Vec<McpSourceSnapshot> {
        self.mcp_source_snapshots.values().cloned().collect()
    }

    pub fn list_persisted_skill_source_snapshots(&self) -> Vec<SkillSourceSnapshot> {
        self.skill_source_snapshots.values().cloned().collect()
    }

    #[cfg(test)]
    pub fn set_memory_write_hook_executor_for_test(
        &mut self,
        executor: Box<dyn MemoryWriteHookExecutor>,
    ) {
        self.memory_write_hook_executor = executor.into();
    }

    #[cfg(test)]
    pub fn set_history_state_hook_executor_for_test(
        &mut self,
        executor: Box<dyn HistoryStateHookExecutor>,
    ) {
        self.history_state_hook_executor = executor.into();
    }

    #[allow(dead_code)]
    pub fn replace_long_term_memory(
        &mut self,
        session_id: Option<&str>,
        entries: Vec<LongTermMemoryRecord>,
    ) -> SessionSnapshot {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
            session.long_term_memory_entries = entries;
            session.memory_write_evidence.clear();
            session.memory_write_hook_trace_records.clear();
            session.history_state_evidence.clear();
            refresh_session_metadata(session, true);
            sync_latest_history_node(session, None);
        }
        let snapshot = self.snapshot_for_session(&session_key);

        self.save_session_to_backend(&session_key);
        snapshot
    }

    #[allow(dead_code)]
    pub fn checkout_history_node(
        &mut self,
        session_id: Option<&str>,
        node_id: &str,
        mode: HistoryCheckoutMode,
        expected_cursor_version: Option<u64>,
    ) -> Result<SessionSnapshot, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let hook_executor = Arc::clone(&self.history_state_hook_executor);
        let mut blocked_error = None;
        // PA-093：块外预折叠——引用化节点由事件流重建视图（&mut self 与
        // ensure_session 借用互斥，先算结果，块内以纯函数应用）。
        let fold_result: Option<(Vec<TurnHistoryMessage>, Vec<TurnTraceRecord>)> = {
            let pre_node = self
                .sessions
                .get(&session_key)
                .and_then(|s| history_node(s, node_id))
                .cloned();
            pre_node
                .as_ref()
                .filter(|n| n.event_seq_range.is_some())
                .map(|n| self.fold_node_views(&session_key, n))
        };
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
            reject_stale_cursor_version(&session.history_cursor, expected_cursor_version)?;
            let requested_mode = mode.clone();
            if let Some(start_envelope) = build_history_state_hook_envelope(
                session,
                HistoryStateHookPoint::HistoryCheckoutStart,
                HistoryStateCommandKind::CheckoutHistoryNode,
                Some(node_id),
                None,
                Some(&requested_mode),
                None,
                None,
                false,
                false,
                false,
                false,
                None,
            ) {
                let hook_results = hook_executor.execute(&start_envelope).unwrap_or_default();
                persist_history_state_hook_evidence(session, &start_envelope, &hook_results);
                if history_state_hook_results_blocked(&hook_results) {
                    refresh_session_metadata(session, true);
                    blocked_error = Some(format!(
                        "history checkout blocked by hook before resolving node: {node_id}"
                    ));
                }
            }
            if blocked_error.is_some() {
                // Keep the current truth-source unchanged when guard hooks block checkout.
                sync_latest_history_node(session, None);
            } else {
                // ── Build node index for O(1) lookups ──
                let node_index: HashMap<&str, usize> = session
                    .history_nodes
                    .iter()
                    .enumerate()
                    .map(|(i, n)| (n.node_id.as_str(), i))
                    .collect();

                let Some(&node_idx) = node_index.get(node_id) else {
                    return Err(format!("unknown history node: {node_id}"));
                };
                let node = session.history_nodes[node_idx].clone();

                // ── Truncate history: mark descendant nodes as cancelled ──
                // Collect all ancestor node IDs (including the target node itself)
                // using the index for O(1) lookups instead of O(n) linear scans.
                let mut ancestor_ids = HashSet::new();
                let mut current = Some(node.node_id.as_str());
                while let Some(id) = current {
                    ancestor_ids.insert(id.to_string());
                    current = node_index
                        .get(id)
                        .and_then(|&i| session.history_nodes[i].parent_node_id.as_deref());
                }
                let branch_id = node.branch_id.clone();
                // Drop the index to release the immutable borrow before mutating
                drop(node_index);

                for n in &mut session.history_nodes {
                    if n.branch_id == branch_id && !ancestor_ids.contains(&n.node_id) {
                        n.kind = HistoryNodeKind::TurnCancelled;
                    }
                }
                // ── Update branch head to point to the target node ──
                // This makes the truncation permanent: the target node IS now the
                // branch head, so subsequent restore_branch_head / new turns will
                // build from here instead of resurrecting the cancelled nodes.
                if let Some(branch) = session
                    .history_branches
                    .iter_mut()
                    .find(|b| b.branch_id == branch_id)
                {
                    branch.head_node_id = Some(node.node_id.clone());
                }
                // PA-093：引用化节点 → 事件折叠重建视图（水位回退）；legacy → 旧路径。
                match &fold_result {
                    Some((history, trace)) => hydrate_session_from_projection(
                        session,
                        &node,
                        history.clone(),
                        trace.clone(),
                    ),
                    None => hydrate_session_from_node(session, &node),
                }
                session.history_cursor.visible_node_id = Some(node.node_id.clone());
                session.history_cursor.active_branch_id = Some(branch_id.clone());
                session.history_cursor.branch_head_node_id = Some(node.node_id.clone());
                session.history_cursor.workspace_node_id = Some(node.node_id.clone());
                // Target node is now the branch head, so mode is always Live.
                session.history_cursor.mode = HistoryCursorMode::Live;
                session.history_cursor.checkout_mode = requested_mode.clone();
                session.history_cursor.checkout_status = match requested_mode {
                    HistoryCheckoutMode::TranscriptOnly => HistoryCheckoutStatus::Applied,
                    HistoryCheckoutMode::TranscriptAndWorkspace => {
                        if node.workspace_ref.rollback_capable {
                            HistoryCheckoutStatus::Applied
                        } else {
                            HistoryCheckoutStatus::DegradedToTranscriptOnly
                        }
                    }
                };
                bump_cursor_version(&mut session.history_cursor);
                // PA-093：水位同步到 cursor（wire 面世，前端可获知当前事件水位）。
                session.history_cursor.event_watermark = session.event_watermark;
                // PA-093：checkout 是视图切换事实，append 事件（分支可见性推导基石）。
                crate::agent::turn_flow::emit_global_event(
                    &session_key,
                    "checkpoint",
                    crate::agent::turn_event::TurnEvent::CheckpointCheckout {
                        node_id: node.node_id.clone(),
                        mode: serde_json::to_string(&requested_mode)
                            .unwrap_or_else(|_| "transcript_only".to_string()),
                    },
                );
                refresh_session_metadata(session, true);
                if let Some(resolved_envelope) = build_history_state_hook_envelope(
                    session,
                    HistoryStateHookPoint::HistoryCheckoutResolved,
                    HistoryStateCommandKind::CheckoutHistoryNode,
                    Some(node_id),
                    None,
                    Some(&requested_mode),
                    Some(node.node_id.as_str()),
                    Some(node.branch_id.as_str()),
                    true,
                    node.workspace_ref.rollback_capable,
                    matches!(
                        session.history_cursor.checkout_status,
                        HistoryCheckoutStatus::Applied
                    ),
                    matches!(
                        session.history_cursor.checkout_status,
                        HistoryCheckoutStatus::DegradedToTranscriptOnly
                    ),
                    matches!(
                        session.history_cursor.checkout_status,
                        HistoryCheckoutStatus::DegradedToTranscriptOnly
                    )
                    .then_some("workspace_rollback_unsupported"),
                ) {
                    let hook_results = hook_executor
                        .execute(&resolved_envelope)
                        .unwrap_or_default();
                    persist_history_state_hook_evidence(session, &resolved_envelope, &hook_results);
                }
            }
        }
        self.persist_session_and_trace_change(
            &session_key,
            SessionTraceMutation::ReplaceAll {
                traces: self.load_turn_traces(&session_key),
            },
        );
        if let Some(error) = blocked_error {
            return Err(error);
        }
        let snapshot = self.snapshot_for_session(&session_key);
        Ok(snapshot)
    }

    pub fn load_history_graph(
        &mut self,
        session_id: Option<&str>,
    ) -> (Vec<HistoryNode>, Vec<HistoryBranch>, HistoryCursor) {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let mut should_save = false;
        let graph = {
            let session = self.ensure_session(&session_key);
            if ensure_history_graph(session) {
                should_save = true;
            }
            (
                session.history_nodes.clone(),
                session.history_branches.clone(),
                session.history_cursor.clone(),
            )
        };
        if should_save {
            self.save_to_backend();
        }
        graph
    }

    pub fn load_history_cursor(&mut self, session_id: Option<&str>) -> HistoryCursor {
        self.load_history_graph(session_id).2
    }

    pub fn restore_branch_head(
        &mut self,
        session_id: Option<&str>,
        branch_id: Option<&str>,
        expected_cursor_version: Option<u64>,
    ) -> Result<SessionSnapshot, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let hook_executor = Arc::clone(&self.history_state_hook_executor);
        let mut blocked_error = None;
        let mut restored_node_id = None;
        // PA-093：块外预折叠（引用化分支头节点由事件流重建视图）。
        let fold_result: Option<(Vec<TurnHistoryMessage>, Vec<TurnTraceRecord>)> = {
            let pre_node = self.sessions.get(&session_key).and_then(|s| {
                let target = branch_id
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_string)
                    .or_else(|| s.history_cursor.active_branch_id.clone())
                    .unwrap_or_else(|| DEFAULT_HISTORY_BRANCH_ID.to_string());
                s.history_branches
                    .iter()
                    .find(|b| b.branch_id == target)
                    .and_then(|b| b.head_node_id.clone())
                    .and_then(|nid| history_node(s, &nid).cloned())
            });
            pre_node
                .as_ref()
                .filter(|n| n.event_seq_range.is_some())
                .map(|n| self.fold_node_views(&session_key, n))
        };
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
            reject_stale_cursor_version(&session.history_cursor, expected_cursor_version)?;
            let target_branch_id = branch_id
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .or_else(|| session.history_cursor.active_branch_id.clone())
                .unwrap_or_else(|| DEFAULT_HISTORY_BRANCH_ID.to_string());
            if let Some(start_envelope) = build_history_state_hook_envelope(
                session,
                HistoryStateHookPoint::BranchRestoreStart,
                HistoryStateCommandKind::RestoreBranchHead,
                None,
                Some(target_branch_id.as_str()),
                None,
                None,
                None,
                false,
                false,
                false,
                false,
                None,
            ) {
                let hook_results = hook_executor.execute(&start_envelope).unwrap_or_default();
                persist_history_state_hook_evidence(session, &start_envelope, &hook_results);
                if history_state_hook_results_blocked(&hook_results) {
                    refresh_session_metadata(session, true);
                    blocked_error = Some(format!(
                        "history branch restore blocked by hook before resolving branch: {target_branch_id}"
                    ));
                }
            }
            if blocked_error.is_some() {
                sync_latest_history_node(session, None);
            } else {
                let branch = session
                    .history_branches
                    .iter()
                    .find(|item| item.branch_id == target_branch_id)
                    .cloned()
                    .ok_or_else(|| format!("unknown history branch: {target_branch_id}"))?;
                let node_id = branch.head_node_id.clone().ok_or_else(|| {
                    format!("history branch has no head node: {target_branch_id}")
                })?;
                let node = history_node(session, &node_id)
                    .cloned()
                    .ok_or_else(|| format!("unknown history node: {node_id}"))?;
                // PA-093：引用化节点 → 事件折叠重建视图；legacy → 旧路径。
                match &fold_result {
                    Some((history, trace)) => hydrate_session_from_projection(
                        session,
                        &node,
                        history.clone(),
                        trace.clone(),
                    ),
                    None => hydrate_session_from_node(session, &node),
                }
                session.history_cursor.visible_node_id = Some(node.node_id.clone());
                session.history_cursor.active_branch_id = Some(branch.branch_id.clone());
                session.history_cursor.branch_head_node_id = Some(node.node_id.clone());
                session.history_cursor.workspace_node_id = Some(node.node_id.clone());
                session.history_cursor.mode = HistoryCursorMode::Live;
                session.history_cursor.checkout_mode = HistoryCheckoutMode::TranscriptOnly;
                session.history_cursor.checkout_status = HistoryCheckoutStatus::NotRequested;
                bump_cursor_version(&mut session.history_cursor);
                session.history_cursor.event_watermark = session.event_watermark;
                // PA-093：分支恢复 = 视图切换事实，append 事件。
                crate::agent::turn_flow::emit_global_event(
                    &session_key,
                    "checkpoint",
                    crate::agent::turn_event::TurnEvent::CheckpointCheckout {
                        node_id: node.node_id.clone(),
                        mode: "transcript_only".to_string(),
                    },
                );
                refresh_session_metadata(session, true);
                if let Some(resolved_envelope) = build_history_state_hook_envelope(
                    session,
                    HistoryStateHookPoint::BranchRestoreResolved,
                    HistoryStateCommandKind::RestoreBranchHead,
                    None,
                    Some(target_branch_id.as_str()),
                    None,
                    Some(node.node_id.as_str()),
                    Some(branch.branch_id.as_str()),
                    true,
                    false,
                    false,
                    false,
                    None,
                ) {
                    let hook_results = hook_executor
                        .execute(&resolved_envelope)
                        .unwrap_or_default();
                    persist_history_state_hook_evidence(session, &resolved_envelope, &hook_results);
                }
                restored_node_id = Some(node.node_id);
            }
        }
        self.persist_session_and_trace_change(
            &session_key,
            SessionTraceMutation::ReplaceAll {
                traces: self.load_turn_traces(&session_key),
            },
        );
        if let Some(error) = blocked_error {
            return Err(error);
        }
        let restored_node_id = restored_node_id.expect("restored node id should be available");
        let snapshot = self.snapshot_for_session_at(&session_key, Some(restored_node_id.as_str()));
        Ok(snapshot)
    }

    pub fn fork_from_history_node(
        &mut self,
        session_id: Option<&str>,
        node_id: &str,
        expected_cursor_version: Option<u64>,
    ) -> Result<SessionSnapshot, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let hook_executor = Arc::clone(&self.history_state_hook_executor);
        let mut blocked_error = None;
        // PA-093：块外预折叠（引用化源节点由事件流重建视图）。
        let fold_result: Option<(Vec<TurnHistoryMessage>, Vec<TurnTraceRecord>)> = {
            let pre_node = self
                .sessions
                .get(&session_key)
                .and_then(|s| history_node(s, node_id))
                .cloned();
            pre_node
                .as_ref()
                .filter(|n| n.event_seq_range.is_some())
                .map(|n| self.fold_node_views(&session_key, n))
        };
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
            reject_stale_cursor_version(&session.history_cursor, expected_cursor_version)?;
            if let Some(start_envelope) = build_history_state_hook_envelope(
                session,
                HistoryStateHookPoint::BranchForkStart,
                HistoryStateCommandKind::ForkFromHistoryNode,
                Some(node_id),
                None,
                None,
                None,
                None,
                false,
                false,
                false,
                false,
                None,
            ) {
                let hook_results = hook_executor.execute(&start_envelope).unwrap_or_default();
                persist_history_state_hook_evidence(session, &start_envelope, &hook_results);
                if history_state_hook_results_blocked(&hook_results) {
                    refresh_session_metadata(session, true);
                    blocked_error = Some(format!(
                        "history branch fork blocked by hook before resolving node: {node_id}"
                    ));
                }
            }
            if blocked_error.is_some() {
                sync_latest_history_node(session, None);
            } else {
                let source_node = history_node(session, node_id)
                    .cloned()
                    .ok_or_else(|| format!("unknown history node: {node_id}"))?;
                let source_branch_id = source_node.branch_id.clone();
                let created_at_ms = now_timestamp_ms();
                let label_index = session.history_branches.len() + 1;
                let new_branch_id = new_history_branch_id(session, label_index);
                session.history_branches.push(HistoryBranch {
                    branch_id: new_branch_id.clone(),
                    session_id: session.conversation_id.clone(),
                    base_node_id: Some(source_node.node_id.clone()),
                    head_node_id: Some(source_node.node_id.clone()),
                    forked_from_branch_id: Some(source_branch_id),
                    forked_from_node_id: Some(source_node.node_id.clone()),
                    label: format!("fork-{label_index}"),
                    created_at_ms,
                    updated_at_ms: created_at_ms,
                });
                // PA-093：引用化节点 → 事件折叠重建视图；legacy → 旧路径。
                match &fold_result {
                    Some((history, trace)) => hydrate_session_from_projection(
                        session,
                        &source_node,
                        history.clone(),
                        trace.clone(),
                    ),
                    None => hydrate_session_from_node(session, &source_node),
                }
                session.history_cursor.visible_node_id = Some(source_node.node_id.clone());
                session.history_cursor.active_branch_id = Some(new_branch_id.clone());
                session.history_cursor.branch_head_node_id = Some(source_node.node_id.clone());
                session.history_cursor.workspace_node_id = Some(source_node.node_id.clone());
                session.history_cursor.mode = HistoryCursorMode::Live;
                session.history_cursor.checkout_mode = HistoryCheckoutMode::TranscriptOnly;
                session.history_cursor.checkout_status = HistoryCheckoutStatus::NotRequested;
                bump_cursor_version(&mut session.history_cursor);
                session.history_cursor.event_watermark = session.event_watermark;
                // PA-093：fork 是分支事实，append 事件（新分支事件携带 branch_id，
                // 折叠时按血缘链推导可见集合）。
                crate::agent::turn_flow::emit_global_event(
                    &session_key,
                    "fork",
                    crate::agent::turn_event::TurnEvent::ForkCreated {
                        branch_id: new_branch_id.clone(),
                        from_node_id: source_node.node_id.clone(),
                    },
                );
                refresh_session_metadata(session, true);
                if let Some(resolved_envelope) = build_history_state_hook_envelope(
                    session,
                    HistoryStateHookPoint::BranchForkResolved,
                    HistoryStateCommandKind::ForkFromHistoryNode,
                    Some(node_id),
                    None,
                    None,
                    Some(source_node.node_id.as_str()),
                    Some(new_branch_id.as_str()),
                    true,
                    false,
                    false,
                    false,
                    None,
                ) {
                    let hook_results = hook_executor
                        .execute(&resolved_envelope)
                        .unwrap_or_default();
                    persist_history_state_hook_evidence(session, &resolved_envelope, &hook_results);
                }
            }
        }
        self.persist_session_and_trace_change(
            &session_key,
            SessionTraceMutation::ReplaceAll {
                traces: self.load_turn_traces(&session_key),
            },
        );
        if let Some(error) = blocked_error {
            return Err(error);
        }
        let snapshot = self.snapshot_for_session(&session_key);
        Ok(snapshot)
    }

    pub fn switch_history_branch(
        &mut self,
        session_id: Option<&str>,
        branch_id: &str,
        expected_cursor_version: Option<u64>,
    ) -> Result<SessionSnapshot, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let hook_executor = Arc::clone(&self.history_state_hook_executor);
        let mut blocked_error = None;
        let mut target_node_id = None;
        // PA-093：块外预折叠（引用化分支头节点由事件流重建视图）。
        let fold_result: Option<(Vec<TurnHistoryMessage>, Vec<TurnTraceRecord>)> = {
            let pre_node = self.sessions.get(&session_key).and_then(|s| {
                s.history_branches
                    .iter()
                    .find(|b| b.branch_id == branch_id)
                    .and_then(|b| b.head_node_id.clone())
                    .and_then(|nid| history_node(s, &nid).cloned())
            });
            pre_node
                .as_ref()
                .filter(|n| n.event_seq_range.is_some())
                .map(|n| self.fold_node_views(&session_key, n))
        };
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
            reject_stale_cursor_version(&session.history_cursor, expected_cursor_version)?;
            if let Some(start_envelope) = build_history_state_hook_envelope(
                session,
                HistoryStateHookPoint::BranchSwitchStart,
                HistoryStateCommandKind::SwitchHistoryBranch,
                None,
                Some(branch_id),
                None,
                None,
                None,
                false,
                false,
                false,
                false,
                None,
            ) {
                let hook_results = hook_executor.execute(&start_envelope).unwrap_or_default();
                persist_history_state_hook_evidence(session, &start_envelope, &hook_results);
                if history_state_hook_results_blocked(&hook_results) {
                    refresh_session_metadata(session, true);
                    blocked_error = Some(format!(
                        "history branch switch blocked by hook before resolving branch: {branch_id}"
                    ));
                }
            }
            if blocked_error.is_some() {
                sync_latest_history_node(session, None);
            } else {
                let branch = session
                    .history_branches
                    .iter()
                    .find(|item| item.branch_id == branch_id)
                    .cloned()
                    .ok_or_else(|| format!("unknown history branch: {branch_id}"))?;
                let node_id = branch
                    .head_node_id
                    .clone()
                    .ok_or_else(|| format!("history branch has no head node: {branch_id}"))?;
                let node = history_node(session, &node_id)
                    .cloned()
                    .ok_or_else(|| format!("unknown history node: {node_id}"))?;
                // PA-093：引用化节点 → 事件折叠重建视图；legacy → 旧路径。
                match &fold_result {
                    Some((history, trace)) => hydrate_session_from_projection(
                        session,
                        &node,
                        history.clone(),
                        trace.clone(),
                    ),
                    None => hydrate_session_from_node(session, &node),
                }
                session.history_cursor.visible_node_id = Some(node.node_id.clone());
                session.history_cursor.active_branch_id = Some(branch.branch_id.clone());
                session.history_cursor.branch_head_node_id = Some(node.node_id.clone());
                session.history_cursor.workspace_node_id = Some(node.node_id.clone());
                session.history_cursor.mode = HistoryCursorMode::Live;
                session.history_cursor.checkout_mode = HistoryCheckoutMode::TranscriptOnly;
                session.history_cursor.checkout_status = HistoryCheckoutStatus::NotRequested;
                bump_cursor_version(&mut session.history_cursor);
                session.history_cursor.event_watermark = session.event_watermark;
                // PA-093：分支切换 = 视图切换事实，append 事件。
                crate::agent::turn_flow::emit_global_event(
                    &session_key,
                    "checkpoint",
                    crate::agent::turn_event::TurnEvent::CheckpointCheckout {
                        node_id: node.node_id.clone(),
                        mode: "transcript_only".to_string(),
                    },
                );
                refresh_session_metadata(session, true);
                if let Some(resolved_envelope) = build_history_state_hook_envelope(
                    session,
                    HistoryStateHookPoint::BranchSwitchResolved,
                    HistoryStateCommandKind::SwitchHistoryBranch,
                    None,
                    Some(branch_id),
                    None,
                    Some(node.node_id.as_str()),
                    Some(branch.branch_id.as_str()),
                    true,
                    false,
                    false,
                    false,
                    None,
                ) {
                    let hook_results = hook_executor
                        .execute(&resolved_envelope)
                        .unwrap_or_default();
                    persist_history_state_hook_evidence(session, &resolved_envelope, &hook_results);
                }
                target_node_id = Some(node.node_id);
            }
        }
        self.persist_session_and_trace_change(
            &session_key,
            SessionTraceMutation::ReplaceAll {
                traces: self.load_turn_traces(&session_key),
            },
        );
        if let Some(error) = blocked_error {
            return Err(error);
        }
        let target_node_id = target_node_id.expect("target node id should be available");
        let snapshot = self.snapshot_for_session_at(&session_key, Some(target_node_id.as_str()));
        Ok(snapshot)
    }

    pub fn list_sessions(&self) -> Vec<SessionOverview> {
        let mut sessions = self
            .sessions
            .values()
            .filter(|session| session_is_persistable(session))
            .map(|session| SessionOverview {
                conversation_id: session.conversation_id.clone(),
                title: session.title.clone(),
                summary: session.summary.clone(),
                turn_count: session.turn_count,
                last_referenced_file: session.last_referenced_file.clone(),
                updated_at_ms: session.updated_at_ms,
                workspace_id: session.workspace_id.clone(),
            })
            .collect::<Vec<_>>();

        sessions.sort_by(|left, right| {
            right
                .updated_at_ms
                .cmp(&left.updated_at_ms)
                .then_with(|| left.conversation_id.cmp(&right.conversation_id))
        });
        sessions
    }

    pub fn load_turn_traces(&self, session_id: &str) -> Vec<TurnTraceRecord> {
        self.sessions
            .get(session_id)
            .map(|session| session.turn_trace_history.clone())
            .unwrap_or_default()
    }

    pub fn remove_session(&mut self, session_id: &str) -> Vec<SessionOverview> {
        if self.sessions.remove(session_id).is_some() {
            delete_session_attachment_dir(&self.attachment_root, session_id);
            self.remove_session_attachment_catalog(session_id);

            if !self.backend.remove_session(
                session_id,
                &self.attachment_assets,
                &self.session_attachment_index,
                &self.mcp_source_snapshots,
                &self.skill_source_snapshots,
            ) {
                self.save_to_backend();
            }
        }

        if self.sessions.is_empty() {
            self.sessions = default_sessions();
        }
        self.list_sessions()
    }

    pub fn save_input_attachments(
        &mut self,
        session_id: &str,
        images: &[TurnInputImage],
    ) -> Result<Vec<SessionAttachment>, String> {
        if images.is_empty() {
            return Ok(Vec::new());
        }

        let session_dir = self.attachment_root.join(session_id);
        fs::create_dir_all(&session_dir)
            .map_err(|error| format!("failed to create attachment directory: {error}"))?;

        let created_at_ms = now_timestamp_ms();
        let mut attachments = Vec::with_capacity(images.len());
        for (index, image) in images.iter().enumerate() {
            let attachment_id = format!("att-{created_at_ms}-{}", index + 1);
            let file_name = format!("{attachment_id}.dataurl");
            let relative_path = format!("{session_id}/{file_name}");
            let asset_id = attachment_asset_id(&relative_path);
            let absolute_path = session_dir.join(&file_name);
            fs::write(&absolute_path, &image.data_url)
                .map_err(|error| format!("failed to persist attachment payload: {error}"))?;
            self.attachment_assets.insert(
                asset_id.clone(),
                AttachmentAsset {
                    id: asset_id.clone(),
                    session_id: session_id.to_string(),
                    name: image.name.clone(),
                    mime_type: image.mime_type.clone(),
                    relative_path: relative_path.clone(),
                    size_bytes: image.payload_size_bytes(),
                    created_at_ms,
                    status: AttachmentLifecycleStatus::Reclaimable,
                    reference_count: 0,
                    last_referenced_at_ms: None,
                    expires_at_ms: Some(
                        created_at_ms.saturating_add(DEFAULT_ATTACHMENT_RECLAIM_TTL_MS),
                    ),
                },
            );
            attachments.push(SessionAttachment {
                id: attachment_id,
                asset_id,
                name: image.name.clone(),
                mime_type: image.mime_type.clone(),
                relative_path,
                size_bytes: image.payload_size_bytes(),
                created_at_ms,
            });
        }
        Ok(attachments)
    }

    pub fn load_recent_images(
        &self,
        session_id: Option<&str>,
        limit: usize,
    ) -> Vec<TurnInputImage> {
        let Some(session_id) = session_id else {
            return Vec::new();
        };
        let Some(session) = self.sessions.get(session_id) else {
            return Vec::new();
        };
        if limit == 0 {
            return Vec::new();
        }

        session
            .history
            .iter()
            .rev()
            .find(|message| message.role == "user")
            .map(|message| {
                if message.attachments.is_empty() {
                    return Vec::new();
                }
                message
                    .attachments
                    .iter()
                    .take(limit)
                    .filter_map(|attachment| {
                        load_attachment_image(
                            &self.attachment_root,
                            &self.attachment_assets,
                            attachment,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    }

    #[allow(dead_code)]
    pub fn list_attachment_assets(&self, session_id: Option<&str>) -> Vec<AttachmentAsset> {
        self.query_attachment_assets(&AttachmentAssetQuery {
            session_id: session_id.map(str::to_string),
            ..AttachmentAssetQuery::default()
        })
    }

    #[allow(dead_code)]
    pub fn query_attachment_assets(&self, query: &AttachmentAssetQuery) -> Vec<AttachmentAsset> {
        let mut assets = attachment_assets_for_query(
            &self.sessions,
            &self.attachment_assets,
            &self.session_attachment_index,
            &self.attachment_root,
            query,
            now_timestamp_ms(),
        );
        if let Some(limit) = query.limit {
            assets.truncate(limit);
        }
        assets
    }

    #[allow(dead_code)]
    pub fn cleanup_attachment_assets(
        &mut self,
        request: &AttachmentCleanupRequest,
    ) -> AttachmentCleanupResult {
        if !request.include_expired && !request.include_reclaimable {
            return AttachmentCleanupResult::default();
        }

        let mut query = AttachmentAssetQuery {
            session_id: request.session_id.clone(),
            ..AttachmentAssetQuery::default()
        };
        if request.include_expired {
            query.statuses.push(AttachmentLifecycleStatus::Expired);
        }
        if request.include_reclaimable {
            query.statuses.push(AttachmentLifecycleStatus::Reclaimable);
        }
        query.limit = request.limit;

        let candidates = self.query_attachment_assets(&query);
        let mut removed_asset_ids = Vec::new();
        let mut removed_file_count = 0;

        for asset in candidates {
            if asset.reference_count > 0 {
                continue;
            }

            let matches_cleanup_policy = match asset.status {
                AttachmentLifecycleStatus::Reclaimable => request.include_reclaimable,
                AttachmentLifecycleStatus::Expired => {
                    request.include_expired
                        && request.expire_before_ms.map_or(true, |cutoff| {
                            asset
                                .expires_at_ms
                                .map(|expires_at_ms| expires_at_ms <= cutoff)
                                .unwrap_or(asset.created_at_ms <= cutoff)
                        })
                }
                AttachmentLifecycleStatus::Active | AttachmentLifecycleStatus::MissingPayload => {
                    false
                }
            };
            if !matches_cleanup_policy {
                continue;
            }

            let path = self.attachment_root.join(&asset.relative_path);
            if path.is_file() && fs::remove_file(&path).is_ok() {
                removed_file_count += 1;
            }
            if let Some(parent) = path.parent() {
                let _ = fs::remove_dir(parent);
            }

            if self.attachment_assets.remove(&asset.id).is_some() {
                removed_asset_ids.push(asset.id);
            }
        }

        if !removed_asset_ids.is_empty() {
            self.refresh_attachment_catalog();
            self.save_to_backend();
        }

        AttachmentCleanupResult {
            removed_catalog_count: removed_asset_ids.len(),
            removed_asset_ids,
            removed_file_count,
        }
    }

    fn ensure_session(&mut self, session_id: &str) -> &mut SessionState {
        let initial_trace_migration_state = match self.backend.trace_storage_mode() {
            SeparateTraceTableMode::Off => TraceMigrationState::LegacyBlob,
            SeparateTraceTableMode::DualWrite => TraceMigrationState::DualWrite,
            // PA-088：新会话无旧数据，直接表权威（blob 只存 refs，不膨胀）。
            // 存量会话（load_store 反序列化）保持其 blob 中的状态，不在此晋升。
            SeparateTraceTableMode::WriteSeparate => TraceMigrationState::TraceTableAuthoritative,
        };
        self.sessions
            .entry(session_id.to_string())
            .or_insert_with(|| SessionState {
                conversation_id: session_id.to_string(),
                title: DEFAULT_SESSION_TITLE.to_string(),
                summary: DEFAULT_SESSION_SUMMARY.to_string(),
                history: Vec::new(),
                provider_native_transcript: Vec::new(),
                turn_trace_history: Vec::new(),
                trace_migration_state: initial_trace_migration_state,
                turn_trace_refs: None,
                long_term_memory_entries: Vec::new(),
                memory_write_evidence: Vec::new(),
                memory_write_hook_trace_records: Vec::new(),
                history_state_evidence: Vec::new(),
                turn_count: 0,
                last_referenced_file: None,
                updated_at_ms: now_timestamp_ms(),
                history_nodes: Vec::new(),
                history_branches: Vec::new(),
                history_cursor: HistoryCursor {
                    session_id: session_id.to_string(),
                    ..HistoryCursor::default()
                },
                event_watermark: 0,
                last_commit_watermark: 0,
                workspace_id: None,
            })
    }

    /// Workspace 注册表（PA-079）。
    pub fn list_workspaces(&self) -> Vec<crate::agent::workspace::WorkspaceRecord> {
        self.workspaces.clone()
    }

    pub fn create_workspace(
        &mut self,
        name: &str,
        root_path: &str,
    ) -> Result<crate::agent::workspace::WorkspaceRecord, String> {
        let record = crate::agent::workspace::create_workspace_entry(&mut self.workspaces, name, root_path)?;
        self.save_to_backend();
        Ok(record)
    }

    pub fn resolve_workspace_root(&self, workspace_id: Option<&str>) -> Result<String, String> {
        crate::agent::workspace::resolve_workspace_root(&self.workspaces, workspace_id)
    }

    /// 首次持久化盖章 workspace_id（PA-079）：会话 workspace_id 为 None 时写入并落盘；
    /// 已盖章则 no-op（幂等，不影响既有会话的后续轮）。
    /// **先 `ensure_session` 再盖章**：turn 提交时全新会话尚未被 `prepare_turn` 创建，
    /// 若只对已存在会话盖章，首轮 workspace_id 会丢失。
    pub fn stamp_workspace_id(&mut self, session_id: &str, workspace_id: &str) {
        let changed = {
            let session = self.ensure_session(session_id);
            if session.workspace_id.is_none() {
                session.workspace_id = Some(workspace_id.to_string());
                true
            } else {
                false
            }
        };
        if changed {
            self.save_to_backend();
        }
    }

    /// 共享路径授权清单（PA-080）：host control plane 与工具执行器共享同一 `Arc`。
    pub fn path_authorizations(&self) -> Arc<crate::agent::path_permission::AuthorizeStore> {
        Arc::clone(&self.path_authorizations)
    }

    /// 显式授权一个路径（仅读；目标必须存在）。变更即持久化（store_metadata
    /// key=`path_authorizations.v1`），重启后生效。
    pub fn authorize_path(&mut self, canonical: PathBuf) -> Result<crate::agent::path_permission::AuthorizedPathEntry, String> {
        let entry = self.path_authorizations.grant(canonical)?;
        self.save_to_backend();
        Ok(entry)
    }

    /// 撤销授权（精确路径；子授权保留）。返回是否实际移除。
    pub fn revoke_authorization(&mut self, canonical: &Path) -> bool {
        let removed = self.path_authorizations.revoke(canonical);
        if removed {
            self.save_to_backend();
        }
        removed
    }

    /// 列出全部授权条目。
    pub fn list_authorizations(&self) -> Vec<crate::agent::path_permission::AuthorizedPathEntry> {
        self.path_authorizations.entries()
    }

    /// PA-093：读取会话事件流（backend 支持时）；否则空。
    pub fn load_turn_events(
        &self,
        session_id: &str,
        up_to_seq: Option<u64>,
    ) -> Vec<(u64, String, crate::agent::turn_event::TurnEvent)> {
        self.backend.load_turn_events(session_id, up_to_seq)
    }

    /// PA-093：当前事件水位（backend 支持时）；否则 0。
    pub fn load_event_watermark(&self, session_id: &str) -> u64 {
        self.backend.load_event_watermark(session_id)
    }

    /// PA-093：当前 active 分支（事件落列用）；会话不存在时 None。
    pub fn session_active_branch_id(&self, session_id: &str) -> Option<String> {
        self.sessions.get(session_id).and_then(|session| {
            session
                .history_cursor
                .active_branch_id
                .clone()
                .or_else(|| Some(DEFAULT_HISTORY_BRANCH_ID.to_string()))
        })
    }

    /// PA-093：flush 成功后同步会话水位 + 将 branch head 节点升级为引用化
    /// （commit 先于 flush 的时序下，节点先以 legacy 快照提交，事件落盘后
    /// 清空快照并记录事件区间——flush 失败则节点保持 legacy，数据不丢）。
    /// 幂等：水位未增长或无新节点时无操作。
    pub fn finalize_event_watermark(&mut self, session_id: &str, _turn_id: &str) {
        let Some(session) = self.sessions.get_mut(session_id) else {
            return;
        };
        let new_watermark = self.backend.load_event_watermark(session_id);
        if new_watermark <= session.event_watermark {
            return;
        }
        session.event_watermark = new_watermark;
        // branch head 节点 = 最近一次 commit 的节点（单线程 turn 流程保证）。
        if let Some(head_id) = session.history_cursor.branch_head_node_id.clone() {
            if let Some(node) = session
                .history_nodes
                .iter_mut()
                .find(|n| n.node_id == head_id)
            {
                if node.event_seq_range.is_none()
                    && new_watermark > session.last_commit_watermark
                {
                    node.event_seq_range =
                        Some((session.last_commit_watermark, new_watermark - 1));
                    // 引用化：事件可重建的视图字段不再内嵌快照。
                    node.history.clear();
                    node.provider_native_transcript.clear();
                    node.turn_trace_history.clear();
                }
            }
        }
        session.last_commit_watermark = new_watermark;
    }

    /// PA-093：折叠事件流到节点区间终点，得到会话视图（history/trace）。
    /// 优先命中投影缓存（节点区间不可变 → 缓存天然有效）；未命中时全量重折叠。
    fn fold_node_views(
        &mut self,
        session_id: &str,
        node: &HistoryNode,
    ) -> (Vec<TurnHistoryMessage>, Vec<TurnTraceRecord>) {
        let Some((_start, end)) = node.event_seq_range else {
            return (node.history.clone(), node.turn_trace_history.clone());
        };
        if let Some((cached_end, history, trace)) = self.projection_cache.get(&node.node_id) {
            if *cached_end == end {
                return (history.clone(), trace.clone());
            }
        }
        let events = self.backend.load_turn_events(session_id, Some(end));
        if events.is_empty() {
            eprintln!(
                "[pony-agent][session] event stream empty for referenced node {} (backend without event support?); falling back to empty view",
                node.node_id
            );
        }
        let (history, trace) = {
            let session = self.sessions.get(session_id).expect("session exists");
            fold_session_views(
                &events,
                &node.branch_id,
                &session.history_nodes,
                &session.history_branches,
            )
        };
        self.projection_cache
            .insert(node.node_id.clone(), (end, history.clone(), trace.clone()));
        (history, trace)
    }

    /// PA-091：turn 终态 flush 事件批次（与快照写入同事务，由 backend 保证）。
    /// 返回是否成功；失败时调用方只记日志（contained，不阻断流）。
    pub fn persist_events(
        &self,
        session_id: &str,
        turn_id: &str,
        branch_id: &str,
        events: Vec<crate::agent::turn_event::TurnEvent>,
    ) -> bool {
        if events.is_empty() {
            return true;
        }
        let outcome = self.backend.persist_command(PersistCommand::FlushEvents {
            epoch: 1,
            session_id: session_id.to_string(),
            turn_id: turn_id.to_string(),
            branch_id: branch_id.to_string(),
            events,
        });
        matches!(outcome, PersistCommandOutcome::Succeeded)
    }

    fn save_to_backend(&self) {
        if matches!(
            self.backend.trace_storage_mode(),
            SeparateTraceTableMode::WriteSeparate
        ) {
            for (session_id, session) in &self.sessions {
                if matches!(
                    session.trace_migration_state,
                    TraceMigrationState::TraceTableAuthoritative
                ) {
                    // PA-088：写表用全量 Union（顶层 ∪ 全部节点 trace，按 turn_id 去重取最新）。
                    // 仅写顶层会导致 >24 轮会话的旧节点 refs 在重启 materialize 时解析不到
                    // （表被 prune 到 24 行），节点 trace 丢失。
                    let union = collect_trace_union(session);
                    let _ = self.backend.replace_session_traces(session_id, &union);
                }
            }
        }
        let trace_mode = self.backend.trace_storage_mode();
        let store = PersistedStore {
            sessions: self
                .sessions
                .iter()
                .filter(|(_, session)| session_is_persistable(session))
                .map(|(session_id, session)| {
                    (
                        session_id.clone(),
                        session_state_for_backend(session, trace_mode),
                    )
                })
                .collect::<SessionMap>(),
            attachment_assets: self.attachment_assets.clone(),
            session_attachment_index: self.session_attachment_index.clone(),
            mcp_source_snapshots: self.mcp_source_snapshots.clone(),
            skill_source_snapshots: self.skill_source_snapshots.clone(),
            workspaces: self.workspaces.clone(),
            path_authorizations: self.path_authorizations.entries(),
        };
        self.backend.save_store(&store);
    }

    fn save_session_to_backend(&self, session_id: &str) {
        let Some(session) = self.sessions.get(session_id) else {
            return;
        };
        let prepared = session_state_for_backend(session, self.backend.trace_storage_mode());
        if matches!(
            prepared.trace_migration_state,
            TraceMigrationState::TraceTableAuthoritative
        ) && matches!(
            self.backend.persist_session_with_trace_mutation(
                session_id,
                &prepared,
                // PA-090：Authoritative 会话 ReplaceAll 用全量 Union（同主事务路径）。
                SessionTraceMutation::ReplaceAll {
                    traces: collect_trace_union(session),
                },
            ),
            SessionBackendMutationResult::Succeeded
        ) {
            return;
        }
        if session_is_persistable(session) && self.backend.upsert_session(session_id, &prepared) {
            return;
        }
        self.save_to_backend();
    }

    fn persist_session_and_trace_change(
        &mut self,
        session_id: &str,
        mutation: SessionTraceMutation,
    ) {
        let Some(session) = self.sessions.get(session_id) else {
            return;
        };
        let prepared = session_state_for_backend(session, self.backend.trace_storage_mode());
        // PA-090：Authoritative 会话的 ReplaceAll 改用全量 Union——
        // 否则用顶层 trace（24 行截断）替换表会清空节点 refs 指向的 trace，
        // 重启 materialize 全部 missing（checkout/restore/fork/switch 均走此路径）。
        let effective_mutation = if matches!(
            prepared.trace_migration_state,
            TraceMigrationState::TraceTableAuthoritative
        ) && matches!(mutation, SessionTraceMutation::ReplaceAll { .. })
        {
            SessionTraceMutation::ReplaceAll {
                traces: collect_trace_union(session),
            }
        } else {
            mutation.clone()
        };
        // PA-089 阶段 3 接线：Authoritative 会话的增量 trace mutation 走 PersistCommand
        // 增量命令（AppendTrace/UpdateTraceTerminal/AppendHookRecords）——只写变化行，
        // 替代全量 persist_session_with_trace_mutation（每次全量序列化 blob + 重建表，
        // 是"生成结束后卡顿"的根因：completed 后全量写 3.77MB blob + 全部 trace 行）。
        let is_authoritative = matches!(
            prepared.trace_migration_state,
            TraceMigrationState::TraceTableAuthoritative
        );
        if is_authoritative && session_is_persistable(session) {
            let command = match &effective_mutation {
                SessionTraceMutation::UpsertOne { trace, trace_order } => {
                    Some(PersistCommand::AppendTrace {
                        epoch: 1,
                        session_id: session_id.to_string(),
                        trace: trace.clone(),
                        trace_order: *trace_order,
                    })
                }
                SessionTraceMutation::UpdateTerminalEvent {
                    turn_id,
                    event_id,
                    event_type,
                    event_version,
                    sequence,
                    emitted_at_ms,
                    updated_at,
                } => Some(PersistCommand::UpdateTraceTerminal {
                    epoch: 1,
                    session_id: session_id.to_string(),
                    turn_id: turn_id.clone(),
                    terminal_patch: TraceTerminalPatch {
                        event_id: event_id.clone(),
                        event_type: event_type.clone(),
                        event_version: event_version.clone(),
                        sequence: *sequence,
                        emitted_at_ms: *emitted_at_ms,
                        updated_at: *updated_at,
                        phase: None,
                    },
                }),
                SessionTraceMutation::AppendHookRecords {
                    turn_id,
                    hook_trace_records,
                    updated_at,
                } => Some(PersistCommand::AppendHookRecords {
                    epoch: 1,
                    session_id: session_id.to_string(),
                    turn_id: turn_id.clone(),
                    hook_trace_records: hook_trace_records.clone(),
                    updated_at: *updated_at,
                }),
                _ => None,
            };
            if let Some(command) = command {
                let outcome = self.backend.persist_command(command);
                if matches!(outcome, PersistCommandOutcome::Succeeded) {
                    return;
                }
                // 命令失败 → 回退全量路径（不丢数据）
            }
        }
        let result = if session_is_persistable(session) {
            self.backend.persist_session_with_trace_mutation(
                session_id,
                &prepared,
                effective_mutation,
            )
        } else {
            SessionBackendMutationResult::Unsupported
        };

        if matches!(result, SessionBackendMutationResult::Succeeded) {
            if matches!(
                self.backend.trace_storage_mode(),
                SeparateTraceTableMode::WriteSeparate
            ) {
                if let Some(session) = self.sessions.get_mut(session_id) {
                    // PA-088：按原状态分派晋升——存量 LegacyBlob 首次写 → DualWrite
                    // （保留 blob 双写兜底窗口，不直接表权威）；DualWrite 保持；
                    // 新会话（ensure_session 已置 Authoritative）保持。
                    session.trace_migration_state = match session.trace_migration_state {
                        TraceMigrationState::LegacyBlob => TraceMigrationState::DualWrite,
                        TraceMigrationState::DualWrite => TraceMigrationState::DualWrite,
                        TraceMigrationState::TraceTableAuthoritative => {
                            TraceMigrationState::TraceTableAuthoritative
                        }
                    };
                }
            }
            return;
        }

        if matches!(result, SessionBackendMutationResult::NotFound)
            && matches!(
                prepared.trace_migration_state,
                TraceMigrationState::TraceTableAuthoritative
            )
        {
            let replace_all = SessionTraceMutation::ReplaceAll {
                // PA-090：Authoritative 会话 NotFound 重试同样用全量 Union。
                traces: collect_trace_union(session),
            };
            if matches!(
                self.backend.persist_session_with_trace_mutation(
                    session_id,
                    &prepared,
                    replace_all
                ),
                SessionBackendMutationResult::Succeeded
            ) {
                return;
            }
        }

        if matches!(result, SessionBackendMutationResult::Failed) {
            eprintln!(
                "[pony-agent][session] persist_session_and_trace_change backend write failed for session {}; falling back to separate blob+table writes",
                session_id
            );
        }

        self.persist_trace_change(session_id, trace_action_from_mutation(mutation));
        self.save_session_to_backend(session_id);
    }

    fn persist_trace_change(&self, session_id: &str, action: TracePersistenceAction) {
        let result = match action {
            TracePersistenceAction::ReplaceAll => self
                .sessions
                .get(session_id)
                .map(|session| {
                    // PA-088：ReplaceAll 用全量 Union（顶层 ∪ 节点 trace），
                    // 避免用内存顶层（24 行截断）替换表导致节点 refs 解析不到。
                    let union = collect_trace_union(session);
                    self.backend.replace_session_traces(session_id, &union)
                })
                .unwrap_or(SessionBackendMutationResult::Unsupported),
            TracePersistenceAction::UpsertOne { trace, trace_order } => self
                .backend
                .upsert_turn_trace(session_id, &trace, trace_order),
            TracePersistenceAction::UpdateTerminalEvent {
                turn_id,
                event_id,
                event_type,
                event_version,
                sequence,
                emitted_at_ms,
                updated_at,
            } => self.backend.update_turn_trace_terminal_event(
                session_id,
                &turn_id,
                event_id.as_deref(),
                event_type.as_deref(),
                event_version.as_deref(),
                sequence,
                emitted_at_ms,
                updated_at,
            ),
            TracePersistenceAction::AppendHookRecords {
                turn_id,
                hook_trace_records,
                updated_at,
            } => self.backend.append_turn_trace_hook_records(
                session_id,
                &turn_id,
                &hook_trace_records,
                updated_at,
            ),
        };

        if matches!(
            result,
            SessionBackendMutationResult::Failed | SessionBackendMutationResult::NotFound
        ) {
            eprintln!(
                "[pony-agent][session] trace-level persistence {:?} for session {}",
                result, session_id
            );
        }
    }

    fn snapshot_for_session(&self, session_id: &str) -> SessionSnapshot {
        self.snapshot_for_session_at(session_id, None)
    }

    fn snapshot_for_session_at(&self, session_id: &str, node_id: Option<&str>) -> SessionSnapshot {
        let session = self
            .sessions
            .get(session_id)
            .expect("session must exist before snapshot");
        let attachment_assets = attachment_assets_for_query(
            &self.sessions,
            &self.attachment_assets,
            &self.session_attachment_index,
            &self.attachment_root,
            &AttachmentAssetQuery {
                session_id: Some(session_id.to_string()),
                ..AttachmentAssetQuery::default()
            },
            now_timestamp_ms(),
        );
        // PA-093：时间旅行——引用化节点由事件流折叠重建视图（克隆会话，不污染
        // 当前内存态；legacy 节点走 snapshot_from_state 内嵌快照路径）。
        if let Some(nid) = node_id {
            let is_referenced = history_node(session, nid)
                .map(|node| node.event_seq_range.is_some())
                .unwrap_or(false);
            if is_referenced {
                let mut view = session.clone();
                let node = history_node(&view, nid)
                    .cloned()
                    .expect("node exists after check");
                let (history, trace) = fold_session_views(
                    &self.backend.load_turn_events(
                        session_id,
                        node.event_seq_range.map(|(_, e)| e),
                    ),
                    &node.branch_id,
                    &view.history_nodes,
                    &view.history_branches,
                );
                hydrate_session_from_projection(&mut view, &node, history, trace);
                // 节点视角 cursor（与 snapshot_from_state 节点路径语义对齐），
                // 然后走会话路径组装（折叠视图在 session 字段，非节点快照）。
                let branch_head_node_id = view
                    .history_branches
                    .iter()
                    .find(|b| b.branch_id == node.branch_id)
                    .and_then(|b| b.head_node_id.clone());
                view.history_cursor.visible_node_id = Some(node.node_id.clone());
                view.history_cursor.active_branch_id = Some(node.branch_id.clone());
                view.history_cursor.branch_head_node_id = branch_head_node_id.clone();
                view.history_cursor.workspace_node_id = Some(node.node_id.clone());
                view.history_cursor.mode =
                    if branch_head_node_id.as_deref() == Some(node.node_id.as_str()) {
                        HistoryCursorMode::Live
                    } else {
                        HistoryCursorMode::Historical
                    };
                view.history_cursor.checkout_mode = HistoryCheckoutMode::TranscriptOnly;
                view.history_cursor.checkout_status = HistoryCheckoutStatus::NotRequested;
                return snapshot_from_state(&view, attachment_assets, None);
            }
        }
        snapshot_from_state(session, attachment_assets, node_id)
    }

    fn refresh_attachment_catalog(&mut self) {
        self.attachment_assets = rebuild_attachment_assets(
            &self.sessions,
            &self.attachment_assets,
            &self.attachment_root,
        );
        self.session_attachment_index = rebuild_session_attachment_index(&self.sessions);
    }

    fn remove_session_attachment_catalog(&mut self, session_id: &str) {
        let removed_asset_ids = self
            .session_attachment_index
            .remove(session_id)
            .unwrap_or_default()
            .into_iter()
            .collect::<HashSet<_>>();
        self.attachment_assets.retain(|asset_id, asset| {
            asset.session_id != session_id && !removed_asset_ids.contains(asset_id)
        });
    }
}

impl FileSessionBackend {
    pub fn new(storage_path: PathBuf) -> Self {
        Self { storage_path }
    }
}

impl SessionBackend for FileSessionBackend {
    fn load_store(&self) -> Option<PersistedStore> {
        eprintln!(
            "[pony-agent][session] loading sessions from {}",
            self.storage_path.display()
        );
        load_store_from_path(&self.storage_path)
    }

    fn save_store(&self, store: &PersistedStore) {
        let Some(parent) = self.storage_path.parent() else {
            return;
        };
        if fs::create_dir_all(parent).is_err() {
            return;
        }

        let Ok(serialized) = serde_json::to_string_pretty(store) else {
            return;
        };
        eprintln!(
            "[pony-agent][session] saving sessions to {}",
            self.storage_path.display()
        );
        let _ = fs::write(&self.storage_path, serialized);
    }

    fn attachment_root(&self) -> Option<PathBuf> {
        self.storage_path
            .parent()
            .map(|parent| parent.join("attachments"))
    }
}

#[cfg(test)]
impl SessionBackend for MemorySessionBackend {
    fn load_store(&self) -> Option<PersistedStore> {
        None
    }

    fn save_store(&self, _store: &PersistedStore) {}

    fn attachment_root(&self) -> Option<PathBuf> {
        Some(self.attachment_root.clone())
    }
}

#[cfg(test)]
impl Drop for MemorySessionBackend {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.attachment_root);
    }
}

/// 从 trace_timeline 提取最终 assistant 消息的 reasoning_content。
/// 取最后一条 `call_model` 类型 timeline entry 的 reasoning_content，
/// 与前端 traceReasoningContent 逻辑一致。
fn extract_reasoning_content(trace: &TurnTraceRecord) -> Option<String> {
    trace
        .trace_timeline
        .iter()
        .rev()
        .find(|e| e.kind == "call_model")
        .and_then(|e| e.reasoning_content.clone())
}

/// TurnTraceRecord.phase → MessageStatus 映射
fn derive_status_from_trace(trace: &TurnTraceRecord) -> MessageStatus {
    match trace.phase.as_str() {
        "completed" => MessageStatus::Done,
        "failed" | "cancelled" => MessageStatus::Error,
        _ => MessageStatus::Error,
    }
}

/// 用 turn_trace_history 中的元数据补全 TurnHistoryMessage 列表。
/// 使用末端对齐策略处理 history 与 turn_trace_history 截断步长不一致的问题。
/// 幂等：已携带元数据的条目不覆写。
fn enrich_history_from_traces(
    history: &mut [TurnHistoryMessage],
    turn_trace_history: &[TurnTraceRecord],
) {
    let msg_count = history.len();
    let trace_count = turn_trace_history.len();
    if msg_count == 0 || trace_count == 0 {
        return;
    }
    let offset = trace_count.saturating_sub(msg_count / 2);

    for (i, msg) in history.iter_mut().enumerate() {
        let turn_idx = i / 2;
        let trace_idx = turn_idx + offset;

        if msg.turn_id.is_some() {
            continue; // 幂等：跳过已携带元数据的条目
        }

        if let Some(trace) = turn_trace_history.get(trace_idx) {
            if msg.role == "assistant" {
                msg.turn_id = Some(trace.turn_id.clone());
                msg.model_name = trace.provider_model.clone();
                msg.token_count = trace.output_tokens;
                msg.reasoning_content = extract_reasoning_content(trace);
                msg.status = Some(derive_status_from_trace(trace));
            } else if msg.role == "user" {
                msg.turn_id = Some(trace.turn_id.clone());
            }
        }
    }
}

/// 会话尚不存在时的只读默认快照（与 ensure_session 默认结构一致，但不创建、不落盘）。
fn default_snapshot_for_session(session_key: &str, node_id: Option<&str>) -> SessionSnapshot {
    let session = SessionState {
        conversation_id: session_key.to_string(),
        title: default_session_title(),
        summary: DEFAULT_SESSION_SUMMARY.to_string(),
        history: Vec::new(),
        provider_native_transcript: Vec::new(),
        turn_trace_history: Vec::new(),
        trace_migration_state: TraceMigrationState::LegacyBlob,
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
        history_cursor: HistoryCursor {
            session_id: session_key.to_string(),
            ..HistoryCursor::default()
        },
        event_watermark: 0,
        last_commit_watermark: 0,
        workspace_id: None,
    };
    snapshot_from_state(&session, Vec::new(), node_id)
}

fn snapshot_from_state(
    session: &SessionState,
    attachment_assets: Vec<AttachmentAsset>,
    node_id: Option<&str>,
) -> SessionSnapshot {
    let latest_node_id = session
        .history_branches
        .iter()
        .find(|branch| {
            session
                .history_cursor
                .active_branch_id
                .as_deref()
                .map(|active| active == branch.branch_id)
                .unwrap_or(branch.branch_id == DEFAULT_HISTORY_BRANCH_ID)
        })
        .and_then(|branch| branch.head_node_id.clone())
        .or_else(|| {
            session
                .history_nodes
                .last()
                .map(|node| node.node_id.clone())
        });

    if let Some(selected_node) = node_id.and_then(|id| history_node(session, id)) {
        let branch_head_node_id = session
            .history_branches
            .iter()
            .find(|branch| branch.branch_id == selected_node.branch_id)
            .and_then(|branch| branch.head_node_id.clone());
        let checkout_status = if session.history_cursor.visible_node_id.as_deref()
            == Some(selected_node.node_id.as_str())
        {
            session.history_cursor.checkout_status.clone()
        } else {
            HistoryCheckoutStatus::NotRequested
        };
        let history_cursor = HistoryCursor {
            session_id: session.conversation_id.clone(),
            visible_node_id: Some(selected_node.node_id.clone()),
            active_branch_id: Some(selected_node.branch_id.clone()),
            branch_head_node_id: branch_head_node_id.clone(),
            workspace_node_id: Some(selected_node.node_id.clone()),
            cursor_version: session.history_cursor.cursor_version,
            event_watermark: session.history_cursor.event_watermark,
            mode: if branch_head_node_id.as_deref() == Some(selected_node.node_id.as_str()) {
                HistoryCursorMode::Live
            } else {
                HistoryCursorMode::Historical
            },
            checkout_mode: HistoryCheckoutMode::TranscriptOnly,
            checkout_status,
        };
        let mut enriched_history = selected_node.history.clone();
        enrich_history_from_traces(&mut enriched_history, &selected_node.turn_trace_history);
        return SessionSnapshot {
            conversation_id: session.conversation_id.clone(),
            title: selected_node.title.clone(),
            summary: selected_node.summary.clone(),
            history: enriched_history,
            attachment_assets,
            provider_native_transcript: selected_node.provider_native_transcript.clone(),
            turn_trace_history: selected_node.turn_trace_history.clone(),
            long_term_memory_entries: selected_node.long_term_memory_entries.clone(),
            memory_write_evidence: selected_node.memory_write_evidence.clone(),
            memory_write_hook_trace_records: selected_node.memory_write_hook_trace_records.clone(),
            history_state_evidence: session.history_state_evidence.clone(),
            history_state_audit_summary: build_history_state_audit_summary(
                &history_cursor,
                &session.history_state_evidence,
            ),
            run_control_audit_summary: build_missing_run_control_audit_summary(),
            turn_count: selected_node.turn_count,
            last_referenced_file: selected_node.last_referenced_file.clone(),
            updated_at_ms: session.updated_at_ms,
            // PA-088：轻量投影——节点不携带完整 trace（避免 IPC payload 膨胀导致前端
            // JSON.parse 卡死）；选中节点的 trace 已放快照顶层 turn_trace_history。
            history_nodes: project_lightweight_nodes(&session.history_nodes),
            history_branches: session.history_branches.clone(),
            history_cursor,
            resolved_node_id: Some(selected_node.node_id.clone()),
            latest_node_id,
            env_info: Some(collect_env_info()),
            workspace_id: session.workspace_id.clone(),
        };
    }

    let mut enriched_history = session.history.clone();
    enrich_history_from_traces(&mut enriched_history, &session.turn_trace_history);
    SessionSnapshot {
        conversation_id: session.conversation_id.clone(),
        title: session.title.clone(),
        summary: session.summary.clone(),
        history: enriched_history,
        attachment_assets,
        provider_native_transcript: session.provider_native_transcript.clone(),
        turn_trace_history: session.turn_trace_history.clone(),
        long_term_memory_entries: session.long_term_memory_entries.clone(),
        memory_write_evidence: session.memory_write_evidence.clone(),
        memory_write_hook_trace_records: session.memory_write_hook_trace_records.clone(),
        history_state_evidence: session.history_state_evidence.clone(),
        history_state_audit_summary: build_history_state_audit_summary(
            &session.history_cursor,
            &session.history_state_evidence,
        ),
        run_control_audit_summary: build_missing_run_control_audit_summary(),
        turn_count: session.turn_count,
        last_referenced_file: session.last_referenced_file.clone(),
        updated_at_ms: session.updated_at_ms,
        // PA-088：轻量投影（同选中节点路径）。
        history_nodes: project_lightweight_nodes(&session.history_nodes),
        history_branches: session.history_branches.clone(),
        history_cursor: session.history_cursor.clone(),
        resolved_node_id: session.history_cursor.visible_node_id.clone(),
        latest_node_id,
        env_info: Some(collect_env_info()),
        workspace_id: session.workspace_id.clone(),
    }
}

fn build_history_state_audit_summary(
    cursor: &HistoryCursor,
    evidence: &[HistoryStateHookEvidence],
) -> HistoryStateAuditSummary {
    let action = evidence.last().map_or_else(
        || HistoryStateAuditActionSummary {
            status: "missing".to_string(),
            source_family: "history_state".to_string(),
            command_kind: None,
            boundary: None,
            result_kind: None,
            summary: "history-state audit summary unavailable".to_string(),
            elapsed_ms: None,
            blocked: false,
            degraded: false,
            evidence_id: None,
            observed_at_ms: None,
            requested_node_id: None,
            requested_branch_id: None,
            resolved_node_id: None,
            resolved_branch_id: None,
        },
        |latest| HistoryStateAuditActionSummary {
            status: "available".to_string(),
            source_family: "history_state".to_string(),
            command_kind: Some(latest.command_kind.clone()),
            boundary: Some(latest.boundary.clone()),
            result_kind: Some(latest.result_kind.clone()),
            summary: latest.summary.clone(),
            elapsed_ms: Some(latest.elapsed_ms),
            blocked: latest.blocked,
            degraded: latest.degraded,
            evidence_id: Some(latest.evidence_id.clone()),
            observed_at_ms: Some(latest.recorded_at_ms),
            requested_node_id: latest.requested_node_id.clone(),
            requested_branch_id: latest.requested_branch_id.clone(),
            resolved_node_id: latest.resolved_node_id.clone(),
            resolved_branch_id: latest.resolved_branch_id.clone(),
        },
    );

    HistoryStateAuditSummary {
        action,
        current_context: HistoryStateAuditCurrentContext {
            mode: history_cursor_mode_label(&cursor.mode).to_string(),
            visible_node_id: cursor.visible_node_id.clone(),
            active_branch_id: cursor.active_branch_id.clone(),
            branch_head_node_id: cursor.branch_head_node_id.clone(),
            workspace_node_id: cursor.workspace_node_id.clone(),
        },
    }
}

pub fn build_missing_run_control_audit_summary() -> RunControlAuditSummary {
    RunControlAuditSummary {
        action_evidence_summary: RunControlAuditActionSummary {
            status: "missing".to_string(),
            source_family: "run_control".to_string(),
            command_kind: None,
            boundary: None,
            result_kind: None,
            summary: "run-control audit summary unavailable".to_string(),
            target_summary: "target unavailable".to_string(),
            elapsed_ms: None,
            blocked: false,
            degraded: false,
            evidence_id: None,
            observed_at_ms: None,
            run_id: None,
            turn_id: None,
            checkpoint_turn_id: None,
            checkpoint_kind: None,
            recovery_mode: None,
            projected_command: None,
            degradation_reason: None,
            request_summary: None,
            start_reason: None,
        },
        current_context_projection: RunControlAuditCurrentContext {
            phase: "idle".to_string(),
            checkpoint_status: "missing".to_string(),
            active_run_id: None,
            checkpoint_kind: None,
            checkpoint_recovery_mode: None,
            submission_plan_command: None,
        },
    }
}

fn ensure_history_graph(session: &mut SessionState) -> bool {
    let mut changed = false;
    if session.history_cursor.session_id.is_empty() {
        session.history_cursor.session_id = session.conversation_id.clone();
        changed = true;
    }
    if session.history_branches.is_empty() {
        session.history_branches.push(HistoryBranch {
            branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
            session_id: session.conversation_id.clone(),
            base_node_id: None,
            head_node_id: None,
            forked_from_branch_id: None,
            forked_from_node_id: None,
            label: "main".to_string(),
            created_at_ms: session.updated_at_ms,
            updated_at_ms: session.updated_at_ms,
        });
        changed = true;
    }
    if session.history_nodes.is_empty() && !session.history.is_empty() {
        session.history_nodes.push(HistoryNode {
            node_id: legacy_history_root_node_id(session),
            session_id: session.conversation_id.clone(),
            parent_node_id: None,
            branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
            forked_from_node_id: None,
            kind: HistoryNodeKind::Checkpoint,
            run_id: None,
            workspace_ref: WorkspaceRef::default(),
            summary: DEFAULT_SESSION_SUMMARY.to_string(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            history: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: Vec::new(),
            turn_id: None,
            turn_trace_refs: Some(Vec::new()),
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            turn_count: 0,
            last_referenced_file: None,
            created_at_ms: session.updated_at_ms.saturating_sub(1),
            event_seq_range: None,
        });
        let user_indexes = session
            .history
            .iter()
            .enumerate()
            .filter_map(|(index, message)| (message.role == "user").then_some(index))
            .collect::<Vec<_>>();
        for (turn_index, _start) in user_indexes.iter().enumerate() {
            let end = user_indexes
                .get(turn_index + 1)
                .copied()
                .unwrap_or(session.history.len());
            let history = session.history[..end].to_vec();
            let mut materialized = SessionState {
                conversation_id: session.conversation_id.clone(),
                title: DEFAULT_SESSION_TITLE.to_string(),
                summary: DEFAULT_SESSION_SUMMARY.to_string(),
                history,
                provider_native_transcript: if turn_index + 1 == user_indexes.len() {
                    session.provider_native_transcript.clone()
                } else {
                    Vec::new()
                },
                turn_trace_history: session
                    .turn_trace_history
                    .iter()
                    .take((turn_index + 1).min(session.turn_trace_history.len()))
                    .cloned()
                    .collect(),
                trace_migration_state: session.trace_migration_state,
                turn_trace_refs: None,
                long_term_memory_entries: replay_long_term_memory(&session.history[..end]),
                memory_write_evidence: Vec::new(),
                memory_write_hook_trace_records: Vec::new(),
                history_state_evidence: Vec::new(),
                turn_count: 0,
                last_referenced_file: None,
                updated_at_ms: session.updated_at_ms,
                history_nodes: Vec::new(),
                history_branches: Vec::new(),
                history_cursor: HistoryCursor::default(),
                event_watermark: 0,
                last_commit_watermark: 0,
                workspace_id: session.workspace_id.clone(),
            };
            refresh_session_metadata(&mut materialized, false);
            session.history_nodes.push(HistoryNode {
                node_id: legacy_history_node_id(session, turn_index + 1),
                session_id: session.conversation_id.clone(),
                parent_node_id: session
                    .history_nodes
                    .last()
                    .map(|node| node.node_id.clone()),
                branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                forked_from_node_id: None,
                kind: HistoryNodeKind::TurnCommitted,
                run_id: materialized
                    .turn_trace_history
                    .last()
                    .map(|trace| trace.turn_id.clone()),
                workspace_ref: WorkspaceRef::default(),
                summary: materialized.summary.clone(),
                title: materialized.title.clone(),
                history: materialized.history.clone(),
                provider_native_transcript: materialized.provider_native_transcript.clone(),
                turn_trace_history: materialized.turn_trace_history.clone(),
                turn_id: materialized
                    .turn_trace_history
                    .last()
                    .map(|trace| trace.turn_id.clone()),
                turn_trace_refs: Some(
                    materialized
                        .turn_trace_history
                        .iter()
                        .map(|trace| TurnTraceRef {
                            turn_id: trace.turn_id.clone(),
                            updated_at_ms: trace.updated_at,
                            version: None,
                        })
                        .collect(),
                ),
                long_term_memory_entries: materialized.long_term_memory_entries.clone(),
                memory_write_evidence: materialized.memory_write_evidence.clone(),
                memory_write_hook_trace_records: materialized
                    .memory_write_hook_trace_records
                    .clone(),
                turn_count: materialized.turn_count,
                last_referenced_file: materialized.last_referenced_file.clone(),
                created_at_ms: session.updated_at_ms.saturating_add(turn_index as u64),
                event_seq_range: None,
            });
        }
        changed = true;
    }

    let root_node_id = legacy_history_root_node_id(session);
    let first_main_branch_turn_index = session
        .history_nodes
        .iter()
        .position(|node| node.branch_id == DEFAULT_HISTORY_BRANCH_ID && node.turn_count > 0);
    let has_root_node = session
        .history_nodes
        .iter()
        .any(|node| node.node_id == root_node_id);
    if !has_root_node {
        if let Some(first_index) = first_main_branch_turn_index {
            let first_node_id = session.history_nodes[first_index].node_id.clone();
            let main_branch_base_node_id = session
                .history_branches
                .iter()
                .find(|branch| branch.branch_id == DEFAULT_HISTORY_BRANCH_ID)
                .and_then(|branch| branch.base_node_id.clone());
            let should_insert_root = session.history_nodes[first_index].parent_node_id.is_none()
                && main_branch_base_node_id
                    .as_deref()
                    .map(|base| base == first_node_id)
                    .unwrap_or(true);
            if should_insert_root {
                let created_at_ms = session.history_nodes[first_index]
                    .created_at_ms
                    .saturating_sub(1);
                session.history_nodes.insert(
                    first_index,
                    HistoryNode {
                        node_id: root_node_id.clone(),
                        session_id: session.conversation_id.clone(),
                        parent_node_id: None,
                        branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                        forked_from_node_id: None,
                        kind: HistoryNodeKind::Checkpoint,
                        run_id: None,
                        workspace_ref: WorkspaceRef::default(),
                        summary: DEFAULT_SESSION_SUMMARY.to_string(),
                        title: DEFAULT_SESSION_TITLE.to_string(),
                        history: Vec::new(),
                        provider_native_transcript: Vec::new(),
                        turn_trace_history: Vec::new(),
                        turn_id: None,
                        turn_trace_refs: Some(Vec::new()),
                        long_term_memory_entries: Vec::new(),
                        memory_write_evidence: Vec::new(),
                        memory_write_hook_trace_records: Vec::new(),
                        turn_count: 0,
                        last_referenced_file: None,
                        created_at_ms,
                        event_seq_range: None,
                    },
                );
                if let Some(first_node) = session.history_nodes.get_mut(first_index + 1) {
                    first_node.parent_node_id = Some(root_node_id.clone());
                }
                changed = true;
            }
        }
    }

    let latest_node_id = session
        .history_nodes
        .last()
        .map(|node| node.node_id.clone());
    let main_branch_head_node_id = session
        .history_nodes
        .iter()
        .rev()
        .find(|node| node.branch_id == DEFAULT_HISTORY_BRANCH_ID)
        .map(|node| node.node_id.clone());
    let first_node_id = session
        .history_nodes
        .first()
        .map(|node| node.node_id.clone());
    let updated_at_ms = session.updated_at_ms;
    let has_legacy_root_node = session
        .history_nodes
        .iter()
        .any(|node| node.node_id == root_node_id);
    if let Some(main_branch) = history_branch_mut(session, DEFAULT_HISTORY_BRANCH_ID) {
        if main_branch.base_node_id.is_none() {
            main_branch.base_node_id = first_node_id;
            changed = true;
        }
        if main_branch.base_node_id.as_deref() != Some(root_node_id.as_str())
            && has_legacy_root_node
        {
            main_branch.base_node_id = Some(root_node_id.clone());
            changed = true;
        }
        if main_branch.head_node_id != main_branch_head_node_id {
            main_branch.head_node_id = main_branch_head_node_id.clone();
            main_branch.updated_at_ms = updated_at_ms;
            changed = true;
        }
    }

    if session.history_cursor.active_branch_id.is_none() {
        session.history_cursor.active_branch_id = Some(DEFAULT_HISTORY_BRANCH_ID.to_string());
        changed = true;
    }
    let active_branch_head_node_id =
        session
            .history_cursor
            .active_branch_id
            .as_deref()
            .and_then(|branch_id| {
                session
                    .history_branches
                    .iter()
                    .find(|branch| branch.branch_id == branch_id)
                    .and_then(|branch| branch.head_node_id.clone())
            });
    if session.history_cursor.branch_head_node_id != active_branch_head_node_id {
        session.history_cursor.branch_head_node_id = active_branch_head_node_id.clone();
        changed = true;
    }
    if session.history_cursor.visible_node_id.is_none() {
        session.history_cursor.visible_node_id = active_branch_head_node_id.clone();
        changed = true;
    }
    if session.history_cursor.workspace_node_id.is_none() {
        session.history_cursor.workspace_node_id = active_branch_head_node_id.or(latest_node_id);
        changed = true;
    }

    changed
}

fn prepare_session_for_new_turn(session: &mut SessionState) {
    let Some(visible_node_id) = session.history_cursor.visible_node_id.clone() else {
        return;
    };
    let Some(branch_head_node_id) = session.history_cursor.branch_head_node_id.clone() else {
        return;
    };
    if visible_node_id == branch_head_node_id {
        return;
    }

    let Some(visible_node) = history_node(session, &visible_node_id).cloned() else {
        return;
    };
    // PA-093：引用化节点（事件流重建视图）不在续写路径重复 hydrate——
    // checkout/restore/switch 已折叠填充；此处仅创建隐式 fork 分支。
    if visible_node.event_seq_range.is_none() {
        hydrate_session_from_node(session, &visible_node);
    }
    let previous_branch_id = session
        .history_cursor
        .active_branch_id
        .clone()
        .unwrap_or_else(|| DEFAULT_HISTORY_BRANCH_ID.to_string());
    let new_branch_id = new_history_branch_id(session, session.history_branches.len() + 1);
    let created_at_ms = now_timestamp_ms();
    session.history_branches.push(HistoryBranch {
        branch_id: new_branch_id.clone(),
        session_id: session.conversation_id.clone(),
        base_node_id: Some(visible_node_id.clone()),
        head_node_id: Some(visible_node_id.clone()),
        forked_from_branch_id: Some(previous_branch_id),
        forked_from_node_id: Some(visible_node_id.clone()),
        label: format!("fork-{}", session.history_branches.len() + 1),
        created_at_ms,
        updated_at_ms: created_at_ms,
    });
    session.history_cursor.visible_node_id = Some(visible_node_id.clone());
    session.history_cursor.active_branch_id = Some(new_branch_id.clone());
    session.history_cursor.branch_head_node_id = Some(visible_node_id.clone());
    session.history_cursor.workspace_node_id = Some(visible_node_id);
    session.history_cursor.mode = HistoryCursorMode::HistoricalDirty;
}

fn commit_history_node_from_live_state(
    session: &mut SessionState,
    kind: HistoryNodeKind,
    run_id: Option<String>,
) {
    let created_at_ms = now_timestamp_ms();
    let parent_node_id = session.history_cursor.visible_node_id.clone();
    let branch_id = session
        .history_cursor
        .active_branch_id
        .clone()
        .unwrap_or_else(|| DEFAULT_HISTORY_BRANCH_ID.to_string());
    let forked_from_node_id = session
        .history_branches
        .iter()
        .find(|branch| branch.branch_id == branch_id)
        .and_then(|branch| {
            if branch.head_node_id == parent_node_id {
                None
            } else {
                branch.forked_from_node_id.clone()
            }
        });
    let node_id = new_history_node_id(session, session.history_nodes.len() + 1, created_at_ms);
    session.history_nodes.push(HistoryNode {
        node_id: node_id.clone(),
        session_id: session.conversation_id.clone(),
        parent_node_id,
        branch_id: branch_id.clone(),
        forked_from_node_id,
        kind,
        run_id,
        workspace_ref: WorkspaceRef::default(),
        summary: session.summary.clone(),
        title: session.title.clone(),
        history: session.history.clone(),
        provider_native_transcript: session.provider_native_transcript.clone(),
        turn_trace_history: session.turn_trace_history.clone(),
        turn_id: session.turn_trace_history.last().map(|trace| trace.turn_id.clone()),
        turn_trace_refs: Some(
            session
                .turn_trace_history
                .iter()
                .map(|trace| TurnTraceRef {
                    turn_id: trace.turn_id.clone(),
                    updated_at_ms: trace.updated_at,
                    version: None,
                })
                .collect(),
        ),
        long_term_memory_entries: session.long_term_memory_entries.clone(),
        memory_write_evidence: session.memory_write_evidence.clone(),
        memory_write_hook_trace_records: session.memory_write_hook_trace_records.clone(),
        turn_count: session.turn_count,
        last_referenced_file: session.last_referenced_file.clone(),
        created_at_ms,
        // PA-093：引用化由 finalize_event_watermark 在事件 flush 后升级
        // （commit 先于 flush 的时序下，此处先以 legacy 快照提交）。
        event_seq_range: None,
    });
    // PA-093：记录本次提交时的事件水位（引用化区间起点）。
    session.last_commit_watermark = session.event_watermark;
    if let Some(branch) = history_branch_mut(session, &branch_id) {
        if branch.base_node_id.is_none() {
            branch.base_node_id = Some(node_id.clone());
        }
        branch.head_node_id = Some(node_id.clone());
        branch.updated_at_ms = created_at_ms;
    }
    session.history_cursor.visible_node_id = Some(node_id.clone());
    session.history_cursor.branch_head_node_id = Some(node_id.clone());
    session.history_cursor.workspace_node_id = Some(node_id.clone());
    session.history_cursor.mode = HistoryCursorMode::Live;
    session.history_cursor.checkout_mode = HistoryCheckoutMode::TranscriptOnly;
    session.history_cursor.checkout_status = HistoryCheckoutStatus::NotRequested;
    bind_unanchored_memory_write_evidence_to_history_node(session, &node_id);
}

fn sync_latest_history_node(session: &mut SessionState, run_id: Option<String>) {
    let Some(latest_node_id) = session.history_cursor.branch_head_node_id.clone() else {
        return;
    };
    let summary = session.summary.clone();
    let title = session.title.clone();
    let history = session.history.clone();
    let provider_native_transcript = session.provider_native_transcript.clone();
    let turn_trace_history = session.turn_trace_history.clone();
    let long_term_memory_entries = session.long_term_memory_entries.clone();
    let memory_write_evidence = session.memory_write_evidence.clone();
    let memory_write_hook_trace_records = session.memory_write_hook_trace_records.clone();
    let turn_count = session.turn_count;
    let last_referenced_file = session.last_referenced_file.clone();
    let Some(node) = history_node_mut(session, &latest_node_id) else {
        return;
    };
    node.summary = summary;
    node.title = title;
    // PA-093：引用化节点（event_seq_range 已定）不再内嵌快照——事件流是
    // 唯一真源，sync 只同步元数据（summary/title/run_id），避免快照复活。
    if node.event_seq_range.is_none() {
        node.history = history;
        node.provider_native_transcript = provider_native_transcript;
        node.turn_trace_history = turn_trace_history;
        // PA-088：同步更新 turn_id 与 refs，保持内存态与持久化重算一致
        // （前端 checkpoint 列表直接读 node.turnId）。
        node.turn_id = node.turn_trace_history.last().map(|trace| trace.turn_id.clone());
        node.turn_trace_refs = Some(
            node.turn_trace_history
                .iter()
                .map(|trace| TurnTraceRef {
                    turn_id: trace.turn_id.clone(),
                    updated_at_ms: trace.updated_at,
                    version: None,
                })
                .collect(),
        );
        node.long_term_memory_entries = long_term_memory_entries;
        node.memory_write_evidence = memory_write_evidence;
        node.memory_write_hook_trace_records = memory_write_hook_trace_records;
        node.turn_count = turn_count;
        node.last_referenced_file = last_referenced_file;
    }
    if run_id.is_some() {
        node.run_id = run_id;
    }
}

fn bind_unanchored_memory_write_evidence_to_history_node(
    session: &mut SessionState,
    history_node_id: &str,
) {
    let mut changed = false;
    for evidence in &mut session.memory_write_evidence {
        if evidence.source_history_node_id.is_none() {
            evidence.source_history_node_id = Some(history_node_id.to_string());
            changed = true;
        }
    }
    if !changed {
        return;
    }

    let evidence = session.memory_write_evidence.clone();
    let Some(node) = history_node_mut(session, history_node_id) else {
        return;
    };
    node.memory_write_evidence = evidence;
}

fn hydrate_session_from_node(session: &mut SessionState, node: &HistoryNode) {
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

/// PA-093：引用化节点 → 会话内存态（history/transcript/trace 由事件折叠提供，
/// memory/摘要等节点元数据仍从节点拷贝；transcript 事件流不覆盖 → 引用化后为空，
/// 前端 transcript 视图依赖 trace 数据，文档已声明降级）。
fn hydrate_session_from_projection(
    session: &mut SessionState,
    node: &HistoryNode,
    history: Vec<TurnHistoryMessage>,
    trace: Vec<TurnTraceRecord>,
) {
    session.title = node.title.clone();
    session.summary = node.summary.clone();
    session.history = history;
    session.provider_native_transcript = node.provider_native_transcript.clone();
    session.turn_trace_history = trace;
    session.long_term_memory_entries = node.long_term_memory_entries.clone();
    session.memory_write_evidence = node.memory_write_evidence.clone();
    session.memory_write_hook_trace_records = node.memory_write_hook_trace_records.clone();
    session.turn_count = node.turn_count;
    session.last_referenced_file = node.last_referenced_file.clone();
}

/// PA-093：事件流 → 会话视图（history/trace 双投影，按分支可见集合过滤）。
/// 可见集合语义：初始 = 目标节点所在分支的血缘链（折叠目标是该节点的视图）；
/// 流内 `checkpoint/checkout` 事件按序重放更新集合（历史切换序列正确重放）；
/// 其余事件 branch_id ∉ 可见集合则跳过（被撤回分支的事件不复活）。
fn fold_session_views(
    events: &[(u64, String, crate::agent::turn_event::TurnEvent)],
    node_branch_id: &str,
    nodes: &[HistoryNode],
    branches: &[HistoryBranch],
) -> (Vec<TurnHistoryMessage>, Vec<TurnTraceRecord>) {
    use crate::agent::projection::{
        fold_all_with_branches, HistoryProjectionState, TraceProjectionState,
    };
    let node_branch: HashMap<&str, &str> = nodes
        .iter()
        .map(|n| (n.node_id.as_str(), n.branch_id.as_str()))
        .collect();
    let history_state = fold_all_with_branches::<_, HistoryProjectionState>(
        events,
        node_branch_id,
        &node_branch,
        branches,
    );
    let trace_state = fold_all_with_branches::<_, TraceProjectionState>(
        events,
        node_branch_id,
        &node_branch,
        branches,
    );
    (history_state.messages(), trace_state.traces())
}

/// PA-088/PA-090：收集会话的 trace 全量 Union（顶层 ∪ 全部节点 trace，按 turn_id 去重取最新）。
/// 用于 Authoritative 会话写表——保证节点 refs 在重启 materialize 时可解析。
/// **稳定顺序（PA-090）**：顶层 trace 原位顺序优先 → 节点独有 trace 按节点顺序追加；
/// 同 turn_id 多版本取 updated_at 最新（同时间顶层优先）。禁止 HashMap 无序输出。
fn collect_trace_union(session: &SessionState) -> Vec<TurnTraceRecord> {
    let mut by_id: HashMap<String, TurnTraceRecord> = HashMap::new();
    let mut ordered: Vec<TurnTraceRecord> = Vec::new();

    // 顶层原位优先
    for trace in &session.turn_trace_history {
        if let Some(existing) = by_id.get(&trace.turn_id) {
            if trace.updated_at > existing.updated_at {
                if let Some(slot) = ordered.iter_mut().find(|t| t.turn_id == trace.turn_id) {
                    *slot = trace.clone();
                }
                by_id.insert(trace.turn_id.clone(), trace.clone());
            }
        } else {
            by_id.insert(trace.turn_id.clone(), trace.clone());
            ordered.push(trace.clone());
        }
    }

    // 节点独有按节点顺序追加
    for node in &session.history_nodes {
        for trace in &node.turn_trace_history {
            if let Some(existing) = by_id.get(&trace.turn_id) {
                if trace.updated_at > existing.updated_at {
                    if let Some(slot) = ordered.iter_mut().find(|t| t.turn_id == trace.turn_id) {
                        *slot = trace.clone();
                    }
                    by_id.insert(trace.turn_id.clone(), trace.clone());
                }
            } else {
                by_id.insert(trace.turn_id.clone(), trace.clone());
                ordered.push(trace.clone());
            }
        }
    }

    ordered
}

/// PA-088：轻量节点投影——清空节点内嵌 trace（保留 turnId/refs/摘要/元数据）。
/// 前端 trace 面板读快照顶层 turn_trace_history；节点只用于 checkpoint/回滚映射。
/// 存量节点（legacy 反序列化）turn_id 可能为 None：先回填（trace 末条），
/// 否则前端 checkpoint 列表（依赖 node.turnId）会丢失全部存量节点。
fn project_lightweight_nodes(nodes: &[HistoryNode]) -> Vec<HistoryNode> {
    nodes
        .iter()
        .map(|node| {
            let mut projected = node.clone();
            if projected.turn_id.is_none() {
                projected.turn_id = projected
                    .turn_trace_history
                    .last()
                    .map(|trace| trace.turn_id.clone());
            }
            projected.turn_trace_history.clear();
            projected
        })
        .collect()
}

fn session_state_for_backend(
    session: &SessionState,
    trace_mode: SeparateTraceTableMode,
) -> SessionState {
    let mut prepared = session.clone();
    match (trace_mode, prepared.trace_migration_state) {
        (SeparateTraceTableMode::Off, _) => {}
        (SeparateTraceTableMode::DualWrite, TraceMigrationState::LegacyBlob) => {
            prepared.trace_migration_state = TraceMigrationState::DualWrite;
        }
        (SeparateTraceTableMode::DualWrite, TraceMigrationState::DualWrite)
        | (SeparateTraceTableMode::DualWrite, TraceMigrationState::TraceTableAuthoritative) => {}
        (SeparateTraceTableMode::WriteSeparate, TraceMigrationState::TraceTableAuthoritative) => {
            prepared.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
        }
        // PA-088：WriteSeparate + DualWrite 保持双写（blob 完整 + 表），不晋升剥离。
        // 若在此晋升，内存态仍是 DualWrite（save_to_backend 按内存态分派跳过写表），
        // 持久化副本却被剥离 → 表里没有节点 trace → 重启 materialize 失败（数据丢失）。
        // 存量会话的迁移归 PA-090。
        (SeparateTraceTableMode::WriteSeparate, TraceMigrationState::DualWrite) => {}
        (SeparateTraceTableMode::WriteSeparate, TraceMigrationState::LegacyBlob) => {
            // Do not auto-promote a legacy-only session straight to authoritative.
        }
    }
    if matches!(trace_mode, SeparateTraceTableMode::WriteSeparate)
        && matches!(
            prepared.trace_migration_state,
            TraceMigrationState::TraceTableAuthoritative
        )
    {
        // PA-088：authoritative 会话持久化时剥离 trace，只保留轻量引用。
        // 内存中的 SessionState 不受影响（本函数只作用于持久化副本），
        // 运行中 checkout/fork 仍使用内存完整快照；重启后由 load_store 按 refs 物化。
        prepared.turn_trace_refs = Some(
            prepared
                .turn_trace_history
                .iter()
                .map(|trace| TurnTraceRef {
                    turn_id: trace.turn_id.clone(),
                    updated_at_ms: trace.updated_at,
                    version: None,
                })
                .collect(),
        );
        prepared.turn_trace_history.clear();
        for node in &mut prepared.history_nodes {
            node.turn_trace_refs = Some(
                node.turn_trace_history
                    .iter()
                    .map(|trace| TurnTraceRef {
                        turn_id: trace.turn_id.clone(),
                        updated_at_ms: trace.updated_at,
                        version: None,
                    })
                    .collect(),
            );
            node.turn_id = node.turn_trace_history.last().map(|trace| trace.turn_id.clone());
            node.turn_trace_history.clear();
        }
    }
    prepared
}

enum TracePersistenceAction {
    ReplaceAll,
    UpsertOne {
        trace: TurnTraceRecord,
        trace_order: usize,
    },
    UpdateTerminalEvent {
        turn_id: String,
        event_id: Option<String>,
        event_type: Option<String>,
        event_version: Option<String>,
        sequence: Option<u64>,
        emitted_at_ms: Option<u64>,
        updated_at: u64,
    },
    AppendHookRecords {
        turn_id: String,
        hook_trace_records: Vec<HookTraceRecord>,
        updated_at: u64,
    },
}

fn trace_action_from_mutation(mutation: SessionTraceMutation) -> TracePersistenceAction {
    match mutation {
        SessionTraceMutation::ReplaceAll { .. } => TracePersistenceAction::ReplaceAll,
        SessionTraceMutation::UpsertOne { trace, trace_order } => {
            TracePersistenceAction::UpsertOne { trace, trace_order }
        }
        SessionTraceMutation::UpdateTerminalEvent {
            turn_id,
            event_id,
            event_type,
            event_version,
            sequence,
            emitted_at_ms,
            updated_at,
        } => TracePersistenceAction::UpdateTerminalEvent {
            turn_id,
            event_id,
            event_type,
            event_version,
            sequence,
            emitted_at_ms,
            updated_at,
        },
        SessionTraceMutation::AppendHookRecords {
            turn_id,
            hook_trace_records,
            updated_at,
        } => TracePersistenceAction::AppendHookRecords {
            turn_id,
            hook_trace_records,
            updated_at,
        },
    }
}

fn build_history_state_cursor_summary(cursor: &HistoryCursor) -> HistoryStateCursorSummary {
    HistoryStateCursorSummary {
        visible_node_id: cursor.visible_node_id.clone(),
        active_branch_id: cursor.active_branch_id.clone(),
        branch_head_node_id: cursor.branch_head_node_id.clone(),
        workspace_node_id: cursor.workspace_node_id.clone(),
        mode: history_cursor_mode_label(&cursor.mode).to_string(),
        checkout_mode: history_checkout_mode_label(&cursor.checkout_mode).to_string(),
        checkout_status: history_checkout_status_label(&cursor.checkout_status).to_string(),
    }
}

fn history_cursor_mode_label(mode: &HistoryCursorMode) -> &'static str {
    match mode {
        HistoryCursorMode::Live => "live",
        HistoryCursorMode::Historical => "historical",
        HistoryCursorMode::HistoricalDirty => "historical_dirty",
    }
}

fn history_checkout_mode_label(mode: &HistoryCheckoutMode) -> &'static str {
    match mode {
        HistoryCheckoutMode::TranscriptOnly => "transcript_only",
        HistoryCheckoutMode::TranscriptAndWorkspace => "transcript_and_workspace",
    }
}

fn history_checkout_status_label(status: &HistoryCheckoutStatus) -> &'static str {
    match status {
        HistoryCheckoutStatus::NotRequested => "not_requested",
        HistoryCheckoutStatus::Applied => "applied",
        HistoryCheckoutStatus::DegradedToTranscriptOnly => "degraded_to_transcript_only",
    }
}

fn history_state_boundary_label(hook_point: &HistoryStateHookPoint) -> Option<&'static str> {
    match hook_point {
        HistoryStateHookPoint::HistoryCheckoutStart => Some("history.checkout.start"),
        HistoryStateHookPoint::HistoryCheckoutResolved => Some("history.checkout.resolved"),
        HistoryStateHookPoint::BranchRestoreStart => Some("history.branch_restore.start"),
        HistoryStateHookPoint::BranchRestoreResolved => Some("history.branch_restore.resolved"),
        HistoryStateHookPoint::BranchForkStart => Some("history.branch_fork.start"),
        HistoryStateHookPoint::BranchForkResolved => Some("history.branch_fork.resolved"),
        HistoryStateHookPoint::BranchSwitchStart => Some("history.branch_switch.start"),
        HistoryStateHookPoint::BranchSwitchResolved => Some("history.branch_switch.resolved"),
    }
}

fn build_history_state_hook_envelope(
    session: &SessionState,
    hook_point: HistoryStateHookPoint,
    command_kind: HistoryStateCommandKind,
    requested_node_id: Option<&str>,
    requested_branch_id: Option<&str>,
    requested_mode: Option<&HistoryCheckoutMode>,
    resolved_node_id: Option<&str>,
    resolved_branch_id: Option<&str>,
    transcript_restore_applied: bool,
    workspace_rollback_capable: bool,
    workspace_rollback_applied: bool,
    degraded: bool,
    degradation_reason: Option<&str>,
) -> Option<HistoryStateHookEnvelope> {
    Some(HistoryStateHookEnvelope {
        session_id: session.conversation_id.clone(),
        hook_point: hook_point.clone(),
        command_kind,
        source_boundary: history_state_boundary_label(&hook_point)?.to_string(),
        requested_node_id: requested_node_id.map(|value| value.to_string()),
        requested_branch_id: requested_branch_id.map(|value| value.to_string()),
        requested_checkout_mode: requested_mode
            .map(|value| history_checkout_mode_label(value).to_string()),
        resolved_node_id: resolved_node_id.map(|value| value.to_string()),
        resolved_branch_id: resolved_branch_id.map(|value| value.to_string()),
        transcript_restore_applied,
        workspace_rollback_capable,
        workspace_rollback_applied,
        degraded,
        degradation_reason: degradation_reason.map(|value| value.to_string()),
        cursor_summary: build_history_state_cursor_summary(&session.history_cursor),
    })
}

fn history_state_hook_results_blocked(
    hook_results: &[crate::agent::hooks::HookExecutionResult],
) -> bool {
    hook_results.iter().any(|result| {
        result.blocked || matches!(result.structured_result, HookStructuredResult::Deny(_))
    })
}

fn history_state_command_kind_label(command_kind: &HistoryStateCommandKind) -> &'static str {
    match command_kind {
        HistoryStateCommandKind::CheckoutHistoryNode => "checkout_history_node",
        HistoryStateCommandKind::RestoreBranchHead => "restore_branch_head",
        HistoryStateCommandKind::ForkFromHistoryNode => "fork_from_history_node",
        HistoryStateCommandKind::SwitchHistoryBranch => "switch_history_branch",
    }
}

fn hook_result_kind_label(result_kind: &HookResultKind) -> &'static str {
    match result_kind {
        HookResultKind::Observe => "observe",
        HookResultKind::Allow => "allow",
        HookResultKind::Deny => "deny",
        HookResultKind::Patch => "patch",
        HookResultKind::SideEffectRequest => "side_effect_request",
    }
}

fn persist_history_state_hook_evidence(
    session: &mut SessionState,
    envelope: &HistoryStateHookEnvelope,
    hook_results: &[crate::agent::hooks::HookExecutionResult],
) {
    if hook_results.is_empty() {
        return;
    }

    let recorded_at_ms = now_timestamp_ms();
    for (index, result) in hook_results.iter().enumerate() {
        session
            .history_state_evidence
            .push(HistoryStateHookEvidence {
                evidence_id: format!(
                    "history-state:{}:{}:{}:{}",
                    session.conversation_id, envelope.source_boundary, recorded_at_ms, index
                ),
                session_id: session.conversation_id.clone(),
                boundary: envelope.source_boundary.clone(),
                command_kind: history_state_command_kind_label(&envelope.command_kind).to_string(),
                result_kind: hook_result_kind_label(&result.result_kind).to_string(),
                summary: result.trace_summary.clone(),
                elapsed_ms: result.elapsed_ms,
                blocked: result.blocked
                    || matches!(result.structured_result, HookStructuredResult::Deny(_)),
                degraded: envelope.degraded,
                requested_node_id: envelope.requested_node_id.clone(),
                requested_branch_id: envelope.requested_branch_id.clone(),
                resolved_node_id: envelope.resolved_node_id.clone(),
                resolved_branch_id: envelope.resolved_branch_id.clone(),
                recorded_at_ms,
            });
    }
}

fn replay_long_term_memory(history: &[TurnHistoryMessage]) -> Vec<LongTermMemoryRecord> {
    let mut entries = Vec::new();
    for message in history.iter().filter(|message| message.role == "user") {
        for entry in extract_long_term_memory_from_user_message(&message.content) {
            match entries
                .iter_mut()
                .find(|existing| memory_record_identity(existing) == memory_record_identity(&entry))
            {
                Some(existing) => *existing = entry,
                None => entries.push(entry),
            }
        }
    }
    entries
}

fn classify_turn_node_kind(assistant_message: &str) -> HistoryNodeKind {
    if assistant_message.trim() == "用户终止，发送消息可继续。" {
        HistoryNodeKind::TurnCancelled
    } else {
        HistoryNodeKind::TurnCommitted
    }
}

fn history_node<'a>(session: &'a SessionState, node_id: &str) -> Option<&'a HistoryNode> {
    session
        .history_nodes
        .iter()
        .find(|node| node.node_id == node_id)
}

fn history_node_mut<'a>(
    session: &'a mut SessionState,
    node_id: &str,
) -> Option<&'a mut HistoryNode> {
    session
        .history_nodes
        .iter_mut()
        .find(|node| node.node_id == node_id)
}

fn history_branch_mut<'a>(
    session: &'a mut SessionState,
    branch_id: &str,
) -> Option<&'a mut HistoryBranch> {
    session
        .history_branches
        .iter_mut()
        .find(|branch| branch.branch_id == branch_id)
}

fn legacy_history_node_id(session: &SessionState, turn_count: usize) -> String {
    format!("{}-legacy-node-{}", session.conversation_id, turn_count)
}

fn legacy_history_root_node_id(session: &SessionState) -> String {
    format!("{}-legacy-root", session.conversation_id)
}

fn new_history_node_id(session: &SessionState, ordinal: usize, created_at_ms: u64) -> String {
    format!(
        "{}-node-{}-{}",
        session.conversation_id, created_at_ms, ordinal
    )
}

fn new_history_branch_id(session: &SessionState, ordinal: usize) -> String {
    format!("{}-branch-{}", session.conversation_id, ordinal)
}

fn refresh_session_metadata(session: &mut SessionState, touch_updated_at: bool) {
    normalize_trace_timeline_entries(&mut session.turn_trace_history);
    session.turn_count = session
        .history
        .iter()
        .filter(|message| message.role == "user")
        .count();
    session.last_referenced_file = session
        .history
        .iter()
        .rev()
        .find_map(|message| extract_explicit_file_name(&message.content));
    if session.history.is_empty() && !session.turn_trace_history.is_empty() {
        if let Some(trace) = session.turn_trace_history.last() {
            session.title = trace.title.clone();
            session.summary = trace
                .session_summary
                .clone()
                .or(trace.error.clone())
                .or(trace.fallback_reason.clone())
                .unwrap_or_else(|| DEFAULT_SESSION_SUMMARY.to_string());
        }
    } else {
        session.title = build_title(&session.history);
        session.summary =
            build_summary(session.turn_count, session.last_referenced_file.as_deref());
    }
    if touch_updated_at {
        session.updated_at_ms = now_timestamp_ms();
    }
}

fn canonical_trace_timeline_kind(kind: &str) -> &str {
    match kind {
        "context" => "build_context",
        "model" => "call_model",
        "tool" => "call_tool",
        "return" => "return_result",
        other => other,
    }
}

fn normalize_trace_timeline_entries(turn_trace_history: &mut [TurnTraceRecord]) {
    for trace in turn_trace_history {
        for entry in &mut trace.trace_timeline {
            entry.kind = canonical_trace_timeline_kind(&entry.kind).to_string();
        }
    }
}

fn update_long_term_memory_from_user_message(
    session: &mut SessionState,
    user_message: &str,
    hook_executor: &dyn MemoryWriteHookExecutor,
) -> bool {
    let extracted_entries = extract_long_term_memory_from_user_message(user_message);
    if extracted_entries.is_empty() {
        return false;
    }

    let existing_entries = session.long_term_memory_entries.clone();
    let mut planned_writes = plan_memory_write_intents(&existing_entries, &extracted_entries);
    if let Some(envelope) = build_memory_write_hook_envelope(
        &session.conversation_id,
        user_message,
        planned_writes.clone(),
    ) {
        let hook_results = hook_executor.execute(&envelope).unwrap_or_default();
        session.memory_write_hook_trace_records.extend(
            hook_results
                .iter()
                .map(crate::agent::hooks::HookExecutionResult::to_trace_record),
        );
        let hook_outcome =
            apply_memory_write_hook_results(&existing_entries, &mut planned_writes, &hook_results);
        if hook_outcome.blocked {
            return false;
        }
    }

    recalculate_planned_memory_write_operations(&existing_entries, &mut planned_writes);
    if let Some(envelope) = build_memory_write_hook_envelope(
        &session.conversation_id,
        user_message,
        planned_writes.clone(),
    ) {
        session.memory_write_evidence.extend(
            planned_writes
                .iter()
                .filter(|planned| planned.operation != MemoryWriteOperation::Noop)
                .map(|planned| build_persisted_memory_write_evidence(&envelope, planned)),
        );
    }

    let mut changed = false;
    for planned in planned_writes {
        let entry = planned.entry;
        match session
            .long_term_memory_entries
            .iter_mut()
            .find(|existing| memory_record_identity(existing) == memory_record_identity(&entry))
        {
            Some(existing)
                if existing.content == entry.content && existing.source == entry.source => {}
            Some(existing) => {
                *existing = entry;
                changed = true;
            }
            None => {
                session.long_term_memory_entries.push(entry);
                changed = true;
            }
        }
    }

    changed
}

#[derive(Default)]
struct MemoryWriteHookApplicationOutcome {
    blocked: bool,
}

#[derive(Clone)]
struct PlannedMemoryWrite {
    key: String,
    entry: LongTermMemoryRecord,
    operation: MemoryWriteOperation,
}

fn plan_memory_write_intents(
    existing_entries: &[LongTermMemoryRecord],
    extracted_entries: &[LongTermMemoryRecord],
) -> Vec<PlannedMemoryWrite> {
    extracted_entries
        .iter()
        .cloned()
        .map(|entry| {
            let operation = match existing_entries
                .iter()
                .find(|existing| memory_record_identity(existing) == memory_record_identity(&entry))
            {
                Some(existing)
                    if existing.content == entry.content && existing.source == entry.source =>
                {
                    MemoryWriteOperation::Noop
                }
                Some(_) => MemoryWriteOperation::Update,
                None => MemoryWriteOperation::Insert,
            };
            PlannedMemoryWrite {
                key: memory_record_identity(&entry),
                entry,
                operation,
            }
        })
        .collect()
}

fn recalculate_planned_memory_write_operations(
    existing_entries: &[LongTermMemoryRecord],
    planned_writes: &mut [PlannedMemoryWrite],
) {
    for planned in planned_writes {
        planned.operation = match existing_entries.iter().find(|existing| {
            memory_record_identity(existing) == memory_record_identity(&planned.entry)
        }) {
            Some(existing)
                if existing.content == planned.entry.content
                    && existing.source == planned.entry.source =>
            {
                MemoryWriteOperation::Noop
            }
            Some(_) => MemoryWriteOperation::Update,
            None => MemoryWriteOperation::Insert,
        };
        planned.key = memory_record_identity(&planned.entry);
    }
}

fn apply_memory_write_hook_results(
    existing_entries: &[LongTermMemoryRecord],
    planned_writes: &mut [PlannedMemoryWrite],
    hook_results: &[crate::agent::hooks::HookExecutionResult],
) -> MemoryWriteHookApplicationOutcome {
    for result in hook_results {
        if result.blocked {
            return MemoryWriteHookApplicationOutcome { blocked: true };
        }
        if matches!(result.structured_result, HookStructuredResult::Deny(_)) {
            return MemoryWriteHookApplicationOutcome { blocked: true };
        }
    }

    let transform_results = hook_results
        .iter()
        .filter(|result| result.result_kind == crate::agent::hooks::HookResultKind::Patch)
        .cloned()
        .collect::<Vec<_>>();
    if transform_results.is_empty() {
        return MemoryWriteHookApplicationOutcome { blocked: false };
    }

    let merged =
        match merge_patch_results(&transform_results, HookPatchConflictPolicy::LastWriteWins) {
            Ok(merged) => merged,
            Err(_) => return MemoryWriteHookApplicationOutcome { blocked: false },
        };

    for operation in merged.operations {
        let patch = operation.operation;
        if patch.target != HookPatchTarget::MemoryWriteIntent {
            continue;
        }
        if patch.operation != HookPatchOperationKind::Set {
            continue;
        }

        let Some((index, field)) = parse_memory_write_patch_path(&patch.path) else {
            continue;
        };
        let Some(planned) = planned_writes.get_mut(index) else {
            continue;
        };
        let Some(value_text) = patch
            .value_text
            .clone()
            .or_else(|| patch.value_summary.clone())
        else {
            continue;
        };

        match field {
            MemoryWritePatchField::Kind => planned.entry.kind = value_text,
            MemoryWritePatchField::Content => planned.entry.content = value_text,
            MemoryWritePatchField::Source => planned.entry.source = value_text,
            MemoryWritePatchField::Operation => {
                if let Some(operation) = parse_memory_write_operation(&value_text) {
                    planned.operation = operation;
                }
            }
        }
    }

    recalculate_planned_memory_write_operations(existing_entries, planned_writes);
    MemoryWriteHookApplicationOutcome { blocked: false }
}

#[derive(Clone, Copy)]
enum MemoryWritePatchField {
    Kind,
    Content,
    Source,
    Operation,
}

fn parse_memory_write_patch_path(path: &str) -> Option<(usize, MemoryWritePatchField)> {
    let suffix = path.strip_prefix("writes[")?;
    let (index_text, field_text) = suffix.split_once("].")?;
    let index = index_text.parse::<usize>().ok()?;
    let field = match field_text {
        "kind" => MemoryWritePatchField::Kind,
        "content" => MemoryWritePatchField::Content,
        "source" => MemoryWritePatchField::Source,
        "operation" => MemoryWritePatchField::Operation,
        _ => return None,
    };
    Some((index, field))
}

fn parse_memory_write_operation(value: &str) -> Option<MemoryWriteOperation> {
    match value.trim().to_ascii_lowercase().as_str() {
        "insert" => Some(MemoryWriteOperation::Insert),
        "update" => Some(MemoryWriteOperation::Update),
        "noop" => Some(MemoryWriteOperation::Noop),
        _ => None,
    }
}

fn build_memory_write_hook_envelope(
    session_id: &str,
    user_message: &str,
    planned_writes: Vec<PlannedMemoryWrite>,
) -> Option<MemoryWriteHookEnvelope> {
    if planned_writes.is_empty() {
        return None;
    }

    Some(MemoryWriteHookEnvelope {
        session_id: Some(session_id.to_string()),
        hook_point: MemoryWriteHookPoint::LongTermMemoryWrite,
        target: MemoryWriteTarget::LongTermMemory,
        source_boundary: "session.update_long_term_memory_from_user_message".to_string(),
        user_message_summary: summarize_memory_text(user_message),
        writes: planned_writes
            .into_iter()
            .map(|planned| MemoryWriteIntentRecord {
                key: planned.key,
                kind: planned.entry.kind,
                content_summary: summarize_memory_text(&planned.entry.content),
                content: planned.entry.content,
                source: planned.entry.source,
                operation: planned.operation,
            })
            .collect(),
    })
}

fn build_persisted_memory_write_evidence(
    envelope: &MemoryWriteHookEnvelope,
    planned: &PlannedMemoryWrite,
) -> PersistedEffectEvidence {
    PersistedEffectEvidence {
        evidence_id: format!(
            "memory-write:{}:{}",
            planned.key, planned.entry.updated_at_ms
        ),
        effect_kind: "memory_write.long_term_memory".to_string(),
        boundary: "session.update_long_term_memory_from_user_message".to_string(),
        target_session_id: envelope.session_id.clone(),
        source_history_node_id: None,
        target_summary: format!(
            "{}:{}",
            planned.entry.kind,
            summarize_memory_text(&planned.entry.content)
        ),
        persistence_ref: format!("long_term_memory_entries/{}", planned.key),
        replay_decision_basis:
            "persisted memory write evidence is required to avoid replaying side effects"
                .to_string(),
        persisted_at_ms: planned.entry.updated_at_ms,
        replay_required_if_missing: true,
    }
}

fn summarize_memory_text(text: &str) -> String {
    let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized = normalized.trim();
    if normalized.chars().count() <= 80 {
        return normalized.to_string();
    }
    let mut summary = String::new();
    for (index, ch) in normalized.chars().enumerate() {
        if index >= 77 {
            break;
        }
        summary.push(ch);
    }
    summary.push_str("...");
    summary
}

fn extract_long_term_memory_from_user_message(user_message: &str) -> Vec<LongTermMemoryRecord> {
    let lowered = user_message.to_lowercase();
    let updated_at_ms = now_timestamp_ms();
    let mut entries = Vec::new();

    if contains_any_phrase(
        &lowered,
        &[
            "全部使用中文",
            "请用中文回复",
            "请用中文回答",
            "用中文回复",
            "用中文回答",
            "中文回复",
            "中文回答",
            "reply in chinese",
            "answer in chinese",
        ],
    ) {
        entries.push(LongTermMemoryRecord {
            kind: "user_preference.response_language".to_string(),
            content: "Reply in Chinese.".to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if contains_any_phrase(
        &lowered,
        &[
            "请简洁",
            "尽量简洁",
            "简洁一点",
            "简短一点",
            "回答简洁",
            "回复简洁",
            "keep it concise",
            "be concise",
            "brief answers",
        ],
    ) {
        entries.push(LongTermMemoryRecord {
            kind: "user_preference.response_style".to_string(),
            content: "Keep answers concise.".to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if contains_any_phrase(
        &lowered,
        &[
            "请使用绝对路径",
            "请用绝对路径",
            "使用绝对路径",
            "用绝对路径",
            "请给我绝对路径",
            "引用文件时请使用绝对路径",
            "不要使用相对路径",
            "不要用相对路径",
            "use absolute paths",
            "use absolute file paths",
            "prefer absolute paths",
            "reference files with absolute paths",
        ],
    ) {
        entries.push(LongTermMemoryRecord {
            kind: "user_preference.file_reference_style".to_string(),
            content: "Use absolute paths when referencing workspace files.".to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if contains_any_phrase(
        &lowered,
        &[
            "更新任务文档",
            "同步任务文档",
            "回写任务文档",
            "更新任务系统",
            "同步任务系统",
            "回写任务卡",
            "记得更新任务文档",
            "keep the task documents updated",
            "update the task documents",
            "sync the task system",
            "write back the task card",
        ],
    ) {
        entries.push(LongTermMemoryRecord {
            kind: "user_preference.task_system_sync".to_string(),
            content: "Keep task-system documents updated while progressing work.".to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if contains_any_phrase(
        &lowered,
        &[
            "不要修改无关文件",
            "不要动无关文件",
            "不要碰无关文件",
            "不要回滚无关改动",
            "不要碰无关改动",
            "不要动无关改动",
            "不要改无关文件",
            "don't modify unrelated files",
            "do not modify unrelated files",
            "don't touch unrelated changes",
            "do not touch unrelated changes",
            "don't revert unrelated changes",
            "do not revert unrelated changes",
        ],
    ) {
        entries.push(LongTermMemoryRecord {
            kind: "user_preference.change_scope".to_string(),
            content: "Avoid modifying unrelated existing changes.".to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if let Some(note) = extract_explicit_memory_note(user_message) {
        entries.push(LongTermMemoryRecord {
            kind: "user_memory.explicit_note".to_string(),
            content: note,
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if let Some(task_id) = extract_explicit_current_task_focus(user_message, &lowered) {
        entries.push(LongTermMemoryRecord {
            kind: "project_focus.active_task".to_string(),
            content: format!("Current active task is {task_id}."),
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if extract_explicit_acceptance_gate_requirement(&lowered) {
        entries.push(LongTermMemoryRecord {
            kind: "project_workflow.acceptance_gate".to_string(),
            content:
                "Establish acceptance criteria and run a closeout audit before claiming delivery."
                    .to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if let Some(prerequisite) =
        extract_explicit_project_dependency_prerequisite(user_message, &lowered)
    {
        entries.push(LongTermMemoryRecord {
            kind: "project_dependency.prerequisite".to_string(),
            content: prerequisite,
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if extract_explicit_closeout_requirement(&lowered) {
        entries.push(LongTermMemoryRecord {
            kind: "project_workflow.closeout_requirement".to_string(),
            content:
                "Summarize changed files, verification performed, and unresolved risks at closeout."
                    .to_string(),
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    if let Some(task_boundary) = extract_explicit_task_boundary(&lowered) {
        entries.push(LongTermMemoryRecord {
            kind: "project_scope.task_boundary".to_string(),
            content: task_boundary,
            source: "explicit_user_message".to_string(),
            updated_at_ms,
        });
    }

    entries
}

fn memory_record_identity(entry: &LongTermMemoryRecord) -> String {
    if entry.kind.starts_with("user_preference.")
        || entry.kind.starts_with("project_dependency.")
        || entry.kind == "project_focus.active_task"
        || entry.kind == "project_workflow.acceptance_gate"
        || entry.kind == "project_workflow.closeout_requirement"
        || entry.kind == "project_scope.task_boundary"
    {
        entry.kind.clone()
    } else {
        format!("{}::{}", entry.kind, entry.content)
    }
}

fn extract_explicit_memory_note(user_message: &str) -> Option<String> {
    let trimmed = user_message.trim();
    if trimmed.is_empty() {
        return None;
    }

    let chinese_markers = ["请记住", "帮我记住", "记住：", "记住:", "记住一下", "记住"];
    for marker in chinese_markers {
        if let Some(index) = trimmed.find(marker) {
            let note = trimmed[index + marker.len()..]
                .trim()
                .trim_start_matches(['，', ',', '：', ':', ' '])
                .trim();
            if !note.is_empty() {
                return Some(note.to_string());
            }
        }
    }

    let lowered = trimmed.to_lowercase();
    let english_markers = [
        "please remember that",
        "please remember",
        "remember that",
        "remember:",
        "remember this",
    ];
    for marker in english_markers {
        if let Some(index) = lowered.find(marker) {
            let note = trimmed[index + marker.len()..]
                .trim()
                .trim_start_matches([',', ':', ' '])
                .trim();
            if !note.is_empty() {
                return Some(note.to_string());
            }
        }
    }

    None
}

fn extract_explicit_current_task_focus(
    user_message: &str,
    lowered_user_message: &str,
) -> Option<String> {
    if !contains_any_phrase(
        lowered_user_message,
        &[
            "现在开始",
            "当前优先推进",
            "优先推进",
            "先做",
            "先推进",
            "本轮先做",
            "先处理",
            "当前主线",
            "focus on",
            "start with",
            "start on",
            "prioritize",
            "current priority",
            "work on",
        ],
    ) {
        return None;
    }

    first_task_like_token(user_message)
}

fn extract_explicit_acceptance_gate_requirement(lowered_user_message: &str) -> bool {
    if contains_any_phrase(
        lowered_user_message,
        &[
            "建立验收标准",
            "先建立验收标准",
            "补齐验收标准",
            "验收审计",
            "正式验收",
            "closeout audit",
            "acceptance criteria",
            "acceptance audit",
        ],
    ) {
        return true;
    }

    contains_any_phrase(lowered_user_message, &["验收标准"])
        && contains_any_phrase(
            lowered_user_message,
            &[
                "完成交付",
                "成功完成交付",
                "交付",
                "delivery",
                "claiming delivery",
                "mark the work complete",
            ],
        )
}

fn extract_explicit_project_dependency_prerequisite(
    user_message: &str,
    lowered_user_message: &str,
) -> Option<String> {
    let task_ids = collect_task_like_tokens(user_message);
    if task_ids.len() < 2 {
        return None;
    }

    if contains_any_phrase(
        lowered_user_message,
        &["依赖", "depends on", "blocked on", "prerequisite"],
    ) {
        return Some(format!("{} depends on {}.", task_ids[0], task_ids[1]));
    }

    if contains_any_phrase(
        lowered_user_message,
        &["先完成", "完成前", "before starting", "before working on"],
    ) && contains_any_phrase(
        lowered_user_message,
        &[
            "再做",
            "再推进",
            "再处理",
            "再开始",
            "then start",
            "then work on",
        ],
    ) {
        return Some(format!("{} depends on {}.", task_ids[1], task_ids[0]));
    }

    None
}

fn extract_explicit_closeout_requirement(lowered_user_message: &str) -> bool {
    contains_any_phrase(
        lowered_user_message,
        &[
            "改了哪些文件",
            "做了什么验证",
            "未解决风险",
            "changed files",
            "verification performed",
            "unresolved risks",
        ],
    ) && contains_any_phrase(
        lowered_user_message,
        &["验证", "verification", "风险", "risks"],
    )
}

fn extract_explicit_task_boundary(lowered_user_message: &str) -> Option<String> {
    let markers = [
        "不要越界到",
        "不能越界到",
        "不要扩到",
        "do not expand into",
        "don't expand into",
    ];

    for marker in markers {
        if let Some(index) = lowered_user_message.find(marker) {
            let boundary_text = &lowered_user_message[index + marker.len()..];
            let task_ids = collect_task_like_tokens(boundary_text);
            if !task_ids.is_empty() {
                return Some(format!("Do not expand scope into {}.", task_ids.join(", ")));
            }
        }
    }

    None
}

fn collect_task_like_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for ch in text.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            current.push(ch);
            continue;
        }

        let normalized = current.to_ascii_uppercase();
        if looks_like_task_id(&normalized) && !tokens.iter().any(|existing| existing == &normalized)
        {
            tokens.push(normalized);
        }
        current.clear();
    }

    tokens
}

fn first_task_like_token(text: &str) -> Option<String> {
    let mut current = String::new();
    for ch in text.chars().chain(std::iter::once(' ')) {
        if ch.is_ascii_alphanumeric() || ch == '-' {
            current.push(ch);
            continue;
        }

        let normalized = current.to_ascii_uppercase();
        if looks_like_task_id(&normalized) {
            return Some(normalized);
        }
        current.clear();
    }

    None
}

fn looks_like_task_id(token: &str) -> bool {
    let mut parts = token.split('-');
    let Some(prefix) = parts.next() else {
        return false;
    };
    let Some(number) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }

    (2..=6).contains(&prefix.len())
        && prefix.chars().all(|ch| ch.is_ascii_uppercase())
        && (1..=6).contains(&number.len())
        && number.chars().all(|ch| ch.is_ascii_digit())
}

fn contains_any_phrase(text: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|phrase| text.contains(phrase))
}

fn sanitize_provider_native_transcript(session: &mut SessionState) -> bool {
    if !has_legacy_reasoning_gap(&session.provider_native_transcript)
        && !has_incomplete_tool_roundtrip(&session.provider_native_transcript)
    {
        return false;
    }

    session.provider_native_transcript.clear();
    true
}

fn has_legacy_reasoning_gap(transcript: &[Value]) -> bool {
    let mut awaiting_tool_turn_reasoning = false;

    for message in transcript {
        match message.get("role").and_then(Value::as_str) {
            Some("user") => {
                awaiting_tool_turn_reasoning = false;
            }
            Some("tool") => {
                awaiting_tool_turn_reasoning = true;
            }
            Some("assistant") => {
                let has_tool_calls = message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .map(|calls| !calls.is_empty())
                    .unwrap_or(false);
                let missing_reasoning = message
                    .get("reasoning_content")
                    .map(reasoning_content_missing)
                    .unwrap_or(true);

                if (has_tool_calls || awaiting_tool_turn_reasoning) && missing_reasoning {
                    return true;
                }

                if has_tool_calls {
                    awaiting_tool_turn_reasoning = true;
                }
            }
            _ => {}
        }
    }

    false
}

fn reasoning_content_missing(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::String(text) => text.trim().is_empty(),
        Value::Array(items) => items.is_empty(),
        Value::Object(map) => map.is_empty(),
        _ => false,
    }
}

fn has_incomplete_tool_roundtrip(transcript: &[Value]) -> bool {
    let mut pending_tool_call_ids: Vec<String> = Vec::new();

    for message in transcript {
        match message.get("role").and_then(Value::as_str) {
            Some("assistant") => {
                let tool_call_ids = message
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .map(|calls| {
                        calls
                            .iter()
                            .filter_map(|call| call.get("id").and_then(Value::as_str))
                            .map(str::to_string)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                if !tool_call_ids.is_empty() {
                    pending_tool_call_ids = tool_call_ids;
                }
            }
            Some("tool") => {
                let Some(tool_call_id) = message.get("tool_call_id").and_then(Value::as_str) else {
                    return true;
                };

                if pending_tool_call_ids.is_empty() {
                    return true;
                }

                pending_tool_call_ids.retain(|id| id != tool_call_id);
            }
            Some("user") => {
                if !pending_tool_call_ids.is_empty() {
                    return true;
                }
            }
            _ => {}
        }
    }

    !pending_tool_call_ids.is_empty()
}

fn session_is_persistable(session: &SessionState) -> bool {
    !session.history.is_empty()
        || !session.turn_trace_history.is_empty()
        || !session.long_term_memory_entries.is_empty()
        || !session.memory_write_evidence.is_empty()
        || !session.memory_write_hook_trace_records.is_empty()
        || !session.history_state_evidence.is_empty()
        // A session that has been truncated to the initial (empty) state
        // still has a non-empty history graph (the legacy root node) and
        // must be persisted so the truncation survives a restart.
        || !session.history_nodes.is_empty()
}

fn build_title(history: &[TurnHistoryMessage]) -> String {
    history
        .iter()
        .find(|message| message.role == "user")
        .and_then(|message| normalize_title_candidate(&message.content))
        .unwrap_or_else(|| DEFAULT_SESSION_TITLE.to_string())
}

fn build_summary(_turn_count: usize, last_referenced_file: Option<&str>) -> String {
    match last_referenced_file {
        Some(path) => format!("{} / 当前关注 {}", DEFAULT_SESSION_SUMMARY, path),
        None => DEFAULT_SESSION_SUMMARY.to_string(),
    }
}

fn normalize_title_candidate(text: &str) -> Option<String> {
    let normalized = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))?;

    if normalized.is_empty() {
        return None;
    }

    let title = truncate_chars(&normalized, TITLE_MAX_CHARS);
    if title.is_empty() {
        None
    } else {
        Some(title)
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut truncated = String::new();
    let mut count = 0;
    for ch in text.chars() {
        if count >= max_chars {
            truncated.push_str("...");
            return truncated;
        }
        truncated.push(ch);
        count += 1;
    }
    truncated
}

fn default_session_title() -> String {
    DEFAULT_SESSION_TITLE.to_string()
}

fn extract_explicit_file_name(text: &str) -> Option<String> {
    let mut candidates = Vec::new();
    let mut current = String::new();

    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-' | '/' | '\\') {
            current.push(ch);
        } else if !current.is_empty() {
            candidates.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        candidates.push(current);
    }

    candidates
        .into_iter()
        .map(|segment| {
            segment
                .trim_matches(|ch: char| ch == '`' || ch == '.' || ch == '!')
                .to_string()
        })
        .find(|segment| {
            !segment.is_empty()
                && segment.contains('.')
                && !segment.starts_with("http://")
                && !segment.starts_with("https://")
                && segment
                    .rsplit('.')
                    .next()
                    .map(|ext| !ext.is_empty() && ext.chars().all(|ch| ch.is_ascii_alphanumeric()))
                    .unwrap_or(false)
        })
}

fn default_storage_path() -> PathBuf {
    #[cfg(test)]
    {
        // 测试必须隔离：绝不读写用户生产数据（%LOCALAPPDATA%/PonyAgent/）。
        unique_test_session_dir("pony-agent-storage").join("sessions.json")
    }

    #[cfg(not(test))]
    {
        dirs::data_local_dir()
            .or_else(dirs::home_dir)
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_else(|| PathBuf::from("."))
            .join("PonyAgent")
            .join("sessions.json")
    }
}

fn default_attachment_root() -> PathBuf {
    default_storage_path()
        .parent()
        .map(|parent| parent.join("attachments"))
        .unwrap_or_else(|| PathBuf::from("attachments"))
}

#[cfg(test)]
fn unique_test_session_dir(prefix: &str) -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    std::env::temp_dir()
        .join(format!("{prefix}-{stamp}"))
        .join("sessions")
}

fn default_sessions() -> SessionMap {
    let mut sessions = HashMap::new();
    sessions.insert(
        DEFAULT_SESSION_ID.to_string(),
        SessionState {
            conversation_id: DEFAULT_SESSION_ID.to_string(),
            title: DEFAULT_SESSION_TITLE.to_string(),
            summary: DEFAULT_SESSION_SUMMARY.to_string(),
            history: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: Vec::new(),
            trace_migration_state: TraceMigrationState::default(),
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 0,
            last_referenced_file: None,
            updated_at_ms: now_timestamp_ms(),
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: HistoryCursor::default(),
            workspace_id: None,
            event_watermark: 0,
            last_commit_watermark: 0,
        },
    );
    sessions
}

fn now_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn load_store_from_path(path: &Path) -> Option<PersistedStore> {
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

fn sanitize_attachment_references(session: &mut SessionState) -> bool {
    let mut changed = false;
    for message in &mut session.history {
        for attachment in &mut message.attachments {
            let normalized_relative_path = attachment.relative_path.replace('\\', "/");
            if normalized_relative_path != attachment.relative_path {
                attachment.relative_path = normalized_relative_path;
                changed = true;
            }
        }
    }
    changed
}

fn backfill_attachment_reference_assets(session: &mut SessionState) -> bool {
    let mut changed = false;
    for message in &mut session.history {
        for attachment in &mut message.attachments {
            if attachment.asset_id.trim().is_empty() {
                attachment.asset_id = attachment_asset_id(&attachment.relative_path);
                changed = true;
            }
        }
    }
    changed
}

fn rebuild_attachment_assets(
    sessions: &SessionMap,
    existing_assets: &AttachmentAssetMap,
    attachment_root: &Path,
) -> AttachmentAssetMap {
    let mut assets = scan_attachment_assets(attachment_root, existing_assets);
    merge_session_attachment_assets(sessions, &mut assets);
    assets
}

fn rebuild_attachment_assets_from_sessions(
    sessions: &SessionMap,
    existing_assets: &AttachmentAssetMap,
) -> AttachmentAssetMap {
    let mut assets = existing_assets.clone();
    merge_session_attachment_assets(sessions, &mut assets);
    assets
}

/// 判定消息附件是否应物化为 `AttachmentAsset`。
///
/// 图片资产的 `relative_path` 为 `<session_id>/att-...` 且 `asset_id` 已注册；doc/text
/// 引用（PA-078）仅携带 `relative_path`（`.tmp/imports/...`）、无 `asset_id`，不应进入
/// 附件目录——AC#4 收窄：只对图片保持 AttachmentAsset 生命周期合同。显式 `asset_id`
/// （引用已注册资产）始终保留。
fn is_asset_eligible_attachment(session_id: &str, attachment: &SessionAttachment) -> bool {
    if !attachment.asset_id.trim().is_empty() {
        return true;
    }
    let relative = attachment.relative_path.replace('\\', "/");
    relative.starts_with(&format!("{session_id}/"))
}

fn merge_session_attachment_assets(sessions: &SessionMap, assets: &mut AttachmentAssetMap) {
    for session in sessions.values() {
        for attachment in session
            .history
            .iter()
            .flat_map(|message| message.attachments.iter())
            .filter(|attachment| is_asset_eligible_attachment(&session.conversation_id, attachment))
        {
            let asset_id = if attachment.asset_id.trim().is_empty() {
                attachment_asset_id(&attachment.relative_path)
            } else {
                attachment.asset_id.clone()
            };
            let asset = assets
                .entry(asset_id.clone())
                .or_insert_with(|| AttachmentAsset {
                    id: asset_id.clone(),
                    session_id: session.conversation_id.clone(),
                    name: attachment.name.clone(),
                    mime_type: attachment.mime_type.clone(),
                    relative_path: attachment.relative_path.clone(),
                    size_bytes: attachment.size_bytes,
                    created_at_ms: attachment.created_at_ms,
                    status: AttachmentLifecycleStatus::Active,
                    reference_count: 0,
                    last_referenced_at_ms: None,
                    expires_at_ms: None,
                });
            asset.id = asset_id;
            asset.session_id = session.conversation_id.clone();
            asset.name = attachment.name.clone().or(asset.name.clone());
            if !attachment.mime_type.trim().is_empty() {
                asset.mime_type = attachment.mime_type.clone();
            }
            asset.relative_path = attachment.relative_path.replace('\\', "/");
            if attachment.size_bytes > 0 {
                asset.size_bytes = attachment.size_bytes;
            }
            if attachment.created_at_ms > 0 {
                asset.created_at_ms = attachment.created_at_ms;
            }
        }
    }
}

fn rebuild_session_attachment_index(sessions: &SessionMap) -> SessionAttachmentIndex {
    let mut index = SessionAttachmentIndex::new();
    for session in sessions.values() {
        let mut entry = Vec::new();
        for attachment in session
            .history
            .iter()
            .flat_map(|message| message.attachments.iter())
        {
            let asset_id = if attachment.asset_id.trim().is_empty() {
                attachment_asset_id(&attachment.relative_path)
            } else {
                attachment.asset_id.clone()
            };
            if !entry.iter().any(|existing| existing == &asset_id) {
                entry.push(asset_id);
            }
        }
        if !entry.is_empty() {
            index.insert(session.conversation_id.clone(), entry);
        }
    }
    index
}

fn attachment_assets_for_query(
    sessions: &SessionMap,
    attachment_assets: &AttachmentAssetMap,
    _session_attachment_index: &SessionAttachmentIndex,
    attachment_root: &Path,
    query: &AttachmentAssetQuery,
    now_ms: u64,
) -> Vec<AttachmentAsset> {
    let reference_stats = attachment_reference_stats(sessions);
    let requested_mime = query
        .mime_type
        .as_ref()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let requested_name = query
        .name_contains
        .as_ref()
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());
    let requested_statuses = query
        .statuses
        .iter()
        .cloned()
        .collect::<HashSet<AttachmentLifecycleStatus>>();

    let mut assets = attachment_assets
        .values()
        .filter(|asset| {
            query
                .session_id
                .as_ref()
                .map_or(true, |session_id| asset.session_id == *session_id)
        })
        .map(|asset| {
            decorate_attachment_asset(
                asset,
                attachment_root,
                reference_stats.get(&asset.id),
                now_ms,
            )
        })
        .filter(|asset| {
            requested_mime.as_ref().map_or(true, |mime| {
                asset.mime_type.to_ascii_lowercase().contains(mime)
            })
        })
        .filter(|asset| {
            requested_name.as_ref().map_or(true, |name| {
                asset
                    .name
                    .as_deref()
                    .map(|value| value.to_ascii_lowercase().contains(name))
                    .unwrap_or(false)
                    || asset.relative_path.to_ascii_lowercase().contains(name)
            })
        })
        .filter(|asset| {
            query
                .created_after_ms
                .map_or(true, |after_ms| asset.created_at_ms >= after_ms)
        })
        .filter(|asset| {
            query
                .created_before_ms
                .map_or(true, |before_ms| asset.created_at_ms <= before_ms)
        })
        .filter(|asset| requested_statuses.is_empty() || requested_statuses.contains(&asset.status))
        .collect::<Vec<_>>();
    assets.sort_by(|left, right| {
        right
            .created_at_ms
            .cmp(&left.created_at_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    assets
}

#[derive(Clone, Debug, Default)]
struct AttachmentReferenceStat {
    reference_count: usize,
    last_referenced_at_ms: Option<u64>,
}

fn attachment_reference_stats(sessions: &SessionMap) -> HashMap<String, AttachmentReferenceStat> {
    let mut stats = HashMap::<String, AttachmentReferenceStat>::new();
    for session in sessions.values() {
        for attachment in session
            .history
            .iter()
            .flat_map(|message| message.attachments.iter())
        {
            let asset_id = if attachment.asset_id.trim().is_empty() {
                attachment_asset_id(&attachment.relative_path)
            } else {
                attachment.asset_id.clone()
            };
            let entry = stats.entry(asset_id).or_default();
            entry.reference_count += 1;
            entry.last_referenced_at_ms = Some(
                entry
                    .last_referenced_at_ms
                    .unwrap_or(0)
                    .max(attachment.created_at_ms.max(session.updated_at_ms)),
            );
        }
    }
    stats
}

fn decorate_attachment_asset(
    asset: &AttachmentAsset,
    attachment_root: &Path,
    reference_stat: Option<&AttachmentReferenceStat>,
    now_ms: u64,
) -> AttachmentAsset {
    let mut asset = asset.clone();
    let reference_count = reference_stat.map(|stat| stat.reference_count).unwrap_or(0);
    let payload_exists = attachment_root.join(&asset.relative_path).is_file();
    let expires_at_ms = if reference_count == 0 && asset.created_at_ms > 0 {
        Some(
            asset
                .created_at_ms
                .saturating_add(DEFAULT_ATTACHMENT_RECLAIM_TTL_MS),
        )
    } else {
        None
    };

    asset.reference_count = reference_count;
    asset.last_referenced_at_ms = reference_stat.and_then(|stat| stat.last_referenced_at_ms);
    asset.expires_at_ms = expires_at_ms;
    asset.status = if reference_count > 0 {
        if payload_exists {
            AttachmentLifecycleStatus::Active
        } else {
            AttachmentLifecycleStatus::MissingPayload
        }
    } else if expires_at_ms.map_or(false, |deadline| now_ms >= deadline) {
        AttachmentLifecycleStatus::Expired
    } else {
        AttachmentLifecycleStatus::Reclaimable
    };
    asset
}

fn scan_attachment_assets(
    attachment_root: &Path,
    existing_assets: &AttachmentAssetMap,
) -> AttachmentAssetMap {
    let mut assets = AttachmentAssetMap::new();
    let Ok(session_dirs) = fs::read_dir(attachment_root) else {
        return assets;
    };

    for session_dir in session_dirs.flatten() {
        let session_path = session_dir.path();
        if !session_path.is_dir() {
            continue;
        }
        let Some(session_id) = session_path.file_name().and_then(|value| value.to_str()) else {
            continue;
        };
        let Ok(entries) = fs::read_dir(&session_path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(file_name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let relative_path = format!("{session_id}/{file_name}");
            let asset_id = attachment_asset_id(&relative_path);
            let mut asset = existing_assets.get(&asset_id).cloned().unwrap_or_else(|| {
                let metadata = fs::metadata(&path).ok();
                AttachmentAsset {
                    id: asset_id.clone(),
                    session_id: session_id.to_string(),
                    name: Some(infer_attachment_name(file_name)),
                    mime_type: infer_attachment_mime_type(&path),
                    relative_path: relative_path.clone(),
                    size_bytes: metadata.as_ref().map_or(0, |value| value.len()),
                    created_at_ms: metadata
                        .and_then(|value| value.modified().ok())
                        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
                        .map(|value| value.as_millis() as u64)
                        .unwrap_or(0),
                    status: AttachmentLifecycleStatus::Reclaimable,
                    reference_count: 0,
                    last_referenced_at_ms: None,
                    expires_at_ms: None,
                }
            });
            asset.session_id = session_id.to_string();
            asset.relative_path = relative_path;
            assets.insert(asset_id, asset);
        }
    }

    assets
}

fn delete_session_attachment_dir(attachment_root: &Path, session_id: &str) {
    let _ = fs::remove_dir_all(attachment_root.join(session_id));
}

#[allow(dead_code)]
fn delete_attachment_file(attachment_root: &Path, attachment: &SessionAttachment) {
    let path = attachment_file_path(attachment_root, attachment);
    let _ = fs::remove_file(&path);
    if let Some(parent) = path.parent() {
        let _ = fs::remove_dir(parent);
    }
}

fn attachment_file_path(attachment_root: &Path, attachment: &SessionAttachment) -> PathBuf {
    attachment_root.join(&attachment.relative_path)
}

fn attachment_asset_id(relative_path: &str) -> String {
    format!("asset:{}", relative_path.replace('\\', "/"))
}

fn infer_attachment_name(file_name: &str) -> String {
    file_name
        .strip_suffix(".dataurl")
        .unwrap_or(file_name)
        .to_string()
}

fn infer_attachment_mime_type(path: &Path) -> String {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| {
            content
                .strip_prefix("data:")
                .and_then(|value| value.split(';').next())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .unwrap_or_else(|| "application/octet-stream".to_string())
}

fn load_attachment_image(
    attachment_root: &Path,
    attachment_assets: &AttachmentAssetMap,
    attachment: &SessionAttachment,
) -> Option<TurnInputImage> {
    let path = attachment_assets
        .get(&attachment.asset_id)
        .map(|asset| attachment_root.join(&asset.relative_path))
        .unwrap_or_else(|| attachment_file_path(attachment_root, attachment));
    let data_url = fs::read_to_string(path).ok()?;
    Some(TurnInputImage {
        data_url,
        mime_type: attachment.mime_type.clone(),
        name: attachment.name.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::turn_event::{TurnEndReason, TurnEvent};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct StaticMemoryWriteHookExecutor {
        results: Vec<crate::agent::hooks::HookExecutionResult>,
    }

    impl MemoryWriteHookExecutor for StaticMemoryWriteHookExecutor {
        fn execute(
            &self,
            _envelope: &MemoryWriteHookEnvelope,
        ) -> Result<Vec<crate::agent::hooks::HookExecutionResult>, String> {
            Ok(self.results.clone())
        }
    }

    struct StaticHistoryStateHookExecutor {
        start_results: Vec<crate::agent::hooks::HookExecutionResult>,
        resolved_results: Vec<crate::agent::hooks::HookExecutionResult>,
    }

    impl HistoryStateHookExecutor for StaticHistoryStateHookExecutor {
        fn execute(
            &self,
            envelope: &HistoryStateHookEnvelope,
        ) -> Result<Vec<crate::agent::hooks::HookExecutionResult>, String> {
            Ok(match envelope.hook_point {
                HistoryStateHookPoint::HistoryCheckoutStart
                | HistoryStateHookPoint::BranchRestoreStart
                | HistoryStateHookPoint::BranchForkStart
                | HistoryStateHookPoint::BranchSwitchStart => self.start_results.clone(),
                HistoryStateHookPoint::HistoryCheckoutResolved
                | HistoryStateHookPoint::BranchRestoreResolved
                | HistoryStateHookPoint::BranchForkResolved
                | HistoryStateHookPoint::BranchSwitchResolved => self.resolved_results.clone(),
            })
        }
    }

    #[test]
    fn memory_backend_keeps_turns_in_process() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("test"),
            "查看 tauri.conf.json",
            "已读取",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("test"), &[]);
        assert_eq!(snapshot.title, "查看 tauri.conf.json");
        assert_eq!(snapshot.turn_count, 1);
        assert_eq!(snapshot.history.len(), 2);
        assert_eq!(
            snapshot.last_referenced_file.as_deref(),
            Some("tauri.conf.json")
        );
    }

    #[test]
    fn serde_roundtrip_enriched_metadata() {
        let msg = TurnHistoryMessage {
            role: "assistant".to_string(),
            content: "test".to_string(),
            attachments: Vec::new(),
            turn_id: Some("turn-1".to_string()),
            status: Some(MessageStatus::Done),
            model_name: Some("gpt-5".to_string()),
            token_count: Some(42),
            reasoning_content: Some("thinking...".to_string()),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        let deserialized: TurnHistoryMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(deserialized.turn_id, Some("turn-1".to_string()));
        assert_eq!(deserialized.status, Some(MessageStatus::Done));
        assert_eq!(deserialized.model_name, Some("gpt-5".to_string()));
        assert_eq!(deserialized.token_count, Some(42));
        assert_eq!(
            deserialized.reasoning_content,
            Some("thinking...".to_string())
        );
        assert_eq!(deserialized.stable_id(), "turn-1-assistant");
    }

    #[test]
    fn serde_roundtrip_old_blob_compatible() {
        let old_json = r#"{"role":"user","content":"hello","attachments":[]}"#;
        let msg: TurnHistoryMessage = serde_json::from_str(old_json).expect("old blob deserialize");
        assert_eq!(msg.role, "user");
        assert_eq!(msg.content, "hello");
        assert!(msg.turn_id.is_none());
        assert!(msg.status.is_none());
        assert!(msg.model_name.is_none());
        assert!(msg.token_count.is_none());
        assert!(msg.reasoning_content.is_none());
        assert_eq!(msg.stable_id(), "unknown-user");

        let re_json = serde_json::to_string(&msg).expect("serialize");
        assert!(!re_json.contains("turnId"));
        assert!(!re_json.contains("status"));
        assert!(!re_json.contains("modelName"));
        assert!(!re_json.contains("tokenCount"));
        assert!(!re_json.contains("reasoningContent"));
    }

    #[test]
    fn snapshot_enriched_history_metadata() {
        let mut store = SessionStore::memory_only();
        let session_id = "enriched-test";
        store.sessions.insert(
            session_id.to_string(),
            SessionState {
                conversation_id: session_id.to_string(),
                title: DEFAULT_SESSION_TITLE.to_string(),
                summary: DEFAULT_SESSION_SUMMARY.to_string(),
                history: vec![
                    TurnHistoryMessage {
                        role: "user".to_string(),
                        content: "你好".to_string(),
                        attachments: Vec::new(),
                        ..Default::default()
                    },
                    TurnHistoryMessage {
                        role: "assistant".to_string(),
                        content: "收到。".to_string(),
                        attachments: Vec::new(),
                        ..Default::default()
                    },
                ],
                provider_native_transcript: Vec::new(),
                turn_trace_history: vec![TurnTraceRecord {
                    turn_id: "turn-1".to_string(),
                    phase: "completed".to_string(),
                    title: "test turn".to_string(),
                    provider_model: Some("gpt-5".to_string()),
                    output_tokens: Some(42),
                    ..Default::default()
                }],
                trace_migration_state: TraceMigrationState::default(),
                turn_trace_refs: None,
                long_term_memory_entries: Vec::new(),
                memory_write_evidence: Vec::new(),
                memory_write_hook_trace_records: Vec::new(),
                history_state_evidence: Vec::new(),
                turn_count: 1,
                last_referenced_file: None,
                updated_at_ms: now_timestamp_ms(),
                history_nodes: Vec::new(),
                history_branches: Vec::new(),
                history_cursor: HistoryCursor::default(),
                workspace_id: None,
                event_watermark: 0,
                last_commit_watermark: 0,
            },
        );
        let snapshot = store.snapshot(Some(session_id), &[]);
        assert_eq!(snapshot.history.len(), 2);
        assert_eq!(snapshot.history[0].role, "user");
        assert_eq!(snapshot.history[0].turn_id.as_deref(), Some("turn-1"));
        assert_eq!(snapshot.history[1].role, "assistant");
        assert_eq!(snapshot.history[1].turn_id.as_deref(), Some("turn-1"));
        assert_eq!(snapshot.history[1].model_name.as_deref(), Some("gpt-5"));
        assert_eq!(snapshot.history[1].token_count, Some(42));
        assert_eq!(snapshot.history[1].status, Some(MessageStatus::Done));
    }

    #[test]
    fn snapshot_mixed_metadata_preserves_existing() {
        // 空 trace：幂等性——已有元数据不被清除
        let msg = TurnHistoryMessage {
            role: "assistant".to_string(),
            content: "已有元数据".to_string(),
            attachments: Vec::new(),
            turn_id: Some("existing-turn".to_string()),
            status: Some(MessageStatus::Done),
            model_name: Some("existing-model".to_string()),
            token_count: Some(99),
            reasoning_content: Some("existing".to_string()),
        };
        let empty_trace: Vec<TurnTraceRecord> = Vec::new();
        let mut history = vec![msg];
        enrich_history_from_traces(&mut history, &empty_trace);
        assert_eq!(history[0].turn_id, Some("existing-turn".to_string()));
        assert_eq!(history[0].model_name, Some("existing-model".to_string()));
        assert_eq!(history[0].token_count, Some(99));

        // 有 trace：已有元数据不被覆写
        let msg2 = TurnHistoryMessage {
            role: "assistant".to_string(),
            content: "不变".to_string(),
            attachments: Vec::new(),
            turn_id: Some("preserved-turn".to_string()),
            ..Default::default()
        };
        let traces = vec![TurnTraceRecord {
            turn_id: "trace-turn".to_string(),
            phase: "completed".to_string(),
            title: "trace".to_string(),
            provider_model: Some("trace-model".to_string()),
            output_tokens: Some(1),
            ..Default::default()
        }];
        let mut history2 = vec![
            TurnHistoryMessage {
                role: "user".to_string(),
                content: "hi".to_string(),
                attachments: Vec::new(),
                ..Default::default()
            },
            msg2,
        ];
        enrich_history_from_traces(&mut history2, &traces);
        assert_eq!(history2[0].turn_id, Some("trace-turn".to_string()));
        assert_eq!(history2[1].turn_id, Some("preserved-turn".to_string()));
        assert!(history2[1].model_name.is_none());
    }

    #[test]
    fn checkout_history_node_metadata() {
        let mut store = SessionStore::memory_only();
        let session_id = "checkout-meta";
        let node_id = "node-checkout-1".to_string();
        store.sessions.insert(
            session_id.to_string(),
            SessionState {
                conversation_id: session_id.to_string(),
                title: DEFAULT_SESSION_TITLE.to_string(),
                summary: DEFAULT_SESSION_SUMMARY.to_string(),
                history: vec![
                    TurnHistoryMessage {
                        role: "user".to_string(),
                        content: "第一轮".to_string(),
                        attachments: Vec::new(),
                        ..Default::default()
                    },
                    TurnHistoryMessage {
                        role: "assistant".to_string(),
                        content: "收到。".to_string(),
                        attachments: Vec::new(),
                        ..Default::default()
                    },
                ],
                provider_native_transcript: Vec::new(),
                turn_trace_history: vec![TurnTraceRecord {
                    turn_id: "turn-1".to_string(),
                    phase: "completed".to_string(),
                    title: "first turn".to_string(),
                    provider_model: Some("gpt-5".to_string()),
                    output_tokens: Some(42),
                    ..Default::default()
                }],
                trace_migration_state: TraceMigrationState::default(),
                turn_trace_refs: None,
                long_term_memory_entries: Vec::new(),
                memory_write_evidence: Vec::new(),
                memory_write_hook_trace_records: Vec::new(),
                history_state_evidence: Vec::new(),
                turn_count: 1,
                last_referenced_file: None,
                updated_at_ms: now_timestamp_ms(),
                history_nodes: vec![HistoryNode {
                    node_id: node_id.clone(),
                    session_id: session_id.to_string(),
                    branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                    title: DEFAULT_SESSION_TITLE.to_string(),
                    summary: DEFAULT_SESSION_SUMMARY.to_string(),
                    history: vec![
                        TurnHistoryMessage {
                            role: "user".to_string(),
                            content: "第一轮".to_string(),
                            attachments: Vec::new(),
                            ..Default::default()
                        },
                        TurnHistoryMessage {
                            role: "assistant".to_string(),
                            content: "收到。".to_string(),
                            attachments: Vec::new(),
                            ..Default::default()
                        },
                    ],
                    turn_trace_history: vec![TurnTraceRecord {
                        turn_id: "turn-1".to_string(),
                        phase: "completed".to_string(),
                        title: "first turn".to_string(),
                        provider_model: Some("gpt-5".to_string()),
                        output_tokens: Some(42),
                        ..Default::default()
                    }],
                    ..Default::default()
                }],
                history_branches: vec![HistoryBranch {
                    branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                    head_node_id: Some(node_id.clone()),
                    ..Default::default()
                }],
                history_cursor: HistoryCursor {
                    session_id: session_id.to_string(),
                    visible_node_id: Some(node_id.clone()),
                    active_branch_id: Some(DEFAULT_HISTORY_BRANCH_ID.to_string()),
                    branch_head_node_id: Some(node_id.clone()),
                    workspace_node_id: Some(node_id.clone()),
                    mode: HistoryCursorMode::Live,
                    ..Default::default()
                },
                workspace_id: None,
                event_watermark: 0,
                last_commit_watermark: 0,
            },
        );
        let snapshot = store
            .checkout_history_node(
                Some(session_id),
                &node_id,
                HistoryCheckoutMode::TranscriptOnly,
                None,
            )
            .expect("checkout should succeed");

        assert!(snapshot.history.len() >= 2);
        assert_eq!(
            snapshot.history[0].turn_id.as_deref(),
            Some("turn-1"),
            "user message should have turn_id after enrichment"
        );
        assert_eq!(
            snapshot.history[1].turn_id.as_deref(),
            Some("turn-1"),
            "assistant message should have turn_id after enrichment"
        );
        assert_eq!(snapshot.history[1].model_name.as_deref(), Some("gpt-5"));
        assert_eq!(snapshot.history[1].token_count, Some(42));
        assert_eq!(snapshot.history[1].status, Some(MessageStatus::Done));
    }

    #[test]
    fn ensure_history_graph_inserts_initial_root_node_for_legacy_sessions() {
        let mut store = SessionStore::memory_only();
        let session_id = "legacy-root-session";
        let node_id = "node-checkout-1".to_string();
        store.sessions.insert(
            session_id.to_string(),
            SessionState {
                conversation_id: session_id.to_string(),
                title: DEFAULT_SESSION_TITLE.to_string(),
                summary: DEFAULT_SESSION_SUMMARY.to_string(),
                history: vec![
                    TurnHistoryMessage {
                        role: "user".to_string(),
                        content: "第一轮".to_string(),
                        attachments: Vec::new(),
                        ..Default::default()
                    },
                    TurnHistoryMessage {
                        role: "assistant".to_string(),
                        content: "收到。".to_string(),
                        attachments: Vec::new(),
                        ..Default::default()
                    },
                ],
                provider_native_transcript: Vec::new(),
                turn_trace_history: vec![TurnTraceRecord {
                    turn_id: "turn-1".to_string(),
                    phase: "completed".to_string(),
                    title: "first turn".to_string(),
                    ..Default::default()
                }],
                trace_migration_state: TraceMigrationState::default(),
                turn_trace_refs: None,
                long_term_memory_entries: Vec::new(),
                memory_write_evidence: Vec::new(),
                memory_write_hook_trace_records: Vec::new(),
                history_state_evidence: Vec::new(),
                turn_count: 1,
                last_referenced_file: None,
                updated_at_ms: now_timestamp_ms(),
                history_nodes: vec![HistoryNode {
                    node_id: node_id.clone(),
                    session_id: session_id.to_string(),
                    parent_node_id: None,
                    branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                    title: DEFAULT_SESSION_TITLE.to_string(),
                    summary: DEFAULT_SESSION_SUMMARY.to_string(),
                    history: vec![
                        TurnHistoryMessage {
                            role: "user".to_string(),
                            content: "第一轮".to_string(),
                            attachments: Vec::new(),
                            ..Default::default()
                        },
                        TurnHistoryMessage {
                            role: "assistant".to_string(),
                            content: "收到。".to_string(),
                            attachments: Vec::new(),
                            ..Default::default()
                        },
                    ],
                    turn_trace_history: vec![TurnTraceRecord {
                        turn_id: "turn-1".to_string(),
                        phase: "completed".to_string(),
                        title: "first turn".to_string(),
                        ..Default::default()
                    }],
                    turn_count: 1,
                    ..Default::default()
                }],
                history_branches: vec![HistoryBranch {
                    branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                    head_node_id: Some(node_id.clone()),
                    base_node_id: Some(node_id.clone()),
                    ..Default::default()
                }],
                history_cursor: HistoryCursor {
                    session_id: session_id.to_string(),
                    visible_node_id: Some(node_id.clone()),
                    active_branch_id: Some(DEFAULT_HISTORY_BRANCH_ID.to_string()),
                    branch_head_node_id: Some(node_id.clone()),
                    workspace_node_id: Some(node_id.clone()),
                    mode: HistoryCursorMode::Live,
                    ..Default::default()
                },
                workspace_id: None,
                event_watermark: 0,
                last_commit_watermark: 0,
            },
        );

        let (history_nodes, history_branches, _) = store.load_history_graph(Some(session_id));
        let root_node_id = format!("{}-legacy-root", session_id);

        assert_eq!(history_nodes.len(), 2);
        assert_eq!(history_nodes[0].node_id, root_node_id);
        assert_eq!(history_nodes[0].turn_count, 0);
        assert_eq!(
            history_nodes[1].parent_node_id.as_deref(),
            Some(history_nodes[0].node_id.as_str())
        );
        assert_eq!(
            history_branches[0].base_node_id.as_deref(),
            Some(history_nodes[0].node_id.as_str())
        );
    }

    #[test]
    fn file_backend_roundtrip_restores_sessions() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);
        store.append_turn(
            Some("persisted"),
            "打开 Cargo.toml",
            "已读取",
            None,
            Vec::new(),
        );

        let reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let mut reloaded = reloaded;
        let snapshot = reloaded.snapshot(Some("persisted"), &[]);
        assert_eq!(snapshot.title, "打开 Cargo.toml");

        assert_eq!(snapshot.turn_count, 1);
        assert_eq!(snapshot.history.len(), 2);
        assert_eq!(snapshot.last_referenced_file.as_deref(), Some("Cargo.toml"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn long_term_memory_entries_roundtrip_through_store() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);
        let snapshot = store.replace_long_term_memory(
            Some("memory-session"),
            vec![LongTermMemoryRecord {
                kind: "user_preference".to_string(),
                content: "Reply in Chinese and keep answers concise.".to_string(),
                source: "explicit_user_message".to_string(),
                updated_at_ms: 42,
            }],
        );

        assert_eq!(snapshot.long_term_memory_entries.len(), 1);
        assert_eq!(snapshot.long_term_memory_entries[0].kind, "user_preference");

        let reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let mut reloaded = reloaded;
        let snapshot = reloaded.snapshot(Some("memory-session"), &[]);
        assert_eq!(snapshot.long_term_memory_entries.len(), 1);
        assert_eq!(
            snapshot.long_term_memory_entries[0].content,
            "Reply in Chinese and keep answers concise."
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn append_turn_persists_memory_write_evidence_for_explicit_note() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("memory-evidence"),
            "请记住这个项目当前优先推进 PA-018。",
            "我会记住这条信息。",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("memory-evidence"), &[]);
        assert!(!snapshot.memory_write_evidence.is_empty());
        let latest_node_id = snapshot
            .history_cursor
            .visible_node_id
            .clone()
            .expect("latest history node id");
        assert!(snapshot.memory_write_evidence.iter().any(|evidence| {
            evidence.effect_kind == "memory_write.long_term_memory"
                && evidence.boundary == "session.update_long_term_memory_from_user_message"
                && evidence.replay_required_if_missing
                && evidence
                    .persistence_ref
                    .starts_with("long_term_memory_entries/")
                && evidence.source_history_node_id.as_deref() == Some(latest_node_id.as_str())
        }));
    }

    #[test]
    fn memory_write_evidence_roundtrip_through_store() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);
        store.append_turn(
            Some("memory-evidence-roundtrip"),
            "请记住这个项目当前优先推进 PA-018。",
            "我会记住这条信息。",
            None,
            Vec::new(),
        );

        let reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let mut reloaded = reloaded;
        let snapshot = reloaded.snapshot(Some("memory-evidence-roundtrip"), &[]);
        assert!(!snapshot.memory_write_evidence.is_empty());
        assert!(snapshot.memory_write_evidence.iter().all(|evidence| {
            evidence.effect_kind == "memory_write.long_term_memory"
                && evidence.replay_required_if_missing
                && evidence.source_history_node_id.is_some()
        }));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn memory_write_guard_deny_blocks_persistence_and_memory_mutation() {
        let mut store = SessionStore::memory_only();
        store.set_memory_write_hook_executor_for_test(Box::new(StaticMemoryWriteHookExecutor {
            results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "memory.guard".to_string(),
                hook_class: crate::agent::hooks::HookClass::Guard,
                hook_point: crate::agent::hooks::TurnHookPoint::ContextBuildStart,
                hook_order: 1,
                result_kind: crate::agent::hooks::HookResultKind::Deny,
                structured_result: HookStructuredResult::Deny(
                    crate::agent::hooks::HookDenyDecision {
                        reason_code: "memory_write_blocked".to_string(),
                        message: "memory write denied by guard".to_string(),
                    },
                ),
                blocked: true,
                elapsed_ms: 1,
                input_summary: Some("deny memory write".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "memory write denied".to_string(),
            }],
        }));

        store.append_turn(
            Some("memory-guard-deny"),
            "请记住这个项目当前优先推进 PA-039。",
            "收到。",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("memory-guard-deny"), &[]);
        assert!(snapshot.long_term_memory_entries.is_empty());
        assert!(snapshot.memory_write_evidence.is_empty());
        assert_eq!(snapshot.memory_write_hook_trace_records.len(), 1);
        assert_eq!(
            snapshot.memory_write_hook_trace_records[0].hook_name,
            "memory.guard"
        );
        assert!(snapshot.memory_write_hook_trace_records[0].blocked);
        assert_eq!(
            snapshot.memory_write_hook_trace_records[0].summary,
            "memory write denied"
        );
    }

    #[test]
    fn memory_write_transform_patch_can_rewrite_persisted_memory_intent() {
        let mut store = SessionStore::memory_only();
        store.set_memory_write_hook_executor_for_test(Box::new(StaticMemoryWriteHookExecutor {
            results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "memory.transform".to_string(),
                hook_class: crate::agent::hooks::HookClass::Transform,
                hook_point: crate::agent::hooks::TurnHookPoint::ContextBuildEnd,
                hook_order: 1,
                result_kind: crate::agent::hooks::HookResultKind::Patch,
                structured_result: HookStructuredResult::Patch {
                    operations: vec![crate::agent::hooks::HookPatchOperation {
                        target: HookPatchTarget::MemoryWriteIntent,
                        path: "writes[1].content".to_string(),
                        operation: HookPatchOperationKind::Set,
                        value_summary: Some("Current active task is PA-040.".to_string()),
                        value_text: Some("Current active task is PA-040.".to_string()),
                    }],
                },
                blocked: false,
                elapsed_ms: 1,
                input_summary: Some("rewrite memory content".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "memory write transformed".to_string(),
            }],
        }));

        store.append_turn(
            Some("memory-transform"),
            "请记住这个项目当前优先推进 PA-039。",
            "收到。",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("memory-transform"), &[]);
        let active_task = snapshot
            .long_term_memory_entries
            .iter()
            .find(|entry| entry.kind == "project_focus.active_task")
            .expect("transformed active task entry");
        assert_eq!(active_task.content, "Current active task is PA-040.");
        assert!(snapshot.memory_write_evidence.iter().any(|evidence| {
            evidence.target_summary.contains("PA-040")
                && evidence.persistence_ref == "long_term_memory_entries/project_focus.active_task"
        }));
        assert_eq!(snapshot.memory_write_hook_trace_records.len(), 1);
        assert_eq!(
            snapshot.memory_write_hook_trace_records[0].hook_name,
            "memory.transform"
        );
        assert!(!snapshot.memory_write_hook_trace_records[0].blocked);
    }

    #[test]
    fn memory_write_hook_trace_records_roundtrip_through_store() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);
        store.set_memory_write_hook_executor_for_test(Box::new(StaticMemoryWriteHookExecutor {
            results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "memory.transform".to_string(),
                hook_class: crate::agent::hooks::HookClass::Transform,
                hook_point: crate::agent::hooks::TurnHookPoint::ContextBuildEnd,
                hook_order: 1,
                result_kind: crate::agent::hooks::HookResultKind::Patch,
                structured_result: HookStructuredResult::Patch {
                    operations: vec![crate::agent::hooks::HookPatchOperation {
                        target: HookPatchTarget::MemoryWriteIntent,
                        path: "writes[1].content".to_string(),
                        operation: HookPatchOperationKind::Set,
                        value_summary: Some("Current active task is PA-040.".to_string()),
                        value_text: Some("Current active task is PA-040.".to_string()),
                    }],
                },
                blocked: false,
                elapsed_ms: 1,
                input_summary: Some("rewrite memory content".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "memory write transformed".to_string(),
            }],
        }));
        store.append_turn(
            Some("memory-hook-trace-roundtrip"),
            "请记住这个项目当前优先推进 PA-039。",
            "收到。",
            None,
            Vec::new(),
        );

        let mut reloaded =
            SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let snapshot = reloaded.snapshot(Some("memory-hook-trace-roundtrip"), &[]);
        assert_eq!(snapshot.memory_write_hook_trace_records.len(), 1);
        assert_eq!(
            snapshot.memory_write_hook_trace_records[0].hook_name,
            "memory.transform"
        );
        assert_eq!(
            snapshot.memory_write_hook_trace_records[0].summary,
            "memory write transformed"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn history_checkout_restores_memory_write_hook_trace_records_from_selected_node() {
        let mut store = SessionStore::memory_only();
        store.set_memory_write_hook_executor_for_test(Box::new(StaticMemoryWriteHookExecutor {
            results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "memory.transform".to_string(),
                hook_class: crate::agent::hooks::HookClass::Transform,
                hook_point: crate::agent::hooks::TurnHookPoint::ContextBuildEnd,
                hook_order: 1,
                result_kind: crate::agent::hooks::HookResultKind::Patch,
                structured_result: HookStructuredResult::Patch {
                    operations: vec![crate::agent::hooks::HookPatchOperation {
                        target: HookPatchTarget::MemoryWriteIntent,
                        path: "writes[1].content".to_string(),
                        operation: HookPatchOperationKind::Set,
                        value_summary: Some("Current active task is PA-040.".to_string()),
                        value_text: Some("Current active task is PA-040.".to_string()),
                    }],
                },
                blocked: false,
                elapsed_ms: 1,
                input_summary: Some("rewrite memory content".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "memory write transformed".to_string(),
            }],
        }));
        store.append_turn(
            Some("memory-hook-history"),
            "请记住这个项目当前优先推进 PA-039。",
            "收到。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("memory-hook-history"),
            "请记住这个项目当前风险是 trace reload 不稳定。",
            "收到。",
            None,
            Vec::new(),
        );

        let (nodes, _, _) = store.load_history_graph(Some("memory-hook-history"));
        assert!(nodes.len() >= 3);
        assert_eq!(
            nodes[nodes.len() - 2].memory_write_hook_trace_records.len(),
            1
        );
        assert_eq!(
            nodes[nodes.len() - 1].memory_write_hook_trace_records.len(),
            2
        );

        let historical = store
            .checkout_history_node(
                Some("memory-hook-history"),
                nodes[1].node_id.as_str(),
                HistoryCheckoutMode::TranscriptOnly,
                None,
            )
            .expect("checkout should succeed");
        assert_eq!(historical.memory_write_hook_trace_records.len(), 1);
        assert_eq!(
            historical.memory_write_hook_trace_records[0].hook_name,
            "memory.transform"
        );

        let live = store.snapshot(Some("memory-hook-history"), &[]);
        assert_eq!(live.memory_write_hook_trace_records.len(), 1);
    }

    #[test]
    fn checkout_history_node_persists_history_state_hook_evidence() {
        let mut store = SessionStore::memory_only();
        store.set_history_state_hook_executor_for_test(Box::new(StaticHistoryStateHookExecutor {
            start_results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "history.guard.observe".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareStart,
                hook_order: 1,
                result_kind: HookResultKind::Observe,
                structured_result: HookStructuredResult::Observe {
                    summary: "history checkout start observed".to_string(),
                },
                blocked: false,
                elapsed_ms: 2,
                input_summary: Some("checkout start".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "history checkout start observed".to_string(),
            }],
            resolved_results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "history.resolved.observe".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareEnd,
                hook_order: 1,
                result_kind: HookResultKind::Observe,
                structured_result: HookStructuredResult::Observe {
                    summary: "history checkout resolved".to_string(),
                },
                blocked: false,
                elapsed_ms: 3,
                input_summary: Some("checkout resolved".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "history checkout resolved".to_string(),
            }],
        }));
        store.append_turn(
            Some("history-hook-session"),
            "第一轮对话",
            "收到第一轮。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("history-hook-session"),
            "第二轮对话",
            "收到第二轮。",
            None,
            Vec::new(),
        );

        let (nodes, _, _) = store.load_history_graph(Some("history-hook-session"));
        let snapshot = store
            .checkout_history_node(
                Some("history-hook-session"),
                nodes[0].node_id.as_str(),
                HistoryCheckoutMode::TranscriptOnly,
                None,
            )
            .expect("checkout should succeed");

        assert_eq!(snapshot.history_state_evidence.len(), 2);
        assert_eq!(
            snapshot.history_state_evidence[0].boundary,
            "history.checkout.start"
        );
        assert_eq!(
            snapshot.history_state_evidence[1].boundary,
            "history.checkout.resolved"
        );
        assert_eq!(
            snapshot.history_state_evidence[1]
                .resolved_node_id
                .as_deref(),
            Some(nodes[0].node_id.as_str())
        );
        assert!(!snapshot.history_state_evidence[1].degraded);
    }

    #[test]
    fn checkout_history_node_blocked_by_hook_persists_only_start_evidence() {
        let mut store = SessionStore::memory_only();
        store.set_history_state_hook_executor_for_test(Box::new(StaticHistoryStateHookExecutor {
            start_results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "history.guard.deny".to_string(),
                hook_class: crate::agent::hooks::HookClass::Guard,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareStart,
                hook_order: 1,
                result_kind: HookResultKind::Deny,
                structured_result: HookStructuredResult::Deny(
                    crate::agent::hooks::HookDenyDecision {
                        reason_code: "history_checkout_blocked".to_string(),
                        message: "history checkout denied by guard".to_string(),
                    },
                ),
                blocked: true,
                elapsed_ms: 1,
                input_summary: Some("checkout denied".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "history checkout denied".to_string(),
            }],
            resolved_results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "history.resolved.observe".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareEnd,
                hook_order: 1,
                result_kind: HookResultKind::Observe,
                structured_result: HookStructuredResult::Observe {
                    summary: "should not execute".to_string(),
                },
                blocked: false,
                elapsed_ms: 1,
                input_summary: Some("unexpected resolved".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "unexpected resolved".to_string(),
            }],
        }));
        store.append_turn(
            Some("history-hook-blocked"),
            "第一轮对话",
            "收到第一轮。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("history-hook-blocked"),
            "第二轮对话",
            "收到第二轮。",
            None,
            Vec::new(),
        );

        let live_before = store.snapshot(Some("history-hook-blocked"), &[]);
        let latest_before = live_before
            .history_cursor
            .visible_node_id
            .clone()
            .expect("latest visible node before blocked checkout");
        let (nodes, _, _) = store.load_history_graph(Some("history-hook-blocked"));
        let error = store
            .checkout_history_node(
                Some("history-hook-blocked"),
                nodes[0].node_id.as_str(),
                HistoryCheckoutMode::TranscriptOnly,
                None,
            )
            .expect_err("checkout should be blocked by hook");
        assert!(error.contains("history checkout blocked by hook"));

        let live_after = store.snapshot(Some("history-hook-blocked"), &[]);
        assert_eq!(live_after.history_state_evidence.len(), 1);
        assert_eq!(
            live_after.history_state_evidence[0].boundary,
            "history.checkout.start"
        );
        assert_eq!(live_after.history_state_evidence[0].resolved_node_id, None);
        assert_eq!(
            live_after.history_cursor.visible_node_id.as_deref(),
            Some(latest_before.as_str())
        );
        assert_eq!(live_after.history_cursor.mode, HistoryCursorMode::Live);
    }

    #[test]
    fn history_state_hook_evidence_roundtrip_through_file_backend() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);
        store.set_history_state_hook_executor_for_test(Box::new(StaticHistoryStateHookExecutor {
            start_results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "history.guard.observe".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareStart,
                hook_order: 1,
                result_kind: HookResultKind::Observe,
                structured_result: HookStructuredResult::Observe {
                    summary: "history checkout start observed".to_string(),
                },
                blocked: false,
                elapsed_ms: 2,
                input_summary: Some("checkout start".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "history checkout start observed".to_string(),
            }],
            resolved_results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "history.resolved.observe".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareEnd,
                hook_order: 1,
                result_kind: HookResultKind::Observe,
                structured_result: HookStructuredResult::Observe {
                    summary: "history checkout resolved".to_string(),
                },
                blocked: false,
                elapsed_ms: 3,
                input_summary: Some("checkout resolved".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "history checkout resolved".to_string(),
            }],
        }));
        store.append_turn(
            Some("history-hook-roundtrip"),
            "第一轮对话",
            "收到第一轮。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("history-hook-roundtrip"),
            "第二轮对话",
            "收到第二轮。",
            None,
            Vec::new(),
        );

        let (nodes, _, _) = store.load_history_graph(Some("history-hook-roundtrip"));
        store
            .checkout_history_node(
                Some("history-hook-roundtrip"),
                nodes[0].node_id.as_str(),
                HistoryCheckoutMode::TranscriptOnly,
                None,
            )
            .expect("checkout should succeed");

        let mut reloaded =
            SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let snapshot = reloaded.snapshot(Some("history-hook-roundtrip"), &[]);
        assert_eq!(snapshot.history_state_evidence.len(), 2);
        assert_eq!(
            snapshot.history_state_evidence[0].boundary,
            "history.checkout.start"
        );
        assert_eq!(
            snapshot.history_state_evidence[1].boundary,
            "history.checkout.resolved"
        );
        assert_eq!(
            snapshot.history_state_evidence[1]
                .resolved_node_id
                .as_deref(),
            Some(nodes[0].node_id.as_str())
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn restore_branch_head_persists_history_state_hook_evidence() {
        let mut store = SessionStore::memory_only();
        store.set_history_state_hook_executor_for_test(Box::new(StaticHistoryStateHookExecutor {
            start_results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "history.restore.observe".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareStart,
                hook_order: 1,
                result_kind: HookResultKind::Observe,
                structured_result: HookStructuredResult::Observe {
                    summary: "history restore start observed".to_string(),
                },
                blocked: false,
                elapsed_ms: 2,
                input_summary: Some("restore start".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "history restore start observed".to_string(),
            }],
            resolved_results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "history.restore.resolved".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareEnd,
                hook_order: 1,
                result_kind: HookResultKind::Observe,
                structured_result: HookStructuredResult::Observe {
                    summary: "history restore resolved".to_string(),
                },
                blocked: false,
                elapsed_ms: 3,
                input_summary: Some("restore resolved".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "history restore resolved".to_string(),
            }],
        }));
        store.append_turn(
            Some("history-restore-session"),
            "第一轮对话",
            "收到第一轮。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("history-restore-session"),
            "第二轮对话",
            "收到第二轮。",
            None,
            Vec::new(),
        );

        let snapshot = store
            .restore_branch_head(Some("history-restore-session"), Some("branch-main"), None)
            .expect("restore should succeed");

        assert_eq!(snapshot.history_state_evidence.len(), 2);
        assert_eq!(
            snapshot.history_state_evidence[0].boundary,
            "history.branch_restore.start"
        );
        assert_eq!(
            snapshot.history_state_evidence[1].boundary,
            "history.branch_restore.resolved"
        );
        assert_eq!(
            snapshot.history_state_evidence[1]
                .resolved_branch_id
                .as_deref(),
            Some("branch-main")
        );
        assert_eq!(snapshot.history_cursor.mode, HistoryCursorMode::Live);
    }

    #[test]
    fn fork_from_history_node_blocked_by_hook_persists_only_start_evidence() {
        let mut store = SessionStore::memory_only();
        store.set_history_state_hook_executor_for_test(Box::new(StaticHistoryStateHookExecutor {
            start_results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "history.fork.deny".to_string(),
                hook_class: crate::agent::hooks::HookClass::Guard,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareStart,
                hook_order: 1,
                result_kind: HookResultKind::Deny,
                structured_result: HookStructuredResult::Deny(
                    crate::agent::hooks::HookDenyDecision {
                        reason_code: "history_fork_blocked".to_string(),
                        message: "history fork denied by guard".to_string(),
                    },
                ),
                blocked: true,
                elapsed_ms: 1,
                input_summary: Some("fork denied".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "history fork denied".to_string(),
            }],
            resolved_results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "history.fork.resolved".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareEnd,
                hook_order: 1,
                result_kind: HookResultKind::Observe,
                structured_result: HookStructuredResult::Observe {
                    summary: "should not execute".to_string(),
                },
                blocked: false,
                elapsed_ms: 1,
                input_summary: Some("unexpected fork resolved".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "unexpected fork resolved".to_string(),
            }],
        }));
        store.append_turn(
            Some("history-fork-blocked"),
            "第一轮对话",
            "收到第一轮。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("history-fork-blocked"),
            "第二轮对话",
            "收到第二轮。",
            None,
            Vec::new(),
        );

        let live_before = store.snapshot(Some("history-fork-blocked"), &[]);
        let latest_before = live_before
            .history_cursor
            .visible_node_id
            .clone()
            .expect("latest visible node before blocked fork");
        let (nodes, branches, _) = store.load_history_graph(Some("history-fork-blocked"));
        let error = store
            .fork_from_history_node(
                Some("history-fork-blocked"),
                nodes[0].node_id.as_str(),
                None,
            )
            .expect_err("fork should be blocked by hook");
        assert!(error.contains("history branch fork blocked by hook"));

        let live_after = store.snapshot(Some("history-fork-blocked"), &[]);
        assert_eq!(live_after.history_state_evidence.len(), 1);
        assert_eq!(
            live_after.history_state_evidence[0].boundary,
            "history.branch_fork.start"
        );
        assert_eq!(
            live_after.history_cursor.visible_node_id.as_deref(),
            Some(latest_before.as_str())
        );
        let (branches_after, _, _) = {
            let (nodes_after, branches_after, cursor_after) =
                store.load_history_graph(Some("history-fork-blocked"));
            (branches_after, nodes_after, cursor_after)
        };
        assert_eq!(branches_after.len(), branches.len());
    }

    #[test]
    fn switch_history_branch_persists_history_state_hook_evidence() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("history-switch-session"),
            "第一轮对话",
            "收到第一轮。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("history-switch-session"),
            "第二轮对话",
            "收到第二轮。",
            None,
            Vec::new(),
        );
        let (nodes, _, _) = store.load_history_graph(Some("history-switch-session"));
        store
            .fork_from_history_node(
                Some("history-switch-session"),
                nodes[0].node_id.as_str(),
                None,
            )
            .expect("fork should succeed before switch test");
        store.set_history_state_hook_executor_for_test(Box::new(StaticHistoryStateHookExecutor {
            start_results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "history.switch.observe".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareStart,
                hook_order: 1,
                result_kind: HookResultKind::Observe,
                structured_result: HookStructuredResult::Observe {
                    summary: "history switch start observed".to_string(),
                },
                blocked: false,
                elapsed_ms: 2,
                input_summary: Some("switch start".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "history switch start observed".to_string(),
            }],
            resolved_results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "history.switch.resolved".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareEnd,
                hook_order: 1,
                result_kind: HookResultKind::Observe,
                structured_result: HookStructuredResult::Observe {
                    summary: "history switch resolved".to_string(),
                },
                blocked: false,
                elapsed_ms: 3,
                input_summary: Some("switch resolved".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "history switch resolved".to_string(),
            }],
        }));

        let snapshot = store
            .switch_history_branch(Some("history-switch-session"), "branch-main", None)
            .expect("switch should succeed");

        assert_eq!(snapshot.history_state_evidence.len(), 2);
        assert_eq!(
            snapshot.history_state_evidence[0].boundary,
            "history.branch_switch.start"
        );
        assert_eq!(
            snapshot.history_state_evidence[1].boundary,
            "history.branch_switch.resolved"
        );
        assert_eq!(
            snapshot.history_state_evidence[1]
                .resolved_branch_id
                .as_deref(),
            Some("branch-main")
        );
        assert_eq!(
            snapshot.history_cursor.active_branch_id.as_deref(),
            Some("branch-main")
        );
    }

    #[test]
    fn checkout_history_node_preserves_degraded_truth_source_with_hooks() {
        let mut store = SessionStore::memory_only();
        store.set_history_state_hook_executor_for_test(Box::new(StaticHistoryStateHookExecutor {
            start_results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "history.checkout.observe".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareStart,
                hook_order: 1,
                result_kind: HookResultKind::Observe,
                structured_result: HookStructuredResult::Observe {
                    summary: "history checkout start observed".to_string(),
                },
                blocked: false,
                elapsed_ms: 2,
                input_summary: Some("checkout start".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "history checkout start observed".to_string(),
            }],
            resolved_results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "history.checkout.resolved".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareEnd,
                hook_order: 1,
                result_kind: HookResultKind::Observe,
                structured_result: HookStructuredResult::Observe {
                    summary: "history checkout resolved".to_string(),
                },
                blocked: false,
                elapsed_ms: 3,
                input_summary: Some("checkout resolved".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "history checkout resolved".to_string(),
            }],
        }));
        store.append_turn(
            Some("history-degrade-truth"),
            "第一轮对话",
            "收到第一轮。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("history-degrade-truth"),
            "第二轮对话",
            "收到第二轮。",
            None,
            Vec::new(),
        );

        let (nodes, _, _) = store.load_history_graph(Some("history-degrade-truth"));
        let snapshot = store
            .checkout_history_node(
                Some("history-degrade-truth"),
                nodes[0].node_id.as_str(),
                HistoryCheckoutMode::TranscriptAndWorkspace,
                None,
            )
            .expect("degraded checkout should succeed");

        assert_eq!(
            snapshot.history_cursor.checkout_mode,
            HistoryCheckoutMode::TranscriptAndWorkspace
        );
        assert_eq!(
            snapshot.history_cursor.checkout_status,
            HistoryCheckoutStatus::DegradedToTranscriptOnly
        );
        // After truncation the target node IS the branch head, so mode is Live.
        assert_eq!(snapshot.history_cursor.mode, HistoryCursorMode::Live);
        assert_eq!(snapshot.history_state_evidence.len(), 2);
        assert_eq!(
            snapshot.history_state_evidence[1].boundary,
            "history.checkout.resolved"
        );
        assert!(snapshot.history_state_evidence[1].degraded);
        assert_eq!(
            snapshot.history_state_audit_summary.action.status,
            "available"
        );
        assert_eq!(
            snapshot
                .history_state_audit_summary
                .action
                .boundary
                .as_deref(),
            Some("history.checkout.resolved")
        );
        assert!(snapshot.history_state_audit_summary.action.degraded);
        assert_eq!(
            snapshot.history_state_audit_summary.current_context.mode,
            "live"
        );
    }

    #[test]
    fn missing_history_state_evidence_does_not_reconstruct_restore_conclusion_after_reload() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);
        store.set_history_state_hook_executor_for_test(Box::new(StaticHistoryStateHookExecutor {
            start_results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "history.checkout.observe".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareStart,
                hook_order: 1,
                result_kind: HookResultKind::Observe,
                structured_result: HookStructuredResult::Observe {
                    summary: "history checkout start observed".to_string(),
                },
                blocked: false,
                elapsed_ms: 2,
                input_summary: Some("checkout start".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "history checkout start observed".to_string(),
            }],
            resolved_results: vec![crate::agent::hooks::HookExecutionResult {
                hook_name: "history.checkout.resolved".to_string(),
                hook_class: crate::agent::hooks::HookClass::Observe,
                hook_point: crate::agent::hooks::TurnHookPoint::TurnPrepareEnd,
                hook_order: 1,
                result_kind: HookResultKind::Observe,
                structured_result: HookStructuredResult::Observe {
                    summary: "history checkout resolved".to_string(),
                },
                blocked: false,
                elapsed_ms: 3,
                input_summary: Some("checkout resolved".to_string()),
                persistence_evidence_ref: None,
                trace_summary: "history checkout resolved".to_string(),
            }],
        }));
        store.append_turn(
            Some("history-missing-evidence"),
            "第一轮对话",
            "收到第一轮。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("history-missing-evidence"),
            "第二轮对话",
            "收到第二轮。",
            None,
            Vec::new(),
        );

        let (nodes, _, _) = store.load_history_graph(Some("history-missing-evidence"));
        let initial = store
            .checkout_history_node(
                Some("history-missing-evidence"),
                nodes[0].node_id.as_str(),
                HistoryCheckoutMode::TranscriptAndWorkspace,
                None,
            )
            .expect("degraded checkout should succeed");
        // TranscriptAndWorkspace checkout on a non-rollback-capable node
        // degrades to transcript-only, so the cursor status reflects the
        // degrade conclusion rather than NotRequested (d6e1fbf semantics).
        assert_eq!(
            initial.history_cursor.checkout_status,
            HistoryCheckoutStatus::DegradedToTranscriptOnly
        );
        assert_eq!(initial.history_state_evidence.len(), 2);
        let resolved_node_id = initial
            .resolved_node_id
            .clone()
            .expect("resolved node id after checkout");

        {
            let session = store
                .sessions
                .get_mut("history-missing-evidence")
                .expect("persisted session should exist");
            session.history_state_evidence.clear();
        }
        store.save_to_backend();

        let mut reloaded =
            SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let snapshot = reloaded.snapshot(Some("history-missing-evidence"), &[]);
        assert!(snapshot.history_state_evidence.is_empty());
        assert_eq!(
            snapshot.history_state_audit_summary.action.status,
            "missing"
        );
        assert_eq!(
            snapshot.history_cursor.checkout_status,
            HistoryCheckoutStatus::DegradedToTranscriptOnly
        );
        assert_eq!(
            snapshot.history_cursor.checkout_mode,
            HistoryCheckoutMode::TranscriptAndWorkspace
        );
        // After destructive truncation the checked-out node IS the branch
        // head, so the persisted mode is Live rather than Historical.
        assert_eq!(snapshot.history_cursor.mode, HistoryCursorMode::Live);
        assert_eq!(
            snapshot.resolved_node_id.as_deref(),
            Some(resolved_node_id.as_str())
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn append_turn_extracts_explicit_user_preferences_into_long_term_memory() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("memory-write"),
            "请用中文回复，并尽量简洁。",
            "好的，我会用中文并尽量简洁。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("memory-write"),
            "请用中文回复，并尽量简洁。",
            "收到。",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("memory-write"), &[]);
        assert_eq!(snapshot.long_term_memory_entries.len(), 2);
        assert_eq!(
            snapshot.long_term_memory_entries[0].kind,
            "user_preference.response_language"
        );
        assert_eq!(
            snapshot.long_term_memory_entries[0].source,
            "explicit_user_message"
        );
        assert_eq!(
            snapshot.long_term_memory_entries[1].kind,
            "user_preference.response_style"
        );
    }

    #[test]
    fn append_turn_extracts_explicit_memory_note_without_overwriting_preferences() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("memory-note"),
            "请用中文回复，并尽量简洁。",
            "好的。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("memory-note"),
            "请记住这个项目当前优先推进 PA-018。",
            "我会记住这条信息。",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("memory-note"), &[]);
        assert_eq!(snapshot.long_term_memory_entries.len(), 4);
        assert!(snapshot
            .long_term_memory_entries
            .iter()
            .any(|entry| entry.kind == "user_preference.response_language"));
        assert!(snapshot
            .long_term_memory_entries
            .iter()
            .any(|entry| entry.kind == "user_preference.response_style"));
        assert!(snapshot.long_term_memory_entries.iter().any(|entry| {
            entry.kind == "user_memory.explicit_note"
                && entry.content == "这个项目当前优先推进 PA-018。"
                && entry.source == "explicit_user_message"
        }));
        assert!(snapshot.long_term_memory_entries.iter().any(|entry| {
            entry.kind == "project_focus.active_task"
                && entry.content == "Current active task is PA-018."
                && entry.source == "explicit_user_message"
        }));
    }

    #[test]
    fn append_turn_extracts_explicit_file_reference_preference_into_long_term_memory() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("memory-path-style"),
            "引用文件时请使用绝对路径，不要使用相对路径。",
            "好的，后续引用文件时我会使用绝对路径。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("memory-path-style"),
            "引用文件时请使用绝对路径，不要使用相对路径。",
            "收到。",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("memory-path-style"), &[]);
        let matching_entries = snapshot
            .long_term_memory_entries
            .iter()
            .filter(|entry| entry.kind == "user_preference.file_reference_style")
            .collect::<Vec<_>>();
        assert_eq!(matching_entries.len(), 1);
        assert_eq!(
            matching_entries[0].content,
            "Use absolute paths when referencing workspace files."
        );
        assert_eq!(matching_entries[0].source, "explicit_user_message");
    }

    #[test]
    fn append_turn_extracts_task_system_sync_preference_into_long_term_memory() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("memory-task-sync"),
            "推进 PA-018 的同时记得更新任务文档，并同步任务系统。",
            "好的，我会同步更新任务文档和任务系统。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("memory-task-sync"),
            "推进 PA-018 的同时记得更新任务文档，并同步任务系统。",
            "收到。",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("memory-task-sync"), &[]);
        let matching_entries = snapshot
            .long_term_memory_entries
            .iter()
            .filter(|entry| entry.kind == "user_preference.task_system_sync")
            .collect::<Vec<_>>();
        assert_eq!(matching_entries.len(), 1);
        assert_eq!(
            matching_entries[0].content,
            "Keep task-system documents updated while progressing work."
        );
        assert_eq!(matching_entries[0].source, "explicit_user_message");
    }

    #[test]
    fn append_turn_extracts_change_scope_preference_into_long_term_memory() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("memory-change-scope"),
            "不要修改无关文件，也不要回滚无关改动。",
            "好的，我会避免修改无关文件和无关改动。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("memory-change-scope"),
            "不要修改无关文件，也不要回滚无关改动。",
            "收到。",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("memory-change-scope"), &[]);
        let matching_entries = snapshot
            .long_term_memory_entries
            .iter()
            .filter(|entry| entry.kind == "user_preference.change_scope")
            .collect::<Vec<_>>();
        assert_eq!(matching_entries.len(), 1);
        assert_eq!(
            matching_entries[0].content,
            "Avoid modifying unrelated existing changes."
        );
        assert_eq!(matching_entries[0].source, "explicit_user_message");
    }

    #[test]
    fn append_turn_extracts_acceptance_gate_into_long_term_memory() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("memory-acceptance-gate"),
            "现在开始 PA-018 任务，建立验收标准，确保执行成功完成交付，并更新任务文档。",
            "收到，我会先建立验收标准并持续回写任务文档。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("memory-acceptance-gate"),
            "现在开始 PA-018 任务，建立验收标准，确保执行成功完成交付，并更新任务文档。",
            "继续推进。",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("memory-acceptance-gate"), &[]);
        let matching_entries = snapshot
            .long_term_memory_entries
            .iter()
            .filter(|entry| entry.kind == "project_workflow.acceptance_gate")
            .collect::<Vec<_>>();
        assert_eq!(matching_entries.len(), 1);
        assert_eq!(
            matching_entries[0].content,
            "Establish acceptance criteria and run a closeout audit before claiming delivery."
        );
        assert_eq!(matching_entries[0].source, "explicit_user_message");
    }

    #[test]
    fn append_turn_extracts_project_dependency_prerequisite_into_long_term_memory() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("memory-prerequisite"),
            "先完成 PA-017，再做 PA-018。",
            "收到，我会先确认前置任务。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("memory-prerequisite"),
            "先完成 PA-017，再做 PA-018。",
            "继续推进。",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("memory-prerequisite"), &[]);
        let matching_entries = snapshot
            .long_term_memory_entries
            .iter()
            .filter(|entry| entry.kind == "project_dependency.prerequisite")
            .collect::<Vec<_>>();
        assert_eq!(matching_entries.len(), 1);
        assert_eq!(matching_entries[0].content, "PA-018 depends on PA-017.");
        assert_eq!(matching_entries[0].source, "explicit_user_message");
    }

    #[test]
    fn append_turn_extracts_closeout_requirement_into_long_term_memory() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("memory-closeout"),
            "完成后说明改了哪些文件、做了什么验证、还有什么未解决风险。",
            "收到，我会按这个收口口径汇报。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("memory-closeout"),
            "完成后说明改了哪些文件、做了什么验证、还有什么未解决风险。",
            "继续。",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("memory-closeout"), &[]);
        let matching_entries = snapshot
            .long_term_memory_entries
            .iter()
            .filter(|entry| entry.kind == "project_workflow.closeout_requirement")
            .collect::<Vec<_>>();
        assert_eq!(matching_entries.len(), 1);
        assert_eq!(
            matching_entries[0].content,
            "Summarize changed files, verification performed, and unresolved risks at closeout."
        );
        assert_eq!(matching_entries[0].source, "explicit_user_message");
    }

    #[test]
    fn append_turn_extracts_task_boundary_into_long_term_memory() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("memory-task-boundary"),
            "目标是 PA-018，不能越界到 PA-024、PA-025。",
            "收到，我会控制范围。",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("memory-task-boundary"), &[]);
        let matching_entries = snapshot
            .long_term_memory_entries
            .iter()
            .filter(|entry| entry.kind == "project_scope.task_boundary")
            .collect::<Vec<_>>();
        assert_eq!(matching_entries.len(), 1);
        assert_eq!(
            matching_entries[0].content,
            "Do not expand scope into PA-024, PA-025."
        );
        assert_eq!(matching_entries[0].source, "explicit_user_message");
    }

    #[test]
    fn append_turn_extracts_explicit_active_task_focus_into_long_term_memory() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("memory-active-task"),
            "现在开始 PA-018 任务，当前优先推进这个任务。",
            "好的，我会优先推进 PA-018。",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("memory-active-task"), &[]);
        let matching_entries = snapshot
            .long_term_memory_entries
            .iter()
            .filter(|entry| entry.kind == "project_focus.active_task")
            .collect::<Vec<_>>();
        assert_eq!(matching_entries.len(), 1);
        assert_eq!(
            matching_entries[0].content,
            "Current active task is PA-018."
        );
        assert_eq!(matching_entries[0].source, "explicit_user_message");
    }

    #[test]
    fn append_turn_updates_active_task_focus_instead_of_accumulating_duplicates() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("memory-active-task-update"),
            "现在开始 PA-018 任务，当前优先推进这个任务。",
            "好的，我会优先推进 PA-018。",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("memory-active-task-update"),
            "现在开始 PA-020 任务，后续优先推进这个任务。",
            "好的，我会切换到 PA-020。",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("memory-active-task-update"), &[]);
        let matching_entries = snapshot
            .long_term_memory_entries
            .iter()
            .filter(|entry| entry.kind == "project_focus.active_task")
            .collect::<Vec<_>>();
        assert_eq!(matching_entries.len(), 1);
        assert_eq!(
            matching_entries[0].content,
            "Current active task is PA-020."
        );
    }

    #[test]
    fn append_turn_does_not_extract_active_task_focus_from_incidental_task_mentions() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("memory-active-task-incidental"),
            "PA-018 看起来像是目前其他任务的前置任务，对吗？",
            "是的，它像一个前置任务。",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("memory-active-task-incidental"), &[]);
        assert!(!snapshot
            .long_term_memory_entries
            .iter()
            .any(|entry| entry.kind == "project_focus.active_task"));
    }

    #[test]
    fn append_turn_does_not_extract_acceptance_gate_from_incidental_acceptance_mentions() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("memory-incidental-acceptance"),
            "帮我看看这个目录里有没有验收标准文档模板。",
            "我先去定位相关文档。",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("memory-incidental-acceptance"), &[]);
        assert!(!snapshot
            .long_term_memory_entries
            .iter()
            .any(|entry| entry.kind == "project_workflow.acceptance_gate"));
    }

    #[test]
    fn attachment_payloads_can_be_restored_from_recent_history() {
        let path = temp_sessions_path();
        let attachment_root = path
            .parent()
            .map(|parent| parent.join("attachments"))
            .expect("attachment root");
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);
        let images = vec![TurnInputImage {
            data_url: "data:image/png;base64,AAAA".to_string(),
            mime_type: "image/png".to_string(),
            name: Some("diagram.png".to_string()),
        }];

        let attachments = store
            .save_input_attachments("with-attachments", &images)
            .expect("save attachments");
        store.append_turn(
            Some("with-attachments"),
            "[已附图片 1 张：diagram.png]",
            "我看到了这张图。",
            None,
            attachments,
        );

        let restored = store.load_recent_images(Some("with-attachments"), 1);
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].data_url, images[0].data_url);
        assert_eq!(restored[0].mime_type, "image/png");
        let snapshot = store.snapshot(Some("with-attachments"), &[]);
        assert_eq!(snapshot.attachment_assets.len(), 1);
        assert_eq!(snapshot.attachment_assets[0].mime_type, "image/png");
        assert_eq!(
            store.list_attachment_assets(Some("with-attachments")).len(),
            1
        );

        store.remove_session("with-attachments");
        assert!(!attachment_root.join("with-attachments").exists());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn recent_images_only_recall_from_latest_user_turn() {
        let mut store = SessionStore::memory_only();
        let images = vec![TurnInputImage {
            data_url: "data:image/png;base64,AAAA".to_string(),
            mime_type: "image/png".to_string(),
            name: Some("diagram.png".to_string()),
        }];

        let attachments = store
            .save_input_attachments("latest-only", &images)
            .expect("save attachments");
        store.append_turn(
            Some("latest-only"),
            "[已附图片 1 张：diagram.png]",
            "我看到了这张图。",
            None,
            attachments,
        );
        store.append_turn(
            Some("latest-only"),
            "继续查看 runtime.rs。",
            "好的，我继续查看代码。",
            None,
            Vec::new(),
        );

        let restored = store.load_recent_images(Some("latest-only"), 1);
        assert!(restored.is_empty());
    }

    #[test]
    fn attachment_assets_are_indexed_across_sessions_without_scanning_history() {
        let mut store = SessionStore::memory_only();
        let alpha_images = vec![TurnInputImage {
            data_url: "data:image/png;base64,AAAA".to_string(),
            mime_type: "image/png".to_string(),
            name: Some("alpha.png".to_string()),
        }];
        let beta_images = vec![TurnInputImage {
            data_url: "data:image/jpeg;base64,BBBB".to_string(),
            mime_type: "image/jpeg".to_string(),
            name: Some("beta.jpg".to_string()),
        }];

        let alpha_attachments = store
            .save_input_attachments("alpha", &alpha_images)
            .expect("save alpha attachments");
        store.append_turn(
            Some("alpha"),
            "[已附图片 1 张：alpha.png]",
            "我看到了 alpha 图片。",
            None,
            alpha_attachments,
        );

        let beta_attachments = store
            .save_input_attachments("beta", &beta_images)
            .expect("save beta attachments");
        store.append_turn(
            Some("beta"),
            "[已附图片 1 张：beta.jpg]",
            "我看到了 beta 图片。",
            None,
            beta_attachments,
        );

        let all_assets = store.list_attachment_assets(None);
        assert_eq!(all_assets.len(), 2);
        assert_eq!(store.list_attachment_assets(Some("alpha")).len(), 1);
        assert_eq!(store.list_attachment_assets(Some("beta")).len(), 1);
    }

    #[test]
    fn doc_text_reference_attachments_do_not_materialize_phantom_assets() {
        // PA-078 AC#4：doc/text 以引用附着（.tmp/imports/...），不应进入 AttachmentAsset 目录
        // （relative_path 相对 attachment_root 不存在 → 否则物化为 MissingPayload 幽灵资产）。
        let mut store = SessionStore::memory_only();
        let session_id = format!("docref-{}", now_timestamp_ms());
        let doc_attachment = AttachmentReference {
            id: "ref-doc-1".to_string(),
            asset_id: String::new(),
            name: Some("foo.md".to_string()),
            mime_type: "text/markdown".to_string(),
            relative_path: ".tmp/imports/foo.md".to_string(),
            size_bytes: 12,
            created_at_ms: now_timestamp_ms(),
        };
        store.append_turn(
            Some(&session_id),
            "[附件: foo.md]",
            "已收到。",
            None,
            vec![doc_attachment.clone()],
        );
        store.refresh_attachment_catalog();

        let phantom_id = attachment_asset_id(".tmp/imports/foo.md");
        assert!(
            !store.attachment_assets.contains_key(&phantom_id),
            "doc/text 引用不应物化为 AttachmentAsset（phantom: {phantom_id}）"
        );

        // 对照：图片资产仍正常物化
        let images = vec![TurnInputImage {
            data_url: "data:image/png;base64,AAAA".to_string(),
            mime_type: "image/png".to_string(),
            name: Some("active.png".to_string()),
        }];
        let image_attachments = store
            .save_input_attachments(&session_id, &images)
            .expect("save image attachments");
        store.append_turn(
            Some(&session_id),
            "[image: active.png]",
            "看到了。",
            None,
            image_attachments,
        );
        store.refresh_attachment_catalog();
        assert_eq!(store.list_attachment_assets(None).len(), 1, "图片资产应保留");
    }

    #[test]
    fn session_store_workspace_registry_default_and_crud() {
        // PA-079：默认 workspace 始终存在；create/list/resolve 正确。
        let mut store = SessionStore::memory_only();
        let workspaces = store.list_workspaces();
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].id, crate::agent::workspace::DEFAULT_WORKSPACE_ID);

        let root = std::env::temp_dir().join(format!("pa079-store-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let created = store.create_workspace("Docs", root.to_str().unwrap()).unwrap();
        assert!(created.id.starts_with("ws-docs-"));

        assert_eq!(store.list_workspaces().len(), 2);
        assert_eq!(
            store.resolve_workspace_root(Some(&created.id)).unwrap(),
            crate::agent::workspace::normalize_workspace_root(root.to_str().unwrap()).unwrap()
        );
        // 缺省 → 默认 workspace
        assert!(store.resolve_workspace_root(None).is_ok());

        // 重复 root 拒绝
        assert!(store.create_workspace("Docs2", root.to_str().unwrap()).is_err());

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn session_store_stamps_workspace_id_only_once() {
        // PA-079：TurnInput.workspace_id 首次盖章；后续轮 no-op。
        let mut store = SessionStore::memory_only();
        store.ensure_session("ws-session");
        assert!(store.sessions["ws-session"].workspace_id.is_none());

        store.stamp_workspace_id("ws-session", "ws-proj-1");
        assert_eq!(store.sessions["ws-session"].workspace_id.as_deref(), Some("ws-proj-1"));

        store.stamp_workspace_id("ws-session", "ws-proj-2");
        assert_eq!(store.sessions["ws-session"].workspace_id.as_deref(), Some("ws-proj-1"));
    }

    #[test]
    fn legacy_session_without_workspace_id_projects_to_default() {
        // PA-079：旧会话（无 workspace_id 字段）serde 兼容 + 快照投影 None。
        let mut store = SessionStore::memory_only();
        store.ensure_session("legacy");
        let snapshot = store.snapshot(Some("legacy"), &[]);
        assert_eq!(snapshot.workspace_id, None);

        // P2-7b：模拟旧 schema blob（无 workspace_id 字段）→ 全字段原样往返 + workspace_id=None。
        let mut legacy_session = store.sessions["legacy"].clone();
        legacy_session.title = "旧标题".to_string();
        legacy_session.summary = "旧摘要".to_string();
        legacy_session.turn_count = 3;
        legacy_session.updated_at_ms = 12345;
        legacy_session.history = vec![TurnHistoryMessage {
            role: "user".to_string(),
            content: "旧内容".to_string(),
            attachments: Vec::new(),
            ..Default::default()
        }];
        let serialized = serde_json::to_string(&legacy_session).unwrap();
        // 去掉 workspace_id 键模拟旧版本写入的 blob
        let mut value: serde_json::Value = serde_json::from_str(&serialized).unwrap();
        if let serde_json::Value::Object(map) = &mut value {
            map.remove("workspaceId");
        }
        let restored: SessionState = serde_json::from_str(&value.to_string()).unwrap();
        assert_eq!(restored.workspace_id, None, "旧 blob 无 workspace_id → None");
        assert_eq!(restored.title, "旧标题");
        assert_eq!(restored.summary, "旧摘要");
        assert_eq!(restored.turn_count, 3);
        assert_eq!(restored.updated_at_ms, 12345);
        assert_eq!(restored.history.len(), 1);
        assert_eq!(restored.history[0].content, "旧内容");
    }

    #[test]
    fn corrupt_file_backend_falls_back_to_default_workspace() {
        // PA-079 P1-2：File backend 整文件损坏 → load_store None → 默认 workspace 重建。
        let dir = std::env::temp_dir().join(format!("pa079-file-corrupt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let storage_path = dir.join("store.json");
        std::fs::write(&storage_path, "{bad json").unwrap();

        let backend = FileSessionBackend {
            storage_path: storage_path.clone(),
        };
        let store = SessionStore::with_backend(Box::new(backend));
        let workspaces = store.list_workspaces();
        assert_eq!(workspaces.len(), 1, "损坏回退后应只剩默认 workspace");
        assert_eq!(workspaces[0].id, crate::agent::workspace::DEFAULT_WORKSPACE_ID);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn attachment_assets_expose_lifecycle_statuses_and_queries() {
        let mut store = SessionStore::memory_only();
        let session_id = format!("lifecycle-{}", now_timestamp_ms());
        let active_images = vec![TurnInputImage {
            data_url: "data:image/png;base64,AAAA".to_string(),
            mime_type: "image/png".to_string(),
            name: Some("active.png".to_string()),
        }];
        let missing_images = vec![TurnInputImage {
            data_url: "data:image/jpeg;base64,BBBB".to_string(),
            mime_type: "image/jpeg".to_string(),
            name: Some("missing.jpg".to_string()),
        }];
        let reclaimable_images = vec![TurnInputImage {
            data_url: "data:image/webp;base64,CCCC".to_string(),
            mime_type: "image/webp".to_string(),
            name: Some("draft.webp".to_string()),
        }];
        let expired_images = vec![TurnInputImage {
            data_url: "data:image/gif;base64,DDDD".to_string(),
            mime_type: "image/gif".to_string(),
            name: Some("old.gif".to_string()),
        }];

        let active_attachments = store
            .save_input_attachments(&session_id, &active_images)
            .expect("save active attachments");
        store.append_turn(
            Some(&session_id),
            "[image 1: active.png]",
            "active image is still referenced",
            None,
            active_attachments,
        );

        // 每次保存之间推进时钟，避免同一毫秒内 asset_id 碰撞（asset_id 基于 created_at_ms）
        std::thread::sleep(std::time::Duration::from_millis(2));
        let missing_attachments = store
            .save_input_attachments(&session_id, &missing_images)
            .expect("save missing attachments");
        let missing_relative_path = missing_attachments[0].relative_path.clone();
        store.append_turn(
            Some(&session_id),
            "[image 1: missing.jpg]",
            "missing image is still referenced",
            None,
            missing_attachments.clone(),
        );
        let _ = fs::remove_file(store.attachment_root.join(&missing_relative_path));

        std::thread::sleep(std::time::Duration::from_millis(2));
        let reclaimable_attachments = store
            .save_input_attachments(&session_id, &reclaimable_images)
            .expect("save reclaimable attachments");
        let reclaimable_asset_id = reclaimable_attachments[0].asset_id.clone();

        std::thread::sleep(std::time::Duration::from_millis(2));
        let expired_attachments = store
            .save_input_attachments(&session_id, &expired_images)
            .expect("save expired attachments");
        let expired_asset_id = expired_attachments[0].asset_id.clone();
        let expired_created_at_ms = now_timestamp_ms()
            .saturating_sub(DEFAULT_ATTACHMENT_RECLAIM_TTL_MS)
            .saturating_sub(1_000);
        store
            .attachment_assets
            .get_mut(&expired_asset_id)
            .expect("expired asset")
            .created_at_ms = expired_created_at_ms;

        let all_assets = store.list_attachment_assets(None);
        let active_asset = all_assets
            .iter()
            .find(|asset| asset.name.as_deref() == Some("active.png"))
            .expect("active asset");
        let missing_asset = all_assets
            .iter()
            .find(|asset| asset.name.as_deref() == Some("missing.jpg"))
            .expect("missing asset");
        let reclaimable_asset = all_assets
            .iter()
            .find(|asset| asset.id == reclaimable_asset_id)
            .expect("reclaimable asset");
        let expired_asset = all_assets
            .iter()
            .find(|asset| asset.id == expired_asset_id)
            .expect("expired asset");

        assert_eq!(active_asset.status, AttachmentLifecycleStatus::Active);
        assert_eq!(active_asset.reference_count, 1);
        assert_eq!(
            missing_asset.status,
            AttachmentLifecycleStatus::MissingPayload
        );
        assert_eq!(missing_asset.reference_count, 1);
        assert_eq!(
            reclaimable_asset.status,
            AttachmentLifecycleStatus::Reclaimable
        );
        assert_eq!(reclaimable_asset.reference_count, 0);
        assert_eq!(expired_asset.status, AttachmentLifecycleStatus::Expired);
        assert_eq!(expired_asset.reference_count, 0);
        assert!(expired_asset.expires_at_ms.is_some());

        let filtered = store.query_attachment_assets(&AttachmentAssetQuery {
            session_id: Some(session_id.clone()),
            mime_type: Some("jpeg".to_string()),
            name_contains: Some("missing".to_string()),
            created_after_ms: None,
            created_before_ms: None,
            statuses: vec![AttachmentLifecycleStatus::MissingPayload],
            limit: Some(1),
        });
        assert_eq!(filtered.len(), 1);
        assert_eq!(
            filtered[0].status,
            AttachmentLifecycleStatus::MissingPayload
        );

        let expired_only = store.query_attachment_assets(&AttachmentAssetQuery {
            created_before_ms: Some(expired_created_at_ms),
            statuses: vec![AttachmentLifecycleStatus::Expired],
            ..AttachmentAssetQuery::default()
        });
        assert_eq!(expired_only.len(), 1);
        assert_eq!(expired_only[0].id, expired_asset_id);
    }

    #[test]
    fn cleanup_attachment_assets_only_reclaims_unreferenced_payloads() {
        let mut store = SessionStore::memory_only();
        let session_id = format!("cleanup-{}", now_timestamp_ms());
        let active_images = vec![TurnInputImage {
            data_url: "data:image/png;base64,AAAA".to_string(),
            mime_type: "image/png".to_string(),
            name: Some("keep.png".to_string()),
        }];
        let reclaimable_images = vec![TurnInputImage {
            data_url: "data:image/png;base64,BBBB".to_string(),
            mime_type: "image/png".to_string(),
            name: Some("trash.png".to_string()),
        }];
        let expired_images = vec![TurnInputImage {
            data_url: "data:image/png;base64,CCCC".to_string(),
            mime_type: "image/png".to_string(),
            name: Some("old.png".to_string()),
        }];
        let missing_images = vec![TurnInputImage {
            data_url: "data:image/png;base64,DDDD".to_string(),
            mime_type: "image/png".to_string(),
            name: Some("missing.png".to_string()),
        }];

        let active_attachments = store
            .save_input_attachments(&session_id, &active_images)
            .expect("save active attachments");
        let active_relative_path = active_attachments[0].relative_path.clone();
        store.append_turn(
            Some(&session_id),
            "[image 1: keep.png]",
            "keep it",
            None,
            active_attachments,
        );

        // 每次保存之间推进时钟，避免同一毫秒内 asset_id 碰撞（asset_id 基于 created_at_ms）
        std::thread::sleep(std::time::Duration::from_millis(2));
        let reclaimable_attachments = store
            .save_input_attachments(&session_id, &reclaimable_images)
            .expect("save reclaimable attachments");
        let reclaimable_asset_id = reclaimable_attachments[0].asset_id.clone();
        let reclaimable_path = store
            .attachment_root
            .join(&reclaimable_attachments[0].relative_path);

        std::thread::sleep(std::time::Duration::from_millis(2));
        let expired_attachments = store
            .save_input_attachments(&session_id, &expired_images)
            .expect("save expired attachments");
        let expired_asset_id = expired_attachments[0].asset_id.clone();
        let expired_path = store
            .attachment_root
            .join(&expired_attachments[0].relative_path);
        store
            .attachment_assets
            .get_mut(&expired_asset_id)
            .expect("expired asset")
            .created_at_ms = now_timestamp_ms()
            .saturating_sub(DEFAULT_ATTACHMENT_RECLAIM_TTL_MS)
            .saturating_sub(1_000);

        std::thread::sleep(std::time::Duration::from_millis(2));
        let missing_attachments = store
            .save_input_attachments(&session_id, &missing_images)
            .expect("save missing attachments");
        let missing_asset_id = missing_attachments[0].asset_id.clone();
        let missing_path = store
            .attachment_root
            .join(&missing_attachments[0].relative_path);
        store.append_turn(
            Some(&session_id),
            "[image 1: missing.png]",
            "still referenced",
            None,
            missing_attachments,
        );
        let _ = fs::remove_file(&missing_path);

        let result = store.cleanup_attachment_assets(&AttachmentCleanupRequest {
            session_id: Some(session_id.clone()),
            expire_before_ms: Some(now_timestamp_ms()),
            include_reclaimable: true,
            include_expired: true,
            limit: None,
        });

        assert_eq!(result.removed_catalog_count, 2);
        assert!(result.removed_asset_ids.contains(&reclaimable_asset_id));
        assert!(result.removed_asset_ids.contains(&expired_asset_id));
        assert!(!result.removed_asset_ids.contains(&missing_asset_id));
        assert!(!reclaimable_path.exists());
        assert!(!expired_path.exists());
        assert!(store.attachment_root.join(active_relative_path).exists());
        assert_eq!(store.load_recent_images(Some(&session_id), 1).len(), 0);

        let remaining_assets = store.list_attachment_assets(Some(&session_id));
        assert!(remaining_assets
            .iter()
            .any(|asset| asset.status == AttachmentLifecycleStatus::Active));
        assert!(remaining_assets
            .iter()
            .any(|asset| asset.status == AttachmentLifecycleStatus::MissingPayload));
    }

    #[test]
    fn session_title_uses_first_user_message_preview() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("preview"),
            "Please inspect runtime.rs session switching and trace consistency after tool execution.",
            "I will check it.",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("preview"),
            "Also verify provider fallback behavior.",
            "Done.",
            None,
            Vec::new(),
        );

        let snapshot = store.snapshot(Some("preview"), &[]);
        assert_eq!(snapshot.title, "Please inspect runtime.rs se...");
    }

    #[test]
    fn removing_last_session_recreates_default_session() {
        let mut store = SessionStore::memory_only();
        let sessions = store.remove_session(DEFAULT_SESSION_ID);

        assert!(sessions.is_empty());
        let snapshot = store.snapshot(Some(DEFAULT_SESSION_ID), &[]);
        assert_eq!(snapshot.conversation_id, DEFAULT_SESSION_ID);
        assert_eq!(snapshot.title, DEFAULT_SESSION_TITLE);
    }

    #[test]
    fn removing_session_clears_attachment_catalog_without_full_refresh() {
        let mut store = SessionStore::memory_only();
        let images = vec![TurnInputImage {
            data_url: "data:image/png;base64,AAAA".to_string(),
            mime_type: "image/png".to_string(),
            name: Some("delete-me.png".to_string()),
        }];

        let attachments = store
            .save_input_attachments("remove-attachments", &images)
            .expect("save attachments");
        store.append_turn(
            Some("remove-attachments"),
            "[已附图片 1 张：delete-me.png]",
            "我看到了图片。",
            None,
            attachments,
        );

        assert_eq!(
            store
                .list_attachment_assets(Some("remove-attachments"))
                .len(),
            1
        );
        assert_eq!(
            store
                .session_attachment_index
                .get("remove-attachments")
                .map(Vec::len),
            Some(1)
        );

        store.remove_session("remove-attachments");

        assert!(store
            .list_attachment_assets(Some("remove-attachments"))
            .is_empty());
        assert!(!store
            .session_attachment_index
            .contains_key("remove-attachments"));
        assert!(store
            .attachment_assets
            .values()
            .all(|asset| asset.session_id != "remove-attachments"));
    }

    #[test]
    fn snapshot_clears_legacy_native_transcript_without_reasoning_content() {
        let session_id = "legacy-reasoning";
        let mut store = SessionStore::memory_only();
        store.sessions.insert(
            session_id.to_string(),
            SessionState {
                conversation_id: session_id.to_string(),
                title: DEFAULT_SESSION_TITLE.to_string(),
                summary: DEFAULT_SESSION_SUMMARY.to_string(),
                history: vec![TurnHistoryMessage {
                    role: "user".to_string(),
                    attachments: Vec::new(),
                    content: "继续".to_string(),
                    ..Default::default()
                }],
                provider_native_transcript: vec![
                    serde_json::json!({
                        "role": "user",
                        "content": "看看文件"
                    }),
                    serde_json::json!({
                        "role": "assistant",
                        "tool_calls": [
                            {
                                "id": "call_legacy",
                                "type": "function",
                                "function": {
                                    "name": "workspace_list_files",
                                    "arguments": "{}"
                                }
                            }
                        ]
                    }),
                ],
                turn_trace_history: Vec::new(),
                trace_migration_state: TraceMigrationState::default(),
                turn_trace_refs: None,
                long_term_memory_entries: Vec::new(),
                memory_write_evidence: Vec::new(),
                memory_write_hook_trace_records: Vec::new(),
                history_state_evidence: Vec::new(),
                turn_count: 1,
                last_referenced_file: None,
                updated_at_ms: now_timestamp_ms(),
                history_nodes: Vec::new(),
                history_branches: Vec::new(),
                history_cursor: HistoryCursor::default(),
                workspace_id: None,
                event_watermark: 0,
                last_commit_watermark: 0,
            },
        );

        let snapshot = store.snapshot(Some(session_id), &[]);
        assert!(snapshot.provider_native_transcript.is_empty());
    }

    #[test]
    fn snapshot_clears_tool_turn_transcript_when_final_assistant_lacks_reasoning() {
        let session_id = "legacy-tool-final";
        let mut store = SessionStore::memory_only();
        store.sessions.insert(
            session_id.to_string(),
            SessionState {
                conversation_id: session_id.to_string(),
                title: DEFAULT_SESSION_TITLE.to_string(),
                summary: DEFAULT_SESSION_SUMMARY.to_string(),
                history: vec![TurnHistoryMessage {
                    role: "user".to_string(),
                    attachments: Vec::new(),
                    content: "继续".to_string(),
                    ..Default::default()
                }],
                provider_native_transcript: vec![
                    serde_json::json!({
                        "role": "user",
                        "content": "读取 tauri.conf.json"
                    }),
                    serde_json::json!({
                        "role": "assistant",
                        "reasoning_content": "先读取文件。",
                        "tool_calls": [
                            {
                                "id": "call_tool",
                                "type": "function",
                                "function": {
                                    "name": "workspace_read_file",
                                    "arguments": "{\"path\":\"src-tauri/tauri.conf.json\"}"
                                }
                            }
                        ]
                    }),
                    serde_json::json!({
                        "role": "tool",
                        "tool_call_id": "call_tool",
                        "content": "{...}"
                    }),
                    serde_json::json!({
                        "role": "assistant",
                        "content": "这是 Tauri 配置文件。"
                    }),
                ],
                turn_trace_history: Vec::new(),
                trace_migration_state: TraceMigrationState::default(),
                turn_trace_refs: None,
                long_term_memory_entries: Vec::new(),
                memory_write_evidence: Vec::new(),
                memory_write_hook_trace_records: Vec::new(),
                history_state_evidence: Vec::new(),
                turn_count: 1,
                last_referenced_file: Some("src-tauri/tauri.conf.json".to_string()),
                updated_at_ms: now_timestamp_ms(),
                history_nodes: Vec::new(),
                history_branches: Vec::new(),
                history_cursor: HistoryCursor::default(),
                workspace_id: None,
                event_watermark: 0,
                last_commit_watermark: 0,
            },
        );

        let snapshot = store.snapshot(Some(session_id), &[]);
        assert!(snapshot.provider_native_transcript.is_empty());
    }

    #[test]
    fn snapshot_clears_incomplete_native_tool_roundtrip() {
        let session_id = "incomplete-tool-roundtrip";
        let mut store = SessionStore::memory_only();
        store.sessions.insert(
            session_id.to_string(),
            SessionState {
                conversation_id: session_id.to_string(),
                title: DEFAULT_SESSION_TITLE.to_string(),
                summary: DEFAULT_SESSION_SUMMARY.to_string(),
                history: vec![
                    TurnHistoryMessage {
                        role: "user".to_string(),
                        attachments: Vec::new(),
                        content: "请记住 tauri.conf.json".to_string(),
                        ..Default::default()
                    },
                    TurnHistoryMessage {
                        role: "assistant".to_string(),
                        attachments: Vec::new(),
                        content: "搜索没有直接命中，让我查看一下工作区的文件结构。".to_string(),
                        ..Default::default()
                    },
                ],
                provider_native_transcript: vec![
                    serde_json::json!({
                        "role": "user",
                        "content": "请记住 tauri.conf.json"
                    }),
                    serde_json::json!({
                        "role": "assistant",
                        "content": "先搜索文件",
                        "reasoning_content": "先搜索一下。",
                        "tool_calls": [
                            {
                                "id": "call_search",
                                "type": "function",
                                "function": {
                                    "name": "workspace_search_text",
                                    "arguments": "{\"query\":\"tauri.conf.json\"}"
                                }
                            }
                        ]
                    }),
                    serde_json::json!({
                        "role": "tool",
                        "tool_call_id": "call_search",
                        "content": "{\"matchCount\":0}"
                    }),
                    serde_json::json!({
                        "role": "assistant",
                        "content": "搜索没有直接命中，让我查看一下工作区的文件结构。",
                        "reasoning_content": "继续列目录。",
                        "tool_calls": [
                            {
                                "id": "call_list",
                                "type": "function",
                                "function": {
                                    "name": "workspace_list_files",
                                    "arguments": "{\"path\":\".\"}"
                                }
                            }
                        ]
                    }),
                ],
                turn_trace_history: Vec::new(),
                trace_migration_state: TraceMigrationState::default(),
                turn_trace_refs: None,
                long_term_memory_entries: Vec::new(),
                memory_write_evidence: Vec::new(),
                memory_write_hook_trace_records: Vec::new(),
                history_state_evidence: Vec::new(),
                turn_count: 1,
                last_referenced_file: Some("tauri.conf.json".to_string()),
                updated_at_ms: now_timestamp_ms(),
                history_nodes: Vec::new(),
                history_branches: Vec::new(),
                history_cursor: HistoryCursor::default(),
                workspace_id: None,
                event_watermark: 0,
                last_commit_watermark: 0,
            },
        );

        let snapshot = store.snapshot(Some(session_id), &[]);
        assert!(snapshot.provider_native_transcript.is_empty());
    }

    #[test]
    fn snapshot_keeps_structured_reasoning_content_in_native_transcript() {
        let session_id = "structured-reasoning";
        let mut store = SessionStore::memory_only();
        store.sessions.insert(
            session_id.to_string(),
            SessionState {
                conversation_id: session_id.to_string(),
                title: DEFAULT_SESSION_TITLE.to_string(),
                summary: DEFAULT_SESSION_SUMMARY.to_string(),
                history: vec![TurnHistoryMessage {
                    role: "user".to_string(),
                    attachments: Vec::new(),
                    content: "继续".to_string(),
                    ..Default::default()
                }],
                provider_native_transcript: vec![
                    serde_json::json!({
                        "role": "user",
                        "content": "读取 tauri.conf.json"
                    }),
                    serde_json::json!({
                        "role": "assistant",
                        "reasoning_content": [
                            { "type": "reasoning", "text": "先读取文件。" }
                        ],
                        "tool_calls": [
                            {
                                "id": "call_tool",
                                "type": "function",
                                "function": {
                                    "name": "workspace_read_file",
                                    "arguments": "{\"path\":\"src-tauri/tauri.conf.json\"}"
                                }
                            }
                        ]
                    }),
                    serde_json::json!({
                        "role": "tool",
                        "tool_call_id": "call_tool",
                        "content": "{...}"
                    }),
                    serde_json::json!({
                        "role": "assistant",
                        "reasoning_content": [
                            { "type": "reasoning", "text": "已读取并总结。" }
                        ],
                        "content": "这是 Tauri 配置文件。"
                    }),
                ],
                turn_trace_history: Vec::new(),
                trace_migration_state: TraceMigrationState::default(),
                turn_trace_refs: None,
                long_term_memory_entries: Vec::new(),
                memory_write_evidence: Vec::new(),
                memory_write_hook_trace_records: Vec::new(),
                history_state_evidence: Vec::new(),
                turn_count: 1,
                last_referenced_file: Some("src-tauri/tauri.conf.json".to_string()),
                updated_at_ms: now_timestamp_ms(),
                history_nodes: Vec::new(),
                history_branches: Vec::new(),
                history_cursor: HistoryCursor::default(),
                workspace_id: None,
                event_watermark: 0,
                last_commit_watermark: 0,
            },
        );

        let snapshot = store.snapshot(Some(session_id), &[]);
        assert_eq!(snapshot.provider_native_transcript.len(), 4);
    }

    fn temp_sessions_path() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir()
            .join(format!("pony-agent-session-test-{stamp}"))
            .join("sessions.json")
    }

    #[test]
    fn snapshot_does_not_persist_new_empty_session() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);

        let snapshot = store.snapshot(Some("fresh"), &[]);
        assert_eq!(snapshot.conversation_id, "fresh");
        assert_eq!(snapshot.turn_count, 0);

        let persisted = load_store_from_path(&path);
        assert!(persisted.is_some());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn file_backend_roundtrip_restores_turn_trace_history() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);
        store.record_turn_trace(
            Some("trace-persisted"),
            TurnTraceRecord {
                turn_id: "turn-1".to_string(),
                session_id: Some("trace-persisted".to_string()),
                event_id: Some("turn-1:4".to_string()),
                event_type: Some("turn.completed".to_string()),
                event_version: Some("turn-event-v1".to_string()),
                sequence: Some(4),
                emitted_at_ms: Some(4242),
                title: "检查流式输出".to_string(),
                phase: "completed".to_string(),
                trace_steps: vec![TurnTraceStep {
                    id: "step-return".to_string(),
                    label: "Return result".to_string(),
                    state: "completed".to_string(),
                }],
                trace_timeline: vec![TraceTimelineEntry {
                    id: "return-1".to_string(),
                    kind: "return".to_string(),
                    label: "RETURN RESULT".to_string(),
                    state: "completed".to_string(),
                    sequence: 1,
                    provider_requested_name: Some("ppx".to_string()),
                    provider_name: Some("ppx".to_string()),
                    provider_protocol: Some("openai".to_string()),
                    provider_model: Some("gpt-5.4".to_string()),
                    provider_source: Some("provider_decision".to_string()),
                    provider_mode: Some("live".to_string()),
                    build_context_observation: None,
                    tool_activities: Vec::new(),
                    text: Some("ok".to_string()),
                    reasoning_content: None,
                    fallback_reason: None,
                    error: None,
                    input_tokens: Some(12),
                    cache_hit_input_tokens: Some(5),
                    reasoning_tokens: Some(3),
                    output_tokens: Some(34),
                    total_tokens: Some(46),
                    first_token_latency_ms: Some(180),
                    turn_duration_ms: Some(920),
                }],
                tool_activities: vec![TurnToolActivity {
                    id: "tool-1".to_string(),
                    name: "workspace.read_file".to_string(),
                    canonical_tool_name: Some("Read".to_string()),
                    display_name_zh: Some("读取".to_string()),
                    status: "done".to_string(),
                    description: "读取文件".to_string(),
                    arguments_text: Some("{\"path\":\"src/main.ts\"}".to_string()),
                    result_text: Some("ok".to_string()),
                    duration_seconds: Some(0.12),
                    parent_activity_id: None,
                    artifacts: None,
                    error: None,
                    capability_invocation: Some(
                        crate::agent::telemetry::CapabilityInvocationRecord {
                            tool_name: "workspace.read_file".to_string(),
                            capability_id: Some("mcp:tool:workspace.read_file".to_string()),
                            source_id: Some("mcp-local".to_string()),
                            source_kind: Some("mcp".to_string()),
                            capability_kind: Some("tool".to_string()),
                            invocation_mode: Some("direct_tool_call".to_string()),
                            failure_kind: None,
                            requires_approval: Some(false),
                            host_mediated: Some(true),
                            permission_scope: Some("workspace.read".to_string()),
                            permission_facts: Some(crate::agent::tools::ToolPermissionFacts {
                                requires_approval: Some(false),
                                permission_scope: Some("workspace.read".to_string()),
                                host_mediated: Some(true),
                                permission_profile: Some("capability_registry".to_string()),
                                approval_mode: Some("none".to_string()),
                                decision_source: Some("capability_registry".to_string()),
                            }),
                            skill_id: None,
                            skill_source_id: None,
                            composed_capability_refs: None,
                            composed_capability_kinds: None,
                            failure_layer: None,
                        },
                    ),
                }],
                provider_call_records: vec![ProviderCallCacheRecord {
                    request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                    provider_source: Some("provider_decision".to_string()),
                    provider_mode: Some("live".to_string()),
                    input_tokens: Some(12),
                    cache_hit_input_tokens: Some(5),
                    cache_hit_source: None,
                    cache_miss_input_tokens: Some(7),
                    reasoning_tokens: Some(3),
                    output_tokens: Some(34),
                    total_tokens: Some(46),
                    first_token_latency_ms: Some(180),
                    turn_duration_ms: Some(920),
                    latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
                    prefix_mutation_reasons: Vec::new(),
                }],
                hook_trace_records: vec![HookTraceRecord {
                    hook_name: "audit.observe".to_string(),
                    hook_class: crate::agent::hooks::HookClass::Observe,
                    hook_point: crate::agent::hooks::TurnHookPoint::ModelCallStart,
                    hook_order: 1,
                    result_kind: crate::agent::hooks::HookResultKind::Observe,
                    structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                        summary: "hook observed lifecycle boundary without mutation".to_string(),
                    },
                    blocked: false,
                    elapsed_ms: 2,
                    input_summary: Some("prompt-prefix".to_string()),
                    persistence_evidence_ref: None,
                    summary: "observe hook summary".to_string(),
                }],
                provider_requested_name: Some("ppx".to_string()),
                provider_name: Some("ppx".to_string()),
                provider_protocol: Some("openai".to_string()),
                provider_model: Some("gpt-5.4".to_string()),
                provider_source: Some("provider_decision".to_string()),
                provider_mode: Some("live".to_string()),
                build_context_observation: None,
                session_summary: Some("测试 trace 持久化".to_string()),
                fallback_reason: None,
                error: None,
                input_tokens: Some(12),
                cache_hit_input_tokens: Some(5),
                reasoning_tokens: Some(3),
                output_tokens: Some(34),
                total_tokens: Some(46),
                first_token_latency_ms: Some(180),
                turn_duration_ms: Some(920),
                updated_at: 0,
            },
        );

        let expected_snapshot = store.snapshot(Some("trace-persisted"), &[]);
        let expected_trace = serde_json::to_value(&expected_snapshot.turn_trace_history[0])
            .expect("expected trace should serialize");

        let mut reloaded =
            SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let snapshot = reloaded.snapshot(Some("trace-persisted"), &[]);

        assert_eq!(snapshot.turn_trace_history.len(), 1);
        assert_eq!(snapshot.turn_trace_history[0].turn_id, "turn-1");
        assert_eq!(
            snapshot.turn_trace_history[0].session_id.as_deref(),
            Some("trace-persisted")
        );
        assert_eq!(
            snapshot.turn_trace_history[0].event_id.as_deref(),
            Some("turn-1:4")
        );
        assert_eq!(
            snapshot.turn_trace_history[0].event_type.as_deref(),
            Some("turn.completed")
        );
        assert_eq!(
            snapshot.turn_trace_history[0].event_version.as_deref(),
            Some("turn-event-v1")
        );
        assert_eq!(snapshot.turn_trace_history[0].sequence, Some(4));
        assert_eq!(snapshot.turn_trace_history[0].emitted_at_ms, Some(4242));
        assert_eq!(
            snapshot.turn_trace_history[0].provider_model.as_deref(),
            Some("gpt-5.4")
        );
        assert_eq!(
            snapshot.turn_trace_history[0].provider_call_records.len(),
            1
        );
        assert_eq!(
            snapshot.turn_trace_history[0].provider_call_records[0].cache_miss_input_tokens,
            Some(7)
        );
        assert_eq!(
            snapshot.turn_trace_history[0].tool_activities[0]
                .capability_invocation
                .as_ref()
                .and_then(|record| record.source_id.as_deref()),
            Some("mcp-local")
        );
        assert_eq!(snapshot.turn_trace_history[0].hook_trace_records.len(), 1);
        assert_eq!(
            snapshot.turn_trace_history[0].hook_trace_records[0].hook_name,
            "audit.observe"
        );
        assert_eq!(
            snapshot.turn_trace_history[0].trace_timeline[0].kind,
            "return_result"
        );
        assert_eq!(
            serde_json::to_value(&snapshot.turn_trace_history[0])
                .expect("reloaded trace should serialize"),
            expected_trace
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn file_backend_roundtrip_restores_runtime_generated_multi_boundary_hook_traces() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);

        store.record_turn_trace(
            Some("trace-multi-hook"),
            TurnTraceRecord {
                turn_id: "turn-multi-hook".to_string(),
                session_id: Some("trace-multi-hook".to_string()),
                event_id: Some("turn-multi-hook:11".to_string()),
                event_type: Some("turn.completed".to_string()),
                event_version: Some("turn-event-v1".to_string()),
                sequence: Some(11),
                emitted_at_ms: Some(1111),
                title: "multi boundary hook roundtrip".to_string(),
                phase: "completed".to_string(),
                trace_steps: vec![TurnTraceStep {
                    id: "step-return".to_string(),
                    label: "Return result".to_string(),
                    state: "completed".to_string(),
                }],
                trace_timeline: Vec::new(),
                tool_activities: Vec::new(),
                provider_call_records: Vec::new(),
                hook_trace_records: vec![
                    HookTraceRecord {
                        hook_name: "audit.observe".to_string(),
                        hook_class: crate::agent::hooks::HookClass::Observe,
                        hook_point: crate::agent::hooks::TurnHookPoint::ModelCallStart,
                        hook_order: 1,
                        result_kind: crate::agent::hooks::HookResultKind::Observe,
                        structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                            summary: "model boundary observed".to_string(),
                        },
                        blocked: false,
                        elapsed_ms: 2,
                        input_summary: Some("model".to_string()),
                        persistence_evidence_ref: None,
                        summary: "model hook summary".to_string(),
                    },
                    HookTraceRecord {
                        hook_name: "guard.tool".to_string(),
                        hook_class: crate::agent::hooks::HookClass::Guard,
                        hook_point: crate::agent::hooks::TurnHookPoint::ToolCallEnd,
                        hook_order: 1,
                        result_kind: crate::agent::hooks::HookResultKind::Allow,
                        structured_result: crate::agent::hooks::HookStructuredResult::Allow {
                            summary: "tool boundary allowed".to_string(),
                        },
                        blocked: false,
                        elapsed_ms: 4,
                        input_summary: Some("tool".to_string()),
                        persistence_evidence_ref: None,
                        summary: "tool hook summary".to_string(),
                    },
                ],
                provider_requested_name: Some("ppx".to_string()),
                provider_name: Some("ppx".to_string()),
                provider_protocol: Some("openai".to_string()),
                provider_model: Some("gpt-5.4".to_string()),
                provider_source: Some("provider_decision".to_string()),
                provider_mode: Some("live".to_string()),
                build_context_observation: None,
                session_summary: Some("多边界 hook 持久化".to_string()),
                fallback_reason: None,
                error: None,
                input_tokens: Some(12),
                cache_hit_input_tokens: Some(5),
                reasoning_tokens: Some(3),
                output_tokens: Some(34),
                total_tokens: Some(46),
                first_token_latency_ms: Some(180),
                turn_duration_ms: Some(920),
                updated_at: 0,
            },
        );

        let mut reloaded =
            SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let snapshot = reloaded.snapshot(Some("trace-multi-hook"), &[]);

        assert_eq!(snapshot.turn_trace_history.len(), 1);
        assert_eq!(snapshot.turn_trace_history[0].hook_trace_records.len(), 2);
        assert_eq!(
            snapshot.turn_trace_history[0].hook_trace_records[0].hook_name,
            "audit.observe"
        );
        assert_eq!(
            snapshot.turn_trace_history[0].hook_trace_records[1].hook_name,
            "guard.tool"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn file_backend_roundtrip_restores_terminal_event_annotation_after_trace_update() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);

        store.record_turn_trace(
            Some("trace-annotated"),
            TurnTraceRecord {
                turn_id: "turn-annotated".to_string(),
                session_id: Some("trace-annotated".to_string()),
                event_id: None,
                event_type: None,
                event_version: None,
                sequence: None,
                emitted_at_ms: None,
                title: "等待 terminal event".to_string(),
                phase: "completed".to_string(),
                trace_steps: vec![TurnTraceStep {
                    id: "step-return".to_string(),
                    label: "Return result".to_string(),
                    state: "completed".to_string(),
                }],
                trace_timeline: vec![TraceTimelineEntry {
                    id: "return-1".to_string(),
                    kind: "return".to_string(),
                    label: "RETURN RESULT".to_string(),
                    state: "completed".to_string(),
                    sequence: 1,
                    provider_requested_name: None,
                    provider_name: None,
                    provider_protocol: None,
                    provider_model: None,
                    provider_source: None,
                    provider_mode: None,
                    build_context_observation: None,
                    tool_activities: Vec::new(),
                    text: Some("ok".to_string()),
                    reasoning_content: None,
                    fallback_reason: None,
                    error: None,
                    input_tokens: Some(8),
                    cache_hit_input_tokens: Some(3),
                    reasoning_tokens: Some(1),
                    output_tokens: Some(13),
                    total_tokens: Some(21),
                    first_token_latency_ms: Some(90),
                    turn_duration_ms: Some(420),
                }],
                tool_activities: Vec::new(),
                provider_call_records: Vec::new(),
                hook_trace_records: vec![HookTraceRecord {
                    hook_name: "guard.input".to_string(),
                    hook_class: crate::agent::hooks::HookClass::Guard,
                    hook_point: crate::agent::hooks::TurnHookPoint::ContextBuildStart,
                    hook_order: 1,
                    result_kind: crate::agent::hooks::HookResultKind::Allow,
                    structured_result: crate::agent::hooks::HookStructuredResult::Allow {
                        summary: "guard allowed runtime to continue".to_string(),
                    },
                    blocked: false,
                    elapsed_ms: 1,
                    input_summary: Some("context-window".to_string()),
                    persistence_evidence_ref: None,
                    summary: "guard hook summary".to_string(),
                }],
                provider_requested_name: None,
                provider_name: None,
                provider_protocol: None,
                provider_model: None,
                provider_source: None,
                provider_mode: None,
                build_context_observation: None,
                session_summary: Some("等待回写".to_string()),
                fallback_reason: None,
                error: None,
                input_tokens: Some(8),
                cache_hit_input_tokens: Some(3),
                reasoning_tokens: Some(1),
                output_tokens: Some(13),
                total_tokens: Some(21),
                first_token_latency_ms: Some(90),
                turn_duration_ms: Some(420),
                updated_at: 0,
            },
        );

        let annotation_snapshot = store
            .annotate_turn_trace_terminal_event(
                Some("trace-annotated"),
                "turn-annotated",
                Some("turn-annotated:7".to_string()),
                Some("turn.completed".to_string()),
                Some("turn-event-v1".to_string()),
                Some(7),
                Some(7007),
            )
            .expect("annotation should update trace");
        assert_eq!(
            annotation_snapshot.turn_trace_history[0]
                .event_id
                .as_deref(),
            Some("turn-annotated:7")
        );

        let mut reloaded =
            SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let snapshot = reloaded.snapshot(Some("trace-annotated"), &[]);

        assert_eq!(snapshot.turn_trace_history.len(), 1);
        assert_eq!(
            snapshot.turn_trace_history[0].event_id.as_deref(),
            Some("turn-annotated:7")
        );
        assert_eq!(
            snapshot.turn_trace_history[0].event_type.as_deref(),
            Some("turn.completed")
        );
        assert_eq!(
            snapshot.turn_trace_history[0].event_version.as_deref(),
            Some("turn-event-v1")
        );
        assert_eq!(snapshot.turn_trace_history[0].sequence, Some(7));
        assert_eq!(snapshot.turn_trace_history[0].emitted_at_ms, Some(7007));
        assert_eq!(snapshot.turn_trace_history[0].hook_trace_records.len(), 1);
        assert_eq!(
            snapshot.turn_trace_history[0].hook_trace_records[0].hook_name,
            "guard.input"
        );
        assert_eq!(
            snapshot.turn_trace_history[0].trace_timeline[0].kind,
            "return_result"
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn append_turn_trace_hook_records_updates_existing_trace_and_roundtrips() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);

        store.record_turn_trace(
            Some("trace-append-hook"),
            TurnTraceRecord {
                turn_id: "turn-append-hook".to_string(),
                session_id: Some("trace-append-hook".to_string()),
                event_id: Some("turn-append-hook:3".to_string()),
                event_type: Some("turn.completed".to_string()),
                event_version: Some("turn-event-v1".to_string()),
                sequence: Some(3),
                emitted_at_ms: Some(3003),
                title: "等待 graph decision evidence".to_string(),
                phase: "completed".to_string(),
                trace_steps: vec![TurnTraceStep {
                    id: "step-return".to_string(),
                    label: "Return result".to_string(),
                    state: "completed".to_string(),
                }],
                trace_timeline: Vec::new(),
                tool_activities: Vec::new(),
                provider_call_records: Vec::new(),
                hook_trace_records: vec![HookTraceRecord {
                    hook_name: "planner.preflight.observe".to_string(),
                    hook_class: crate::agent::hooks::HookClass::Observe,
                    hook_point: crate::agent::hooks::TurnHookPoint::PlannerTurnPreflight,
                    hook_order: 1,
                    result_kind: crate::agent::hooks::HookResultKind::Observe,
                    structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                        summary: "planner preflight summary".to_string(),
                    },
                    blocked: false,
                    elapsed_ms: 1,
                    input_summary: Some("message=hello".to_string()),
                    persistence_evidence_ref: None,
                    summary: "planner preflight summary".to_string(),
                }],
                provider_requested_name: None,
                provider_name: None,
                provider_protocol: None,
                provider_model: None,
                provider_source: None,
                provider_mode: None,
                build_context_observation: None,
                session_summary: Some("append hook trace".to_string()),
                fallback_reason: None,
                error: None,
                input_tokens: None,
                cache_hit_input_tokens: None,
                reasoning_tokens: None,
                output_tokens: None,
                total_tokens: None,
                first_token_latency_ms: None,
                turn_duration_ms: None,
                updated_at: 0,
            },
        );

        let appended = store
            .append_turn_trace_hook_records(
                Some("trace-append-hook"),
                "turn-append-hook",
                vec![HookTraceRecord {
                    hook_name: "planner.graph_decision.observe".to_string(),
                    hook_class: crate::agent::hooks::HookClass::Observe,
                    hook_point: crate::agent::hooks::TurnHookPoint::PlannerGraphDecision,
                    hook_order: 1,
                    result_kind: crate::agent::hooks::HookResultKind::Observe,
                    structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                        summary: "planner graph decision summary".to_string(),
                    },
                    blocked: false,
                    elapsed_ms: 2,
                    input_summary: Some("run_phase=ready".to_string()),
                    persistence_evidence_ref: None,
                    summary: "planner graph decision summary".to_string(),
                }],
            )
            .expect("append should update existing trace");

        assert_eq!(appended.turn_trace_history.len(), 1);
        assert_eq!(appended.turn_trace_history[0].hook_trace_records.len(), 2);
        assert_eq!(
            appended.turn_trace_history[0].hook_trace_records[1].hook_name,
            "planner.graph_decision.observe"
        );

        let mut reloaded =
            SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let snapshot = reloaded.snapshot(Some("trace-append-hook"), &[]);
        assert_eq!(snapshot.turn_trace_history[0].hook_trace_records.len(), 2);
        assert_eq!(
            snapshot.turn_trace_history[0].hook_trace_records[1].hook_point,
            crate::agent::hooks::TurnHookPoint::PlannerGraphDecision
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn persisted_mcp_source_snapshots_roundtrip_through_store() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);
        store.persist_mcp_source_snapshot(crate::agent::capability_bridge::McpSourceSnapshot {
            source: crate::agent::capability_bridge::CapabilitySourceView {
                source_id: "mcp-local".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                display_name: "Local MCP".to_string(),
                transport_kind: "stdio".to_string(),
                server_identity: "mcp://local".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                declared_capabilities: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
                permission_profile: "host-mediated".to_string(),
                updated_at_ms: 7,
                last_ingress_observation: Some(
                    crate::agent::capability_bridge::SourceIngressObservation {
                        boundary: "control_plane.apply_mcp_source_snapshot".to_string(),
                        summary:
                            "mcp source ingress registered `mcp-local` with 1 capability candidates"
                                .to_string(),
                        candidate_ids: vec!["mcp:tool:workspace-search".to_string()],
                        observed_at_ms: 77,
                    },
                ),
            },
            capabilities: vec![crate::agent::capability_bridge::CapabilityView {
                capability_id: "mcp:tool:workspace-search".to_string(),
                source_id: "mcp-local".to_string(),
                source_kind: crate::agent::capability_bridge::CapabilitySourceKind::Mcp,
                kind: crate::agent::capability_bridge::CapabilityKind::Tool,
                label: "workspace.search".to_string(),
                description: "Search workspace files".to_string(),
                invocation_mode:
                    crate::agent::capability_bridge::CapabilityInvocationMode::DirectToolCall,
                input_schema_summary: "{}".to_string(),
                safety_class: "host_tool".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec!["mcp".to_string()],
                requires_approval: false,
                host_mediated: true,
                permission_scope: "workspace.read".to_string(),
            }],
        });

        let reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let snapshots = reloaded.list_persisted_mcp_source_snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].source.source_id, "mcp-local");
        assert_eq!(
            snapshots[0]
                .source
                .last_ingress_observation
                .as_ref()
                .expect("ingress observation should persist")
                .candidate_ids,
            vec!["mcp:tool:workspace-search".to_string()]
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn persisted_skill_source_snapshots_roundtrip_through_store() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);
        store.persist_skill_source_snapshot(crate::agent::capability_bridge::SkillSourceSnapshot {
            source: crate::agent::capability_bridge::SkillSourceView {
                source_id: "host-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                display_name: "Host Skills".to_string(),
                availability: crate::agent::capability_bridge::CapabilityAvailability::Available,
                transport_kind: "host".to_string(),
                server_identity: "skills://host".to_string(),
                updated_at_ms: 9,
                last_ingress_observation: Some(
                    crate::agent::capability_bridge::SourceIngressObservation {
                        boundary: "control_plane.apply_skill_source_snapshot".to_string(),
                        summary:
                            "skill source ingress registered `host-skills` with 1 skill candidates"
                                .to_string(),
                        candidate_ids: vec!["skill:search".to_string()],
                        observed_at_ms: 99,
                    },
                ),
            },
            skills: vec![crate::agent::capability_bridge::SkillDescriptor {
                skill_id: "skill:search".to_string(),
                source_id: "host-skills".to_string(),
                source_kind: crate::agent::capability_bridge::SkillSourceKind::Host,
                label: "search".to_string(),
                description: "Search workspace".to_string(),
                input_schema_summary: "{}".to_string(),
                safety_class: "".to_string(),
                visibility: "default".to_string(),
                observability_tags: vec!["host".to_string()],
                requires_approval: false,
                host_mediated: false,
                permission_scope: "".to_string(),
                composed_capability_refs: vec!["mcp:tool:workspace-search".to_string()],
                composed_capability_kinds: vec![
                    crate::agent::capability_bridge::CapabilityKind::Tool,
                ],
                executable_in_v1: true,
            }],
        });

        let reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let snapshots = reloaded.list_persisted_skill_source_snapshots();
        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].source.source_id, "host-skills");
        assert_eq!(
            snapshots[0]
                .source
                .last_ingress_observation
                .as_ref()
                .expect("ingress observation should persist")
                .candidate_ids,
            vec!["skill:search".to_string()]
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn file_backend_roundtrip_restores_checkpoint_persist_evidence() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);

        store.record_turn_trace(
            Some("trace-checkpoint-boundary"),
            TurnTraceRecord {
                turn_id: "turn-checkpoint-boundary".to_string(),
                session_id: Some("trace-checkpoint-boundary".to_string()),
                event_id: Some("turn-checkpoint-boundary:9".to_string()),
                event_type: Some("turn.completed".to_string()),
                event_version: Some("turn-event-v1".to_string()),
                sequence: Some(9),
                emitted_at_ms: Some(9009),
                title: "checkpoint boundary roundtrip".to_string(),
                phase: "completed".to_string(),
                trace_steps: vec![TurnTraceStep {
                    id: "step-return".to_string(),
                    label: "Return result".to_string(),
                    state: "completed".to_string(),
                }],
                trace_timeline: vec![
                    TraceTimelineEntry {
                        id: "return-1".to_string(),
                        kind: "return".to_string(),
                        label: "RETURN RESULT".to_string(),
                        state: "completed".to_string(),
                        sequence: 1,
                        provider_requested_name: Some("openai".to_string()),
                        provider_name: Some("openai".to_string()),
                        provider_protocol: Some("openai".to_string()),
                        provider_model: Some("gpt-5".to_string()),
                        provider_source: Some("provider_decision".to_string()),
                        provider_mode: Some("live".to_string()),
                        build_context_observation: None,
                        tool_activities: Vec::new(),
                        text: Some("final answer".to_string()),
                        reasoning_content: None,
                        fallback_reason: None,
                        error: None,
                        input_tokens: Some(11),
                        cache_hit_input_tokens: Some(4),
                        reasoning_tokens: Some(2),
                        output_tokens: Some(19),
                        total_tokens: Some(30),
                        first_token_latency_ms: Some(120),
                        turn_duration_ms: Some(860),
                    },
                    TraceTimelineEntry {
                        id: "checkpoint-2".to_string(),
                        kind: "checkpoint_persist".to_string(),
                        label: "PERSIST CHECKPOINT".to_string(),
                        state: "completed".to_string(),
                        sequence: 2,
                        provider_requested_name: Some("openai".to_string()),
                        provider_name: Some("openai".to_string()),
                        provider_protocol: Some("openai".to_string()),
                        provider_model: Some("gpt-5".to_string()),
                        provider_source: Some("provider_decision".to_string()),
                        provider_mode: Some("live".to_string()),
                        build_context_observation: None,
                        tool_activities: Vec::new(),
                        text: None,
                        reasoning_content: None,
                        fallback_reason: None,
                        error: None,
                        input_tokens: Some(11),
                        cache_hit_input_tokens: Some(4),
                        reasoning_tokens: Some(2),
                        output_tokens: Some(19),
                        total_tokens: Some(30),
                        first_token_latency_ms: Some(120),
                        turn_duration_ms: Some(860),
                    },
                ],
                tool_activities: Vec::new(),
                provider_call_records: Vec::new(),
                hook_trace_records: Vec::new(),
                provider_requested_name: Some("openai".to_string()),
                provider_name: Some("openai".to_string()),
                provider_protocol: Some("openai".to_string()),
                provider_model: Some("gpt-5".to_string()),
                provider_source: Some("provider_decision".to_string()),
                provider_mode: Some("live".to_string()),
                build_context_observation: None,
                session_summary: Some("checkpoint boundary summary".to_string()),
                fallback_reason: None,
                error: None,
                input_tokens: Some(11),
                cache_hit_input_tokens: Some(4),
                reasoning_tokens: Some(2),
                output_tokens: Some(19),
                total_tokens: Some(30),
                first_token_latency_ms: Some(120),
                turn_duration_ms: Some(860),
                updated_at: 0,
            },
        );

        let mut reloaded =
            SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let snapshot = reloaded.snapshot(Some("trace-checkpoint-boundary"), &[]);

        assert_eq!(snapshot.turn_trace_history.len(), 1);
        assert_eq!(
            snapshot.turn_trace_history[0].event_type.as_deref(),
            Some("turn.completed")
        );
        assert_eq!(snapshot.turn_trace_history[0].phase, "completed");
        assert_eq!(
            snapshot.turn_trace_history[0]
                .trace_timeline
                .last()
                .map(|entry| entry.kind.as_str()),
            Some("checkpoint_persist")
        );
        assert_eq!(
            snapshot.turn_trace_history[0]
                .trace_timeline
                .last()
                .map(|entry| entry.label.as_str()),
            Some("PERSIST CHECKPOINT")
        );
        assert_eq!(
            snapshot.turn_trace_history[0]
                .trace_timeline
                .last()
                .and_then(|entry| entry.turn_duration_ms),
            Some(860)
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn file_backend_roundtrip_restores_failed_terminal_envelope_and_existing_evidence() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);

        store.record_turn_trace(
            Some("trace-failed-terminal"),
            TurnTraceRecord {
                turn_id: "turn-failed-terminal".to_string(),
                session_id: Some("trace-failed-terminal".to_string()),
                event_id: Some("turn-failed-terminal:8".to_string()),
                event_type: Some("turn.failed".to_string()),
                event_version: Some("turn-event-v1".to_string()),
                sequence: Some(8),
                emitted_at_ms: Some(8008),
                title: "failed terminal roundtrip".to_string(),
                phase: "failed".to_string(),
                trace_steps: vec![TurnTraceStep {
                    id: "step-return".to_string(),
                    label: "Return result".to_string(),
                    state: "failed".to_string(),
                }],
                trace_timeline: vec![TraceTimelineEntry {
                    id: "return-1".to_string(),
                    kind: "return".to_string(),
                    label: "RETURN RESULT".to_string(),
                    state: "failed".to_string(),
                    sequence: 1,
                    provider_requested_name: Some("openai".to_string()),
                    provider_name: Some("openai".to_string()),
                    provider_protocol: Some("openai".to_string()),
                    provider_model: Some("gpt-5".to_string()),
                    provider_source: Some("provider_decision".to_string()),
                    provider_mode: Some("live".to_string()),
                    build_context_observation: None,
                    tool_activities: Vec::new(),
                    text: Some("failed terminal".to_string()),
                    reasoning_content: None,
                    fallback_reason: None,
                    error: Some("hook blocked finalize".to_string()),
                    input_tokens: Some(11),
                    cache_hit_input_tokens: Some(4),
                    reasoning_tokens: Some(2),
                    output_tokens: Some(0),
                    total_tokens: Some(13),
                    first_token_latency_ms: Some(120),
                    turn_duration_ms: Some(640),
                }],
                tool_activities: vec![TurnToolActivity {
                    id: "tool-1".to_string(),
                    name: "workspace_list_files".to_string(),
                    canonical_tool_name: Some("List".to_string()),
                    display_name_zh: Some("列表".to_string()),
                    status: "completed".to_string(),
                    description: "tool completed before finalize failed".to_string(),
                    arguments_text: Some("{\"path\":\".\"}".to_string()),
                    result_text: Some("ok".to_string()),
                    duration_seconds: Some(0.2),
                    parent_activity_id: None,
                    artifacts: None,
                    error: None,
                    capability_invocation: None,
                }],
                provider_call_records: vec![ProviderCallCacheRecord {
                    request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                    provider_source: Some("provider_decision".to_string()),
                    provider_mode: Some("live".to_string()),
                    input_tokens: Some(11),
                    cache_hit_input_tokens: Some(4),
                    cache_hit_source: None,
                    cache_miss_input_tokens: Some(7),
                    reasoning_tokens: Some(2),
                    output_tokens: Some(0),
                    total_tokens: Some(13),
                    first_token_latency_ms: Some(120),
                    turn_duration_ms: Some(640),
                    latency_kind: crate::agent::telemetry::ProviderLatencyKind::BufferedResponse,
                    prefix_mutation_reasons: vec![
                        crate::agent::provider::PrefixMutationReason::SessionSummaryChanged,
                    ],
                }],
                hook_trace_records: vec![HookTraceRecord {
                    hook_name: "observe.sync-finalize-failturn".to_string(),
                    hook_class: crate::agent::hooks::HookClass::Observe,
                    hook_point: crate::agent::hooks::TurnHookPoint::TurnFinalizeEnd,
                    hook_order: 1,
                    result_kind: crate::agent::hooks::HookResultKind::Deny,
                    structured_result: crate::agent::hooks::HookStructuredResult::Deny(
                        crate::agent::hooks::HookDenyDecision {
                            reason_code: "hook_blocked_finalize".to_string(),
                            message: "hook blocked finalize".to_string(),
                        },
                    ),
                    blocked: true,
                    elapsed_ms: 5,
                    input_summary: Some("failed".to_string()),
                    persistence_evidence_ref: Some(
                        "trace://turn-failed-terminal/finalize".to_string(),
                    ),
                    summary: "finalize hook blocked terminal".to_string(),
                }],
                provider_requested_name: Some("openai".to_string()),
                provider_name: Some("openai".to_string()),
                provider_protocol: Some("openai".to_string()),
                provider_model: Some("gpt-5".to_string()),
                provider_source: Some("provider_decision".to_string()),
                provider_mode: Some("live".to_string()),
                build_context_observation: None,
                session_summary: Some("failed summary".to_string()),
                fallback_reason: None,
                error: Some("hook blocked finalize".to_string()),
                input_tokens: Some(11),
                cache_hit_input_tokens: Some(4),
                reasoning_tokens: Some(2),
                output_tokens: Some(0),
                total_tokens: Some(13),
                first_token_latency_ms: Some(120),
                turn_duration_ms: Some(640),
                updated_at: 0,
            },
        );

        let mut reloaded =
            SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let snapshot = reloaded.snapshot(Some("trace-failed-terminal"), &[]);

        assert_eq!(snapshot.turn_trace_history.len(), 1);
        let trace = &snapshot.turn_trace_history[0];
        assert_eq!(trace.phase, "failed");
        assert_eq!(trace.event_id.as_deref(), Some("turn-failed-terminal:8"));
        assert_eq!(trace.event_type.as_deref(), Some("turn.failed"));
        assert_eq!(trace.event_version.as_deref(), Some("turn-event-v1"));
        assert_eq!(trace.sequence, Some(8));
        assert_eq!(trace.emitted_at_ms, Some(8008));
        assert_eq!(trace.error.as_deref(), Some("hook blocked finalize"));
        assert_eq!(trace.provider_call_records.len(), 1);
        assert_eq!(
            trace.provider_call_records[0].request_kind,
            crate::agent::telemetry::ProviderRequestKind::InitialRequest
        );
        assert_eq!(trace.tool_activities.len(), 1);
        assert_eq!(trace.tool_activities[0].name, "workspace_list_files");
        assert_eq!(trace.hook_trace_records.len(), 1);
        assert!(trace.hook_trace_records[0].blocked);
        assert_eq!(
            trace.hook_trace_records[0]
                .persistence_evidence_ref
                .as_deref(),
            Some("trace://turn-failed-terminal/finalize")
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn file_backend_roundtrip_persists_failed_turn_into_visible_history() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);

        store.append_failed_turn(
            Some("failed-visible-history"),
            "请继续排查 session 跳转问题",
            "hook blocked finalize",
            TurnTraceRecord {
                turn_id: "turn-failed-visible-history".to_string(),
                session_id: Some("failed-visible-history".to_string()),
                event_id: None,
                event_type: None,
                event_version: None,
                sequence: None,
                emitted_at_ms: None,
                title: "failed visible history".to_string(),
                phase: "failed".to_string(),
                trace_steps: vec![TurnTraceStep {
                    id: "step-return".to_string(),
                    label: "Return result".to_string(),
                    state: "failed".to_string(),
                }],
                trace_timeline: vec![TraceTimelineEntry {
                    id: "return-1".to_string(),
                    kind: "return".to_string(),
                    label: "RETURN RESULT".to_string(),
                    state: "failed".to_string(),
                    sequence: 1,
                    provider_requested_name: None,
                    provider_name: None,
                    provider_protocol: None,
                    provider_model: None,
                    provider_source: None,
                    provider_mode: None,
                    build_context_observation: None,
                    tool_activities: Vec::new(),
                    text: Some("hook blocked finalize".to_string()),
                    reasoning_content: None,
                    fallback_reason: None,
                    error: Some("hook blocked finalize".to_string()),
                    input_tokens: None,
                    cache_hit_input_tokens: None,
                    reasoning_tokens: None,
                    output_tokens: None,
                    total_tokens: None,
                    first_token_latency_ms: None,
                    turn_duration_ms: None,
                }],
                tool_activities: Vec::new(),
                provider_call_records: Vec::new(),
                hook_trace_records: Vec::new(),
                provider_requested_name: None,
                provider_name: None,
                provider_protocol: None,
                provider_model: None,
                provider_source: None,
                provider_mode: None,
                build_context_observation: None,
                session_summary: Some("hook blocked finalize".to_string()),
                fallback_reason: None,
                error: Some("hook blocked finalize".to_string()),
                input_tokens: None,
                cache_hit_input_tokens: None,
                reasoning_tokens: None,
                output_tokens: None,
                total_tokens: None,
                first_token_latency_ms: None,
                turn_duration_ms: None,
                updated_at: 0,
            },
        );

        let mut reloaded =
            SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let snapshot = reloaded.snapshot(Some("failed-visible-history"), &[]);

        assert_eq!(snapshot.turn_count, 1);
        assert_eq!(snapshot.history.len(), 2);
        assert_eq!(snapshot.history[0].role, "user");
        assert_eq!(snapshot.history[0].content, "请继续排查 session 跳转问题");
        assert_eq!(snapshot.history[1].role, "assistant");
        assert_eq!(snapshot.history[1].content, "hook blocked finalize");
        assert_eq!(snapshot.turn_trace_history.len(), 1);
        assert_eq!(
            snapshot.turn_trace_history[0].error.as_deref(),
            Some("hook blocked finalize")
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn file_backend_roundtrip_restores_cancelled_terminal_envelope_and_existing_evidence() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);

        store.record_turn_trace(
            Some("trace-cancelled-terminal"),
            TurnTraceRecord {
                turn_id: "turn-cancelled-terminal".to_string(),
                session_id: Some("trace-cancelled-terminal".to_string()),
                event_id: Some("turn-cancelled-terminal:6".to_string()),
                event_type: Some("turn.cancelled".to_string()),
                event_version: Some("turn-event-v1".to_string()),
                sequence: Some(6),
                emitted_at_ms: Some(6006),
                title: "cancelled terminal roundtrip".to_string(),
                phase: "cancelled".to_string(),
                trace_steps: vec![TurnTraceStep {
                    id: "step-call-tool".to_string(),
                    label: "Call tool".to_string(),
                    state: "completed".to_string(),
                }],
                trace_timeline: vec![TraceTimelineEntry {
                    id: "tool-1".to_string(),
                    kind: "tool".to_string(),
                    label: "CALL TOOL #1".to_string(),
                    state: "completed".to_string(),
                    sequence: 1,
                    provider_requested_name: Some("openai".to_string()),
                    provider_name: Some("openai".to_string()),
                    provider_protocol: Some("openai".to_string()),
                    provider_model: Some("gpt-5".to_string()),
                    provider_source: Some("provider_decision".to_string()),
                    provider_mode: Some("live".to_string()),
                    build_context_observation: None,
                    tool_activities: vec![TurnToolActivity {
                        id: "tool-1".to_string(),
                        name: "workspace_list_files".to_string(),
                        canonical_tool_name: Some("List".to_string()),
                        display_name_zh: Some("列表".to_string()),
                        status: "completed".to_string(),
                        description: "tool completed before cancel".to_string(),
                        arguments_text: Some("{\"path\":\".\"}".to_string()),
                        result_text: Some("ok".to_string()),
                        duration_seconds: Some(0.2),
                        parent_activity_id: None,
                        artifacts: None,
                        error: None,
                        capability_invocation: None,
                    }],
                    text: None,
                    reasoning_content: None,
                    fallback_reason: None,
                    error: None,
                    input_tokens: Some(9),
                    cache_hit_input_tokens: Some(3),
                    reasoning_tokens: Some(1),
                    output_tokens: Some(0),
                    total_tokens: Some(10),
                    first_token_latency_ms: Some(90),
                    turn_duration_ms: Some(510),
                }],
                tool_activities: vec![TurnToolActivity {
                    id: "tool-1".to_string(),
                    name: "workspace_list_files".to_string(),
                    canonical_tool_name: Some("List".to_string()),
                    display_name_zh: Some("列表".to_string()),
                    status: "completed".to_string(),
                    description: "tool completed before cancel".to_string(),
                    arguments_text: Some("{\"path\":\".\"}".to_string()),
                    result_text: Some("ok".to_string()),
                    duration_seconds: Some(0.2),
                    parent_activity_id: None,
                    artifacts: None,
                    error: None,
                    capability_invocation: None,
                }],
                provider_call_records: vec![ProviderCallCacheRecord {
                    request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                    provider_source: Some("provider_decision".to_string()),
                    provider_mode: Some("live".to_string()),
                    input_tokens: Some(9),
                    cache_hit_input_tokens: Some(3),
                    cache_hit_source: None,
                    cache_miss_input_tokens: Some(6),
                    reasoning_tokens: Some(1),
                    output_tokens: Some(0),
                    total_tokens: Some(10),
                    first_token_latency_ms: Some(90),
                    turn_duration_ms: Some(510),
                    latency_kind: crate::agent::telemetry::ProviderLatencyKind::ProviderStream,
                    prefix_mutation_reasons: vec![
                        crate::agent::provider::PrefixMutationReason::HistoryBoundaryShifted,
                    ],
                }],
                hook_trace_records: vec![HookTraceRecord {
                    hook_name: "observe.cancelled-finalize".to_string(),
                    hook_class: crate::agent::hooks::HookClass::Observe,
                    hook_point: crate::agent::hooks::TurnHookPoint::TurnFinalizeEnd,
                    hook_order: 1,
                    result_kind: crate::agent::hooks::HookResultKind::Observe,
                    structured_result: crate::agent::hooks::HookStructuredResult::Observe {
                        summary: "cancelled terminal observed".to_string(),
                    },
                    blocked: false,
                    elapsed_ms: 3,
                    input_summary: Some("cancelled".to_string()),
                    persistence_evidence_ref: Some(
                        "trace://turn-cancelled-terminal/finalize".to_string(),
                    ),
                    summary: "cancelled hook summary".to_string(),
                }],
                provider_requested_name: Some("openai".to_string()),
                provider_name: Some("openai".to_string()),
                provider_protocol: Some("openai".to_string()),
                provider_model: Some("gpt-5".to_string()),
                provider_source: Some("provider_decision".to_string()),
                provider_mode: Some("live".to_string()),
                build_context_observation: None,
                session_summary: Some("cancelled summary".to_string()),
                fallback_reason: Some("stopped_by_user".to_string()),
                error: Some("stopped_by_user".to_string()),
                input_tokens: Some(9),
                cache_hit_input_tokens: Some(3),
                reasoning_tokens: Some(1),
                output_tokens: Some(0),
                total_tokens: Some(10),
                first_token_latency_ms: Some(90),
                turn_duration_ms: Some(510),
                updated_at: 0,
            },
        );

        let mut reloaded =
            SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let snapshot = reloaded.snapshot(Some("trace-cancelled-terminal"), &[]);

        assert_eq!(snapshot.turn_trace_history.len(), 1);
        let trace = &snapshot.turn_trace_history[0];
        assert_eq!(trace.phase, "cancelled");
        assert_eq!(trace.event_id.as_deref(), Some("turn-cancelled-terminal:6"));
        assert_eq!(trace.event_type.as_deref(), Some("turn.cancelled"));
        assert_eq!(trace.event_version.as_deref(), Some("turn-event-v1"));
        assert_eq!(trace.sequence, Some(6));
        assert_eq!(trace.emitted_at_ms, Some(6006));
        assert_eq!(trace.error.as_deref(), Some("stopped_by_user"));
        assert_eq!(trace.fallback_reason.as_deref(), Some("stopped_by_user"));
        assert_eq!(trace.provider_call_records.len(), 1);
        assert_eq!(
            trace.provider_call_records[0].request_kind,
            crate::agent::telemetry::ProviderRequestKind::InitialRequest
        );
        assert_eq!(trace.tool_activities.len(), 1);
        assert_eq!(trace.tool_activities[0].status, "completed");
        assert_eq!(trace.hook_trace_records.len(), 1);
        assert_eq!(
            trace.hook_trace_records[0]
                .persistence_evidence_ref
                .as_deref(),
            Some("trace://turn-cancelled-terminal/finalize")
        );

        let _ = fs::remove_file(path);
    }

    #[test]
    fn history_checkout_can_rehydrate_older_node_and_degrade_workspace_restore() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("history-session"),
            "第一问",
            "第一答",
            None,
            Vec::new(),
        );
        store.append_turn(
            Some("history-session"),
            "第二问",
            "第二答",
            None,
            Vec::new(),
        );

        let (nodes, branches, _) = store.load_history_graph(Some("history-session"));
        assert_eq!(nodes.len(), 3);
        assert_eq!(branches.len(), 1);

        let snapshot = store
            .checkout_history_node(
                Some("history-session"),
                &nodes[1].node_id,
                HistoryCheckoutMode::TranscriptAndWorkspace,
                None,
            )
            .expect("history checkout should succeed");

        assert_eq!(
            snapshot.resolved_node_id.as_deref(),
            Some(nodes[1].node_id.as_str())
        );
        assert_eq!(snapshot.history.len(), 2);
        assert_eq!(snapshot.history[0].content, "第一问");
        assert_eq!(snapshot.history[1].content, "第一答");
        assert_eq!(
            snapshot.latest_node_id.as_deref(),
            Some(nodes[1].node_id.as_str())
        );
        // checkout_history_node truncates descendants and makes the target
        // node the branch head, so mode is Live (d6e1fbf semantics).
        assert_eq!(snapshot.history_cursor.mode, HistoryCursorMode::Live);
        assert_eq!(
            snapshot.history_cursor.checkout_status,
            HistoryCheckoutStatus::DegradedToTranscriptOnly
        );
    }

    #[test]
    fn appending_from_historical_node_creates_a_fork_and_preserves_main_branch_head() {
        let mut store = SessionStore::memory_only();
        store.append_turn(Some("fork-session"), "第一问", "第一答", None, Vec::new());
        store.append_turn(Some("fork-session"), "第二问", "第二答", None, Vec::new());

        let (nodes_before, branches_before, _) = store.load_history_graph(Some("fork-session"));
        let main_head_before = branches_before[0].head_node_id.clone();
        let fork_base = nodes_before[0].node_id.clone();

        store
            .checkout_history_node(
                Some("fork-session"),
                fork_base.as_str(),
                HistoryCheckoutMode::TranscriptOnly,
                None,
            )
            .expect("checkout should succeed");

        let fork_snapshot = store.append_turn(
            Some("fork-session"),
            "历史节点上继续追问",
            "分叉后的回答",
            None,
            Vec::new(),
        );

        let (nodes_after, branches_after, cursor_after) =
            store.load_history_graph(Some("fork-session"));
        assert_eq!(nodes_after.len(), 4);
        assert_eq!(branches_after.len(), 2);
        assert_eq!(cursor_after.mode, HistoryCursorMode::Live);

        let main_branch = branches_after
            .iter()
            .find(|branch| branch.branch_id == DEFAULT_HISTORY_BRANCH_ID)
            .expect("main branch should exist");
        assert_eq!(main_branch.head_node_id, main_head_before);

        let fork_branch = branches_after
            .iter()
            .find(|branch| branch.branch_id != DEFAULT_HISTORY_BRANCH_ID)
            .expect("fork branch should be created");
        assert_eq!(
            fork_branch.base_node_id.as_deref(),
            Some(fork_base.as_str())
        );
        assert_eq!(
            fork_branch.forked_from_node_id.as_deref(),
            Some(fork_base.as_str())
        );
        assert_eq!(
            fork_snapshot.history_cursor.active_branch_id.as_deref(),
            Some(fork_branch.branch_id.as_str())
        );

        let fork_head = nodes_after
            .iter()
            .find(|node| node.branch_id == fork_branch.branch_id)
            .expect("fork branch head node should exist");
        assert_eq!(
            fork_head.parent_node_id.as_deref(),
            Some(fork_base.as_str())
        );
    }

    #[test]
    fn restore_and_switch_history_branch_move_cursor_between_branch_heads() {
        let mut store = SessionStore::memory_only();
        store.append_turn(Some("switch-session"), "第一问", "第一答", None, Vec::new());
        store.append_turn(Some("switch-session"), "第二问", "第二答", None, Vec::new());

        let (nodes_before, branches_before, _) = store.load_history_graph(Some("switch-session"));
        let first_node_id = nodes_before[0].node_id.clone();
        // The graph includes a checkpoint root node before the two turns, so
        // the main branch head is taken from the branch record (the last turn
        // node), not nodes_before[1].
        let second_node_id = branches_before[0]
            .head_node_id
            .clone()
            .expect("main branch head node should exist");

        store
            .fork_from_history_node(Some("switch-session"), first_node_id.as_str(), None)
            .expect("fork should succeed");
        store.append_turn(
            Some("switch-session"),
            "在分叉上继续",
            "分叉回答",
            None,
            Vec::new(),
        );

        let (_, branches_after_fork, _) = store.load_history_graph(Some("switch-session"));
        let fork_branch_id = branches_after_fork
            .iter()
            .find(|branch| branch.branch_id != DEFAULT_HISTORY_BRANCH_ID)
            .map(|branch| branch.branch_id.clone())
            .expect("fork branch should exist");

        let switched = store
            .switch_history_branch(Some("switch-session"), DEFAULT_HISTORY_BRANCH_ID, None)
            .expect("switch to main branch should succeed");
        assert_eq!(
            switched.resolved_node_id.as_deref(),
            Some(second_node_id.as_str())
        );
        assert_eq!(switched.history_cursor.mode, HistoryCursorMode::Live);

        let restored = store
            .restore_branch_head(Some("switch-session"), Some(fork_branch_id.as_str()), None)
            .expect("restore fork branch head should succeed");
        assert_eq!(
            restored.history_cursor.active_branch_id.as_deref(),
            Some(fork_branch_id.as_str())
        );
        assert_eq!(restored.history_cursor.mode, HistoryCursorMode::Live);
        assert_ne!(
            restored.resolved_node_id.as_deref(),
            Some(second_node_id.as_str())
        );
    }

    #[test]
    fn session_state_for_backend_strips_node_traces_and_writes_refs() {
        // PA-088：WriteSeparate + Authoritative 时，持久化副本剥离节点 trace 并生成 refs；
        // 内存副本（原 session）不受影响。
        let mut session = SessionState {
            conversation_id: "s1".to_string(),
            title: "t".to_string(),
            summary: "s".to_string(),
            history: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: vec![
                TurnTraceRecord {
                    turn_id: "turn-1".to_string(),
                    updated_at: 100,
                    ..TurnTraceRecord::default()
                },
                TurnTraceRecord {
                    turn_id: "turn-2".to_string(),
                    updated_at: 200,
                    ..TurnTraceRecord::default()
                },
            ],
            trace_migration_state: TraceMigrationState::TraceTableAuthoritative,
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 2,
            last_referenced_file: None,
            updated_at_ms: 300,
            history_nodes: vec![HistoryNode {
                node_id: "node-1".to_string(),
                session_id: "s1".to_string(),
                parent_node_id: None,
                branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                forked_from_node_id: None,
                kind: HistoryNodeKind::TurnCommitted,
                run_id: None,
                workspace_ref: WorkspaceRef::default(),
                summary: "s".to_string(),
                title: "t".to_string(),
                history: Vec::new(),
                provider_native_transcript: Vec::new(),
                turn_trace_history: vec![TurnTraceRecord {
                    turn_id: "turn-2".to_string(),
                    updated_at: 200,
                    ..TurnTraceRecord::default()
                }],
                turn_id: None,
                turn_trace_refs: None,
                long_term_memory_entries: Vec::new(),
                memory_write_evidence: Vec::new(),
                memory_write_hook_trace_records: Vec::new(),
                turn_count: 2,
                last_referenced_file: None,
                created_at_ms: 250,
            event_seq_range: None,
            }],
            history_branches: Vec::new(),
            history_cursor: HistoryCursor::default(),
            workspace_id: None,
            event_watermark: 0,
            last_commit_watermark: 0,
        };

        let prepared = session_state_for_backend(&session, SeparateTraceTableMode::WriteSeparate);

        // 持久化副本：顶层 + 节点 trace 被剥离，refs 生成
        assert!(prepared.turn_trace_history.is_empty());
        let top_refs = prepared.turn_trace_refs.expect("top-level refs present");
        assert_eq!(top_refs.len(), 2);
        assert_eq!(top_refs[0].turn_id, "turn-1");
        assert_eq!(top_refs[1].turn_id, "turn-2");
        let node = &prepared.history_nodes[0];
        assert!(node.turn_trace_history.is_empty());
        assert_eq!(node.turn_id.as_deref(), Some("turn-2"));
        let node_refs = node.turn_trace_refs.as_ref().expect("node refs present");
        assert_eq!(node_refs.len(), 1);
        assert_eq!(node_refs[0].turn_id, "turn-2");

        // 内存副本：完整 trace 保留（运行中 checkout 语义不变）
        assert_eq!(session.turn_trace_history.len(), 2);
        assert_eq!(session.history_nodes[0].turn_trace_history.len(), 1);
        assert!(session.turn_trace_refs.is_none());
    }

    #[test]
    fn session_state_for_backend_keeps_legacy_blob_intact_under_write_separate() {
        // PA-088：WriteSeparate + LegacyBlob 存量会话不晋升、不剥离（保持可读写）。
        let session = SessionState {
            conversation_id: "s1".to_string(),
            title: "t".to_string(),
            summary: "s".to_string(),
            history: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: vec![TurnTraceRecord {
                turn_id: "turn-1".to_string(),
                updated_at: 100,
                ..TurnTraceRecord::default()
            }],
            trace_migration_state: TraceMigrationState::LegacyBlob,
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 1,
            last_referenced_file: None,
            updated_at_ms: 100,
            history_nodes: Vec::new(),
            history_branches: Vec::new(),
            history_cursor: HistoryCursor::default(),
            workspace_id: None,
            event_watermark: 0,
            last_commit_watermark: 0,
        };

        let prepared = session_state_for_backend(&session, SeparateTraceTableMode::WriteSeparate);

        assert_eq!(
            prepared.trace_migration_state,
            TraceMigrationState::LegacyBlob,
            "legacy 会话不自动晋升"
        );
        assert_eq!(prepared.turn_trace_history.len(), 1, "blob trace 保留");
        assert!(prepared.turn_trace_refs.is_none());
    }

    #[test]
    fn collect_trace_union_merges_top_level_and_node_traces() {
        // PA-088 P0-1 回归：写表用全量 Union（顶层 ∪ 节点 trace），
        // 保证 >24 轮会话的旧节点 refs 在重启 materialize 时可解析。
        let session = SessionState {
            conversation_id: "s1".to_string(),
            title: "t".to_string(),
            summary: "s".to_string(),
            history: Vec::new(),
            provider_native_transcript: Vec::new(),
            turn_trace_history: vec![TurnTraceRecord {
                turn_id: "turn-1".to_string(),
                updated_at: 100,
                ..TurnTraceRecord::default()
            }],
            trace_migration_state: TraceMigrationState::TraceTableAuthoritative,
            turn_trace_refs: None,
            long_term_memory_entries: Vec::new(),
            memory_write_evidence: Vec::new(),
            memory_write_hook_trace_records: Vec::new(),
            history_state_evidence: Vec::new(),
            turn_count: 3,
            last_referenced_file: None,
            updated_at_ms: 300,
            history_nodes: vec![
                HistoryNode {
                    node_id: "node-1".to_string(),
                    session_id: "s1".to_string(),
                    parent_node_id: None,
                    branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                    forked_from_node_id: None,
                    kind: HistoryNodeKind::TurnCommitted,
                    run_id: None,
                    workspace_ref: WorkspaceRef::default(),
                    summary: "s".to_string(),
                    title: "t".to_string(),
                    history: Vec::new(),
                    provider_native_transcript: Vec::new(),
                    turn_trace_history: vec![TurnTraceRecord {
                        turn_id: "turn-1".to_string(),
                        updated_at: 100,
                        ..TurnTraceRecord::default()
                    }],
                    turn_id: Some("turn-1".to_string()),
                    turn_trace_refs: None,
                    long_term_memory_entries: Vec::new(),
                    memory_write_evidence: Vec::new(),
                    memory_write_hook_trace_records: Vec::new(),
                    turn_count: 1,
                    last_referenced_file: None,
                    created_at_ms: 100,
                event_seq_range: None,
                },
                HistoryNode {
                    node_id: "node-2".to_string(),
                    session_id: "s1".to_string(),
                    parent_node_id: Some("node-1".to_string()),
                    branch_id: DEFAULT_HISTORY_BRANCH_ID.to_string(),
                    forked_from_node_id: None,
                    kind: HistoryNodeKind::TurnCommitted,
                    run_id: None,
                    workspace_ref: WorkspaceRef::default(),
                    summary: "s".to_string(),
                    title: "t".to_string(),
                    history: Vec::new(),
                    provider_native_transcript: Vec::new(),
                    // 节点 2 引用了顶层已淘汰的 turn-2（>24 轮场景）
                    turn_trace_history: vec![TurnTraceRecord {
                        turn_id: "turn-2".to_string(),
                        updated_at: 200,
                        ..TurnTraceRecord::default()
                    }],
                    turn_id: Some("turn-2".to_string()),
                    turn_trace_refs: None,
                    long_term_memory_entries: Vec::new(),
                    memory_write_evidence: Vec::new(),
                    memory_write_hook_trace_records: Vec::new(),
                    turn_count: 2,
                    last_referenced_file: None,
                    created_at_ms: 200,
                event_seq_range: None,
                },
            ],
            history_branches: Vec::new(),
            history_cursor: HistoryCursor::default(),
            workspace_id: None,
            event_watermark: 0,
            last_commit_watermark: 0,
        };

        let union = collect_trace_union(&session);
        let mut turn_ids: Vec<String> = union.iter().map(|trace| trace.turn_id.clone()).collect();
        turn_ids.sort();
        assert_eq!(turn_ids, vec!["turn-1".to_string(), "turn-2".to_string()]);

    }
    // ── PA-093 checkpoint 引用化（阶段 3）测试 ──

    fn pa093_sqlite_store(tag: &str) -> (SessionStore, std::path::PathBuf, String) {
        use crate::agent::sqlite_session::SqliteSessionBackend;
        use crate::agent::session::SeparateTraceTableMode;
        let dir = std::env::temp_dir().join(format!(
            "pa093-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let db_path = dir.join("test.db");
        let backend = Box::new(SqliteSessionBackend::new_with_trace_mode(
            db_path.clone(),
            SeparateTraceTableMode::WriteSeparate,
        ));
        let session_id = format!("pa093-{tag}");
        (SessionStore::with_backend(backend), dir, session_id)
    }

    fn pa093_flush(
        store: &mut SessionStore, session_id: &str, turn_id: &str,
        branch_id: &str, events: &[TurnEvent],
    ) {
        let branch = branch_id.to_string();
        assert!(store.persist_events(session_id, turn_id, &branch, events.to_vec()));
        store.finalize_event_watermark(session_id, turn_id);
    }

    fn pa093_turn_events(turn_id: &str, user_text: &str, assistant_text: &str) -> Vec<TurnEvent> {
        vec![
            TurnEvent::TurnStart { turn_id: turn_id.to_string() },
            TurnEvent::UserMessage { turn_id: turn_id.to_string(), text: user_text.to_string(), attachments: Vec::new() },
            TurnEvent::AssistantMessage { turn_id: turn_id.to_string(), step: 0, text: assistant_text.to_string(), reasoning_content: None, usage: None, chunk_missing: None },
            TurnEvent::TurnEnd { turn_id: turn_id.to_string(), reason: TurnEndReason::Completed, turn_duration_ms: None },
        ]
    }

    fn pa093_first_turn_node_id(store: &SessionStore, session_id: &str) -> String {
        let session = store.sessions.get(session_id).expect("session");
        session.history_nodes.iter().find(|n| n.kind == HistoryNodeKind::TurnCommitted).expect("turn node").node_id.clone()
    }

    #[test]
    fn pa093_commit_finalize_upgrades_node_to_referenced() {
        let (mut store, dir, sid) = pa093_sqlite_store("finalize");
        store.append_turn(Some(&sid), "first", "reply1", None, Vec::new());
        let head = {
            let s = store.sessions.get(&sid).expect("session");
            s.history_cursor.branch_head_node_id.clone().expect("head")
        };
        let s = store.sessions.get(&sid).expect("session");
        let node = s.history_nodes.iter().find(|n| n.node_id == head).expect("node");
        assert!(node.event_seq_range.is_none(), "pre-flush legacy");
        assert_eq!(node.history.len(), 2, "pre-flush keeps snapshot");
        drop(s);
        pa093_flush(&mut store, &sid, "turn-1", "branch-main", &pa093_turn_events("turn-1", "first", "reply1"));
        let s = store.sessions.get(&sid).expect("session");
        let node = s.history_nodes.iter().find(|n| n.node_id == head).expect("node");
        assert_eq!(node.event_seq_range, Some((0, 3)), "range covered");
        assert!(node.history.is_empty(), "snapshot cleared");
        assert!(node.turn_trace_history.is_empty(), "trace cleared");
        assert_eq!(s.event_watermark, 4);
        assert_eq!(s.last_commit_watermark, 4);
        drop(s);
        store.append_turn(Some(&sid), "second", "reply2", None, Vec::new());
        pa093_flush(&mut store, &sid, "turn-2", "branch-main", &pa093_turn_events("turn-2", "second", "reply2"));
        let s = store.sessions.get(&sid).expect("session");
        let nodes = &s.history_nodes;
        assert_eq!(nodes[nodes.len() - 1].event_seq_range, Some((4, 7)), "contiguous range");
        drop(s);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pa093_checkout_referenced_node_folds_events() {
        let (mut store, dir, sid) = pa093_sqlite_store("checkout");
        store.append_turn(Some(&sid), "first", "reply1", None, Vec::new());
        pa093_flush(&mut store, &sid, "turn-1", "branch-main", &pa093_turn_events("turn-1", "first", "reply1"));
        store.append_turn(Some(&sid), "second", "reply2", None, Vec::new());
        pa093_flush(&mut store, &sid, "turn-2", "branch-main", &pa093_turn_events("turn-2", "second", "reply2"));
        let node1 = pa093_first_turn_node_id(&store, &sid);
        let snapshot = store.checkout_history_node(Some(&sid), &node1, HistoryCheckoutMode::TranscriptOnly, None).expect("checkout");
        assert_eq!(snapshot.history.len(), 2, "folded view has turn 1 only");
        assert!(snapshot.history.iter().all(|m| m.content != "reply2"), "watermark rollback");
        assert_eq!(snapshot.history_cursor.event_watermark, 8, "cursor watermark");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pa093_restore_branch_head_folds_referenced_branch() {
        // P1-1：restore_branch_head 对引用化分支头节点的折叠路径
        let (mut store, dir, sid) = pa093_sqlite_store("restore");
        store.append_turn(Some(&sid), "first", "reply1", None, Vec::new());
        pa093_flush(&mut store, &sid, "turn-1", "branch-main", &pa093_turn_events("turn-1", "first", "reply1"));
        let node1 = pa093_first_turn_node_id(&store, &sid);
        let fork_snapshot = store.fork_from_history_node(Some(&sid), &node1, None).expect("fork");
        let fork_branch = fork_snapshot.history_cursor.active_branch_id.expect("active branch");
        store.append_turn(Some(&sid), "fork-q", "fork-answer", None, Vec::new());
        pa093_flush(&mut store, &sid, "fork-turn", &fork_branch, &pa093_turn_events("fork-turn", "fork-q", "fork-answer"));
        let restored = store.restore_branch_head(Some(&sid), Some(&fork_branch), None).expect("restore");
        assert!(restored.history.iter().any(|m| m.content == "fork-answer"), "restore folds fork events");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pa093_fork_from_referenced_node_folds_source_view() {
        // P1-1：fork_from_history_node 对引用化源节点的折叠路径
        let (mut store, dir, sid) = pa093_sqlite_store("fork-fold");
        store.append_turn(Some(&sid), "first", "reply1", None, Vec::new());
        pa093_flush(&mut store, &sid, "turn-1", "branch-main", &pa093_turn_events("turn-1", "first", "reply1"));
        let node1 = pa093_first_turn_node_id(&store, &sid);
        let fork_snapshot = store.fork_from_history_node(Some(&sid), &node1, None).expect("fork");
        assert_eq!(fork_snapshot.history.len(), 2, "fork view folds source node events");
        assert_eq!(fork_snapshot.history[0].content, "first");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pa093_checkout_legacy_node_falls_back_to_snapshot() {
        let (mut store, dir, sid) = pa093_sqlite_store("legacy");
        store.append_turn(Some(&sid), "first", "reply1", None, Vec::new());
        store.append_turn(Some(&sid), "second", "reply2", None, Vec::new());
        let node1 = pa093_first_turn_node_id(&store, &sid);
        let snapshot = store.checkout_history_node(Some(&sid), &node1, HistoryCheckoutMode::TranscriptOnly, None).expect("legacy checkout");
        assert_eq!(snapshot.history.len(), 2, "legacy snapshot");
        assert!(snapshot.history.iter().all(|m| m.content != "reply2"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pa093_time_travel_loads_referenced_node_state() {
        let (mut store, dir, sid) = pa093_sqlite_store("travel");
        store.append_turn(Some(&sid), "first", "reply1", None, Vec::new());
        pa093_flush(&mut store, &sid, "turn-1", "branch-main", &pa093_turn_events("turn-1", "first", "reply1"));
        store.append_turn(Some(&sid), "second", "reply2", None, Vec::new());
        pa093_flush(&mut store, &sid, "turn-2", "branch-main", &pa093_turn_events("turn-2", "second", "reply2"));
        let node1 = pa093_first_turn_node_id(&store, &sid);
        let snapshot = store.snapshot_for_session_at(&sid, Some(&node1));
        assert_eq!(snapshot.history.len(), 2, "traveled = node1");
        assert!(snapshot.history.iter().all(|m| m.content != "reply2"));
        let current = store.snapshot_for_session(&sid);
        assert_eq!(current.history.len(), 4, "live untouched");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pa093_fork_visibility_filters_events() {
        let (mut store, dir, sid) = pa093_sqlite_store("fork-vis");
        store.append_turn(Some(&sid), "first", "reply1", None, Vec::new());
        pa093_flush(&mut store, &sid, "turn-1", "branch-main", &pa093_turn_events("turn-1", "first", "reply1"));
        let node1 = pa093_first_turn_node_id(&store, &sid);
        let fork_snapshot = store.fork_from_history_node(Some(&sid), &node1, None).expect("fork");
        let fork_branch = fork_snapshot.history_cursor.active_branch_id.expect("active branch");
        assert_ne!(fork_branch, "branch-main");
        store.append_turn(Some(&sid), "fork-q", "fork-answer", None, Vec::new());
        pa093_flush(&mut store, &sid, "fork-turn", &fork_branch, &pa093_turn_events("fork-turn", "fork-q", "fork-answer"));
        let main_snapshot = store.switch_history_branch(Some(&sid), "branch-main", None).expect("switch to main");
        assert!(main_snapshot.history.iter().all(|m| m.content != "fork-answer"), "fork invisible after switch");
        let fork_again = store.switch_history_branch(Some(&sid), &fork_branch, None).expect("switch back");
        assert!(fork_again.history.iter().any(|m| m.content == "fork-answer"), "fork visible again");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn pa093_fold_10k_events_stays_under_budget() {
        let mut events = Vec::with_capacity(10_000);
        for i in 0..1_000u32 {
            let tid = format!("t{i}");
            events.push(("branch-main".to_string(), TurnEvent::TurnStart { turn_id: tid.clone() }));
            events.push(("branch-main".to_string(), TurnEvent::UserMessage { turn_id: tid.clone(), text: format!("q{i}"), attachments: Vec::new() }));
            events.push(("branch-main".to_string(), TurnEvent::AssistantMessage { turn_id: tid.clone(), step: 0, text: format!("a{i}"), reasoning_content: None, usage: None, chunk_missing: None }));
            events.push(("branch-main".to_string(), TurnEvent::TurnEnd { turn_id: tid.clone(), reason: TurnEndReason::Completed, turn_duration_ms: None }));
        }
        let with_seq: Vec<(u64, String, TurnEvent)> = events.into_iter().enumerate().map(|(seq, (b, e))| (seq as u64, b, e)).collect();
        let started = std::time::Instant::now();
        let (history, _) = fold_session_views(&with_seq, "branch-main", &[], &[]);
        let elapsed = started.elapsed();
        assert_eq!(history.len(), 24, "window truncated");
        let budget = if cfg!(debug_assertions) { 2000 } else { 100 };
        assert!(elapsed.as_millis() < budget, "10k refold took {}ms", elapsed.as_millis());
    }
}