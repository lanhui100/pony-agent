//! ADR 0107 §5 P1 验收：多进程写索引 / 维护锁 / 会话级单写者（D1–D5）。
//!
//! ## 为什么必须真跨进程（禁止同进程假并发）
//!
//! 基线的库级单实例锁走**进程内登记表**（`LOCK_REGISTRY`，`Weak<File>` 复用）：同一进程里
//! 开两个 `SqliteSessionBackend`，第二次 `open` 会复用同一个 guard，**不会**降级只读。
//! 同一进程内对同一文件再次 `flock` 也常常因为共用打开文件描述而"成功"。用同进程建模
//! "两个 pony 进程"，得到的是假绿——真实缺陷（第二个进程整库只读）根本不会出现。
//!
//! 因此本文件用 **self-exec helper**：
//!   1. 父进程 `Command::new(std::env::current_exe())` 重跑**同一个测试二进制**；
//!   2. 参数 `--exact <helper> --ignored --nocapture --test-threads=1`，只跑指定的
//!      `#[ignore]` helper；
//!   3. 参数经环境变量传递（`P1_HELPER_ROLE` 是"我是 helper"的守卫，其余见下方常量）；
//!   4. 持锁类 helper 拿到锁后向 stdout 打印 `LOCK_HELD`，父进程**读到该标记**才继续，
//!      不靠固定 sleep 赌时序；父进程用退出码 / stdout / stderr 断言。
//!
//! 子进程继承父进程的 cwd，因此会话日志的 cwd 分组与父进程一致（`SessionLogStore` 按
//! cwd 分组存放 `session.jsonl`），避免跨分组找不到日志。
//!
//! ## 锁文件名的两套口径
//!
//! 契约（`contract-p1-0107.md`）写的是 `<db>.maintenance.lock`；当前实现
//! （`sqlite_session_commit.rs::acquire_maintenance_lock`）复用 `<db>.lock`。
//! 测试不赌实现细节：helper 把**两个路径都锁住**，无论实现取哪一个都必须撞锁。
//! 反过来说，若日常写路径偷偷去抢其中任何一把，D1/D3/D5 都会红。
//!
//! ## 基线（`e043f32f`）预期
//!
//! - D1 / D3 / D4 / D5 红：第二个进程抢不到 `<db>.lock` ⇒ `read_only=true` ⇒
//!   `PRAGMA query_only=ON` ⇒ 整库只读（写入失败、索引零行），且 stderr 出现
//!   `single_instance_lock_denied`。
//! - D2 红在"信号"上：基线靠同一把锁顺带挡住维护，但给不出 `maintenance_lock_busy`
//!   这一可 grep 的显式语义（维护 vs 日常写未区分）。

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStderr, ChildStdout, Command, Output, Stdio};
use std::time::Duration;

use pony_agent_core::agent::session::{rebuild_index, RebuildOptions, SessionStore};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;

/// "我是 helper 子进程"守卫：只有被父进程重跑时才设置。
const HELPER_ENV: &str = "P1_HELPER_ROLE";
const ENV_DB: &str = "P1_DB";
const ENV_SESSION: &str = "P1_SESSION";
const ENV_USER: &str = "P1_USER";
const ENV_HOLD_MS: &str = "P1_HOLD_MS";

const HELPER_WRITE: &str = "p1_helper_write_turn";
const HELPER_HOLD: &str = "p1_helper_hold_maintenance_lock";

const MARK_LOCK_HELD: &str = "LOCK_HELD";
const MARK_LOCK_FAIL: &str = "LOCK_FAIL";
const MARK_WRITE_OK: &str = "WRITE_OK";
const MARK_WRITE_ERR: &str = "WRITE_ERR";

/// 基线"整库降级只读"的日志指纹：P1 之后日常写进程不得再打印它。
const DEGRADED_MARKER: &str = "single_instance_lock_denied";

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

/// 临时目录（Drop 清理；子进程仍在写时删除失败就忽略，不影响断言）。
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let unique = format!(
            "pony-p1-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn db_path(dir: &Path) -> PathBuf {
    dir.join("sessions.db")
}

/// 库级锁的两套候选路径：契约口径 `<db>.maintenance.lock` + 当前实现口径 `<db>.lock`。
fn maintenance_lock_paths(db: &Path) -> [PathBuf; 2] {
    let text = db.display().to_string();
    [
        PathBuf::from(format!("{text}.lock")),
        PathBuf::from(format!("{text}.maintenance.lock")),
    ]
}

fn open_instance(dir: &Path) -> SessionStore {
    let backend = SqliteSessionBackend::new(db_path(dir));
    SessionStore::with_backend(Box::new(backend))
}

/// 直接查索引表：该会话的 user 消息里是否包含指定文本（不合并日志，只看索引真身）。
///
/// 判据必须落在**索引**上：P0 之后被降级的实例仍能写会话日志（`LogPersisted`），而
/// `snapshot_at_readonly` 会合并日志真值 ⇒ 只看历史会假绿。
fn indexed_message_count(dir: &Path, session_id: &str, needle: &str) -> i64 {
    let conn = rusqlite::Connection::open(db_path(dir)).expect("open db for index assert");
    conn.query_row(
        "SELECT COUNT(*) FROM normalized_messages WHERE session_id = ?1 AND role = 'user' AND content LIKE '%' || ?2 || '%'",
        rusqlite::params![session_id, needle],
        |row| row.get::<_, i64>(0),
    )
    .unwrap_or(0)
}

fn write_turn_here(store: &mut SessionStore, session_id: &str, user: &str, assistant: &str) {
    store
        .append_turn_fallible(Some(session_id), user, assistant, None, Vec::new())
        .unwrap_or_else(|error| panic!("append_turn_fallible({session_id}) failed: {error}"));
}

fn helper_env(key: &str) -> Option<String> {
    std::env::var(key).ok()
}

/// 起一个真实子进程（同一测试二进制）跑 helper，返回完整输出。
///
/// 附带的防假绿断言：libtest 若因名字不匹配把用例过滤掉，会打印 `0 filtered out` 并以
/// 退出码 0 结束——那种情况下任何 stdout 断言都会失真，所以这里先确认真的跑了 1 个用例。
fn spawn_helper(helper: &str, dir: &Path, extra: &[(&str, &str)]) -> Output {
    let mut command = Command::new(std::env::current_exe().expect("current exe"));
    command
        .arg("--exact")
        .arg(helper)
        .arg("--ignored")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(HELPER_ENV, helper)
        .env(ENV_DB, db_path(dir));
    for (key, value) in extra {
        command.env(key, value);
    }
    let output = command.output().expect("spawn helper process");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        stdout.contains("1 passed"),
        "helper `{helper}` 没有被真正执行（libtest 过滤/未匹配？），stdout：{stdout}\nstderr：{stderr}"
    );
    output
}

fn stdout_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn stderr_of(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

/// 维护锁持有进程：真的锁住两套候选路径，然后一直持到 `hold_ms` 或父进程收尾。
struct LockHolder {
    child: Child,
    stderr: Option<ChildStderr>,
}

impl LockHolder {
    fn spawn(dir: &Path, hold_ms: u64) -> Self {
        let mut child = Command::new(std::env::current_exe().expect("current exe"))
            .arg("--exact")
            .arg(HELPER_HOLD)
            .arg("--ignored")
            .arg("--nocapture")
            .arg("--test-threads=1")
            .env(HELPER_ENV, HELPER_HOLD)
            .env(ENV_DB, db_path(dir))
            .env(ENV_HOLD_MS, hold_ms.to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn maintenance lock holder");

        let mut stdout = BufReader::new(child.stdout.take().expect("holder stdout"));
        let line = read_until_marker(&mut stdout, &[MARK_LOCK_HELD, MARK_LOCK_FAIL]);
        let stderr = child.stderr.take();
        assert!(
            line.contains(MARK_LOCK_HELD),
            "维护锁持有进程没能拿到锁，无法建立断言前提：{line}"
        );
        Self { child, stderr }
    }

    /// 断言"另一进程此刻仍持锁"——用于证明写入/维护尝试是在锁**确实被持有**时发生的，
    /// 而不是等对方释放后才跑的（否则并发语义根本没被测到）。
    fn assert_alive(&mut self, context: &str) {
        let status = self.child.try_wait().expect("try_wait on lock holder");
        assert!(
            status.is_none(),
            "{context}：维护锁持有进程已退出（{status:?}），锁已释放，本次断言的并发前提不成立"
        );
    }

    /// 收尾并返回持有进程的 stderr（诊断用）。
    ///
    /// 直接 kill：锁已被证明"在操作期间确实被持有"（`assert_alive`），没必要再等它睡满，
    /// 进程退出即释放 flock。
    fn finish(mut self) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let mut text = String::new();
        if let Some(mut stderr) = self.stderr.take() {
            let _ = stderr.read_to_string(&mut text);
        }
        text
    }
}

impl Drop for LockHolder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// 逐行读到出现任一标记为止（helper 用 `println!` 输出，行缓冲会立即刷新）。
fn read_until_marker(reader: &mut BufReader<ChildStdout>, markers: &[&str]) -> String {
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).expect("read helper stdout");
        assert!(
            read > 0,
            "helper 在打印 {markers:?} 之前就退出了（进程崩溃或未被真正执行）"
        );
        if markers.iter().any(|marker| line.contains(marker)) {
            return line;
        }
    }
}

// ---------------------------------------------------------------------------
// 子进程 helper（父进程用 --exact --ignored 调起；被普通 `cargo test` 直接扫到时由守卫短路）
// ---------------------------------------------------------------------------

/// helper：在"另一个 pony 进程"里写一轮指定会话。
/// 成功打印 `WRITE_OK`，失败打印 `WRITE_ERR:<错误>`（错误文本是 D4 的断言对象）。
#[test]
#[ignore]
fn p1_helper_write_turn() {
    if helper_env(HELPER_ENV).is_none() {
        return;
    }
    let db = helper_env(ENV_DB).expect("P1_DB");
    let session = helper_env(ENV_SESSION).expect("P1_SESSION");
    let user = helper_env(ENV_USER).unwrap_or_else(|| "子进程问".to_string());

    let backend = SqliteSessionBackend::new(PathBuf::from(db));
    let mut store = SessionStore::with_backend(Box::new(backend));
    match store.append_turn_fallible(Some(&session), &user, "子进程答", None, Vec::new()) {
        Ok(_) => println!("{MARK_WRITE_OK}"),
        Err(error) => println!("{MARK_WRITE_ERR}:{error}"),
    }
}

/// helper：在"另一个进程"里持有库级维护锁（模拟 `rebuild-index` 正在跑 / 长驻维护进程）。
///
/// 同时锁住 `<db>.lock` 与 `<db>.maintenance.lock`：契约与实现目前对锁文件名口径不一致，
/// 两把都锁才能保证"无论实现取哪个名字，都必须撞锁"。
#[test]
#[ignore]
fn p1_helper_hold_maintenance_lock() {
    if helper_env(HELPER_ENV).is_none() {
        return;
    }
    let db = PathBuf::from(helper_env(ENV_DB).expect("P1_DB"));

    let mut held = Vec::new();
    for path in maintenance_lock_paths(&db) {
        let file = match std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) => {
                println!("{MARK_LOCK_FAIL}:open {}: {error}", path.display());
                return;
            }
        };
        match file.try_lock() {
            Ok(()) => held.push(file),
            Err(error) => {
                println!("{MARK_LOCK_FAIL}:lock {}: {error}", path.display());
                return;
            }
        }
    }

    let hold_ms: u64 = helper_env(ENV_HOLD_MS)
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(2000);
    println!("{MARK_LOCK_HELD}");
    std::thread::sleep(Duration::from_millis(hold_ms));
    // 显式释放，避免 Drop 顺序带来的歧义。
    for file in held {
        let _ = file.unlock();
    }
}

// ---------------------------------------------------------------------------
// D1–D5
// ---------------------------------------------------------------------------

/// D1：另一进程持有库级维护锁时，普通写入**不得**降级只读，且必须真的进索引。
#[test]
fn d1_writer_not_degraded_when_maintenance_lock_held() {
    let dir = TempDir::new("d1");
    let mut holder = LockHolder::spawn(dir.path(), 8000);

    let mut store = open_instance(dir.path());
    holder.assert_alive("D1 普通写入前");
    let write = store.append_turn_fallible(Some("p1_d1"), "维护期间的问", "维护期间的答", None, Vec::new());
    let write_error = write.err().map(|error| error.to_string());
    let indexed = indexed_message_count(dir.path(), "p1_d1", "维护期间的问");
    let holder_log = holder.finish();

    assert!(
        write_error.is_none(),
        "另一进程持有库级维护锁时，普通写入必须成功（不得整库降级只读），实际错误：{write_error:?}\n持有进程 stderr：{holder_log}"
    );
    assert!(
        indexed >= 1,
        "普通写入必须进入 SQLite 索引（P1：索引多进程可写），实际 normalized_messages 命中 {indexed} 行"
    );
}

/// D2：维护锁被占时，维护入口必须**显式**拒绝（`maintenance_lock_busy`），不得并发执行。
#[test]
fn d2_maintenance_refuses_when_lock_held() {
    let dir = TempDir::new("d2");
    let mut holder = LockHolder::spawn(dir.path(), 8000);

    let backend = SqliteSessionBackend::new(db_path(dir.path()));
    let outcome = rebuild_index(&backend, RebuildOptions { dry_run: false });
    // 维护尝试必须**立即**返回，而不是等持有者释放后串行执行——所以此刻对方还活着。
    holder.assert_alive("D2 维护尝试返回后");
    let message = outcome.err();
    let holder_log = holder.finish();

    let message = message.unwrap_or_else(|| {
        panic!("维护锁被另一进程持有时 rebuild_index 必须 fail-closed 失败，实际返回 Ok（并发执行了）")
    });
    assert!(
        message.contains("maintenance_lock_busy"),
        "维护冲突必须给出可 grep 的 `maintenance_lock_busy` 信号（契约 D2 / 接口变更），实际错误：{message}\n持有进程 stderr：{holder_log}"
    );
}

/// D3：两个真实进程写**不同**会话，必须都成功，且两边都进索引。
#[test]
fn d3_two_processes_write_different_sessions() {
    let dir = TempDir::new("d3");

    // 父进程**保持** store 存活（持有自己的会话写者），子进程在此期间写另一个会话。
    let mut local = open_instance(dir.path());
    write_turn_here(&mut local, "p1_d3_local", "父进程问", "父进程答");

    // D5 顺带自证：日常写入**不得**留下库级锁文件——库级锁只属于破坏性维护。
    for lock in maintenance_lock_paths(&db_path(dir.path())) {
        assert!(
            !lock.exists(),
            "P1 违约：日常写入创建了库级锁文件 `{}`（它只属于维护操作，日常写一旦持它，第二个进程就被挡成只读）",
            lock.display()
        );
    }

    let child = spawn_helper(
        HELPER_WRITE,
        dir.path(),
        &[(ENV_SESSION, "p1_d3_child"), (ENV_USER, "子进程问")],
    );
    let stdout = stdout_of(&child);
    let stderr = stderr_of(&child);
    assert!(
        child.status.success() && stdout.contains(MARK_WRITE_OK),
        "另一进程写不同会话必须成功（exit={:?}），stdout：{stdout}\nstderr：{stderr}",
        child.status.code()
    );

    drop(local);

    let parent_indexed = indexed_message_count(dir.path(), "p1_d3_local", "父进程问");
    assert!(
        parent_indexed >= 1,
        "本进程的写入也必须进入索引，实际 normalized_messages 命中 {parent_indexed} 行"
    );
    let child_indexed = indexed_message_count(dir.path(), "p1_d3_child", "子进程问");
    assert!(
        child_indexed >= 1,
        "另一进程的写入必须进入 SQLite 索引（P1：索引多进程可写），实际 normalized_messages 命中 {child_indexed} 行"
    );
}

/// D4：两个进程写**同一**会话——后者必须显式失败（`session_writer_busy`），前者完整，
/// 且后者仍能继续写**其它**会话（冲突不得让整个进程只读）。
#[test]
fn d4_same_session_second_writer_fails_explicitly() {
    let dir = TempDir::new("d4");

    // 父进程**保持** store 存活（= 该会话的写者），子进程写同一会话必须被拒。
    let mut local = open_instance(dir.path());
    write_turn_here(&mut local, "p1_d4", "第一问", "第一答");

    let child = spawn_helper(
        HELPER_WRITE,
        dir.path(),
        &[(ENV_SESSION, "p1_d4"), (ENV_USER, "第二问")],
    );
    let stdout = stdout_of(&child);
    let stderr = stderr_of(&child);
    assert!(
        stdout.contains(MARK_WRITE_ERR),
        "同一会话的第二个写者必须显式失败，stdout：{stdout}\nstderr：{stderr}"
    );
    assert!(
        stdout.contains("session_writer_busy"),
        "第二个写者的失败必须可识别为写者冲突（R-D 要求 `SessionError::SessionWriterBusy` + `session_writer_busy` 日志行），stdout：{stdout}"
    );

    let first_indexed = indexed_message_count(dir.path(), "p1_d4", "第一问");
    let second_indexed = indexed_message_count(dir.path(), "p1_d4", "第二问");
    assert!(
        first_indexed >= 1,
        "首个写者的数据必须完整进索引，实际 normalized_messages 命中 {first_indexed} 行"
    );
    assert_eq!(
        second_indexed, 0,
        "被拒绝的第二个写者不得写进索引（静默写入即违约）"
    );

    drop(local);
    let check = open_instance(dir.path());
    let view = check.snapshot_at_readonly(Some("p1_d4"), None, &[]);
    assert!(
        view.history.iter().any(|m| m.content.contains("第一问")),
        "首个写者的数据必须完整（日志真源 + 索引）"
    );
    assert!(
        !view.history.iter().any(|m| m.content.contains("第二问")),
        "被拒绝的第二个写者不得写入任何字节（日志真源也不得出现半条记录）"
    );

    let child_other = spawn_helper(
        HELPER_WRITE,
        dir.path(),
        &[(ENV_SESSION, "p1_d4_other"), (ENV_USER, "第三问")],
    );
    let other_stdout = stdout_of(&child_other);
    let other_stderr = stderr_of(&child_other);
    assert!(
        child_other.status.success() && other_stdout.contains(MARK_WRITE_OK),
        "会话冲突不得让整个进程只读：换一个会话必须仍能写（exit={:?}），stdout：{other_stdout}\nstderr：{other_stderr}",
        child_other.status.code()
    );
}

/// D5：日常写入路径不得再取库级实例锁——维护锁被占时仍能写，且进程日志里没有
/// `single_instance_lock_denied`（基线"整库降级只读"的指纹）。
#[test]
fn d5_daily_write_path_does_not_take_instance_lock() {
    let dir = TempDir::new("d5");
    let mut holder = LockHolder::spawn(dir.path(), 8000);

    // 本进程：维护锁被占时走两次日常写入。
    let mut store = open_instance(dir.path());
    holder.assert_alive("D5 本进程日常写入前");
    let first_error = store
        .append_turn_fallible(Some("p1_d5"), "问一", "答一", None, Vec::new())
        .err()
        .map(|error| error.to_string());
    let second_error = store
        .append_turn_fallible(Some("p1_d5"), "问二", "答二", None, Vec::new())
        .err()
        .map(|error| error.to_string());
    holder.assert_alive("D5 本进程两次写入返回后");

    // 另一进程：同一把维护锁仍被持有，它也必须能正常写。
    let child = spawn_helper(
        HELPER_WRITE,
        dir.path(),
        &[(ENV_SESSION, "p1_d5_child"), (ENV_USER, "子进程问")],
    );
    holder.assert_alive("D5 子进程写入返回后");
    let holder_log = holder.finish();

    let child_stdout = stdout_of(&child);
    let child_stderr = stderr_of(&child);

    assert!(
        first_error.is_none() && second_error.is_none(),
        "日常写入必须完全不依赖库级实例锁（维护锁被占时仍要成功）；first={first_error:?} second={second_error:?}\n持有进程 stderr：{holder_log}"
    );

    let first_indexed = indexed_message_count(dir.path(), "p1_d5", "问一");
    let second_indexed = indexed_message_count(dir.path(), "p1_d5", "问二");
    assert!(
        first_indexed >= 1 && second_indexed >= 1,
        "维护锁被占时日常写入仍必须进索引，实际 问一={first_indexed} 行、问二={second_indexed} 行"
    );

    assert!(
        child_stdout.contains(MARK_WRITE_OK),
        "维护锁被占时另一进程的日常写入也必须成功，stdout：{child_stdout}\nstderr：{child_stderr}"
    );

    // D5 核心判据：日常写路径不得再"抢库级锁失败 ⇒ 整库降级只读"。
    // 基线的子进程会打印 single_instance_lock_denied 并 PRAGMA query_only=ON，故此项必红。
    let combined = format!("{child_stdout}\n{child_stderr}\n{holder_log}");
    assert!(
        !combined.contains(DEGRADED_MARKER),
        "日常写路径不得再触碰库级实例锁：进程日志中不应出现 `{DEGRADED_MARKER}`（出现即代表整库降级只读），实际：{combined}"
    );
}
