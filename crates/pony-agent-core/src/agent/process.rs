//! Session-scoped process lifecycle manager (PA-076 phase 5, tasks 5.3-5.4).
//!
//! `ProcessManager` is the production `ProcessBackend`: it starts processes and hands back an
//! opaque, high-entropy handle that is bound to the requesting session. Every later operation
//! (`poll`, `write_stdin`, `kill`) must present the same session id that created the process;
//! cross-session handle use is rejected and stale handles fail closed.
//!
//! Lifecycle:
//! - `start` spawns the child with piped stdin/stdout/stderr and two detached drain threads that
//!   continuously read each stream into a bounded buffer. Draining never blocks the child on a
//!   full pipe; when a stream produces more than the fixed cap, the oldest bytes are dropped and
//!   the stream is permanently marked truncated with a dropped-bytes counter.
//! - `poll` reports `Running` or `Exited` (with exit code) plus the output accumulated since the
//!   last poll. Buffers are cleared on read. After the process exits, its entry is retained so the
//!   final state can be reported; the entry is only removed by `kill` (idempotent cleanup) or by
//!   the session-scoped `shutdown`.
//! - `write_stdin` writes to the child's stdin and is rejected once the process has exited.
//! - `kill` terminates the child (`child.kill()` plus a bounded best-effort reap), `cancel` is a
//!   documented alias for the same operation, `kill_after` arms a background timer, and
//!   `shutdown(session_id)` terminates and removes every process owned by a session.
//!
//! Containment note (design.md Decision 7): this module is the *lifecycle* backend only. Windows
//! Job Objects / Unix process groups are containment concerns handled by the sandbox layer
//! (`sandbox.rs`); `ProcessManager` deliberately does not promise to contain process trees.

use crate::agent::tool_runtime::{
    ProcessBackend, ProcessPollResult, ProcessStartRequest, ProcessState,
};
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Default per-stream output cap (64 KiB).
const DEFAULT_BUFFER_CAP: usize = 64 * 1024;
/// How long `poll` waits for drain threads to flush after the child has exited.
const DRAIN_SETTLE_MAX: Duration = Duration::from_millis(500);
/// Best-effort reap budget after `kill` (30 × 10 ms = 300 ms).
const KILL_REAP_ATTEMPTS: u32 = 30;
const KILL_REAP_INTERVAL: Duration = Duration::from_millis(10);

/// Minimal environment a sandboxed child is allowed to inherit even after `env_clear()`, so it can
/// still spawn commands / resolve system paths without leaking provider keys, session secrets, or
/// ambient proxy vars (design Decision 7, phase-4..7 review P1-3).
const ESSENTIAL_ENV_VARS: &[&str] = &[
    "PATH",
    "PATHEXT",
    "SystemRoot",
    "SystemDrive",
    "ComSpec",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "HOME",
    "LANG",
    "TZ",
];

/// Bounded output buffer with truncation evidence.
///
/// Drain threads push bytes here; `poll` drains it. When the buffer exceeds its cap the oldest
/// bytes are dropped and the stream is permanently marked truncated, with a cumulative
/// dropped-bytes counter for observability. Memory stays bounded by `cap` at all times.
#[derive(Debug)]
struct BoundedBuffer {
    cap: usize,
    data: Vec<u8>,
    dropped: u64,
    truncated: bool,
}

impl BoundedBuffer {
    fn new(cap: usize) -> Self {
        Self {
            cap,
            data: Vec::with_capacity(cap.min(8192)),
            dropped: 0,
            truncated: false,
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.data.extend_from_slice(bytes);
        if self.data.len() > self.cap {
            let excess = self.data.len() - self.cap;
            self.data.drain(..excess);
            self.dropped += excess as u64;
            self.truncated = true;
        }
    }

    /// Take the accumulated bytes (converted lossily to UTF-8) and clear the buffer. The
    /// truncated flag and dropped-bytes counter are cumulative over the process lifetime.
    fn drain(&mut self) -> (String, bool, u64) {
        let text = String::from_utf8_lossy(&self.data).into_owned();
        self.data.clear();
        (text, self.truncated, self.dropped)
    }

    fn dropped_bytes(&self) -> u64 {
        self.dropped
    }
}

/// Reads a child stream until EOF and feeds a bounded buffer. Runs on its own thread so a child
/// that floods a pipe is never blocked; when the pipe closes the `done` flag is set so `poll`
/// knows the final output has been drained.
fn drain_pipe<R: Read>(mut reader: R, buffer: Arc<Mutex<BoundedBuffer>>, done: Arc<AtomicBool>) {
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break, // EOF
            Ok(n) => buffer
                .lock()
                .expect("drain buffer poisoned")
                .push(&chunk[..n]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break, // pipe closed/broken: nothing more will arrive
        }
    }
    done.store(true, Ordering::Release);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExitState {
    Running,
    Exited { exit_code: i32 },
}

struct ManagedProcess {
    session_id: String,
    description: String,
    child: Mutex<Child>,
    state: Mutex<ExitState>,
    stdin: Mutex<Option<ChildStdin>>,
    stdout: Arc<Mutex<BoundedBuffer>>,
    stderr: Arc<Mutex<BoundedBuffer>>,
    stdout_done: Arc<AtomicBool>,
    stderr_done: Arc<AtomicBool>,
}

struct Inner {
    processes: Mutex<BTreeMap<String, Arc<ManagedProcess>>>,
    seq: AtomicU64,
    stdout_cap: usize,
    stderr_cap: usize,
}

/// Session-scoped `ProcessBackend` implementation. Clones share the same process table via an
/// `Arc`, and the type is `Send + Sync` so the runtime can hold it behind an `Arc<dyn
/// ProcessBackend>`.
#[derive(Clone)]
pub struct ProcessManager {
    inner: Arc<Inner>,
}

impl ProcessManager {
    /// A manager with the default 64 KiB per-stream output caps.
    pub fn new() -> Self {
        Self::with_buffer_caps(DEFAULT_BUFFER_CAP, DEFAULT_BUFFER_CAP)
    }

    /// A manager with explicit per-stream output caps (useful for truncation tests).
    pub fn with_buffer_caps(stdout_cap: usize, stderr_cap: usize) -> Self {
        Self {
            inner: Arc::new(Inner {
                processes: Mutex::new(BTreeMap::new()),
                seq: AtomicU64::new(0),
                stdout_cap,
                stderr_cap,
            }),
        }
    }

    /// Terminate a process owned by `session_id` and (if it had already exited) clean its entry
    /// up. For a running process the entry is retained so the final state remains pollable;
    /// calling `kill` again after the process has exited removes the entry (idempotent, safe).
    pub fn kill(&self, session_id: &str, handle: &str) -> Result<(), String> {
        let entry = self.lookup(session_id, handle)?;
        let already_exited = matches!(
            *entry.state.lock().expect("process state poisoned"),
            ExitState::Exited { .. }
        );
        if already_exited {
            // Idempotent cleanup: close stdin and drop the stale handle.
            drop(entry.stdin.lock().expect("process stdin poisoned").take());
            self.inner
                .processes
                .lock()
                .expect("process map poisoned")
                .remove(handle);
            return Ok(());
        }
        self.terminate(&entry)
    }

    /// Cancellation alias for `kill` (used for session shutdown / run cancellation).
    pub fn cancel(&self, session_id: &str, handle: &str) -> Result<(), String> {
        self.kill(session_id, handle)
    }

    /// Arm a background timer that kills `handle` after `duration`. Used to implement
    /// timeouts/cancellation for runs that outlive their budget.
    pub fn kill_after(&self, session_id: &str, handle: &str, duration: Duration) {
        let this = self.clone();
        let session_id = session_id.to_string();
        let handle = handle.to_string();
        std::thread::spawn(move || {
            std::thread::sleep(duration);
            let _ = this.kill(&session_id, &handle);
        });
    }

    /// Terminate and remove every process owned by `session_id`. Returns the number of processes
    /// shut down. Processes owned by other sessions are untouched.
    pub fn shutdown(&self, session_id: &str) -> usize {
        let entries: Vec<(String, Arc<ManagedProcess>)> = {
            let map = self.inner.processes.lock().expect("process map poisoned");
            map.iter()
                .filter(|(_, entry)| entry.session_id == session_id)
                .map(|(handle, entry)| (handle.clone(), Arc::clone(entry)))
                .collect()
        };
        let count = entries.len();
        for (handle, entry) in entries {
            let _ = self.terminate(&entry);
            self.inner
                .processes
                .lock()
                .expect("process map poisoned")
                .remove(&handle);
        }
        count
    }

    /// Cumulative dropped-bytes evidence per stream `(stdout_dropped, stderr_dropped)` for a
    /// handle. The `ProcessPollResult` contract carries only the boolean truncation flags; the
    /// counters are the underlying evidence those flags derive from.
    pub fn drain_stats(&self, session_id: &str, handle: &str) -> Result<(u64, u64), String> {
        let entry = self.lookup(session_id, handle)?;
        let stdout_dropped = entry
            .stdout
            .lock()
            .expect("stdout buffer poisoned")
            .dropped_bytes();
        let stderr_dropped = entry
            .stderr
            .lock()
            .expect("stderr buffer poisoned")
            .dropped_bytes();
        Ok((stdout_dropped, stderr_dropped))
    }

    // ── internals ─────────────────────────────────────────────────────────────────────────────

    fn start_inner(&self, request: &ProcessStartRequest) -> Result<String, String> {
        if request.program.trim().is_empty() {
            return Err("program must not be empty".to_string());
        }
        let mut command = Command::new(&request.program);
        command
            .args(&request.arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Design Decision 7: sandboxed children run with a minimal environment and must not
        // inherit provider keys, session secrets, or ambient proxy env vars. A child whose
        // `SandboxRequest` sets `isolate_environment` gets a minimal env (`env_clear` plus the
        // essential vars plus the explicit `environment_allowlist`); an empty allowlist still
        // isolates — only the essential vars survive. The legacy (non-sandboxed) path inherits
        // the parent env (phase-4..7 review P1-3, phase-5 review P1-1).
        if request.sandbox.isolate_environment {
            command.env_clear();
            for key in ESSENTIAL_ENV_VARS {
                if let Ok(value) = std::env::var(key) {
                    command.env(key, value);
                }
            }
            for entry in &request.sandbox.environment_allowlist {
                if let Some((key, value)) = entry.split_once('=') {
                    command.env(key, value);
                } else if let Ok(value) = std::env::var(entry) {
                    command.env(entry, value);
                }
            }
        }
        let mut child = command.spawn().map_err(|error| {
            format!("spawn failed for `{}`: {error}", request.program)
        })?;

        // Move the pipe endpoints into drain threads. The child itself stays in the manager so
        // `poll` can reap and `kill` can terminate it.
        let stdout_pipe = child
            .stdout
            .take()
            .ok_or_else(|| "stdout pipe unavailable".to_string())?;
        let stderr_pipe = child
            .stderr
            .take()
            .ok_or_else(|| "stderr pipe unavailable".to_string())?;
        let stdin_pipe = child.stdin.take();

        let stdout = Arc::new(Mutex::new(BoundedBuffer::new(self.inner.stdout_cap)));
        let stderr = Arc::new(Mutex::new(BoundedBuffer::new(self.inner.stderr_cap)));
        let stdout_done = Arc::new(AtomicBool::new(false));
        let stderr_done = Arc::new(AtomicBool::new(false));

        {
            let buffer = Arc::clone(&stdout);
            let done = Arc::clone(&stdout_done);
            std::thread::spawn(move || drain_pipe(stdout_pipe, buffer, done));
        }
        {
            let buffer = Arc::clone(&stderr);
            let done = Arc::clone(&stderr_done);
            std::thread::spawn(move || drain_pipe(stderr_pipe, buffer, done));
        }

        let handle = self.next_handle(child.id());
        let entry = Arc::new(ManagedProcess {
            session_id: request.session_id.clone(),
            description: describe_command(&request.program, &request.arguments),
            child: Mutex::new(child),
            state: Mutex::new(ExitState::Running),
            stdin: Mutex::new(stdin_pipe),
            stdout,
            stderr,
            stdout_done,
            stderr_done,
        });
        self.inner
            .processes
            .lock()
            .expect("process map poisoned")
            .insert(handle.clone(), entry);
        Ok(handle)
    }

    fn poll_inner(&self, session_id: &str, handle: &str) -> Result<ProcessPollResult, String> {
        let entry = self.lookup(session_id, handle)?;
        let mut state = entry.state.lock().expect("process state poisoned");
        if matches!(*state, ExitState::Running) {
            let mut child = entry.child.lock().expect("process child poisoned");
            match child.try_wait() {
                Ok(Some(status)) => {
                    *state = ExitState::Exited {
                        exit_code: status.code().unwrap_or(-1),
                    };
                    // Close stdin so a child blocked on input observes EOF after its work is done.
                    drop(entry.stdin.lock().expect("process stdin poisoned").take());
                }
                Ok(None) => {}
                Err(error) => return Err(format!("poll failed for `{handle}`: {error}")),
            }
        }
        let exited = matches!(*state, ExitState::Exited { .. });
        let exit_code = match *state {
            ExitState::Exited { exit_code } => Some(exit_code),
            ExitState::Running => None,
        };
        drop(state);

        if exited {
            self.wait_for_drain(&entry);
        }

        let (stdout, stdout_truncated, _stdout_dropped) = entry
            .stdout
            .lock()
            .expect("stdout buffer poisoned")
            .drain();
        let (stderr, stderr_truncated, _stderr_dropped) = entry
            .stderr
            .lock()
            .expect("stderr buffer poisoned")
            .drain();
        Ok(ProcessPollResult {
            state: if exited {
                ProcessState::Exited
            } else {
                ProcessState::Running
            },
            exit_code,
            stdout,
            stderr,
            stdout_truncated,
            stderr_truncated,
        })
    }

    fn write_stdin_inner(
        &self,
        session_id: &str,
        handle: &str,
        input: &[u8],
    ) -> Result<(), String> {
        let entry = self.lookup(session_id, handle)?;
        let state = entry.state.lock().expect("process state poisoned");
        if matches!(*state, ExitState::Exited { .. }) {
            return Err(format!("process `{handle}` has exited; cannot write stdin"));
        }
        drop(state);
        let mut stdin_guard = entry.stdin.lock().expect("process stdin poisoned");
        let stdin = stdin_guard
            .as_mut()
            .ok_or_else(|| format!("process `{handle}` has no stdin pipe"))?;
        stdin
            .write_all(input)
            .map_err(|error| format!("stdin write failed for `{handle}`: {error}"))
    }

    /// Kill and best-effort reap a process. The entry is retained so a subsequent `poll` can
    /// report the final `Exited` state; entry removal is handled by `kill` (idempotent cleanup)
    /// and `shutdown`.
    fn terminate(&self, entry: &Arc<ManagedProcess>) -> Result<(), String> {
        let mut state = entry.state.lock().expect("process state poisoned");
        if matches!(*state, ExitState::Exited { .. }) {
            return Ok(());
        }
        {
            let mut child = entry.child.lock().expect("process child poisoned");
            let _ = child.kill();
            for _ in 0..KILL_REAP_ATTEMPTS {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        *state = ExitState::Exited {
                            exit_code: status.code().unwrap_or(-1),
                        };
                        break;
                    }
                    Ok(None) => {}
                    Err(_) => break,
                }
                std::thread::sleep(KILL_REAP_INTERVAL);
            }
        }
        drop(entry.stdin.lock().expect("process stdin poisoned").take());
        Ok(())
    }

    /// After the child has exited, wait briefly for the drain threads to flush so the final
    /// output is captured rather than raced over.
    fn wait_for_drain(&self, entry: &ManagedProcess) {
        let deadline = Instant::now() + DRAIN_SETTLE_MAX;
        while Instant::now() < deadline {
            if entry.stdout_done.load(Ordering::Acquire)
                && entry.stderr_done.load(Ordering::Acquire)
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Resolve a handle under a session. Unknown handles and cross-session handle use fail
    /// closed.
    fn lookup(&self, session_id: &str, handle: &str) -> Result<Arc<ManagedProcess>, String> {
        let entry = self
            .inner
            .processes
            .lock()
            .expect("process map poisoned")
            .get(handle)
            .cloned()
            .ok_or_else(|| format!("unknown process handle `{handle}`"))?;
        if entry.session_id != session_id {
            return Err(format!(
                "process handle `{handle}` (`{}`) belongs to session `{}`, not `{session_id}`; cross-session handle use is rejected",
                entry.description, entry.session_id
            ));
        }
        Ok(entry)
    }

    /// High-entropy opaque handle: wall-clock nanoseconds + child pid + monotonic counter.
    fn next_handle(&self, child_pid: u32) -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos() as u64)
            .unwrap_or(0);
        let seq = self.inner.seq.fetch_add(1, Ordering::Relaxed);
        format!("proc-{nanos:016x}-{child_pid:08x}-{seq:06x}")
    }
}

impl Default for ProcessManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessBackend for ProcessManager {
    fn start(&self, request: &ProcessStartRequest) -> Result<String, String> {
        self.start_inner(request)
    }

    fn poll(&self, session_id: &str, handle: &str) -> Result<ProcessPollResult, String> {
        self.poll_inner(session_id, handle)
    }

    fn write_stdin(&self, session_id: &str, handle: &str, input: &[u8]) -> Result<(), String> {
        self.write_stdin_inner(session_id, handle, input)
    }

    fn kill(&self, session_id: &str, handle: &str) -> Result<(), String> {
        self.kill(session_id, handle)
    }
}

fn describe_command(program: &str, arguments: &[String]) -> String {
    let mut parts = vec![program.to_string()];
    parts.extend(arguments.iter().cloned());
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::tool_runtime::SandboxRequest;

    fn sandbox() -> SandboxRequest {
        SandboxRequest {
            workspace_root: ".".to_string(),
            allow_network: false,
            environment_allowlist: Vec::new(),
            isolate_environment: false,
        }
    }

    fn start(
        manager: &ProcessManager,
        session_id: &str,
        program: &str,
        arguments: Vec<String>,
    ) -> String {
        manager
            .start(&ProcessStartRequest {
                session_id: session_id.to_string(),
                program: program.to_string(),
                arguments,
                sandbox: sandbox(),
            })
            .expect("start should succeed")
    }

    #[test]
    fn sandboxed_child_with_allowlist_does_not_inherit_parent_secrets() {
        // Parent-side secret that must NOT reach a sandboxed child (design Decision 7,
        // phase-4..7 review P1-3). `cmd /c set` prints the child's environment.
        std::env::set_var("PA076_SENTINEL_SECRET", "should-not-leak");
        let manager = ProcessManager::new();
        let session = "session-min-env";
        let handle = manager
            .start(&ProcessStartRequest {
                session_id: session.to_string(),
                program: "cmd".to_string(),
                arguments: vec!["/c".to_string(), "set".to_string()],
                sandbox: SandboxRequest {
                    workspace_root: ".".to_string(),
                    allow_network: false,
                    environment_allowlist: vec!["PA076_ALLOWED=allowlist-value".to_string()],
                    isolate_environment: true,
                },
            })
            .expect("start");
        let stdout = poll_until_exited(&manager, session, &handle).stdout;
        std::env::remove_var("PA076_SENTINEL_SECRET");
        assert!(
            stdout.contains("PA076_ALLOWED=allowlist-value"),
            "allowlisted variable must be present in the minimal child env"
        );
        assert!(
            !stdout.contains("PA076_SENTINEL_SECRET"),
            "parent secret must not leak into a sandboxed child env"
        );
    }

    #[test]
    fn sandboxed_child_with_isolate_flag_and_empty_allowlist_does_not_inherit_parent_secrets() {
        // Phase-5 review P1-1: `isolate_environment: true` with an *empty* allowlist must still
        // produce a minimal child env (`env_clear` + essential vars), not inherit the full parent
        // environment. `cmd /c set` prints the child's environment.
        std::env::set_var("PA_SANDBOX_LEAK_SENTINEL", "should-not-leak");
        let manager = ProcessManager::new();
        let session = "session-isolate-min";
        let handle = manager
            .start(&ProcessStartRequest {
                session_id: session.to_string(),
                program: "cmd".to_string(),
                arguments: vec!["/c".to_string(), "set".to_string()],
                sandbox: SandboxRequest {
                    workspace_root: ".".to_string(),
                    allow_network: false,
                    environment_allowlist: Vec::new(),
                    isolate_environment: true,
                },
            })
            .expect("start");
        let stdout = poll_until_exited(&manager, session, &handle).stdout;
        std::env::remove_var("PA_SANDBOX_LEAK_SENTINEL");
        assert!(
            !stdout.contains("PA_SANDBOX_LEAK_SENTINEL"),
            "parent sentinel must not leak into an isolate_environment child env"
        );
        assert!(
            stdout.contains("PATH"),
            "essential PATH must be present in the isolate_environment minimal child env"
        );
    }

    /// Poll until the process reports `Exited` or a deadline passes, accumulating output across
    /// polls. The backend clears buffers on read, so a caller draining to completion must
    /// accumulate the per-poll chunks (this is exactly what the `Run` migration will do).
    fn poll_until_exited(
        manager: &ProcessManager,
        session_id: &str,
        handle: &str,
    ) -> ProcessPollResult {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut last = manager.poll(session_id, handle).expect("poll should succeed");
        let mut stdout = last.stdout.clone();
        let mut stderr = last.stderr.clone();
        let mut stdout_truncated = last.stdout_truncated;
        let mut stderr_truncated = last.stderr_truncated;
        while last.state == ProcessState::Running && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(25));
            last = manager.poll(session_id, handle).expect("poll should succeed");
            stdout.push_str(&last.stdout);
            stderr.push_str(&last.stderr);
            stdout_truncated |= last.stdout_truncated;
            stderr_truncated |= last.stderr_truncated;
        }
        ProcessPollResult {
            state: last.state,
            exit_code: last.exit_code,
            stdout,
            stderr,
            stdout_truncated,
            stderr_truncated,
        }
    }

    #[cfg(windows)]
    fn echo_command() -> (String, Vec<String>) {
        (
            "cmd.exe".to_string(),
            vec!["/C".to_string(), "echo hello-process-manager".to_string()],
        )
    }

    #[cfg(not(windows))]
    fn echo_command() -> (String, Vec<String>) {
        (
            "sh".to_string(),
            vec!["-lc".to_string(), "echo hello-process-manager".to_string()],
        )
    }

    #[cfg(windows)]
    fn long_running_command() -> (String, Vec<String>) {
        (
            "cmd.exe".to_string(),
            vec!["/C".to_string(), "ping -n 5 127.0.0.1".to_string()],
        )
    }

    #[cfg(not(windows))]
    fn long_running_command() -> (String, Vec<String>) {
        ("sh".to_string(), vec!["-lc".to_string(), "sleep 30".to_string()])
    }

    #[cfg(windows)]
    fn large_output_command() -> (String, Vec<String>) {
        // A single burst well over the 1 KiB test cap. A slow trickle of many small lines would
        // be drained by `poll` (which clears the buffer) before the cap is ever exceeded at once,
        // so truncation must be triggered by one large push, independent of poll timing.
        (
            "cmd.exe".to_string(),
            vec!["/C".to_string(), format!("echo {}", "x".repeat(4096))],
        )
    }

    #[cfg(not(windows))]
    fn large_output_command() -> (String, Vec<String>) {
        (
            "sh".to_string(),
            vec![
                "-lc".to_string(),
                "printf '%4096s' ''; echo".to_string(),
            ],
        )
    }

    #[cfg(windows)]
    fn stdin_echo_command() -> (String, Vec<String>) {
        // `set /p` reads one line from stdin and exits without needing stdin EOF (unlike
        // PowerShell, which blocks on EOF when its stdin pipe is kept open). `call echo %%x%%`
        // echoes the captured value back.
        (
            "cmd.exe".to_string(),
            vec!["/C".to_string(), "set /p x=&&call echo %%x%%".to_string()],
        )
    }

    #[cfg(not(windows))]
    fn stdin_echo_command() -> (String, Vec<String>) {
        (
            "sh".to_string(),
            vec!["-lc".to_string(), "read line; echo \"got:$line\"".to_string()],
        )
    }

    #[test]
    fn handles_are_opaque_and_unique() {
        let manager = ProcessManager::new();
        let (program, arguments) = echo_command();
        let first = start(&manager, "session-uniq", &program, arguments.clone());
        let second = start(&manager, "session-uniq", &program, arguments);
        assert_ne!(first, second, "handles must be unique");
        assert!(!first.contains("session"), "handle must be opaque, got {first}");
    }

    #[test]
    fn short_lived_command_polls_to_exited_with_output() {
        let manager = ProcessManager::new();
        let (program, arguments) = echo_command();
        let handle = start(&manager, "session-short", &program, arguments);
        let result = poll_until_exited(&manager, "session-short", &handle);
        assert_eq!(result.state, ProcessState::Exited);
        assert_eq!(result.exit_code, Some(0));
        assert!(
            result.stdout.contains("hello-process-manager"),
            "stdout was {:?}",
            result.stdout
        );
    }

    #[test]
    fn long_lived_reports_running_then_kill_reaches_exited() {
        let manager = ProcessManager::new();
        let (program, arguments) = long_running_command();
        let handle = start(&manager, "session-long", &program, arguments);

        let first = manager
            .poll("session-long", &handle)
            .expect("first poll should succeed");
        assert_eq!(first.state, ProcessState::Running);

        manager
            .kill("session-long", &handle)
            .expect("kill running process should succeed");

        let after_kill = poll_until_exited(&manager, "session-long", &handle);
        assert_eq!(after_kill.state, ProcessState::Exited);
        assert!(after_kill.exit_code.is_some(), "killed process has an exit code");

        // A second kill is safe/ignored and cleans the entry up; the stale handle fails closed.
        assert!(manager.kill("session-long", &handle).is_ok());
        assert!(
            manager.poll("session-long", &handle).is_err(),
            "stale handle must fail closed"
        );
    }

    #[test]
    fn unknown_handle_is_rejected() {
        let manager = ProcessManager::new();
        assert!(manager.poll("session-a", "proc-nonexistent").is_err());
        assert!(manager
            .write_stdin("session-a", "proc-nonexistent", b"x")
            .is_err());
        assert!(manager.kill("session-a", "proc-nonexistent").is_err());
    }

    #[test]
    fn cross_session_handle_use_is_rejected() {
        let manager = ProcessManager::new();
        let (program, arguments) = long_running_command();
        let handle = start(&manager, "session-a", &program, arguments);

        assert!(manager.poll("session-b", &handle).is_err());
        assert!(manager.write_stdin("session-b", &handle, b"x").is_err());
        assert!(manager.kill("session-b", &handle).is_err());

        // The owner session still works.
        let result = manager
            .poll("session-a", &handle)
            .expect("owner poll should succeed");
        assert_eq!(result.state, ProcessState::Running);

        manager
            .kill("session-a", &handle)
            .expect("owner kill should succeed");
        let _ = poll_until_exited(&manager, "session-a", &handle);
        assert!(manager.kill("session-a", &handle).is_ok());
    }

    #[test]
    fn large_output_is_truncated_with_evidence() {
        let manager = ProcessManager::with_buffer_caps(1024, 1024);
        let (program, arguments) = large_output_command();
        let handle = start(&manager, "session-big", &program, arguments);

        let mut result = poll_until_exited(&manager, "session-big", &handle);
        // Drain settle: after the child exits, the drain threads may still be flushing, so the
        // truncation flag can lag the `Exited` state under load. Keep polling briefly until the
        // sticky truncation evidence appears (timing-sensitive test; closeout hardening).
        for _ in 0..40 {
            if result.stdout_truncated {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
            let polled = manager.poll("session-big", &handle).expect("poll");
            result.stdout_truncated |= polled.stdout_truncated;
            result.stderr_truncated |= polled.stderr_truncated;
        }
        assert_eq!(result.state, ProcessState::Exited);
        assert!(
            result.stdout_truncated,
            "output larger than the buffer cap must be marked truncated"
        );
        let (stdout_dropped, _stderr_dropped) = manager
            .drain_stats("session-big", &handle)
            .expect("drain stats should be available");
        assert!(stdout_dropped > 0, "dropped-bytes counter must be > 0");
        assert!(manager.kill("session-big", &handle).is_ok());
    }

    #[test]
    fn write_stdin_feeds_child_and_poll_sees_echoed_input() {
        let manager = ProcessManager::new();
        let (program, arguments) = stdin_echo_command();
        let handle = start(&manager, "session-stdin", &program, arguments);

        manager
            .write_stdin("session-stdin", &handle, b"hello-from-test\n")
            .expect("write_stdin should succeed");

        let result = poll_until_exited(&manager, "session-stdin", &handle);
        assert_eq!(result.state, ProcessState::Exited);
        assert!(
            result.stdout.contains("hello-from-test"),
            "child must echo the input, stdout was {:?}",
            result.stdout
        );
    }

    #[test]
    fn write_stdin_after_exit_is_rejected() {
        let manager = ProcessManager::new();
        let (program, arguments) = echo_command();
        let handle = start(&manager, "session-ws", &program, arguments);
        let result = poll_until_exited(&manager, "session-ws", &handle);
        assert_eq!(result.state, ProcessState::Exited);
        assert!(manager.write_stdin("session-ws", &handle, b"x").is_err());
    }

    #[test]
    fn cancel_terminates_a_running_process() {
        let manager = ProcessManager::new();
        let (program, arguments) = long_running_command();
        let handle = start(&manager, "session-cancel", &program, arguments);
        let first = manager
            .poll("session-cancel", &handle)
            .expect("poll before cancel");
        assert_eq!(first.state, ProcessState::Running);

        manager
            .cancel("session-cancel", &handle)
            .expect("cancel should succeed");
        let result = poll_until_exited(&manager, "session-cancel", &handle);
        assert_eq!(result.state, ProcessState::Exited);
        assert!(manager.kill("session-cancel", &handle).is_ok());
    }

    #[test]
    fn kill_after_timer_terminates_a_running_process() {
        let manager = ProcessManager::new();
        let (program, arguments) = long_running_command();
        let handle = start(&manager, "session-timer", &program, arguments);
        manager.kill_after(
            "session-timer",
            &handle,
            Duration::from_millis(300),
        );
        let result = poll_until_exited(&manager, "session-timer", &handle);
        assert_eq!(result.state, ProcessState::Exited);
        assert!(manager.kill("session-timer", &handle).is_ok());
    }

    #[test]
    fn shutdown_kills_every_process_owned_by_the_session() {
        let manager = ProcessManager::new();
        let (program, arguments) = long_running_command();
        let h1 = start(
            &manager,
            "session-shut",
            &program,
            arguments.clone(),
        );
        let h2 = start(
            &manager,
            "session-shut",
            &program,
            arguments.clone(),
        );
        let h3 = start(&manager, "session-other", &program, arguments);

        assert_eq!(manager.shutdown("session-shut"), 2);

        assert!(
            manager.poll("session-shut", &h1).is_err(),
            "shutdown must remove owned handles"
        );
        assert!(
            manager.poll("session-shut", &h2).is_err(),
            "shutdown must remove owned handles"
        );

        // Processes owned by another session are untouched.
        let other = manager
            .poll("session-other", &h3)
            .expect("other session process must still be tracked");
        assert_eq!(other.state, ProcessState::Running);

        manager
            .kill("session-other", &h3)
            .expect("clean up the remaining process");
        let _ = poll_until_exited(&manager, "session-other", &h3);
        assert!(manager.kill("session-other", &h3).is_ok());
    }
}
