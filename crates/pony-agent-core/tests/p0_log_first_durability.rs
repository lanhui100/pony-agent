//! ADR 0107 §5 P0 验收（**红相冻结**）：日志先行（log-first durability）+ 失败域隔离。
//!
//! 契约真源：`.dev-team/contract-p0-0107.md`（C1–C4）。四个用例名**逐字固定**，不可改写：
//!
//! - C1 `c1_read_only_backend_still_writes_session_log`：只读降级后端（`is_read_only()`
//!   为真，模拟抢不到 `sessions.db.lock`）提交含 `AppendMessage` 的命令批 ⇒ 该会话
//!   `session.jsonl` 必须出现对应行，SQLite 不得新增行（不变量 1：日志先行）；
//! - C2 `c2_index_failure_still_advances_memory_history`：日志成功、索引失败 ⇒
//!   `SessionStore::append_turn_fallible` 返回 `Ok` 且内存 `history` 追加 user+assistant
//!   （不变量 2：失败域隔离，索引滞后不得吞掉本轮历史）；
//! - C3 `c3_log_failure_keeps_memory_untouched`：日志失败 ⇒ 返回 `Err` 且内存不变
//!   （不变量 3：日志失败仍 fail-loud；防 C2 修过头）；
//! - C4 `c4_writable_path_unchanged`：可写后端（日志+索引均成功）⇒ `Ok`、历史与索引行
//!   都正确（不变量 4：可写实例零回归）。
//!
//! ## 注入方式（不 mock 文件系统、不改 `src/`）
//!
//! | 用例 | 注入点 | 手段 |
//! |---|---|---|
//! | C1 | 真实 `SqliteSessionBackend::new_read_only` 句柄 | 单实例锁路径被占成目录 ⇒ `try_recover_instance_lock` 无法把只读实例"复活"成可写，`is_read_only()` 稳定为真 |
//! | C2 | 真实 SQLite 索引 | `normalized_messages` 上的 `BEFORE INSERT` 触发器 `RAISE(ABORT)` ⇒ 只有索引提交失败，日志 sink 不受影响 |
//! | C3 | 真实日志 sink | 日志根目录被占成**普通文件** ⇒ `create_dir_all` 必然失败 ⇒ 日志写必然失败 |
//! | C4 | 无注入（正常路径对照） | — |
//!
//! 断言全部是**行为级**的（日志文件行内容 / 内存 history 角色序列 / 索引行内容），
//! 不使用「跑通即通过」式空断言；失败信息带「期望 vs 实际」。
//!
//! 隔离：库文件、日志目录都落在 `std::env::temp_dir()` 下的唯一目录，`Drop` 清理，
//! **绝不触碰** `~/.local/share/PonyAgent/`。

use std::path::{Path, PathBuf};

use pony_agent_core::agent::session::{
    session_log_path_for_db, session_log_root_for_db, PersistCommand, PersistCommandOutcome,
    SessionBackend, SessionSnapshot, SessionStore, TurnHistoryMessage,
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
            "pony-p0-0107-{label}-{}-{}",
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

    /// 该会话的 JSONL 真源路径（`<db 目录>/session-logs/--<cwd>--/<sid>/session.jsonl`）。
    fn log_path(&self, session_id: &str) -> PathBuf {
        session_log_path_for_db(&self.db(), session_id).unwrap_or_else(|| {
            panic!("session id `{session_id}` 必须映射到安全的日志路径（不得为 None）")
        })
    }

    /// 单实例锁文件路径（`<db>.lock`，见 `sqlite_session_commit::acquire_single_instance_lock`）。
    fn lock_path(&self) -> PathBuf {
        PathBuf::from(format!("{}.lock", self.db().display()))
    }

    fn backend(&self) -> SqliteSessionBackend {
        SqliteSessionBackend::new(self.db())
    }

    fn store(&self) -> SessionStore {
        SessionStore::with_backend(Box::new(self.backend()))
    }

    fn open_sqlite(&self) -> rusqlite::Connection {
        rusqlite::Connection::open(self.db()).expect("open sqlite index")
    }

    /// 索引里的消息行（`role`, `content`），按 `ordinal` 升序。
    fn index_messages(&self, session_id: &str) -> Vec<(String, String)> {
        let conn = self.open_sqlite();
        let mut statement = conn
            .prepare(
                "SELECT role, content FROM normalized_messages \
                 WHERE session_id = ?1 ORDER BY ordinal ASC, message_id ASC",
            )
            .unwrap_or_else(|error| panic!("prepare normalized_messages 查询：{error}"));
        let rows = statement
            .query_map(rusqlite::params![session_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .unwrap_or_else(|error| panic!("query normalized_messages：{error}"));
        rows.map(|row| row.unwrap_or_else(|error| panic!("decode 索引行：{error}")))
            .collect()
    }

    /// 索引 `sessions` 行数（C1 的「SQLite 不得新增行」判据）。
    fn index_session_rows(&self, session_id: &str) -> i64 {
        let conn = self.open_sqlite();
        conn.query_row(
            "SELECT COUNT(*) FROM sessions WHERE conversation_id = ?1",
            rusqlite::params![session_id],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or_else(|error| panic!("count sessions 行：{error}"))
    }

    /// C2 注入：让 `normalized_messages` 的任何写入在**语句级**失败（`SQLITE_CONSTRAINT`，
    /// 非 busy ⇒ 不重试、不静默），从而构造「日志成功 + 索引失败」的真实组合。
    fn inject_index_write_failure(&self) {
        let conn = self.open_sqlite();
        conn.execute_batch(
            "CREATE TRIGGER p0_0107_inject_index_failure \
             BEFORE INSERT ON normalized_messages \
             BEGIN SELECT RAISE(ABORT, 'p0-0107 injected index write failure'); END;",
        )
        .unwrap_or_else(|error| panic!("注入索引写失败触发器：{error}"));
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// C1/C6：用 **std 文件锁**（不是 backend 的进程内 `LOCK_REGISTRY`）占住 `<db>.lock`，
/// 等价于「另一个进程持有单实例锁」⇒ 新建 backend 首次 `connection()` 抢锁必失败、降级只读。
///
/// 为什么不能用 `new_read_only` 当降级实例：那是**维护句柄**（`storage_read_only=true`，
/// `--dry-run`/诊断），按 R4/E8 必须零写入、连日志都不写；锁降级实例
/// （`storage_read_only=false`）才必须写日志（日志先行）。drop 即解锁。
struct LockHolder(std::fs::File);

impl LockHolder {
    fn acquire(lock_path: &Path) -> Self {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path)
            .unwrap_or_else(|error| {
                panic!("打开单实例锁文件 `{}` 失败：{error}", lock_path.display())
            });
        match file.try_lock() {
            Ok(()) => Self(file),
            Err(error) => panic!(
                "占住单实例锁 `{}` 失败（前置不成立）：{error:?}",
                lock_path.display()
            ),
        }
    }
}

impl Drop for LockHolder {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

// ---------------------------------------------------------------------------
// 断言辅助（行为级读法）
// ---------------------------------------------------------------------------

fn message(role: &str, content: &str) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: role.to_string(),
        content: content.to_string(),
        ..Default::default()
    }
}

/// 会话 JSONL 中 `append_message` 的 `(role, content)` 序列（跳过文件头行）。
///
/// 日志行格式（`log_store`）：文件头 `{"kind":"session",…}` + 每命令一行
/// `{"seq":N,"ts":ms,"cmd":{"type":"append_message",…}}`。
fn log_append_messages(path: &Path) -> Vec<(String, String)> {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("读会话日志 `{}` 失败：{error}", path.display()));
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .filter_map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|error| {
                panic!(
                    "会话日志 `{}` 的行不是合法 JSON：{error}；原文：{line}",
                    path.display()
                )
            });
            let command = value.get("cmd")?;
            if command.get("type")?.as_str()? != "append_message" {
                return None;
            }
            let message = command.get("message")?;
            Some((
                message.get("role")?.as_str()?.to_string(),
                message.get("content")?.as_str()?.to_string(),
            ))
        })
        .collect()
}

fn pairs(pairs: &[(String, String)]) -> Vec<(String, String)> {
    pairs.to_vec()
}

/// 内存快照里的 `(role, content)` 序列——`snapshot_at_readonly` 只读内存状态，
/// 不创建会话、不触发落盘，故它是「内存到底改了没有」的诚实观察口。
fn memory_history(store: &SessionStore, session_id: &str) -> Vec<(String, String)> {
    let snapshot: SessionSnapshot = store.snapshot_at_readonly(Some(session_id), None, &[]);
    snapshot
        .history
        .iter()
        .map(|entry| (entry.role.clone(), entry.content.clone()))
        .collect()
}

fn expected_turn(user: &str, assistant: &str) -> Vec<(String, String)> {
    vec![
        ("user".to_string(), user.to_string()),
        ("assistant".to_string(), assistant.to_string()),
    ]
}

// ---------------------------------------------------------------------------
// C1：只读降级后端仍必须写会话日志（日志先行）
// ---------------------------------------------------------------------------

#[test]
fn c1_read_only_backend_still_writes_session_log() {
    let fixture = Fixture::new("c1");
    let session_id = "p0_c1_read_only";
    let user = "c1 只读实例也必须落日志";
    let assistant = "c1 索引写不了不等于历史不存在";

    // —— Arrange ——
    // 1) 先用正常可写句柄把库建出来，并让建库实例释放单实例锁（drop ⇒ 登记项失效）。
    {
        let writable = fixture.backend();
        let _ = writable.load_store();
    }
    // 2) 测试自己用 std 文件锁**排他占住** `<db>.lock`（模拟「另一个进程已持有单实例锁」；
    //    不走 backend 的进程内登记表，否则会被同进程共享锁逻辑复用掉、根本抢不到）。
    //    随后用**正常构造器** `new()`（`storage_read_only=false`）建实例：
    //    首次 `connection()` 抢锁失败 ⇒ 运行时**锁降级只读**（必须写日志）。
    let lock_holder = LockHolder::acquire(&fixture.lock_path());
    let read_only = fixture.backend();
    assert!(
        read_only.load_store().is_some(),
        "前置：只读句柄必须能读到已存在的库（否则本用例证明不了「只读仍写日志」）"
    );
    assert!(
        read_only.is_read_only(),
        "前置：抢不到 `<db>.lock` 时必须降级只读（`is_read_only()==true`）"
    );
    assert!(
        !read_only.storage_read_only(),
        "前置：锁降级实例 ≠ 维护句柄（`storage_read_only()` 必须为 false，否则日志被 E8 短路）"
    );

    let commands = vec![
        PersistCommand::AppendMessage {
            epoch: 1,
            session_id: session_id.to_string(),
            message: message("user", user),
            ordinal: 0,
        },
        PersistCommand::AppendMessage {
            epoch: 1,
            session_id: session_id.to_string(),
            message: message("assistant", assistant),
            ordinal: 1,
        },
    ];

    // —— Act ——
    let outcome = read_only.persist_commands_batch(commands);

    // —— Assert ——
    // (1) 不变量 1：日志是真源——只读实例也必须把它写下来。
    let log_path = fixture.log_path(session_id);
    assert!(
        log_path.is_file(),
        "日志先行被击穿：只读降级后端提交含 AppendMessage 的命令批后，会话 JSONL \
         必须存在，实际 `{}` 不存在（`append_session_log_to_file` 在只读时提前 return）",
        log_path.display()
    );
    let logged = log_append_messages(&log_path);
    assert_eq!(
        pairs(&logged),
        expected_turn(user, assistant),
        "日志内容不符：期望 JSONL 出现 {:?}，实际 {:?}（路径 {}）",
        expected_turn(user, assistant),
        logged,
        log_path.display()
    );

    // (2) 索引不得新增行（只读实例本来就不可能提交 SQLite）。
    let indexed = fixture.index_messages(session_id);
    assert!(
        indexed.is_empty(),
        "只读实例不得写索引：期望 `normalized_messages` 零行，实际 {indexed:?}"
    );
    assert_eq!(
        fixture.index_session_rows(session_id),
        0,
        "只读实例不得写索引：期望 `sessions` 表零行，实际有行"
    );

    // (3) 索引没写成功，批次就不得自称成功（失败必须可观测）。
    assert!(
        !matches!(outcome, PersistCommandOutcome::Succeeded),
        "只读实例的索引写必然失败，批次不得返回 Succeeded，实际 outcome={outcome:?}"
    );
    drop(lock_holder);
}

// ---------------------------------------------------------------------------
// C2：索引失败不得吞掉内存历史（失败域隔离）
// ---------------------------------------------------------------------------

#[test]
fn c2_index_failure_still_advances_memory_history() {
    let fixture = Fixture::new("c2");
    let session_id = "p0_c2_index_fail";
    let user = "c2 日志已落盘用户消息";
    let assistant = "c2 索引失败助手消息";

    // —— Arrange ——
    let mut store = fixture.store();
    // 注入：只让索引侧（normalized_messages）写失败，日志 sink 照常成功。
    fixture.inject_index_write_failure();

    // —— Act ——
    let result =
        store.append_turn_fallible(Some(session_id), user, assistant, None, Vec::new());

    // —— Assert ——
    // 前提 1：日志确实成功了（否则这组「日志成功 + 索引失败」的组合没被构造出来）。
    let log_path = fixture.log_path(session_id);
    let logged = log_append_messages(&log_path);
    assert_eq!(
        pairs(&logged),
        expected_turn(user, assistant),
        "前提（日志成功）不成立：期望 JSONL 出现 {:?}，实际 {:?}（路径 {}）",
        expected_turn(user, assistant),
        logged,
        log_path.display()
    );

    // 前提 2：索引确实失败了（注入有效 ⇒ 索引滞后而非索引已写）。
    assert!(
        fixture.index_messages(session_id).is_empty(),
        "前提（索引失败）不成立：注入后索引不该有消息行，实际 {:?}",
        fixture.index_messages(session_id)
    );

    // 契约 C2：索引失败不得让本轮失败。
    let snapshot = result.unwrap_or_else(|error| {
        panic!(
            "失败域隔离被击穿：日志已成功、索引失败时 `append_turn_fallible` 必须返回 Ok，\
             实际返回 Err({error})（期望 Ok + 内存 history 追加 user+assistant）"
        )
    });

    // 契约 C2：内存历史必须前进一步（返回快照 + 内存本体双读，防止只在返回值里"假装"前进）。
    let expected = expected_turn(user, assistant);
    let returned: Vec<(String, String)> = snapshot
        .history
        .iter()
        .map(|entry| (entry.role.clone(), entry.content.clone()))
        .collect();
    assert_eq!(
        pairs(&returned),
        expected,
        "索引失败后返回快照的历史不符：期望 {:?}，实际 {:?}",
        expected,
        returned
    );
    let live = memory_history(&store, session_id);
    assert_eq!(
        pairs(&live),
        expected,
        "索引失败后内存 `session.history` 必须追加 user+assistant：期望 {:?}，实际 {:?}",
        expected,
        live
    );
}

// ---------------------------------------------------------------------------
// C3：日志失败仍 fail-loud 且内存不变（回归保护）
// ---------------------------------------------------------------------------

#[test]
fn c3_log_failure_keeps_memory_untouched() {
    let fixture = Fixture::new("c3");
    let session_id = "p0_c3_log_fail";
    let user = "c3 日志写不进去";
    let assistant = "c3 本轮必须整体失败";

    // —— Arrange ——
    // 让日志写入必然失败：日志根目录被普通文件占住 ⇒ `create_dir_all(session_dir)`
    // 返回 ENOTDIR ⇒ sink 必然 `Err`（真实文件系统错误，不 mock）。
    std::fs::write(fixture.log_root(), b"not-a-directory")
        .expect("用普通文件占住日志根目录（模拟日志不可写）");
    let mut store = fixture.store();

    // —— Act ——
    let result =
        store.append_turn_fallible(Some(session_id), user, assistant, None, Vec::new());

    // —— Assert ——
    // (1) 日志失败必须 fail loud。
    let error = result.err().unwrap_or_else(|| {
        panic!(
            "日志失败被静默吞掉：日志根目录不可写时 `append_turn_fallible` 必须返回 Err，实际 Ok"
        )
    });

    // (2) 内存不得前进（否则「日志失败」变成了「历史只在内存里」的假成功）。
    let live = memory_history(&store, session_id);
    assert!(
        live.is_empty(),
        "日志失败后内存 `session.history` 必须保持不变（期望空），实际 {live:?}；错误：{error}"
    );
    let snapshot = store.snapshot_at_readonly(Some(session_id), None, &[]);
    assert_eq!(
        snapshot.turn_count, 0,
        "日志失败后 `turn_count` 不得增长（期望 0，实际 {}）；错误：{error}",
        snapshot.turn_count
    );

    // (3) 索引同样一行不写（真源与索引必须一起不动，无「部分成功」）。
    let indexed = fixture.index_messages(session_id);
    assert!(
        indexed.is_empty(),
        "日志失败时 SQLite 索引不得改动：期望 `normalized_messages` 零行，实际 {indexed:?}"
    );

    // (4) 日志文件确实没被写出来。
    let log_path = fixture.log_path(session_id);
    assert!(
        !log_path.is_file(),
        "日志写入报错后不得留下日志文件，实际 `{}` 存在",
        log_path.display()
    );
}

// ---------------------------------------------------------------------------
// C4：可写路径零回归（日志 + 索引都成功）
// ---------------------------------------------------------------------------

#[test]
fn c4_writable_path_unchanged() {
    let fixture = Fixture::new("c4");
    let session_id = "p0_c4_writable";
    let user = "c4 正常用户消息";
    let assistant = "c4 正常助手消息";

    // —— Arrange ——
    let mut store = fixture.store();

    // —— Act ——
    let result =
        store.append_turn_fallible(Some(session_id), user, assistant, None, Vec::new());

    // —— Assert ——
    // (1) 返回值：可写路径必须 Ok。
    let snapshot = result.unwrap_or_else(|error| {
        panic!("零回归被破坏：可写路径 `append_turn_fallible` 必须返回 Ok，实际 Err({error})")
    });

    // (2) 内存历史正确。
    let expected = expected_turn(user, assistant);
    let live = memory_history(&store, session_id);
    assert_eq!(
        pairs(&live),
        expected,
        "可写路径内存历史不符：期望 {:?}，实际 {:?}",
        expected,
        live
    );
    let returned: Vec<(String, String)> = snapshot
        .history
        .iter()
        .map(|entry| (entry.role.clone(), entry.content.clone()))
        .collect();
    assert_eq!(
        pairs(&returned),
        expected,
        "可写路径返回快照的历史不符：期望 {:?}，实际 {:?}",
        expected,
        returned
    );

    // (3) 索引行正确（内容 + 角色顺序）。
    let indexed = fixture.index_messages(session_id);
    assert_eq!(
        pairs(&indexed),
        expected,
        "可写路径索引行不符：期望 {:?}，实际 {:?}",
        expected,
        indexed
    );
    assert_eq!(
        fixture.index_session_rows(session_id),
        1,
        "可写路径 `sessions` 表必须恰好 1 行（会话索引行），实际 {}",
        fixture.index_session_rows(session_id)
    );

    // (4) 真源日志正确（与今日语义一致：日志先写，索引随后）。
    let log_path = fixture.log_path(session_id);
    assert!(
        log_path.is_file(),
        "可写路径必须产出会话日志，实际 `{}` 不存在",
        log_path.display()
    );
    let logged = log_append_messages(&log_path);
    assert_eq!(
        pairs(&logged),
        expected,
        "可写路径日志内容不符：期望 {:?}，实际 {:?}",
        expected,
        logged
    );
}

// ---------------------------------------------------------------------------
// C5：维护句柄（`new_read_only` / `--dry-run` / 诊断）必须保持 E8 零写入（R4 + R5）
// ---------------------------------------------------------------------------

#[test]
fn c5_maintenance_handle_stays_zero_write() {
    let fixture = Fixture::new("c5");
    let session_id = "p0_c5_maintenance";
    let user = "c5 维护句柄不得写任何字节";

    // —— Arrange ——：先建出库（维护句柄要求库文件已存在）。
    {
        let writable = fixture.backend();
        let _ = writable.load_store();
    }
    let maintenance = SqliteSessionBackend::new_read_only(fixture.db());
    assert!(
        maintenance.storage_read_only(),
        "前置：`new_read_only` 必须是存储级只读（维护句柄，`storage_read_only()==true`）"
    );
    assert!(maintenance.load_store().is_some(), "前置：维护句柄必须能读库");

    // —— Act ——：维护句柄收到一批**写入类**命令。
    let outcome = maintenance.persist_commands_batch(vec![PersistCommand::AppendMessage {
        epoch: 1,
        session_id: session_id.to_string(),
        message: message("user", user),
        ordinal: 0,
    }]);

    // —— Assert ——
    // (1) R4 / E8：维护句柄零写入——连日志目录都不许建出来。
    let log_root = fixture.log_root();
    assert!(
        !log_root.exists(),
        "E8 零写入被击穿：维护句柄不得创建日志根目录，实际 `{}` 存在",
        log_root.display()
    );
    let log_path = fixture.log_path(session_id);
    assert!(
        !log_path.is_file(),
        "E8 零写入被击穿：维护句柄不得产出 session.jsonl，实际 `{}` 存在",
        log_path.display()
    );
    assert!(
        fixture.index_messages(session_id).is_empty(),
        "维护句柄（`mode=ro`）不得写索引，实际有行"
    );

    // (2) R5：日志层零写入 ⇒ 索引失败必须判 `Failed`，不得自称「日志已落盘」。
    assert_eq!(
        outcome,
        PersistCommandOutcome::Failed,
        "日志零写入时索引失败必须是 Failed（`LogPersisted` 的前提是日志真写入了字节）"
    );
}

// ---------------------------------------------------------------------------
// C6：锁降级实例的日志 writer 必须**按批释放** flock（R3）
// ---------------------------------------------------------------------------

#[test]
fn c6_degraded_instance_releases_session_log_lock_after_batch() {
    let fixture = Fixture::new("c6");
    let session_id = "p0_c6_lock_handoff";
    let degraded_user = "c6 降级实例写入";
    let main_user = "c6 主实例用户消息";
    let main_assistant = "c6 主实例助手消息";

    // —— Arrange ——
    { let writable = fixture.backend(); let _ = writable.load_store(); }
    // 占住单实例锁 ⇒ 第二个实例是**锁降级只读**（`storage_read_only=false`）。
    let lock_holder = LockHolder::acquire(&fixture.lock_path());
    let degraded = fixture.backend();
    let _ = degraded.load_store();
    assert!(degraded.is_read_only(), "前置：第二个实例必须锁降级只读");

    // —— Act 1 ——：降级实例写一批（日志成功、索引失败）。
    let outcome = degraded.persist_commands_batch(vec![PersistCommand::AppendMessage {
        epoch: 1,
        session_id: session_id.to_string(),
        message: message("user", degraded_user),
        ordinal: 0,
    }]);
    assert_eq!(
        outcome,
        PersistCommandOutcome::LogPersisted,
        "日志已落盘、索引失败 ⇒ 必须是 LogPersisted，实际 {outcome:?}"
    );
    assert_eq!(
        pairs(&log_append_messages(&fixture.log_path(session_id))),
        vec![("user".to_string(), degraded_user.to_string())],
        "降级实例必须把命令写进真源日志"
    );

    // —— Act 2 ——：主（可写）实例接手同一会话。降级实例若仍缓存 writer 持 flock，
    // 这里会被 `Locked` 挡成整批失败（R3 的回归点）。
    drop(lock_holder);
    let mut writable_store = SessionStore::with_backend(Box::new(fixture.backend()));
    let result = writable_store.append_turn_fallible(
        Some(session_id),
        main_user,
        main_assistant,
        None,
        Vec::new(),
    );

    // —— Assert ——
    assert!(
        result.is_ok(),
        "R3 被破坏：降级实例必须按批释放 session log flock，否则主可写实例同会话 append 被 Locked 挡住：{result:?}"
    );
    assert_eq!(
        pairs(&log_append_messages(&fixture.log_path(session_id))),
        vec![
            ("user".to_string(), degraded_user.to_string()),
            ("user".to_string(), main_user.to_string()),
            ("assistant".to_string(), main_assistant.to_string()),
        ],
        "主实例的两条消息必须追加进同一份真源日志"
    );
}

// ---------------------------------------------------------------------------
// C7：零字节写入批（`RemoveSession`-only）不得自称「日志已落盘」（R5）
// ---------------------------------------------------------------------------

#[test]
fn c7_remove_session_only_batch_never_claims_log_persisted() {
    let fixture = Fixture::new("c7");
    let session_id = "p0_c7_remove_only";

    // —— Arrange ——：锁降级实例（索引必失败）+ 一批**完全无日志字节**的命令。
    // `RemoveSession` 按既有语义不进日志（删除由 `delete_session` 在索引提交后承担）
    // ⇒ 日志层 `grouped.is_empty()` ⇒ 零写入。
    { let writable = fixture.backend(); let _ = writable.load_store(); }
    let lock_holder = LockHolder::acquire(&fixture.lock_path());
    let degraded = fixture.backend();
    let _ = degraded.load_store();
    assert!(degraded.is_read_only(), "前置：必须锁降级只读（索引必失败）");

    // —— Act ——
    let outcome = degraded.persist_commands_batch(vec![PersistCommand::RemoveSession {
        epoch: 1,
        session_id: session_id.to_string(),
    }]);

    // —— Assert ——
    assert_ne!(
        outcome,
        PersistCommandOutcome::LogPersisted,
        "R5 被击穿：日志层零字节写入的批不得判 `LogPersisted`（该变体的前提是日志真写入了字节），实际 {outcome:?}"
    );
    assert!(
        !fixture.log_path(session_id).is_file(),
        "`RemoveSession` 不进日志：不得产出 session.jsonl"
    );
    drop(lock_holder);
}
