use crate::agent::telemetry::{TurnToolActivity, TurnTraceStep};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_SESSION_ID: &str = "local-dev-session";
const DEFAULT_SESSION_SUMMARY: &str = "Pony Agent 本地开发会话";
const DEFAULT_HISTORY_LIMIT: usize = 24;
const DEFAULT_SESSION_TITLE: &str = "\u{65B0}\u{5BF9}\u{8BDD}";
const TITLE_MAX_CHARS: usize = 28;

type SessionMap = HashMap<String, SessionState>;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnHistoryMessage {
    pub role: String,
    pub content: String,
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
    pub provider_native_transcript: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turn_trace_history: Vec<TurnTraceRecord>,
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
    pub session_summary: Option<String>,
    pub fallback_reason: Option<String>,
    pub error: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
    pub first_token_latency_ms: Option<u64>,
    #[serde(default)]
    pub updated_at: u64,
}

pub trait SessionBackend: Send {
    fn load_sessions(&self) -> Option<SessionMap>;
    fn save_sessions(&self, sessions: &SessionMap);
}

pub struct SessionStore {
    sessions: SessionMap,
    backend: Box<dyn SessionBackend>,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PersistedSessions {
    sessions: SessionMap,
}

pub struct FileSessionBackend {
    storage_path: PathBuf,
}

#[cfg(test)]
pub struct MemorySessionBackend;

impl SessionStore {
    pub fn new() -> Self {
        Self::with_backend(Box::new(FileSessionBackend::new(default_storage_path())))
    }

    #[cfg(test)]
    pub fn memory_only() -> Self {
        Self::with_backend(Box::new(MemorySessionBackend))
    }

    pub fn with_backend(backend: Box<dyn SessionBackend>) -> Self {
        let mut sessions = backend.load_sessions().unwrap_or_else(default_sessions);
        for session in sessions.values_mut() {
            refresh_session_metadata(session, false);
            if session.updated_at_ms == 0 {
                session.updated_at_ms = now_timestamp_ms();
            }
        }
        Self { sessions, backend }
    }

    pub fn snapshot(
        &mut self,
        session_id: Option<&str>,
        fallback_history: &[TurnHistoryMessage],
    ) -> SessionSnapshot {
        let session_key = session_id.unwrap_or(DEFAULT_SESSION_ID);
        let mut should_save = false;
        let snapshot = {
            let session = self.ensure_session(session_key);
            if sanitize_provider_native_transcript(session) {
                should_save = true;
            }
            if session.history.is_empty() && !fallback_history.is_empty() {
                session.history = fallback_history.to_vec();
                refresh_session_metadata(session, false);
                should_save = true;
            }

            snapshot_from_state(session)
        };

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
    ) -> SessionSnapshot {
        let snapshot = {
            let session = self.ensure_session(session_id.unwrap_or(DEFAULT_SESSION_ID));
            session.history.push(TurnHistoryMessage {
                role: "user".to_string(),
                content: user_message.to_string(),
            });
            session.history.push(TurnHistoryMessage {
                role: "assistant".to_string(),
                content: assistant_message.to_string(),
            });

            if session.history.len() > DEFAULT_HISTORY_LIMIT {
                let keep_from = session.history.len() - DEFAULT_HISTORY_LIMIT;
                session.history = session.history[keep_from..].to_vec();
            }

            if let Some(messages) = provider_native_transcript {
                session.provider_native_transcript.extend(messages);
            }

            refresh_session_metadata(session, true);

            snapshot_from_state(session)
        };

        self.save_to_backend();
        snapshot
    }

    pub fn record_turn_trace(
        &mut self,
        session_id: Option<&str>,
        mut trace: TurnTraceRecord,
    ) -> SessionSnapshot {
        let snapshot = {
            let session = self.ensure_session(session_id.unwrap_or(DEFAULT_SESSION_ID));
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
            snapshot_from_state(session)
        };

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
        self.sessions.remove(session_id);

        if self.sessions.is_empty() {
            self.sessions = default_sessions();
        }

        self.save_to_backend();
        self.list_sessions()
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
                turn_count: 0,
                last_referenced_file: None,
                updated_at_ms: now_timestamp_ms(),
            })
    }

    fn save_to_backend(&self) {
        self.backend.save_sessions(&self.sessions);
    }
}

impl FileSessionBackend {
    pub fn new(storage_path: PathBuf) -> Self {
        Self { storage_path }
    }
}

impl SessionBackend for FileSessionBackend {
    fn load_sessions(&self) -> Option<SessionMap> {
        eprintln!(
            "[pony-agent][session] loading sessions from {}",
            self.storage_path.display()
        );
        load_sessions_from_path(&self.storage_path)
    }

    fn save_sessions(&self, sessions: &SessionMap) {
        let sessions = sessions
            .iter()
            .filter(|(_, session)| session_is_persistable(session))
            .map(|(session_id, session)| (session_id.clone(), session.clone()))
            .collect::<SessionMap>();

        let Some(parent) = self.storage_path.parent() else {
            return;
        };
        if fs::create_dir_all(parent).is_err() {
            return;
        }

        let payload = PersistedSessions {
            sessions,
        };
        let Ok(serialized) = serde_json::to_string_pretty(&payload) else {
            return;
        };
        eprintln!(
            "[pony-agent][session] saving sessions to {}",
            self.storage_path.display()
        );
        let _ = fs::write(&self.storage_path, serialized);
    }
}

#[cfg(test)]
impl SessionBackend for MemorySessionBackend {
    fn load_sessions(&self) -> Option<SessionMap> {
        None
    }

    fn save_sessions(&self, _sessions: &SessionMap) {}
}

fn snapshot_from_state(session: &SessionState) -> SessionSnapshot {
    SessionSnapshot {
        conversation_id: session.conversation_id.clone(),
        title: session.title.clone(),
        summary: session.summary.clone(),
        history: session.history.clone(),
        provider_native_transcript: session.provider_native_transcript.clone(),
        turn_trace_history: session.turn_trace_history.clone(),
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
        session.summary = build_summary(session.turn_count, session.last_referenced_file.as_deref());
    }
    if touch_updated_at {
        session.updated_at_ms = now_timestamp_ms();
    }
}

fn sanitize_provider_native_transcript(session: &mut SessionState) -> bool {
    if !has_legacy_reasoning_gap(&session.provider_native_transcript) {
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

fn session_is_persistable(session: &SessionState) -> bool {
    !session.history.is_empty() || !session.turn_trace_history.is_empty()
}

fn build_title(history: &[TurnHistoryMessage]) -> String {
    history
        .iter()
        .find(|message| message.role == "user")
        .and_then(|message| normalize_title_candidate(&message.content))
        .unwrap_or_else(|| DEFAULT_SESSION_TITLE.to_string())
}

fn build_summary(turn_count: usize, last_referenced_file: Option<&str>) -> String {
    match (turn_count, last_referenced_file) {
        (0, _) => DEFAULT_SESSION_SUMMARY.to_string(),
        (_, Some(path)) => format!(
            "{} / 已完成 {} 轮 / 当前关注 {}",
            DEFAULT_SESSION_SUMMARY, turn_count, path
        ),
        (_, None) => format!("{} / 已完成 {} 轮", DEFAULT_SESSION_SUMMARY, turn_count),
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

fn load_sessions_from_path(path: &Path) -> Option<SessionMap> {
    let raw = fs::read_to_string(path).ok()?;
    let persisted: PersistedSessions = serde_json::from_str(&raw).ok()?;
    if persisted.sessions.is_empty() {
        None
    } else {
        Some(persisted.sessions)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn memory_backend_keeps_turns_in_process() {
        let mut store = SessionStore::memory_only();
        store.append_turn(Some("test"), "查看 tauri.conf.json", "已读取", None);

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
        store.append_turn(Some("persisted"), "打开 Cargo.toml", "已读取", None);

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
    fn session_title_uses_first_user_message_preview() {
        let mut store = SessionStore::memory_only();
        store.append_turn(
            Some("preview"),
            "Please inspect runtime.rs session switching and trace consistency after tool execution.",
            "I will check it.",
            None,
        );
        store.append_turn(
            Some("preview"),
            "Also verify provider fallback behavior.",
            "Done.",
            None,
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
                turn_count: 1,
                last_referenced_file: Some("src-tauri/tauri.conf.json".to_string()),
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
        std::env::temp_dir().join(format!("pony-agent-session-test-{}.json", stamp))
    }

    #[test]
    fn snapshot_does_not_persist_new_empty_session() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);

        let snapshot = store.snapshot(Some("fresh"), &[]);
        assert_eq!(snapshot.conversation_id, "fresh");
        assert_eq!(snapshot.turn_count, 0);

        let persisted = load_sessions_from_path(&path);
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
                session_summary: Some("测试 trace 持久化".to_string()),
                fallback_reason: None,
                error: None,
                input_tokens: Some(12),
                output_tokens: Some(34),
                total_tokens: Some(46),
                first_token_latency_ms: Some(180),
                updated_at: 0,
            },
        );

        let mut reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let snapshot = reloaded.snapshot(Some("trace-persisted"), &[]);

        assert_eq!(snapshot.turn_trace_history.len(), 1);
        assert_eq!(snapshot.turn_trace_history[0].turn_id, "turn-1");
        assert_eq!(snapshot.turn_trace_history[0].provider_model.as_deref(), Some("gpt-5.4"));

        let _ = fs::remove_file(path);
    }
}
