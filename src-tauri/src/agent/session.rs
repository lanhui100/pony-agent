use crate::agent::input::TurnInputImage;
use crate::agent::provider::BuildContextObservation;
use crate::agent::telemetry::{TurnToolActivity, TurnTraceStep};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_SESSION_ID: &str = "local-dev-session";
const DEFAULT_SESSION_SUMMARY: &str = "Pony Agent 本地开发会话";
const DEFAULT_HISTORY_LIMIT: usize = 24;
const DEFAULT_SESSION_TITLE: &str = "\u{65B0}\u{5BF9}\u{8BDD}";
const TITLE_MAX_CHARS: usize = 28;
const DEFAULT_ATTACHMENT_RECLAIM_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1000;

type SessionMap = HashMap<String, SessionState>;
type AttachmentAssetMap = HashMap<String, AttachmentAsset>;
type SessionAttachmentIndex = HashMap<String, Vec<String>>;

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
    pub turn_count: usize,
    pub last_referenced_file: Option<String>,
    #[serde(default)]
    pub updated_at_ms: u64,
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
    pub turn_count: usize,
    pub last_referenced_file: Option<String>,
    pub updated_at_ms: u64,
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
pub struct TurnTraceRecord {
    pub turn_id: String,
    pub title: String,
    pub phase: String,
    #[serde(default)]
    pub trace_steps: Vec<TurnTraceStep>,
    #[serde(default)]
    pub tool_activities: Vec<TurnToolActivity>,
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
    backend: Box<dyn SessionBackend>,
    attachment_root: PathBuf,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedStore {
    sessions: SessionMap,
    #[serde(default)]
    attachment_assets: AttachmentAssetMap,
    #[serde(default)]
    session_attachment_index: SessionAttachmentIndex,
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
        let rebuilt_assets =
            rebuild_attachment_assets(&sessions, &attachment_assets, &attachment_root);
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
            backend,
            attachment_root,
        };
        if should_save {
            store.save_to_backend();
        }
        store
    }

    pub fn snapshot(
        &mut self,
        session_id: Option<&str>,
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
        }
        if should_refresh_catalog {
            self.refresh_attachment_catalog();
        }
        let snapshot = self.snapshot_for_session(session_key);

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
        {
            let session = self.ensure_session(&session_key);
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

            update_long_term_memory_from_user_message(session, user_message);
            refresh_session_metadata(session, true);
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
            trace.updated_at = now_timestamp_ms();

            if let Some(existing) = session
                .turn_trace_history
                .iter_mut()
                .find(|item| item.turn_id == trace.turn_id)
            {
                *existing = trace;
            } else {
                session.turn_trace_history.push(trace);
            }

            if session.turn_trace_history.len() > DEFAULT_HISTORY_LIMIT {
                let keep_from = session.turn_trace_history.len() - DEFAULT_HISTORY_LIMIT;
                session.turn_trace_history = session.turn_trace_history[keep_from..].to_vec();
            }

            refresh_session_metadata(session, true);
        }
        let snapshot = self.snapshot_for_session(&session_key);

        self.save_to_backend();
        snapshot
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
            session.long_term_memory_entries = entries;
            refresh_session_metadata(session, true);
        }
        let snapshot = self.snapshot_for_session(&session_key);

        self.save_to_backend();
        snapshot
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
                turn_count: 0,
                last_referenced_file: None,
                updated_at_ms: now_timestamp_ms(),
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
        });
    }

    fn snapshot_for_session(&self, session_id: &str) -> SessionSnapshot {
        let session = self
            .sessions
            .get(session_id)
            .expect("session must exist before snapshot");
        snapshot_from_state(
            session,
            attachment_assets_for_query(
                &self.sessions,
                &self.attachment_assets,
                &self.session_attachment_index,
                &self.attachment_root,
                &AttachmentAssetQuery {
                    session_id: Some(session_id.to_string()),
                    ..AttachmentAssetQuery::default()
                },
                now_timestamp_ms(),
            ),
        )
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
) -> SessionSnapshot {
    SessionSnapshot {
        conversation_id: session.conversation_id.clone(),
        title: session.title.clone(),
        summary: session.summary.clone(),
        history: session.history.clone(),
        attachment_assets,
        provider_native_transcript: session.provider_native_transcript.clone(),
        turn_trace_history: session.turn_trace_history.clone(),
        long_term_memory_entries: session.long_term_memory_entries.clone(),
        turn_count: session.turn_count,
        last_referenced_file: session.last_referenced_file.clone(),
        updated_at_ms: session.updated_at_ms,
    }
}

fn refresh_session_metadata(session: &mut SessionState, touch_updated_at: bool) {
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

fn update_long_term_memory_from_user_message(
    session: &mut SessionState,
    user_message: &str,
) -> bool {
    let extracted_entries = extract_long_term_memory_from_user_message(user_message);
    if extracted_entries.is_empty() {
        return false;
    }

    let mut changed = false;
    for entry in extracted_entries {
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
                    .and_then(Value::as_str)
                    .map(|value| value.trim().is_empty())
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
            turn_count: 0,
            last_referenced_file: None,
            updated_at_ms: now_timestamp_ms(),
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
    assets
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
                turn_count: 1,
                last_referenced_file: None,
                updated_at_ms: now_timestamp_ms(),
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
                turn_count: 1,
                last_referenced_file: Some("src-tauri/tauri.conf.json".to_string()),
                updated_at_ms: now_timestamp_ms(),
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
                turn_count: 1,
                last_referenced_file: Some("tauri.conf.json".to_string()),
                updated_at_ms: now_timestamp_ms(),
            },
        );

        let snapshot = store.snapshot(Some(session_id), &[]);
        assert!(snapshot.provider_native_transcript.is_empty());
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
        assert!(persisted.is_none());

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
                title: "检查流式输出".to_string(),
                phase: "completed".to_string(),
                trace_steps: vec![TurnTraceStep {
                    id: "step-return".to_string(),
                    label: "Return result".to_string(),
                    state: "completed".to_string(),
                }],
                tool_activities: vec![TurnToolActivity {
                    id: "tool-1".to_string(),
                    name: "workspace.read_file".to_string(),
                    status: "done".to_string(),
                    summary: "读取文件".to_string(),
                    arguments_text: Some("{\"path\":\"src/main.ts\"}".to_string()),
                    result_text: Some("ok".to_string()),
                    duration_seconds: Some(0.12),
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

        let mut reloaded =
            SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let snapshot = reloaded.snapshot(Some("trace-persisted"), &[]);

        assert_eq!(snapshot.turn_trace_history.len(), 1);
        assert_eq!(snapshot.turn_trace_history[0].turn_id, "turn-1");
        assert_eq!(
            snapshot.turn_trace_history[0].provider_model.as_deref(),
            Some("gpt-5.4")
        );

        let _ = fs::remove_file(path);
    }
}
