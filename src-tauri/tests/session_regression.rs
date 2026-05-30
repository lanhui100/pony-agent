mod agent {
    #[path = "../../src/agent/input.rs"]
    pub mod input;

    #[path = "../../src/agent/tools.rs"]
    pub mod tools;

    #[path = "../../src/agent/telemetry.rs"]
    pub mod telemetry;
}

#[path = "../src/agent/session.rs"]
mod session;

use serde_json::json;
use session::{
    AttachmentAssetQuery, AttachmentCleanupRequest, AttachmentLifecycleStatus, FileSessionBackend,
    SessionStore, TurnHistoryMessage,
};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_sessions_path() -> PathBuf {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    std::env::temp_dir()
        .join(format!("pony-agent-session-regression-{stamp}"))
        .join("sessions.json")
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
            attachments: Vec::new(),
        },
        TurnHistoryMessage {
            role: "assistant".to_string(),
            content: "I will inspect it.".to_string(),
            attachments: Vec::new(),
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
    assert_eq!(
        snapshot.last_referenced_file.as_deref(),
        Some("src/main.rs")
    );

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
        Vec::new(),
    );
    store.append_turn(
        Some("beta"),
        "Review src-tauri/src/agent/runtime.rs flow.",
        "Done.",
        None,
        Vec::new(),
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

#[test]
fn legacy_session_file_backfills_attachment_catalog_and_index() {
    let path = temp_sessions_path();
    let attachment_root = path
        .parent()
        .map(|parent| parent.join("attachments").join("legacy-session"))
        .expect("attachment root");
    fs::create_dir_all(&attachment_root).expect("create attachment dir");
    fs::write(
        attachment_root.join("att-1.dataurl"),
        "data:image/png;base64,AAAA",
    )
    .expect("write attachment payload");

    fs::write(
        &path,
        serde_json::to_string_pretty(&json!({
            "sessions": {
                "legacy-session": {
                    "conversationId": "legacy-session",
                    "title": "Legacy",
                    "summary": "Legacy session",
                    "history": [
                        {
                            "role": "user",
                            "content": "[已附图片 1 张：legacy.png]",
                            "attachments": [
                                {
                                    "id": "att-1",
                                    "name": "legacy.png",
                                    "mimeType": "image/png",
                                    "relativePath": "legacy-session/att-1.dataurl",
                                    "sizeBytes": 4,
                                    "createdAtMs": 1
                                }
                            ]
                        },
                        {
                            "role": "assistant",
                            "content": "我看到了图片。",
                            "attachments": []
                        }
                    ],
                    "providerNativeTranscript": [],
                    "turnTraceHistory": [],
                    "turnCount": 1,
                    "lastReferencedFile": null,
                    "updatedAtMs": 1
                }
            }
        }))
        .expect("serialize legacy session"),
    )
    .expect("write legacy session file");

    let mut store = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let snapshot = store.snapshot(Some("legacy-session"), &[]);
    let session_assets = store.list_attachment_assets(Some("legacy-session"));
    let all_assets = store.list_attachment_assets(None);

    assert_eq!(snapshot.attachment_assets.len(), 1);
    assert_eq!(
        snapshot.attachment_assets[0].id,
        "asset:legacy-session/att-1.dataurl"
    );
    assert_eq!(session_assets.len(), 1);
    assert_eq!(all_assets.len(), 1);

    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(
        attachment_root
            .parent()
            .expect("legacy attachment parent")
            .to_path_buf(),
    );
}

#[test]
fn orphan_attachment_payloads_are_cataloged_and_cleanup_preserves_referenced_history() {
    let path = temp_sessions_path();
    let attachment_root = path
        .parent()
        .map(|parent| parent.join("attachments").join("legacy-reclaim"))
        .expect("attachment root");
    fs::create_dir_all(&attachment_root).expect("create attachment dir");
    fs::write(
        attachment_root.join("att-1.dataurl"),
        "data:image/png;base64,AAAA",
    )
    .expect("write referenced attachment payload");
    fs::write(
        attachment_root.join("draft-1.dataurl"),
        "data:image/webp;base64,BBBB",
    )
    .expect("write orphan attachment payload");

    fs::write(
        &path,
        serde_json::to_string_pretty(&json!({
            "sessions": {
                "legacy-reclaim": {
                    "conversationId": "legacy-reclaim",
                    "title": "Legacy reclaim",
                    "summary": "Legacy reclaim session",
                    "history": [
                        {
                            "role": "user",
                            "content": "[宸查檮鍥剧墖 1 寮狅細keep.png]",
                            "attachments": [
                                {
                                    "id": "att-1",
                                    "assetId": "asset:legacy-reclaim/att-1.dataurl",
                                    "name": "keep.png",
                                    "mimeType": "image/png",
                                    "relativePath": "legacy-reclaim/att-1.dataurl",
                                    "sizeBytes": 4,
                                    "createdAtMs": 1
                                }
                            ]
                        },
                        {
                            "role": "assistant",
                            "content": "still referenced",
                            "attachments": []
                        }
                    ],
                    "providerNativeTranscript": [],
                    "turnTraceHistory": [],
                    "turnCount": 1,
                    "lastReferencedFile": null,
                    "updatedAtMs": 1
                }
            }
        }))
        .expect("serialize legacy session"),
    )
    .expect("write legacy session file");

    let mut store = SessionStore::with_backend(Box::new(FileSessionBackend::new(path.clone())));
    let assets = store.list_attachment_assets(Some("legacy-reclaim"));
    assert_eq!(assets.len(), 2);
    assert!(assets
        .iter()
        .any(|asset| asset.status == AttachmentLifecycleStatus::Active));
    assert!(assets
        .iter()
        .any(|asset| asset.status == AttachmentLifecycleStatus::Reclaimable));

    let reclaimable = store.query_attachment_assets(&AttachmentAssetQuery {
        session_id: Some("legacy-reclaim".to_string()),
        mime_type: Some("webp".to_string()),
        name_contains: Some("draft".to_string()),
        created_after_ms: None,
        created_before_ms: None,
        statuses: vec![AttachmentLifecycleStatus::Reclaimable],
        limit: None,
    });
    assert_eq!(reclaimable.len(), 1);

    let cleanup = store.cleanup_attachment_assets(&AttachmentCleanupRequest {
        session_id: Some("legacy-reclaim".to_string()),
        expire_before_ms: Some(u64::MAX),
        include_reclaimable: true,
        include_expired: true,
        limit: None,
    });
    assert_eq!(cleanup.removed_catalog_count, 1);

    let remaining_assets = store.list_attachment_assets(Some("legacy-reclaim"));
    assert_eq!(remaining_assets.len(), 1);
    assert_eq!(
        remaining_assets[0].status,
        AttachmentLifecycleStatus::Active
    );
    assert_eq!(store.load_recent_images(Some("legacy-reclaim"), 1).len(), 1);

    let _ = fs::remove_file(path);
    let _ = fs::remove_dir_all(
        attachment_root
            .parent()
            .expect("legacy attachment parent")
            .to_path_buf(),
    );
}
