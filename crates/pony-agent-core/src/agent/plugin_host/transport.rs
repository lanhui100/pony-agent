//! 插件宿主的 stdio 传输层：按行分隔的 JSON-RPC 2.0（与 MCP / hooks 同构）。
//!
//! 设计取舍（对齐 `agent/mcp_client.rs` 的既有规范实现，但**不复用**其类型）：
//! - 子进程 stdout 由专用线程逐行读取 → `mpsc`，主线程按 deadline 取用，避免阻塞读把
//!   超时语义变成"等进程自然退出"；
//! - 请求按 id 匹配；服务端通知（带 `method`、无 `id`）跳过；迟到响应用有界 `pending`
//!   缓冲，绝不丢弃也绝不无界增长；
//! - 同一 transport 上的请求串行化（stdout 不可重放）；
//! - stderr 排空线程保留尾部若干行，进程崩溃时作为诊断证据；
//! - Unix 以 `process_group(0)` 启动，终止时对**进程组**发 SIGKILL，覆盖插件派生的孙进程。
//!
//! 平台局限（ADR 0067 已知局限）：Windows 无进程组，[`PluginTransport::terminate`] 只 kill
//! 直接子进程，孙进程为"尽力回收"；彻底回收需要 Job Object（`agent/process/windows_job.rs`
//! 的可见性为 `pub(super)`，提升可见性属契约变更，不在本批范围）。

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use super::capability::apply_child_env;
use super::PluginHostError;

/// 单次请求的最小预算（零/超小 duration 会让"刚写出就超时"，语义无意义）。
pub const MIN_REQUEST_TIMEOUT: Duration = Duration::from_millis(1);

/// 宽限期：先关 stdin 等自然退出，超时才 SIGKILL 进程组。
const SHUTDOWN_GRACE: Duration = Duration::from_millis(200);

/// stderr 诊断尾部行数上限。
const STDERR_TAIL_LINES: usize = 32;

/// 启动子进程，容忍 `ETXTBSY`（"Text file busy"）这一**真实的启动期竞态**。
///
/// 成因：多线程宿主里，另一个线程正在 `write` 一个可执行脚本并持有写句柄时，任何进程
/// `exec` 该文件都会得到 `ETXTBSY`，即使写句柄属于同进程的兄弟线程。插件/夹具脚本常在
/// 运行期内联生成后立即启动，因此这个窗口必然存在。
///
/// 处理：仅对 `ETXTBSY` 做**极短的有界重试**（≤5 次 × 20ms）。这不是"插件调用重试"，
/// 不违反 ADR 0067 的"崩溃不重试"语义：重试发生在 `exec` 之前（进程尚未创建），
/// 一旦 spawn 成功就不再有任何启动重试。
#[cfg(unix)]
fn spawn_tolerating_etxtbsy(process: &mut Command) -> std::io::Result<Child> {
    const ETXTBSY: i32 = libc::ETXTBSY;
    const MAX_ATTEMPTS: u32 = 5;
    let mut attempt = 0u32;
    loop {
        match process.spawn() {
            Ok(child) => return Ok(child),
            Err(err) if err.raw_os_error() == Some(ETXTBSY) && attempt < MAX_ATTEMPTS => {
                attempt += 1;
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(err) => return Err(err),
        }
    }
}

/// 非 Unix：`ETXTBSY` 不适用，直接 spawn。
#[cfg(not(unix))]
fn spawn_tolerating_etxtbsy(process: &mut Command) -> std::io::Result<Child> {
    process.spawn()
}

/// 一个已经 spawn 起来的插件子进程 + stdio JSON-RPC 通道。
pub struct PluginTransport {
    child: Child,
    /// `Option` 便于 `terminate` 时 take 并关闭（EOF 让规范插件自行退出）。
    stdin: Mutex<Option<ChildStdin>>,
    responses: Mutex<Receiver<String>>,
    /// 请求串行锁：同一 transport 上的请求必须排队，否则收包方互相抢响应。
    request_lock: Mutex<()>,
    /// 迟到/乱序响应缓冲（按 id），有界。
    pending: Mutex<HashMap<u64, Value>>,
    next_id: AtomicU64,
    stderr_tail: Arc<Mutex<Vec<String>>>,
    /// 进程组已发出终止信号（幂等标记）。
    terminated: Mutex<bool>,
}

impl std::fmt::Debug for PluginTransport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PluginTransport")
            .field("pid", &self.child.id())
            .finish()
    }
}

impl PluginTransport {
    /// 启动插件进程。
    ///
    /// 隔离面全部在此收口：`env_clear()` + 白名单、`current_dir = workspace_root`、
    /// Unix 独立进程组。调用方必须先校验 `command` 与 `workspace_root`。
    pub fn spawn(
        command: &Path,
        args: &[String],
        workspace_root: &Path,
        extra_env: &[(String, String)],
    ) -> Result<Self, PluginHostError> {
        let mut process = Command::new(command);
        process
            .args(args)
            .current_dir(workspace_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Unix：独立进程组，Drop/超时可连同插件派生的孙进程一起回收。
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            process.process_group(0);
        }
        // F8a：插件是外部代码，绝不能继承宿主完整环境（provider key / 内部 token）。
        apply_child_env(&mut process, extra_env);

        let mut child = spawn_tolerating_etxtbsy(&mut process).map_err(|err| {
            PluginHostError::Spawn(format!(
                "启动插件进程 {} 失败：{err}。",
                command.display()
            ))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| PluginHostError::Spawn("无法获取 stdin。".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| {
            PluginHostError::Spawn("无法获取 stdout。".into())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            PluginHostError::Spawn("无法获取 stderr。".into())
        })?;

        let (sender, receiver) = mpsc::channel::<String>();
        std::thread::Builder::new()
            .name("plugin-stdout".to_string())
            .spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines() {
                    match line {
                        Ok(line) => {
                            if sender.send(line).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
            })
            .map_err(|err| {
                PluginHostError::Spawn(format!("启动 stdout 读线程失败：{err}。"))
            })?;

        let stderr_tail = Arc::new(Mutex::new(Vec::<String>::new()));
        let tail = Arc::clone(&stderr_tail);
        std::thread::Builder::new()
            .name("plugin-stderr".to_string())
            .spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    if let Ok(mut tail) = tail.lock() {
                        tail.push(line);
                        if tail.len() > STDERR_TAIL_LINES {
                            tail.remove(0);
                        }
                    }
                }
            })
            .ok();

        Ok(Self {
            child,
            stdin: Mutex::new(Some(stdin)),
            responses: Mutex::new(receiver),
            request_lock: Mutex::new(()),
            pending: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            stderr_tail,
            terminated: Mutex::new(false),
        })
    }

    /// 插件进程 pid。
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// 进程是否仍在运行（`try_wait` 非阻塞观察，不吞掉退出码）。
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// 非阻塞观察子进程退出状态（`Ok(None)` = 仍在运行）。
    ///
    /// 与 [`PluginTransport::is_running`] 的区别：把 `io::Error` 上抛而非折叠为 `false`，
    /// 供验收做"有界轮询直到退出"时区分"还活着"与"观察失败"。
    pub fn try_wait_status(
        &mut self,
    ) -> std::io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }

    /// stderr 尾部若干行（诊断用）。
    pub fn stderr_tail(&self) -> Vec<String> {
        self.stderr_tail
            .lock()
            .map(|tail| tail.clone())
            .unwrap_or_default()
    }

    /// 发送通知（无 id、不等待响应）。
    pub fn notify(&self, method: &str, params: Value) -> Result<(), PluginHostError> {
        self.write_line(&json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        }))
    }

    /// 发送请求并按 `deadline` 等待同 id 响应，返回 `result`。
    pub fn request(
        &self,
        method: &str,
        params: Value,
        deadline: Duration,
    ) -> Result<Value, PluginHostError> {
        let _serial = self.request_lock.lock().map_err(|_| {
            PluginHostError::Unavailable("transport 锁已中毒。".into())
        })?;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.write_line(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        }))?;

        let deadline = deadline.max(MIN_REQUEST_TIMEOUT);
        let started = Instant::now();
        let end = started.checked_add(deadline).unwrap_or(started + Duration::from_secs(3600));

        loop {
            if let Some(buffered) = self
                .pending
                .lock()
                .ok()
                .and_then(|mut pending| pending.remove(&id))
            {
                return Self::extract_result(method, buffered);
            }

            let remaining = end.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(PluginHostError::Timeout(format!(
                    "插件 `{method}` 请求超过 {deadline:?} 未响应。"
                )));
            }

            let receiver = self.responses.lock().map_err(|_| {
                PluginHostError::Unavailable("transport 收包锁已中毒。".into())
            })?;
            let line = match receiver.recv_timeout(remaining) {
                Ok(line) => line,
                Err(RecvTimeoutError::Timeout) => {
                    return Err(PluginHostError::Timeout(format!(
                        "plugin_timeout: 插件 `{method}` 请求超过 {deadline:?} 未响应。"
                    )))
                }
                // 管道关闭 = 插件进程已退出/崩溃。这是 fail-closed 路径，不重试。
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(PluginHostError::Unavailable(format!(
                        "插件进程管道已关闭（`{method}` 期间崩溃或自行退出）。"
                    )))
                }
            };
            drop(receiver);

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(trimmed).map_err(|err| {
                PluginHostError::Protocol(format!(
                    "插件响应不是合法 JSON（{err}）：{trimmed}"
                ))
            })?;

            // 服务端发起的报文（带 `method`）必须在 id 匹配**之前**分流：
            // - `method` + `id` = 服务端→客户端**请求**（ping / roots / sampling 之类）：
            //   必须应答，否则对端永久挂起；v1 不支持任何服务端请求，回 -32601。
            // - `method` 无 `id` = 通知：只执行、不应答。
            // 若不先分流，带 id 的服务端请求会落进"响应"分支（甚至与宿主请求 id 撞车被当成
            // 自己的响应），既不应答又可能被误解析——与 `mcp_client::request` 同构的处理。
            if let Some(server_method) = value.get("method").and_then(Value::as_str) {
                match value.get("id") {
                    Some(server_id) if !server_id.is_null() => {
                        self.respond_method_not_found(server_id, server_method)?;
                    }
                    _ => {}
                }
                continue;
            }

            match Self::response_id(&value) {
                Some(response_id) if response_id == id => {
                    return Self::extract_result(method, value)
                }
                Some(response_id) => {
                    // 迟到响应：有界暂存，绝不静默丢弃（丢弃会让调用方永久等不到）。
                    if let Ok(mut pending) = self.pending.lock() {
                        pending.insert(response_id, value);
                        if pending.len() > 32 {
                            if let Some(oldest) = pending.keys().copied().min() {
                                pending.remove(&oldest);
                            }
                        }
                    }
                }
                None => {
                    if value.get("method").is_some() {
                        // 服务端通知：无需响应，跳过。
                        continue;
                    }
                    if let Some(error) = value.get("error") {
                        return Err(PluginHostError::Protocol(format!(
                            "`{method}` 收到无 id 的错误响应：{}",
                            Self::error_message(error)
                        )));
                    }
                    return Err(PluginHostError::Protocol(format!(
                        "`{method}` 收到无法归属的报文：{value}"
                    )));
                }
            }
        }
    }

    fn write_line(&self, payload: &Value) -> Result<(), PluginHostError> {
        let mut line = serde_json::to_string(payload).map_err(|err| {
            PluginHostError::Protocol(format!("序列化请求失败：{err}。"))
        })?;
        line.push('\n');
        let mut guard = self.stdin.lock().map_err(|_| {
            PluginHostError::Unavailable("stdin 锁已中毒。".into())
        })?;
        let stdin = guard.as_mut().ok_or_else(|| {
            PluginHostError::Unavailable("插件 stdin 已关闭。".into())
        })?;
        stdin
            .write_all(line.as_bytes())
            .and_then(|_| stdin.flush())
            .map_err(|_| {
                PluginHostError::Unavailable(
                    "写入插件 stdin 失败（进程已退出）。".to_string(),
                )
            })
    }

    /// 应答服务端发起的请求：v1 不支持任何服务端→客户端方法，统一回 JSON-RPC `-32601`。
    ///
    /// 为什么必须应答而不是忽略：JSON-RPC 的请求语义是"等待响应"，忽略会让对端挂起
    /// （插件可能因此阻塞在自己的 ping/sampling 上，最终把宿主调用拖成超时）。
    fn respond_method_not_found(
        &self,
        server_id: &Value,
        method: &str,
    ) -> Result<(), PluginHostError> {
        self.write_line(&json!({
            "jsonrpc": "2.0",
            "id": server_id,
            "error": {
                "code": -32601,
                "message": format!("method not found: {method}"),
            },
        }))
    }

    fn response_id(value: &Value) -> Option<u64> {
        match value.get("id") {
            Some(Value::Number(number)) => number.as_u64(),
            Some(Value::String(text)) => text.trim().parse::<u64>().ok(),
            _ => None,
        }
    }

    fn error_message(error: &Value) -> String {
        error
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| error.to_string())
    }

    fn extract_result(method: &str, value: Value) -> Result<Value, PluginHostError> {
        if let Some(error) = value.get("error") {
            return Err(PluginHostError::Server(format!(
                "`{method}` 返回错误：{}",
                Self::error_message(error)
            )));
        }
        value.get("result").cloned().ok_or_else(|| {
            PluginHostError::Protocol(format!(
                "`{method}` 响应缺少 result：{value}"
            ))
        })
    }

    /// 终止插件进程并回收（幂等）。
    ///
    /// 顺序：关 stdin（EOF 让规范插件自行退出）→ 宽限 200ms → Unix 对进程组 SIGKILL
    /// → `kill()` + `wait()` 回收僵尸进程。**必须**同步回收：验收要求"超时后无残留子进程"。
    pub fn terminate(&mut self) {
        if let Ok(mut terminated) = self.terminated.lock() {
            if *terminated {
                return;
            }
            *terminated = true;
        }

        if let Ok(mut guard) = self.stdin.lock() {
            if let Some(mut stdin) = guard.take() {
                let _ = stdin.flush();
                drop(stdin);
            }
        }

        let grace_deadline = Instant::now() + SHUTDOWN_GRACE;
        while Instant::now() < grace_deadline {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(_) => break,
            }
        }

        #[cfg(unix)]
        {
            let pid = self.child.id() as i32;
            // safety: 进程组由本模块以 process_group(0) 创建，pgid == 子进程 pid。
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for PluginTransport {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pony-plugin-transport-{tag}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create dir");
        dir
    }

    /// 服务端→客户端请求必须被**应答**（-32601），否则对端挂起。
    ///
    /// fixture 是 POSIX `sh` 脚本：它先读宿主的 initialize 请求，然后主动发一个
    /// `{"id":42,"method":"ping"}`，再读宿主的应答；只有应答里带 `"id":42` 与 `-32601`
    /// 才回 `answered=true`。宿主要是把该报文当"通知"丢弃，脚本的第二次 `read` 会
    /// 永久阻塞 → 本用例以超时失败（而不是静默通过）。
    #[test]
    fn server_initiated_request_is_answered_with_method_not_found() {
        let dir = temp_dir("server-request");
        let script = dir.join("plugin.sh");
        std::fs::write(
            &script,
            r#"#!/bin/sh
read -r host_request
printf '{"jsonrpc":"2.0","id":42,"method":"ping"}\n'
read -r host_reply
id_ok=false
err_ok=false
case "$host_reply" in
  *'"id":42'*) id_ok=true ;;
esac
case "$host_reply" in
  *-32601*) err_ok=true ;;
esac
if [ "$id_ok" = true ] && [ "$err_ok" = true ]; then ok=true; else ok=false; fi
printf '{"jsonrpc":"2.0","id":1,"result":{"answered":%s}}\n' "$ok"
"#,
        )
        .expect("write plugin script");
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755))
                .expect("chmod plugin script");
        }

        let mut transport = PluginTransport::spawn(&script, &[], &dir, &[]).expect("spawn");
        let result = transport
            .request("initialize", json!({}), Duration::from_secs(10))
            .expect("宿主必须应答服务端请求，且自己的请求不得被误解析");
        assert_eq!(
            result.get("answered").and_then(Value::as_bool),
            Some(true),
            "服务端请求必须收到 -32601 应答，实际 {result}"
        );
        transport.terminate();
        let _ = std::fs::remove_dir_all(dir);
    }
}
