//! ADR 0099 **阶段 3-D** 验收：`rebuild-index` 的比较口径（单一豁免清单）+ `--explain` 诊断。
//!
//! 单独成文件的原因：这些用例会额外创建 backend/subprocess（`--explain` CLI 用例），
//! 与本目录里既有的 fold/rebuild 性能敏感用例放在同一测试二进制会显著抬高进程内并发，
//! 触发既有的单实例 flock 竞态（实测偶发 `single_instance_lock_denied`）。独立二进制
//! 既保留用例强度，又不影响既有套件稳定性。
//!
//! 五类用例：
//! 1. [`legacy_blob_trace_form_is_not_a_mismatch`] —— trace 存独立表、blob 内嵌为空属约定；
//! 2. [`real_content_difference_is_still_reported_as_mismatch`] —— 真差异仍必须判 mismatch；
//! 3. [`log_uncarried_fields_are_exempt_but_listed_by_explain`] —— 豁免清单只有一处定义；
//! 4. [`explain_prints_field_level_summaries_without_full_payloads`] —— 摘要格式可断言；
//! 5. [`explain_cli_is_read_only_and_mutually_exclusive_with_dry_run`] —— CLI `--explain`。

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use pony_agent_core::agent::session::{
    rebuild_index, read_session_state, session_log_root_for_db, session_parity_diff,
    session_parity_issues, RebuildOptions, SeparateTraceTableMode, SessionLogStore, SessionState,
    SessionStore, TurnTraceRecord, MIGRATION_PARITY_EXEMPTIONS,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;


// ---------------------------------------------------------------------------
// 夹具（与原 `session_fold_rebuild.rs` 同构的紧凑版）
// ---------------------------------------------------------------------------

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let unique = format!(
            "pony-3d-compare-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let root = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&root).expect("create fixture root");
        Self { root }
    }

    fn db(&self) -> PathBuf {
        self.root.join("sessions.db")
    }

    fn log_root(&self) -> PathBuf {
        session_log_root_for_db(&self.db())
    }

    fn backend(&self) -> SqliteSessionBackend {
        SqliteSessionBackend::new_with_trace_mode(self.db(), SeparateTraceTableMode::WriteSeparate)
    }

    fn store(&self) -> SessionStore {
        SessionStore::with_backend(Box::new(self.backend()))
    }

    fn folded(&self, session_id: &str) -> SessionState {
        read_session_state(&SessionLogStore::new(self.log_root()), session_id)
            .unwrap_or_else(|error| panic!("fold `{session_id}`: {error}"))
            .state
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// ---------------------------------------------------------------------------
// ADR 0099 阶段 3-D：rebuild-index 的比较口径（单一豁免清单）+ `--explain` 诊断
// ---------------------------------------------------------------------------


/// 把某会话的索引行改写成 `legacy_blob` 形态：**trace 存独立表、blob 内嵌 trace 为空**
/// （这正是真实库里 46 个「计数全一致、只有 trace 面不同」的形态）。
fn rewrite_row_as_legacy_blob(fixture: &Fixture, session_id: &str, state: &SessionState) {
    let mut blob_state = state.clone();
    blob_state.turn_trace_history = Vec::new();
    blob_state.trace_migration_state = pony_agent_core::agent::session::TraceMigrationState::LegacyBlob;
    blob_state.turn_trace_refs = None;
    let conn = rusqlite::Connection::open(fixture.db()).expect("open db");
    conn.execute(
        "UPDATE sessions SET session_data = ?2 WHERE conversation_id = ?1",
        rusqlite::params![session_id, serde_json::to_string(&blob_state).expect("encode")],
    )
    .expect("rewrite row");
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").ok();
}

#[test]
fn legacy_blob_trace_form_is_not_a_mismatch() {
    let fixture = Fixture::new("legacy_blob_trace");
    let session_id = "pa_3d_legacy_trace";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(session_id), "问", "答", None, Vec::new())
            .expect("append_turn");
        store.record_turn_trace(
            Some(session_id),
            TurnTraceRecord {
                turn_id: "turn-0".to_string(),
                title: "t".to_string(),
                phase: "completed".to_string(),
                ..Default::default()
            },
        );
    }
    let folded = fixture.folded(session_id);
    assert_eq!(folded.turn_trace_history.len(), 1, "日志侧必须真有 append_trace");
    rewrite_row_as_legacy_blob(&fixture, session_id, &folded);

    // 索引侧：blob 的内嵌 trace 为空，但 trace 存在**独立表**里 —— `load_index_store`
    // 会按 `legacy_blob` 约定把表内 trace 合并进视图（这就是「唯一约定」的落地点）。
    let strict = SqliteSessionBackend::new_with_trace_mode(
        fixture.db(),
        SeparateTraceTableMode::WriteSeparate,
    );
    let index_view = strict.load_index_store().expect("index view");
    let indexed = index_view
        .sessions_view()
        .get(session_id)
        .expect("indexed session")
        .clone();
    let raw_row: String = rusqlite::Connection::open(fixture.db())
        .expect("db")
        .query_row(
            "SELECT session_data FROM sessions WHERE conversation_id = ?1",
            rusqlite::params![session_id],
            |row| row.get(0),
        )
        .expect("row");
    let raw_state: SessionState = serde_json::from_str(&raw_row).expect("row json");
    assert!(
        raw_state.turn_trace_history.is_empty(),
        "前置：blob 侧内嵌 trace 必须为空（真实库形态）"
    );
    assert_eq!(
        indexed.turn_trace_history.len(),
        1,
        "索引视图必须从独立表把 trace 合并回来（legacy_blob 约定）"
    );
    assert!(
        session_parity_diff(&folded, &indexed).is_none(),
        "trace 存独立表属既有约定，严格比对也不该报差异"
    );
    // rebuild-index 的 dry-run 结论是这条判定的公开观测面：
    let report = rebuild_index(&fixture.backend(), RebuildOptions::dry_run()).expect("rebuild");
    assert_eq!(report.rebuilt, vec![session_id.to_string()]);
    assert!(
        report.mismatched.is_empty(),
        "rebuild-index 不得把 legacy_blob trace 形态计为 mismatched：{:?}",
        report.mismatched
    );
}

#[test]
fn real_content_difference_is_still_reported_as_mismatch() {
    let fixture = Fixture::new("real_diff");
    let session_id = "pa_3d_real_diff";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(session_id), "问", "答", None, Vec::new())
            .expect("append_turn");
    }
    let folded = fixture.folded(session_id);
    // 索引侧少一条消息 ⇒ 真差异，必须仍被判为 mismatch（豁免不能过头）。
    let mut tampered = folded.clone();
    tampered.history.pop();
    let conn = rusqlite::Connection::open(fixture.db()).expect("open db");
    conn.execute(
        "UPDATE sessions SET session_data = ?2 WHERE conversation_id = ?1",
        rusqlite::params![session_id, serde_json::to_string(&tampered).expect("encode")],
    )
    .expect("tamper row");
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").ok();

    let report = rebuild_index(&fixture.backend(), RebuildOptions::dry_run()).expect("rebuild");
    assert_eq!(
        report.mismatched,
        vec![session_id.to_string()],
        "history 条数不同必须仍判 mismatch：{report:?}"
    );
    let issues = session_parity_issues(&folded, &tampered);
    assert!(
        issues.iter().any(|issue| issue.starts_with("history.len:")),
        "差异必须具体到字段：{issues:?}"
    );
    eprintln!("[real-diff] issues={issues:?}");
}

#[test]
fn log_uncarried_fields_are_exempt_but_listed_by_explain() {
    let fixture = Fixture::new("uncarried");
    let session_id = "pa_3d_uncarried";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(session_id), "问", "答", None, Vec::new())
            .expect("append_turn");
    }
    let folded = fixture.folded(session_id);
    let mut indexed = folded.clone();
    indexed.workspace_id = Some("ws-somewhere".to_string());
    indexed.last_commit_watermark = 7;
    // 该夹具只用于「比对函数」层面的语义验证（落盘形态由 rebuild 用例覆盖）。

    // 严格比对会报 2 条；跨真源判定（rebuild/migrate 共用）认为一致。
    let strict = session_parity_issues(&folded, &indexed);
    assert_eq!(strict.len(), 2, "严格比对必须看到这两条：{strict:?}");
    assert!(
        strict.iter().all(|issue| issue.starts_with("workspace_id:")
            || issue.starts_with("last_commit_watermark:")),
        "只有这两条属豁免清单：{strict:?}"
    );
    // 豁免清单只有一处定义：rebuild 侧（`SqliteSessionBackend`）与迁移侧别名必须相等。
    assert_eq!(
        SqliteSessionBackend::parity_ignored_state_fields(),
        ["workspace_id", "last_commit_watermark"],
        "豁免清单是唯一真源"
    );
    assert_eq!(
        MIGRATION_PARITY_EXEMPTIONS,
        SqliteSessionBackend::parity_ignored_state_fields(),
        "迁移与 rebuild 必须共用同一份豁免清单"
    );
}

#[test]
fn explain_prints_field_level_summaries_without_full_payloads() {
    let fixture = Fixture::new("explain");
    let session_id = "pa_3d_explain";
    let long_text = "长".repeat(400);
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(session_id), &long_text, "答", None, Vec::new())
            .expect("append_turn");
    }
    let folded = fixture.folded(session_id);
    // 造一个「确实有差异」的索引行：少一条消息。
    let mut tampered = folded.clone();
    tampered.history.pop();
    let conn = rusqlite::Connection::open(fixture.db()).expect("open db");
    conn.execute(
        "UPDATE sessions SET session_data = ?2 WHERE conversation_id = ?1",
        rusqlite::params![session_id, serde_json::to_string(&tampered).expect("encode")],
    )
    .expect("tamper row");
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").ok();

    let backend = fixture.backend();
    let lines = backend
        .explain_session_parity(session_id)
        .expect("explain 必须成功");
    assert!(lines.iter().any(|line| line == "no difference") == false, "有差异时不得说 no difference");
    let history_line = lines
        .iter()
        .find(|line| line.starts_with("history.len"))
        .unwrap_or_else(|| panic!("必须逐字段列出差异：{lines:#?}"));
    assert!(history_line.contains("fold: 2"), "必须给出条数：{history_line}");
    assert!(history_line.contains("index: 1"), "必须给出条数：{history_line}");
    let history_line = lines
        .iter()
        .find(|line| line.starts_with("history |"))
        .expect("history 内容差异也要有一行");
    assert!(
        !history_line.contains(&long_text),
        "摘要不得打印大字段全文：{history_line}"
    );
    assert!(
        history_line.chars().count() < 400,
        "摘要长度必须受控（80 字符预览）：{history_line}"
    );

    // 无差异的会话 ⇒ 恰好一行 `no difference`。
    let clean_id = "pa_3d_explain_clean";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(clean_id), "问", "答", None, Vec::new())
            .expect("append_turn");
    }
    let clean_lines = fixture
        .backend()
        .explain_session_parity(clean_id)
        .expect("explain clean");
    assert_eq!(
        clean_lines,
        vec!["no difference".to_string()],
        "同源比对无差异时必须只有一行 no difference：{clean_lines:?}"
    );
    eprintln!("[explain] {history_line}");
}

#[test]
fn explain_cli_is_read_only_and_mutually_exclusive_with_dry_run() {
    let fixture = Fixture::new("explain_cli");
    let session_id = "pa_3d_explain_cli";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(session_id), "问", "答", None, Vec::new())
            .expect("append_turn");
    }
    let db = fixture.db();
    let before = std::fs::read(&db).expect("read db");

    let ok = std::process::Command::new(env!("CARGO_BIN_EXE_ponycli"))
        .args(["sessions", "rebuild-index", "--explain", session_id, "--db"])
        .arg(&db)
        .output()
        .expect("run ponycli --explain");
    let stdout = String::from_utf8_lossy(&ok.stdout);
    assert_eq!(
        ok.status.code(),
        Some(0),
        "explain 必须 EXIT=0\nstdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&ok.stderr)
    );
    assert!(stdout.contains("mode: explain"), "必须标明模式：\n{stdout}");
    assert!(
        stdout.contains("no difference"),
        "该会话折叠与索引一致，必须打印 no difference：\n{stdout}"
    );

    let conflict = std::process::Command::new(env!("CARGO_BIN_EXE_ponycli"))
        .args(["sessions", "rebuild-index", "--dry-run", "--explain", session_id, "--db"])
        .arg(&db)
        .output()
        .expect("run conflicting flags");
    assert_eq!(
        conflict.status.code(),
        Some(2),
        "`--explain` 与 `--dry-run` 互斥（EXIT=2）"
    );
    assert_eq!(
        std::fs::read(&db).expect("read db"),
        before,
        "explain/冲突路径都必须零写入"
    );
    eprintln!("[explain-cli] 退出码 0 / 互斥 2；db 字节未变");
}
