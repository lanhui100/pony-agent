//! wave-9 对抗破坏测试（L2-AT）。
//!
//! 契约来源：`.dev-team/contract-matrix-wave9.json`（T1 冻结）。
//! 本文件为红相冻结产物：一经 Lead 冻结即只读，禁止 Executor 修改断言。
//!
//! 对抗面：
//!   1. 指纹穿越 —— 模型用不同措辞 / 内联 env 前缀 / 超时值重试同一核心命令，
//!      连续失败断路器的归一化指纹必须恒等，否则断路器可被措辞绕过（重复检索 / 重复超时雪崩）。
//!   2. 载荷边界 —— 超大 stdout 的超时命令，错误载荷必须不 panic、不截断成空、且 JSON 可解析。
//!   3. 确定性竞态 —— 并发 4 线程 × 固定迭代次数调用 `tool_failure_signal`，
//!      全部指纹必须一致（固定迭代，禁止 sleep + 概率等待）。
//!
//! 环境事实：本机 Linux x86_64、无 rustup，`#[cfg(windows)]` 分支不可编译/不可运行。

use pony_agent_core::agent::runtime::tool_failure_signal;
use pony_agent_core::agent::tools::{ToolCall, ToolResult};
use serde_json::{json, Value};

/// 构造 `error.kind == "timeout"` 的 Run 失败结果（`output` 为 JSON 字符串）。
fn wave9_timeout_result() -> ToolResult {
    ToolResult {
        tool_name: "workspace_run_command".to_string(),
        status: "error".to_string(),
        duration_ms: 120_000,
        output: json!({
            "ok": false,
            "error": {
                "code": "timeout",
                "kind": "timeout",
                "message": "命令执行超过超时上限 120000 ms，已终止。"
            }
        })
        .to_string(),
    }
}

/// 构造 Run 工具调用。
fn wave9_run_call(command: &str, timeout_ms: u64, description: &str, call_id: &str) -> ToolCall {
    ToolCall {
        name: "workspace_run_command".to_string(),
        arguments: json!({
            "command": command,
            "cwd": "/repo",
            "timeoutMs": timeout_ms,
            "description": description
        }),
        call_id: Some(call_id.to_string()),
        plan: None,
    }
}

/// 对抗 1：指纹穿越 —— 4 种不同措辞 / 前缀 / 超时值重试同一核心命令，归一化指纹必须恒等。
/// 红相：当前实现仅剔除 `description`，`timeoutMs` 与内联 env 前缀均参与指纹。
#[test]
fn test_adversarial_1_fingerprint_traversal_resists_rephrasing() {
    let result = wave9_timeout_result();

    let variants: Vec<(&str, u64, &str, &str)> = vec![
        ("git push origin main", 60_000, "推送失败，重试", "adv1-a"),
        ("git push origin main", 600_000, "换个措辞再来一次", "adv1-b"),
        (
            "GIT_TERMINAL_PROMPT=0 GIT_TRACE=1 git push origin main",
            60_000,
            "加上调试环境变量后重试",
            "adv1-c",
        ),
        (
            "set GIT_TERMINAL_PROMPT=0 && git push origin main",
            120_000,
            "Windows 风格 set 前缀重试",
            "adv1-d",
        ),
    ];

    let mut fingerprints: Vec<String> = Vec::new();
    for (command, timeout_ms, description, call_id) in &variants {
        let call = wave9_run_call(command, *timeout_ms, description, call_id);
        let signal = tool_failure_signal(&call, &result)
            .unwrap_or_else(|| panic!("变体 {call_id} 的 timeout 失败必须产生指纹信号"));
        assert_eq!(signal.2, "timeout", "变体 {call_id} 错误码必须为 timeout");
        fingerprints.push(signal.1);
    }

    let baseline = fingerprints[0].clone();
    for (index, fingerprint) in fingerprints.iter().enumerate() {
        assert_eq!(
            *fingerprint, baseline,
            "对抗 1 失败：变体 #{} 的归一化指纹与基线不同，断路器可被措辞/前缀/超时值绕过。\n baseline={}\n variant ={}",
            index, baseline, fingerprint
        );
    }
}

/// 对抗 2：载荷边界 —— 超大 stdout 的超时命令，错误载荷必须不 panic、不截断成空、JSON 可解析。
/// 红相：当前 timeout 分支走 `error_result`，完全没有 stdout/stderr 证据键。
#[test]
fn test_adversarial_2_oversized_stdout_timeout_payload_is_intact() {
    use pony_agent_core::agent::sandbox::TestSandboxBackend;
    use pony_agent_core::agent::tools::ToolRouter;

    let workspace = std::env::temp_dir().join(format!(
        "wave9_adversarial_oversize_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).expect("create temp workspace");

    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_sandbox_backend(TestSandboxBackend::available());

    // 先灌出约 200KB stdout，再挂起等待超时。
    #[cfg(windows)]
    let command = "for /L %i in (1,1,4000) do @echo wave9-padding-line-0123456789012345678901234567890123456789& ping -n 6 127.0.0.1 >nul";
    #[cfg(not(windows))]
    let command = "yes wave9-padding-line-0123456789012345678901234567890123456789 | head -c 200000; sleep 5";

    let call = ToolCall {
        name: "workspace_run_command".to_string(),
        arguments: json!({
            "command": command,
            "cwd": workspace.display().to_string(),
            "timeoutMs": 800,
            "description": "wave9 对抗 2 超大 stdout 超时"
        }),
        call_id: Some("adv2-oversize".to_string()),
        plan: None,
    };

    let result = router.execute(&call);
    let _ = std::fs::remove_dir_all(&workspace);

    // 不 panic：能走到这里即执行链路未崩溃；随后断言载荷完整。
    assert_eq!(
        result.status, "error",
        "超时命令必须返回 error 状态：{}",
        result.output
    );

    let payload: Value = serde_json::from_str(&result.output)
        .expect("超大 stdout 超时载荷必须是合法 JSON 字符串（不得被截断成不可解析文本）");

    let error = payload
        .get("error")
        .expect("超大 stdout 超时载荷必须含 error 对象");
    assert_eq!(
        error.get("code").and_then(Value::as_str),
        Some("timeout"),
        "超大 stdout 超时错误码必须仍为 timeout：{}",
        payload
    );

    // P0-2：证据键落在载荷根级（契约 error_result_with_evidence）。
    let stdout = payload
        .get("stdout")
        .and_then(Value::as_str)
        .expect("超大 stdout 超时载荷必须保留根级 stdout 字符串证据");
    assert!(
        !stdout.is_empty(),
        "超大 stdout 不得被截断成空字符串（截断必须保留边界内证据）"
    );
    assert!(
        payload
            .get("stdoutTruncated")
            .and_then(Value::as_bool)
            .is_some(),
        "超大 stdout 必须显式给出根级 stdoutTruncated 布尔标记：{}",
        payload
    );
    assert!(
        payload
            .get("stderrTruncated")
            .and_then(Value::as_bool)
            .is_some(),
        "超大 stdout 场景必须显式给出根级 stderrTruncated 布尔标记：{}",
        payload
    );
    assert!(
        payload.get("cleanupError").is_some(),
        "超大 stdout 场景必须保留根级 cleanupError 键：{}",
        payload
    );
}

/// 对抗 3：确定性竞态 —— 并发 4 线程各自对**同一** (command, timeout) 调用 `tool_failure_signal`，
/// 固定迭代次数（禁止 sleep + 概率等待），断言全部返回同一指纹。
#[test]
fn test_adversarial_3_concurrent_same_signal_fingerprint_is_deterministic() {
    use std::sync::Mutex;

    const THREADS: usize = 4;
    const ITERATIONS: usize = 50;

    let result = wave9_timeout_result();
    let collected: Mutex<Vec<String>> = Mutex::new(Vec::new());

    std::thread::scope(|scope| {
        for thread_index in 0..THREADS {
            let collected = &collected;
            let result = &result;
            scope.spawn(move || {
                for iteration in 0..ITERATIONS {
                    let call = wave9_run_call(
                        "git push origin main",
                        60_000,
                        "并发同一信号",
                        "adv3-same",
                    );
                    let signal = tool_failure_signal(&call, result).unwrap_or_else(|| {
                        panic!("线程 {thread_index} 第 {iteration} 次迭代必须产生指纹信号")
                    });
                    collected
                        .lock()
                        .expect("fingerprint collector mutex poisoned")
                        .push(format!("{}|{}", signal.1, signal.2));
                }
            });
        }
    });

    let fingerprints = collected.into_inner().expect("collector mutex poisoned");
    assert_eq!(
        fingerprints.len(),
        THREADS * ITERATIONS,
        "并发指纹采集条数必须等于 线程数 × 迭代数"
    );
    let baseline = fingerprints[0].clone();
    for (index, fingerprint) in fingerprints.iter().enumerate() {
        assert_eq!(
            *fingerprint, baseline,
            "对抗 3 失败：并发第 {} 次采样的指纹与基线不一致（非确定性）\n baseline={}\n sample  ={}",
            index, baseline, fingerprint
        );
    }
}

/// 对抗 3（变体面）：并发线程各自使用不同的重试变体（超时值 / 内联 env 前缀 / `set` 前缀），
/// 但核心命令相同 —— 归一化后的指纹仍必须全局唯一一致。
/// 红相：当前实现下并发变体产生 4 种不同指纹。
#[test]
fn test_adversarial_3_concurrent_retry_variants_share_one_fingerprint() {
    use std::sync::Mutex;

    const THREADS: usize = 4;
    const ITERATIONS: usize = 50;

    let result = wave9_timeout_result();
    let collected: Mutex<Vec<(usize, String)>> = Mutex::new(Vec::new());

    std::thread::scope(|scope| {
        for thread_index in 0..THREADS {
            let collected = &collected;
            let result = &result;
            scope.spawn(move || {
                for iteration in 0..ITERATIONS {
                    let (command, timeout_ms) = match thread_index {
                        0 => ("git push origin main".to_string(), 60_000u64),
                        1 => ("git push origin main".to_string(), 300_000u64),
                        2 => (
                            "GIT_TERMINAL_PROMPT=0 git push origin main".to_string(),
                            60_000u64,
                        ),
                        _ => (
                            "set GIT_TERMINAL_PROMPT=0 && git push origin main".to_string(),
                            90_000u64,
                        ),
                    };
                    let call = wave9_run_call(
                        &command,
                        timeout_ms,
                        "并发变体重试",
                        "adv3-variant",
                    );
                    let signal = tool_failure_signal(&call, result).unwrap_or_else(|| {
                        panic!("线程 {thread_index} 第 {iteration} 次迭代必须产生指纹信号")
                    });
                    assert_eq!(signal.2, "timeout");
                    collected
                        .lock()
                        .expect("fingerprint collector mutex poisoned")
                        .push((thread_index, signal.1));
                }
            });
        }
    });

    let samples = collected.into_inner().expect("collector mutex poisoned");
    assert_eq!(samples.len(), THREADS * ITERATIONS);

    let baseline = samples[0].1.clone();
    for (thread_index, fingerprint) in &samples {
        assert_eq!(
            *fingerprint, baseline,
            "对抗 3 变体面失败：线程 {} 的归一化指纹与基线不同，并发重试变体可绕过断路器。\n baseline={}\n variant ={}",
            thread_index, baseline, fingerprint
        );
    }
}
