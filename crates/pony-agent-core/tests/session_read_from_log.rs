//! ADR 0099 **阶段 2-B** 验收：读路径切到**日志真源**（「文件即真相」的兑现点）。
//!
//! 阶段 1 把写路径真源切到了每会话一份 append-only 日志；阶段 2-A 补了 `fold` 与索引重建。
//! 本文件证明最后一步：**读**不再依赖索引行。核心用例是
//! [`session_survives_index_row_deletion_because_the_log_is_the_truth`]——把 `sessions` 行
//! 物理删掉之后，运行时 `load_store()` / `list_sessions()` 仍返回完整会话。
//!
//! 七类用例（全部真断言，无「跑通即放行」）：
//!
//! 1. 删索引行后仍可读（核心证明）；
//! 2. 索引 `session_data` 损坏 ⇒ 读日志不受影响；
//! 3. 只造索引行、不写日志的存量会话仍可读（grandfather 不丢）；
//! 4. 混合场景：1 个有日志 + 1 个只有索引 ⇒ 两者都返回，同 id 以日志为准；
//! 5. 删除不复活（含日志墓碑）；
//! 6. `AppendHookRecords` 口径一致（两次命令后 fold 与索引条数/顺序一致）；
//! 7. 性能守卫：50 个有日志会话的装载耗时 + 日志根不可用时的降级回落；
//! 8. 守卫：运行时路径不得使用「索引原样视图」（`load_index_store`）。
//!
//! 隔离：所有库与日志都落在 `std::env::temp_dir()` 下的唯一目录，**绝不触碰**
//! `~/.local/share/PonyAgent/`；fixture `Drop` 清理。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pony_agent_core::agent::hooks::{build_observe_hook_trace_record, TurnHookPoint};
use pony_agent_core::agent::session::{
    current_log_cwd, read_session_state, session_log_root_for_db, HistoryCursor, PersistCommand,
    PersistCommandOutcome, SeparateTraceTableMode, SessionBackend, SessionLogStore, SessionState,
    SessionStore, TraceMigrationState, TurnHistoryMessage, TurnTraceRecord,
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
            "pony-0099-2b-read-{label}-{}-{}",
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

    fn db(&self) -> PathBuf {
        self.root.join("sessions.db")
    }

    fn log_root(&self) -> PathBuf {
        session_log_root_for_db(&self.db())
    }

    /// 可写后端（真实运行形态：TraceTableAuthoritative 投影）。
    fn backend(&self) -> SqliteSessionBackend {
        SqliteSessionBackend::new_with_trace_mode(
            self.db(),
            SeparateTraceTableMode::WriteSeparate,
        )
    }

    fn store(&self) -> SessionStore {
        SessionStore::with_backend(Box::new(self.backend()))
    }

    /// **运行时读路径**（业务句柄）：`load_store()` = 日志 fold ∪ 索引 grandfather。
    fn runtime_view(&self) -> HashMap<String, SessionState> {
        self.backend()
            .load_store()
            .expect("运行时 load_store 必须成功")
            .sessions_view()
            .clone()
    }

    /// 索引原样视图：维护/诊断入口（`new_read_only` 刻意不合并日志）。
    fn index_view(&self) -> HashMap<String, SessionState> {
        SqliteSessionBackend::new_read_only(self.db())
            .load_store()
            .expect("索引原样视图必须成功")
            .sessions_view()
            .clone()
    }

    /// 索引原样视图（另一条入口：`load_index_store`，双读比对基准）。
    fn index_view_raw(&self) -> HashMap<String, SessionState> {
        self.backend()
            .load_index_store()
            .expect("load_index_store 必须成功")
            .sessions_view()
            .clone()
    }

    fn log_path(&self, session_id: &str) -> PathBuf {
        SessionLogStore::new(self.log_root())
            .log_path(session_id, &current_log_cwd())
            .expect("session id 必须映射到安全的日志路径")
    }

    fn open_sqlite(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.db()).expect("open sqlite")
    }

    /// 直接对索引执行一条带 session_id 参数的 SQL（模拟「索引被删/被改坏」）。
    fn run_sql(&self, statement: &str, session_id: &str) {
        let conn = self.open_sqlite();
        conn.execute(statement, rusqlite::params![session_id])
            .unwrap_or_else(|error| panic!("execute `{statement}`: {error}"));
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").ok();
    }

    /// 把某会话的索引 blob 覆盖成任意字符串（损坏/陈旧注入）。
    fn overwrite_index_blob(&self, session_id: &str, payload: &str) {
        let conn = self.open_sqlite();
        conn.execute(
            "UPDATE sessions SET session_data = ?2 WHERE conversation_id = ?1",
            rusqlite::params![session_id, payload],
        )
        .expect("overwrite session_data");
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").ok();
    }

    fn index_row_exists(&self, session_id: &str) -> bool {
        let conn = self.open_sqlite();
        conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sessions WHERE conversation_id = ?1)",
            rusqlite::params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value != 0)
        .unwrap_or(false)
    }

    /// 从日志 fold 出会话状态（真源侧，纯函数口径）。
    fn folded(&self, session_id: &str) -> SessionState {
        read_session_state(&SessionLogStore::new(self.log_root()), session_id)
            .unwrap_or_else(|error| panic!("fold session `{session_id}`: {error}"))
            .state
    }

    /// 往某会话日志直接追加命令信封（构造墓碑等「写侧不会生成」的日志形态）。
    fn append_log_commands(&self, session_id: &str, commands: &[serde_json::Value]) {
        let store = SessionLogStore::new(self.log_root());
        let meta = pony_agent_core::agent::session::SessionLogMeta::new(session_id, current_log_cwd());
        let mut writer = store
            .open_write(session_id, &meta)
            .expect("open session log for synthetic append");
        writer
            .append_commands(1_760_000_000_000, commands)
            .expect("append synthetic commands");
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn json_of<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("serialize")
}

fn contents(session: &SessionState) -> Vec<String> {
    session
        .history
        .iter()
        .map(|message| message.content.clone())
        .collect()
}

fn user_message(content: &str) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: "user".to_string(),
        content: content.to_string(),
        ..Default::default()
    }
}

fn hook_record(name: &str, order: u32) -> pony_agent_core::agent::hooks::HookTraceRecord {
    build_observe_hook_trace_record(
        name,
        TurnHookPoint::TurnPrepareStart,
        order,
        format!("{name} summary"),
        None,
    )
}

fn hook_names(trace: &TurnTraceRecord) -> Vec<String> {
    trace
        .hook_trace_records
        .iter()
        .map(|record| record.hook_name.clone())
        .collect()
}

/// 从索引行读出的 hook 记录名（legacy `session_turn_traces.trace_data`）。
fn legacy_index_hook_names(fixture: &Fixture, session_id: &str, turn_id: &str) -> Vec<String> {
    let conn = fixture.open_sqlite();
    let raw: String = conn
        .query_row(
            "SELECT trace_data FROM session_turn_traces WHERE session_id = ?1 AND turn_id = ?2",
            rusqlite::params![session_id, turn_id],
            |row| row.get(0),
        )
        .unwrap_or_else(|error| panic!("读 legacy trace 行 `{session_id}/{turn_id}`: {error}"));
    hook_names_from_json(&raw, "legacy trace_data")
}

/// 从索引行读出的 hook 记录名（`normalized_turn_traces.extension_json`）。
fn normalized_index_hook_names(fixture: &Fixture, session_id: &str, turn_id: &str) -> Vec<String> {
    let conn = fixture.open_sqlite();
    let raw: Option<String> = conn
        .query_row(
            "SELECT extension_json FROM normalized_turn_traces WHERE session_id = ?1 AND turn_id = ?2",
            rusqlite::params![session_id, turn_id],
            |row| row.get::<_, Option<String>>(0),
        )
        .unwrap_or_else(|error| panic!("读规范化 trace 行 `{session_id}/{turn_id}`: {error}"));
    hook_names_from_json(
        raw.as_deref().unwrap_or("{}"),
        "normalized extension_json",
    )
}

fn hook_names_from_json(raw: &str, source: &str) -> Vec<String> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .unwrap_or_else(|error| panic!("parse {source}: {error} / {raw}"));
    value
        .get("hookTraceRecords")
        .and_then(|records| records.as_array())
        .map(|records| {
            records
                .iter()
                .map(|record| {
                    record
                        .get("hookName")
                        .and_then(|name| name.as_str())
                        .unwrap_or_default()
                        .to_string()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// 只存在于索引里的会话（单行写 `upsert_session`：**不产生任何日志命令**）。
fn index_only_session(session_id: &str, title: &str, contents: &[&str]) -> SessionState {
    SessionState {
        conversation_id: session_id.to_string(),
        title: title.to_string(),
        summary: "只有索引行，没有日志文件".to_string(),
        history: contents.iter().map(|text| user_message(text)).collect(),
        provider_native_transcript: Vec::new(),
        turn_trace_history: Vec::new(),
        trace_migration_state: TraceMigrationState::LegacyBlob,
        turn_trace_refs: None,
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        turn_count: contents.len(),
        last_referenced_file: Some("src/index_only.rs".to_string()),
        updated_at_ms: 1_700_000_000_000,
        history_nodes: Vec::new(),
        history_branches: Vec::new(),
        history_cursor: HistoryCursor::default(),
        event_watermark: 0,
        last_commit_watermark: 0,
        workspace_id: None,
        title_override: Some(title.to_string()),
        archived: true,
    }
}

// ---------------------------------------------------------------------------
// 1.【核心证明】删掉索引行后仍能读到会话
// ---------------------------------------------------------------------------

#[test]
fn session_survives_index_row_deletion_because_the_log_is_the_truth() {
    let fixture = Fixture::new("delete_index_row");
    let session_id = "pa_2b_read_after_delete";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(session_id), "hello", "world", None, Vec::new())
            .expect("append_turn 必须经日志真源落盘");
    }
    assert!(
        fixture.log_path(session_id).is_file(),
        "前置：真源侧（日志文件）必须存在"
    );
    assert!(
        fixture.index_row_exists(session_id),
        "前置：索引行必须**先**存在，否则这条用例证明不了任何东西"
    );

    // —— 物理删掉索引行（这就是「索引可删可建」的最终检验）——
    fixture.run_sql("DELETE FROM sessions WHERE conversation_id = ?1", session_id);
    assert!(
        !fixture.index_row_exists(session_id),
        "索引行必须真的被删掉（不是「看起来不在」）"
    );
    assert!(
        fixture.index_view().get(session_id).is_none(),
        "索引原样视图（维护句柄）里也必须已经没有这一行：若能读到，来源只可能是日志"
    );
    assert!(
        fixture.index_view_raw().get(session_id).is_none(),
        "索引原样视图（`load_index_store`，双读比对基准）同样不得有这一行"
    );

    // —— 核心断言：运行时读路径仍返回完整会话 ——
    let view = fixture.runtime_view();
    let session = view.get(session_id).unwrap_or_else(|| {
        panic!(
            "删掉索引行后 session `{session_id}` 必须仍能从日志读到（文件即真相），\
             实际读到的会话：{:?}",
            view.keys().collect::<Vec<_>>()
        )
    });
    assert_eq!(session.conversation_id, session_id);
    assert_eq!(contents(session), vec!["hello", "world"], "内容必须完整");
    assert_eq!(session.history.len(), 2);
    assert_eq!(session.turn_count, 1, "turn_count 必须复现");
    assert!(
        !session.title.is_empty(),
        "标题必须复现（不能读出一个空壳会话）：{}",
        json_of(session)
    );

    // 物理收据（`--nocapture` 可见）：把「索引已无此行」与「读到的完整内容」一次打出来。
    eprintln!(
        "[C.1] index_row_exists={} runtime_session={{id:{}, title:{}, turn_count:{}, history:{:?}}}",
        fixture.index_row_exists(session_id),
        session.conversation_id,
        session.title,
        session.turn_count,
        contents(session)
    );

    // —— `list_sessions()`（运行时入口）同样看得见 ——
    let store = fixture.store();
    let overviews = store.list_sessions();
    assert!(
        overviews
            .iter()
            .any(|overview| overview.conversation_id == session_id
                && overview.turn_count == 1),
        "list_sessions() 必须列出该会话（实际 {:?}）",
        overviews
            .iter()
            .map(|overview| overview.conversation_id.clone())
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// 2. 索引损坏可恢复（读日志不受影响）
// ---------------------------------------------------------------------------

#[test]
fn corrupted_index_blob_does_not_hide_the_session() {
    let fixture = Fixture::new("corrupt_index");
    let session_id = "pa_2b_corrupt_index";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(session_id), "u-log", "a-log", None, Vec::new())
            .expect("append_turn");
    }
    // 把索引行的 blob 改成垃圾 JSON（模拟索引损坏 / 半写）。
    fixture.overwrite_index_blob(session_id, "{ this is not json");
    assert!(
        fixture.index_view().get(session_id).is_none(),
        "索引侧已损坏 ⇒ 索引原样视图读不出该行（对照组）"
    );

    let view = fixture.runtime_view();
    let session = view
        .get(session_id)
        .unwrap_or_else(|| panic!("索引 blob 损坏不得影响读日志：{session_id} 应仍可读"));
    assert_eq!(contents(session), vec!["u-log", "a-log"]);

    // 与 fold 结果逐字节一致（不是「凑了两条」，而是同一份真源）。
    let folded = fixture.folded(session_id);
    assert_eq!(
        json_of(session),
        json_of(&folded),
        "运行时读出的状态必须与日志 fold 完全一致"
    );
}

// ---------------------------------------------------------------------------
// 3. 无日志会话仍可读（grandfather 不丢）
// ---------------------------------------------------------------------------

#[test]
fn grandfather_sessions_without_a_log_still_load() {
    let fixture = Fixture::new("grandfather");
    let session_id = "pa_2b_index_only";
    {
        let backend = fixture.backend();
        assert!(
            backend.upsert_session(
                session_id,
                &index_only_session(session_id, "索引独有标题", &["历史一", "历史二"])
            ),
            "索引独有会话必须写入成功"
        );
    }
    assert!(
        !fixture.log_path(session_id).exists(),
        "前置：该会话**必须没有**日志文件（否则 grandfather 用例不成立）"
    );

    let view = fixture.runtime_view();
    let session = view
        .get(session_id)
        .unwrap_or_else(|| panic!("无日志的存量会话不得在读切换后丢失：{session_id}"));
    assert_eq!(session.title, "索引独有标题");
    assert_eq!(contents(session), vec!["历史一", "历史二"]);
    assert!(session.archived, "归档标记属于索引行，读切换不得抹掉");
    assert_eq!(session.last_referenced_file.as_deref(), Some("src/index_only.rs"));
}

// ---------------------------------------------------------------------------
// 4. 混合场景：同 id 以日志为准
// ---------------------------------------------------------------------------

#[test]
fn mixed_sessions_merge_and_the_log_wins_for_the_same_id() {
    let fixture = Fixture::new("mixed");
    let logged = "pa_2b_mixed_logged";
    let index_only = "pa_2b_mixed_index_only";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(logged), "hello", "world", None, Vec::new())
            .expect("append_turn");
    }
    {
        let backend = fixture.backend();
        assert!(backend.upsert_session(
            index_only,
            &index_only_session(index_only, "只有索引", &["索引历史"])
        ));
    }
    // 让「日志版」与「索引版」内容**明显不同**：同 id 以日志为准才有可证伪性。
    let mut stale = index_only_session(logged, "陈旧标题", &["陈旧内容"]);
    stale.archived = false;
    fixture.overwrite_index_blob(logged, &json_of(&stale));
    assert!(
        fixture
            .index_view()
            .get(logged)
            .is_some_and(|session| contents(session) == vec!["陈旧内容"]),
        "对照组：索引侧现在持有陈旧内容"
    );

    let view = fixture.runtime_view();
    let logged_state = view.get(logged).expect("有日志的会话必须返回");
    assert_eq!(
        contents(logged_state),
        vec!["hello", "world"],
        "同 id 必须**以日志为准**（不得读成索引里的陈旧内容）"
    );
    assert_ne!(logged_state.title, "陈旧标题", "标题也必须来自日志");
    let index_state = view.get(index_only).expect("只有索引的会话必须返回");
    assert_eq!(contents(index_state), vec!["索引历史"]);
    assert_eq!(view.len(), 2, "混合场景必须两者都在，不多不少");
}

// ---------------------------------------------------------------------------
// 5. 删除不复活
// ---------------------------------------------------------------------------

#[test]
fn removed_sessions_are_not_resurrected_by_the_read_path() {
    let fixture = Fixture::new("deleted");
    let deleted = "pa_2b_deleted";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(deleted), "u", "a", None, Vec::new())
            .expect("append_turn");
        assert!(fixture.log_path(deleted).is_file());
        store.remove_session(deleted);
    }
    assert!(
        !fixture.log_path(deleted).exists(),
        "remove_session 必须删掉日志目录（真源侧同步删除）"
    );
    assert!(!fixture.index_row_exists(deleted), "索引行也必须删掉");
    assert!(
        fixture.runtime_view().get(deleted).is_none(),
        "已删除会话不得被读路径复活"
    );

    // 日志墓碑（D3）：索引行仍在，但日志里有 `remove_session` ⇒ 读路径视其为不存在。
    let tombstone = "pa_2b_tombstone";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(tombstone), "u", "a", None, Vec::new())
            .expect("append_turn");
    }
    fixture.append_log_commands(
        tombstone,
        &[serde_json::json!({
            "type": "remove_session",
            "encodingVersion": 1,
            "epoch": 1,
            "sessionId": tombstone,
        })],
    );
    assert!(
        fixture.index_row_exists(tombstone),
        "前置：索引行仍残留（模拟旧快照/未传播的删除）"
    );
    assert!(
        fixture.runtime_view().get(tombstone).is_none(),
        "日志墓碑必须让读路径视该会话不存在（删除单调不可复活）"
    );
}

// ---------------------------------------------------------------------------
// 6. hook 口径一致（A 的钉子：追加，不是替换）
// ---------------------------------------------------------------------------

#[test]
fn hook_records_are_appended_identically_by_log_fold_and_index() {
    let fixture = Fixture::new("hook_append");
    let session_id = "pa_2b_hook_append";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(session_id), "u", "a", None, Vec::new())
            .expect("append_turn");
        store.record_turn_trace(
            Some(session_id),
            TurnTraceRecord {
                turn_id: "turn-1".to_string(),
                title: "turn title".to_string(),
                phase: "completed".to_string(),
                ..Default::default()
            },
        );
    }

    // 两个**独立批次**各追加一条 hook 记录：追加语义 ⇒ 合计 2 条、顺序稳定。
    let backend = fixture.backend();
    for (order, name) in [(0u32, "hook-a"), (1u32, "hook-b")] {
        assert_eq!(
            backend.persist_commands_batch(vec![PersistCommand::AppendHookRecords {
                epoch: 1,
                session_id: session_id.to_string(),
                turn_id: "turn-1".to_string(),
                hook_trace_records: vec![hook_record(name, order)],
                updated_at: 1_760_000_000_000 + order as u64,
            }]),
            PersistCommandOutcome::Succeeded,
            "AppendHookRecords 第 {order} 批必须成功"
        );
    }

    let folded = fixture.folded(session_id);
    let fold_trace = folded
        .turn_trace_history
        .iter()
        .find(|trace| trace.turn_id == "turn-1")
        .expect("fold 必须折叠出 turn-1 的 trace");
    let fold_hooks = hook_names(fold_trace);

    assert_eq!(
        fold_hooks,
        vec!["hook-a".to_string(), "hook-b".to_string()],
        "日志 fold 必须按追加语义得到 2 条（不是最后一批覆盖）"
    );

    // 索引侧的 hook 记录落在两处投影行上，逐一与 fold 对齐：
    // ① legacy `session_turn_traces.trace_data`——本阶段 A 修的正是它（原来是按键替换）；
    // ② `normalized_turn_traces.extension_json`——命令流投影。
    //
    // 为什么不做「loader 视图」比对：legacy blob 视图的 trace 会被 `merge_trace_history`
    // 按陈旧的 `turn_trace_refs` 过滤成空（阶段 2-A 已把这条缺口记录在案，属写路径缺口，
    // 不在本 lane 写域）。索引行是这条断言的**真值面**，比对它才能证伪「索引 1 条」。
    let legacy_hooks = legacy_index_hook_names(&fixture, session_id, "turn-1");
    assert_eq!(
        legacy_hooks, fold_hooks,
        "两次 AppendHookRecords 后 legacy 索引行与 fold 的 hook 记录**条数与顺序**必须一致\
         （修复前是「fold 2 条 / 索引 1 条」）"
    );
    let normalized_hooks = normalized_index_hook_names(&fixture, session_id, "turn-1");
    assert_eq!(
        normalized_hooks, fold_hooks,
        "规范化投影的 hook 记录必须与 fold 一致"
    );
}

// ---------------------------------------------------------------------------
// 7. 性能守卫：50 个有日志会话 + 日志根不可用时的降级回落
// ---------------------------------------------------------------------------

#[test]
fn fifty_logged_sessions_load_within_budget_and_degrade_keeps_the_index_complete() {
    let fixture = Fixture::new("perf");
    let sessions: Vec<String> = (0..50)
        .map(|index| format!("pa_2b_perf_{index:02}"))
        .collect();
    {
        let mut store = fixture.store();
        for session_id in &sessions {
            for turn in 0..3u32 {
                store
                    .append_turn_fallible(
                        Some(session_id),
                        &format!("u-{turn}"),
                        &format!("a-{turn}"),
                        None,
                        Vec::new(),
                    )
                    .expect("append_turn");
            }
        }
    }
    assert_eq!(
        SessionLogStore::new(fixture.log_root())
            .list_sessions()
            .expect("list session logs")
            .len(),
        50,
        "前置：50 个会话都必须有日志文件"
    );

    let started = Instant::now();
    let view = fixture.runtime_view();
    let elapsed = started.elapsed();
    eprintln!(
        "[session_read_from_log] 50 个会话 × 6 条消息：运行时 load_store 耗时 {}ms",
        elapsed.as_millis()
    );
    assert_eq!(view.len(), 50, "50 个有日志会话必须全部装载");
    for session_id in &sessions {
        let session = view
            .get(session_id)
            .unwrap_or_else(|| panic!("会话 {session_id} 缺失"));
        assert_eq!(session.history.len(), 6, "{session_id} 的历史必须完整");
        assert_eq!(session.history[0].content, "u-0");
        assert_eq!(session.history[5].content, "a-2");
    }
    // 不做「越界即失败」的脆弱阈值：这里只钉住「没有退化到不可用」的量级。
    assert!(
        elapsed < Duration::from_secs(5),
        "50 个会话的启动装载不得退化到不可用（实际 {}ms；预算阈值 800ms 见 fold.rs）",
        elapsed.as_millis()
    );

    // 降级回落：日志根不可用（等价于「折叠不可用 ⇒ 按索引读」）时索引仍须完整。
    let log_root = fixture.log_root();
    let parked = fixture.root.join("session-logs-parked");
    std::fs::rename(&log_root, &parked).expect("park session log root");
    let degraded_view = fixture.runtime_view();
    std::fs::rename(&parked, &log_root).expect("restore session log root");
    assert_eq!(
        degraded_view.len(),
        50,
        "日志侧不可用时必须降级为读索引，且索引仍返回完整列表"
    );
    assert!(
        degraded_view
            .get(&sessions[0])
            .is_some_and(|session| session.history.len() == 6),
        "降级回落读到的内容必须完整（不是空壳行）"
    );
}

// ---------------------------------------------------------------------------
// 8. 守卫：运行时路径不得使用「索引原样视图」
// ---------------------------------------------------------------------------

/// 扫描 `src/**/*.rs`，钉住 `load_index_store`（索引原样视图）的调用面。
///
/// 裁决（ADR 0099 阶段 2-B）：它是**诊断/比对**口径，运行时业务路径必须走
/// `SessionBackend::load_store`（日志真源）。允许的代码出现位置只有两处：
/// - `agent/sqlite_session.rs`：定义 + `load_store` 的分层（先取索引，再叠日志）；
/// - `agent/session/fold.rs`：`rebuild_index` 的双读比对基准。
///
/// 任何第三处出现都会被这条守卫打红——包括未来某次「顺手用索引视图兜底」的改动。
#[test]
fn runtime_paths_never_read_the_index_only_view() {
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    // 只看**调用点**（`.load_index_store(`），避免把 doc/日志字面量算成调用方。
    let needle = ".load_index_store(";
    let mut offenders: Vec<String> = Vec::new();
    let mut files_with_usage: Vec<String> = Vec::new();
    let mut fold_call_lines = 0usize;
    let mut stack = vec![src_root.clone()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).expect("read src dir");
        for entry in entries {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read source file");
            let relative = path
                .strip_prefix(&src_root)
                .expect("strip src prefix")
                .to_string_lossy()
                .replace('\\', "/");
            let mut used = false;
            for line in text.lines() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//") || !line.contains(needle) {
                    continue;
                }
                used = true;
                let allowed =
                    relative == "agent/sqlite_session.rs" || relative == "agent/session/fold.rs";
                if relative == "agent/session/fold.rs" {
                    fold_call_lines += 1;
                }
                if !allowed {
                    offenders.push(format!("{relative}: {trimmed}"));
                }
            }
            if used {
                files_with_usage.push(relative);
            }
        }
    }
    files_with_usage.sort();
    assert!(
        offenders.is_empty(),
        "运行时路径不得调用索引原样视图 `load_index_store`（只允许 sqlite_session.rs 的\
         load_store 分层与 fold.rs 的双读比对）：{offenders:#?}"
    );
    assert_eq!(
        files_with_usage,
        vec![
            "agent/session/fold.rs".to_string(),
            "agent/sqlite_session.rs".to_string()
        ],
        "`load_index_store` 的出现文件集合变化了——请先确认新调用方属于「诊断/比对」口径"
    );
    assert_eq!(
        fold_call_lines, 1,
        "fold.rs 里只允许 `rebuild_index_at_root` 一处使用它（双读比对基准）"
    );
    // 定义必须仍在，且外部用法只有调用点（防止有人把它改名成「看起来像索引」的东西）。
    let sqlite_session_src = std::fs::read_to_string(src_root.join("agent/sqlite_session.rs"))
        .expect("read sqlite_session.rs");
    assert!(
        sqlite_session_src.contains("pub fn load_index_store"),
        "索引原样视图必须保持显式命名（`load_index_store`）"
    );
    // 运行时读路径必须真的是 load_store（不是「看起来像」）。
    let sqlite_session = std::fs::read_to_string(src_root.join("agent/sqlite_session.rs"))
        .expect("read sqlite_session.rs");
    assert!(
        sqlite_session.contains("merge_log_truth_into_sessions"),
        "`load_store` 必须叠加日志真源合并（ADR 0099 阶段 2-B）"
    );
}
