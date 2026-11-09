//! ADR 0102 验收：为日志补齐 **transcript / 长期记忆四件套 / `history_state_evidence`** 的
//! 事件载体（`AppendProviderTranscript` / `UpdateSessionMemory`）。
//!
//! 六类用例（全部真断言，无「跑通即放行」）：
//!
//! 1. [`transcript_survives_restart_while_the_index_row_stays_light`] —— 有日志会话写入
//!    transcript → 重启（重新 `with_backend`）→ `load_store()` 逐字段一致，且索引行仍是
//!    `__index_only` 轻量行（blob 不再承载内容）；
//! 2. [`memory_snapshot_replaces_but_evidence_appends`] —— 记忆三项**整份替换**（两次命令
//!    后条数不累加）、`history_state_evidence` **追加**（条数累加）；
//! 3. [`memory_and_evidence_round_trip_through_restart`] —— 重启后记忆四件套逐字段一致；
//! 4. [`command_payloads_round_trip_through_encode_and_fold`] —— 两类命令 encode → fold 无损；
//! 5. [`log_write_failure_leaves_sqlite_untouched`] —— 日志写失败 ⇒ `Failed` 且 SQLite 零改动；
//! 6. [`migration_now_reproduces_transcript_and_memory`] —— 带 transcript 的 grandfather
//!    会话迁移后 fold == blob 逐字段一致（不再进 `mismatched`）。
//!
//! 隔离：全部落在 `std::env::temp_dir()` 下的唯一目录，**绝不触碰** `~/.local/share/PonyAgent/`。

use std::path::{Path, PathBuf};

use pony_agent_core::agent::hooks::{HistoryStateHookEvidence, PersistedEffectEvidence};
use pony_agent_core::agent::session::{
    fold_session, migrate_grandfathered_sessions, read_log_events, read_session_state,
    session_log_root_for_db, HistoryCursor, LongTermMemoryRecord, LogEvent, MigrateOptions,
    PersistCommand, PersistCommandOutcome, SeparateTraceTableMode, SessionBackend, SessionLogStore,
    SessionState, SessionStore, TurnHistoryMessage, TurnTraceRecord, MIGRATION_PARITY_EXEMPTIONS,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;

const SESSION_ID: &str = "pa_adr0102";

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let unique = format!(
            "pony-adr0102-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).expect("create fixture root");
        Self { root }
    }

    fn app_dir(&self, label: &str) -> PathBuf {
        let dir = self.root.join(label);
        std::fs::create_dir_all(&dir).expect("create app dir");
        dir
    }

    fn db(&self, app: &Path) -> PathBuf {
        app.join("sessions.db")
    }

    fn log_root(&self, app: &Path) -> PathBuf {
        session_log_root_for_db(&self.db(app))
    }

    fn backend(&self, app: &Path) -> SqliteSessionBackend {
        SqliteSessionBackend::new_with_trace_mode(self.db(app), SeparateTraceTableMode::WriteSeparate)
    }

    fn store(&self, app: &Path) -> SessionStore {
        SessionStore::with_backend(Box::new(self.backend(app)))
    }

    fn folded(&self, app: &Path, session_id: &str) -> SessionState {
        read_session_state(&SessionLogStore::new(self.log_root(app)), session_id)
            .unwrap_or_else(|error| panic!("fold `{session_id}`: {error}"))
            .state
    }

    fn blob(&self, app: &Path, session_id: &str) -> String {
        let conn = rusqlite::Connection::open(self.db(app)).expect("open db");
        conn.query_row(
            "SELECT session_data FROM sessions WHERE conversation_id = ?1",
            rusqlite::params![session_id],
            |row| row.get::<_, String>(0),
        )
        .unwrap_or_else(|error| panic!("read blob: {error}"))
    }

    fn db_fingerprint(&self, app: &Path) -> String {
        use sha2::Digest;
        let db = self.db(app);
        let mut hasher = sha2::Sha256::new();
        let mut wal = db.as_os_str().to_owned();
        wal.push("-wal");
        for path in [db, PathBuf::from(wal)] {
            let bytes = std::fs::read(&path).unwrap_or_default();
            hasher.update((bytes.len() as u64).to_le_bytes());
            hasher.update(&bytes);
        }
        format!("{:x}", hasher.finalize())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn transcript_message(content: &str) -> serde_json::Value {
    serde_json::json!({ "role": "user", "content": content })
}

fn memory_entry(kind: &str, content: &str) -> LongTermMemoryRecord {
    LongTermMemoryRecord {
        kind: kind.to_string(),
        content: content.to_string(),
        source: "test".to_string(),
        updated_at_ms: 1_700_000_000_000,
    }
}

fn effect_evidence(id: &str) -> PersistedEffectEvidence {
    PersistedEffectEvidence {
        evidence_id: id.to_string(),
        effect_kind: "memory_write".to_string(),
        boundary: "turn_end".to_string(),
        target_session_id: Some(SESSION_ID.to_string()),
        source_history_node_id: None,
        target_summary: format!("summary-{id}"),
        persistence_ref: format!("ref-{id}"),
        replay_decision_basis: "test".to_string(),
        persisted_at_ms: 1_700_000_000_000,
        replay_required_if_missing: false,
    }
}

fn history_evidence(id: &str) -> HistoryStateHookEvidence {
    HistoryStateHookEvidence {
        evidence_id: id.to_string(),
        session_id: SESSION_ID.to_string(),
        boundary: "turn_commit".to_string(),
        command_kind: "CommitHistoryNode".to_string(),
        result_kind: "applied".to_string(),
        summary: format!("evidence-{id}"),
        elapsed_ms: 3,
        blocked: false,
        degraded: false,
        requested_node_id: None,
        requested_branch_id: None,
        resolved_node_id: None,
        resolved_branch_id: None,
        recorded_at_ms: 1_700_000_000_000,
    }
}

// ---------------------------------------------------------------------------
// 1. transcript 不再丢（且索引行仍是轻量行）
// ---------------------------------------------------------------------------

#[test]
fn transcript_survives_restart_while_the_index_row_stays_light() {
    let fixture = Fixture::new("transcript");
    let app = fixture.app_dir("app");
    let transcript = vec![transcript_message("第一轮原生"), transcript_message("第二轮原生")];
    {
        let mut store = fixture.store(&app);
        store
            .append_turn_fallible(
                Some(SESSION_ID),
                "问",
                "答",
                Some(transcript.clone()),
                Vec::new(),
            )
            .expect("append_turn 必须成功");
    }

    // 索引行仍是轻量行（blob 不承载内容），但日志侧已经带上 transcript。
    let value: serde_json::Value =
        serde_json::from_str(&fixture.blob(&app, SESSION_ID)).expect("blob json");
    assert_eq!(value["__index_only"], serde_json::json!(true), "索引行必须是轻量行");
    assert_eq!(
        value["history"].as_array().map(Vec::len),
        Some(0),
        "轻量行不得携带 history"
    );
    assert_eq!(
        value["providerNativeTranscript"].as_array().map(Vec::len),
        Some(0),
        "轻量行不得再承载 transcript（载体在日志里）"
    );

    // 重启：重新 with_backend 打开同一库（`SessionStore` 启动即走 load_store）。
    let _reloaded_store = fixture.store(&app);
    let loaded = fixture.backend(&app).load_store().expect("load_store 必须成功");
    let session = loaded
        .sessions_view()
        .get(SESSION_ID)
        .unwrap_or_else(|| panic!("重启后必须读回会话"));
    assert_eq!(
        session.provider_native_transcript, transcript,
        "transcript 必须逐字段一致（ADR 0102 前这条会读回空）"
    );
    assert_eq!(
        session.history.len(),
        2,
        "history 也必须完整（消息本就有载体）"
    );
    eprintln!(
        "[1] 重启后 transcript={} 条、history={} 条；索引行仍为轻量行",
        session.provider_native_transcript.len(),
        session.history.len()
    );
}

// ---------------------------------------------------------------------------
// 2. 记忆整份替换 / evidence 追加（语义区分）
// ---------------------------------------------------------------------------

#[test]
fn memory_snapshot_replaces_but_evidence_appends() {
    let fixture = Fixture::new("semantics");
    let app = fixture.app_dir("app");
    let backend = fixture.backend(&app);

    // 第一批：entries=[A]、evidence=[e1]、hook=[h1]、history_evidence=[ev1]
    let first = backend.persist_commands_batch(vec![PersistCommand::UpdateSessionMemory {
        epoch: 1,
        session_id: SESSION_ID.to_string(),
        entries: vec![memory_entry("fact", "记忆 A")],
        evidence: vec![effect_evidence("e1")],
        hook_trace_records: Vec::new(),
        history_state_evidence: vec![history_evidence("ev1")],
    }]);
    assert_eq!(first, PersistCommandOutcome::Succeeded, "第一批必须成功");

    // 第二批：entries 换成 [B1, B2]（**整份替换**）、evidence 换 [e2]、history_evidence 追加 [ev2]
    let second = backend.persist_commands_batch(vec![PersistCommand::UpdateSessionMemory {
        epoch: 1,
        session_id: SESSION_ID.to_string(),
        entries: vec![memory_entry("fact", "记忆 B1"), memory_entry("pref", "记忆 B2")],
        evidence: vec![effect_evidence("e2")],
        hook_trace_records: Vec::new(),
        history_state_evidence: vec![history_evidence("ev2")],
    }]);
    assert_eq!(second, PersistCommandOutcome::Succeeded, "第二批必须成功");

    let folded = fixture.folded(&app, SESSION_ID);
    let contents: Vec<&str> = folded
        .long_term_memory_entries
        .iter()
        .map(|entry| entry.content.as_str())
        .collect();
    assert_eq!(
        contents,
        vec!["记忆 B1", "记忆 B2"],
        "记忆三项必须**整份替换**（不得累加成 A+B1+B2）：{contents:?}"
    );
    assert_eq!(
        folded.memory_write_evidence.len(),
        1,
        "evidence 同样整份替换（第二次只剩 e2）"
    );
    assert_eq!(folded.memory_write_evidence[0].evidence_id, "e2");
    let evidence_ids: Vec<&str> = folded
        .history_state_evidence
        .iter()
        .map(|evidence| evidence.evidence_id.as_str())
        .collect();
    assert_eq!(
        evidence_ids,
        vec!["ev1", "ev2"],
        "`history_state_evidence` 必须**追加**（两次后累加）：{evidence_ids:?}"
    );
    eprintln!(
        "[2] 两次 UpdateSessionMemory：entries={:?}（替换）evidence={} 条（替换）history_evidence={:?}（追加）",
        contents,
        folded.memory_write_evidence.len(),
        evidence_ids
    );
}

// ---------------------------------------------------------------------------
// 3. 记忆四件套重启往返
// ---------------------------------------------------------------------------

#[test]
fn memory_and_evidence_round_trip_through_restart() {
    let fixture = Fixture::new("memory_roundtrip");
    let app = fixture.app_dir("memory_roundtrip_app");
    let entries = vec![memory_entry("fact", "长期事实"), memory_entry("pref", "偏好")];
    let evidence = vec![effect_evidence("e-round")];
    {
        let backend = fixture.backend(&app);
        assert_eq!(
            backend.persist_commands_batch(vec![PersistCommand::UpdateSessionMemory {
                epoch: 1,
                session_id: SESSION_ID.to_string(),
                entries: entries.clone(),
                evidence: evidence.clone(),
                hook_trace_records: Vec::new(),
                history_state_evidence: vec![history_evidence("ev-round")],
            }]),
            PersistCommandOutcome::Succeeded
        );
    }
    let source = fixture.folded(&app, SESSION_ID);

    // 重启：重新打开库并读日志（命令流重放）。
    let reloaded = fixture.backend(&app);
    let loaded = reloaded.load_store().expect("load_store");
    let session = loaded.sessions_view().get(SESSION_ID).expect("session");
    assert_eq!(session.long_term_memory_entries, entries, "记忆条目必须逐字段一致");
    assert_eq!(session.memory_write_evidence, evidence, "写入证据必须逐字段一致");
    assert_eq!(
        session.history_state_evidence, source.history_state_evidence,
        "history_state_evidence 必须逐字段一致"
    );
    eprintln!(
        "[3] 重启往返：entries={} / evidence={} / history_evidence={}",
        session.long_term_memory_entries.len(),
        session.memory_write_evidence.len(),
        session.history_state_evidence.len()
    );
}

// ---------------------------------------------------------------------------
// 4. 编解码往返（encode → fold 解码）
// ---------------------------------------------------------------------------

#[test]
fn command_payloads_round_trip_through_encode_and_fold() {
    let fixture = Fixture::new("roundtrip");
    let app = fixture.app_dir("roundtrip_app");
    let transcript = vec![transcript_message("往返原生")];
    let entries = vec![memory_entry("fact", "往返记忆")];
    let evidence = vec![effect_evidence("e-rt")];
    let history_state = vec![history_evidence("ev-rt")];
    {
        let backend = fixture.backend(&app);
        assert_eq!(
            backend.persist_commands_batch(vec![
                PersistCommand::AppendProviderTranscript {
                    epoch: 1,
                    session_id: SESSION_ID.to_string(),
                    messages: transcript.clone(),
                },
                PersistCommand::UpdateSessionMemory {
                    epoch: 1,
                    session_id: SESSION_ID.to_string(),
                    entries: entries.clone(),
                    evidence: evidence.clone(),
                    hook_trace_records: Vec::new(),
                    history_state_evidence: history_state.clone(),
                },
            ]),
            PersistCommandOutcome::Succeeded,
            "写路径必须把两条命令编码进日志"
        );
    }

    // ① 日志行自描述：`type` 标签必须存在且可被无歧义识别。
    let log_path = SessionLogStore::new(fixture.log_root(&app))
        .find_log_path(SESSION_ID)
        .expect("日志文件");
    let raw = std::fs::read_to_string(&log_path).expect("read log");
    let kinds: Vec<String> = raw
        .lines()
        .skip(1)
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).expect("log line json");
            value["cmd"]["type"].as_str().unwrap_or_default().to_string()
        })
        .collect();
    assert!(kinds.contains(&"append_provider_transcript".to_string()), "日志行必须自描述：{kinds:?}");
    assert!(kinds.contains(&"update_session_memory".to_string()), "日志行必须自描述：{kinds:?}");

    // ② encode → fold 解码无损（`fold` 是命令流的权威解码器）。
    let store = SessionLogStore::new(fixture.log_root(&app));
    let meta = store
        .list_sessions()
        .expect("list")
        .into_iter()
        .find(|meta| meta.session_id == SESSION_ID)
        .expect("meta");
    let events: Vec<LogEvent> = read_log_events(&store, &meta).expect("read events");
    let folded = fold_session(&events).expect("fold 必须能解码这两类命令");
    assert_eq!(
        folded.state.provider_native_transcript, transcript,
        "transcript 编码往返必须无损"
    );
    assert_eq!(folded.state.long_term_memory_entries, entries, "记忆条目往返必须无损");
    assert_eq!(folded.state.memory_write_evidence, evidence, "写入证据往返必须无损");
    assert_eq!(
        folded.state.history_state_evidence, history_state,
        "history_state_evidence 往返必须无损"
    );
    eprintln!("[4] 日志行类型={kinds:?}；encode→fold 无损");
}

// ---------------------------------------------------------------------------
// 5. 日志写失败 ⇒ Failed 且 SQLite 零改动
// ---------------------------------------------------------------------------

#[test]
fn log_write_failure_leaves_sqlite_untouched() {
    let fixture = Fixture::new("failed_write");
    let app = fixture.app_dir("failed_write_app");
    // 先写一条正常会话，确保库里有内容、日志根也真实存在。
    {
        let backend = fixture.backend(&app);
        assert_eq!(
            backend.persist_commands_batch(vec![PersistCommand::AppendMessage {
                epoch: 1,
                session_id: SESSION_ID.to_string(),
                message: TurnHistoryMessage {
                    role: "user".to_string(),
                    content: "种子".to_string(),
                    ..Default::default()
                },
                ordinal: 0,
            }]),
            PersistCommandOutcome::Succeeded
        );
    }
    // 平台中立的反向守门：把日志根**换成一个普通文件** ⇒ 任何 create_dir_all 都失败。
    let log_root = fixture.log_root(&app);
    std::fs::remove_dir_all(&log_root).ok();
    std::fs::write(&log_root, b"not a directory").expect("occupy log root");

    let backend = fixture.backend(&app);
    let _ = backend.load_store();
    let before = fixture.db_fingerprint(&app);

    let outcome = backend.persist_commands_batch(vec![
        PersistCommand::AppendProviderTranscript {
            epoch: 1,
            session_id: SESSION_ID.to_string(),
            messages: vec![transcript_message("不该落盘的原生")],
        },
        PersistCommand::UpdateSessionMemory {
            epoch: 1,
            session_id: SESSION_ID.to_string(),
            entries: vec![memory_entry("fact", "不该落盘的记忆")],
            evidence: Vec::new(),
            hook_trace_records: Vec::new(),
            history_state_evidence: vec![history_evidence("ev-blocked")],
        },
    ]);
    let after = fixture.db_fingerprint(&app);
    assert_eq!(
        outcome,
        PersistCommandOutcome::Failed,
        "日志（真源）写不进去 ⇒ 整批必须 Failed（不得静默成功）"
    );
    assert_eq!(
        before, after,
        "日志失败时 SQLite 索引必须一个字节都没改（真源优先：失败即整体失败）"
    );
    eprintln!("[5] 日志根不可用：persist=Failed，db+wal sha256 未变");
}

// ---------------------------------------------------------------------------
// 6. 迁移联动：带 transcript 的 grandfather 会话迁移后逐字段一致
// ---------------------------------------------------------------------------

fn legacy_state_with_transcript(session_id: &str) -> SessionState {
    let history = vec![
        TurnHistoryMessage {
            role: "user".to_string(),
            content: "老会话问".to_string(),
            turn_id: Some("turn-1".to_string()),
            ..Default::default()
        },
        TurnHistoryMessage {
            role: "assistant".to_string(),
            content: "老会话答".to_string(),
            turn_id: Some("turn-1".to_string()),
            ..Default::default()
        },
    ];
    SessionState {
        conversation_id: session_id.to_string(),
        title: "带 transcript 的老会话".to_string(),
        summary: "摘要".to_string(),
        history,
        provider_native_transcript: vec![
            transcript_message("老原生一"),
            transcript_message("老原生二"),
        ],
        turn_trace_history: vec![TurnTraceRecord {
            turn_id: "turn-1".to_string(),
            title: "turn-1".to_string(),
            phase: "completed".to_string(),
            updated_at: 1_700_000_000_100,
            ..Default::default()
        }],
        turn_count: 1,
        last_referenced_file: Some("src/legacy.rs".to_string()),
        updated_at_ms: 1_700_000_000_200,
        trace_migration_state: Default::default(),
        turn_trace_refs: None,
        long_term_memory_entries: vec![memory_entry("fact", "老记忆")],
        memory_write_evidence: vec![effect_evidence("e-legacy")],
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: vec![history_evidence("ev-legacy")],
        history_nodes: Vec::new(),
        history_branches: Vec::new(),
        // fold 会把 cursor.session_id 归一为行键（既有口径），种子按同一形态给值。
        history_cursor: HistoryCursor {
            session_id: session_id.to_string(),
            ..Default::default()
        },
        event_watermark: 0,
        last_commit_watermark: 0,
        workspace_id: None,
        title_override: None,
        archived: false,
    }
}

#[test]
fn migration_now_reproduces_transcript_and_memory() {
    let fixture = Fixture::new("migration");
    let app = fixture.app_dir("migration_app");
    let source = legacy_state_with_transcript(SESSION_ID);
    {
        let backend = fixture.backend(&app);
        assert!(
            backend.upsert_session(SESSION_ID, &source),
            "种子：grandfather 会话必须写入成功（无日志）"
        );
    }
    assert!(
        SessionLogStore::new(fixture.log_root(&app))
            .find_log_path(SESSION_ID)
            .is_none(),
        "前置：迁移前该会话必须没有日志"
    );

    let backend = fixture.backend(&app);
    let report = migrate_grandfathered_sessions(&backend, MigrateOptions::apply()).expect("migrate");
    assert_eq!(report.migrated, vec![SESSION_ID.to_string()], "{report:?}");
    assert!(
        report.mismatched.is_empty(),
        "带 transcript 的会话**不得**再进 mismatched：{:?}",
        report.mismatched
    );

    let folded = fixture.folded(&app, SESSION_ID);
    let issues: Vec<String> = pony_agent_core::agent::session::session_parity_issues(&source, &folded)
        .into_iter()
        .filter(|issue| {
            let field = issue.split(':').next().unwrap_or_default();
            !MIGRATION_PARITY_EXEMPTIONS.iter().any(|exempt| field == *exempt)
        })
        .collect();
    assert!(issues.is_empty(), "迁移后 fold 与 blob 必须逐字段一致：{issues:#?}");
    assert_eq!(
        folded.provider_native_transcript, source.provider_native_transcript,
        "transcript 必须随迁移进入真源"
    );
    assert_eq!(
        folded.long_term_memory_entries, source.long_term_memory_entries,
        "记忆条目必须随迁移进入真源"
    );
    assert_eq!(
        folded.history_state_evidence, source.history_state_evidence,
        "history_state_evidence 必须随迁移进入真源"
    );
    eprintln!(
        "[6] 迁移联动：{} migrated，mismatched={}；transcript={} 条 / 记忆={} 条 / 证据={} 条 全部一致",
        report.migrated.len(),
        report.mismatched.len(),
        folded.provider_native_transcript.len(),
        folded.long_term_memory_entries.len(),
        folded.history_state_evidence.len()
    );
}
