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
    pub job_id: String,
    pub wait: Option<bool>,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobKillArgs {
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

    fn push(&mut self, chunk: &[u8]) {
        for &b in chunk {
            if self.data.len() >= self.max_cap {
                self.data.pop_front();
                self.head_cursor = self.head_cursor.saturating_add(1);
            }
            self.data.push_back(b);
        }
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

    fn kill_internal(&self, reason: Option<&str>) -> bool {
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
                            buf.lock().unwrap().push(&temp[..n]);
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
                            buf.lock().unwrap().push(&temp[..n]);
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

            return Ok(JobOutputResult {
                job_id: session.job_id.clone(),
                status,
                output,
                cursor: new_cursor,
                exit_code,
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
            }
        })
        .collect();
    Ok(JobListResult { jobs })
}
