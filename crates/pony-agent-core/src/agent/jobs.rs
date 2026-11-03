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
            return match manager.poll(session_id, handle) {
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
                        (status, code)
                    } else {
                        ("running".to_string(), None)
                    }
                }
                Err(_) => {
                    // handle 已被回收（session shutdown / 已被 kill）：按终态收敛，已读输出保留。
                    self.is_alive.store(false, Ordering::Release);
                    *status_guard = "failed".to_string();
                    let code = *self.exit_code.lock().unwrap();
                    ("failed".to_string(), code)
                }
            };
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
pub fn job_start_with_workspace_root(
    args: JobStartArgs,
    workspace_root: Option<&Path>,
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
// 参数个数由 B005 契约的可测接缝固定（manager + 租借三元组 + 命令行 + 首读快照 + 两个截断
// 标记 = 8）；不合并成结构体是为了让接缝与调用点逐字对应契约签名，避免额外类型。
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
        kind: JobKind::Promoted {
            manager,
            session_id,
            handle,
        },
        #[cfg(windows)]
        job: None,
    });

    registry().lock().unwrap().insert(job_id.clone(), session);
    Ok(job_id)
}

pub fn job_output(args: JobOutputArgs) -> Result<JobOutputResult, String> {
    let session = registry()
        .lock()
        .unwrap()
        .get(&args.job_id)
        .ok_or_else(|| format!("Job `{}` not found", args.job_id))?;

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

pub fn job_kill(args: JobKillArgs) -> Result<JobKillResult, String> {
    let session = registry()
        .lock()
        .unwrap()
        .get(&args.job_id)
        .ok_or_else(|| format!("Job `{}` not found", args.job_id))?;

    let killed = session.kill_internal(args.reason.as_deref());

    Ok(JobKillResult {
        job_id: args.job_id,
        killed,
    })
}

pub fn job_list(_args: JobListArgs) -> Result<JobListResult, String> {
    let sessions = registry().lock().unwrap().list();
    let jobs = sessions
        .into_iter()
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
