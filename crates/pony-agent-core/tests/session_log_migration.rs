//! ADR 0099 **阶段 3-B** 验收：存量会话日志回填（grandfather 转正，方案 A）。
//!
//! 八类用例（全部真断言，无「跑通即放行」）：
//!
//! 1. [`migration_reproduces_the_blob_state_field_by_field`] —— 3 个只有 blob 的老会话
//!    （多轮消息 + trace + 分支 + cursor + 节点）迁移后，`fold_session` 与 blob 状态
//!    **逐字段一致**；
//! 2. [`migration_is_idempotent`] —— 连跑两次，第二次 `migrated=0 / skipped_has_log=全部`；
//! 3. [`sessions_with_a_real_log_are_never_rewritten`] —— 已有原始日志的会话文件 sha256
//!    前后一致（连 `--force` 也不覆盖原始过程流）；
//! 4. [`unfaithful_session_is_mismatched_and_leaves_no_trace`] —— 无法忠实合成的会话
//!    （同 turn_id 同角色消息会被日志去重语义折叠）⇒ 计入 `mismatched`、**无日志文件**、
//!    blob 行逐字节不变；
//! 5. [`dry_run_writes_nothing`] —— 日志根文件清单 + db/wal sha256 前后一致；
//! 6. [`migrated_sessions_survive_index_row_deletion`] —— **转正证明**：迁移后删掉索引行，
//!    运行时 `load_store()` 仍返回完整会话（此前 grandfather 会直接丢）；
//! 7. [`migrated_header_is_tagged_and_normal_writes_are_not`] —— 迁移产物 header 带
//!    `source.kind = migrated-from-blob`，正常写入路径该字段缺失；
//! 8. [`migrated_log_is_readable_by_old_readers_and_exportable`] —— `read_header` 仍归
//!    `Current`，且 bundle 导出 → 新目录导入可往返（旧读方兼容）。
//!
//! 隔离：所有库、日志与包都落在 `std::env::temp_dir()` 下的唯一目录，**绝不触碰**
//! `~/.local/share/PonyAgent/`；fixture `Drop` 清理。

use std::path::{Path, PathBuf};

use pony_agent_core::agent::session::{
    export_session, import_bundle, migrate_grandfathered_sessions, read_header, read_header_source,
    read_session_state, session_log_root_for_db, BundleFormat, HeaderClass, HistoryBranch,
    HistoryCursor, HistoryNode, HistoryNodeKind, ImportOptions, MigrateOptions,
    SeparateTraceTableMode, SessionBackend, SessionLogStore, SessionState, SessionStore,
    TraceMigrationState, TurnHistoryMessage, TurnTraceRecord, SESSION_LOG_FILE_NAME,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let unique = format!(
            "pony-0099-3b-migrate-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before unix epoch")
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
        SqliteSessionBackend::new_with_trace_mode(
            self.db(app),
            SeparateTraceTableMode::WriteSeparate,
        )
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
        .unwrap_or_else(|error| panic!("read blob `{session_id}`: {error}"))
    }

    /// 会话日志文件路径（不关心 cwd 分组：按 id 扫日志根）。
    fn log_path(&self, app: &Path, session_id: &str) -> Option<PathBuf> {
        let store = SessionLogStore::new(self.log_root(app));
        let meta = store
            .list_sessions()
            .ok()?
            .into_iter()
            .find(|meta| meta.session_id == session_id)?;
        store.log_path(session_id, &meta.cwd).ok()
    }

    /// 日志根文件清单 + mtime + 大小（dry-run 零写入的物理凭证）。
    fn tree_fingerprint(&self, root: &Path) -> Vec<(String, u64, u128)> {
        let mut entries = Vec::new();
        if !root.exists() {
            return entries;
        }
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for item in std::fs::read_dir(&dir).expect("read dir").flatten() {
                let path = item.path();
                let meta = item.metadata().expect("metadata");
                if meta.is_dir() {
                    stack.push(path);
                    continue;
                }
                let relative = path
                    .strip_prefix(root)
                    .expect("strip root")
                    .to_string_lossy()
                    .replace('\\', "/");
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|duration| duration.as_nanos())
                    .unwrap_or_default();
                entries.push((relative, meta.len(), mtime));
            }
        }
        entries.sort();
        entries
    }

    fn db_fingerprint(&self, db: &Path) -> String {
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        let mut wal = db.as_os_str().to_owned();
        wal.push("-wal");
        for path in [db.to_path_buf(), PathBuf::from(wal)] {
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

fn user_message(content: &str, turn_id: Option<&str>) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: "user".to_string(),
        content: content.to_string(),
        turn_id: turn_id.map(str::to_string),
        ..Default::default()
    }
}

/// 造一个「只有 blob」的 grandfathered 老会话（多轮消息 + trace + 分支 + cursor + 节点）。
fn legacy_state(session_id: &str, title: &str, workspace_id: Option<&str>) -> SessionState {
    let turn_ids = ["turn-a", "turn-b"];
    let mut history = Vec::new();
    for turn_id in turn_ids {
        history.push(user_message(&format!("{turn_id}-问"), Some(turn_id)));
        history.push(TurnHistoryMessage {
            role: "assistant".to_string(),
            content: format!("{turn_id}-答"),
            turn_id: Some(turn_id.to_string()),
            model_name: Some("mock".to_string()),
            token_count: Some(7),
            ..Default::default()
        });
    }
    let root_node = format!("{session_id}-legacy-root");
    let branch = format!("branch-main");
    let head = format!("{session_id}-node-1");
    let nodes = vec![
        HistoryNode {
            node_id: root_node.clone(),
            session_id: session_id.to_string(),
            parent_node_id: None,
            branch_id: branch.clone(),
            forked_from_node_id: None,
            kind: HistoryNodeKind::Checkpoint,
            title: title.to_string(),
            summary: "root".to_string(),
            history: Vec::new(),
            turn_count: 0,
            created_at_ms: 1_700_000_000_000,
            ..Default::default()
        },
        HistoryNode {
            node_id: head.clone(),
            session_id: session_id.to_string(),
            parent_node_id: Some(root_node.clone()),
            branch_id: branch.clone(),
            forked_from_node_id: None,
            kind: HistoryNodeKind::TurnCommitted,
            title: title.to_string(),
            summary: "head".to_string(),
            history: history.clone(),
            turn_count: 2,
            created_at_ms: 1_700_000_000_100,
            ..Default::default()
        },
    ];
    let cursor = HistoryCursor {
        session_id: session_id.to_string(),
        visible_node_id: Some(head.clone()),
        active_branch_id: Some(branch.clone()),
        branch_head_node_id: Some(head.clone()),
        workspace_node_id: Some(head.clone()),
        cursor_version: 3,
        event_watermark: 11,
        ..Default::default()
    };
    let branches = vec![HistoryBranch {
        branch_id: branch.clone(),
        session_id: session_id.to_string(),
        base_node_id: Some(root_node),
        head_node_id: Some(head),
        forked_from_branch_id: None,
        forked_from_node_id: None,
        label: "main".to_string(),
        created_at_ms: 1_700_000_000_000,
        updated_at_ms: 1_700_000_000_200,
    }];
    SessionState {
        conversation_id: session_id.to_string(),
        title: title.to_string(),
        summary: format!("{title} 摘要"),
        history,
        provider_native_transcript: Vec::new(),
        turn_trace_history: vec![TurnTraceRecord {
            turn_id: "turn-b".to_string(),
            title: "turn-b".to_string(),
            phase: "completed".to_string(),
            provider_name: Some("mock".to_string()),
            total_tokens: Some(14),
            updated_at: 1_700_000_000_300,
            ..Default::default()
        }],
        trace_migration_state: TraceMigrationState::LegacyBlob,
        turn_trace_refs: None,
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        turn_count: 2,
        last_referenced_file: Some("src/legacy.rs".to_string()),
        updated_at_ms: 1_700_000_000_400,
        history_nodes: nodes,
        history_branches: branches,
        history_cursor: cursor,
        event_watermark: 11,
        last_commit_watermark: 9,
        workspace_id: workspace_id.map(str::to_string),
        title_override: Some(title.to_string()),
        archived: false,
    }
}

fn seed_legacy(fixture: &Fixture, app: &Path, session_id: &str, title: &str, workspace: Option<&str>) {
    let backend = fixture.backend(app);
    assert!(
        backend.upsert_session(session_id, &legacy_state(session_id, title, workspace)),
        "种子：grandfathered 会话必须写入成功"
    );
}

fn contents(state: &SessionState) -> Vec<String> {
    state
        .history
        .iter()
        .map(|message| message.content.clone())
        .collect()
}

/// 逐字段对拍（与迁移器同一口径：只豁免 `workspace_id` / `last_commit_watermark`，
/// `trace_migration_state` / `turn_trace_refs` 由 `session_parity_issues` 自行跳过）。
fn assert_parity(source: &SessionState, imported: &SessionState, context: &str) {
    let issues: Vec<String> = pony_agent_core::agent::session::session_parity_issues(source, imported)
        .into_iter()
        .filter(|issue| {
            let field = issue.split(':').next().unwrap_or_default();
            !pony_agent_core::agent::session::MIGRATION_PARITY_EXEMPTIONS
                .iter()
                .any(|exempt| field == *exempt)
        })
        .collect();
    assert!(
        issues.is_empty(),
        "{context}：合成流与源状态必须逐字段一致，差异：{issues:#?}"
    );
    assert_eq!(contents(imported), contents(source), "{context}：history 必须一致");
    assert_eq!(imported.title, source.title, "{context}：title 必须一致");
    assert_eq!(imported.turn_count, source.turn_count, "{context}：turn_count 必须一致");
    let source_nodes: Vec<&str> = source
        .history_nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect();
    let imported_nodes: Vec<&str> = imported
        .history_nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect();
    assert_eq!(imported_nodes, source_nodes, "{context}：nodes 必须一致");
    let source_branches: Vec<&str> = source
        .history_branches
        .iter()
        .map(|branch| branch.branch_id.as_str())
        .collect();
    let imported_branches: Vec<&str> = imported
        .history_branches
        .iter()
        .map(|branch| branch.branch_id.as_str())
        .collect();
    assert_eq!(imported_branches, source_branches, "{context}：branches 必须一致");
    assert_eq!(
        serde_json::to_string(&imported.history_cursor).expect("cursor json"),
        serde_json::to_string(&source.history_cursor).expect("cursor json"),
        "{context}：cursor 必须一致"
    );
    let source_turns: Vec<&str> = source
        .turn_trace_history
        .iter()
        .map(|trace| trace.turn_id.as_str())
        .collect();
    let imported_turns: Vec<&str> = imported
        .turn_trace_history
        .iter()
        .map(|trace| trace.turn_id.as_str())
        .collect();
    assert_eq!(imported_turns, source_turns, "{context}：traces 必须一致");
}

// ---------------------------------------------------------------------------
// 1. 逐字段一致
// ---------------------------------------------------------------------------

#[test]
fn migration_reproduces_the_blob_state_field_by_field() {
    let fixture = Fixture::new("parity");
    let app = fixture.app_dir("app");
    let sessions = [
        ("pa_3b_legacy_a", "老会话 A", Some("ws-legacy")),
        ("pa_3b_legacy_b", "老会话 B", None),
        ("pa_3b_legacy_c", "老会话 C", None),
    ];
    for (session_id, title, workspace) in sessions {
        seed_legacy(&fixture, &app, session_id, title, workspace);
    }
    for (session_id, _, _) in sessions {
        assert!(
            fixture.log_path(&app, session_id).is_none(),
            "前置：{session_id} 在迁移前必须没有日志（否则本用例证明不了转正）"
        );
    }

    let backend = fixture.backend(&app);
    let report = migrate_grandfathered_sessions(&backend, MigrateOptions::apply())
        .expect("迁移必须成功");
    assert_eq!(report.scanned, 3);
    assert_eq!(report.migrated.len(), 3, "3 个老会话都必须转正：{report:?}");
    assert!(report.mismatched.is_empty(), "不得有 mismatched：{:?}", report.mismatched);
    assert!(report.failed.is_empty(), "不得有 failed：{:?}", report.failed);
    assert_eq!(report.exit_code(), 0);

    for (session_id, title, workspace) in sessions {
        let source = legacy_state(session_id, title, workspace);
        let folded = fixture.folded(&app, session_id);
        assert_parity(&source, &folded, session_id);
        let blob = fixture.blob(&app, session_id);
        let blob_state: SessionState =
            serde_json::from_str(&blob).unwrap_or_else(|error| panic!("parse blob: {error}"));
        assert_parity(&blob_state, &folded, &format!("{session_id}（对 blob 行）"));
        eprintln!(
            "[1] {session_id}: 合成流 fold 后与 blob 逐字段一致（history={} nodes={} branches={} traces={}）",
            folded.history.len(),
            folded.history_nodes.len(),
            folded.history_branches.len(),
            folded.turn_trace_history.len()
        );
    }
}

// ---------------------------------------------------------------------------
// 2. 幂等
// ---------------------------------------------------------------------------

#[test]
fn migration_is_idempotent() {
    let fixture = Fixture::new("idempotent");
    let app = fixture.app_dir("app");
    seed_legacy(&fixture, &app, "pa_3b_idem_a", "幂等 A", None);
    seed_legacy(&fixture, &app, "pa_3b_idem_b", "幂等 B", None);

    let backend = fixture.backend(&app);
    let first = migrate_grandfathered_sessions(&backend, MigrateOptions::apply()).expect("first");
    assert_eq!(first.migrated.len(), 2);
    let log_paths: Vec<PathBuf> = ["pa_3b_idem_a", "pa_3b_idem_b"]
        .iter()
        .map(|id| fixture.log_path(&app, id).expect("log exists"))
        .collect();
    let before: Vec<Vec<u8>> = log_paths.iter().map(|p| std::fs::read(p).unwrap()).collect();

    let second = migrate_grandfathered_sessions(&backend, MigrateOptions::apply()).expect("second");
    assert!(
        second.migrated.is_empty(),
        "第二次不得再迁移：{:?}",
        second.migrated
    );
    assert_eq!(second.skipped_has_log, 2, "第二次必须全部跳过");
    assert_eq!(second.scanned, 2);
    let after: Vec<Vec<u8>> = log_paths.iter().map(|p| std::fs::read(p).unwrap()).collect();
    assert_eq!(before, after, "第二次跑不得改动任何日志字节");
    eprintln!(
        "[2] 第一次 migrated={:?}；第二次 {:?}",
        first.migrated,
        second.summary_line()
    );
}

// ---------------------------------------------------------------------------
// 3. 不覆盖已有日志
// ---------------------------------------------------------------------------

#[test]
fn sessions_with_a_real_log_are_never_rewritten() {
    use pony_agent_core::agent::session::bundle::sha256_hex;
    let fixture = Fixture::new("no_overwrite");
    let app = fixture.app_dir("app");
    // 原始过程流会话：走真实写路径（append_turn）。
    {
        let mut store = SessionStore::with_backend(Box::new(fixture.backend(&app)));
        store
            .append_turn_fallible(Some("pa_3b_real_log"), "问", "答", None, Vec::new())
            .expect("append_turn");
    }
    // 另一个只有 blob 的老会话，用来证明迁移确实跑起来了。
    seed_legacy(&fixture, &app, "pa_3b_legacy_only", "只有 blob", None);
    let real_log = fixture.log_path(&app, "pa_3b_real_log").expect("real log");
    let before = sha256_hex(&std::fs::read(&real_log).expect("read real log"));

    let backend = fixture.backend(&app);
    let report = migrate_grandfathered_sessions(&backend, MigrateOptions::apply()).expect("migrate");
    assert_eq!(report.skipped_has_log, 1, "已有日志的会话必须跳过");
    assert_eq!(report.migrated, vec!["pa_3b_legacy_only".to_string()]);
    let after = sha256_hex(&std::fs::read(&real_log).expect("read real log"));
    assert_eq!(before, after, "已有原始日志的字节不得被改动");

    // 即使 `--force`：原始过程流也不得被覆盖（force 只重跑迁移产物）。
    let forced = migrate_grandfathered_sessions(
        &backend,
        MigrateOptions {
            dry_run: false,
            force: true,
        },
    )
    .expect("force migrate");
    assert_eq!(
        forced.skipped_has_log, 1,
        "force 也不得覆盖原始过程流：{forced:?}"
    );
    assert_eq!(
        sha256_hex(&std::fs::read(&real_log).expect("read real log")),
        before,
        "force 之后原始日志字节仍必须一致"
    );
    eprintln!("[3] 原始日志 sha256 前后一致：{before}");
}

// ---------------------------------------------------------------------------
// 4. 无法忠实合成 ⇒ mismatched 且不落盘
// ---------------------------------------------------------------------------

#[test]
fn unfaithful_session_is_mismatched_and_leaves_no_trace() {
    let fixture = Fixture::new("mismatched");
    let app = fixture.app_dir("app");
    let session_id = "pa_3b_unfaithful";
    // 同一 turn_id + 同一 role 的两条消息：日志语义里 `message_id = {turn_id}-{role}`
    // 会去重成一条 ⇒ 合成流无法复现 2 条消息，必须诚实判 mismatched。
    let mut state = legacy_state(session_id, "无法忠实合成", None);
    let duplicate = user_message("重复的同一轮用户消息", Some("turn-a"));
    state.history.insert(1, duplicate);
    let blob_before = {
        let backend = fixture.backend(&app);
        assert!(backend.upsert_session(session_id, &state));
        fixture.blob(&app, session_id)
    };

    let backend = fixture.backend(&app);
    let report = migrate_grandfathered_sessions(&backend, MigrateOptions::apply()).expect("migrate");
    assert_eq!(report.migrated.len(), 0, "不得把不忠实的会话算作成功");
    assert_eq!(report.mismatched.len(), 1, "必须计入 mismatched：{report:?}");
    let (reported_id, reason) = &report.mismatched[0];
    assert_eq!(reported_id, session_id);
    assert!(
        reason.contains("history") || reason.contains("不一致"),
        "mismatched 原因必须指明差异：{reason}"
    );
    assert_eq!(report.exit_code(), 5, "存在 mismatched ⇒ EXIT=5");
    assert!(
        fixture.log_path(&app, session_id).is_none(),
        "mismatched 的会话**不得**生成日志文件"
    );
    assert_eq!(
        fixture.blob(&app, session_id),
        blob_before,
        "mismatched 不得改动原数据（blob 行逐字节不变）"
    );
    eprintln!("[4] mismatched：{session_id} → {reason}");
}

// ---------------------------------------------------------------------------
// 5. dry-run 零写入
// ---------------------------------------------------------------------------

#[test]
fn dry_run_writes_nothing() {
    let fixture = Fixture::new("dry_run");
    let app = fixture.app_dir("app");
    seed_legacy(&fixture, &app, "pa_3b_dry_a", "dry A", None);
    seed_legacy(&fixture, &app, "pa_3b_dry_b", "dry B", None);

    let log_root_before = fixture.tree_fingerprint(&fixture.log_root(&app));
    let db_before = fixture.db_fingerprint(&fixture.db(&app));
    let backend = fixture.backend(&app);
    let report = migrate_grandfathered_sessions(&backend, MigrateOptions::dry_run()).expect("dry");
    let log_root_after = fixture.tree_fingerprint(&fixture.log_root(&app));
    let db_after = fixture.db_fingerprint(&fixture.db(&app));

    assert!(report.dry_run, "报告必须标注 dry-run");
    assert_eq!(report.migrated.len(), 2, "预演也必须逐个校验并报告：{report:?}");
    assert_eq!(
        log_root_before, log_root_after,
        "dry-run 必须零写入：日志根文件清单/大小/mtime 前后一致"
    );
    assert_eq!(db_before, db_after, "dry-run 必须零写入：db+wal sha256 前后一致");
    assert!(
        fixture.log_path(&app, "pa_3b_dry_a").is_none(),
        "dry-run 不得生成日志文件"
    );
    eprintln!(
        "[5] dry-run：日志根 {} 项指纹一致，db+wal sha256 一致",
        log_root_after.len()
    );
}

// ---------------------------------------------------------------------------
// 6. 转正证明：删掉索引行仍可读
// ---------------------------------------------------------------------------

#[test]
fn migrated_sessions_survive_index_row_deletion() {
    let fixture = Fixture::new("promoted");
    let app = fixture.app_dir("app");
    let session_id = "pa_3b_promoted";
    seed_legacy(&fixture, &app, session_id, "待转正", None);
    let source = legacy_state(session_id, "待转正", None);

    // 迁移前：删掉索引行 ⇒ 会话就没了（证明这是真的 grandfather）。
    {
        let backend = fixture.backend(&app);
        let before = backend.load_store().expect("load_store");
        assert!(
            before.sessions_view().contains_key(session_id),
            "前置：迁移前该会话由 blob 提供"
        );
    }

    let backend = fixture.backend(&app);
    migrate_grandfathered_sessions(&backend, MigrateOptions::apply()).expect("migrate");
    assert!(fixture.log_path(&app, session_id).is_some(), "迁移后必须有日志");

    // 删掉索引行（只留日志）⇒ 运行时读路径仍必须返回完整会话。
    {
        let conn = rusqlite::Connection::open(fixture.db(&app)).expect("open db");
        let removed = conn
            .execute(
                "DELETE FROM sessions WHERE conversation_id = ?1",
                rusqlite::params![session_id],
            )
            .expect("delete row");
        assert_eq!(removed, 1, "必须真的删掉索引行");
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").ok();
    }
    let view = fixture
        .backend(&app)
        .load_store()
        .expect("load_store")
        .sessions_view()
        .clone();
    let session = view
        .get(session_id)
        .unwrap_or_else(|| panic!("删掉索引行后会话必须仍可读（文件即真相）"));
    assert_parity(&source, session, "删索引行后");
    assert_eq!(contents(session).len(), 4, "历史必须完整");
    eprintln!(
        "[6] 转正证明：DELETE sessions 行后仍读到 {} 条消息 / nodes={} / traces={}",
        session.history.len(),
        session.history_nodes.len(),
        session.turn_trace_history.len()
    );
}

// ---------------------------------------------------------------------------
// 7. 头标记
// ---------------------------------------------------------------------------

#[test]
fn migrated_header_is_tagged_and_normal_writes_are_not() {
    let fixture = Fixture::new("header_tag");
    let app = fixture.app_dir("app");
    seed_legacy(&fixture, &app, "pa_3b_tag_legacy", "合成日志", None);
    {
        let mut store = SessionStore::with_backend(Box::new(fixture.backend(&app)));
        store
            .append_turn_fallible(Some("pa_3b_tag_real"), "问", "答", None, Vec::new())
            .expect("append_turn");
    }

    let backend = fixture.backend(&app);
    migrate_grandfathered_sessions(&backend, MigrateOptions::apply()).expect("migrate");

    // 合成流：header 带 source 标记（文件原文 + 读取 API 两处都断言）。
    let migrated_log = fixture.log_path(&app, "pa_3b_tag_legacy").expect("migrated log");
    let raw_first_line = std::fs::read_to_string(&migrated_log)
        .expect("read log")
        .lines()
        .next()
        .expect("header line")
        .to_string();
    let header_value: serde_json::Value =
        serde_json::from_str(&raw_first_line).expect("header json");
    assert_eq!(
        header_value["source"]["kind"],
        serde_json::json!("migrated-from-blob"),
        "合成日志必须在 header 里自证身份：{raw_first_line}"
    );
    assert!(
        header_value["source"]["migratedAtMs"].as_i64().is_some(),
        "必须记录合成时刻：{raw_first_line}"
    );
    assert!(
        header_value["source"]["appVersion"].as_str().is_some(),
        "必须记录合成方版本：{raw_first_line}"
    );
    let source = read_header_source(&migrated_log)
        .expect("read header source")
        .expect("source marker");
    assert!(source.is_migrated_from_blob());

    // 正常写入路径：header 无 source 字段（保持 None）。
    let real_log = fixture.log_path(&app, "pa_3b_tag_real").expect("real log");
    let real_first_line = std::fs::read_to_string(&real_log)
        .expect("read log")
        .lines()
        .next()
        .expect("header line")
        .to_string();
    let real_value: serde_json::Value = serde_json::from_str(&real_first_line).expect("json");
    assert!(
        real_value.get("source").is_none(),
        "正常写入路径不得写 source 字段（否则就是伪装）：{real_first_line}"
    );
    assert!(
        read_header_source(&real_log).expect("read").is_none(),
        "正常写入路径的来源标记必须为 None"
    );
    eprintln!("[7] 合成 header={}", raw_first_line);
}

// ---------------------------------------------------------------------------
// 8. 旧读方兼容 + bundle 往返
// ---------------------------------------------------------------------------

#[test]
fn migrated_log_is_readable_by_old_readers_and_exportable() {
    let fixture = Fixture::new("compat");
    let app = fixture.app_dir("app");
    let session_id = "pa_3b_compat";
    seed_legacy(&fixture, &app, session_id, "兼容性", None);
    let source = legacy_state(session_id, "兼容性", None);
    let backend = fixture.backend(&app);
    migrate_grandfathered_sessions(&backend, MigrateOptions::apply()).expect("migrate");

    // ① 旧读方：`read_header` 仍归 Current（四态分类不变）。
    let log_path = fixture.log_path(&app, session_id).expect("log");
    assert_eq!(log_path.file_name().unwrap(), SESSION_LOG_FILE_NAME);
    let class = read_header(&log_path).expect("read header");
    assert!(
        matches!(class, HeaderClass::Current(_)),
        "带 source 字段的 header 仍必须归 Current（未知键忽略）：{class:?}"
    );
    let header = class.header().expect("header");
    assert_eq!(header.session_id, session_id);

    // ② bundle 导出（目录形态）→ 全新目录导入 → fold 一致。
    let bundle = fixture.root.join("bundle-dir");
    let export = export_session(
        &fixture.log_root(&app),
        session_id,
        &bundle,
        BundleFormat::Dir,
        "test-version",
        None,
    )
    .expect("export");
    assert_eq!(export.exit_code(), 0);
    let target_app = fixture.app_dir("target");
    let import = import_bundle(
        &fixture.log_root(&target_app),
        &bundle,
        ImportOptions::apply(),
    )
    .expect("import");
    assert_eq!(import.exit_code(), 0);
    let imported = fixture.folded(&target_app, session_id);
    assert_parity(&source, &imported, "bundle 往返");
    eprintln!(
        "[8] read_header={:?}；bundle events={} → 新目录 fold 与源逐字段一致",
        class.label(),
        export.event_count
    );
}

// ---------------------------------------------------------------------------
// 9. 跨 cwd 判定（D1）：日志在别的分组里也必须算「有日志」
// ---------------------------------------------------------------------------

/// 在指定 cwd 分组下写一条**最小**日志（模拟「会话在别的工作目录里创建」）。
fn write_log_in_cwd(fixture: &Fixture, app: &Path, session_id: &str, cwd: &str) {
    use pony_agent_core::agent::session::{SessionLogMeta, SessionLogStore};
    let store = SessionLogStore::new(fixture.log_root(app));
    let meta = SessionLogMeta::new(session_id, cwd);
    let mut writer = store
        .open_write(session_id, &meta)
        .unwrap_or_else(|error| panic!("open_write({session_id}@{cwd}): {error}"));
    writer
        .append_commands(
            1_700_000_000_000,
            &[serde_json::json!({
                "type": "append_message",
                "encodingVersion": 1,
                "epoch": 1,
                "sessionId": session_id,
                "message": {"role": "user", "content": "别的工作区里写的"},
                "ordinal": 0,
            })],
        )
        .expect("append");
}

fn group_dir(fixture: &Fixture, app: &Path, cwd: &str) -> PathBuf {
    fixture
        .log_root(app)
        .join(pony_agent_core::agent::session::normalize_cwd_segment(cwd))
}

#[test]
fn session_has_log_is_cross_cwd_so_blob_writes_become_light_rows() {
    let fixture = Fixture::new("cross_cwd");
    let app = fixture.app_dir("app");
    let session_id = "pa_3b_cross_cwd";
    // 会话的日志建在**另一个** cwd 分组里（进程 cwd 是仓库根，两者必然不同）。
    let foreign_cwd = "/cwd/foreign/project";
    assert_ne!(
        foreign_cwd,
        pony_agent_core::agent::session::current_log_cwd(),
        "前置：外部分组必须与进程 cwd 不同，否则本用例证明不了跨分组查找"
    );
    write_log_in_cwd(&fixture, &app, session_id, foreign_cwd);
    let foreign_log = group_dir(&fixture, &app, foreign_cwd)
        .join(pony_agent_core::agent::session::encode_session_id_segment(session_id).expect("seg"))
        .join(SESSION_LOG_FILE_NAME);
    assert!(foreign_log.is_file(), "前置：外部分组日志必须存在");

    let backend = fixture.backend(&app);
    assert!(
        backend.has_session_log(session_id),
        "D1：日志在别的 cwd 分组里也必须判为「有日志」（否则会被当 grandfather 继续写整包 blob）"
    );

    // 2-D 写路径的后果：该会话的索引行必须是**轻量索引行**（不再写整包 blob）。
    let mut state = legacy_state(session_id, "跨工作区会话", None);
    state.history = vec![user_message("一", Some("t1")), user_message("二", Some("t2"))];
    assert!(backend.upsert_session(session_id, &state), "upsert 必须成功");
    let blob = fixture.blob(&app, session_id);
    let value: serde_json::Value = serde_json::from_str(&blob).expect("blob json");
    assert_eq!(
        value["__index_only"], serde_json::json!(true),
        "跨 cwd 有日志 ⇒ 2-D 必须写轻量索引行（修复前会写整包 blob）：{blob}"
    );
    assert_eq!(
        value["history"].as_array().map(Vec::len),
        Some(0),
        "轻量行不得携带 history"
    );
    eprintln!("[9] 跨 cwd：has_session_log=true，索引行=轻量行（历史 0 条）");
}

// ---------------------------------------------------------------------------
// 10. 迁移不制造重复分组（D2）
// ---------------------------------------------------------------------------

#[test]
fn migration_never_creates_a_duplicate_group_for_sessions_that_already_have_logs() {
    use pony_agent_core::agent::session::bundle::sha256_hex;
    use pony_agent_core::agent::session::{encode_session_id_segment, SessionLogStore};
    let fixture = Fixture::new("no_dup_group");
    let app = fixture.app_dir("app");
    let session_id = "pa_3b_dup_group";
    let foreign_cwd = "/cwd/foreign/group-a";
    write_log_in_cwd(&fixture, &app, session_id, foreign_cwd);
    let segment = encode_session_id_segment(session_id).expect("segment");
    let foreign_log = group_dir(&fixture, &app, foreign_cwd)
        .join(&segment)
        .join(SESSION_LOG_FILE_NAME);
    let before = sha256_hex(&std::fs::read(&foreign_log).expect("read foreign log"));
    // 再放一个真的要迁移的 grandfather 会话，证明迁移循环确实跑过。
    seed_legacy(&fixture, &app, "pa_3b_dup_group_legacy", "要迁移的", None);

    let backend = fixture.backend(&app);
    let report = migrate_grandfathered_sessions(&backend, MigrateOptions::apply()).expect("migrate");
    assert_eq!(report.skipped_has_log, 1, "已有日志的会话必须计入 skipped：{report:?}");
    assert_eq!(report.migrated, vec!["pa_3b_dup_group_legacy".to_string()]);

    // 进程 cwd 分组里**不得**为它新建目录，也不得有任何新日志。
    let process_cwd = pony_agent_core::agent::session::current_log_cwd();
    let process_group = group_dir(&fixture, &app, &process_cwd);
    let duplicated = process_group.join(&segment);
    assert!(
        !duplicated.exists(),
        "D2：迁移不得在进程 cwd 分组里为已有日志的会话再造一份：{}",
        duplicated.display()
    );
    assert!(
        SessionLogStore::new(fixture.log_root(&app))
            .find_log_path(session_id)
            .is_some_and(|path| path.starts_with(group_dir(&fixture, &app, foreign_cwd))),
        "该会话的日志必须仍在原分组里"
    );
    assert_eq!(
        sha256_hex(&std::fs::read(&foreign_log).expect("read foreign log")),
        before,
        "原分组里的日志字节不得被改动"
    );
    eprintln!(
        "[10] 迁移后分组：{:?}（无 --{}-- 重复）",
        list_groups(&fixture, &app),
        pony_agent_core::agent::session::normalize_cwd_segment(&process_cwd)
    );
}

/// 日志根下的分组目录名（排序）。
fn list_groups(fixture: &Fixture, app: &Path) -> Vec<String> {
    let root = fixture.log_root(app);
    let mut groups = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&root) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                groups.push(entry.file_name().to_string_lossy().into_owned());
            }
        }
    }
    groups.sort();
    groups
}

// ---------------------------------------------------------------------------
// 11. 迁移 cwd 归属（D2）：workspace 能解析就落到 workspace root 分组
// ---------------------------------------------------------------------------

#[test]
fn migration_attributes_cwd_by_workspace_and_records_fallbacks() {
    use pony_agent_core::agent::session::{encode_session_id_segment, SessionStore};
    let fixture = Fixture::new("cwd_attribution");
    let app = fixture.app_dir("app");
    let workspace_root = fixture.root.join("ws-root");
    std::fs::create_dir_all(&workspace_root).expect("create workspace root");

    // 产品在 `create_workspace` 里**规范化** root（`canonicalize` + Windows 去 `\\?\` 前缀，
    // 见 `agent/workspace.rs::normalize_workspace_root`）。Windows 的临时目录是 8.3 短名
    // （`C:\Users\RUNNER~1\…`），canonicalize 会把它展开成长名 ⇒ 期望分组必须用**记录里的**
    // root_path，而不是入参字符串（B051：用入参会把「归属正确」误判成红，且只在 Windows 暴露）。
    let (workspace_id, workspace_root_recorded) = {
        let mut store = SessionStore::with_backend(Box::new(fixture.backend(&app)));
        let record = store
            .create_workspace("归属测试", workspace_root.to_string_lossy().as_ref())
            .expect("create_workspace");
        (record.id, record.root_path)
    };

    let attributed = "pa_3b_ws_attributed";
    let orphan = "pa_3b_ws_orphan";
    {
        let backend = fixture.backend(&app);
        let mut attributed_state = legacy_state(attributed, "有 workspace", Some(&workspace_id));
        attributed_state.workspace_id = Some(workspace_id.clone());
        assert!(backend.upsert_session(attributed, &attributed_state));
        // 死 workspace id（注册表里没有）⇒ 必须走 cwd_fallback（None 视为默认工作区，
        // 不属异常，见 resolve_session_cwd 的口径说明）。
        let mut dead_state = legacy_state(orphan, "死 workspace id", Some("ws-dead-unknown"));
        dead_state.workspace_id = Some("ws-dead-unknown".to_string());
        assert!(backend.upsert_session(orphan, &dead_state));
    }

    let backend = fixture.backend(&app);
    let report = migrate_grandfathered_sessions(&backend, MigrateOptions::apply()).expect("migrate");
    assert_eq!(report.migrated.len(), 2, "{report:?}");
    assert_eq!(
        report.cwd_fallback,
        vec![orphan.to_string()],
        "workspace id 在注册表里解析不到 ⇒ 必须逐条记入 cwd_fallback（不得静默）：{report:?}"
    );

    assert!(
        !workspace_root_recorded.trim().is_empty(),
        "前置：workspace 记录必须带 root_path（迁移按它归属 cwd 分组）"
    );
    let ws_group = group_dir(&fixture, &app, &workspace_root_recorded);
    let attributed_segment = encode_session_id_segment(attributed).expect("segment");
    assert!(
        ws_group
            .join(&attributed_segment)
            .join(SESSION_LOG_FILE_NAME)
            .is_file(),
        "D2：能解析 workspace 的会话必须落在 workspace root 对应的分组（{}）",
        ws_group.display()
    );
    let process_cwd = pony_agent_core::agent::session::current_log_cwd();
    let orphan_segment = encode_session_id_segment(orphan).expect("segment");
    assert!(
        group_dir(&fixture, &app, &process_cwd)
            .join(&orphan_segment)
            .join(SESSION_LOG_FILE_NAME)
            .is_file(),
        "解析不到归属的会话退回进程 cwd 分组"
    );
    eprintln!(
        "[11] 分组归属：{} → {}；{} → 进程 cwd（cwd_fallback={:?}）",
        attributed,
        ws_group.file_name().unwrap().to_string_lossy(),
        orphan,
        report.cwd_fallback
    );
}

// ---------------------------------------------------------------------------
// 12. 计数稳定（D3）：同一快照连续两次 dry-run 结果完全一致
// ---------------------------------------------------------------------------

#[test]
fn dry_run_counts_are_stable_across_runs() {
    let fixture = Fixture::new("count_stable");
    let app = fixture.app_dir("app");
    seed_legacy(&fixture, &app, "pa_3b_cnt_a", "可迁移 A", None);
    seed_legacy(&fixture, &app, "pa_3b_cnt_b", "可迁移 B", None);
    // 一个无法忠实合成的会话 + 一个已有日志的会话，让三类计数都非零。
    let mut unfaithful = legacy_state("pa_3b_cnt_bad", "不忠实", None);
    unfaithful
        .history
        .insert(1, user_message("重复同轮消息", Some("turn-a")));
    {
        let backend = fixture.backend(&app);
        assert!(backend.upsert_session("pa_3b_cnt_bad", &unfaithful));
    }
    write_log_in_cwd(&fixture, &app, "pa_3b_cnt_haslog", "/cwd/elsewhere");

    let backend = fixture.backend(&app);
    let first = migrate_grandfathered_sessions(&backend, MigrateOptions::dry_run()).expect("dry 1");
    let second = migrate_grandfathered_sessions(&backend, MigrateOptions::dry_run()).expect("dry 2");

    assert_eq!(first.migrated, second.migrated, "两次 dry-run 的 migrated 必须一致");
    assert_eq!(first.mismatched, second.mismatched, "两次 dry-run 的 mismatched 必须一致");
    assert_eq!(first.skipped_has_log, second.skipped_has_log);
    assert_eq!(first.scanned, second.scanned);
    assert_eq!(first.cwd_fallback, second.cwd_fallback);
    assert_eq!(first.summary_line(), second.summary_line());
    assert_eq!(first.migrated.len(), 2, "{first:?}");
    assert_eq!(first.mismatched.len(), 1, "{first:?}");
    assert_eq!(first.skipped_has_log, 1, "{first:?}");
    assert!(
        fixture.log_path(&app, "pa_3b_cnt_a").is_none(),
        "dry-run 不得落盘"
    );
    eprintln!("[12] 两次 dry-run 完全一致：{}", second.summary_line());
}
