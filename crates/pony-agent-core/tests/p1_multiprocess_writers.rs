//! ADR 0107 §5 P1 验收：多进程写索引 / 维护锁 / 会话级单写者（D1–D5）。
//!
//! ## 为什么必须真跨进程
//!
//! 基线的单实例锁走**进程内登记表**（`LOCK_REGISTRY`，Weak 复用），同进程开两个
//! `SqliteSessionBackend` 会复用同一 guard、**不会降级只读**——用它建模两进程会得到假绿。
//! 因此本文件用 **self-exec helper**：父进程 `Command::new(current_exe())` 重跑同一测试
//! 二进制（`--exact <helper> --ignored --nocapture`）+ 环境变量传参，子进程做"另一个
//! pony 进程"的写库动作，父进程按退出码 / 标准输出断言。
//!
//! 基线（P0 分支 `e043f32f`）预期：D1/D3/D4/D5 红（第二进程整库降级只读），D2 绿（保护）。

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use pony_agent_core::agent::session::{rebuild_index, RebuildOptions, SessionStore};
use pony_agent_core::agent::sqlite_session::SqliteSessionBackend;

const HELPER_ENV: &str = "PONY_P1_HELPER";

fn temp_dir(label: &str) -> PathBuf {
    let unique = format!(
        "pony-p1-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn db_path(dir: &Path) -> PathBuf {
    dir.join("sessions.db")
}

fn open_instance(dir: &Path) -> SessionStore {
    let backend = SqliteSessionBackend::new(db_path(dir));
    SessionStore::with_backend(Box::new(backend))
}

/// 直接查索引表：该会话的 user 消息里是否包含指定文本（不合并日志，只看索引）。
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

/// 起一个真实子进程（同一测试二进制）执行 helper。
fn spawn_helper(helper: &str, dir: &Path, extra: &[(&str, &str)]) -> Output {
    let mut command = Command::new(std::env::current_exe().expect("current exe"));
    command
        .arg("--exact")
        .arg(helper)
        .arg("--ignored")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(HELPER_ENV, "1")
        .env("PONY_P1_DB", db_path(dir));
    for (key, value) in extra {
        command.env(key, value);
    }
    command.output().expect("spawn helper process")
}

/// 起一个持锁子进程（不等待，返回句柄）。
fn spawn_lock_holder(dir: &Path, hold_ms: u64) -> std::process::Child {
    Command::new(std::env::current_exe().expect("current exe"))
        .arg("--exact")
        .arg("p1_helper_hold_db_lock")
        .arg("--ignored")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env(HELPER_ENV, "1")
        .env("PONY_P1_DB", db_path(dir))
        .env("PONY_P1_HOLD_MS", hold_ms.to_string())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("spawn lock holder")
}

// ---------------------------------------------------------------------------
// 子进程 helper（父进程用 --ignored --exact 调起；直接跑会被守卫短路）
// ---------------------------------------------------------------------------

/// helper：在"另一进程"里写一轮指定会话，成功打印 `WRITE_OK`，失败打印 `WRITE_ERR:<err>`。
#[test]
#[ignore]
fn p1_helper_write_turn() {
    if helper_env(HELPER_ENV).is_none() {
        return;
    }
    let db = helper_env("PONY_P1_DB").expect("PONY_P1_DB");
    let session = helper_env("PONY_P1_SESSION").expect("PONY_P1_SESSION");
    let user = helper_env("PONY_P1_USER").unwrap_or_else(|| "子进程问".to_string());

    let backend = SqliteSessionBackend::new(PathBuf::from(db));
    let mut store = SessionStore::with_backend(Box::new(backend));
    match store.append_turn_fallible(Some(&session), &user, "子进程答", None, Vec::new()) {
        Ok(_) => println!("WRITE_OK"),
        Err(error) => println!("WRITE_ERR:{error}"),
    }
}

/// helper：持有库级锁若干毫秒（模拟"另一进程正在维护/长驻"）。
#[test]
#[ignore]
fn p1_helper_hold_db_lock() {
    if helper_env(HELPER_ENV).is_none() {
        return;
    }
    let db = helper_env("PONY_P1_DB").expect("PONY_P1_DB");
    let lock_path = PathBuf::from(format!("{db}.lock"));
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .expect("open db lock");
    file.try_lock().expect("acquire db lock");
    let hold_ms: u64 = helper_env("PONY_P1_HOLD_MS")
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(1500);
    println!("LOCK_HELD");
    std::thread::sleep(std::time::Duration::from_millis(hold_ms));
}

// ---------------------------------------------------------------------------
// D1–D5
// ---------------------------------------------------------------------------

/// D1：另一进程持着库级锁时，普通写入**不得**降级只读。
#[test]
fn d1_writer_not_degraded_when_maintenance_lock_held() {
    let dir = temp_dir("d1");
    let mut holder = spawn_lock_holder(&dir, 1500);
    std::thread::sleep(std::time::Duration::from_millis(400));

    let mut store = open_instance(&dir);
    let write = store.append_turn_fallible(Some("p1_d1"), "问", "答", None, Vec::new());
    let _ = holder.wait();

    assert!(
        write.is_ok(),
        "另一进程持库级锁时普通写入必须成功（不得整库降级只读），实际：{:?}",
        write.err().map(|error| error.to_string())
    );
}

/// D2：维护锁被占时，维护入口必须拒绝并发执行。
#[test]
fn d2_maintenance_refuses_when_lock_held() {
    let dir = temp_dir("d2");
    let mut holder = spawn_lock_holder(&dir, 1500);
    std::thread::sleep(std::time::Duration::from_millis(400));

    let backend = SqliteSessionBackend::new(db_path(&dir));
    let outcome = rebuild_index(&backend, RebuildOptions { dry_run: false });
    let _ = holder.wait();

    assert!(
        outcome.is_err(),
        "维护锁被占时 rebuild_index 必须显式失败，实际返回 Ok(rebuilt={})",
        outcome.map(|report| report.rebuilt.len()).unwrap_or(0)
    );
}

/// D3：两个真实进程写不同会话，必须都成功。
#[test]
fn d3_two_processes_write_different_sessions() {
    let dir = temp_dir("d3");

    // 父进程**保持** store 存活（持有库级锁），子进程在此期间写另一个会话。
    let mut local = open_instance(&dir);
    write_turn_here(&mut local, "p1_d3_local", "本地问", "本地答");
    // 自证（P1 语义）：日常写入**不得**留下库级独占锁——库级锁只属于破坏性维护。
    // P0 时代此处断言的是"父进程持锁"，P1 明确推翻：日常写若仍持库级锁，
    // 第二个进程就会被挡成只读（正是本次要修的病根）。
    {
        let lock_path = std::path::PathBuf::from(format!("{}.lock", db_path(&dir).display()));
        if lock_path.exists() {
            let probe = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&lock_path)
                .expect("open lock probe");
            assert!(
                probe.try_lock().is_ok(),
                "P1 违约：日常写入后仍持有库级独占锁（`<db>.lock`），第二进程会被降级只读"
            );
        }
    }

    let child = spawn_helper(
        "p1_helper_write_turn",
        &dir,
        &[("PONY_P1_SESSION", "p1_d3_child"), ("PONY_P1_USER", "子进程问")],
    );
    let stdout = String::from_utf8_lossy(&child.stdout).to_string();
    assert!(
        child.status.success() && stdout.contains("WRITE_OK"),
        "另一进程写不同会话必须成功（exit={:?}），输出：{stdout}",
        child.status.code()
    );

    drop(local);

    // 判据必须落在**索引**上：P0 之后降级实例仍能写会话日志（`LogPersisted`），
    // 而 `snapshot_at_readonly` 会合并日志真值 ⇒ 只看历史会假绿。
    // P1 要求：另一进程的写入进入 SQLite 索引（索引多进程可写）。
    let indexed = indexed_message_count(&dir, "p1_d3_child", "子进程问");
    assert!(
        indexed >= 1,
        "另一进程的写入必须进入 SQLite 索引（P1：索引多进程可写），实际 normalized_messages 命中 {indexed} 行"
    );
}

/// D4：两个进程写**同一**会话——后者必须显式失败，前者完整，后者换会话仍成功。
#[test]
fn d4_same_session_second_writer_fails_explicitly() {
    let dir = temp_dir("d4");

    // 父进程**保持** store 存活（持有该会话的写者），子进程写同一会话必须被拒。
    let mut local = open_instance(&dir);
    write_turn_here(&mut local, "p1_d4", "第一问", "第一答");

    let child = spawn_helper(
        "p1_helper_write_turn",
        &dir,
        &[("PONY_P1_SESSION", "p1_d4"), ("PONY_P1_USER", "第二问")],
    );
    let stdout = String::from_utf8_lossy(&child.stdout).to_string();
    assert!(
        stdout.contains("WRITE_ERR"),
        "同一会话的第二个写者必须失败，实际输出：{stdout}"
    );
    assert!(
        stdout.contains("session_writer_busy")
            || stdout.contains("Locked")
            || stdout.contains("lock"),
        "第二个写者的失败必须可识别为写者冲突，实际输出：{stdout}"
    );

    drop(local);
    let check = open_instance(&dir);
    let view = check.snapshot_at_readonly(Some("p1_d4"), None, &[]);
    assert!(
        view.history.iter().any(|m| m.content.contains("第一问")),
        "首个写者的数据必须完整"
    );
    assert!(
        !view.history.iter().any(|m| m.content.contains("第二问")),
        "被拒绝的第二个写者不得写入任何字节"
    );

    let child_other = spawn_helper(
        "p1_helper_write_turn",
        &dir,
        &[("PONY_P1_SESSION", "p1_d4_other"), ("PONY_P1_USER", "第三问")],
    );
    let other_stdout = String::from_utf8_lossy(&child_other.stdout).to_string();
    assert!(
        child_other.status.success() && other_stdout.contains("WRITE_OK"),
        "会话冲突不得让整个进程只读：换一个会话必须仍能写，输出：{other_stdout}"
    );
}

/// D5：日常写入路径不依赖库级实例锁——另一进程持锁期间仍能连续写入。
#[test]
fn d5_daily_write_path_does_not_take_instance_lock() {
    let dir = temp_dir("d5");
    let mut holder = spawn_lock_holder(&dir, 1500);
    std::thread::sleep(std::time::Duration::from_millis(400));

    let mut store = open_instance(&dir);
    let first = store.append_turn_fallible(Some("p1_d5"), "问一", "答一", None, Vec::new());
    let second = store.append_turn_fallible(Some("p1_d5"), "问二", "答二", None, Vec::new());
    let _ = holder.wait();

    assert!(
        first.is_ok() && second.is_ok(),
        "日常写入必须完全不依赖库级实例锁；first={:?} second={:?}",
        first.err().map(|error| error.to_string()),
        second.err().map(|error| error.to_string())
    );
}
