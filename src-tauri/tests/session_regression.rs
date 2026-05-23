#[path = "../src/agent/session.rs"]
mod session;

use session::{FileSessionBackend, SessionStore, TurnHistoryMessage};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_sessions_path() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("pony-agent-session-regression-{stamp}.json"))
}

#[test]
fn snapshot_with_fallback_history_persists_hydrated_session_metadata() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);
    let fallback_history = vec![
        TurnHistoryMessage {
            role: "user".to_string(),
            content: "Inspect src/main.rs for startup wiring.".to_string(),
        },
        TurnHistoryMessage {
            role: "assistant".to_string(),
            content: "I will inspect it.".to_string(),
        },
    ];

    let snapshot = store.snapshot(Some("hydrated"), &fallback_history);
    let reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let mut reloaded = reloaded;
    let persisted = reloaded.snapshot(Some("hydrated"), &[]);

    assert_eq!(snapshot.conversation_id, "hydrated");
    assert_eq!(snapshot.history.len(), 2);
    assert_eq!(snapshot.turn_count, 1);
    assert_eq!(snapshot.title, "Inspect src/main.rs for star...");
    assert_eq!(snapshot.last_referenced_file.as_deref(), Some("src/main.rs"));

    assert_eq!(persisted.history.len(), 2);
    assert_eq!(persisted.turn_count, 1);
    assert_eq!(persisted.title, snapshot.title);
    assert_eq!(
        persisted.last_referenced_file.as_deref(),
        Some("src/main.rs")
    );

    let _ = fs::remove_file(path);
}

#[test]
fn removing_one_session_keeps_other_persisted_sessions_intact() {
    let path = temp_sessions_path();
    let backend = Box::new(FileSessionBackend::new(path.clone()));
    let mut store = SessionStore::with_backend(backend);

    store.append_turn(
        Some("alpha"),
        "Review src/lib.rs command registration.",
        "Done.",
        None,
    );
    store.append_turn(
        Some("beta"),
        "Review src-tauri/src/agent/runtime.rs flow.",
        "Done.",
        None,
    );

    let remaining = store.remove_session("alpha");
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].conversation_id, "beta");

    let reloaded = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let mut reloaded = reloaded;
    let beta = reloaded.snapshot(Some("beta"), &[]);
    let alpha = reloaded.snapshot(Some("alpha"), &[]);

    assert_eq!(beta.turn_count, 1);
    assert_eq!(
        beta.last_referenced_file.as_deref(),
        Some("src-tauri/src/agent/runtime.rs")
    );
    assert_eq!(alpha.turn_count, 0);
    assert!(alpha.history.is_empty());

    let _ = fs::remove_file(path);
}
