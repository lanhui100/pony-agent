//! 单实例会话库锁（`<db>.lock`）：跨进程独占 + **持有者身份自描述** + 三态查询。
//!
//! 事故背景（真实数据丢失）：第二个 `pony` TUI 抢不到 `sessions.db.lock` 时，旧实现只在
//! stderr 打一句「已被其他进程独占」——而 TUI 启动后把 stderr `dup2` 到
//! `~/.pony-agent/tui-logs/tui-runtime.log`，用户永远看不到；它照常进 TUI、照常接收输入、
//! 每轮在提交边界失败，最后还打印 `pony resume <id>` 这种假承诺。修这个洞需要两件事：
//!
//! 1. 锁文件里写清楚**谁**持有（pid / 启动时间 / 命令行），抢不到时日志直接带身份；
//! 2. 让宿主（`bin/tui.rs`）在**创建任何 `SessionStore` 之前**能查询锁状态并 fail-closed。
//!
//! 第 2 点要求 `pub` 可达，而原实作所在 `sqlite_session_commit.rs` 是
//! `sqlite_session.rs` 里的私有子模块（外部 crate 目标无法寻址），且 `sqlite_session.rs`
//! 已计入 file-size 棘轮（净值不得增长）。因此「锁原语 + 进程内登记表 + 状态查询」收敛到
//! 本模块；`sqlite_session_commit.rs` 侧只保留一行 re-export，既有调用点零改动。
//!
//! 纪律：查询**不阻塞、不忙等、不 panic**；抢锁失败仍然只降级只读（调用方决定），本模块
//! 不做「重试直到成功」这类会把启动卡住的尝试。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

/// 进程内锁登记表：`flock` 按 open file description 记账，同进程重复 `open` 会自锁，
/// 故同一 db 路径在本进程内共享同一把锁（一个进程 = 一个 writer 实例语义）。
///
/// 用 `Weak` 持有：锁的存活由 `SqliteSessionBackend` 侧持有的 `Arc` 决定（backend drop
/// ⇒ 锁释放 ⇒ 登记项自然失效），登记表因此**有界**（每条 acquire 顺带 prune 死项）。
static LOCK_REGISTRY: OnceLock<Mutex<HashMap<PathBuf, Weak<std::fs::File>>>> = OnceLock::new();

/// 写进 `<db>.lock` 的持有者身份（字段式文本，`cat` / `fuser -v` 旁边就能读）。
///
/// 字段全是 `Option`：锁文件可能是旧版本（只建空文件）或被外部改写，读不全时**降级为
/// 部分信息**，绝不 panic、绝不把「读不到」当成「没有持有者」。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InstanceLockHolder {
    pub pid: Option<u32>,
    pub command: Option<String>,
    pub since_unix_ms: Option<u64>,
}

/// 单实例锁的三态查询结果（语义与 [`acquire_single_instance_lock`] 一致）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstanceLockStatus {
    /// 本进程已持有，或当前无人持有 ⇒ 现在启动会拿到写权限。
    Writable,
    /// 另一个进程正独占该库 ⇒ 宿主必须 fail-closed（不得静默降级为「可交互只读」）。
    HeldByOther {
        pid: Option<u32>,
        command: Option<String>,
        since_unix_ms: Option<u64>,
    },
    /// 连探测都做不到（锁文件打不开等）⇒ 交由调用方按既有流程处理。
    Unavailable { reason: String },
}

/// `<db>` 对应的锁文件路径（与历史实作逐字一致：`<db>.lock`）。
pub fn lock_file_path(db_path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.lock", db_path.display()))
}

/// E5 / NFR `concurrency_lock`：尝试独占 `<db>.lock`。
///
/// - `Ok(Some(guard))`：已持有（本进程），调用方必须把 `guard` 存住直到 backend drop；
/// - `Ok(None)`：其他进程独占（或锁文件不可独占）⇒ 调用方**必须**明确降级只读
///   （`PRAGMA query_only=ON` + 结构化日志），不得忙等、不得 panic，也不得继续写；
/// - `Err`：连锁文件都建不出来（路径不可写）⇒ 同样按降级处理由调用方决定。
pub(crate) fn acquire_single_instance_lock(
    db_path: &Path,
) -> Result<Option<Arc<std::fs::File>>, String> {
    let registry = LOCK_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()));
    let key = db_path.to_path_buf();
    let mut held = registry
        .lock()
        .map_err(|error| format!("lock registry poisoned: {error}"))?;
    held.retain(|_, weak| weak.strong_count() > 0);
    if let Some(existing) = held.get(&key).and_then(Weak::upgrade) {
        return Ok(Some(existing));
    }
    let lock_path = lock_file_path(db_path);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|error| format!("open lock file {}: {error}", lock_path.display()))?;
    if try_lock_exclusive(&file) {
        // best-effort：身份写失败**不得**影响抢锁结果（拿不到锁才是致命路径）。
        let _ = write_holder_identity(&file);
        let guard = Arc::new(file);
        held.insert(key, Arc::downgrade(&guard));
        return Ok(Some(guard));
    }
    let holder = read_holder_identity(&lock_path);
    eprintln!(
        "[pony-agent][session] single_instance_lock_denied: {} {}；\
         本实例降级只读（不并发写、不静默覆盖）",
        lock_path.display(),
        holder_summary(holder.as_ref())
    );
    Ok(None)
}

/// 查询 `<db>.lock` 的当前归属：`Writable` / `HeldByOther{..}` / `Unavailable{..}`。
///
/// 供宿主**启动预检**使用（`bin/tui.rs` 在创建任何 `SessionStore` 之前调用）。语义：
///
/// - 先查进程内登记表：本进程已持有 ⇒ `Writable`。**必须**走这一步，否则同进程二次
///   `open` + `flock` 会自锁，把一个正常实例误判成「被别人占着」；
/// - 再非阻塞试锁一次：拿到即 `Writable`，并**立刻释放**（查询不得变成长期占用）；
/// - 拿不到 ⇒ 读回锁文件里的持有者身份，`HeldByOther`（读不到就返回全 `None`）；
/// - 锁文件不存在 ⇒ `Writable`（首次启动的正常路径，不去创建文件）；
/// - 打不开锁文件（权限等）⇒ `Unavailable`，不 panic。
pub fn single_instance_lock_status(db_path: &Path) -> InstanceLockStatus {
    if process_holds_lock(db_path) {
        return InstanceLockStatus::Writable;
    }
    let lock_path = lock_file_path(db_path);
    let file = match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            // 没人建过锁文件 = 没有进程在用这个库的写权限。
            return InstanceLockStatus::Writable;
        }
        Err(error) => {
            return InstanceLockStatus::Unavailable {
                reason: format!("open lock file {}: {error}", lock_path.display()),
            };
        }
    };
    if try_lock_exclusive(&file) {
        // 探测即释放：drop 关闭 fd，`flock` 随之释放（不得把查询变成长期占用）。
        drop(file);
        return InstanceLockStatus::Writable;
    }
    let holder = read_holder_identity(&lock_path);
    InstanceLockStatus::HeldByOther {
        pid: holder.as_ref().and_then(|holder| holder.pid),
        command: holder.as_ref().and_then(|holder| holder.command.clone()),
        since_unix_ms: holder.as_ref().and_then(|holder| holder.since_unix_ms),
    }
}

/// 本进程是否已经持有该库的实例锁（登记表判定；`Weak` 失效 = 已释放）。
fn process_holds_lock(db_path: &Path) -> bool {
    let Some(registry) = LOCK_REGISTRY.get() else {
        return false;
    };
    registry
        .lock()
        .map(|held| held.get(db_path).and_then(Weak::upgrade).is_some())
        .unwrap_or(false)
}

/// 把当前进程身份写进锁文件（truncate + write + flush）。
///
/// 只由**抢到锁**的进程调用：此时无人并发写这个文件。失败一律由调用方忽略。
fn write_holder_identity(file: &std::fs::File) -> Result<(), String> {
    use std::io::{Seek, SeekFrom, Write};
    let payload = format!(
        "pid={}\nstart_unix_ms={}\ncommand={}\n",
        std::process::id(),
        now_unix_ms(),
        current_process_command()
    );
    let mut file = file;
    file.seek(SeekFrom::Start(0))
        .map_err(|error| format!("seek lock file: {error}"))?;
    file.set_len(0)
        .map_err(|error| format!("truncate lock file: {error}"))?;
    file.write_all(payload.as_bytes())
        .map_err(|error| format!("write lock file: {error}"))?;
    file.flush()
        .map_err(|error| format!("flush lock file: {error}"))?;
    Ok(())
}

/// 读回锁文件里的持有者身份。读不到 / 格式不认识 ⇒ `None`（调用方按 `holder=unknown` 记）。
fn read_holder_identity(lock_path: &Path) -> Option<InstanceLockHolder> {
    let text = std::fs::read_to_string(lock_path).ok()?;
    let mut holder = InstanceLockHolder::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "pid" => holder.pid = value.parse::<u32>().ok(),
            "start_unix_ms" => holder.since_unix_ms = value.parse::<u64>().ok(),
            "command" if !value.is_empty() => holder.command = Some(value.to_string()),
            _ => {}
        }
    }
    if holder.pid.is_none() && holder.since_unix_ms.is_none() && holder.command.is_none() {
        return None;
    }
    Some(holder)
}

/// 日志/诊断里的身份摘要：`holder pid=… cmd=… since=…`，读不到就是 `holder=unknown`。
fn holder_summary(holder: Option<&InstanceLockHolder>) -> String {
    let unknown = || "unknown".to_string();
    match holder {
        None => "holder=unknown".to_string(),
        Some(holder) => format!(
            "holder pid={} cmd={} since={}",
            holder.pid.map(|pid| pid.to_string()).unwrap_or_else(unknown),
            holder.command.clone().unwrap_or_else(unknown),
            holder
                .since_unix_ms
                .map(|since| since.to_string())
                .unwrap_or_else(unknown)
        ),
    }
}

/// 当前进程的命令行（Linux 读 `/proc/self/cmdline`）；其他平台/读失败退化为 exe 路径。
fn current_process_command() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(bytes) = std::fs::read("/proc/self/cmdline") {
            let args: Vec<String> = bytes
                .split(|byte| *byte == 0)
                .filter(|arg| !arg.is_empty())
                .map(|arg| String::from_utf8_lossy(arg).to_string())
                .collect();
            if !args.is_empty() {
                return sanitize_holder_command(&args.join(" "));
            }
        }
    }
    let exe = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
    sanitize_holder_command(&exe)
}

/// 锁文件是**逐行**解析的：命令行里的换行/回车必须压平，否则会伪造出额外字段。
/// 同时限长（argv 可能很长），避免把锁文件写成日志。
fn sanitize_holder_command(command: &str) -> String {
    const MAX_COMMAND_CHARS: usize = 512;
    let flattened: String = command
        .chars()
        .map(|c| if c == '\n' || c == '\r' || c == '\0' { ' ' } else { c })
        .collect();
    if flattened.chars().count() <= MAX_COMMAND_CHARS {
        return flattened;
    }
    let truncated: String = flattened.chars().take(MAX_COMMAND_CHARS).collect();
    format!("{truncated}…")
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// 尝试非阻塞独占锁。返回 `true` = 已持有；`false` = 抢不到（含非 EWOULDBLOCK 的异常
/// 错误——一律降级只读并落日志，硬失败会让用户连"只读看历史"都做不到）。
#[cfg(unix)]
fn try_lock_exclusive(file: &std::fs::File) -> bool {
    use std::os::unix::io::AsRawFd;
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if result == 0 {
        return true;
    }
    let error = std::io::Error::last_os_error();
    if !matches!(error.raw_os_error(), Some(code) if code == libc::EWOULDBLOCK) {
        eprintln!(
            "[pony-agent][session] single_instance_lock_error：flock 异常，按抢不到处理并降级只读：{error}"
        );
    }
    false
}

#[cfg(windows)]
fn try_lock_exclusive(file: &std::fs::File) -> bool {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{ERROR_LOCK_VIOLATION, HANDLE};
    use windows_sys::Win32::Storage::FileSystem::{
        LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
    };
    use windows_sys::Win32::System::IO::OVERLAPPED;
    let handle = file.as_raw_handle() as HANDLE;
    let mut overlapped: OVERLAPPED = unsafe { std::mem::zeroed() };
    let flags = LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY;
    // 锁住全部字节区间（0xFFFFFFFF/0xFFFFFFFF = 整个文件）。
    let locked = unsafe { LockFileEx(handle, flags, 0, u32::MAX, u32::MAX, &mut overlapped) };
    if locked != 0 {
        return true;
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() != Some(ERROR_LOCK_VIOLATION as i32) {
        eprintln!(
            "[pony-agent][session] single_instance_lock_error：LockFileEx 异常，按抢不到处理并降级只读：{error}"
        );
    }
    false
}

#[cfg(not(any(unix, windows)))]
fn try_lock_exclusive(_file: &std::fs::File) -> bool {
    true
}

#[cfg(test)]
mod tests {
    //! 回归防线（真实事故：第二个实例静默只读 → 会话没落库）。
    //!
    //! 覆盖三件事：
    //! 1. 同进程二次获取**不得**自锁，状态仍是 `Writable`；
    //! 2. 跨进程持有者身份可读（pid 必须对得上真实子进程，而不是「unknown」）；
    //! 3. 持有者退出后状态回到 `Writable`。

    use super::*;
    use std::time::{Duration, Instant};

    /// 子进程持有锁时要等的上限：父进程异常时自己退出，绝不留僵尸进程。
    const CHILD_HOLD_TIMEOUT: Duration = Duration::from_secs(30);
    const PARENT_WAIT_TIMEOUT: Duration = Duration::from_secs(15);

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new(label: &str) -> Self {
            let unique = format!(
                "pony-instance-lock-{label}-{}-{}",
                std::process::id(),
                now_unix_ms()
            );
            let path = std::env::temp_dir().join(unique);
            std::fs::create_dir_all(&path).expect("create temp dir");
            Self { path }
        }

        fn db(&self) -> PathBuf {
            self.path.join("sessions.db")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn missing_lock_file_reads_as_writable() {
        let temp = TempDir::new("missing");
        let db = temp.db();
        assert!(
            !lock_file_path(&db).exists(),
            "前置条件：锁文件不应存在（{}）",
            lock_file_path(&db).display()
        );
        assert_eq!(
            single_instance_lock_status(&db),
            InstanceLockStatus::Writable,
            "首次启动（没有锁文件）必须判可写"
        );
        assert!(
            !lock_file_path(&db).exists(),
            "查询不得有副作用（不得顺手创建锁文件）"
        );
    }

    #[test]
    fn same_process_second_acquire_is_writable_not_self_locked() {
        let temp = TempDir::new("self");
        let db = temp.db();
        let first = acquire_single_instance_lock(&db)
            .expect("acquire first")
            .expect("first acquire must win the lock");
        assert_eq!(
            single_instance_lock_status(&db),
            InstanceLockStatus::Writable,
            "本进程已持有 ⇒ 必须判 Writable（否则查询会把正常实例误判成被别人占着）"
        );
        let second = acquire_single_instance_lock(&db)
            .expect("acquire second")
            .expect("同进程二次获取必须复用同一把锁，不得自锁");
        assert!(
            Arc::ptr_eq(&first, &second),
            "同进程二次获取必须拿到同一个 guard（登记表复用）"
        );
        drop(second);
        drop(first);
        assert_eq!(
            single_instance_lock_status(&db),
            InstanceLockStatus::Writable,
            "guard 全部释放后必须回到 Writable"
        );
    }

    #[test]
    fn identity_is_written_into_lock_file_by_the_holder() {
        let temp = TempDir::new("identity");
        let db = temp.db();
        let guard = acquire_single_instance_lock(&db)
            .expect("acquire")
            .expect("first acquire must win the lock");
        let text = std::fs::read_to_string(lock_file_path(&db)).expect("lock file readable");
        assert!(
            text.contains(&format!("pid={}", std::process::id())),
            "抢到锁后必须把本进程 pid 写进锁文件，实际 = {text:?}"
        );
        assert!(
            text.contains("start_unix_ms="),
            "必须写启动时间，实际 = {text:?}"
        );
        assert!(
            text.contains("command="),
            "必须写命令行，实际 = {text:?}"
        );
        drop(guard);
    }

    /// 子进程入口：被父测试用 `PONY_INSTANCE_LOCK_HOLDER_DB` 重新执行本测试二进制后，
    /// 拿住锁、写 ready 标记，然后等 stop 标记（最多 [`CHILD_HOLD_TIMEOUT`]）。
    ///
    /// 这条用例在正常 `cargo test` 下是空跑（环境变量不存在 ⇒ 立即返回）。
    #[test]
    fn holder_child_process() {
        let Ok(db_path) = std::env::var("PONY_INSTANCE_LOCK_HOLDER_DB") else {
            return;
        };
        let ready_path =
            std::env::var("PONY_INSTANCE_LOCK_HOLDER_READY").expect("ready path env");
        let stop_path = std::env::var("PONY_INSTANCE_LOCK_HOLDER_STOP").expect("stop path env");
        let _guard = acquire_single_instance_lock(Path::new(&db_path))
            .expect("child acquire")
            .expect("child must win the lock");
        std::fs::write(&ready_path, std::process::id().to_string()).expect("write ready");
        let deadline = Instant::now() + CHILD_HOLD_TIMEOUT;
        while Instant::now() < deadline {
            if Path::new(&stop_path).exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn other_process_holder_is_reported_with_its_pid() {
        let temp = TempDir::new("cross-process");
        let db = temp.db();
        let ready = temp.path.join("ready");
        let stop = temp.path.join("stop");
        let exe = std::env::current_exe().expect("current test exe");
        let mut child = std::process::Command::new(exe)
            .arg("--exact")
            .arg("agent::instance_lock::tests::holder_child_process")
            .arg("--nocapture")
            .env("PONY_INSTANCE_LOCK_HOLDER_DB", &db)
            .env("PONY_INSTANCE_LOCK_HOLDER_READY", &ready)
            .env("PONY_INSTANCE_LOCK_HOLDER_STOP", &stop)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn lock holder child");

        let wait_ready = Instant::now() + PARENT_WAIT_TIMEOUT;
        while !ready.exists() && Instant::now() < wait_ready {
            assert!(
                child.try_wait().expect("poll child").is_none(),
                "持锁子进程提前退出（ready 未出现）"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(ready.exists(), "持锁子进程在超时内未就绪");
        assert!(
            lock_file_path(&db).exists(),
            "子进程抢到锁后应留下锁文件（{}）",
            lock_file_path(&db).display()
        );

        let status = single_instance_lock_status(&db);
        let InstanceLockStatus::HeldByOther {
            pid,
            command,
            since_unix_ms,
        } = status
        else {
            let _ = std::fs::write(&stop, b"stop");
            let _ = child.wait();
            panic!("另一个进程持锁时必须判 HeldByOther");
        };
        assert_eq!(
            pid,
            Some(child.id()),
            "持有者 pid 必须等于真实子进程 pid（读不到身份就是「用户不知道谁占着库」）"
        );
        assert!(
            command.as_deref().is_some_and(|cmd| !cmd.is_empty()),
            "持有者命令行不得为空，实际 = {command:?}"
        );
        assert!(
            since_unix_ms.is_some_and(|since| since > 0),
            "持有者启动时间必须可读，实际 = {since_unix_ms:?}"
        );

        std::fs::write(&stop, b"stop").expect("write stop marker");
        let wait_exit = Instant::now() + PARENT_WAIT_TIMEOUT;
        loop {
            if child.try_wait().expect("poll child").is_some() {
                break;
            }
            if Instant::now() >= wait_exit {
                let _ = child.kill();
                let _ = child.wait();
                panic!("持锁子进程未在超时内退出（已强杀，避免留下僵尸进程）");
            }
            std::thread::sleep(Duration::from_millis(20));
        }

        assert_eq!(
            single_instance_lock_status(&db),
            InstanceLockStatus::Writable,
            "持有者退出后必须回到 Writable（否则下一个实例会被永久拒绝启动）"
        );
    }
}
