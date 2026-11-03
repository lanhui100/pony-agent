//! B010 红相验收：promote 摘掉的「超时熔断输入」必须补回来。
//!
//! 契约真源：`.dev-team/contracts/tool-timeout-policy.md` §波次四 · B-1 → B010
//! （promote 成功会摘掉 `error.code=timeout`，使连续失败熔断 / 超时指纹归一 / 非幂等盲重试防护
//! 同时失去输入）。**本文件不改动任何已冻结阈值**，只验收「信号与指引」两件事：
//!
//! - CT-B010-1：promote 结果（`status="ok"` + 根部 `timedOut:true` + `promoted:true` + `jobId`）
//!   必须被 `tool_failure_signal` 识别为 **timeout 类失败信号**（首元/次元与既有 timeout 错误路径一致，
//!   第三元同为 `timeout`），而不是被当成成功（现状：`status != "error"` ⇒ `None`）。
//! - CT-B010-2：指纹位必须由同一函数产生并可复现：同 command + 同 timeoutMs ⇒ 相同；
//!   不同 command ⇒ 不同（禁止测试自己拼字符串比较）。
//! - CT-B010-3：既有 error 路径不受影响（防回归）。
//! - CT-B010-4：promote 结果正文必须指引模型「用 `job_output` 续读、不要重复执行同一命令」。
//!
//! 本文件是**外部黑盒验收**（integration test，独立 crate）：①②③ 用 `ToolCall`/`ToolResult`
//! 直接驱动纯函数；④ 走**真实 Run**（唯一能验证文案来源的方式，避免构造文本导致永真断言）。

use std::sync::{Mutex, MutexGuard};
use pony_agent_core::agent::runtime::tool_failure_signal;
use pony_agent_core::agent::sandbox::TestSandboxBackend;
use pony_agent_core::agent::tools::{
    job_kill, JobKillArgs, ToolCall, ToolResult, ToolRouter,
};
use serde_json::{json, Value};

/// promote 开关 env（契约 v6：默认关闭，`1` 开启）。本文件只有真实 Run 用例需要它。
const PROMOTE_ENV: &str = "PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT";

/// 串行锁：保护进程级 env（只有 `promoted_run_result_guides_to_job_output` 会写它）。
static GATE_LOCK: Mutex<()> = Mutex::new(());

fn gate_lock() -> MutexGuard<'static, ()> {
    GATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

struct EnvVarGuard {
    key: &'static str,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        std::env::set_var(key, value);
        Self { key }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        std::env::remove_var(self.key);
    }
}

/// Run 类工具调用（`workspace_run_command`）。
fn run_call(command: &str, timeout_ms: u64) -> ToolCall {
    ToolCall {
        name: "workspace_run_command".to_string(),
        arguments: json!({
            "command": command,
            "cwd": "/repo",
            "timeoutMs": timeout_ms,
            "description": "B010 验收",
        }),
        call_id: Some("call-b010".to_string()),
        plan: None,
    }
}

/// promote 成功结果：与真实 Run 的 promote 分支同形（`status="ok"`，根部带 timedOut/promoted/jobId）。
fn promote_result(command: &str, timeout_ms: u64, job_id: &str) -> ToolResult {
    ToolResult {
        tool_name: "workspace_run_command".to_string(),
        status: "ok".to_string(),
        duration_ms: 0,
        output: json!({
            "ok": true,
            "command": command,
            "timeoutMs": timeout_ms,
            "timedOut": true,
            "promoted": true,
            "jobId": job_id,
            "exitCode": Value::Null,
            "stdout": "",
            "stderr": "",
            "error": Value::Null,
            "summary": { "text": "（真实文案由 ④ 的真实 Run 验收，此处不复制）" }
        })
        .to_string(),
    }
}

/// 波次一的超时错误结果（`status="error"` + `error.code=timeout`）—— 既有基线。
fn timeout_error_result(timeout_ms: u64) -> ToolResult {
    ToolResult {
        tool_name: "workspace_run_command".to_string(),
        status: "error".to_string(),
        duration_ms: 0,
        output: json!({
            "ok": false,
            "command": "sleep 5",
            "timeoutMs": timeout_ms,
            "timedOut": true,
            "error": {
                "code": "timeout",
                "kind": "timeout",
                "message": format!("命令执行超过超时上限 {timeout_ms} ms，已终止。")
            }
        })
        .to_string(),
    }
}

// ── CT-B010-1 / -3 ─────────────────────────────────────────────────────────

/// CT-B010-1：promote 载荷必须产生 timeout 类信号（首元/次元与既有 timeout 错误路径一致）。
/// CT-B010-3：既有 error 路径必须仍然 Some（防回归）。
#[test]
fn promote_payload_yields_timeout_failure_signal() {
    // Arrange
    let call = run_call("pnpm test", 120_000);
    let promote = promote_result("pnpm test", 120_000, "job-abc");
    let timeout_error = timeout_error_result(120_000);

    // Act
    let promote_signal = tool_failure_signal(&call, &promote);
    let error_signal = tool_failure_signal(&call, &timeout_error);

    // Assert 1（CT-B010-3）：既有 error 路径不受影响。
    let error_signal = error_signal
        .unwrap_or_else(|| panic!("CT-B010-3：timeout 错误路径必须仍产生失败信号（回归）"));
    assert_eq!(
        error_signal.2, "timeout",
        "CT-B010-3：既有 timeout 错误路径的 code 位必须仍为 timeout，实际 `{}`",
        error_signal.2
    );

    // Assert 2（CT-B010-1）：promote 必须被识别为超时类失败，而不是「成功」。
    let promote_signal = promote_signal.unwrap_or_else(|| {
        panic!(
            "CT-B010-1：promote 结果必须产生 timeout 类失败信号（现状 status!=error ⇒ None，\
             熔断输入被摘掉）。payload={}",
            promote.output
        )
    });
    assert_eq!(
        promote_signal.0, error_signal.0,
        "CT-B010-1：首元（tool name）必须与 timeout 错误路径一致，实际 `{}` vs `{}`",
        promote_signal.0, error_signal.0
    );
    assert_eq!(
        promote_signal.1, error_signal.1,
        "CT-B010-1：次元（参数指纹）必须与 timeout 错误路径一致（同一归一化面），\
         实际 promote={} timeout={}",
        promote_signal.1, error_signal.1
    );
    assert_eq!(
        promote_signal.2, "timeout",
        "CT-B010-1：第三元（错误码）必须是 `timeout` —— B010 的目的就是把 timeout 熔断输入补回来，\
         实际 `{}`",
        promote_signal.2
    );
}

// ── CT-B010-2 ─────────────────────────────────────────────────────────────

/// CT-B010-2：指纹位必须可复现且可区分 —— 同 command + 同 timeoutMs ⇒ 相同；不同 command ⇒ 不同。
/// 两个指纹都取自 `tool_failure_signal` 的次元，不由测试自行拼接。
#[test]
fn promote_fingerprint_is_stable_and_command_sensitive() {
    // Arrange
    let timeout_ms = 120_000;
    let call_a1 = run_call("pnpm test", timeout_ms);
    let call_a2 = run_call("pnpm test", timeout_ms);
    let call_b = run_call("pnpm build", timeout_ms);

    // Act
    let sig_a1 = tool_failure_signal(&call_a1, &promote_result("pnpm test", timeout_ms, "job-1"))
        .unwrap_or_else(|| panic!("CT-B010-2：promote 结果必须产生失败信号（同 CT-B010-1）"));
    let sig_a2 = tool_failure_signal(&call_a2, &promote_result("pnpm test", timeout_ms, "job-2"))
        .unwrap_or_else(|| panic!("CT-B010-2：promote 结果必须产生失败信号（同 CT-B010-1）"));
    let sig_b = tool_failure_signal(&call_b, &promote_result("pnpm build", timeout_ms, "job-3"))
        .unwrap_or_else(|| panic!("CT-B010-2：promote 结果必须产生失败信号（同 CT-B010-1）"));

    // Assert
    assert_eq!(
        sig_a1.1, sig_a2.1,
        "CT-B010-2：同 command + 同 timeoutMs 的两次 promote 必须产生相同指纹，\
         a1={} a2={}",
        sig_a1.1, sig_a2.1
    );
    assert_ne!(
        sig_a1.1, sig_b.1,
        "CT-B010-2：不同 command 必须产生不同指纹（否则连续失败断路器会误杀正常推进的工作），\
         test={} build={}",
        sig_a1.1, sig_b.1
    );
    // 指纹非空且不是「同一常量」的退化实现。
    assert!(
        !sig_a1.1.trim().is_empty(),
        "CT-B010-2：指纹位不得为空串，实际 {:?}",
        sig_a1.1
    );
}

// ── CT-B010-4 ─────────────────────────────────────────────────────────────

/// CT-B010-4：promote 结果必须指引「用 `job_output` 续读、不要重复执行同一命令」。
///
/// 这里必须走**真实 Run**：若用测试自己构造的文本断言，就成了永真断言。
/// 因此显式开启 promote、跑一条 300ms 到期后仍存活的长命令，再对真实载荷断言。
#[test]
fn promoted_run_result_guides_to_job_output() {
    let _gate = gate_lock();
    let _env = EnvVarGuard::set(PROMOTE_ENV, "1");

    // Arrange
    let workspace = std::env::temp_dir().join(format!(
        "b010_guide_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).expect("create temp workspace");
    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_sandbox_backend(TestSandboxBackend::available());

    #[cfg(windows)]
    let command = "ping -n 6 127.0.0.1";
    #[cfg(not(windows))]
    let command = "sleep 5";

    let call = ToolCall {
        name: "workspace_run_command".to_string(),
        arguments: json!({
            "command": command,
            "cwd": workspace.display().to_string(),
            "timeoutMs": 300,
            "description": "B010 指引文案验收",
        }),
        call_id: Some("call-b010-real".to_string()),
        plan: None,
    };

    // Act
    let result = router.execute_with_context(
        &call,
        &pony_agent_core::agent::tools::ToolExecutionContext {
            workspace_root: Some(workspace.clone()),
            ..Default::default()
        },
    );
    let _ = std::fs::remove_dir_all(&workspace);

    // 清理：真实 promote 出来的作业必须回收（本用例只关心文案）。
    if let Ok(payload) = serde_json::from_str::<Value>(&result.output) {
        if let Some(job_id) = payload.get("jobId").and_then(Value::as_str) {
            let _ = job_kill(JobKillArgs {
                job_id: job_id.to_string(),
                reason: Some("b010 cleanup".to_string()),
            });
        }
    }

    // Assert 1：先决条件 —— 必须真的 promote，否则文案断言不成立。
    assert_eq!(
        result.status, "ok",
        "CT-B010-4：先决条件 —— 过期命令必须 promote 成功（ok），实际 `{}`：{}",
        result.status, result.output
    );
    let payload: Value = serde_json::from_str(&result.output)
        .expect("CT-B010-4：Run 输出必须是合法 JSON 字符串");
    assert_eq!(
        payload.get("promoted").and_then(Value::as_bool),
        Some(true),
        "CT-B010-4：先决条件 —— 必须 promoted=true：{payload}"
    );

    // Assert 2：正文必须指引用 job_output 续读，并明确「不要重复执行同一命令」。
    let text = payload.to_string();
    assert!(
        text.contains("job_output"),
        "CT-B010-4：promote 结果正文必须指引用 `job_output` 续读增量输出：{payload}"
    );
    assert!(
        text.contains("重复执行"),
        "CT-B010-4：promote 结果正文必须给出「不要重复执行同一命令」的指引（防非幂等盲重试）：{payload}"
    );

    // Assert 3：真实 promote 载荷同样必须产生 timeout 类信号（CT-B010-1 的真实链路复核）。
    let signal = tool_failure_signal(
        &call,
        &ToolResult {
            tool_name: result.tool_name.clone(),
            status: result.status.clone(),
            duration_ms: result.duration_ms,
            output: result.output.clone(),
        },
    )
    .unwrap_or_else(|| {
        panic!(
            "CT-B010-1（真实链路）：promote 的 Run 结果必须产生 timeout 类失败信号，实际 None：{}",
            result.output
        )
    });
    assert_eq!(
        signal.2, "timeout",
        "CT-B010-1（真实链路）：promote 信号的错误码位必须是 timeout，实际 `{}`",
        signal.2
    );
}
