//! Asynchronous background job lifecycle tool family (PA-105).
//!
//! Provides `job_start`, `job_output`, `job_kill`, and `job_list` tools.
//!
//! Design & Invariants:
//! - Background asynchronous child process spawning: non-blocking start returning `job_id` and `pid`.
//! - Output incremental reading: ring buffer with monotonically increasing cursor and `offset`.
//! - Blocking wait with bounded timeout support in `job_output`.
//! - Process tree cascading kill & reap (Windows: Job Object kill-on-close; POSIX: Process group setpgid + killpg).
//! - History retention: retains terminated jobs up to `MAX_HISTORY_JOBS` for `job_list` and output inspection.
//! - Fail-closed path traversal validation for `cwd`.
//! - Thread-safe global registry with UUID job keys.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::agent::budget::CancellationToken;
use crate::agent::process::ProcessManager;
use crate::agent::tool_runtime::{ProcessBackend, ProcessPollResult, ProcessState};

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

/// Max buffer capacity per job: 2MB.
pub const JOB_BUFFER_CAP: usize = 2 * 1024 * 1024;

/// Maximum retained completed/killed jobs in history.
pub const MAX_HISTORY_JOBS: usize = 64;

/// 契约 B012：跨会话访问作业的错误码（`error.code` 与 `error.kind` 同值）。
pub const JOB_NOT_OWNED_CODE: &str = "job_not_owned";

/// 契约 B012：作业归属判定。仅当**双方都有 owner 且不相等**时拒绝；任一侧为 `None`
/// （内部直调 / 未提供会话上下文）一律放行，保持既有单测与内部调用不受限。
fn ensure_owned(job: &JobSession, caller: Option<&str>) -> Result<(), String> {
    match (job.owner.as_deref(), caller) {
        (Some(owner), Some(caller)) if owner != caller => {
            // message 不得含任何作业输出/命令行，避免拒绝响应成为泄露通道。
            Err(format!("{JOB_NOT_OWNED_CODE}: job not owned by caller"))
        }
        _ => Ok(()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStartArgs {
    pub command: String,
    pub args: Option<Vec<String>>,
    pub cwd: Option<String>,
    /// 工具 schema 声明 `timeoutMs`（camelCase），结构体是 snake_case：两种拼法都收。
    #[serde(alias = "timeoutMs")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobStartResult {
    pub job_id: String,
    pub pid: u32,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobOutputArgs {
    /// schema 口径是 `jobId`；模型也常发 `job_id`，两种拼法都收（见 stream_support 的别名归一）。
    #[serde(alias = "jobId")]
    pub job_id: String,
    pub wait: Option<bool>,
    #[serde(alias = "timeoutMs")]
    pub timeout_ms: Option<u64>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobOutputResult {
    pub job_id: String,
    pub status: String,
    pub output: String,
    pub cursor: usize,
    pub exit_code: Option<i32>,
    /// promoted job 的粘性截断证据（`Some`）；`job_start` 自建作业不追踪截断（`None`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_truncated: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobKillArgs {
    #[serde(alias = "jobId")]
    pub job_id: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobKillResult {
    pub job_id: String,
    pub killed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobListArgs {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobListItem {
    pub job_id: String,
    pub command: String,
    pub status: String,
    pub pid: u32,
    pub started_at: u64,
    /// 作业来源：`owned`（`job_start` 自建）或 `promoted`（Run 到期转后台）。
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobListResult {
    pub jobs: Vec<JobListItem>,
}

/// Circular ring buffer with absolute sequence cursor tracking.
struct RingBuffer {
    data: VecDeque<u8>,
    head_cursor: usize,
    max_cap: usize,
}

impl RingBuffer {
    fn new(max_cap: usize) -> Self {
        Self {
            data: VecDeque::new(),
            head_cursor: 0,
            max_cap,
        }
    }

    /// 追加字节；返回**因容量上限被丢弃的字节数**（契约 CT-P7b：promoted 路径据此置
    /// truncated 标记，禁止静默丢字节）。
    fn push(&mut self, chunk: &[u8]) -> usize {
        let mut dropped = 0usize;
        for &b in chunk {
            if self.data.len() >= self.max_cap {
                self.data.pop_front();
                self.head_cursor = self.head_cursor.saturating_add(1);
                dropped = dropped.saturating_add(1);
            }
            self.data.push_back(b);
        }
        dropped
    }

    fn total_written(&self) -> usize {
        self.head_cursor + self.data.len()
    }

    fn read_from(&self, offset: usize) -> (Vec<u8>, usize) {
        let total = self.total_written();
        if offset >= total {
            return (Vec::new(), total);
        }
        let start_idx = if offset <= self.head_cursor {
            0
        } else {
            offset - self.head_cursor
        };
        let mut slice = Vec::with_capacity(self.data.len().saturating_sub(start_idx));
        for i in start_idx..self.data.len() {
            slice.push(self.data[i]);
        }
        (slice, total)
    }
}

/// 后台 job 的来源（契约 v4 / B005）。
enum JobKind {
    /// [`job_start`] 自建作业：本结构持有 `Child` 所有权，状态靠 `try_wait`。
    Owned,
    /// Run 到期 promote 而来：进程所有权仍在 [`ProcessManager`]，此处只持租借三元组，
    /// 状态与增量输出靠 `manager.poll()`（`poll` 取走即清空，须持续追加进 ring buffer）。
    Promoted {
        manager: ProcessManager,
        session_id: String,
        handle: String,
    },
}

impl JobKind {
    fn label(&self) -> &'static str {
        match self {
            Self::Owned => "owned",
            Self::Promoted { .. } => "promoted",
        }
    }
}

/// Internal job session representation.
struct JobSession {
    job_id: String,
    command_line: String,
    pid: u32,
    started_at: u64,
    child: Mutex<Option<Child>>,
    buffer: Arc<Mutex<RingBuffer>>,
    status: Mutex<String>, // "running", "completed", "failed", "killed"
    exit_code: Mutex<Option<i32>>,
    is_alive: Arc<AtomicBool>,
    deadline: Option<Instant>,
    /// promoted job 的粘性截断标记（`job_start` 自建作业恒 false，不对外声称）。
    stdout_truncated: AtomicBool,
    stderr_truncated: AtomicBool,
    /// B008：promoted 作业进入终态后，`ProcessManager` 里的条目是否已回收（一次性开关）。
    released: AtomicBool,
    /// B012：作业归属的会话 id（`job_start` / promote 时的 `ToolExecutionContext.session_id`）。
    /// `None` = 无归属（内部直调 / 未提供上下文），对任意 caller 放行。
    owner: Option<String>,
    /// B013：promote 时移交的临时脚本（Windows `<ws>/.tmp/pony-agent-run-{sid}.cmd`）。
    /// 前台 Run 在 promote 路径上**不得**自己删除它（进程仍可能读该脚本），清理责任移交
    /// job 生命周期：在终态回收点（[`JobSession::release_promoted_entry`]）删除，POSIX 恒 `None`。
    cleanup_script: Option<PathBuf>,
    /// B009：promoted 作业继承的取消令牌。promote 后前台 Run 的 `CommandCancelWatcher` 会随函数
    /// 返回被 drop，用户 Stop 就再也停不下这条命令；watcher 线程改由此字段判定取消。
    cancel: Option<Arc<CancellationToken>>,
    kind: JobKind,
    #[cfg(windows)]
    #[allow(dead_code)]
    job: Option<std::os::windows::io::OwnedHandle>,
}

impl JobSession {
    fn update_status(&self) -> (String, Option<i32>) {
        let mut status_guard = self.status.lock().unwrap();
        if *status_guard != "running" {
            let code = *self.exit_code.lock().unwrap();
            return (status_guard.clone(), code);
        }

        // 契约 v4 / B005：promoted 作业的进程归 `ProcessManager`，此处**不再**走 `child.try_wait`
        // （本作业没有 Child 所有权）；每次状态查询都用 `poll` 取增量并追加进 ring buffer。
        if let JobKind::Promoted {
            manager,
            session_id,
            handle,
        } = &self.kind
        {
            let outcome = match manager.poll(session_id, handle) {
                Ok(result) => {
                    self.append_poll_output(&result);
                    if result.state == ProcessState::Exited {
                        let code = result.exit_code;
                        *self.exit_code.lock().unwrap() = code;
                        self.is_alive.store(false, Ordering::Release);
                        let status = if code == Some(0) {
                            "completed".to_string()
                        } else {
                            "failed".to_string()
                        };
                        *status_guard = status.clone();
                        Some((status, code))
                    } else {
                        None
                    }
                }
                Err(_) => {
                    // handle 已被回收（session shutdown / 已被 kill）：按终态收敛，已读输出保留。
                    self.is_alive.store(false, Ordering::Release);
                    *status_guard = "failed".to_string();
                    let code = *self.exit_code.lock().unwrap();
                    Some(("failed".to_string(), code))
                }
            };
            // B008：终态后回收 ProcessManager 条目。必须在**释放 `status` 锁之后**再进
            // ProcessManager（那里还会取 child/state 锁），避免跨模块持锁。
            let reached_terminal = outcome.is_some();
            drop(status_guard);
            if reached_terminal {
                self.release_promoted_entry(manager, session_id, handle);
            }
            return outcome.unwrap_or_else(|| ("running".to_string(), None));
        }

        // Check if deadline passed
        if let Some(dl) = self.deadline {
            if Instant::now() >= dl {
                // Timeout reached, kill process
                let mut child_guard = self.child.lock().unwrap();
                self.kill_internal_locked(&mut child_guard, &mut status_guard, Some("timeout"));
                *status_guard = "failed".to_string();
                return ("failed".to_string(), *self.exit_code.lock().unwrap());
            }
        }

        let mut child_guard = self.child.lock().unwrap();
        if let Some(child) = child_guard.as_mut() {
            match child.try_wait() {
                Ok(Some(exit_status)) => {
                    self.is_alive.store(false, Ordering::Release);
                    let code = exit_status.code();
                    *self.exit_code.lock().unwrap() = code;
                    let st = if exit_status.success() {
                        "completed".to_string()
                    } else {
                        "failed".to_string()
                    };
                    *status_guard = st.clone();
                    (st, code)
                }
                Ok(None) => ("running".to_string(), None),
                Err(_) => {
                    self.is_alive.store(false, Ordering::Release);
                    *status_guard = "failed".to_string();
                    ("failed".to_string(), None)
                }
            }
        } else {
            (status_guard.clone(), *self.exit_code.lock().unwrap())
        }
    }

    /// 把一次 `poll` 的增量输出与截断标记并入本作业的 ring buffer（promoted 路径专用）。
    ///
    /// 契约 CT-P7b：ring buffer 自身的容量溢出也算截断 —— `push` 返回被丢弃字节数，
    /// 非零即置对应流的 truncated 标记，`job_output` 会以 `stdoutTruncated/stderrTruncated`
    /// 透传，绝不静默丢字节。
    fn append_poll_output(&self, result: &ProcessPollResult) {
        {
            let mut buffer = self.buffer.lock().unwrap();
            if !result.stdout.is_empty() && buffer.push(result.stdout.as_bytes()) > 0 {
                self.stdout_truncated.store(true, Ordering::Release);
            }
            if !result.stderr.is_empty() && buffer.push(result.stderr.as_bytes()) > 0 {
                self.stderr_truncated.store(true, Ordering::Release);
            }
        }
        if result.stdout_truncated {
            self.stdout_truncated.store(true, Ordering::Release);
        }
        if result.stderr_truncated {
            self.stderr_truncated.store(true, Ordering::Release);
        }
    }

    /// B008：promoted 作业进入终态后回收 `ProcessManager` 里的条目。
    ///
    /// 复用既有回收机制：`ProcessManager::kill_inner` 对「已退出」的条目会移除 entry
    /// （**不新造 release API**）。`released.swap(true, AcqRel)` 保证只调一次；调用方必须在
    /// **释放 `status` 锁之后**调用，避免持锁跨模块（ProcessManager 还要取 child/state 锁）。
    /// 回收失败不改变作业语义：ring buffer 里已读内容不受影响，`job_output` 仍可读到终态输出。
    fn release_promoted_entry(&self, manager: &ProcessManager, session_id: &str, handle: &str) {
        if self.released.swap(true, Ordering::AcqRel) {
            return;
        }
        // B008（Windows 实测）：`cancel` 之后子进程的退出状态是**异步**可见的，首次
        // `kill_if_exists` 可能仍走「running ⇒ 保留条目」分支，导致条目泄漏（CI Windows 实测
        // `tracked=true`）。故做有界重试；最终仍失败只 warn，不改变作业语义。
        let mut released = false;
        for _ in 0..20 {
            if let Err(error) = manager.kill_if_exists(session_id, handle) {
                eprintln!("[jobs] promoted entry release failed for `{handle}`: {error}");
            }
            if !manager.is_tracked(session_id, handle) {
                released = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if !released {
            eprintln!("[jobs] promoted entry still tracked after release retries: `{handle}`");
        }
        // B013：终态之后才删临时脚本（promote 路径前台不删，进程此前可能仍在读它）。
        // 失败**只**打 warn：不改 status、不 panic、不把 killed 变 failed，也不影响 ring buffer。
        if let Some(path) = self.cleanup_script.as_deref() {
            if let Err(error) = std::fs::remove_file(path) {
                eprintln!(
                    "[jobs] promoted cleanup script removal failed for `{}`: {error}",
                    path.display()
                );
            }
        }
    }

    fn kill_internal(&self, reason: Option<&str>) -> bool {
        // 契约 v4 / B005：promoted 作业的终止走 `ProcessManager::cancel`（自有进程树语义），
        // 不走 `child.kill`（此处无 Child 所有权）。
        if let JobKind::Promoted {
            manager,
            session_id,
            handle,
        } = &self.kind
        {
            let mut status_guard = self.status.lock().unwrap();
            if *status_guard != "running" {
                return false;
            }
            let _ = manager.cancel(session_id, handle);
            // 取一次终态与残余输出作证据（进程已被终止；错误忽略，状态由下面强制收敛）。
            if let Ok(result) = manager.poll(session_id, handle) {
                self.append_poll_output(&result);
                *self.exit_code.lock().unwrap() = result.exit_code;
            }
            self.is_alive.store(false, Ordering::Release);
            *status_guard = "killed".to_string();
            // B008：kill 后同步回收条目（放锁后再进 ProcessManager）。
            drop(status_guard);
            self.release_promoted_entry(manager, session_id, handle);
            return true;
        }

        let mut child_guard = self.child.lock().unwrap();
        let mut status_guard = self.status.lock().unwrap();
        self.kill_internal_locked(&mut child_guard, &mut status_guard, reason)
    }

    fn kill_internal_locked(
        &self,
        child_guard: &mut Option<Child>,
        status_guard: &mut String,
        _reason: Option<&str>,
    ) -> bool {
        self.is_alive.store(false, Ordering::Release);

        #[cfg(unix)]
        {
            let pgid = self.pid as i32;
            unsafe {
                let _ = libc::killpg(pgid, libc::SIGKILL);
                let _ = libc::kill(self.pid as i32, libc::SIGKILL);
            }
        }

        #[cfg(windows)]
        {
            if let Some(child) = child_guard.as_mut() {
                let _ = child.kill();
            }
        }

        if let Some(mut child) = child_guard.take() {
            let _ = child.kill();
            // Reap with bounded wait to avoid hanging if process or group termination has delay
            let start = Instant::now();
            let timeout = Duration::from_millis(500);
            let mut reaped = false;
            while start.elapsed() < timeout {
                if let Ok(Some(status)) = child.try_wait() {
                    *self.exit_code.lock().unwrap() = status.code().or(Some(137));
                    reaped = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if !reaped {
                let _ = child.kill();
                let status = child.wait().ok();
                *self.exit_code.lock().unwrap() = status.and_then(|s| s.code()).or(Some(137));
            }
        }

        if *status_guard == "running" {
            *status_guard = "killed".to_string();
        }
        true
    }
}

static REGISTRY: OnceLock<Mutex<JobRegistry>> = OnceLock::new();

struct JobRegistry {
    active_jobs: HashMap<String, Arc<JobSession>>,
    order: VecDeque<String>,
}

impl JobRegistry {
    fn new() -> Self {
        Self {
            active_jobs: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn insert(&mut self, job_id: String, session: Arc<JobSession>) {
        self.active_jobs.insert(job_id.clone(), session);
        self.order.push_back(job_id);

        // Enforce max history jobs retention
        while self.order.len() > MAX_HISTORY_JOBS {
            if let Some(old_id) = self.order.pop_front() {
                // If it is dead/finished, remove it from active_jobs
                if let Some(s) = self.active_jobs.get(&old_id) {
                    let (st, _) = s.update_status();
                    if st != "running" {
                        self.active_jobs.remove(&old_id);
                    } else {
                        // Keep running jobs
                        self.order.push_back(old_id);
                        break;
                    }
                }
            }
        }
    }

    fn get(&self, job_id: &str) -> Option<Arc<JobSession>> {
        self.active_jobs.get(job_id).cloned()
    }

    pub fn list(&self) -> Vec<Arc<JobSession>> {
        self.order
            .iter()
            .filter_map(|job_id| self.active_jobs.get(job_id).cloned())
            .collect()
    }
}

fn registry() -> &'static Mutex<JobRegistry> {
    REGISTRY.get_or_init(|| Mutex::new(JobRegistry::new()))
}

/// Session-anchored cwd validator (fail-closed on traversal / outside-anchor).
/// `workspace_root=Some(ws)` pins resolution to the session workspace;
/// `None` falls back to `compute_default_workspace_root()` → process cwd (legacy/compat path).
fn validate_and_resolve_cwd_with_root(
    cwd_opt: Option<&str>,
    workspace_root: Option<&Path>,
) -> Result<PathBuf, String> {
    let (join_base, canonical_workspace) = resolve_cwd_anchor(workspace_root)?;

    let path_str = match cwd_opt {
        Some(s) if !s.trim().is_empty() => s.trim(),
        _ => return Ok(canonical_workspace),
    };

    let path = Path::new(path_str);
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        join_base.join(path)
    };

    if !candidate.exists() || !candidate.is_dir() {
        return Err(format!("cwd does not exist or is not a directory: {path_str}"));
    }

    let canonical = candidate
        .canonicalize()
        .map_err(|e| format!("invalid cwd: {e}"))?;

    if !canonical.starts_with(&canonical_workspace) {
        return Err(format!("Cwd outside workspace or traversal denied: {path_str}"));
    }

    Ok(canonical)
}

/// Resolves the cwd anchor: returns `(join_base, canonical_anchor)`.
fn resolve_cwd_anchor(workspace_root: Option<&Path>) -> Result<(PathBuf, PathBuf), String> {
    if let Some(ws) = workspace_root {
        let base = ws.to_path_buf();
        let canonical = base
            .canonicalize()
            .map_err(|e| format!("canonicalize workspace root: {e}"))?;
        return Ok((base, canonical));
    }
    // Legacy/compat anchor (side-effect free): compute_default path is used as-is when it
    // already exists; missing dir falls back to process cwd. Directory creation belongs
    // solely to bootstrap_default_workspace — this path must never touch ~/pony_agent.
    if let Some(def) = crate::agent::workspace::compute_default_workspace_root() {
        if let Ok(canonical) = def.canonicalize() {
            return Ok((def, canonical));
        }
    }
    let current_dir = std::env::current_dir().map_err(|e| format!("current_dir failed: {e}"))?;
    let canonical_workspace = current_dir
        .canonicalize()
        .map_err(|e| format!("canonicalize current dir: {e}"))?;
    Ok((current_dir, canonical_workspace))
}

pub fn job_start(args: JobStartArgs) -> Result<JobStartResult, String> {
    job_start_with_workspace_root(args, None)
}

/// Session-anchored variant (TASK-2-WIRE target): `workspace_root=Some(ws)` pins the default
/// cwd and the boundary check to the session workspace root.
/// 无归属的 `job_start_with_workspace_root`（内部/单测直调路径，等价 `owner = None`）。
///
/// 契约 B012 只约束**工具路径**；既有直接调用方（内部逻辑、冻结集成测试）保持原签名不受限。
pub fn job_start_with_workspace_root(
    args: JobStartArgs,
    workspace_root: Option<&Path>,
) -> Result<JobStartResult, String> {
    job_start_scoped(args, workspace_root, None)
}

/// 带会话归属的 `job_start`（工具路径用）：`owner` 记入 `JobSession.owner`，供后续
/// `job_output_scoped` / `job_kill_scoped` / `job_list_scoped` 做归属校验。
pub fn job_start_scoped(
    args: JobStartArgs,
    workspace_root: Option<&Path>,
    owner: Option<&str>,
) -> Result<JobStartResult, String> {
    let cwd = validate_and_resolve_cwd_with_root(args.cwd.as_deref(), workspace_root)?;

    let mut command = Command::new(&args.command);
    let command_line = if let Some(ref cmd_args) = args.args {
        command.args(cmd_args);
        format!("{} {}", args.command, cmd_args.join(" "))
    } else {
        args.command.clone()
    };

    command.current_dir(&cwd);
    command.stdin(Stdio::null());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    // Windows 弹窗修复：控制台子进程不得弹出可见控制台窗口（与 Run/ProcessManager 一致）。
    #[cfg(windows)]
    crate::agent::process::hide_console_window(&mut command);

    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            // Put child in its own process group to reap descendants cleanly
            libc::setpgid(0, 0);
            Ok(())
        });
    }

    #[cfg(windows)]
    let job = {
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, SetInformationJobObject,
            JobObjectExtendedLimitInformation, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        use std::os::windows::io::FromRawHandle;

        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if !raw.is_null() && raw != INVALID_HANDLE_VALUE {
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            unsafe {
                SetInformationJobObject(
                    raw,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as *const _,
                    std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
                );
            }
            Some(unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(raw) })
        } else {
            None
        }
    };

    let mut child = command
        .spawn()
        .map_err(|e| format!("Failed to spawn `{}`: {e}", args.command))?;
    let pid = child.id();

    #[cfg(windows)]
    if let Some(job_handle) = &job {
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        unsafe {
            AssignProcessToJobObject(job_handle.as_raw_handle(), child.as_raw_handle());
        }
    }

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let ring_buffer = Arc::new(Mutex::new(RingBuffer::new(JOB_BUFFER_CAP)));
    let is_alive = Arc::new(AtomicBool::new(true));

    if let Some(mut out) = stdout {
        let buf = Arc::clone(&ring_buffer);
        std::thread::Builder::new()
            .name("job-stdout-drain".to_string())
            .spawn(move || {
                let mut temp = [0u8; 4096];
                loop {
                    match out.read(&mut temp) {
                        Ok(0) => break,
                        Ok(n) => {
                            // owned 作业不追踪截断（CT-P7b 只要求 promoted 路径透传）。
                            let _ = buf.lock().unwrap().push(&temp[..n]);
                        }
                        Err(_) => break,
                    }
                }
            })
            .ok();
    }

    if let Some(mut err) = stderr {
        let buf = Arc::clone(&ring_buffer);
        std::thread::Builder::new()
            .name("job-stderr-drain".to_string())
            .spawn(move || {
                let mut temp = [0u8; 4096];
                loop {
                    match err.read(&mut temp) {
                        Ok(0) => break,
                        Ok(n) => {
                            // owned 作业不追踪截断（CT-P7b 只要求 promoted 路径透传）。
                            let _ = buf.lock().unwrap().push(&temp[..n]);
                        }
                        Err(_) => break,
                    }
                }
            })
            .ok();
    }

    let job_id = uuid::Uuid::new_v4().to_string();
    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let deadline = args
        .timeout_ms
        .map(|ms| Instant::now() + Duration::from_millis(ms));

    let session = Arc::new(JobSession {
        job_id: job_id.clone(),
        command_line,
        pid,
        started_at,
        child: Mutex::new(Some(child)),
        buffer: ring_buffer,
        status: Mutex::new("running".to_string()),
        exit_code: Mutex::new(None),
        is_alive,
        deadline,
        stdout_truncated: AtomicBool::new(false),
        stderr_truncated: AtomicBool::new(false),
        released: AtomicBool::new(false),
        owner: owner.map(str::to_string),
        cleanup_script: None,
        cancel: None,
        kind: JobKind::Owned,
        #[cfg(windows)]
        job,
    });

    registry().lock().unwrap().insert(job_id.clone(), session);

    Ok(JobStartResult {
        job_id,
        pid,
        status: "running".to_string(),
    })
}

/// 契约 v4 / B005：把一个仍在前台 `Run` 里运行的进程登记为后台 promoted 作业。
///
/// 设计 §1：不搬移 `Child` 所有权，jobs 侧只持 `(ProcessManager, session_id, handle)` 租借
/// 三元组；后续输出靠「首读快照 + 每次 `poll` 追加」。`first_stdout` / `first_stderr` 是
/// promote 时刻已经累计读到的输出，`*_truncated` 是对应流的粘性截断标记（设计 §4 要求透传）。
///
/// 失败（registry 不可用等）返回 `Err`，调用方负责回落到既有「kill + timeout 错误」路径。
/// B009：`cancel` 是前台 Run 继承的 turn 取消令牌。promote 之后前台 Run 的
/// `CommandCancelWatcher` 随函数返回被 drop，用户 Stop 再也停不下命令；因此注册时额外 spawn
/// 一个**短生命周期** watcher 线程（20ms tick）：令牌取消 ⇒ `kill_internal` 落 `killed`；
/// 作业进入终态（或 `JobSession` 被回收）⇒ 线程立即退出，禁止常驻泄漏。
// 参数个数由契约可测接缝固定（manager + 租借三元组 + 命令行 + 首读快照 + 两个截断标记 +
// 取消令牌 + owner + 待清理脚本）；不合并成结构体是为了让接缝与调用点逐字对应契约签名，避免额外类型。
#[allow(clippy::too_many_arguments)]
pub fn register_promoted(
    manager: ProcessManager,
    session_id: String,
    handle: String,
    command_line: String,
    first_stdout: &str,
    first_stderr: &str,
    stdout_truncated: bool,
    stderr_truncated: bool,
    cancel: Option<Arc<CancellationToken>>,
    owner: Option<&str>,
    cleanup_script: Option<PathBuf>,
) -> Result<String, String> {
    let buffer = Arc::new(Mutex::new(RingBuffer::new(JOB_BUFFER_CAP)));
    // 契约 CT-P7b：首读快照若本身已超过 ring buffer 容量，同样置 truncated 标记。
    let mut stdout_truncated = stdout_truncated;
    let mut stderr_truncated = stderr_truncated;
    {
        let mut buffer_guard = buffer.lock().unwrap();
        if !first_stdout.is_empty() && buffer_guard.push(first_stdout.as_bytes()) > 0 {
            stdout_truncated = true;
        }
        if !first_stderr.is_empty() && buffer_guard.push(first_stderr.as_bytes()) > 0 {
            stderr_truncated = true;
        }
    }

    let job_id = uuid::Uuid::new_v4().to_string();
    let started_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let session = Arc::new(JobSession {
        job_id: job_id.clone(),
        command_line,
        // promoted 作业的进程 pid 未单独暴露（所有权仍在 ProcessManager），固定为 0。
        pid: 0,
        started_at,
        // 无 Child 所有权：promoted 分支不读该字段。
        child: Mutex::new(None),
        buffer,
        status: Mutex::new("running".to_string()),
        exit_code: Mutex::new(None),
        is_alive: Arc::new(AtomicBool::new(true)),
        // promote 语义：到期不杀、无作业级 deadline（超时已由前台 Run 承担）。
        deadline: None,
        stdout_truncated: AtomicBool::new(stdout_truncated),
        stderr_truncated: AtomicBool::new(stderr_truncated),
        released: AtomicBool::new(false),
        owner: owner.map(str::to_string),
        cleanup_script,
        cancel: cancel.clone(),
        kind: JobKind::Promoted {
            manager,
            session_id,
            handle,
        },
        #[cfg(windows)]
        job: None,
    });

    registry().lock().unwrap().insert(job_id.clone(), session.clone());
    spawn_cancel_watcher(Arc::downgrade(&session));
    Ok(job_id)
}

/// B009：promoted 作业的取消观察者（短生命周期，20ms tick）。
///
/// 退出条件（三者任一即退出，保证不常驻）：
/// 1. `JobSession` 已被回收（`Weak::upgrade` 失败）；
/// 2. 作业已进入终态（`status != "running"`，含被 `job_kill` 或自然结束）；
/// 3. 令牌已取消 ⇒ 调 [`JobSession::kill_internal`] 落 `killed` 后退出。
fn spawn_cancel_watcher(weak: std::sync::Weak<JobSession>) {
    std::thread::Builder::new()
        .name("promoted-cancel-watch".to_string())
        .spawn(move || {
            const TICK: Duration = Duration::from_millis(20);
            loop {
                let Some(session) = weak.upgrade() else {
                    return;
                };
                let cancelled = session
                    .cancel
                    .as_ref()
                    .is_some_and(|token| token.is_cancelled());
                let status = session.status.lock().unwrap().clone();
                if status != "running" {
                    return;
                }
                if cancelled {
                    let _ = session.kill_internal(Some("cancelled"));
                    return;
                }
                drop(session);
                std::thread::sleep(TICK);
            }
        })
        .ok();
}

/// 无归属校验的 `job_output`（内部/单测直调路径，等价 `caller = None`）。
///
/// 契约 B012 只约束**工具路径**；既有直接调用方（内部逻辑、冻结集成测试）保持原签名不受限。
pub fn job_output(args: JobOutputArgs) -> Result<JobOutputResult, String> {
    job_output_scoped(args, None)
}

/// 带调用方归属的 `job_output`（工具路径用）：`caller` 与作业 owner 都有值且不等 ⇒
/// `job_not_owned`，且**在校验通过前不读任何输出**。
pub fn job_output_scoped(
    args: JobOutputArgs,
    caller: Option<&str>,
) -> Result<JobOutputResult, String> {
    let session = registry()
        .lock()
        .unwrap()
        .get(&args.job_id)
        .ok_or_else(|| format!("Job `{}` not found", args.job_id))?;
    // 契约 B012：归属校验先于任何输出读取（拒绝响应不得夹带作业输出）。
    ensure_owned(&session, caller)?;

    let should_wait = args.wait.unwrap_or(false);
    let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(30_000));
    let offset = args.offset.unwrap_or(0);
    let start = Instant::now();

    loop {
        let (status, exit_code) = session.update_status();

        let is_running = status == "running";

        if !should_wait || !is_running || start.elapsed() >= timeout {
            // Drain output
            let (slice, new_cursor) = session.buffer.lock().unwrap().read_from(offset);
            let output = crate::agent::process::decode_process_output(&slice);

            // promoted 作业透传粘性截断证据；`job_start` 自建作业不追踪截断（None）。
            let (stdout_truncated, stderr_truncated) = match session.kind {
                JobKind::Promoted { .. } => (
                    Some(session.stdout_truncated.load(Ordering::Acquire)),
                    Some(session.stderr_truncated.load(Ordering::Acquire)),
                ),
                JobKind::Owned => (None, None),
            };

            return Ok(JobOutputResult {
                job_id: session.job_id.clone(),
                status,
                output,
                cursor: new_cursor,
                exit_code,
                stdout_truncated,
                stderr_truncated,
            });
        }

        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 无归属校验的 `job_kill`（内部/单测直调路径，等价 `caller = None`）。
pub fn job_kill(args: JobKillArgs) -> Result<JobKillResult, String> {
    job_kill_scoped(args, None)
}

/// 带调用方归属的 `job_kill`（工具路径用）：归属校验**必须早于** `kill_internal`，
/// 否则就是「拒了但照杀」。
pub fn job_kill_scoped(args: JobKillArgs, caller: Option<&str>) -> Result<JobKillResult, String> {
    let session = registry()
        .lock()
        .unwrap()
        .get(&args.job_id)
        .ok_or_else(|| format!("Job `{}` not found", args.job_id))?;
    // 契约 B012：归属校验**必须在 kill_internal 之前**，否则就是「拒了但照杀」。
    ensure_owned(&session, caller)?;

    let killed = session.kill_internal(args.reason.as_deref());

    Ok(JobKillResult {
        job_id: args.job_id,
        killed,
    })
}

/// 无归属过滤的 `job_list`（内部/单测直调路径，等价 `caller = None`）。
pub fn job_list(_args: JobListArgs) -> Result<JobListResult, String> {
    job_list_scoped(_args, None)
}

/// 带调用方归属的 `job_list`（工具路径用）：非 owner 的作业被**过滤掉**（不暴露存在性）。
pub fn job_list_scoped(_args: JobListArgs, caller: Option<&str>) -> Result<JobListResult, String> {
    let sessions = registry().lock().unwrap().list();
    let jobs = sessions
        .into_iter()
        // 契约 B012：非 owner 的作业**直接过滤掉**（列表接口泄露存在性同样算泄露），
        // 而不是返回错误 —— 错误会让调用方区分「不存在」与「存在但无权」。
        .filter(|session| ensure_owned(session, caller).is_ok())
        .map(|session| {
            let (status, _) = session.update_status();
            JobListItem {
                job_id: session.job_id.clone(),
                command: session.command_line.clone(),
                status,
                pid: session.pid,
                started_at: session.started_at,
                kind: session.kind.label().to_string(),
            }
        })
        .collect();
    Ok(JobListResult { jobs })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tool_runtime::{ProcessStartRequest, SandboxRequest};

    /// 被 promote 的作业在进入终态后，必须把 `ProcessManager` 里的条目回收掉（B008）。
    ///
    /// 断言口径：`ProcessManager::is_tracked` —— 回收只删条目，不影响 ring buffer 里已读内容
    /// （`job_output` 仍能读到终态输出，由 `run_timeout_promote_test` 的 P2/P3/P5 覆盖）。
    fn sandbox_request() -> SandboxRequest {
        SandboxRequest {
            workspace_root: ".".to_string(),
            allow_network: false,
            environment_allowlist: Vec::new(),
            isolate_environment: false,
        }
    }

    fn start(manager: &ProcessManager, session_id: &str, program: &str, args: &[&str]) -> String {
        manager
            .start(&ProcessStartRequest {
                session_id: session_id.to_string(),
                program: program.to_string(),
                arguments: args.iter().map(|arg| arg.to_string()).collect(),
                sandbox: sandbox_request(),
            })
            .expect("start must succeed")
    }

    /// 轮询进程直到终态（或超时），返回终态那次 poll 的结果（含最后一段输出）。
    /// 轮询到进程退出，**累积**每次 poll 的输出（`manager.poll` 每次都会排空缓冲区）。
    ///
    /// 为什么必须累积：`poll` 是「取走」语义——若某次 poll 看到 `Running`（进程还没退出）
    /// 但它已经把此前的 stdout 排空返回，那么丢弃这次结果就等于**丢输出**。旧实现在这里
    /// 直接丢弃中间结果，只在「恰好由终态那次 poll 拿到输出」时才正确；Linux 上时序通常如此，
    /// Windows CI 上 `cmd /C echo done` 更快，输出被一次 `Running` 的 poll 取走 ⇒ 断言
    /// `output=""` 失败（jobs.rs:1091）。
    fn poll_until_exited(
        manager: &ProcessManager,
        session_id: &str,
        handle: &str,
    ) -> Option<ProcessPollResult> {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stdout = String::new();
        let mut stderr = String::new();
        let mut stdout_truncated = false;
        let mut stderr_truncated = false;
        while Instant::now() < deadline {
            let result = manager.poll(session_id, handle).expect("poll must succeed");
            stdout.push_str(&result.stdout);
            stderr.push_str(&result.stderr);
            stdout_truncated |= result.stdout_truncated;
            stderr_truncated |= result.stderr_truncated;
            if result.state == ProcessState::Exited {
                return Some(ProcessPollResult {
                    state: result.state,
                    exit_code: result.exit_code,
                    stdout,
                    stderr,
                    stdout_truncated,
                    stderr_truncated,
                });
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        None
    }

    /// 自然结束的 promoted 作业：`update_status` 检出终态后条目被回收。
    #[test]
    fn promoted_job_releases_manager_entry_after_natural_exit() {
        let manager = ProcessManager::new();
        let session_id = "b008-natural";
        #[cfg(windows)]
        let handle = start(&manager, session_id, "cmd", &["/C", "echo done"]);
        #[cfg(not(windows))]
        let handle = start(&manager, session_id, "sh", &["-c", "echo done"]);

        let first_read = poll_until_exited(&manager, session_id, &handle)
            .expect("helper command must exit before the deadline");

        // 用终态那次 poll 的输出作为 promote 首读快照（真实链路里快照来自 Run 的 poll 累积）。
        let job_id = register_promoted(
            manager.clone(),
            session_id.to_string(),
            handle.clone(),
            "echo done".to_string(),
            &first_read.stdout,
            &first_read.stderr,
            first_read.stdout_truncated,
            first_read.stderr_truncated,
            None,
            None,
            None,
        )
        .expect("register_promoted must succeed");
        assert!(
            manager.is_tracked(session_id, &handle),
            "注册后条目应仍被 ProcessManager 追踪"
        );

        // 驱动到终态：update_status → poll 看到 Exited → 置 completed → 回收条目。
        // 终态读取：CI 实测存在「进程刚退出、管道尚未排空」的窄窗（首读快照为空），
        // 故按 offset=0 做有界重试，仍为空才判失败（不放过真实的数据丢失）。
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut out = job_output(JobOutputArgs {
            job_id: job_id.clone(),
            wait: Some(true),
            timeout_ms: Some(5_000),
            offset: None,
        })
        .expect("job_output must succeed");
        while out.output.is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
            out = job_output(JobOutputArgs {
                job_id: job_id.clone(),
                wait: Some(false),
                timeout_ms: Some(0),
                offset: Some(0),
            })
            .expect("job_output must succeed");
        }
        assert_eq!(out.status, "completed", "natural exit must converge to completed");
        assert_eq!(out.exit_code, Some(0), "exit code must be preserved");
        assert!(
            out.output.contains("done"),
            "回收不得影响已读输出，实际 output={:?}",
            out.output
        );
        let tracked_after_terminal = manager.is_tracked(session_id, &handle);
        eprintln!(
            "[b008] natural exit: tracked_after_register=true tracked_after_terminal={tracked_after_terminal}"
        );
        assert!(
            !tracked_after_terminal,
            "B008：promoted 作业终态后 ProcessManager 条目必须被回收（不得残留泄漏）"
        );

        // 幂等：再读一次终态仍可读（回收只删 ProcessManager 条目，ring buffer 不变）。
        let again = job_output(JobOutputArgs {
            job_id,
            wait: Some(false),
            timeout_ms: Some(0),
            offset: None,
        })
        .expect("job_output after release must still succeed");
        assert_eq!(again.status, "completed", "回收后仍须读到终态状态");
        assert!(
            again.output.contains("done"),
            "回收后 ring buffer 内容必须保留，实际 output={:?}",
            again.output
        );
    }

    /// B009：promote 后取消令牌生效 —— watcher 终止进程组、作业落 `killed`、条目被回收。
    ///
    /// 同时输出物理证据：cancel→终态耗时、进程组（pgid）是否已无存活成员。
    #[test]
    fn promoted_job_cancel_token_terminates_process_group() {
        let manager = ProcessManager::new();
        let session_id = "b009-cancel";
        #[cfg(windows)]
        let handle = start(
            &manager,
            session_id,
            "cmd",
            &["/C", "ping -n 6 127.0.0.1 >nul & echo done"],
        );
        #[cfg(not(windows))]
        let handle = start(&manager, session_id, "sh", &["-c", "sleep 5; echo done"]);

        // 进程组长的 pid（= pgid）：必须在 cancel 之前取，回收后条目就没了。
        let pgid = manager.pid_of(session_id, &handle).expect("pid_of");

        let token = Arc::new(CancellationToken::new());
        let job_id = register_promoted(
            manager.clone(),
            session_id.to_string(),
            handle.clone(),
            "cancel watch".to_string(),
            "",
            "",
            false,
            false,
            Some(token.clone()),
            None,
            None,
        )
        .expect("register_promoted must succeed");

        let before = job_output(JobOutputArgs {
            job_id: job_id.clone(),
            wait: Some(false),
            timeout_ms: Some(0),
            offset: None,
        })
        .expect("job_output before cancel");
        assert_eq!(before.status, "running", "cancel 之前作业必须仍在运行");

        // Act：等价于用户 Stop。
        let started = Instant::now();
        token.cancel();
        let mut status = "running".to_string();
        while started.elapsed() < Duration::from_secs(1) {
            status = job_output(JobOutputArgs {
                job_id: job_id.clone(),
                wait: Some(false),
                timeout_ms: Some(0),
                offset: None,
            })
            .expect("job_output after cancel")
            .status;
            if status != "running" {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let elapsed = started.elapsed();

        #[cfg(unix)]
        let pgid_alive = {
            // SAFETY：只发信号 0（探针），不读写内存。
            let rc = unsafe { libc::killpg(pgid as i32, 0) };
            rc == 0
        };
        #[cfg(windows)]
        let pgid_alive = false; // Windows 走 Job Object（kill-on-close），无 POSIX 进程组。
        eprintln!(
            "[b009] cancel → status={status} in {elapsed:?}; pgid={pgid} alive={pgid_alive}; tracked={}",
            manager.is_tracked(session_id, &handle)
        );

        assert_ne!(status, "running", "B009：cancel 后 1s 内作业必须离开 running");
        assert_eq!(status, "killed", "B009：取消后的终态必须是 killed");
        assert!(
            !pgid_alive,
            "B009：cancel 后进程组（pgid={pgid}）内不得再有存活进程（否则孙进程变孤儿继续跑）"
        );
        assert!(
            !manager.is_tracked(session_id, &handle),
            "B009/B008：cancel 后 ProcessManager 条目必须被回收"
        );
    }

    /// 被 `job_kill` 终止的 promoted 作业：kill 路径同样回收条目。
    #[test]
    fn promoted_job_releases_manager_entry_after_job_kill() {
        let manager = ProcessManager::new();
        let session_id = "b008-killed";
        #[cfg(windows)]
        let handle = start(&manager, session_id, "cmd", &["/C", "ping -n 6 127.0.0.1 >nul"]);
        #[cfg(not(windows))]
        let handle = start(&manager, session_id, "sh", &["-c", "sleep 5"]);

        let job_id = register_promoted(
            manager.clone(),
            session_id.to_string(),
            handle.clone(),
            "long running".to_string(),
            "",
            "",
            false,
            false,
            None,
            None,
            None,
        )
        .expect("register_promoted must succeed");

        let killed = job_kill(JobKillArgs {
            job_id: job_id.clone(),
            reason: Some("b008".to_string()),
        })
        .expect("job_kill must succeed");
        assert!(killed.killed, "running promoted job must report killed=true");
        let tracked_after_kill = manager.is_tracked(session_id, &handle);
        eprintln!("[b008] job_kill: tracked_after_kill={tracked_after_kill}");
        assert!(
            !tracked_after_kill,
            "B008：promoted 作业被 kill 后 ProcessManager 条目必须被回收（不得残留泄漏）"
        );

        let out = job_output(JobOutputArgs {
            job_id,
            wait: Some(false),
            timeout_ms: Some(0),
            offset: None,
        })
        .expect("job_output after kill must still succeed");
        assert_eq!(out.status, "killed", "kill 后仍必须能读到终态状态");
    }
}
