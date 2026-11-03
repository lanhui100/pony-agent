//! B012（会话归属）红相验收：job 层跨会话不可读 / 不可杀。
//!
//! 契约真源：`.dev-team/contracts/tool-timeout-policy.md` §B012（会话归属部分）。
//! - CT-B012-1：`job_start` / promote 注册作业时记录 owner session（来自
//!   `ToolExecutionContext.session_id`）；
//! - CT-B012-2：工具路径上的 `job_output` / `job_kill` / `job_list` 只允许 owner 匹配的作业；
//!   **无 owner 的内部直接 API 调用保持不受限**（既有单测依赖，故本文件一律走 `ToolRouter`
//!   工具路径，不直接调 `jobs::job_*` 函数）；
//! - CT-B012-3：跨会话调用必须明确拒绝（错误码由 executor 定，本文件只断「拒绝 + 不泄露输出」）。
//!
//! 本文件是**外部黑盒验收**（integration test，独立 crate）：只经 `ToolRouter::execute_with_context`
//! 观察可观测行为，不读私有字段、不吞异常、不使用永真断言。
//!
//! 红相预期（实现前）：`job_output`/`job_kill` 当前不接收执行上下文 ⇒ 跨会话照样成功，
//! 于是「sess-B 被拒」的两条断言失败，且 sess-B 能直接读到命令输出 marker。
//!
//! Windows `.cmd` 清理（B013）不在本卡范围。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolRouter};
use serde_json::{json, Value};

/// 命令输出的判别串：只有真正读到该作业输出的调用方才看得到它。
const OUTPUT_MARKER: &str = "SCOPE-A-MARKER-7481";

/// sess-A / sess-B 的会话标识（契约要求 owner 精确匹配）。
const SESSION_A: &str = "sess-A";
const SESSION_B: &str = "sess-B";

/// 长命令：立刻产出一个 marker，然后继续运行（便于「跨会话是否泄露输出」可判定）。
#[cfg(windows)]
fn long_command() -> (String, Vec<String>) {
    (
        "cmd".to_string(),
        vec![
            "/C".to_string(),
            format!("echo {OUTPUT_MARKER} & ping -n 6 127.0.0.1 >nul"),
        ],
    )
}

#[cfg(not(windows))]
fn long_command() -> (String, Vec<String>) {
    (
        "sh".to_string(),
        vec![
            "-c".to_string(),
            format!("echo {OUTPUT_MARKER}; sleep 5"),
        ],
    )
}

/// 每个用例独立临时 workspace（job_start 的 cwd 解析需要它），用完即删。
fn unique_workspace(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock must be after UNIX_EPOCH")
        .as_nanos();
    std::env::temp_dir().join(format!("job_scope_{}_{}", tag, nanos))
}

/// 构造某会话的执行上下文（会话归属的唯一来源）。
fn context_for(workspace: &Path, session_id: &str) -> ToolExecutionContext {
    ToolExecutionContext {
        workspace_root: Some(workspace.to_path_buf()),
        session_id: Some(session_id.to_string()),
        ..Default::default()
    }
}

/// 以指定会话身份调用名为 `tool` 的工具，返回 `(status, 解析后的载荷)`。
/// 输出不是合法 JSON 时 panic —— 禁止静默放行。
fn call_tool(
    router: &ToolRouter,
    workspace: &Path,
    session_id: &str,
    tool: &str,
    arguments: Value,
) -> (String, Value) {
    let call = ToolCall {
        name: tool.to_string(),
        arguments,
        call_id: Some(format!("call-{}-{}", session_id, tool)),
        plan: None,
    };
    let result = router.execute_with_context(&call, &context_for(workspace, session_id));
    let payload: Value = serde_json::from_str(&result.output).unwrap_or_else(|error| {
        panic!(
            "`{tool}` 输出必须是合法 JSON（{}）: {}",
            error, result.output
        )
    });
    (result.status, payload)
}

/// 从 `job_start` 载荷里取 jobId。
///
/// 兼容 `result.job_id`（当前 `JobStartResult` 的 snake_case 序列化）与 `result.jobId`
/// （模型面 camelCase）两种拼写 —— id 的大小写不是本卡契约点，跨会话归属才是。
fn job_id_from(payload: &Value) -> Result<String, String> {
    payload
        .get("result")
        .and_then(|result| result.get("job_id").or_else(|| result.get("jobId")))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("job_start 载荷里没有 jobId：{payload}"))
}

/// `job_output` 载荷里的作业状态（`result.status`）。
fn job_status(payload: &Value) -> Option<&str> {
    payload
        .get("result")
        .and_then(|result| result.get("status"))
        .and_then(Value::as_str)
}

/// `job_output` 载荷里的输出文本（`result.output`）。
fn job_text(payload: &Value) -> &str {
    payload
        .get("result")
        .and_then(|result| result.get("output"))
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// 工具调用是否被接受（`ok:true`）。
fn accepted(payload: &Value) -> bool {
    payload.get("ok").and_then(Value::as_bool) == Some(true)
}

/// 清理守卫：以 **owner（sess-A）** 身份 `job_kill`，panic 时同样回收。
struct JobCleanup<'a> {
    router: &'a ToolRouter,
    workspace: &'a Path,
    job_id: Option<String>,
}

impl Drop for JobCleanup<'_> {
    fn drop(&mut self) {
        let Some(job_id) = self.job_id.take() else {
            return;
        };
        let call = ToolCall {
            name: "job_kill".to_string(),
            arguments: json!({ "jobId": job_id }),
            call_id: Some("cleanup-job-kill".to_string()),
            plan: None,
        };
        let context = context_for(self.workspace, SESSION_A);
        let result = self.router.execute_with_context(&call, &context);
        if result.status != "ok" {
            // 清理失败不掩盖用例本身的断言结论，但必须留痕（不静默吞掉）。
            eprintln!(
                "warn: cleanup job_kill failed for `{}`: {}",
                job_id, result.output
            );
        }
    }
}

/// 条件等待：owner 视角下必须读到 marker（说明作业真实产出了输出，跨会话泄露断言才有鉴别力）。
fn wait_for_owner_marker(
    router: &ToolRouter,
    workspace: &Path,
    job_id: &str,
    timeout: Duration,
) -> Value {
    let deadline = Instant::now() + timeout;
    loop {
        let (status, payload) = call_tool(
            router,
            workspace,
            SESSION_A,
            "job_output",
            json!({ "jobId": job_id, "wait": false }),
        );
        if status == "ok" && job_text(&payload).contains(OUTPUT_MARKER) {
            return payload;
        }
        assert!(
            Instant::now() < deadline,
            "前置条件失败：owner(sess-A) 在 {:?} 内读不到命令输出 marker，payload={payload}",
            timeout
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// 共用前置：sess-A 启动长命令作业，等待 owner 视角读到 marker，返回 `(workspace, jobId)`。
/// 调用方负责用 `JobCleanup` 回收。
fn start_owned_job(router: &ToolRouter, workspace: &Path, tag: &str) -> String {
    let (command, command_args) = long_command();
    let (start_status, start_payload) = call_tool(
        router,
        workspace,
        SESSION_A,
        "job_start",
        json!({
            "command": command,
            "args": command_args,
            "cwd": workspace.display().to_string(),
        }),
    );
    assert_eq!(
        start_status, "ok",
        "CT-B012：sess-A 的 job_start 必须成功（红相前置，{tag}），payload={start_payload}"
    );
    let job_id = job_id_from(&start_payload).unwrap_or_else(|error| {
        panic!("CT-B012：{error}（红相前置：job_start 必须返回 jobId，{tag}）")
    });

    // 前置：owner 视角先确认作业真实产出了 marker（否则「跨会话不泄露」无鉴别力）。
    let owner_view = wait_for_owner_marker(router, workspace, &job_id, Duration::from_secs(3));
    assert_eq!(
        job_status(&owner_view),
        Some("running"),
        "CT-B012：长命令作业此时应仍在运行（5s 命令，{tag}），payload={owner_view}"
    );
    job_id
}

/// CT-B012-2/3：**跨会话读取**必须被拒，且响应不得泄露命令输出；owner 自己仍可读。
#[test]
fn job_output_rejects_cross_session_read() {
    // Arrange
    let workspace = unique_workspace("read");
    std::fs::create_dir_all(&workspace).expect("create temp workspace");
    let router = ToolRouter::with_workspace_root(workspace.clone());
    let job_id = start_owned_job(&router, &workspace, "read");
    let _cleanup = JobCleanup {
        router: &router,
        workspace: &workspace,
        job_id: Some(job_id.clone()),
    };

    // Act：sess-B 读 sess-A 的作业。
    let (read_status, read_payload) = call_tool(
        &router,
        &workspace,
        SESSION_B,
        "job_output",
        json!({ "jobId": job_id, "wait": false }),
    );

    // Assert 1：必须被拒（不是「空结果冒充成功」）。
    assert!(
        !accepted(&read_payload),
        "CT-B012：sess-B 读取 sess-A 的作业必须被拒绝（ok=false），\
         实际 status=`{read_status}` payload={read_payload}"
    );
    assert_eq!(
        read_status, "error",
        "CT-B012：跨会话读取必须以 error 状态返回，实际 `{read_status}` payload={read_payload}"
    );
    assert!(
        read_payload.get("error").is_some(),
        "CT-B012：跨会话读取必须给出结构化 error（明确拒绝，而非空结果），payload={read_payload}"
    );
    // Assert 2：拒绝响应不得夹带命令输出。
    assert!(
        !read_payload.to_string().contains(OUTPUT_MARKER),
        "CT-B012：跨会话拒绝响应**不得泄露**命令输出（出现 marker `{OUTPUT_MARKER}`），\
         payload={read_payload}"
    );

    // Assert 3：owner 自己可读，且读得到 marker（正控：作业确实有输出可读）。
    let (owner_status, owner_payload) = call_tool(
        &router,
        &workspace,
        SESSION_A,
        "job_output",
        json!({ "jobId": job_id, "wait": false }),
    );
    assert_eq!(
        owner_status, "ok",
        "CT-B012：owner(sess-A) 读取自己的作业必须成功，payload={owner_payload}"
    );
    assert!(
        job_text(&owner_payload).contains(OUTPUT_MARKER),
        "CT-B012：owner 必须能读到作业输出 marker（正控），payload={owner_payload}"
    );
}

/// CT-B012-2/3：**跨会话终止**必须被拒，且作业仍 running（证明没被杀）；owner 后续仍可读。
#[test]
fn job_kill_rejects_cross_session_kill() {
    // Arrange
    let workspace = unique_workspace("kill");
    std::fs::create_dir_all(&workspace).expect("create temp workspace");
    let router = ToolRouter::with_workspace_root(workspace.clone());
    let job_id = start_owned_job(&router, &workspace, "kill");
    let _cleanup = JobCleanup {
        router: &router,
        workspace: &workspace,
        job_id: Some(job_id.clone()),
    };

    // Act：sess-B 杀 sess-A 的作业。
    let (kill_status, kill_payload) = call_tool(
        &router,
        &workspace,
        SESSION_B,
        "job_kill",
        json!({ "jobId": job_id }),
    );

    // Assert 1：必须被拒。
    assert!(
        !accepted(&kill_payload),
        "CT-B012：sess-B 终止 sess-A 的作业必须被拒绝（ok=false），\
         实际 status=`{kill_status}` payload={kill_payload}"
    );
    assert_eq!(
        kill_status, "error",
        "CT-B012：跨会话终止必须以 error 状态返回，实际 `{kill_status}` payload={kill_payload}"
    );
    assert!(
        kill_payload.get("error").is_some(),
        "CT-B012：跨会话终止必须给出结构化 error，payload={kill_payload}"
    );

    // Assert 2：拒绝必须**生效** —— 作业仍 running（不是「拒了但照杀」）。
    let (after_status, after_payload) = call_tool(
        &router,
        &workspace,
        SESSION_A,
        "job_output",
        json!({ "jobId": job_id, "wait": false }),
    );
    assert_eq!(
        after_status, "ok",
        "CT-B012：owner 视角续读必须仍然成功，payload={after_payload}"
    );
    assert_eq!(
        job_status(&after_payload),
        Some("running"),
        "CT-B012：被拒绝的跨会话 kill 不得生效 —— 作业必须仍为 running，payload={after_payload}"
    );
    assert!(
        job_text(&after_payload).contains(OUTPUT_MARKER),
        "CT-B012：owner 续读必须仍能拿到输出 marker，payload={after_payload}"
    );
}
