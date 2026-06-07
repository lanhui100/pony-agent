use crate::agent::hooks::{
    merge_patch_results, HistoryStateCommandKind, HistoryStateCursorSummary,
    HistoryStateHookEnvelope, HistoryStateHookEvidence, HistoryStateHookExecutor,
    HistoryStateHookPoint, HookPatchConflictPolicy, HookPatchOperationKind, HookPatchTarget,
    HookResultKind, HookStructuredResult, HookTraceRecord, MemoryWriteHookEnvelope,
    MemoryWriteHookExecutor, MemoryWriteHookPoint, MemoryWriteIntentRecord,
    MemoryWriteOperation, MemoryWriteTarget, NoopHistoryStateHookExecutor,
    NoopMemoryWriteHookExecutor, PersistedEffectEvidence,
};
use crate::agent::capability_bridge::{McpSourceSnapshot, SkillSourceSnapshot};
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
    pub mode: HistoryCursorMode,
    #[serde(default)]
    pub checkout_mode: HistoryCheckoutMode,
    #[serde(default)]
    pub checkout_status: HistoryCheckoutStatus,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnHistoryMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentReference>,
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

pub trait SessionBackend: Send {
    fn load_store(&self) -> Option<PersistedStore>;
    fn save_store(&self, store: &PersistedStore);
    fn attachment_root(&self) -> Option<PathBuf>;
}

pub struct SessionStore {
    sessions: SessionMap,
    attachment_assets: AttachmentAssetMap,
    session_attachment_index: SessionAttachmentIndex,
    mcp_source_snapshots: HashMap<String, McpSourceSnapshot>,
    skill_source_snapshots: HashMap<String, SkillSourceSnapshot>,
    backend: Box<dyn SessionBackend>,
    attachment_root: PathBuf,
    memory_write_hook_executor: Arc<dyn MemoryWriteHookExecutor>,
    history_state_hook_executor: Arc<dyn HistoryStateHookExecutor>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedStore {
    sessions: SessionMap,
    #[serde(default)]
    attachment_assets: AttachmentAssetMap,
    #[serde(default)]
    session_attachment_index: SessionAttachmentIndex,
    #[serde(default)]
    mcp_source_snapshots: HashMap<String, McpSourceSnapshot>,
    #[serde(default)]
    skill_source_snapshots: HashMap<String, SkillSourceSnapshot>,
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
        Self::with_backend(Box::new(FileSessionBackend::new(default_storage_path())))
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
            backend,
            attachment_root,
            memory_write_hook_executor: Arc::new(NoopMemoryWriteHookExecutor),
            history_state_hook_executor: Arc::new(NoopHistoryStateHookExecutor),
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
            });
            session.history.push(TurnHistoryMessage {
                role: "assistant".to_string(),
                content: assistant_message.to_string(),
                attachments: Vec::new(),
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
            commit_history_node_from_live_state(
                session,
                classify_turn_node_kind(assistant_message),
                None,
            );
        }
        self.refresh_attachment_catalog();
        let snapshot = self.snapshot_for_session(&session_key);

        self.save_to_backend();
        snapshot
    }

    pub fn record_turn_trace(
        &mut self,
        session_id: Option<&str>,
        mut trace: TurnTraceRecord,
    ) -> SessionSnapshot {
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

        self.save_to_backend();
        snapshot
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
        let snapshot = self.snapshot_for_session(&session_key);
        self.save_to_backend();
        Some(snapshot)
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
        let snapshot = self.snapshot_for_session(&session_key);
        self.save_to_backend();
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

        self.save_to_backend();
        snapshot
    }

    #[allow(dead_code)]
    pub fn checkout_history_node(
        &mut self,
        session_id: Option<&str>,
        node_id: &str,
        mode: HistoryCheckoutMode,
    ) -> Result<SessionSnapshot, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let hook_executor = Arc::clone(&self.history_state_hook_executor);
        let mut blocked_error = None;
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
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
                let Some(node) = history_node(session, node_id).cloned() else {
                    return Err(format!("unknown history node: {node_id}"));
                };
                hydrate_session_from_node(session, &node);
                let branch_head_node_id = session
                    .history_branches
                    .iter()
                    .find(|branch| branch.branch_id == node.branch_id)
                    .and_then(|branch| branch.head_node_id.clone());
                session.history_cursor.visible_node_id = Some(node.node_id.clone());
                session.history_cursor.active_branch_id = Some(node.branch_id.clone());
                session.history_cursor.branch_head_node_id = branch_head_node_id;
                session.history_cursor.workspace_node_id = Some(node.node_id.clone());
                session.history_cursor.mode = if session.history_cursor.branch_head_node_id.as_deref()
                    == Some(node.node_id.as_str())
                {
                    HistoryCursorMode::Live
                } else {
                    HistoryCursorMode::Historical
                };
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
                    matches!(session.history_cursor.checkout_status, HistoryCheckoutStatus::Applied),
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
                    let hook_results = hook_executor.execute(&resolved_envelope).unwrap_or_default();
                    persist_history_state_hook_evidence(session, &resolved_envelope, &hook_results);
                }
            }
        }
        self.save_to_backend();
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
    ) -> Result<SessionSnapshot, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let hook_executor = Arc::clone(&self.history_state_hook_executor);
        let mut blocked_error = None;
        let mut restored_node_id = None;
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
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
                let node_id = branch
                    .head_node_id
                    .clone()
                    .ok_or_else(|| format!("history branch has no head node: {target_branch_id}"))?;
                let node = history_node(session, &node_id)
                    .cloned()
                    .ok_or_else(|| format!("unknown history node: {node_id}"))?;
                hydrate_session_from_node(session, &node);
                session.history_cursor.visible_node_id = Some(node.node_id.clone());
                session.history_cursor.active_branch_id = Some(branch.branch_id.clone());
                session.history_cursor.branch_head_node_id = Some(node.node_id.clone());
                session.history_cursor.workspace_node_id = Some(node.node_id.clone());
                session.history_cursor.mode = HistoryCursorMode::Live;
                session.history_cursor.checkout_mode = HistoryCheckoutMode::TranscriptOnly;
                session.history_cursor.checkout_status = HistoryCheckoutStatus::NotRequested;
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
                    let hook_results = hook_executor.execute(&resolved_envelope).unwrap_or_default();
                    persist_history_state_hook_evidence(session, &resolved_envelope, &hook_results);
                }
                restored_node_id = Some(node.node_id);
            }
        }
        self.save_to_backend();
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
    ) -> Result<SessionSnapshot, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let hook_executor = Arc::clone(&self.history_state_hook_executor);
        let mut blocked_error = None;
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
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
                hydrate_session_from_node(session, &source_node);
                session.history_cursor.visible_node_id = Some(source_node.node_id.clone());
                session.history_cursor.active_branch_id = Some(new_branch_id.clone());
                session.history_cursor.branch_head_node_id = Some(source_node.node_id.clone());
                session.history_cursor.workspace_node_id = Some(source_node.node_id.clone());
                session.history_cursor.mode = HistoryCursorMode::Live;
                session.history_cursor.checkout_mode = HistoryCheckoutMode::TranscriptOnly;
                session.history_cursor.checkout_status = HistoryCheckoutStatus::NotRequested;
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
                    let hook_results = hook_executor.execute(&resolved_envelope).unwrap_or_default();
                    persist_history_state_hook_evidence(session, &resolved_envelope, &hook_results);
                }
            }
        }
        self.save_to_backend();
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
    ) -> Result<SessionSnapshot, String> {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID).to_string();
        let hook_executor = Arc::clone(&self.history_state_hook_executor);
        let mut blocked_error = None;
        let mut target_node_id = None;
        {
            let session = self.ensure_session(&session_key);
            ensure_history_graph(session);
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
                hydrate_session_from_node(session, &node);
                session.history_cursor.visible_node_id = Some(node.node_id.clone());
                session.history_cursor.active_branch_id = Some(branch.branch_id.clone());
                session.history_cursor.branch_head_node_id = Some(node.node_id.clone());
                session.history_cursor.workspace_node_id = Some(node.node_id.clone());
                session.history_cursor.mode = HistoryCursorMode::Live;
                session.history_cursor.checkout_mode = HistoryCheckoutMode::TranscriptOnly;
                session.history_cursor.checkout_status = HistoryCheckoutStatus::NotRequested;
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
                    let hook_results = hook_executor.execute(&resolved_envelope).unwrap_or_default();
                    persist_history_state_hook_evidence(session, &resolved_envelope, &hook_results);
                }
                target_node_id = Some(node.node_id);
            }
        }
        self.save_to_backend();
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

    pub fn remove_session(&mut self, session_id: &str) -> Vec<SessionOverview> {
        if self.sessions.remove(session_id).is_some() {
            delete_session_attachment_dir(&self.attachment_root, session_id);
            self.refresh_attachment_catalog();
        }

        if self.sessions.is_empty() {
            self.sessions = default_sessions();
        }

        self.save_to_backend();
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
        self.sessions
            .entry(session_id.to_string())
            .or_insert_with(|| SessionState {
                conversation_id: session_id.to_string(),
                title: DEFAULT_SESSION_TITLE.to_string(),
                summary: DEFAULT_SESSION_SUMMARY.to_string(),
                history: Vec::new(),
                provider_native_transcript: Vec::new(),
                turn_trace_history: Vec::new(),
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
            })
    }

    fn save_to_backend(&self) {
        self.backend.save_store(&PersistedStore {
            sessions: self
                .sessions
                .iter()
                .filter(|(_, session)| session_is_persistable(session))
                .map(|(session_id, session)| (session_id.clone(), session.clone()))
                .collect::<SessionMap>(),
            attachment_assets: self.attachment_assets.clone(),
            session_attachment_index: self.session_attachment_index.clone(),
            mcp_source_snapshots: self.mcp_source_snapshots.clone(),
            skill_source_snapshots: self.skill_source_snapshots.clone(),
        });
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
        let checkout_status = if matches!(
            session.history_cursor.checkout_status,
            HistoryCheckoutStatus::DegradedToTranscriptOnly
        ) && session.history_cursor.visible_node_id.as_deref()
            == Some(selected_node.node_id.as_str())
        {
            HistoryCheckoutStatus::DegradedToTranscriptOnly
        } else {
            HistoryCheckoutStatus::NotRequested
        };
        let history_cursor = HistoryCursor {
            session_id: session.conversation_id.clone(),
            visible_node_id: Some(selected_node.node_id.clone()),
            active_branch_id: Some(selected_node.branch_id.clone()),
            branch_head_node_id: branch_head_node_id.clone(),
            workspace_node_id: Some(selected_node.node_id.clone()),
            mode: if branch_head_node_id.as_deref() == Some(selected_node.node_id.as_str()) {
                HistoryCursorMode::Live
            } else {
                HistoryCursorMode::Historical
            },
            checkout_mode: HistoryCheckoutMode::TranscriptOnly,
            checkout_status,
        };
        return SessionSnapshot {
            conversation_id: session.conversation_id.clone(),
            title: selected_node.title.clone(),
            summary: selected_node.summary.clone(),
            history: selected_node.history.clone(),
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
            history_nodes: session.history_nodes.clone(),
            history_branches: session.history_branches.clone(),
            history_cursor,
            resolved_node_id: Some(selected_node.node_id.clone()),
            latest_node_id,
        };
    }

    SessionSnapshot {
        conversation_id: session.conversation_id.clone(),
        title: session.title.clone(),
        summary: session.summary.clone(),
        history: session.history.clone(),
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
        history_nodes: session.history_nodes.clone(),
        history_branches: session.history_branches.clone(),
        history_cursor: session.history_cursor.clone(),
        resolved_node_id: session.history_cursor.visible_node_id.clone(),
        latest_node_id,
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
                long_term_memory_entries: materialized.long_term_memory_entries.clone(),
                memory_write_evidence: materialized.memory_write_evidence.clone(),
                memory_write_hook_trace_records: materialized
                    .memory_write_hook_trace_records
                    .clone(),
                turn_count: materialized.turn_count,
                last_referenced_file: materialized.last_referenced_file.clone(),
                created_at_ms: session.updated_at_ms.saturating_add(turn_index as u64),
            });
        }
        changed = true;
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
    if let Some(main_branch) = history_branch_mut(session, DEFAULT_HISTORY_BRANCH_ID) {
        if main_branch.base_node_id.is_none() {
            main_branch.base_node_id = first_node_id;
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
    hydrate_session_from_node(session, &visible_node);
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
        long_term_memory_entries: session.long_term_memory_entries.clone(),
        memory_write_evidence: session.memory_write_evidence.clone(),
        memory_write_hook_trace_records: session.memory_write_hook_trace_records.clone(),
        turn_count: session.turn_count,
        last_referenced_file: session.last_referenced_file.clone(),
        created_at_ms,
    });
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
    node.history = history;
    node.provider_native_transcript = provider_native_transcript;
    node.turn_trace_history = turn_trace_history;
    node.long_term_memory_entries = long_term_memory_entries;
    node.memory_write_evidence = memory_write_evidence;
    node.memory_write_hook_trace_records = memory_write_hook_trace_records;
    node.turn_count = turn_count;
    node.last_referenced_file = last_referenced_file;
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

fn history_state_hook_results_blocked(hook_results: &[crate::agent::hooks::HookExecutionResult]) -> bool {
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
        session.history_state_evidence.push(HistoryStateHookEvidence {
            evidence_id: format!(
                "history-state:{}:{}:{}:{}",
                session.conversation_id,
                envelope.source_boundary,
                recorded_at_ms,
                index
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
    if assistant_message.trim() == "This turn was cancelled." {
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
    dirs::data_local_dir()
        .or_else(dirs::home_dir)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
        .join("PonyAgent")
        .join("sessions.json")
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

fn merge_session_attachment_assets(
    sessions: &SessionMap,
    assets: &mut AttachmentAssetMap,
) {
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
        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].memory_write_hook_trace_records.len(), 1);
        assert_eq!(nodes[1].memory_write_hook_trace_records.len(), 2);

        let historical = store
            .checkout_history_node(
                Some("memory-hook-history"),
                nodes[0].node_id.as_str(),
                HistoryCheckoutMode::TranscriptOnly,
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
            snapshot.history_state_evidence[1].resolved_node_id.as_deref(),
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
            )
            .expect_err("checkout should be blocked by hook");
        assert!(error.contains("history checkout blocked by hook"));

        let live_after = store.snapshot(Some("history-hook-blocked"), &[]);
        assert_eq!(live_after.history_state_evidence.len(), 1);
        assert_eq!(
            live_after.history_state_evidence[0].boundary,
            "history.checkout.start"
        );
        assert_eq!(
            live_after.history_state_evidence[0].resolved_node_id,
            None
        );
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
            snapshot.history_state_evidence[1].resolved_node_id.as_deref(),
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
            .restore_branch_head(Some("history-restore-session"), Some("branch-main"))
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
            snapshot.history_state_evidence[1].resolved_branch_id.as_deref(),
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
            .fork_from_history_node(Some("history-fork-blocked"), nodes[0].node_id.as_str())
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
            .fork_from_history_node(Some("history-switch-session"), nodes[0].node_id.as_str())
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
            .switch_history_branch(Some("history-switch-session"), "branch-main")
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
            snapshot.history_state_evidence[1].resolved_branch_id.as_deref(),
            Some("branch-main")
        );
        assert_eq!(snapshot.history_cursor.active_branch_id.as_deref(), Some("branch-main"));
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
        assert_eq!(snapshot.history_cursor.mode, HistoryCursorMode::Historical);
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
            snapshot.history_state_audit_summary.action.boundary.as_deref(),
            Some("history.checkout.resolved")
        );
        assert!(snapshot.history_state_audit_summary.action.degraded);
        assert_eq!(
            snapshot.history_state_audit_summary.current_context.mode,
            "historical"
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
            )
            .expect("degraded checkout should succeed");
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
        assert_eq!(snapshot.history_state_audit_summary.action.status, "missing");
        assert_eq!(
            snapshot.history_cursor.checkout_status,
            HistoryCheckoutStatus::DegradedToTranscriptOnly
        );
        assert_eq!(
            snapshot.history_cursor.checkout_mode,
            HistoryCheckoutMode::TranscriptAndWorkspace
        );
        assert_eq!(snapshot.history_cursor.mode, HistoryCursorMode::Historical);
        assert_eq!(snapshot.resolved_node_id.as_deref(), Some(resolved_node_id.as_str()));

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

        let reclaimable_attachments = store
            .save_input_attachments(&session_id, &reclaimable_images)
            .expect("save reclaimable attachments");
        let reclaimable_asset_id = reclaimable_attachments[0].asset_id.clone();

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

        let reclaimable_attachments = store
            .save_input_attachments(&session_id, &reclaimable_images)
            .expect("save reclaimable attachments");
        let reclaimable_asset_id = reclaimable_attachments[0].asset_id.clone();
        let reclaimable_path = store
            .attachment_root
            .join(&reclaimable_attachments[0].relative_path);

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
                    },
                    TurnHistoryMessage {
                        role: "assistant".to_string(),
                        attachments: Vec::new(),
                        content: "搜索没有直接命中，让我查看一下工作区的文件结构。".to_string(),
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
                    status: "done".to_string(),
                    summary: "读取文件".to_string(),
                    arguments_text: Some("{\"path\":\"src/main.ts\"}".to_string()),
                    result_text: Some("ok".to_string()),
                    duration_seconds: Some(0.12),
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
                        summary: "mcp source ingress registered `mcp-local` with 1 capability candidates"
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
                        summary: "skill source ingress registered `host-skills` with 1 skill candidates"
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
                composed_capability_kinds: vec![crate::agent::capability_bridge::CapabilityKind::Tool],
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
                    status: "completed".to_string(),
                    summary: "tool completed before finalize failed".to_string(),
                    arguments_text: Some("{\"path\":\".\"}".to_string()),
                    result_text: Some("ok".to_string()),
                    duration_seconds: Some(0.2),
                    capability_invocation: None,
                }],
                provider_call_records: vec![ProviderCallCacheRecord {
                    request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                    provider_source: Some("provider_decision".to_string()),
                    provider_mode: Some("live".to_string()),
                    input_tokens: Some(11),
                    cache_hit_input_tokens: Some(4),
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
                        status: "completed".to_string(),
                        summary: "tool completed before cancel".to_string(),
                        arguments_text: Some("{\"path\":\".\"}".to_string()),
                        result_text: Some("ok".to_string()),
                        duration_seconds: Some(0.2),
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
                    status: "completed".to_string(),
                    summary: "tool completed before cancel".to_string(),
                    arguments_text: Some("{\"path\":\".\"}".to_string()),
                    result_text: Some("ok".to_string()),
                    duration_seconds: Some(0.2),
                    capability_invocation: None,
                }],
                provider_call_records: vec![ProviderCallCacheRecord {
                    request_kind: crate::agent::telemetry::ProviderRequestKind::InitialRequest,
                    provider_source: Some("provider_decision".to_string()),
                    provider_mode: Some("live".to_string()),
                    input_tokens: Some(9),
                    cache_hit_input_tokens: Some(3),
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
        assert_eq!(nodes.len(), 2);
        assert_eq!(branches.len(), 1);

        let snapshot = store
            .checkout_history_node(
                Some("history-session"),
                &nodes[0].node_id,
                HistoryCheckoutMode::TranscriptAndWorkspace,
            )
            .expect("history checkout should succeed");

        assert_eq!(
            snapshot.resolved_node_id.as_deref(),
            Some(nodes[0].node_id.as_str())
        );
        assert_eq!(snapshot.history.len(), 2);
        assert_eq!(snapshot.history[0].content, "第一问");
        assert_eq!(snapshot.history[1].content, "第一答");
        assert_eq!(
            snapshot.latest_node_id.as_deref(),
            Some(nodes[1].node_id.as_str())
        );
        assert_eq!(snapshot.history_cursor.mode, HistoryCursorMode::Historical);
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
        assert_eq!(nodes_after.len(), 3);
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

        let (nodes_before, _, _) = store.load_history_graph(Some("switch-session"));
        let first_node_id = nodes_before[0].node_id.clone();
        let second_node_id = nodes_before[1].node_id.clone();

        store
            .fork_from_history_node(Some("switch-session"), first_node_id.as_str())
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
            .switch_history_branch(Some("switch-session"), DEFAULT_HISTORY_BRANCH_ID)
            .expect("switch to main branch should succeed");
        assert_eq!(
            switched.resolved_node_id.as_deref(),
            Some(second_node_id.as_str())
        );
        assert_eq!(switched.history_cursor.mode, HistoryCursorMode::Live);

        let restored = store
            .restore_branch_head(Some("switch-session"), Some(fork_branch_id.as_str()))
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
}
