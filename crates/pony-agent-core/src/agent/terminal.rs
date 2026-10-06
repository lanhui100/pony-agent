//! Interactive persistent terminal (PTY) tool family (PA-104).
//!
//! Provides `terminal_open`, `terminal_send`, `terminal_read`, `terminal_signal`,
//! and `terminal_close` tools according to PA-104 contract specifications.
//!
//! Design & Invariants:
//! - 2MB circular ring buffer per terminal session to prevent unbounded memory growth.
//! - Process tree containment:
//!   - Windows: Job Object containment (`kill-on-close` + reap)
//!   - POSIX: Process group (`setpgid(0, 0)`) with `killpg`
//! - Fail-closed path traversal validation for `cwd`.
//! - Thread-safe global registry with UUID session keys.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(unix)]
use std::os::unix::process::CommandExt;

/// Max circular buffer capacity: 2MB (AC-02).
pub const TERMINAL_BUFFER_CAP: usize = 2 * 1024 * 1024;

/// Maximum allowed active terminal sessions globally (LEAK-01).
pub const MAX_ACTIVE_TERMINALS: usize = 16;

/// Terminal signal enum matching contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TerminalSignal {
    #[serde(rename = "SIGINT")]
    Sigint,
    #[serde(rename = "SIGTERM")]
    Sigterm,
    #[serde(rename = "SIGKILL")]
    Sigkill,
    #[serde(rename = "BREAK")]
    Break,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalOpenArgs {
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub cwd: Option<String>,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub env: Option<HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalOpenResult {
    pub terminal_id: String,
    pub pid: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSendArgs {
    pub terminal_id: String,
    pub input: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSendResult {
    pub bytes_written: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalReadArgs {
    pub terminal_id: String,
    pub timeout_ms: Option<u64>,
    pub offset: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalReadResult {
    pub output: String,
    pub cursor: usize,
    pub has_more: bool,
    pub is_alive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSignalArgs {
    pub terminal_id: String,
    pub signal: TerminalSignal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalSignalResult {
    pub success: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalCloseArgs {
    pub terminal_id: String,
    pub force: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminalCloseResult {
    pub closed: bool,
    pub exit_code: Option<i32>,
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

/// Active terminal session handle.
struct TerminalSession {
    #[allow(dead_code)]
    terminal_id: String,
    pid: u32,
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    buffer: Arc<Mutex<RingBuffer>>,
    is_alive: Arc<AtomicBool>,
    exit_code: Mutex<Option<i32>>,
    #[cfg(windows)]
    #[allow(dead_code)]
    job: Option<std::os::windows::io::OwnedHandle>,
}

impl TerminalSession {
    fn check_alive(&self) -> bool {
        if !self.is_alive.load(Ordering::Acquire) {
            return false;
        }
        let mut child_guard = self.child.lock().unwrap();
        if let Some(child) = child_guard.as_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    self.is_alive.store(false, Ordering::Release);
                    *self.exit_code.lock().unwrap() = status.code();
                    false
                }
                Ok(None) => true,
                Err(_) => {
                    self.is_alive.store(false, Ordering::Release);
                    false
                }
            }
        } else {
            false
        }
    }

    fn send_signal(&self, signal: TerminalSignal) -> Result<(), String> {
        let alive = self.check_alive();
        if !alive {
            return Err("Terminal process is not running".to_string());
        }

        #[cfg(unix)]
        {
            let sig_num = match signal {
                TerminalSignal::Sigint => libc::SIGINT,
                TerminalSignal::Sigterm => libc::SIGTERM,
                TerminalSignal::Sigkill => libc::SIGKILL,
                TerminalSignal::Break => libc::SIGINT,
            };
            // Kill entire process group
            let pgid = self.pid as i32;
            let ret = unsafe { libc::killpg(pgid, sig_num) };
            if ret != 0 {
                // Fallback to single process kill
                let ret2 = unsafe { libc::kill(self.pid as i32, sig_num) };
                if ret2 != 0 {
                    return Err(format!("Failed to send signal: {}", std::io::Error::last_os_error()));
                }
            }
            Ok(())
        }

        #[cfg(windows)]
        {
            match signal {
                TerminalSignal::Sigint | TerminalSignal::Break => {
                    use windows_sys::Win32::System::Console::GenerateConsoleCtrlEvent;
                    let event = if signal == TerminalSignal::Sigint { 0 } else { 1 };
                    let ret = unsafe { GenerateConsoleCtrlEvent(event, self.pid) };
                    if ret == 0 {
                        // fallback to terminate if event fails
                        let mut child_guard = self.child.lock().unwrap();
                        if let Some(child) = child_guard.as_mut() {
                            let _ = child.kill();
                        }
                    }
                }
                TerminalSignal::Sigterm | TerminalSignal::Sigkill => {
                    let mut child_guard = self.child.lock().unwrap();
                    if let Some(child) = child_guard.as_mut() {
                        let _ = child.kill();
                    }
                }
            }
            Ok(())
        }
    }

    fn close(&self, force: bool) -> Result<Option<i32>, String> {
        let _ = force;
        // CONC-01: First send kill/term signal to process so any pending blocked stdin writes abort/unblock
        #[cfg(unix)]
        {
            let pgid = self.pid as i32;
            if force {
                let _ = unsafe { libc::killpg(pgid, libc::SIGKILL) };
                let _ = unsafe { libc::kill(self.pid as i32, libc::SIGKILL) };
            } else {
                let _ = unsafe { libc::killpg(pgid, libc::SIGTERM) };
                let _ = unsafe { libc::kill(self.pid as i32, libc::SIGTERM) };
            }
        }

        #[cfg(windows)]
        {
            let mut child_guard = self.child.lock().unwrap();
            if let Some(child) = child_guard.as_mut() {
                let _ = child.kill();
            }
        }

        // Close stdin after signaling the process
        self.stdin.lock().unwrap().take();

        let mut child_guard = self.child.lock().unwrap();
        if let Some(mut child) = child_guard.take() {
            // Reap with bounded wait
            let start = Instant::now();
            let timeout = Duration::from_millis(500);
            while start.elapsed() < timeout {
                if let Ok(Some(status)) = child.try_wait() {
                    self.is_alive.store(false, Ordering::Release);
                    let code = status.code();
                    *self.exit_code.lock().unwrap() = code;
                    return Ok(code);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            // Force kill if still lingering
            let _ = child.kill();
            let status = child.wait().ok();
            self.is_alive.store(false, Ordering::Release);
            let code = status.and_then(|s| s.code());
            *self.exit_code.lock().unwrap() = code;
            Ok(code)
        } else {
            self.is_alive.store(false, Ordering::Release);
            Ok(*self.exit_code.lock().unwrap())
        }
    }
}

static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<TerminalSession>>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, Arc<TerminalSession>>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Prunes exited sessions from the registry and returns the count of active sessions.
/// LEAK-01: Automatically cleans up zombie / dead terminal sessions.
fn cleanup_dead_sessions(reg: &mut HashMap<String, Arc<TerminalSession>>) -> usize {
    reg.retain(|_, session| session.check_alive());
    reg.len()
}

/// Session-anchored cwd validator (SEC-01: absolute + relative paths checked against the anchor).
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
    // Legacy/compat anchor: compute_default (ensured to exist) → process cwd (dirs total failure).
    if let Some(def) = crate::agent::workspace::compute_default_workspace_root() {
        // Best-effort ensure: matches bootstrap "missing dir is created" behavior.
        let _ = std::fs::create_dir_all(&def);
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

pub fn terminal_open(args: TerminalOpenArgs) -> Result<TerminalOpenResult, String> {
    terminal_open_with_workspace_root(args, None)
}

/// Session-anchored variant (TASK-2-WIRE target): `workspace_root=Some(ws)` pins the default
/// cwd and the boundary check to the session workspace root.
pub fn terminal_open_with_workspace_root(
    args: TerminalOpenArgs,
    workspace_root: Option<&Path>,
) -> Result<TerminalOpenResult, String> {
    let cwd = validate_and_resolve_cwd_with_root(args.cwd.as_deref(), workspace_root)?;

    {
        let mut reg = registry().lock().unwrap();
        let active_count = cleanup_dead_sessions(&mut reg);
        if active_count >= MAX_ACTIVE_TERMINALS {
            return Err(format!(
                "Maximum active terminal limit reached ({}/{})",
                active_count, MAX_ACTIVE_TERMINALS
            ));
        }
    }

    let default_cmd = if cfg!(windows) { "powershell.exe" } else { "sh" };
    let cmd_str = args.command.as_deref().unwrap_or(default_cmd);

    let mut command = Command::new(cmd_str);
    if let Some(cmd_args) = &args.args {
        command.args(cmd_args);
    }
    command.current_dir(&cwd);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());

    if let Some(env_map) = &args.env {
        for (k, v) in env_map {
            command.env(k, v);
        }
    }
    // Set terminal sizing env variables
    let cols = args.cols.unwrap_or(80);
    let rows = args.rows.unwrap_or(24);
    command.env("COLUMNS", cols.to_string());
    command.env("LINES", rows.to_string());
    command.env("TERM", "xterm-256color");

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

    let mut child = command.spawn().map_err(|e| format!("Failed to spawn `{cmd_str}`: {e}"))?;
    let pid = child.id();

    #[cfg(windows)]
    if let Some(job_handle) = &job {
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        unsafe {
            AssignProcessToJobObject(job_handle.as_raw_handle(), child.as_raw_handle());
        }
    }

    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    let ring_buffer = Arc::new(Mutex::new(RingBuffer::new(TERMINAL_BUFFER_CAP)));
    let is_alive = Arc::new(AtomicBool::new(true));

    // Spawn stdout drain thread
    if let Some(mut out) = stdout {
        let buf = Arc::clone(&ring_buffer);
        std::thread::Builder::new()
            .name("term-stdout-drain".to_string())
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

    // Spawn stderr drain thread
    if let Some(mut err) = stderr {
        let buf = Arc::clone(&ring_buffer);
        std::thread::Builder::new()
            .name("term-stderr-drain".to_string())
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

    let terminal_id = uuid::Uuid::new_v4().to_string();
    let session = Arc::new(TerminalSession {
        terminal_id: terminal_id.clone(),
        pid,
        stdin: Mutex::new(stdin),
        child: Mutex::new(Some(child)),
        buffer: ring_buffer,
        is_alive,
        exit_code: Mutex::new(None),
        #[cfg(windows)]
        job,
    });

    {
        let mut reg = registry().lock().unwrap();
        cleanup_dead_sessions(&mut reg);
        if reg.len() >= MAX_ACTIVE_TERMINALS {
            return Err(format!(
                "Maximum active terminal limit reached ({}/{})",
                reg.len(), MAX_ACTIVE_TERMINALS
            ));
        }
        reg.insert(terminal_id.clone(), session);
    }

    Ok(TerminalOpenResult { terminal_id, pid })
}

pub fn terminal_send(args: TerminalSendArgs) -> Result<TerminalSendResult, String> {
    let session = {
        let mut reg = registry().lock().unwrap();
        cleanup_dead_sessions(&mut reg);
        reg.get(&args.terminal_id)
            .cloned()
            .ok_or_else(|| format!("Terminal session `{}` not found", args.terminal_id))?
    };

    let mut stdin_guard = session.stdin.lock().unwrap();
    if let Some(stdin) = stdin_guard.as_mut() {
        let bytes = args.input.as_bytes();
        stdin.write_all(bytes).map_err(|e| format!("Failed to write to stdin: {e}"))?;
        stdin.flush().map_err(|e| format!("Failed to flush stdin: {e}"))?;
        Ok(TerminalSendResult {
            bytes_written: bytes.len(),
        })
    } else {
        Err("Terminal stdin is closed or process has exited".to_string())
    }
}

pub fn terminal_read(args: TerminalReadArgs) -> Result<TerminalReadResult, String> {
    let session = {
        let mut reg = registry().lock().unwrap();
        cleanup_dead_sessions(&mut reg);
        reg.get(&args.terminal_id)
            .cloned()
            .ok_or_else(|| format!("Terminal session `{}` not found", args.terminal_id))?
    };

    let offset = args.offset.unwrap_or(0);
    let timeout = Duration::from_millis(args.timeout_ms.unwrap_or(500));
    let start = Instant::now();

    let mut output_bytes = Vec::new();
    let mut cursor = offset;

    loop {
        {
            let buf = session.buffer.lock().unwrap();
            let (slice, new_total) = buf.read_from(cursor);
            if !slice.is_empty() {
                output_bytes.extend_from_slice(&slice);
                cursor = new_total;
            }
        }

        let is_alive = session.check_alive();

        if !output_bytes.is_empty() || !is_alive || start.elapsed() >= timeout {
            let output = String::from_utf8_lossy(&output_bytes).to_string();
            let total = session.buffer.lock().unwrap().total_written();
            return Ok(TerminalReadResult {
                output,
                cursor,
                has_more: total > cursor,
                is_alive,
            });
        }

        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn terminal_signal(args: TerminalSignalArgs) -> Result<TerminalSignalResult, String> {
    let session = {
        let mut reg = registry().lock().unwrap();
        cleanup_dead_sessions(&mut reg);
        reg.get(&args.terminal_id)
            .cloned()
            .ok_or_else(|| format!("Terminal session `{}` not found", args.terminal_id))?
    };

    session.send_signal(args.signal)?;
    Ok(TerminalSignalResult { success: true })
}

pub fn terminal_close(args: TerminalCloseArgs) -> Result<TerminalCloseResult, String> {
    let session = {
        let mut reg = registry().lock().unwrap();
        cleanup_dead_sessions(&mut reg);
        reg.remove(&args.terminal_id)
            .ok_or_else(|| format!("Terminal session `{}` not found", args.terminal_id))?
    };

    let exit_code = session.close(args.force.unwrap_or(false))?;
    Ok(TerminalCloseResult {
        closed: true,
        exit_code,
    })
}
