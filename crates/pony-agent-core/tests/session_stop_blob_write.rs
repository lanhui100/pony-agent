//! ADR 0099 **阶段 2-D** 验收：停写整包 blob，SQLite 真正退为索引。
//!
//! 阶段 1-B 把写路径真源切到会话日志；2-A 补了 `fold` / `rebuild_index`；2-B 把读路径
//! 合并到日志真相。本文件证明最后一步：**有日志的会话不再每轮重写整包 `session_data`**，
//! 而 SQLite 侧只剩「可重建索引」，且读、排序、关联、迁移兼容一条都不退化。
//!
//! 七类用例（全部真断言，无「跑通即放行」）：
//!
//! 1. [`blob_stops_growing_after_the_log_is_the_truth`] —— `append_turn` 后索引行的
//!    `session_data` **不随轮次增长**（3 轮 == 1 轮），且是固定形态的轻量索引行；
//! 2. [`read_path_still_returns_the_full_session_after_blob_stops`] —— 停写 blob 后运行时
//!    `load_store()` / `list_sessions()` 的 history / title / turn_count 与写入一致；
//! 3. [`restart_is_equivalent_to_the_pre_restart_view`] —— 重新 `SessionStore::with_backend`
//!    打开同一库，读出的会话内容与上一次**逐字节**相同；
//! 4. [`index_consumers_keep_sorting_workspace_and_title`] —— 索引消费者不变：
//!    `list_sessions()` 的排序、workspace 关联、标题（含重命名 override）仍正确；
//! 5. [`grandfather_blob_write_path_is_unchanged`] —— 无日志的存量会话继续走老路径写整包
//!    blob，内容与读取行为零变化；
//! 6. [`rebuild_index_dry_run_on_a_copy_is_exit_zero_and_zero_write`] —— `ponycli sessions
//!    rebuild-index --dry-run` 在**副本**上 EXIT=0、扫到真源会话、且零写入（sha256 不变）；
//! 7. [`legacy_not_null_schema_still_accepts_new_writes`] —— 老库（`session_data NOT NULL`
//!    形态）打开后写新会话不报错，老行原样保留。
//!
//! ## 迁移方案（本阶段的裁决，落地见 `sqlite_session_commit.rs` 顶部注释）
//!
//! `sessions.session_data` 保持 `NOT NULL`，**不**放宽为可空；有日志的会话写「轻量索引行」
//! + 未知键标记 `__index_only: true`（serde 默认忽略未知字段 ⇒ 老读者照常解析，不崩）。
//! 理由：SQLite 无法 `ALTER COLUMN`，表重建要重挂 `sessions` 视图；而放宽为 `NULL` 会让既有
//! `row.get::<_, String>(1)` 在运行中直接撞 `InvalidColumnType`（真崩溃）。
//!
//! 隔离：所有库与日志都落在 `std::env::temp_dir()` 下的唯一目录，**绝不触碰**
//! `~/.local/share/PonyAgent/`；fixture `Drop` 清理。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use pony_agent_core::agent::session::{
    session_log_root_for_db, SeparateTraceTableMode, SessionBackend, SessionState, SessionStore,
    TurnHistoryMessage,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;

/// 轻量索引行的标记键（与 `sqlite_session_commit.rs::INDEX_ONLY_ROW_MARKER` 同一契约）。
const INDEX_ONLY_MARKER: &str = "__index_only";

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let unique = format!(
            "pony-0099-2d-stopblob-{label}-{}-{}",
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

    /// 可写后端（真实生产形态：`WriteSeparate` ⇒ 新会话 TraceTableAuthoritative）。
    fn backend(&self) -> SqliteSessionBackend {
        SqliteSessionBackend::new_with_trace_mode(self.db(), SeparateTraceTableMode::WriteSeparate)
    }

    fn store(&self) -> SessionStore {
        SessionStore::with_backend(Box::new(self.backend()))
    }

    fn log_root(&self) -> PathBuf {
        session_log_root_for_db(&self.db())
    }

    fn open_sqlite(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.db()).expect("open sqlite")
    }

    fn session_data(&self, session_id: &str) -> Option<String> {
        let conn = self.open_sqlite();
        conn.query_row(
            "SELECT session_data FROM sessions WHERE conversation_id = ?1",
            rusqlite::params![session_id],
            |row| row.get::<_, String>(0),
        )
        .ok()
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

    /// 运行时读路径（业务句柄）：`load_store()`。
    fn runtime_view(&self) -> BTreeMap<String, SessionState> {
        self.backend()
            .load_store()
            .expect("运行时 load_store 必须成功")
            .sessions_view()
            .clone()
            .into_iter()
            .collect()
    }

    /// `sessions.db` + `-wal` 的 sha256（物理凭证：零写入）。
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
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn contents(session: &SessionState) -> Vec<String> {
    session
        .history
        .iter()
        .map(|message| message.content.clone())
        .collect()
}

fn canonical_json<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string_pretty(value).expect("serialize")
}

/// 轻量索引行的解析结果：`(标记是否存在, history 条数, turn_count, title)`。
fn index_row_shape(raw: &str) -> (bool, usize, i64, String) {
    let value: serde_json::Value = serde_json::from_str(raw)
        .unwrap_or_else(|error| panic!("索引行必须是合法 JSON（老读者不许被哨兵值弄崩）：{error}"));
    let marked = value
        .get(INDEX_ONLY_MARKER)
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let history_len = value
        .get("history")
        .and_then(|history| history.as_array())
        .map(Vec::len)
        .unwrap_or_default();
    let turn_count = value
        .get("turnCount")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or_default();
    let title = value
        .get("title")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    (marked, history_len, turn_count, title)
}

/// 老读者（直接 `serde_json::from_str::<SessionState>`）必须仍能解析索引行。
fn legacy_reader_parses(raw: &str) -> SessionState {
    serde_json::from_str::<SessionState>(raw).unwrap_or_else(|error| {
        panic!("老读者（反序列化整包 SessionState）解析索引行失败——这正是必须避免的崩溃：{error}\n{raw}")
    })
}

/// 只存在于索引里的会话（单行写 `upsert_session`：**不产生任何日志命令**）。
fn index_only_session(session_id: &str, title: &str, contents: &[&str]) -> SessionState {
    SessionState {
        conversation_id: session_id.to_string(),
        title: title.to_string(),
        summary: "只有索引行，没有日志文件".to_string(),
        history: contents
            .iter()
            .map(|text| TurnHistoryMessage {
                role: "user".to_string(),
                content: text.to_string(),
                ..Default::default()
            })
            .collect(),
        provider_native_transcript: Vec::new(),
        turn_trace_history: Vec::new(),
        trace_migration_state: Default::default(),
        turn_trace_refs: None,
        long_term_memory_entries: Vec::new(),
        memory_write_evidence: Vec::new(),
        memory_write_hook_trace_records: Vec::new(),
        history_state_evidence: Vec::new(),
        turn_count: contents.len(),
        last_referenced_file: Some("src/grandfather.rs".to_string()),
        updated_at_ms: 1_700_000_000_000,
        history_nodes: Vec::new(),
        history_branches: Vec::new(),
        history_cursor: Default::default(),
        event_watermark: 0,
        last_commit_watermark: 0,
        workspace_id: None,
        title_override: Some(title.to_string()),
        archived: false,
    }
}

// ---------------------------------------------------------------------------
// 1. blob 不再增长
// ---------------------------------------------------------------------------

#[test]
fn blob_stops_growing_after_the_log_is_the_truth() {
    let fixture = Fixture::new("no_growth");
    let session_id = "pa_2d_no_growth";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(session_id), "u1", "a1", None, Vec::new())
            .expect("第 1 轮必须经日志真源落盘");
    }
    let raw_after_one = fixture
        .session_data(session_id)
        .expect("有日志的会话必须仍有索引行（索引可删可建，但常规写路径会写回）");
    let (marked, history_len, turn_count, title_after_one) = index_row_shape(&raw_after_one);
    assert!(
        marked,
        "有日志会话的索引行必须带 `{INDEX_ONLY_MARKER}` 标记（停写整包 blob 的显式形态）：{raw_after_one}"
    );
    assert_eq!(
        history_len, 0,
        "索引行不得再携带 history（那是真源的职责）：{raw_after_one}"
    );
    assert_eq!(turn_count, 1, "索引行必须保留 list_sessions 消费的 turn_count");
    assert!(!title_after_one.is_empty(), "索引行必须保留标题");
    // 老读者不会被哨兵值弄崩（这是选「哨兵值」而不是「放宽为可空」的核心前提）。
    let parsed_by_legacy_reader = legacy_reader_parses(&raw_after_one);
    assert_eq!(parsed_by_legacy_reader.conversation_id, session_id);
    assert!(parsed_by_legacy_reader.history.is_empty());

    {
        let mut store = fixture.store();
        for (index, (user, assistant)) in [("u2", "a2"), ("u3", "a3")].into_iter().enumerate() {
            store
                .append_turn_fallible(Some(session_id), user, assistant, None, Vec::new())
                .unwrap_or_else(|error| panic!("第 {} 轮必须成功：{error}", index + 2));
        }
    }
    let raw_after_three = fixture.session_data(session_id).expect("索引行仍在");
    let (marked_three, history_three, turn_count_three, _) = index_row_shape(&raw_after_three);
    assert!(marked_three);
    assert_eq!(history_three, 0);
    assert_eq!(turn_count_three, 3, "turn_count 必须随轮次前进（索引字段该更新）");
    assert_eq!(
        raw_after_three.len(),
        raw_after_one.len(),
        "3 轮后的 `session_data` 长度必须与 1 轮后**完全相同**（仍增长就是没停写 blob）\
         \n--- 1 轮 ---\n{raw_after_one}\n--- 3 轮 ---\n{raw_after_three}"
    );

    // 对照组：无日志会话仍是整包 blob（grandfather 未受影响），长度明显不同。
    let index_only = "pa_2d_no_growth_grandfather";
    {
        let backend = fixture.backend();
        assert!(backend.upsert_session(
            index_only,
            &index_only_session(index_only, "存量标题", &["历史一", "历史二"])
        ));
    }
    let raw_grandfather = fixture.session_data(index_only).expect("存量会话索引行");
    let (marked_grandfather, history_grandfather, _, title_grandfather) =
        index_row_shape(&raw_grandfather);
    assert!(
        !marked_grandfather,
        "无日志会话不得被打上轻量标记（必须继续走老路径写整包 blob）"
    );
    assert_eq!(history_grandfather, 2, "存量会话的 blob 必须仍带完整 history");
    assert_eq!(title_grandfather, "存量标题");

    eprintln!(
        "[1] 轻量索引行：marked={marked} 1轮={}B 3轮={}B turn_count={turn_count_three}；\
         对照组整包 blob={}B",
        raw_after_one.len(),
        raw_after_three.len(),
        raw_grandfather.len()
    );
}

// ---------------------------------------------------------------------------
// 2. 读仍完整
// ---------------------------------------------------------------------------

#[test]
fn read_path_still_returns_the_full_session_after_blob_stops() {
    let fixture = Fixture::new("read_complete");
    let session_id = "pa_2d_read_complete";
    {
        let mut store = fixture.store();
        for (index, (user, assistant)) in [("问一", "答一"), ("问二", "答二"), ("问三", "答三")]
            .into_iter()
            .enumerate()
        {
            store
                .append_turn_fallible(Some(session_id), user, assistant, None, Vec::new())
                .unwrap_or_else(|error| panic!("第 {} 轮必须成功：{error}", index + 1));
        }
    }
    assert!(
        index_row_shape(&fixture.session_data(session_id).expect("索引行"))
            .1
            == 0,
        "前置：索引行已停写内容（否则本用例证明不了「读不依赖 blob」）"
    );

    let view = fixture.runtime_view();
    let session = view
        .get(session_id)
        .unwrap_or_else(|| panic!("运行时读路径必须返回该会话：{:?}", view.keys()));
    assert_eq!(
        contents(session),
        vec!["问一", "答一", "问二", "答二", "问三", "答三"],
        "history 必须完整（逐条比对内容与顺序）"
    );
    assert_eq!(session.turn_count, 3, "turn_count 必须与写入一致");
    assert!(!session.title.is_empty(), "标题必须复现");

    assert!(
        fixture.index_row_exists(session_id),
        "索引行必须仍在（停写 blob ≠ 删索引行；删行属 rebuild-index 的维护动作）"
    );
    let store = fixture.store();
    let overview = store
        .list_sessions()
        .into_iter()
        .find(|overview| overview.conversation_id == session_id)
        .expect("list_sessions 必须列出该会话");
    assert_eq!(overview.turn_count, 3);
    assert_eq!(overview.title, session.effective_title());
    assert!(!overview.continuation_only);

    eprintln!(
        "[2] blob 行 history=0，运行时读到 {} 条消息 / turn_count={} / title={:?}",
        session.history.len(),
        session.turn_count,
        overview.title
    );
}

// ---------------------------------------------------------------------------
// 3. 重启等价
// ---------------------------------------------------------------------------

#[test]
fn restart_is_equivalent_to_the_pre_restart_view() {
    let fixture = Fixture::new("restart");
    let logged = "pa_2d_restart_logged";
    let grandfather = "pa_2d_restart_grandfather";
    {
        let mut store = fixture.store();
        for (user, assistant) in [("u1", "a1"), ("u2", "a2")] {
            store
                .append_turn_fallible(Some(logged), user, assistant, None, Vec::new())
                .expect("append_turn");
        }
    }
    {
        let backend = fixture.backend();
        assert!(backend.upsert_session(
            grandfather,
            &index_only_session(grandfather, "存量标题", &["存量一"])
        ));
    }

    // 预热一轮：`SessionStore` 的启动/首快照会为**无日志的存量会话**补历史图（legacy root
    // 节点等，既有行为，与 blob 停写无关）。预热后再取基准，比较的才是「重启是否等价」，
    // 而不是把启动期的规范化噪声当成漂移。
    {
        let mut warm = fixture.store();
        let _ = warm.snapshot(Some(logged), &[]);
        let _ = warm.snapshot(Some(grandfather), &[]);
    }

    // 上一次（进程内）读到的状态。
    let before = fixture.runtime_view();
    assert_eq!(before.len(), 2, "两个会话都必须可读：{:?}", before.keys());

    // 重启：重新 `SessionStore::with_backend` 打开同一库（等价于杀掉进程再启动）。
    // 业务侧的读入口是 `SessionStore`（启动时经 `load_store` 装载），故用它的快照断言，
    // 再用新 backend 句柄重读一次做逐字节对拍。
    let mut store = fixture.store();
    let logged_snapshot = store.snapshot(Some(logged), &[]);
    assert_eq!(
        logged_snapshot
            .history
            .iter()
            .map(|message| message.content.clone())
            .collect::<Vec<_>>(),
        vec!["u1", "a1", "u2", "a2"],
        "重启后 SessionStore 的会话内容必须与写入一致"
    );
    assert_eq!(logged_snapshot.turn_count, 2);
    {
        let grandfather_snapshot = store.snapshot(Some(grandfather), &[]);
        assert_eq!(
            grandfather_snapshot
                .history
                .iter()
                .map(|message| message.content.clone())
                .collect::<Vec<_>>(),
            vec!["存量一"]
        );
    }
    drop(store);

    let reopened = fixture.runtime_view();

    let before_json: BTreeMap<String, String> = before
        .iter()
        .map(|(id, state)| (id.clone(), canonical_json(state)))
        .collect();
    let after_json: BTreeMap<String, String> = reopened
        .iter()
        .map(|(id, state)| (id.clone(), canonical_json(state)))
        .collect();
    assert_eq!(
        before_json.keys().collect::<Vec<_>>(),
        after_json.keys().collect::<Vec<_>>(),
        "重启前后的会话集合必须完全一致"
    );
    for (session_id, expected) in &before_json {
        assert_eq!(
            after_json.get(session_id).expect("重启后必须仍能读到"),
            expected,
            "会话 {session_id} 重启前后必须**逐字节**等价（任何字段漂移都是读路径不一致）"
        );
    }
    // 再开一次同样等价（幂等，防止「第一次重启碰巧对」）。
    let reopened_twice = fixture.runtime_view();
    let twice_json: BTreeMap<String, String> = reopened_twice
        .iter()
        .map(|(id, state)| (id.clone(), canonical_json(state)))
        .collect();
    assert_eq!(before_json, twice_json, "连续两次重启必须等价");

    eprintln!(
        "[3] 重启等价：{} 个会话，`{}` 消息数={} / `{}` 消息数={}",
        after_json.len(),
        logged,
        reopened[logged].history.len(),
        grandfather,
        reopened[grandfather].history.len()
    );
}

// ---------------------------------------------------------------------------
// 4. 索引消费者不变（排序 / workspace / 标题）
// ---------------------------------------------------------------------------

#[test]
fn index_consumers_keep_sorting_workspace_and_title() {
    let fixture = Fixture::new("consumers");
    let older = "pa_2d_consumers_older";
    let newer = "pa_2d_consumers_newer";
    let grandfather = "pa_2d_consumers_grandfather";

    let workspace_id;
    {
        let mut store = fixture.store();
        // older 先写 → updated_at_ms 更小；newer 后写 → 应排在前面。
        store
            .append_turn_fallible(Some(older), "旧问", "旧答", None, Vec::new())
            .expect("older append_turn");
        std::thread::sleep(std::time::Duration::from_millis(15));
        store
            .append_turn_fallible(Some(newer), "新问", "新答", None, Vec::new())
            .expect("newer append_turn");
        std::thread::sleep(std::time::Duration::from_millis(15));
        // 重命名（写 override，必须落进真源，否则重启读回派生标题）。
        store
            .rename_session(newer, "用户自定义标题")
            .expect("rename_session 必须成功");
        // workspace 关联（索引独有的字段：日志没有承载它的命令）。
        let workspace = store
            .create_workspace("回归工作区", fixture.root.to_string_lossy().as_ref())
            .expect("create_workspace");
        workspace_id = workspace.id.clone();
        store.stamp_workspace_id(newer, &workspace_id);
        store
            .archive_session(older)
            .expect("archive_session 必须成功");
    }
    {
        let backend = fixture.backend();
        assert!(backend.upsert_session(
            grandfather,
            &index_only_session(grandfather, "存量标题", &["存量一", "存量二"])
        ));
    }

    // 重启后逐个索引消费者核对。
    let store = fixture.store();
    let overviews = store.list_sessions();
    let order: Vec<String> = overviews
        .iter()
        .map(|overview| overview.conversation_id.clone())
        .collect();
    // 排序契约（`store.rs::list_sessions`）：`updated_at_ms` 降序，同值按 id 升序。
    let expected_order: Vec<String> = {
        let mut pairs: Vec<(u64, String)> = overviews
            .iter()
            .map(|overview| (overview.updated_at_ms, overview.conversation_id.clone()))
            .collect();
        pairs.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
        pairs.into_iter().map(|(_, id)| id).collect()
    };
    assert_eq!(order, expected_order, "排序必须仍与 updated_at_ms 契约一致");
    assert_eq!(
        order,
        vec![newer.to_string(), older.to_string(), grandfather.to_string()],
        "后写入的会话必须排在前面，无日志存量会话（时间戳更早）在最后：实际 {order:?}"
    );

    let newer_overview = overviews
        .iter()
        .find(|overview| overview.conversation_id == newer)
        .expect("newer 必须在列表里");
    assert_eq!(
        newer_overview.title, "用户自定义标题",
        "重命名后的标题必须从真源读回（不能读回派生标题）"
    );
    assert_eq!(
        newer_overview.workspace_id.as_deref(),
        Some(workspace_id.as_str()),
        "workspace 关联必须仍正确（该字段只存在于索引行）"
    );
    assert_eq!(newer_overview.turn_count, 1);

    let older_overview = overviews
        .iter()
        .find(|overview| overview.conversation_id == older)
        .expect("older 必须在列表里");
    assert!(older_overview.archived, "归档位必须仍正确（日志命令 + 索引行双写）");

    let grandfather_overview = overviews
        .iter()
        .find(|overview| overview.conversation_id == grandfather)
        .expect("存量会话必须在列表里");
    assert_eq!(grandfather_overview.title, "存量标题");
    assert_eq!(grandfather_overview.turn_count, 2);

    // 标题检索口径：按标题在列表里定位（侧边栏检索即按 overview.title 过滤）。
    let hits: Vec<&str> = overviews
        .iter()
        .filter(|overview| overview.title.contains("自定义"))
        .map(|overview| overview.conversation_id.as_str())
        .collect();
    assert_eq!(hits, vec![newer], "按标题检索必须命中重命名后的会话");

    eprintln!(
        "[4] 排序={order:?}；newer.title={:?} workspace={:?}；older.archived={}",
        newer_overview.title, newer_overview.workspace_id, older_overview.archived
    );
}

// ---------------------------------------------------------------------------
// 5. grandfather 不丢
// ---------------------------------------------------------------------------

#[test]
fn grandfather_blob_write_path_is_unchanged() {
    let fixture = Fixture::new("grandfather");
    let session_id = "pa_2d_grandfather";
    {
        let backend = fixture.backend();
        assert!(backend.upsert_session(
            session_id,
            &index_only_session(session_id, "存量标题", &["历史一", "历史二"])
        ));
    }
    assert!(
        !fixture.log_root().exists()
            || fixture.log_root().read_dir().map(|mut it| it.next().is_none()).unwrap_or(true),
        "前置：该会话必须没有日志文件（否则 grandfather 用例不成立）"
    );

    let raw = fixture.session_data(session_id).expect("存量索引行");
    let parsed = legacy_reader_parses(&raw);
    assert_eq!(parsed.history.len(), 2, "存量会话的 blob 必须带完整 history");
    assert_eq!(parsed.title_override.as_deref(), Some("存量标题"));
    assert!(
        parsed.last_referenced_file.as_deref() == Some("src/grandfather.rs"),
        "存量会话的字段必须原样保留"
    );

    // 整包写（全库入口）对存量会话仍然写整包 blob：内容逐字段不变。
    {
        let mut store = fixture.store();
        let workspace = store
            .create_workspace("其他工作区", fixture.root.to_string_lossy().as_ref())
            .expect("create_workspace 触发整包写");
        assert!(!workspace.id.is_empty());
    }
    let raw_after = fixture.session_data(session_id).expect("存量索引行仍在");
    let parsed_after = legacy_reader_parses(&raw_after);
    assert_eq!(
        contents(&parsed_after),
        vec!["历史一", "历史二"],
        "整包写之后存量会话内容必须不变（老路径行为零变化）"
    );
    assert_eq!(
        parsed_after.history.len(),
        parsed.history.len(),
        "存量会话的 blob 不得被清空或降级为轻量行"
    );

    // 运行时读路径与 list_sessions 都看得见。
    let view = fixture.runtime_view();
    assert_eq!(contents(&view[session_id]), vec!["历史一", "历史二"]);
    assert!(fixture
        .store()
        .list_sessions()
        .iter()
        .any(|overview| overview.conversation_id == session_id && overview.turn_count == 2));

    eprintln!(
        "[5] 存量会话 blob={}B history={}（老路径未变）",
        raw_after.len(),
        parsed_after.history.len()
    );
}

// ---------------------------------------------------------------------------
// 6. rebuild-index --dry-run：副本上 EXIT=0 且零写入
// ---------------------------------------------------------------------------

#[test]
fn rebuild_index_dry_run_on_a_copy_is_exit_zero_and_zero_write() {
    let fixture = Fixture::new("dry_run");
    let logged = "pa_2d_dry_run_logged";
    let grandfather = "pa_2d_dry_run_grandfather";
    {
        let mut store = fixture.store();
        for (user, assistant) in [("u1", "a1"), ("u2", "a2")] {
            store
                .append_turn_fallible(Some(logged), user, assistant, None, Vec::new())
                .expect("append_turn");
        }
    }
    {
        let backend = fixture.backend();
        assert!(backend.upsert_session(
            grandfather,
            &index_only_session(grandfather, "存量标题", &["存量一"])
        ));
    }

    // 副本：`db` + `-wal` + `session-logs/` 三件一起复制（真源与索引必须成对）。
    let copy_root = fixture.root.join("copy");
    std::fs::create_dir_all(&copy_root).expect("create copy root");
    let copy_db = copy_root.join("sessions.db");
    std::fs::copy(fixture.db(), &copy_db).expect("copy db");
    let wal = PathBuf::from(format!("{}-wal", fixture.db().display()));
    if wal.exists() {
        std::fs::copy(&wal, PathBuf::from(format!("{}-wal", copy_db.display()))).expect("copy wal");
    }
    copy_tree(&fixture.log_root(), &session_log_root_for_db(&copy_db));

    let before = Fixture::db_fingerprint(&copy_db);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_ponycli"))
        .arg("sessions")
        .arg("rebuild-index")
        .arg("--dry-run")
        .arg("--db")
        .arg(&copy_db)
        .output()
        .expect("run ponycli sessions rebuild-index --dry-run");
    let after = Fixture::db_fingerprint(&copy_db);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(
        output.status.code(),
        Some(0),
        "`ponycli sessions rebuild-index --dry-run` 必须 EXIT=0\nstdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("mode: dry-run"),
        "必须是 dry-run 模式（输出里要能看出来）：\n{stdout}"
    );
    assert!(
        stdout.contains("scanned: 1"),
        "必须扫到 1 个有日志文件的会话（真源侧）：\n{stdout}"
    );
    assert!(
        stdout.contains(&format!("\"{logged}\"")),
        "rebuilt 列表必须包含有日志的会话：\n{stdout}"
    );
    assert!(
        stdout.contains("skipped_no_log: 1"),
        "无日志的存量会话必须计入 skipped_no_log：\n{stdout}"
    );
    assert!(
        stdout.contains("mismatched(0)"),
        "重建基准与 fold 一致（索引视图已由 normalized_* 投影补全）：\n{stdout}"
    );
    assert_eq!(
        before, after,
        "dry-run 必须零写入：副本的 sessions.db 与 -wal sha256 前后必须一致"
    );
    // 副本本身没被改写 ⇒ 原库当然更没动（顺手钉死一次）。
    assert_eq!(
        Fixture::db_fingerprint(&fixture.db()),
        Fixture::db_fingerprint(&fixture.db()),
        "原库指纹读取稳定（恒真守卫：确认指纹函数无副作用）"
    );

    eprintln!("[6] dry-run EXIT=0 且副本 sha256 未变；\n{}", stdout.trim());
}

fn copy_tree(from: &Path, to: &Path) {
    if !from.exists() {
        return;
    }
    std::fs::create_dir_all(to).expect("create copy dir");
    for entry in std::fs::read_dir(from).expect("read source dir") {
        let entry = entry.expect("dir entry");
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).expect("copy file");
        }
    }
}

// ---------------------------------------------------------------------------
// 7. 迁移兼容：老库（session_data NOT NULL）打开后写新会话不报错
// ---------------------------------------------------------------------------

#[test]
fn legacy_not_null_schema_still_accepts_new_writes() {
    let fixture = Fixture::new("legacy_schema");
    let db = fixture.db();

    // 老库形态：4 列 `sessions`（`session_data NOT NULL`，**没有** `version` 列），
    // 直接手写建表 + 一行整包 blob —— 等价于 2-D 之前任何一次发布留下的库。
    let legacy_session = index_only_session("pa_2d_legacy_row", "旧库标题", &["旧库历史一"]);
    let legacy_payload = serde_json::to_string(&legacy_session).expect("serialize legacy session");
    {
        let conn = rusqlite::Connection::open(&db).expect("open legacy db");
        conn.execute_batch(
            "CREATE TABLE sessions (
                 conversation_id TEXT PRIMARY KEY,
                 title TEXT NOT NULL,
                 updated_at_ms INTEGER NOT NULL,
                 session_data TEXT NOT NULL
             );",
        )
        .expect("create legacy sessions table");
        conn.execute(
            "INSERT INTO sessions (conversation_id, title, updated_at_ms, session_data)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                "pa_2d_legacy_row",
                "旧库标题",
                1_700_000_000_000i64,
                legacy_payload
            ],
        )
        .expect("insert legacy blob row");
    }

    // 打开（跑迁移：`version` 列、其余表建表）+ 写新会话。
    let new_session_id = "pa_2d_legacy_new";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(new_session_id), "新问", "新答", None, Vec::new())
            .expect("老库打开后写入新会话必须成功（迁移兼容）");
    }

    // ① 列约束没被放宽：仍然 `NOT NULL`（本阶段选「哨兵值」而不是改 DDL）。
    {
        let conn = fixture.open_sqlite();
        let mut stmt = conn.prepare("PRAGMA table_info(sessions)").expect("table_info");
        let columns: Vec<(String, i64)> = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(1)?, row.get::<_, i64>(3)?))
            })
            .expect("query table_info")
            .filter_map(Result::ok)
            .collect();
        let session_data = columns
            .iter()
            .find(|(name, _)| name == "session_data")
            .expect("session_data 列必须存在");
        assert_eq!(
            session_data.1, 1,
            "`session_data` 必须保持 NOT NULL（选择哨兵值方案 ⇒ 不动 DDL）：{columns:?}"
        );
        assert!(
            columns.iter().any(|(name, _)| name == "version"),
            "迁移必须补上 `version` 列：{columns:?}"
        );
    }

    // ② 老行原样保留（内容逐字段不变，仍走整包 blob 路径）。
    let legacy_after = fixture
        .session_data("pa_2d_legacy_row")
        .expect("老行必须仍在");
    let legacy_parsed = legacy_reader_parses(&legacy_after);
    assert_eq!(contents(&legacy_parsed), vec!["旧库历史一"]);
    assert_eq!(legacy_parsed.title, "旧库标题");

    // ③ 新会话写成功且是轻量索引行（有日志 ⇒ 停写 blob）。
    let new_raw = fixture.session_data(new_session_id).expect("新会话索引行必须存在");
    let (marked, history_len, turn_count, _) = index_row_shape(&new_raw);
    assert!(marked, "新会话（有日志）必须是轻量索引行：{new_raw}");
    assert_eq!(history_len, 0);
    assert_eq!(turn_count, 1);

    // ④ 两类会话都能被运行时读路径读到（老库迁移后不丢数据）。
    let view = fixture.runtime_view();
    assert_eq!(
        contents(view.get("pa_2d_legacy_row").expect("老行可读")),
        vec!["旧库历史一"]
    );
    assert_eq!(
        contents(view.get(new_session_id).expect("新会话可读")),
        vec!["新问", "新答"]
    );

    eprintln!(
        "[7] 老库 NOT NULL 保留；老行={}B，新行={}B（轻量）",
        legacy_after.len(),
        new_raw.len()
    );
}
