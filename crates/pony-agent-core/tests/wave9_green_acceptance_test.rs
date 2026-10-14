//! wave-9 绿相验收补充用例（task-7，由 L2-T 独立测试者编写）。
//!
//! 本文件只新增用例，不改动已冻结的红相断言
//! （`harness_optimizations_test.rs` / `wave9_adversarial_test.rs`）。
//!
//! 覆盖两条契约要求：
//! 1. P1-1 超时清理的行为存活代理 —— `ProcessManager::tracked_count` 是 `#[cfg(test)]` 的
//!    crate 内部 API，集成测试不可达；因此以「超时后执行通道仍可用」的行为代理验收，
//!    内部不变量（tracked_count == 0）由 process.rs 的 crate 内单测覆盖，此处不重复。
//! 2. P1-2 超时 hint 分类 —— 网络类命令给出「网络/认证/代理」三类可能 + 自检步骤，
//!    非网络类命令逐字保持默认文案。
//!
//! 环境事实：本机 Linux x86_64、无 rustup。

use pony_agent_core::agent::sandbox::TestSandboxBackend;
use pony_agent_core::agent::tools::{ToolCall, ToolRouter};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

/// 默认非网络类超时 hint 文案（契约 P1-2 要求逐字保持）。
const DEFAULT_TIMEOUT_HINT: &str = "请缩短命令执行时间，或显式传入更大的 timeoutMs。";

/// 建立隔离的临时 workspace + ToolRouter（每个用例独立目录，用后即清）。
struct Harness {
    router: ToolRouter,
    workspace: std::path::PathBuf,
}

impl Harness {
    fn new(tag: &str) -> Self {
        let workspace = std::env::temp_dir().join(format!(
            "wave9_green_{}_{}",
            tag,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&workspace).expect("create temp workspace");
        let router = ToolRouter::with_workspace_root(workspace.clone())
            .with_sandbox_backend(TestSandboxBackend::available());
        Self { router, workspace }
    }

    fn run(&self, command: &str, timeout_ms: u64) -> (Value, u64) {
        let call = ToolCall {
            name: "workspace_run_command".to_string(),
            arguments: json!({
                "command": command,
                "cwd": self.workspace.display().to_string(),
                "timeoutMs": timeout_ms,
                "description": "wave9 green acceptance"
            }),
            call_id: Some(format!("call-{}", timeout_ms)),
            plan: None,
        };
        let result = self.router.execute(&call);
        let payload: Value = serde_json::from_str(&result.output)
            .unwrap_or_else(|error| panic!("run_command output 必须是合法 JSON：{error}\n{}", result.output));
        (payload, result.duration_ms)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.workspace);
    }
}

/// 非 Windows 的阻塞命令（必然超过用例设置的 timeoutMs）。
#[cfg(windows)]
const BLOCKING_COMMAND: &str = "ping -n 6 127.0.0.1";
#[cfg(not(windows))]
const BLOCKING_COMMAND: &str = "sleep 5";

/// P1-1 行为存活代理：超时清理后执行通道必须仍然可用，且超时载荷 cleanupError 为 null。
///
/// 断言链：
///   a) 超时命令确实以 timeout 结束，且耗时被超时上限约束（证明进程被真实终止，未挂起）；
///   b) 超时载荷根级 `cleanupError == null`（清理成功，无残留错误）；
///   c) 紧接着的快速命令成功，且实测耗时远小于其超时上限（证明无残留占用执行通道）。
#[test]
fn run_command_timeout_leaves_no_process_entry() {
    let harness = Harness::new("no_process_entry");

    // (a) 必然超时的命令：timeoutMs 远小于命令自身的阻塞时长。
    let timeout_started = Instant::now();
    let (timeout_payload, timeout_duration_ms) = harness.run(BLOCKING_COMMAND, 300);
    let timeout_wall = timeout_started.elapsed();

    assert_eq!(
        timeout_payload.get("error").and_then(|error| error.get("code")).and_then(Value::as_str),
        Some("timeout"),
        "阻塞命令必须以 timeout 结束：{timeout_payload}"
    );
    assert!(
        timeout_wall < Duration::from_secs(4),
        "超时必须真实终止进程并快速返回，实测 {timeout_wall:?}（duration_ms={timeout_duration_ms}）"
    );

    // (b) 清理成功：不得留下 cleanupError 证据。
    let cleanup_error = timeout_payload
        .get("cleanupError")
        .expect("超时载荷根级必须含 cleanupError 键");
    assert!(
        cleanup_error.is_null(),
        "超时清理成功后 cleanupError 必须为 null（非 null 表示进程树清理失败，可能遗留占用）：{timeout_payload}"
    );

    // (c) 行为存活代理：执行通道未被超时残留占用。
    let liveness_timeout_ms = 30_000;
    let liveness_started = Instant::now();
    let (liveness_payload, liveness_duration_ms) = harness.run("echo wave9-liveness-probe", liveness_timeout_ms);
    let liveness_wall = liveness_started.elapsed();

    assert!(
        liveness_payload.get("error").map(Value::is_null).unwrap_or(true),
        "超时之后的快速命令必须成功，不得继承上一次超时的错误：{liveness_payload}"
    );
    assert_eq!(
        liveness_payload.get("exitCode").and_then(Value::as_i64),
        Some(0),
        "快速命令退出码必须为 0：{liveness_payload}"
    );
    assert!(
        liveness_payload
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("wave9-liveness-probe"),
        "快速命令必须真实执行并回显探针输出：{liveness_payload}"
    );
    assert!(
        liveness_wall < Duration::from_secs(5) && liveness_duration_ms < 5_000,
        "超时清理后执行通道必须立即可用：实测 {liveness_wall:?}（duration_ms={liveness_duration_ms}），\
         必须远小于其超时上限 {liveness_timeout_ms} ms"
    );
}

/// P1-2 超时 hint 分类：网络类命令给出「网络/认证/代理」三类可能 + 三步自检；
/// 非网络类命令逐字保持默认文案。
#[test]
fn timeout_hint_classifies_network_commands() {
    let harness = Harness::new("timeout_hint");

    // 网络类命令：`git push --dry-run` 是真实网络类命令；追加 `; sleep 5` 仅用于让超时
    // **确定性**发生（避免依赖真实网络时序造成的 flaky），分类函数判定的是命令行文本本身。
    #[cfg(windows)]
    let network_command = "git push --dry-run & ping -n 6 127.0.0.1";
    #[cfg(not(windows))]
    let network_command = "git push --dry-run; sleep 5";

    let (network_payload, _) = harness.run(network_command, 300);
    assert_eq!(
        network_payload.get("error").and_then(|error| error.get("code")).and_then(Value::as_str),
        Some("timeout"),
        "网络类命令必须以 timeout 结束：{network_payload}"
    );
    let network_hint = network_payload
        .get("error")
        .and_then(|error| error.get("hint"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("超时载荷必须含 error.hint：{network_payload}"));
    for marker in [
        "网络/认证/代理",
        "git ls-remote",
        "git config --get remote.origin.url",
        "SSH 身份自检",
    ] {
        assert!(
            network_hint.contains(marker),
            "网络类超时 hint 必须含 `{marker}`，实测 hint：{network_hint}"
        );
    }

    // 非网络类命令：hint 必须逐字等于既有默认文案（防分类器过度扩大）。
    let (plain_payload, _) = harness.run(BLOCKING_COMMAND, 300);
    assert_eq!(
        plain_payload.get("error").and_then(|error| error.get("code")).and_then(Value::as_str),
        Some("timeout"),
        "非网络类命令必须以 timeout 结束：{plain_payload}"
    );
    let plain_hint = plain_payload
        .get("error")
        .and_then(|error| error.get("hint"))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("超时载荷必须含 error.hint：{plain_payload}"));
    assert_eq!(
        plain_hint, DEFAULT_TIMEOUT_HINT,
        "非网络类命令的 hint 必须逐字保持默认文案（不得被网络分类污染）"
    );
}
