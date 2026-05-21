use serde::{Deserialize, Serialize};
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

pub struct MemorySessionBackend;

impl SessionStore {
    pub fn new() -> Self {
        Self::with_backend(Box::new(FileSessionBackend::new(default_storage_path())))
    }

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
            let created = !self.sessions.contains_key(session_key);
            let session = self.ensure_session(session_key);
            if created {
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
        let Some(parent) = self.storage_path.parent() else {
            return;
        };
        if fs::create_dir_all(parent).is_err() {
            return;
        }

        let payload = PersistedSessions {
            sessions: sessions.clone(),
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
    session.title = build_title(&session.history);
    session.summary = build_summary(session.turn_count, session.last_referenced_file.as_deref());
    if touch_updated_at {
        session.updated_at_ms = now_timestamp_ms();
    }
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
        store.append_turn(Some("test"), "查看 tauri.conf.json", "已读取");

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
        store.append_turn(Some("persisted"), "打开 Cargo.toml", "已读取");

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
        );
        store.append_turn(
            Some("preview"),
            "Also verify provider fallback behavior.",
            "Done.",
        );

        let snapshot = store.snapshot(Some("preview"), &[]);
        assert_eq!(snapshot.title, "Please inspect runtime.rs se...");
    }

    #[test]
    fn removing_last_session_recreates_default_session() {
        let mut store = SessionStore::memory_only();
        let sessions = store.remove_session(DEFAULT_SESSION_ID);

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].conversation_id, DEFAULT_SESSION_ID);
        assert_eq!(sessions[0].title, DEFAULT_SESSION_TITLE);
    }

    fn temp_sessions_path() -> PathBuf {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("pony-agent-session-test-{}.json", stamp))
    }

    #[test]
    fn snapshot_persists_new_empty_session() {
        let path = temp_sessions_path();
        let backend = Box::new(FileSessionBackend::new(path.clone()));
        let mut store = SessionStore::with_backend(backend);

        let snapshot = store.snapshot(Some("fresh"), &[]);
        assert_eq!(snapshot.conversation_id, "fresh");
        assert_eq!(snapshot.turn_count, 0);

        let mut reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
        let reloaded_snapshot = reloaded.snapshot(Some("fresh"), &[]);
        assert_eq!(reloaded_snapshot.conversation_id, "fresh");
        assert_eq!(reloaded_snapshot.turn_count, 0);

        let _ = fs::remove_file(path);
    }
}
