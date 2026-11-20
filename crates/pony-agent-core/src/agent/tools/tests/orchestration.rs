//! 编排桥接工具：job/team/goal/todo/present/spill 等处理器。
use super::*;

/// B012 用例后台作业的自身生命周期（非 Windows：`sh -c 'echo <marker>; sleep 5'`）。
const B012_JOB_LIFETIME_SECS: u64 = 5;

/// 等 marker 的**失败上界**（不是断言口径）：marker 一旦出现立即返回，预算只决定「失败报告
/// 迟到多久」。旧值直接取 5s = 作业生命周期，等于把「spawn + 作业登记 + 首次读输出必须在
/// 5s 内跑完」写成隐含前提；全量并行负载下这会偶发假失败。取生命周期 ×6 覆盖排队抖动，
/// 断言语义（owner 必须读到 marker）不变。
const B012_MARKER_WAIT_BUDGET: Duration = Duration::from_secs(B012_JOB_LIFETIME_SECS * 6);

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
        vec![
            "-c".to_string(),
            format!("echo {MARKER}; sleep {B012_JOB_LIFETIME_SECS}"),
        ],
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
    // 条件等待 + 有界失败上界：见 [`B012_MARKER_WAIT_BUDGET`]。
    let deadline = Instant::now() + B012_MARKER_WAIT_BUDGET;
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

#[test]
fn goal_tools_authority_gate_b128() {
    // B130 附带：旧版本不隔离 goal store，并行 goal 用例（临时 sidecar / 损坏 sidecar）
    // 会把它推进「degraded 拒绝写盘」分支而随机假红；本用例只验证权限门，故与其它
    // goal 用例共用同一把测试锁 + 独占 sidecar。
    let _lock = crate::agent::goal::test_support::lock();
    let _store = crate::agent::goal::test_support::TempGoalStore::install("b128-auth");
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace.clone());
    let session = "b128-auth-session";
    let _ = crate::agent::goal::clear_goal(session);

    let call = |name: &str, arguments: serde_json::Value| ToolCall {
        name: name.to_string(),
        arguments,
        call_id: Some("b128-test".to_string()),
        plan: None,
    };

    // 1. 非人类轮次（例如子代理或自主非人类轮）尝试 create_goal -> 必须被权限门拦截
    let non_human_ctx = ToolExecutionContext {
        workspace_root: Some(workspace.clone()),
        session_id: Some(session.to_string()),
        dispatch_context: Some(crate::agent::dispatcher::DispatchContext {
            session_id: Some(session.to_string()),
            is_direct_human: false,
            is_goal_round: false,
            ..Default::default()
        }),
        ..Default::default()
    };
    let unauth_create = router.execute_with_context(
        &call(
            "create_goal",
            json!({ "session_id": session, "objective": "自主新建目标" }),
        ),
        &non_human_ctx,
    );
    assert_eq!(unauth_create.status, "error");
    assert!(unauth_create.output.contains("goal_action_unauthorized"));

    // 2. 人类直接轮次 -> 允许 create_goal
    let human_ctx = ToolExecutionContext {
        workspace_root: Some(workspace.clone()),
        session_id: Some(session.to_string()),
        dispatch_context: Some(crate::agent::dispatcher::DispatchContext {
            session_id: Some(session.to_string()),
            is_direct_human: true,
            is_goal_round: false,
            ..Default::default()
        }),
        ..Default::default()
    };
    let auth_create = router.execute_with_context(
        &call(
            "create_goal",
            json!({ "session_id": session, "objective": "人类批准的目标" }),
        ),
        &human_ctx,
    );
    assert_eq!(auth_create.status, "ok", "create_goal 必须成功: {}", auth_create.output);
    let create_payload: Value = serde_json::from_str(&auth_create.output).unwrap();
    let goal_id = create_payload["result"]["goal"]["id"].as_str().unwrap();

    // 3. 人类暂停该目标
    let pause_call = call(
        "update_goal",
        json!({
            "session_id": session,
            "goal_id": goal_id,
            "revision": 1,
            "action": "pause"
        }),
    );
    let pause_res = router.execute_with_context(&pause_call, &human_ctx);
    assert_eq!(pause_res.status, "ok");

    // 4. 非人类自主轮次（模型）尝试自行 resume 暂停的目标 -> 必须被权限门拦截
    let resume_call = call(
        "update_goal",
        json!({
            "session_id": session,
            "goal_id": goal_id,
            "revision": 2,
            "action": "resume"
        }),
    );
    let unauth_resume = router.execute_with_context(&resume_call, &non_human_ctx);
    assert_eq!(unauth_resume.status, "error");
    assert!(unauth_resume.output.contains("goal_action_unauthorized"));

    // 5. 人类可以 resume 目标
    let auth_resume = router.execute_with_context(&resume_call, &human_ctx);
    assert_eq!(auth_resume.status, "ok");

    // 6. 普通非人类非 goal 轮尝试 complete -> 必须被权限门拦截
    let complete_call = call(
        "update_goal",
        json!({
            "session_id": session,
            "goal_id": goal_id,
            "revision": 3,
            "action": "complete"
        }),
    );
    let unauth_complete = router.execute_with_context(&complete_call, &non_human_ctx);
    assert_eq!(unauth_complete.status, "error");
    assert!(unauth_complete.output.contains("goal_action_unauthorized"));

    // 7. goal-round 轮次允许模型 complete
    let goal_round_ctx = ToolExecutionContext {
        workspace_root: Some(workspace.clone()),
        session_id: Some(session.to_string()),
        dispatch_context: Some(crate::agent::dispatcher::DispatchContext {
            session_id: Some(session.to_string()),
            is_direct_human: false,
            is_goal_round: true,
            ..Default::default()
        }),
        ..Default::default()
    };
    let auth_complete = router.execute_with_context(&complete_call, &goal_round_ctx);
    assert_eq!(auth_complete.status, "ok", "goal round 必须允许 complete: {}", auth_complete.output);

    let _ = std::fs::remove_dir_all(&workspace);
}

/// B129：终态 goal 的收尾上下文必须由**真实工具路径**回灌，而不是只存在于一个孤立函数里。
///
/// 断言三点：active 不注入（避免每轮刷无关文案）、completed/blocked 注入对应引导、
/// 且终态后真实续轮读面（`continuation_goal_for_run`）返回 `None`（不再发起新轮）。
#[test]
fn terminal_goal_wrapup_is_injected_into_tool_result_b129() {
    let _lock = crate::agent::goal::test_support::lock();
    let _store = crate::agent::goal::test_support::TempGoalStore::install("b129-wrapup");
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace.clone());
    let session = "b129-wrapup-session";

    let call = |name: &str, arguments: serde_json::Value| ToolCall {
        name: name.to_string(),
        arguments,
        call_id: Some("b129-wrapup".to_string()),
        plan: None,
    };

    // create_goal 的门禁要求直接人类授权；complete/blocked 允许在 goal 轮内由模型发起。
    let human_ctx = ToolExecutionContext {
        workspace_root: Some(workspace.clone()),
        session_id: Some(session.to_string()),
        dispatch_context: Some(crate::agent::dispatcher::DispatchContext {
            session_id: Some(session.to_string()),
            is_direct_human: true,
            is_goal_round: false,
            ..Default::default()
        }),
        ..Default::default()
    };
    let goal_round_ctx = ToolExecutionContext {
        workspace_root: Some(workspace.clone()),
        session_id: Some(session.to_string()),
        dispatch_context: Some(crate::agent::dispatcher::DispatchContext {
            session_id: Some(session.to_string()),
            is_direct_human: false,
            is_goal_round: true,
            ..Default::default()
        }),
        ..Default::default()
    };

    let created = router.execute_with_context(
        &call(
            "create_goal",
            json!({ "session_id": session, "objective": "把报告写完并给出结论" }),
        ),
        &human_ctx,
    );
    assert_eq!(created.status, "ok", "create_goal 必须成功: {}", created.output);
    let created_payload: Value =
        serde_json::from_str(&created.output).expect("create_goal 输出必须是 JSON");
    let goal_id = created_payload["result"]["goal"]["id"]
        .as_str()
        .expect("create_goal 必须返回 goal.id")
        .to_string();
    assert!(
        created_payload.get("wrapup").is_none(),
        "非终态相位不得注入 wrapup: {}",
        created.output
    );

    // 1. complete：goal 轮内模型自行完成 → tool result 带 <goal_complete>。
    let completed = router.execute_with_context(
        &call(
            "update_goal",
            json!({
                "session_id": session,
                "goal_id": goal_id,
                "revision": 1,
                "action": "complete"
            }),
        ),
        &goal_round_ctx,
    );
    assert_eq!(
        completed.status, "ok",
        "goal round 必须允许 complete: {}",
        completed.output
    );
    let completed_payload: Value =
        serde_json::from_str(&completed.output).expect("update_goal 输出必须是 JSON");
    let complete_text = completed_payload["wrapup"]
        .as_str()
        .unwrap_or_else(|| panic!("completed 必须注入 wrapup: {}", completed.output));
    assert!(complete_text.starts_with("<goal_complete>"), "{complete_text}");
    assert!(
        complete_text.contains("把报告写完并给出结论"),
        "收尾文案必须点名目标：{complete_text}"
    );
    assert!(
        complete_text.contains("不要再调用工具"),
        "必须禁止本轮继续调工具：{complete_text}"
    );
    assert!(
        crate::agent::goal::continuation_goal_for_run(Some(session)).is_none(),
        "completed 后宿主不得再发起 goal 轮"
    );

    // 2. blocked：人类重建目标后，goal 轮内模型标记阻塞 → tool result 带 <goal_blocked>。
    let recreated = router.execute_with_context(
        &call(
            "create_goal",
            json!({ "session_id": session, "objective": "把报告写完并给出结论" }),
        ),
        &human_ctx,
    );
    assert_eq!(
        recreated.status, "ok",
        "completed 目标应允许人类重建: {}",
        recreated.output
    );
    let recreated_payload: Value =
        serde_json::from_str(&recreated.output).expect("create_goal 输出必须是 JSON");
    let second_goal_id = recreated_payload["result"]["goal"]["id"]
        .as_str()
        .expect("create_goal 必须返回 goal.id")
        .to_string();

    let blocked = router.execute_with_context(
        &call(
            "update_goal",
            json!({
                "session_id": session,
                "goal_id": second_goal_id,
                "revision": 1,
                "action": "blocked",
                "blocked_reason": "上游模型连续失败"
            }),
        ),
        &goal_round_ctx,
    );
    assert_eq!(
        blocked.status, "ok",
        "goal round 必须允许 blocked: {}",
        blocked.output
    );
    let blocked_payload: Value =
        serde_json::from_str(&blocked.output).expect("update_goal 输出必须是 JSON");
    let blocked_text = blocked_payload["wrapup"]
        .as_str()
        .unwrap_or_else(|| panic!("blocked 必须注入 wrapup: {}", blocked.output));
    assert!(blocked_text.starts_with("<goal_blocked>"), "{blocked_text}");
    assert!(
        blocked_text.contains("上游模型连续失败"),
        "必须带上阻塞原因：{blocked_text}"
    );
    assert!(
        crate::agent::goal::continuation_goal_for_run(Some(session)).is_none(),
        "blocked 后宿主不得再发起 goal 轮"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

/// B170：timeoutMs=0 是纯输入校验，必须在 agent 解析/spawn 之前拦下，
/// 因此即使 agent id 不存在也应报 invalid_arguments 而非 acp_config_agent_not_found。
#[test]
fn acp_delegate_zero_timeout_checked_before_agent_resolution() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace.clone());

    let zero = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_ACP_DELEGATE.to_string(),
        arguments: json!({
            "agent": "definitely-not-a-configured-agent",
            "prompt": "x",
            "timeoutMs": 0
        }),
        plan: None,
    });
    assert_eq!(zero.status, "error");
    let payload = serde_json::from_str::<Value>(&zero.output).expect("acp_delegate error json");
    let error = payload.get("error").expect("error 键");
    assert_eq!(error.get("code").and_then(Value::as_str), Some("invalid_arguments"));
    assert!(
        error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("timeoutMs 必须大于 0"),
        "文案应说明 timeoutMs 必须大于 0：{payload}"
    );

    // 对照：非 0 值必须走到 agent 解析失败，证明前移只拦 0。
    let positive = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_ACP_DELEGATE.to_string(),
        arguments: json!({
            "agent": "definitely-not-a-configured-agent",
            "prompt": "x",
            "timeoutMs": 200
        }),
        plan: None,
    });
    assert_eq!(positive.status, "error");
    let payload = serde_json::from_str::<Value>(&positive.output).expect("acp_delegate error json");
    assert_eq!(
        payload.get("error").and_then(|e| e.get("code")).and_then(Value::as_str),
        Some("acp_config_agent_not_found")
    );

    let _ = std::fs::remove_dir_all(&workspace);
}
