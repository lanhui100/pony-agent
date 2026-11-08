//! 编排桥接工具：job/team/goal/todo/present/spill 等处理器。
use super::*;

/// B012：job_* 工具的会话归属在**工具路径**上的物理证据（error 码 + owner 正控读到 marker）。
///
/// 与 `tests/job_session_scope_test.rs` 同源但用途不同：那条是可执行验收，这条专门打印
/// 真实载荷，供回执引用（`--nocapture`）。
#[test]
fn job_tools_enforce_session_ownership_b012() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace.clone());
    let owner_ctx = ToolExecutionContext {
        workspace_root: Some(workspace.clone()),
        session_id: Some("sess-A".to_string()),
        ..Default::default()
    };
    let other_ctx = ToolExecutionContext {
        workspace_root: Some(workspace.clone()),
        session_id: Some("sess-B".to_string()),
        ..Default::default()
    };

    const MARKER: &str = "B012-SCOPE-MARKER";
    #[cfg(windows)]
    let (program, args) = (
        "cmd",
        vec![
            "/C".to_string(),
            format!("echo {MARKER} & ping -n 6 127.0.0.1 >nul"),
        ],
    );
    #[cfg(not(windows))]
    let (program, args) = (
        "sh",
        vec!["-c".to_string(), format!("echo {MARKER}; sleep 5")],
    );

    let call = |name: &str, arguments: serde_json::Value, tag: &str| ToolCall {
        name: name.to_string(),
        arguments,
        call_id: Some(format!("b012-{tag}")),
        plan: None,
    };

    let started = router.execute_with_context(
        &call(
            TOOL_JOB_START,
            json!({ "command": program, "args": args, "cwd": workspace.display().to_string() }),
            "start",
        ),
        &owner_ctx,
    );
    assert_eq!(started.status, "ok", "job_start 必须成功：{}", started.output);
    let start_payload: Value = serde_json::from_str(&started.output).expect("job_start json");
    let job_id = start_payload["result"]["job_id"]
        .as_str()
        .expect("job_start 载荷必须含 job_id")
        .to_string();

    let read_args = json!({ "jobId": job_id, "wait": false });
    // 前置：owner 必须真实读到 marker（否则「跨会话不泄露」无鉴别力）。
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let probe = router.execute_with_context(
            &call(TOOL_JOB_OUTPUT, read_args.clone(), "probe"),
            &owner_ctx,
        );
        if probe.output.contains(MARKER) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "owner(sess-A) 读不到作业输出 marker：{}",
            probe.output
        );
        thread::sleep(Duration::from_millis(50));
    }

    // 1) 跨会话读：必须 job_not_owned，且响应不回显 marker。
    let denied_read = router.execute_with_context(
        &call(TOOL_JOB_OUTPUT, read_args.clone(), "denied-read"),
        &other_ctx,
    );
    eprintln!(
        "[b012] cross-session job_output: status={} payload={}",
        denied_read.status, denied_read.output
    );
    assert_eq!(denied_read.status, "error");
    let denied_read_payload: Value =
        serde_json::from_str(&denied_read.output).expect("denied read json");
    assert_eq!(
        denied_read_payload["error"]["code"].as_str(),
        Some(crate::agent::jobs::JOB_NOT_OWNED_CODE)
    );
    assert_eq!(
        denied_read_payload["error"]["kind"].as_str(),
        Some(crate::agent::jobs::JOB_NOT_OWNED_CODE)
    );
    assert!(
        !denied_read.output.contains(MARKER),
        "拒绝响应不得泄露作业输出：{}",
        denied_read.output
    );

    // 2) 跨会话杀：必须 job_not_owned，且作业仍 running（拒了不得照杀）。
    let denied_kill = router.execute_with_context(
        &call(TOOL_JOB_KILL, json!({ "jobId": job_id }), "denied-kill"),
        &other_ctx,
    );
    eprintln!(
        "[b012] cross-session job_kill: status={} payload={}",
        denied_kill.status, denied_kill.output
    );
    assert_eq!(denied_kill.status, "error");
    let denied_kill_payload: Value =
        serde_json::from_str(&denied_kill.output).expect("denied kill json");
    assert_eq!(
        denied_kill_payload["error"]["code"].as_str(),
        Some(crate::agent::jobs::JOB_NOT_OWNED_CODE)
    );

    // 3) owner 正控：仍能读到 marker 且状态 running。
    let owner_read = router.execute_with_context(
        &call(TOOL_JOB_OUTPUT, read_args, "owner-read"),
        &owner_ctx,
    );
    eprintln!(
        "[b012] owner job_output: status={} payload={}",
        owner_read.status, owner_read.output
    );
    assert_eq!(owner_read.status, "ok", "owner 读取必须成功：{}", owner_read.output);
    assert!(owner_read.output.contains(MARKER), "owner 必须读到 marker");
    assert!(
        owner_read.output.contains("\"running\""),
        "被拒绝的跨会话 kill 不得生效：作业必须仍 running，实际 {}",
        owner_read.output
    );

    // 4) owner 自己 kill 收尾（正控：owner 有权终止）。
    let owner_kill = router.execute_with_context(
        &call(TOOL_JOB_KILL, json!({ "jobId": job_id }), "owner-kill"),
        &owner_ctx,
    );
    eprintln!(
        "[b012] owner job_kill: status={} payload={}",
        owner_kill.status, owner_kill.output
    );
    assert_eq!(owner_kill.status, "ok", "owner 必须能终止自己的作业");

    let _ = std::fs::remove_dir_all(&workspace);
}

#[test]
fn time_now_returns_unix_timestamp_and_local_iso() {
    let router = ToolRouter::with_workspace_root(temp_workspace());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_TIME_NOW.to_string(),
        arguments: json!({}),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload: Value =
        serde_json::from_str(&result.output).expect("time_now output should be json");
    let unix_ts = payload
        .get("unixTimestampSeconds")
        .and_then(Value::as_u64)
        .expect("unixTimestampSeconds should exist");
    assert!(
        unix_ts > 1_700_000_000,
        "unix timestamp should be reasonable: {}",
        unix_ts
    );
    let local_iso = payload
        .get("localIso")
        .and_then(Value::as_str)
        .expect("localIso should exist");
    assert!(
        local_iso.contains("2026"),
        "localIso should contain current year: {}",
        local_iso
    );
    assert!(
        local_iso.len() > 16,
        "localIso should be a full datetime string"
    );
    let tz = payload
        .get("timezone")
        .and_then(Value::as_str)
        .expect("timezone should exist");
    assert!(tz.len() >= 3, "timezone offset should be present: {}", tz);
}

#[test]
fn tool_outcome_keeps_pending_control_separate_from_legacy_tool_result() {
    let outcome = ToolOutcome::pending(ToolControlOutcome {
        kind: ToolControlKind::WaitingUser,
        request_id: "request-1".to_string(),
    });

    assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
    assert_eq!(
        outcome
            .control_outcome
            .as_ref()
            .map(|control| &control.kind),
        Some(&ToolControlKind::WaitingUser)
    );
    assert!(outcome.result.is_none());

    let legacy = outcome.into_legacy_result("Ask");
    assert_eq!(legacy.status, "error");
    let payload: Value = serde_json::from_str(&legacy.output).expect("legacy payload json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("control_outcome_pending")
    );
}

#[test]
fn tool_outcome_round_trips_legacy_execution_statuses() {
    for (legacy_status, expected) in [
        ("ok", ToolExecutionStatus::Ok),
        ("error", ToolExecutionStatus::Error),
        ("aborted", ToolExecutionStatus::Cancelled),
    ] {
        assert_eq!(
            ToolExecutionStatus::from_legacy_status(legacy_status),
            expected,
            "legacy status {legacy_status}"
        );
    }
}

#[test]
fn ask_returns_fallback_clarification_when_text_missing() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_ECHO_INPUT.to_string(),
        arguments: json!({
            "question": "请确认要继续执行哪一步？"
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload =
        serde_json::from_str::<Value>(&result.output).expect("ask fallback output json");
    assert_eq!(
        payload.get("mode").and_then(Value::as_str),
        Some("fallback_clarification")
    );
    assert_eq!(
        payload.get("prompt").and_then(Value::as_str),
        Some("请确认要继续执行哪一步？")
    );
}

#[test]
fn ask_keeps_plain_text_output_for_normal_echo_path() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_ECHO_INPUT.to_string(),
        arguments: json!({
            "text": "hello"
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    assert_eq!(result.output, "echo_input 返回：hello");
}

#[test]
fn ask_returns_error_when_text_and_question_both_missing() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_ECHO_INPUT.to_string(),
        arguments: json!({}),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("ask error output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("missing_argument")
    );
}

#[test]
fn error_result_with_evidence_merges_evidence_at_payload_root() {
    let result = error_result_with_evidence(
        TOOL_WORKSPACE_RUN_COMMAND,
        "timeout",
        "命令执行超过超时上限 200 ms，已终止。".to_string(),
        Some("hint".to_string()),
        json!({
            "command": "sleep 5",
            "timeoutMs": 200,
            "exitCode": Value::Null,
            "stdout": "partial",
            "stderr": "",
            "stdoutTruncated": false,
            "stderrTruncated": true,
            "cleanupError": Value::Null,
        }),
    );

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("error output json");
    assert_eq!(payload.get("ok").and_then(Value::as_bool), Some(false));
    assert_eq!(
        payload.get("command").and_then(Value::as_str),
        Some("sleep 5")
    );
    assert_eq!(payload.get("timeoutMs").and_then(Value::as_u64), Some(200));
    assert!(payload.get("exitCode").expect("exitCode 键").is_null());
    assert!(payload
        .get("cleanupError")
        .expect("cleanupError 键")
        .is_null());
    assert_eq!(
        payload.get("stdoutTruncated").and_then(Value::as_bool),
        Some(false)
    );
    assert_eq!(
        payload.get("stderrTruncated").and_then(Value::as_bool),
        Some(true)
    );
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("timeout")
    );
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("kind"))
            .and_then(Value::as_str),
        Some("timeout")
    );
    assert!(
        payload.get("summary").is_some(),
        "证据合并不得挤掉既有 summary 结构"
    );
}

#[test]
fn tool_error_code_wins_on_kind_code_conflict() {
    // B4：冲突时 code 获胜（唯一行为变更）。
    let parsed = json!({ "error": { "code": "code_a", "kind": "kind_b", "message": "m" } });
    let error = tool_error_from_output("error", &parsed).expect("parsed");
    assert_eq!(error.kind, "code_a");
    assert_eq!(error.message, "m");
}

#[test]
fn tool_error_pure_string_preserves_text_with_unknown_kind() {
    // B5：纯字符串 error 保文本，kind 落 unknown 标记。
    let parsed = Value::String("upstream provider exploded".to_string());
    // 注意：顶层纯字符串无 error 键 → None；error 键为纯字符串才进 B5 分支。
    assert!(tool_error_from_output("error", &parsed).is_none());
    let wrapped = json!({ "error": "upstream provider exploded" });
    let error = tool_error_from_output("error", &wrapped).expect("parsed");
    assert_eq!(error.kind, "unknown");
    assert_eq!(error.message, "upstream provider exploded");
}

// ── Stage2 红相验收：Windows 批处理执行契约（Bug A） ──
// 根因：workspace_command_parts 的 Windows 分支把含双引号的命令原样拼进 cmd /C 单参数，
// std::process::Command 按 CommandLineToArgvW 将内嵌 `"` 转义成 `\"`，cmd.exe 不按该规则
// 解析 → 引号失衡/管道误切、stdout 只回显命令文本、真实输出丢失。
// 修复方向（Executor 实现）：Windows 分支写入受控 tmp（<root>/.tmp/）下的临时 .cmd 批处理
// 文件（内容 @echo off / cd /d "<cwd>" / <命令原样> / exit /b %errorlevel%），以
// `cmd /C <批处理文件路径>` 执行后删除。
// A1/A2 依赖 Executor 新增的平台无关纯函数 `build_windows_batch_script(command, cwd)`
// （注：若 Executor 采用等价命名，请同步调整下列测试引用；测试契约本身以本函数签名冻结）。
// 当前实现下函数不存在 → 应编译失败（红相证据）。

#[test]
fn tool_router_executes_todo_write_and_todo_list_lifecycle() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace.clone());

    // 1. Initial write: two items
    let write_call = ToolCall {
        call_id: None,
        name: "todo_write".to_string(),
        arguments: json!({
            "session_id": "test-session-todo",
            "todos": [
                { "content": "Step 1: Inspect code", "status": "in_progress" },
                { "content": "Step 2: Add tests", "status": "pending" }
            ]
        }),
        plan: None,
    };
    let write_res = router.execute(&write_call);
    assert_eq!(write_res.status, "ok");
    let write_payload: Value = serde_json::from_str(&write_res.output).unwrap();
    assert_eq!(write_payload["ok"], true);
    assert_eq!(write_payload["result"]["count"], 2);
    assert_eq!(write_payload["result"]["in_progress_count"], 1);
    assert_eq!(write_payload["result"]["completed_count"], 0);

    // 2. Read list
    let list_call = ToolCall {
        call_id: None,
        name: "todo_list".to_string(),
        arguments: json!({
            "session_id": "test-session-todo"
        }),
        plan: None,
    };
    let list_res = router.execute(&list_call);
    assert_eq!(list_res.status, "ok");
    let list_payload: Value = serde_json::from_str(&list_res.output).unwrap();
    assert_eq!(list_payload["result"]["todos"].as_array().unwrap().len(), 2);
    assert_eq!(list_payload["result"]["todos"][0]["status"], "in_progress");

    // 3. Overwrite / mark completed
    let update_call = ToolCall {
        call_id: None,
        name: "todo_write".to_string(),
        arguments: json!({
            "session_id": "test-session-todo",
            "todos": [
                { "content": "Step 1: Inspect code", "status": "completed" },
                { "content": "Step 2: Add tests", "status": "in_progress" }
            ]
        }),
        plan: None,
    };
    let update_res = router.execute(&update_call);
    assert_eq!(update_res.status, "ok");
    let update_payload: Value = serde_json::from_str(&update_res.output).unwrap();
    assert_eq!(update_payload["result"]["completed_count"], 1);
    assert_eq!(update_payload["result"]["in_progress_count"], 1);

    let _ = std::fs::remove_dir_all(&workspace);
}
