//! ADR 0099 **阶段 2-A** 验收：`fold` 状态重建器 + SQLite 索引重建。
//!
//! task-6 D 节的七类用例（全部真断言，无「跑通即放行」）：
//!
//! 1. [`single_turn_fold_matches_the_sqlite_index_field_by_field`] —— 单轮 `append_turn`
//!    后「日志 fold」与「索引 load_store」**逐字段**一致。
//! 2. [`three_turns_with_trace_fold_matches_the_sqlite_index_field_by_field`] —— 3 轮 +
//!    `record_turn_trace` 后同样逐字段一致。
//! 3. [`meta_patch_three_states_round_trip_through_the_log`] —— `title_override` 的
//!    `None` / `Some(None)` / `Some(Some(x))` 三态都能从日志正确还原。
//! 4. [`sessions_without_a_log_are_never_touched_by_rebuild`] —— 只有索引行的会话在
//!    `rebuild_index` 后**内容不变**且计入 `skipped_no_log`。
//! 5. [`deleted_sessions_are_not_resurrected_by_rebuild`] —— `remove_session` 后重建不复活；
//!    日志里的显式墓碑（`RemoveSession`）同样不重建。
//! 6. [`rebuild_index_is_idempotent`] —— 连跑两次：`rebuilt` 稳定、库内容不变、无 mismatch。
//! 7. [`dry_run_writes_nothing`] —— `sessions.db` 与 `-wal` 的 sha256 前后一致。
//!
//! 隔离：所有库与日志都落在 `std::env::temp_dir()` 下的唯一目录，**绝不触碰**
//! `~/.local/share/PonyAgent/`；fixture `Drop` 清理。

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use pony_agent_core::agent::session::{
    fold_session, read_log_events, read_session_state, rebuild_index, session_log_root_for_db,
    session_parity_diff, session_parity_issues, LogEvent, PersistCommand, PersistCommandOutcome, RebuildOptions,
    SeparateTraceTableMode, SessionBackend, SessionLogMeta, SessionLogStore, SessionState,
    SessionStore, TurnHistoryMessage, TurnTraceRecord,
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
            "pony-0099-2a-fold-{label}-{}-{}",
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

    fn backend(&self) -> SqliteSessionBackend {
        SqliteSessionBackend::new_with_trace_mode(
            self.db(),
            // 新会话进入 TraceTableAuthoritative：真实生产写入形态。
            SeparateTraceTableMode::WriteSeparate,
        )
    }

    fn store(&self) -> SessionStore {
        SessionStore::with_backend(Box::new(self.backend()))
    }

    /// 从日志 fold 出会话状态（真源侧）。
    fn folded(&self, session_id: &str) -> SessionState {
        let store = SessionLogStore::new(self.log_root());
        read_session_state(&store, session_id)
            .unwrap_or_else(|error| panic!("fold session `{session_id}`: {error}"))
            .state
    }

    /// 索引侧：以只读句柄 `load_store()`（不建表、不迁移、不改 journal_mode）。
    fn load_store(&self) -> HashMap<String, SessionState> {
        SqliteSessionBackend::new_read_only(self.db())
            .load_store()
            .expect("load_store must succeed")
            .sessions_view()
            .clone()
    }

    /// 把读路径切到**规范化投影**（`storage.normalized.v1.phase=retired`）。
    ///
    /// 这不是「为了让断言变绿而拨开关」：命令流的投影就落在 `normalized_*` 表上
    /// （live 写路径的 `apply_persist_command_tx` 只写这些表）。阶段 2 的读切换正是把
    /// 读路径从 legacy blob 换到这份投影，所以对拍必须对「命令派生视图」做，否则
    /// 比的是另一份真源（整包 blob 快照）。legacy blob 视图的已知差异由
    /// `legacy_blob_view_diff_is_limited_to_documented_gaps` 显式钉死清单。
    fn cut_over_to_normalized_projection(&self) {
        let outcome = self.backend().persist_command(PersistCommand::PublishMetadata {
            epoch: 1,
            key: "storage.normalized.v1.phase".to_string(),
            value: "retired".to_string(),
        });
        assert_eq!(
            outcome,
            PersistCommandOutcome::Succeeded,
            "切换读路径到规范化投影失败"
        );
    }

    fn indexed(&self, session_id: &str) -> SessionState {
        self.load_store()
            .get(session_id)
            .cloned()
            .unwrap_or_else(|| panic!("索引里必须有会话 {session_id}"))
    }

    fn log_path(&self, session_id: &str) -> PathBuf {
        let store = SessionLogStore::new(self.log_root());
        store
            .log_path(session_id, &cwd_label())
            .expect("session id must map to a safe log path")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn cwd_label() -> String {
    pony_agent_core::agent::session::current_log_cwd()
}

fn json_of<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("serialize for comparison")
}

/// 整库快照（**有序**）：`HashMap` 的迭代顺序不稳定，直接序列化会被键序差异误判。
fn store_snapshot(sessions: &HashMap<String, SessionState>) -> BTreeMap<String, String> {
    sessions
        .iter()
        .map(|(session_id, session)| (session_id.clone(), json_of(session)))
        .collect()
}

fn user_message(content: &str) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: "user".to_string(),
        content: content.to_string(),
        ..Default::default()
    }
}

/// 逐字段对拍：差异即失败，并把两侧全文打出来（不做「条数差不多」的弱断言）。
fn assert_parity(folded: &SessionState, indexed: &SessionState, context: &str) {
    if let Some(diff) = session_parity_diff(folded, indexed) {
        panic!(
            "{context}：fold 与索引侧必须**逐字段**一致，差异如下：\n{diff}\n\n--- fold ---\n{}\n\n--- index ---\n{}",
            json_of(folded),
            json_of(indexed)
        );
    }
}

/// task D.1/D.2 明确点名的字段（除了整体逐字段对拍，再显式断言一遍，失败信息更可读）。
fn assert_named_fields(folded: &SessionState, indexed: &SessionState, context: &str) {
    assert_eq!(
        folded.history.len(),
        indexed.history.len(),
        "{context}：history 条数必须一致"
    );
    let fold_contents: Vec<&str> = folded
        .history
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    let index_contents: Vec<&str> = indexed
        .history
        .iter()
        .map(|message| message.content.as_str())
        .collect();
    assert_eq!(fold_contents, index_contents, "{context}：history 内容必须一致");
    assert_eq!(folded.title, indexed.title, "{context}：title 必须一致");
    assert_eq!(
        folded.turn_count, indexed.turn_count,
        "{context}：turn_count 必须一致"
    );
    let fold_nodes: Vec<&str> = folded
        .history_nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect();
    let index_nodes: Vec<&str> = indexed
        .history_nodes
        .iter()
        .map(|node| node.node_id.as_str())
        .collect();
    assert_eq!(fold_nodes, index_nodes, "{context}：history_nodes 必须一致");
    assert_eq!(
        json_of(&folded.history_cursor),
        json_of(&indexed.history_cursor),
        "{context}：cursor 必须一致"
    );
    assert_eq!(
        json_of(&folded.history_branches),
        json_of(&indexed.history_branches),
        "{context}：branches 必须一致"
    );
}

/// `sessions.db` + `-wal` 的 sha256（物理凭证：dry-run 零写入）。
fn db_fingerprint(db: &Path) -> String {
    use sha2::{Digest, Sha256};

    assert!(db.exists(), "数据库文件必须已存在才谈得上「未被修改」");
    let mut hasher = Sha256::new();
    let mut wal = db.as_os_str().to_owned();
    wal.push("-wal");
    for path in [db.to_path_buf(), PathBuf::from(wal)] {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = std::fs::read(&path).unwrap_or_default();
        hasher.update(name.as_bytes());
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    format!("{:x}", hasher.finalize())
}

/// 只存在于索引里的会话（`upsert_session` 单行写：**不产生任何日志命令**）。
fn index_only_session(session_id: &str, title: &str, contents: &[&str]) -> SessionState {
    SessionState {
        conversation_id: session_id.to_string(),
        title: title.to_string(),
        summary: "只有索引行，没有日志文件".to_string(),
        history: contents.iter().map(|text| user_message(text)).collect(),
        provider_native_transcript: Vec::new(),
        turn_trace_history: Vec::new(),
        trace_migration_state: Default::default(),
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
        history_cursor: Default::default(),
        event_watermark: 0,
        last_commit_watermark: 0,
        workspace_id: None,
        title_override: Some(title.to_string()),
        archived: true,
    }
}

// ---------------------------------------------------------------------------
// 1. 单轮：fold 与索引逐字段一致
// ---------------------------------------------------------------------------

#[test]
fn single_turn_fold_matches_the_sqlite_index_field_by_field() {
    let fixture = Fixture::new("single");
    let session_id = "pa_2a_single";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(session_id), "hello", "world", None, Vec::new())
            .expect("append_turn must persist through the log truth source");
    }
    assert!(
        fixture.log_path(session_id).is_file(),
        "append_turn 必须在真源侧留下日志文件"
    );
    fixture.cut_over_to_normalized_projection();

    let folded = fixture.folded(session_id);
    let indexed = fixture.indexed(session_id);
    assert_parity(&folded, &indexed, "单轮");
    assert_named_fields(&folded, &indexed, "单轮");

    // 具体值断言（防止「两边都是空」也判等）。
    assert_eq!(folded.history.len(), 2, "单轮必须是 user + assistant 两条");
    assert_eq!(folded.history[0].content, "hello");
    assert_eq!(folded.history[1].content, "world");
    assert_eq!(folded.turn_count, 1, "一轮之后 turn_count 必须是 1");
}

// ---------------------------------------------------------------------------
// 2. 多轮 + trace：fold 与索引逐字段一致
// ---------------------------------------------------------------------------

#[test]
fn three_turns_with_trace_fold_matches_the_sqlite_index_field_by_field() {
    let fixture = Fixture::new("three_turns");
    let session_id = "pa_2a_three_turns";
    {
        let mut store = fixture.store();
        for index in 0..3u32 {
            store
                .append_turn_fallible(
                    Some(session_id),
                    &format!("user-{index}"),
                    &format!("assistant-{index}"),
                    None,
                    Vec::new(),
                )
                .expect("append_turn");
        }
        for index in 0..3u32 {
            store.record_turn_trace(
                Some(session_id),
                TurnTraceRecord {
                    turn_id: format!("turn-{index}"),
                    title: format!("turn title {index}"),
                    phase: "completed".to_string(),
                    provider_name: Some("mock".to_string()),
                    total_tokens: Some(10 + index as u64),
                    ..Default::default()
                },
            );
        }
    }

    fixture.cut_over_to_normalized_projection();

    let folded = fixture.folded(session_id);
    let indexed = fixture.indexed(session_id);
    assert_parity(&folded, &indexed, "3 轮 + trace");
    assert_named_fields(&folded, &indexed, "3 轮 + trace");

    assert_eq!(folded.history.len(), 6, "3 轮 = 6 条消息");
    assert_eq!(folded.turn_count, 3);
    let turn_ids: Vec<&str> = folded
        .turn_trace_history
        .iter()
        .map(|trace| trace.turn_id.as_str())
        .collect();
    assert_eq!(turn_ids, vec!["turn-0", "turn-1", "turn-2"], "trace 必须按序折叠");
    assert!(
        folded.history_nodes.len() >= 2,
        "多轮必须折叠出历史节点（实际 {}）",
        folded.history_nodes.len()
    );
}

// ---------------------------------------------------------------------------
// 3. meta patch 三态（`Option<Option<T>>`）从日志正确还原
// ---------------------------------------------------------------------------

#[test]
fn meta_patch_three_states_round_trip_through_the_log() {
    let fixture = Fixture::new("meta_three_states");
    let backend = fixture.backend();

    // 三个会话分别覆盖「不改 / 清空 / 赋值」三态，全部经**真实写路径**（日志 + 索引）。
    let commands = |session_id: &str, patches: Vec<Option<Option<String>>>| {
        let mut batch = Vec::new();
        batch.push(PersistCommand::AppendMessage {
            epoch: 1,
            session_id: session_id.to_string(),
            message: user_message("u"),
            ordinal: 0,
        });
        for (index, title_override) in patches.into_iter().enumerate() {
            batch.push(PersistCommand::UpdateSessionMeta {
                epoch: 1,
                session_id: session_id.to_string(),
                meta_patch: pony_agent_core::agent::session::SessionMetaPatch {
                    title: Some(format!("派生标题-{index}")),
                    title_override,
                    ..Default::default()
                },
            });
        }
        batch
    };
    let untouched = "pa_2a_meta_untouched";
    let cleared = "pa_2a_meta_cleared";
    let assigned = "pa_2a_meta_assigned";
    // ① 从未给出 title_override ⇒ 不改（保持 None）
    assert_eq!(
        backend.persist_commands_batch(commands(untouched, vec![None])),
        PersistCommandOutcome::Succeeded
    );
    // ② 先赋值再显式清空 ⇒ Some(None) 落在 None 上
    assert_eq!(
        backend.persist_commands_batch(commands(
            cleared,
            vec![Some(Some("旧标题".to_string())), Some(None)]
        )),
        PersistCommandOutcome::Succeeded
    );
    // ③ 赋值后保持（后续 patch 不给该字段 ⇒ 仍保留自定义标题）
    assert_eq!(
        backend.persist_commands_batch(commands(
            assigned,
            vec![Some(Some("自定义标题".to_string())), None]
        )),
        PersistCommandOutcome::Succeeded
    );
    drop(backend);

    let untouched_state = fixture.folded(untouched);
    let cleared_state = fixture.folded(cleared);
    let assigned_state = fixture.folded(assigned);
    assert_eq!(
        untouched_state.title_override, None,
        "patch 里没有 title_override ⇒ 必须保持 None（不改）"
    );
    assert_eq!(
        cleared_state.title_override, None,
        "Some(None) ⇒ 必须清空（不能残留旧 override）"
    );
    assert_eq!(
        assigned_state.title_override.as_deref(),
        Some("自定义标题"),
        "Some(Some(x)) ⇒ 必须还原成 x；后续 None patch 不得冲掉它"
    );
    // title 走 COALESCE 语义：最后一次 patch 的派生标题胜出。
    assert_eq!(untouched_state.title, "派生标题-0");
    assert_eq!(cleared_state.title, "派生标题-1");
    assert_eq!(assigned_state.title, "派生标题-1");
}

// ---------------------------------------------------------------------------
// 4. 无日志会话：重建**原封不动** + 计入 skipped_no_log
// ---------------------------------------------------------------------------

#[test]
fn sessions_without_a_log_are_never_touched_by_rebuild() {
    let fixture = Fixture::new("no_log");
    let logged = "pa_2a_logged";
    let index_only = "pa_2a_index_only";

    // 有日志的会话（走 append_turn：日志 + 索引都有）
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(logged), "u", "a", None, Vec::new())
            .expect("append_turn");
    }
    // 只有索引行的会话：单行写不产生任何日志命令（存量库 131 个会话的实际形态）
    {
        let backend = fixture.backend();
        let session = index_only_session(index_only, "索引独有标题", &["历史一", "历史二"]);
        assert!(
            backend.upsert_session(index_only, &session),
            "索引独有会话必须写入成功"
        );
    }
    assert!(
        !fixture.log_path(index_only).exists(),
        "索引独有会话**必须没有**日志文件（否则本用例不成立）"
    );

    let before = fixture.indexed(index_only);
    let report = rebuild_index(&fixture.backend(), RebuildOptions::apply()).expect("rebuild");

    assert!(
        report.rebuilt.contains(&logged.to_string()),
        "有日志的会话必须被重建：{:?}",
        report.rebuilt
    );
    assert!(
        !report.rebuilt.contains(&index_only.to_string()),
        "无日志会话不得进入 rebuilt：{:?}",
        report.rebuilt
    );
    assert_eq!(
        report.skipped_no_log, 1,
        "无日志会话必须计入 skipped_no_log（索引里有、日志里没有）"
    );

    let after = fixture.indexed(index_only);
    assert_eq!(
        json_of(&before),
        json_of(&after),
        "无日志会话的内容必须**逐字节不变**（不得被清空/覆盖）"
    );
    assert_eq!(after.title, "索引独有标题");
    assert_eq!(after.history.len(), 2);
    assert_eq!(after.history[1].content, "历史二");
    assert!(after.archived, "归档标记属于索引行，重建不得改动");
}

// ---------------------------------------------------------------------------
// 5. 删除不复活
// ---------------------------------------------------------------------------

#[test]
fn deleted_sessions_are_not_resurrected_by_rebuild() {
    let fixture = Fixture::new("deleted");
    let session_id = "pa_2a_deleted";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(session_id), "u", "a", None, Vec::new())
            .expect("append_turn");
        assert!(fixture.log_path(session_id).is_file());
        store.remove_session(session_id);
    }
    assert!(
        !fixture.log_path(session_id).exists(),
        "remove_session 必须删掉日志目录（真源侧同步删除）"
    );

    let report = rebuild_index(&fixture.backend(), RebuildOptions::apply()).expect("rebuild");
    assert!(
        !report.rebuilt.contains(&session_id.to_string()),
        "已删除会话不得被重建：{:?}",
        report.rebuilt
    );
    assert!(
        fixture.load_store().get(session_id).is_none(),
        "已删除会话不得复活到索引里"
    );

    // 显式墓碑（写侧不写 RemoveSession，故手工构造一条日志验证 fold 的墓碑语义）。
    let tombstone = "pa_2a_tombstone";
    {
        let store = SessionLogStore::new(fixture.log_root());
        let mut writer = store
            .open_write(
                tombstone,
                &SessionLogMeta::new(tombstone, pony_agent_core::agent::session::current_log_cwd()),
            )
            .expect("open write for synthetic log");
        writer
            .append_commands(
                1_760_000_000_000,
                &[
                    serde_json::json!({
                        "type": "append_message",
                        "encodingVersion": 1,
                        "epoch": 1,
                        "sessionId": tombstone,
                        "message": {"role": "user", "content": "写一条再删"},
                        "ordinal": 0,
                    }),
                    serde_json::json!({
                        "type": "remove_session",
                        "encodingVersion": 1,
                        "epoch": 1,
                        "sessionId": tombstone,
                    }),
                ],
            )
            .expect("append synthetic commands");
    }
    let store = SessionLogStore::new(fixture.log_root());
    let folded = read_session_state(&store, tombstone).expect("fold tombstone log");
    assert!(folded.deleted, "RemoveSession 必须折叠成「已删除」");

    let report = rebuild_index(&fixture.backend(), RebuildOptions::apply()).expect("rebuild");
    assert!(
        report.skipped_deleted.contains(&tombstone.to_string()),
        "墓碑会话必须计入 skipped_deleted：{:?}",
        report.skipped_deleted
    );
    assert!(
        !report.rebuilt.contains(&tombstone.to_string()),
        "墓碑会话不得重建"
    );
    assert!(
        fixture.load_store().get(tombstone).is_none(),
        "墓碑会话不得出现在索引里"
    );
}

// ---------------------------------------------------------------------------
// 6. 幂等：连跑两次结果一致
// ---------------------------------------------------------------------------

#[test]
fn rebuild_index_is_idempotent() {
    let fixture = Fixture::new("idempotent");
    let sessions = ["pa_2a_idem_a", "pa_2a_idem_b"];
    {
        let mut store = fixture.store();
        for session_id in sessions {
            store
                .append_turn_fallible(Some(session_id), "u", "a", None, Vec::new())
                .expect("append_turn");
        }
        store.record_turn_trace(
            Some(sessions[0]),
            TurnTraceRecord {
                turn_id: "turn-0".to_string(),
                title: "t".to_string(),
                phase: "completed".to_string(),
                ..Default::default()
            },
        );
    }

    fixture.cut_over_to_normalized_projection();

    let first = rebuild_index(&fixture.backend(), RebuildOptions::apply()).expect("first rebuild");
    let after_first = store_snapshot(&fixture.load_store());
    let second =
        rebuild_index(&fixture.backend(), RebuildOptions::apply()).expect("second rebuild");
    let after_second = store_snapshot(&fixture.load_store());

    assert_eq!(
        first.rebuilt, second.rebuilt,
        "两次重建的 rebuilt 集合必须一致（幂等）"
    );
    assert_eq!(second.rebuilt.len(), sessions.len(), "两个会话都必须被重建");
    assert!(
        second.mismatched.is_empty(),
        "首次重建后索引已等于 fold 结果，第二次不得再有差异：{:?}",
        second.mismatched
    );
    assert_eq!(
        after_first, after_second,
        "第二次重建不得改动库内容（覆盖语义 + 纯函数 fold ⇒ 幂等）"
    );
}

// ---------------------------------------------------------------------------
// 7. dry_run 零写入（物理凭证：sha256）
// ---------------------------------------------------------------------------

#[test]
fn dry_run_writes_nothing() {
    let fixture = Fixture::new("dry_run");
    let session_id = "pa_2a_dry_run";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(session_id), "u", "a", None, Vec::new())
            .expect("append_turn");
    }
    // 写者句柄全部释放（否则 WAL 仍在由它持有 / 追加，前后指纹不可比）。
    let before = db_fingerprint(&fixture.db());

    // 与 CLI `--dry-run` 同一条打开方式：`mode=ro` + `query_only`，不建表/不迁移。
    let read_only = SqliteSessionBackend::new_read_only(fixture.db());
    let report = rebuild_index(&read_only, RebuildOptions::dry_run()).expect("dry-run rebuild");
    drop(read_only);

    let after = db_fingerprint(&fixture.db());
    assert!(
        report.rebuilt.contains(&session_id.to_string()),
        "预演必须真的扫到并尝试重建（否则「零写入」毫无意义）：{:?}",
        report.rebuilt
    );
    assert_eq!(
        report.scanned,
        1,
        "预演必须扫到 1 个有日志的会话（实际 {}）",
        report.scanned
    );
    assert_eq!(
        before, after,
        "dry-run 必须零写入：sessions.db 与 -wal 的 sha256 前后必须一致"
    );
}

// ---------------------------------------------------------------------------
// 附加：legacy blob 视图的已知差异（清单式钉死，不许出现清单外的差异）
// ---------------------------------------------------------------------------

#[test]
fn legacy_blob_view_diff_is_limited_to_documented_gaps() {
    let fixture = Fixture::new("blob_gap");
    let session_id = "pa_2a_blob_gap";
    {
        let mut store = fixture.store();
        for index in 0..2u32 {
            store
                .append_turn_fallible(
                    Some(session_id),
                    &format!("u{index}"),
                    &format!("a{index}"),
                    None,
                    Vec::new(),
                )
                .expect("append_turn");
        }
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

    // 默认读路径仍是 legacy blob 快照（phase 键不存在）。
    let folded = fixture.folded(session_id);
    let blob_view = fixture.indexed(session_id);
    let issues = session_parity_issues(&folded, &blob_view);
    // 日志命令流尚未携带的两类索引侧产物（写路径缺口，记录在案，不隐藏）：
    // ① `{sid}-legacy-root` 根节点由 `ensure_history_graph` 在整包写时生成，无命令；
    // ② blob 的 trace refs 为陈旧/空 ⇒ `merge_trace_history` 把表内 trace 过滤成空。
    let documented = ["history_nodes", "turn_trace_history"];
    for issue in &issues {
        let field = issue.split(':').next().unwrap_or_default();
        assert!(
            documented.iter().any(|prefix| field.starts_with(prefix)),
            "出现**未记录**的 fold ↔ blob 差异（必须修写路径或补文档，不许默默放过）：{issue}"
        );
    }
    assert!(
        !issues.is_empty(),
        "若这两类缺口已被补上（差异归零），请同步删除本 allow-list 与本用例的说明"
    );
}

// ---------------------------------------------------------------------------
// 附加：日志读侧与 fold 的契约（撕裂尾行 / 未知命令 fail loud）
// ---------------------------------------------------------------------------

#[test]
fn log_reader_drops_the_torn_tail_and_unknown_commands_fail_loud() {
    let fixture = Fixture::new("torn_and_unknown");
    let session_id = "pa_2a_torn";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(session_id), "u", "a", None, Vec::new())
            .expect("append_turn");
    }
    let store = SessionLogStore::new(fixture.log_root());
    let metas = store.list_sessions().expect("list sessions");
    let meta = metas
        .iter()
        .find(|meta| meta.session_id == session_id)
        .expect("session meta");
    let committed = read_log_events(&store, meta).expect("read committed events");
    assert!(!committed.is_empty(), "日志必须有命令信封");
    let folded = fold_session(&committed).expect("fold committed prefix");
    assert_eq!(folded.state.history.len(), 2);

    // 撕裂尾行（无换行结束符）⇒ 丢弃，已提交前缀照旧可 fold。
    let path = fixture.log_path(session_id);
    let mut raw = std::fs::read(&path).expect("read log");
    raw.extend_from_slice(b"{\"seq\":99,\"ts\":1,\"cmd\":{\"type\":\"append_mess");
    std::fs::write(&path, &raw).expect("inject torn tail");
    let after_torn = read_log_events(&store, meta).expect("torn tail is not an error");
    assert_eq!(
        after_torn.len(),
        committed.len(),
        "撕裂尾行必须被丢弃（只保留已提交前缀）"
    );

    // 未知命令不得静默跳过。
    let unknown = vec![LogEvent {
        seq: 42,
        ts: 1,
        cmd: serde_json::json!({"type": "no_such_command", "sessionId": session_id}),
    }];
    let error = fold_session(&unknown).expect_err("unknown command must fail loud");
    assert!(
        error.to_string().contains("no_such_command"),
        "错误信息必须点名未知命令：{error}"
    );
}
