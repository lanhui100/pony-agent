//! ADR 0099 **阶段 1-B** 验收：会话日志文件 = **写入真源**，SQLite = 索引。
//!
//! 用户已裁决「直接切写入」（无重要历史数据，不做双写灰度 / 不回填），因此本套用例只
//! 校验一件事：**写路径必须先把命令流落进日志文件（含 fsync），成功后才写 SQLite；
//! 日志写不进去 ⇒ 整批 `Failed`，SQLite 一行都不许改。**
//!
//! 六类用例（task-5 C 节）：
//!
//! 1. `append_turn_writes_header_and_contiguous_command_events` —— 日志存在、header 正确、
//!    事件条数 > 0、`seq` 严格连续、信封形如 `{"seq","ts","cmd"}`。
//! 2. `torn_tail_is_truncated_and_appends_continue_contiguously` —— 外来撕裂尾行污染后
//!    仍能续写（截断到已提交长度），已提交前缀逐字节不动。
//! 3. `remove_session_deletes_the_session_log_directory` —— 删会话后日志目录消失。
//! 4. `eight_concurrent_sessions_do_not_cross_contaminate` —— 8 会话并发 `append_turn`
//!    各自日志互不干扰。
//! 5. `read_only_log_root_makes_persist_fail_without_touching_sqlite` —— 日志根目录只读 ⇒
//!    `Failed` **且 SQLite 未被修改**（反向守门，真源优先）。另附一条平台中立变体
//!    `log_root_being_a_file_...`（Windows 上目录权限不阻止新建文件，故用「根路径是文件」
//!    注入同一种故障）。
//! 6. `command_stream_is_equivalent_to_the_sqlite_index` —— 索引重建前置条件：日志 `cmd`
//!    流可还原出与 `load_store()` 一致的会话（弱断言：消息条数 / seq 一致）。
//!
//! 隔离：所有库与日志都落在 `std::env::temp_dir()` 下的唯一目录，**绝不触碰**
//! `~/.local/share/PonyAgent/`；fixture `Drop` 清理。

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use pony_agent_core::agent::session::{
    current_log_cwd, normalize_cwd_segment, session_log_path_for_db, PersistCommand,
    PersistCommandOutcome, PersistedStore, SeparateTraceTableMode, SessionBackend, SessionState,
    SessionStore, TurnHistoryMessage, SESSION_LOG_FILE_NAME,
};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;

// ---------------------------------------------------------------------------
// 夹具与工具
// ---------------------------------------------------------------------------

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(label: &str) -> Self {
        let unique = format!(
            "pony-0099-1b-wiring-{label}-{}-{}",
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

    fn db_named(&self, name: &str) -> PathBuf {
        self.root.join(format!("{name}.db"))
    }

    /// 日志根目录（与 `SqliteSessionBackend` 的派生口径一致：库文件所在目录 + session-logs）。
    fn log_root(&self) -> PathBuf {
        self.root.join("session-logs")
    }

    fn backend(&self, db: &Path) -> SqliteSessionBackend {
        SqliteSessionBackend::new_with_trace_mode(
            db.to_path_buf(),
            // WriteSeparate ⇒ 新会话进入 TraceTableAuthoritative，走真实生产写入形态。
            SeparateTraceTableMode::WriteSeparate,
        )
    }

    fn store(&self) -> SessionStore {
        SessionStore::with_backend(Box::new(self.backend(&self.db())))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // 用例 5 会把目录改成只读；先尽量恢复可写再清理。
        restore_tree_writable(&self.root);
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn restore_tree_writable(root: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755));
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        stack.push(entry.path());
                    }
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = root;
    }
}

fn log_file(db: &Path, session_id: &str) -> PathBuf {
    session_log_path_for_db(db, session_id)
        .unwrap_or_else(|| panic!("session id `{session_id}` must map to a safe log path"))
}

/// 读日志文件的**所有**行（JSON 对象）。首行是 header，其余是命令信封。
fn read_log_lines(path: &Path) -> Vec<serde_json::Value> {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("read `{}`: {error}", path.display()));
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<serde_json::Value>(line)
                .unwrap_or_else(|error| panic!("log line is not JSON ({error}): {line}"))
        })
        .collect()
}

fn header_of(path: &Path) -> serde_json::Value {
    let lines = read_log_lines(path);
    lines.first().cloned().expect("log file must have a header")
}

/// 命令信封 `(seq, cmd)` 列表（跳过 header）。断言每行都是 `{"seq","ts","cmd"}` 形态。
fn command_events(path: &Path) -> Vec<(u64, serde_json::Value)> {
    let lines = read_log_lines(path);
    let mut events = Vec::new();
    for line in lines.iter().skip(1) {
        let seq = line["seq"]
            .as_u64()
            .unwrap_or_else(|| panic!("envelope must carry integer `seq`: {line}"));
        assert!(
            line["ts"].as_i64().is_some(),
            "envelope must carry integer `ts`: {line}"
        );
        let cmd = line
            .get("cmd")
            .unwrap_or_else(|| panic!("envelope must carry `cmd`: {line}"));
        assert!(
            cmd["type"].as_str().is_some_and(|tag| !tag.is_empty()),
            "`cmd` must carry a variant `type` tag: {line}"
        );
        events.push((seq, cmd.clone()));
    }
    events
}

fn assert_seq_contiguous(events: &[(u64, serde_json::Value)]) {
    let seqs: Vec<u64> = events.iter().map(|(seq, _)| *seq).collect();
    let expected: Vec<u64> = (0..seqs.len() as u64).collect();
    assert_eq!(seqs, expected, "日志 seq 必须从 0 起严格连续（不允许空洞/重复）");
}

/// 「SQLite 是否被修改」的物理凭证：主库 + WAL 的 sha256（含各文件长度）。
///
/// 不把 `-shm` 计入：它是连接级共享内存索引（读标记/锁槽），不是用户数据，
/// 与「SQLite 索引有没有被写入」无关。WAL 必须计入——SQLite 的提交先落 WAL。
fn db_fingerprint(db: &Path) -> String {
    use sha2::{Digest, Sha256};

    assert!(db.exists(), "数据库文件必须已存在才谈得上「未被修改」");
    let mut hasher = Sha256::new();
    let mut targets = vec![db.to_path_buf()];
    let mut wal = db.as_os_str().to_owned();
    wal.push("-wal");
    targets.push(PathBuf::from(wal));
    for path in targets {
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

fn load_store_detached(db: &Path) -> PersistedStore {
    // 只读句柄：不建表、不迁移、不改 journal_mode（维护入口同款打开方式）。
    SqliteSessionBackend::new_read_only(db.to_path_buf())
        .load_store()
        .expect("load_store must succeed")
}

fn history_len(store: &PersistedStore, session_id: &str) -> Option<usize> {
    store
        .sessions_view()
        .get(session_id)
        .map(|session| session.history.len())
}

fn user_message(text: &str) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: "user".to_string(),
        content: text.to_string(),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// 1. 日志存在 / header / 事件条数 / seq 连续 / 信封形态
// ---------------------------------------------------------------------------

#[test]
fn append_turn_writes_header_and_contiguous_command_events() {
    let fixture = Fixture::new("append");
    let session_id = "pa_1b_wiring_append";
    let mut store = fixture.store();
    store
        .append_turn_fallible(Some(session_id), "hello", "world", None, Vec::new())
        .expect("append_turn must persist through the log source of truth");

    let path = log_file(&fixture.db(), session_id);
    assert!(path.exists(), "日志文件必须存在：{}", path.display());
    assert_eq!(path.file_name().unwrap(), SESSION_LOG_FILE_NAME);

    // Windows `LockFileEx` 是**强制**锁：写者句柄仍缓存时，同进程另一个句柄读不了该文件
    // （OS error 33；B044/B050/B052 同源）。读盘前显式放锁，断言内容与强度不变。
    store.release_log_writers_for_test();

    // header 正确（自描述块：type/version/id/cwd/createdAt）。
    let header = header_of(&path);
    assert_eq!(header["type"], serde_json::json!("session"));
    assert_eq!(header["version"], serde_json::json!(1));
    assert_eq!(header["id"], serde_json::json!(session_id));
    // cwd 分组走既有路径解析（进程当前目录 → `--<normalized-cwd>--`）。
    let expected_cwd = normalize_cwd_segment(&current_log_cwd());
    assert_eq!(header["cwd"], serde_json::json!(current_log_cwd()));
    // 用**路径组件**判分组目录名，而不是拿 `/` 拼字符串去 `contains`：Windows 分隔符是 `\\`，
    // 旧写法「/…/」在 windows-latest 上恒不命中（B053：把「分组正确」误判成红）。
    let group_dir_name = path
        .parent()
        .and_then(Path::parent)
        .and_then(|group| group.file_name())
        .map(|name| name.to_string_lossy().into_owned());
    assert_eq!(
        group_dir_name.as_deref(),
        Some(expected_cwd.as_str()),
        "日志路径必须落在 --cwd-- 分组目录下：{}",
        path.display()
    );
    assert!(
        header["createdAt"].as_i64().is_some_and(|ts| ts > 0),
        "header.createdAt 必须是正整数毫秒时间戳：{header}"
    );

    // 事件条数 > 0 且 seq 严格连续。
    let events = command_events(&path);
    assert!(!events.is_empty(), "append_turn 必须至少落一条命令事件");
    assert_seq_contiguous(&events);

    // 首条事件必须是本轮 turn 的消息追加，且归属本会话（信封内 cmd 自描述）。
    let (first_seq, first_cmd) = &events[0];
    assert_eq!(*first_seq, 0);
    assert_eq!(first_cmd["type"], serde_json::json!("append_message"));
    assert_eq!(first_cmd["sessionId"], serde_json::json!(session_id));
    assert_eq!(first_cmd["message"]["role"], serde_json::json!("user"));
    assert_eq!(first_cmd["message"]["content"], serde_json::json!("hello"));

    // 全量事件都属于本会话（不会串会话）。
    for (_, cmd) in &events {
        assert_eq!(cmd["sessionId"], serde_json::json!(session_id));
    }
}

// ---------------------------------------------------------------------------
// 2. 外来撕裂尾行 ⇒ 截断后继续续写
// ---------------------------------------------------------------------------

#[test]
fn torn_tail_is_truncated_and_appends_continue_contiguously() {
    let fixture = Fixture::new("torn");
    let session_id = "pa_1b_wiring_torn";
    let mut store = fixture.store();
    store
        .append_turn_fallible(Some(session_id), "first", "answer-1", None, Vec::new())
        .expect("first append_turn");

    let path = log_file(&fixture.db(), session_id);
    // Windows `LockFileEx` 是**强制**锁：写者句柄仍缓存时，同进程另一个句柄读不了该文件
    // （OS error 33；B044/B050/B052 同源）。读盘前显式放锁，断言内容与强度不变。
    store.release_log_writers_for_test();
    let committed_prefix = std::fs::read(&path).expect("read committed log");
    let committed_events = command_events(&path);
    assert!(!committed_events.is_empty());
    assert_seq_contiguous(&committed_events);

    // 崩溃语义：写者进程先死掉（句柄关闭、锁释放），磁盘上留下撕裂的半行。
    drop(store);
    let mut corrupted = committed_prefix.clone();
    corrupted.extend_from_slice(b"{\"seq\":99,\"ts\":1,\"cmd\":{\"type\":\"append_mess");
    std::fs::write(&path, &corrupted).expect("inject torn tail");

    // 重新打开（= 重启后恢复）：必须截断撕裂尾行并从已提交前缀之后续写。
    let mut store = fixture.store();
    store
        .append_turn_fallible(Some(session_id), "second", "answer-2", None, Vec::new())
        .expect("append_turn after torn tail must keep writing");

    // Windows `LockFileEx` 是**强制**锁：写者句柄仍缓存时，同进程另一个句柄读不了该文件
    // （OS error 33；B044/B050/B052 同源）。读盘前显式放锁，断言内容与强度不变。
    store.release_log_writers_for_test();
    let after = std::fs::read(&path).expect("read log after recovery");
    assert!(
        after.starts_with(&committed_prefix),
        "已提交前缀必须逐字节保留（append-only）"
    );
    assert!(
        !after.ends_with(b"append_mess"),
        "撕裂尾行必须被截断，不能留在文件里"
    );
    assert_eq!(
        after.iter().filter(|byte| **byte == b'\n').count(),
        read_log_lines(&path).len(),
        "截断后每一行都必须以换行结束（行边界完整）"
    );

    let events = command_events(&path);
    assert!(
        events.len() > committed_events.len(),
        "恢复后追加的事件必须真的写进去了"
    );
    assert_seq_contiguous(&events);
    // 两轮 turn 各自的消息都在，且顺序正确。
    let contents: Vec<String> = events
        .iter()
        .filter(|(_, cmd)| cmd["type"] == serde_json::json!("append_message"))
        .filter_map(|(_, cmd)| cmd["message"]["content"].as_str().map(str::to_string))
        .collect();
    assert_eq!(
        contents,
        vec![
            "first".to_string(),
            "answer-1".to_string(),
            "second".to_string(),
            "answer-2".to_string()
        ]
    );
}

// ---------------------------------------------------------------------------
// 3. remove_session 清掉日志目录
// ---------------------------------------------------------------------------

#[test]
fn remove_session_deletes_the_session_log_directory() {
    let fixture = Fixture::new("remove");
    let session_id = "pa_1b_wiring_remove";
    let mut store = fixture.store();
    store
        .append_turn_fallible(Some(session_id), "hi", "bye", None, Vec::new())
        .expect("append_turn");

    let path = log_file(&fixture.db(), session_id);
    let dir = path.parent().expect("log dir").to_path_buf();
    assert!(path.is_file(), "删除前日志文件必须存在");
    assert!(dir.is_dir(), "删除前日志目录必须存在");

    store.remove_session(session_id);

    assert!(!dir.exists(), "remove_session 必须删掉该会话的日志目录");
    assert!(!path.exists(), "remove_session 必须删掉该会话的日志文件");
}

// ---------------------------------------------------------------------------
// 4. 8 会话并发 append_turn，各自日志互不干扰
// ---------------------------------------------------------------------------

#[test]
fn eight_concurrent_sessions_do_not_cross_contaminate() {
    let fixture = Fixture::new("concurrent");
    let mut handles = Vec::new();
    for index in 0..8u32 {
        // 每个线程一个独立库文件（同一 SQLite 库上的第二实例会降级只读 = 阶段 0 E5），
        // 但都落在同一个应用数据目录下 ⇒ 共享同一个 `session-logs/` 根，正好检验
        // 「不同会话各写各的文件」。
        let db = fixture.db_named(&format!("concurrent-{index}"));
        let session_id = format!("pa_1b_wiring_conc_{index}");
        handles.push(std::thread::spawn(move || {
            let mut store = SessionStore::with_backend(Box::new(
                SqliteSessionBackend::new_with_trace_mode(
                    db,
                    SeparateTraceTableMode::WriteSeparate,
                ),
            ));
            store
                .append_turn_fallible(
                    Some(&session_id),
                    &format!("user-{index}"),
                    &format!("assistant-{index}"),
                    None,
                    Vec::new(),
                )
                .expect("concurrent append_turn must persist");
        }));
    }
    for handle in handles {
        handle.join().expect("concurrent writer thread must not panic");
    }

    let mut seen_dirs = HashMap::new();
    for index in 0..8u32 {
        let db = fixture.db_named(&format!("concurrent-{index}"));
        let session_id = format!("pa_1b_wiring_conc_{index}");
        let path = log_file(&db, &session_id);
        assert!(path.is_file(), "会话 {session_id} 的日志必须存在");

        let header = header_of(&path);
        assert_eq!(header["id"], serde_json::json!(session_id));

        let events = command_events(&path);
        assert!(!events.is_empty(), "会话 {session_id} 日志不能是空的");
        assert_seq_contiguous(&events);
        for (_, cmd) in &events {
            assert_eq!(
                cmd["sessionId"],
                serde_json::json!(session_id),
                "会话 {session_id} 的日志里不得出现别的会话命令"
            );
        }
        let contents: Vec<&str> = events
            .iter()
            .filter_map(|(_, cmd)| cmd["message"]["content"].as_str())
            .collect();
        assert!(contents.contains(&format!("user-{index}").as_str()));
        assert!(contents.contains(&format!("assistant-{index}").as_str()));

        let dir = path.parent().expect("log dir").to_path_buf();
        assert!(
            seen_dirs.insert(dir, session_id.clone()).is_none(),
            "两个会话不得共享同一个日志目录（{session_id}）"
        );
    }
    assert_eq!(seen_dirs.len(), 8, "8 个会话必须有 8 个互不相同的日志目录");
}

// ---------------------------------------------------------------------------
// 5. 反向守门：日志根目录不可写 ⇒ Failed 且 SQLite 未被修改
// ---------------------------------------------------------------------------

/// 把 `session-logs/` 下**所有目录**（含根）改成 `r-x`（0o555）。
///
/// 只改根不够：既有分组目录仍可写，新建会话目录会落进它里面。故整棵树一起只读。
#[cfg(unix)]
fn make_tree_read_only(root: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut stack = vec![root.to_path_buf()];
    let mut dirs = Vec::new();
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if entry.path().is_dir() {
                    stack.push(entry.path());
                }
            }
        }
        dirs.push(dir);
    }
    for dir in dirs {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555))
            .unwrap_or_else(|error| panic!("chmod read-only `{}`: {error}", dir.display()));
    }
}

#[cfg(unix)]
#[test]
fn read_only_log_root_makes_persist_fail_without_touching_sqlite() {
    let fixture = Fixture::new("readonly");
    let existing = "pa_1b_wiring_ro_existing";
    let blocked = "pa_1b_wiring_ro_blocked";

    // 先正常写一条会话：库里有内容、日志根目录也真的建起来（这样「只读」才有对象）。
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(existing), "u", "a", None, Vec::new())
            .expect("seed append_turn");
    }
    assert!(
        log_file(&fixture.db(), existing).is_file(),
        "种子会话的日志必须已存在"
    );
    make_tree_read_only(&fixture.log_root());

    // 打开写后端并先让连接初始化完成（建表/迁移/WAL 建档只发生一次），再取指纹。
    let backend = fixture.backend(&fixture.db());
    let _already_loaded = backend.load_store().expect("backend must load");
    let before = db_fingerprint(&fixture.db());

    let outcome = backend.persist_commands_batch(vec![PersistCommand::AppendMessage {
        epoch: 1,
        session_id: blocked.to_string(),
        message: user_message("must-not-be-persisted"),
        ordinal: 0,
    }]);
    assert_eq!(
        outcome,
        PersistCommandOutcome::Failed,
        "日志（真源）写不进去时整批必须 Failed，绝不能静默成功"
    );

    let after = db_fingerprint(&fixture.db());
    assert_eq!(
        before, after,
        "日志写失败时 SQLite 索引必须一个字节都没改（真源优先：失败即整体失败）"
    );
    drop(backend);

    // 内容级旁证：被拒的命令没有变成索引行，既有会话历史也没被改动。
    let persisted = load_store_detached(&fixture.db());
    assert!(
        persisted.sessions_view().get(blocked).is_none(),
        "失败的批次不得在 SQLite 里留下会话行：{blocked}"
    );
    assert_eq!(
        history_len(&persisted, existing),
        Some(2),
        "既有会话的历史必须保持两轮消息，未被失败批次影响"
    );

    // 恢复可写后同一会话必须能写成功（证明上面失败的原因确实是权限，而不是命令本身有问题）。
    restore_tree_writable(&fixture.log_root());
    let backend = fixture.backend(&fixture.db());
    let recovered = backend.persist_commands_batch(vec![PersistCommand::AppendMessage {
        epoch: 1,
        session_id: blocked.to_string(),
        message: user_message("now-it-works"),
        ordinal: 0,
    }]);
    assert_eq!(recovered, PersistCommandOutcome::Succeeded);
    let recovered_path = log_file(&fixture.db(), blocked);
    assert!(
        recovered_path.is_file(),
        "权限恢复后日志文件必须被创建"
    );
    // 真源侧证据：恢复后那条命令确实按信封写进了该会话的日志。
    let recovered_events = command_events(&recovered_path);
    assert_eq!(recovered_events.len(), 1);
    assert_eq!(
        recovered_events[0].1["sessionId"],
        serde_json::json!(blocked)
    );
    assert_eq!(
        recovered_events[0].1["message"]["content"],
        serde_json::json!("now-it-works")
    );
}

/// 平台中立的反向守门：日志根路径**是个文件**（不是目录）⇒ 目录无法创建 ⇒ `Failed`。
///
/// Windows 上「目录只读」并不阻止在其中新建文件，故 Windows 用这条注入同一种故障；
/// Linux/macOS 上它与上面的 chmod 用例互补（一条证权限、一条证结构）。
#[test]
fn log_root_being_a_file_makes_persist_fail_without_touching_sqlite() {
    let fixture = Fixture::new("blocked_root");
    let existing = "pa_1b_wiring_blocked_existing";
    let blocked = "pa_1b_wiring_blocked_new";
    {
        let mut store = fixture.store();
        store
            .append_turn_fallible(Some(existing), "u", "a", None, Vec::new())
            .expect("seed append_turn");
    }
    // 把整个日志根目录换成一个普通文件：任何 `create_dir_all` 都会失败。
    std::fs::remove_dir_all(fixture.log_root()).expect("remove log root");
    std::fs::write(fixture.log_root(), b"not a directory").expect("occupy log root with a file");

    let backend = fixture.backend(&fixture.db());
    let _already_loaded = backend.load_store().expect("backend must load");
    let before = db_fingerprint(&fixture.db());
    let outcome = backend.persist_commands_batch(vec![PersistCommand::AppendMessage {
        epoch: 1,
        session_id: blocked.to_string(),
        message: user_message("must-not-be-persisted"),
        ordinal: 0,
    }]);
    let after = db_fingerprint(&fixture.db());
    assert_eq!(outcome, PersistCommandOutcome::Failed);
    assert_eq!(
        before, after,
        "日志根不可用时 SQLite 索引必须未被修改"
    );
    drop(backend);

    let persisted = load_store_detached(&fixture.db());
    assert!(persisted.sessions_view().get(blocked).is_none());
    assert_eq!(history_len(&persisted, existing), Some(2));
}

// ---------------------------------------------------------------------------
// 6. 索引重建前置条件：cmd 流 ↔ load_store() 等价（弱断言）
// ---------------------------------------------------------------------------

#[test]
fn command_stream_is_equivalent_to_the_sqlite_index() {
    let fixture = Fixture::new("rebuild");
    let session_id = "pa_1b_wiring_rebuild";
    let mut store = fixture.store();
    store
        .append_turn_fallible(Some(session_id), "u1", "a1", None, Vec::new())
        .expect("first append_turn");
    store
        .append_turn_fallible(Some(session_id), "u2", "a2", None, Vec::new())
        .expect("second append_turn");
    drop(store);

    let path = log_file(&fixture.db(), session_id);
    let events = command_events(&path);
    assert_seq_contiguous(&events);

    // 日志侧折叠（弱断言：只数消息追加命令，不重建完整快照——fold 投影属后续阶段）。
    let appended_messages: Vec<String> = events
        .iter()
        .filter(|(_, cmd)| cmd["type"] == serde_json::json!("append_message"))
        .map(|(_, cmd)| {
            cmd["message"]["content"]
                .as_str()
                .unwrap_or_default()
                .to_string()
        })
        .collect();
    assert_eq!(
        appended_messages,
        vec![
            "u1".to_string(),
            "a1".to_string(),
            "u2".to_string(),
            "a2".to_string()
        ],
        "日志命令流必须按序保存两轮 turn 的四条消息"
    );

    // 索引侧：SQLite 里该会话的历史条数必须与日志命令流一致。
    let persisted = load_store_detached(&fixture.db());
    let session: &SessionState = persisted
        .sessions_view()
        .get(session_id)
        .unwrap_or_else(|| panic!("SQLite 索引里必须有会话 {session_id}"));
    assert_eq!(
        session.history.len(),
        appended_messages.len(),
        "日志 cmd 流与 SQLite 索引的消息条数必须一致（索引可重建的前置条件）"
    );
    let index_contents: Vec<String> = session
        .history
        .iter()
        .map(|message| message.content.clone())
        .collect();
    assert_eq!(index_contents, appended_messages, "两边消息序列必须一致");
    assert!(
        session.history.iter().all(|message| !message.content.is_empty()),
        "索引里的消息不得为空（日志侧同样不为空）"
    );
}
