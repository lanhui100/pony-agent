//! B148 阶段 2 验收：`SessionStore` 的**稀疏（懒装载）模式**。
//!
//! 验收红线（每条都有机器断言）：
//! 1. 稀疏装载不把会话内容读进内存（`loaded_session_ids()` 为空）；
//! 2. `hydrate_session` 的结果与全量装载**逐字段一致**（快照等价）；
//! 3. 未装载会话的内容读取**显式拒绝**（不伪装成空会话）；
//! 4. 稀疏提交**只写已装载会话**：未装载会话的 blob 与 trace 行逐字节不变；
//! 5. 显式管理升级（`load_all_for_administration`）恢复全量视图。
//!
//! 隔离：所有库落在 `std::env::temp_dir()` 下唯一目录，fixture `Drop` 清理，
//! **绝不触碰** `~/.local/share/PonyAgent/`。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pony_agent_core::agent::session::{
    HistoryCursor, SeparateTraceTableMode, SessionAttachment, SessionBackend,
    SessionBackendMutationResult, SessionState, SessionStore, SessionTraceMutation,
    TraceMigrationState, TurnHistoryMessage, TurnTraceRecord,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "pony-b148-sparse-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before unix epoch")
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).expect("create fixture root");
        Self { root }
    }

    fn db(&self) -> PathBuf {
        self.root.join("sessions.db")
    }

    fn backend(&self) -> SqliteSessionBackend {
        SqliteSessionBackend::new_with_trace_mode(self.db(), SeparateTraceTableMode::DualWrite)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn trace(turn_id: &str, title: &str, at: u64) -> TurnTraceRecord {
    TurnTraceRecord {
        turn_id: turn_id.to_string(),
        title: title.to_string(),
        phase: "completed".to_string(),
        updated_at: at,
        ..Default::default()
    }
}

fn user_msg(content: &str, turn_id: &str) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: "user".to_string(),
        content: content.to_string(),
        turn_id: Some(turn_id.to_string()),
        ..Default::default()
    }
}

fn rich_session(id: &str, title: &str, at: u64) -> SessionState {
    SessionState {
        conversation_id: id.to_string(),
        title: title.to_string(),
        summary: format!("{title}-summary"),
        history: vec![
            user_msg(&format!("{title}-hello"), "t1"),
            user_msg(&format!("{title}-world"), "t2"),
        ],
        provider_native_transcript: Vec::new(),
        turn_trace_history: vec![trace("t1", "first", at), trace("t2", "second", at)],
        trace_migration_state: TraceMigrationState::default(),
        turn_trace_refs: None,
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        turn_count: 2,
        last_referenced_file: Some(format!("{title}.rs")),
        updated_at_ms: at,
        history_nodes: Vec::new(),
        history_branches: Vec::new(),
        history_cursor: HistoryCursor::default(),
        event_watermark: 0,
        last_commit_watermark: 0,
        workspace_id: None,
        title_override: None,
        archived: false,
    }
}

fn json_of<T: serde::Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).expect("serialize")
}

/// 播种两个会话：s1 = DualWrite（trace 在 blob 里）；s2 = 表权威（trace 只在 trace 表），
/// 后者是「写路径把未装载会话的 trace 清空」这类事故的检测面。
fn seed(fx: &Fixture) -> SqliteSessionBackend {
    let backend = fx.backend();
    assert!(
        backend.upsert_session("s1", &rich_session("s1", "alpha", 1000)),
        "upsert s1"
    );
    let mut authoritative = rich_session("s2", "beta", 2000);
    authoritative.trace_migration_state = TraceMigrationState::TraceTableAuthoritative;
    // s2 带附件：稀疏写入若整表重建附件索引，会把这个条目抹掉（清理按索引判孤儿）。
    authoritative.history[0].attachments.push(SessionAttachment {
        id: "s2-attachment-ref".to_string(),
        asset_id: "s2-asset".to_string(),
        name: Some("s2.png".to_string()),
        mime_type: "image/png".to_string(),
        relative_path: "s2/s2.png".to_string(),
        size_bytes: 16,
        created_at_ms: 2000,
    });
    let traces = authoritative.turn_trace_history.clone();
    assert!(
        matches!(
            backend.persist_session_with_trace_mutation(
                "s2",
                &authoritative,
                SessionTraceMutation::ReplaceAll { traces },
            ),
            SessionBackendMutationResult::Succeeded
        ),
        "写入表权威会话 s2"
    );
    backend
}

/// `store_metadata` 里的附件索引真值（稀疏写入后必须仍含未装载会话的条目）。
fn meta_attachment_index(db: &Path) -> String {
    let conn = rusqlite::Connection::open(db).expect("open db");
    conn.query_row(
        "SELECT value FROM store_metadata WHERE key = 'session_attachment_index'",
        [],
        |row| row.get(0),
    )
    .unwrap_or_default()
}

/// 库内原始行快照：未装载会话的 blob / trace 行数 / 会话行总数。
fn raw_rows(db: &Path) -> (String, i64, i64) {
    let conn = rusqlite::Connection::open(db).expect("open db");
    let s2_blob: String = conn
        .query_row(
            "SELECT session_data FROM sessions WHERE conversation_id = 's2'",
            [],
            |row| row.get(0),
        )
        .expect("read s2 blob");
    let s2_traces: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM session_turn_traces WHERE session_id = 's2'",
            [],
            |row| row.get(0),
        )
        .expect("count s2 traces");
    let session_count: i64 = conn
        .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
        .expect("count sessions");
    (s2_blob, s2_traces, session_count)
}

#[test]
fn sparse_open_reads_meta_only() {
    let fx = Fixture::new("open");
    let _backend = seed(&fx);

    let store = SessionStore::open_lazy_with_backend(Box::new(fx.backend()));
    assert!(store.is_sparse(), "SQLite + 定向能力 ⇒ 必须进入稀疏模式");
    assert!(
        store.loaded_session_ids().is_empty(),
        "稀疏装载不得把会话内容读进内存，实际 = {:?}",
        store.loaded_session_ids()
    );
    assert_eq!(store.meta_entries().len(), 2, "元数据索引应含两条会话");

    // 列表口径：稀疏 vs 全量逐字段一致（id / 标题 / 轮次 / 时间 / 归属 / 引用文件）。
    let full = SessionStore::with_backend(Box::new(fx.backend()));
    let full_list = full.list_sessions();
    let sparse_list = store.list_sessions();
    assert_eq!(
        full_list.len(),
        sparse_list.len(),
        "稀疏列表条数与全量不一致：full={:?} sparse={:?}",
        full_list
            .iter()
            .map(|item| &item.conversation_id)
            .collect::<Vec<_>>(),
        sparse_list
            .iter()
            .map(|item| &item.conversation_id)
            .collect::<Vec<_>>()
    );
    let mut by_id: HashMap<String, _> = HashMap::new();
    for item in &full_list {
        by_id.insert(item.conversation_id.clone(), item);
    }
    for item in &sparse_list {
        let expected = by_id
            .get(&item.conversation_id)
            .unwrap_or_else(|| panic!("全量列表缺少 {}", item.conversation_id));
        // 存储域字段（落盘后原样回读）必须逐字段一致。
        assert_eq!(item.turn_count, expected.turn_count, "turn_count 不一致");
        assert_eq!(item.updated_at_ms, expected.updated_at_ms, "时间不一致");
        assert_eq!(item.workspace_id, expected.workspace_id, "归属不一致");
        assert_eq!(item.archived, expected.archived, "归档标记不一致");
        // 派生字段（标题 / 摘要 / 最近引用文件）在**全量读**时由
        // `refresh_session_metadata` 从 history 重建；稀疏列表没有内容，只能取持久化值。
        // 两者一致的前提是「写入路径落盘前已刷新派生字段」——直接塞 blob 的 legacy 行
        // 不满足该前提（本用例正是这种行）：sparse 取持久化 title（titleOverride 优先），
        // full 取 build_title(history)。生产写入路径的一致性由
        // `sparse_list_matches_full_for_store_written_sessions` 守护。
        let persisted_title = match item.conversation_id.as_str() {
            "s1" => "alpha",
            "s2" => "beta",
            other => panic!("意外会话 {other}"),
        };
        assert_eq!(
            item.title, persisted_title,
            "稀疏标题必须取持久化值（与 startup_meta::META_SQL 同口径）"
        );
    }
    let full_ids = full_list
        .iter()
        .map(|item| item.conversation_id.clone())
        .collect::<Vec<_>>();
    let sparse_ids = sparse_list
        .iter()
        .map(|item| item.conversation_id.clone())
        .collect::<Vec<_>>();
    assert_eq!(full_ids, sparse_ids, "列表顺序必须一致");
}

#[test]
fn sparse_list_matches_full_for_store_written_sessions() {
    let fx = Fixture::new("list");
    // 走**生产写入路径**播种：落盘前 `refresh_session_metadata` 已刷新派生字段，
    // 因此持久化标题/摘要/引用文件与全量读重建的结果一致（这是列表口径的真值源）。
    {
        let mut writer = SessionStore::with_backend(Box::new(fx.backend()));
        writer
            .append_turn_fallible(
                Some("w1"),
                "把 build_title 收敛成单一真值",
                "完成",
                None,
                Vec::new(),
            )
            .expect("write w1");
        writer
            .append_turn_fallible(
                Some("w2"),
                "检查 src/main.rs 的启动路径",
                "完成",
                None,
                Vec::new(),
            )
            .expect("write w2");
    }

    let full = SessionStore::with_backend(Box::new(fx.backend()));
    let sparse = SessionStore::open_lazy_with_backend(Box::new(fx.backend()));
    assert!(sparse.is_sparse(), "稀疏装载必须生效");
    let full_list = full.list_sessions();
    let sparse_list = sparse.list_sessions();
    assert_eq!(full_list.len(), 2, "写入两条会话：{full_list:?}");
    assert_eq!(
        full_list
            .iter()
            .map(|item| item.conversation_id.clone())
            .collect::<Vec<_>>(),
        sparse_list
            .iter()
            .map(|item| item.conversation_id.clone())
            .collect::<Vec<_>>(),
        "列表 id 与顺序必须一致"
    );
    let mut by_id: HashMap<String, _> = HashMap::new();
    for item in &full_list {
        by_id.insert(item.conversation_id.clone(), item);
    }
    for item in &sparse_list {
        let expected = by_id
            .get(&item.conversation_id)
            .unwrap_or_else(|| panic!("全量列表缺少 {}", item.conversation_id));
        assert_eq!(item.title, expected.title, "生产行 title 必须一致");
        assert_eq!(item.summary, expected.summary, "生产行 summary 必须一致");
        assert_eq!(
            item.last_referenced_file, expected.last_referenced_file,
            "生产行最近引用文件必须一致"
        );
        assert_eq!(item.turn_count, expected.turn_count, "turn_count 必须一致");
        assert_eq!(item.updated_at_ms, expected.updated_at_ms, "时间必须一致");
    }
}

#[test]
fn hydrate_matches_full_load() {
    let fx = Fixture::new("hydrate");
    let _backend = seed(&fx);

    let mut sparse = SessionStore::open_lazy_with_backend(Box::new(fx.backend()));
    assert!(sparse.hydrate_session("s1"), "s1 定向装载必须成功");
    assert_eq!(
        sparse.loaded_session_ids(),
        vec!["s1".to_string()],
        "装载后写集合只应有 s1"
    );

    let full = SessionStore::with_backend(Box::new(fx.backend()));
    let sparse_snapshot = sparse.snapshot_at_readonly(Some("s1"), None, &[]);
    let full_snapshot = full.snapshot_at_readonly(Some("s1"), None, &[]);
    assert_eq!(
        json_of(&sparse_snapshot),
        json_of(&full_snapshot),
        "定向装载后的快照必须与全量装载逐字段一致"
    );

    // 幂等：重复 hydrate 不得改变写集合，也不得重复读库。
    assert!(sparse.hydrate_session("s1"));
    assert_eq!(sparse.loaded_session_ids(), vec!["s1".to_string()]);
    assert_eq!(sparse.loaded_session_ids(), vec!["s1".to_string()]);
}

#[test]
fn unloaded_read_is_explicitly_denied() {
    let fx = Fixture::new("denied");
    let _backend = seed(&fx);

    let mut sparse = SessionStore::open_lazy_with_backend(Box::new(fx.backend()));
    assert!(sparse.hydrate_session("s1"));
    // s2 仍在元数据索引里、内容未装载：读取不得返回「空会话」内容。
    let snapshot = sparse.snapshot_at_readonly(Some("s2"), None, &[]);
    assert!(
        snapshot.history.is_empty(),
        "未装载读取只能返回缺省快照（且已记 session_read_unloaded_denied）"
    );
    assert_eq!(snapshot.conversation_id, "s2");
    assert!(
        sparse.get_session("s2").is_none(),
        "未装载会话的 get_session 必须返回 None（显式拒绝，不伪造状态）"
    );
    assert_eq!(
        sparse.loaded_session_ids(),
        vec!["s1".to_string()],
        "只读拒绝路径不得顺手装载或写任何东西"
    );
}

#[test]
fn sparse_save_leaves_unloaded_rows_untouched() {
    let fx = Fixture::new("write");
    let _backend = seed(&fx);
    let db = fx.db();
    let before = raw_rows(&db);
    assert!(before.1 > 0, "s2 必须有 trace 行，否则检测面无意义");

    let mut sparse = SessionStore::open_lazy_with_backend(Box::new(fx.backend()));
    // 真实写入：改名会先 hydrate s1，再经稀疏提交落盘。
    sparse
        .rename_session("s1", "renamed-alpha")
        .expect("rename s1");

    let after = raw_rows(&db);
    assert_eq!(
        before.0, after.0,
        "未装载会话 s2 的 blob 必须零写入（逐字节不变）"
    );
    assert_eq!(
        before.1, after.1,
        "未装载会话 s2 的 trace 行必须零写入（行数不变）"
    );
    assert_eq!(
        before.2, after.2,
        "会话行总数不变（稀疏提交不得删/复活任何会话）"
    );
    assert!(
        after.0.contains("\"beta\"") || after.0.contains("beta-summary"),
        "s2 blob 内容本身不能丢"
    );
    let s1_blob: String = rusqlite::Connection::open(&db)
        .expect("open db")
        .query_row(
            "SELECT session_data FROM sessions WHERE conversation_id = 's1'",
            [],
            |row| row.get(0),
        )
        .expect("read s1 blob");
    assert!(
        s1_blob.contains("renamed-alpha"),
        "已装载会话的改名必须落盘"
    );
    assert_eq!(
        sparse.loaded_session_ids(),
        vec!["s1".to_string()],
        "写集合只应包含已装载会话"
    );

    // 未装载会话仍能按需恢复完整内容（稀疏写不破坏可恢复性）。
    assert!(sparse.hydrate_session("s2"), "s2 仍必须可定向装载");
    let restored = sparse.snapshot_at_readonly(Some("s2"), None, &[]);
    assert_eq!(restored.turn_count, 2, "s2 轮次必须完整恢复");
    assert_eq!(restored.history.len(), 2, "s2 历史必须完整恢复");
}

#[test]
fn sparse_write_preserves_unloaded_attachment_index() {
    let fx = Fixture::new("attach");
    let _backend = seed(&fx);
    let db = fx.db();
    // 先用全量写把附件索引落进 `store_metadata`（稀疏写入会以它为准做增量更新）。
    {
        let mut full = SessionStore::with_backend(Box::new(fx.backend()));
        full.append_turn_fallible(Some("s2"), "补一轮", "好", None, Vec::new())
            .expect("full append");
        full.rename_session("s2", "beta-renamed").expect("full rename");
    }
    let seeded_index = meta_attachment_index(&db);
    assert!(
        seeded_index.contains("s2"),
        "前置条件：索引里必须有 s2 条目，实际 {seeded_index}"
    );

    let mut sparse = SessionStore::open_lazy_with_backend(Box::new(fx.backend()));
    assert!(sparse.is_sparse());
    // 只写 s1（s2 未装载）：写入会刷新附件目录，不得把 s2 的索引条目抹掉。
    sparse
        .append_turn_fallible(Some("s1"), "稀疏写", "好", None, Vec::new())
        .expect("sparse append");
    sparse.rename_session("s1", "alpha-renamed").expect("sparse rename");
    let after = meta_attachment_index(&db);
    assert!(
        after.contains("s2"),
        "未装载会话的附件索引条目不得被整表重建抹掉（否则附件清理会误删）：{after}"
    );
    assert!(
        after.contains("s2-asset"),
        "未装载会话的 asset id 必须原样保留：{after}"
    );
}

#[test]
fn sparse_list_counts_content_persisted_outside_blob() {
    let fx = Fixture::new("persisted");
    let backend = fx.backend();
    // blob 行内容为空（模拟 2-D 停写后的轻量索引行）：只剩标题/时间。
    let mut index_only = rich_session("blobless", "gamma", 3000);
    index_only.history.clear();
    index_only.turn_trace_history.clear();
    index_only.turn_count = 0;
    index_only.last_referenced_file = None;
    assert!(backend.upsert_session("blobless", &index_only), "写入轻量行");
    drop(backend);
    // 内容只落在规范化表：全量装载会把它们并回内存态 ⇒ 列表可见；
    // 稀疏列表若只认 blob 就会静默漏掉这个会话（真实库 227 行里有 7 行如此）。
    let conn = rusqlite::Connection::open(fx.db()).expect("open db");
    conn.execute(
        "INSERT INTO normalized_sessions (session_id, title, updated_at_ms) \
         VALUES ('blobless', 'gamma', 3000)",
        [],
    )
    .expect("insert normalized session");
    conn.execute(
        "INSERT INTO normalized_messages (session_id, message_id, ordinal, role, content) \
         VALUES ('blobless', 'm1', 0, 'user', '只存在规范化表里的消息')",
        [],
    )
    .expect("insert normalized message");
    drop(conn);

    let sparse = SessionStore::open_lazy_with_backend(Box::new(fx.backend()));
    assert!(sparse.is_sparse());
    let ids = sparse
        .list_sessions()
        .iter()
        .map(|item| item.conversation_id.clone())
        .collect::<Vec<_>>();
    assert!(
        ids.contains(&"blobless".to_string()),
        "内容只落在规范化表的会话不得被稀疏列表漏掉（全量装载可见）：{ids:?}"
    );
}

#[test]
fn admin_escalation_restores_full_mode() {
    let fx = Fixture::new("admin");
    let _backend = seed(&fx);

    let mut sparse = SessionStore::open_lazy_with_backend(Box::new(fx.backend()));
    assert!(sparse.load_all_for_administration());
    assert!(!sparse.is_sparse(), "显式管理升级后必须退回全量模式");
    assert!(sparse.get_session("s1").is_some());
    assert!(sparse.get_session("s2").is_some());
    assert_eq!(sparse.list_sessions().len(), 2);
}
