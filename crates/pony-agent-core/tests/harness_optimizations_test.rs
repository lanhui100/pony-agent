//! 验证 4 项 Harness 调优需求：
//! 1. 单轮工具调用上限提升为 500，且 MAX_ALLOWED 提升；
//! 2. TodoWrite 工具命名映射正确，且不降级为 Run；
//! 3. BatchExecute 子调用失败时，父级 Activity/Telemetry 必须包含子调用错误信息，不能为 null；
//! 4. 搜索工具默认忽略 target/、node_modules/、.git/。

use pony_agent_core::agent::runtime::{
    max_tool_followups_per_turn, parse_max_tool_followups_per_turn,
};
use pony_agent_core::agent::tools::{
    canonical_tool_name, model_visible_tool_name,
};

#[test]
fn test_tool_followups_limit_500() {
    assert_eq!(parse_max_tool_followups_per_turn(None), 500);
    assert_eq!(max_tool_followups_per_turn(), 500);
    assert_eq!(parse_max_tool_followups_per_turn(Some("1000")), 1000);
}

#[test]
fn test_todowrite_tool_mapping() {
    assert_eq!(canonical_tool_name("TodoWrite"), Some("todo_write"));
    assert_eq!(canonical_tool_name("todo_write"), Some("todo_write"));
    assert_eq!(model_visible_tool_name("todo_write"), "TodoWrite");
    assert_eq!(model_visible_tool_name("TodoWrite"), "TodoWrite");
    assert_ne!(canonical_tool_name("TodoWrite"), Some("workspace_run_command"));
}

#[test]
fn test_batch_execute_error_propagation_in_telemetry() {
    use pony_agent_core::agent::telemetry::{DefaultTurnTelemetryBuilder, TurnTelemetryBuilder};
    use pony_agent_core::agent::tools::{ToolCall, ToolResult};
    use serde_json::json;

    let telemetry = DefaultTurnTelemetryBuilder;
    let call = ToolCall {
        name: "workspace_batch".to_string(),
        arguments: json!({
            "calls": [
                {
                    "name": "workspace_read_file",
                    "arguments": { "path": "non_existent.txt" }
                }
            ]
        }),
        call_id: Some("call-1".to_string()),
        plan: None,
    };
    let result = ToolResult {
        tool_name: "workspace_batch".to_string(),
        status: "error".to_string(),
        duration_ms: 10,
        output: json!({
            "ok": false,
            "status": "error",
            "results": [
                {
                    "index": 0,
                    "tool": "workspace_read_file",
                    "ok": false,
                    "aggregateStatus": "error",
                    "error": {
                        "code": "file_not_found",
                        "message": "文件不存在: non_existent.txt"
                    }
                }
            ]
        }).to_string(),
    };

    let activities = telemetry.tool_activities_after_result(&call, &result);
    assert!(!activities.is_empty());
    let parent_activity = &activities[0];
    assert!(parent_activity.error.is_some(), "Parent activity error must not be null when subcall fails");
    let err_val = parent_activity.error.as_ref().unwrap();
    assert_eq!(err_val["kind"], "file_not_found");
    assert_eq!(err_val["message"], "文件不存在: non_existent.txt");
}

#[test]
fn test_search_engine_ignores_heavy_directories() {
    use pony_agent_core::agent::search::{SearchEngine, SearchOptions};
    use std::fs;
    let temp_dir = std::env::temp_dir().join(format!("test_harness_search_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
    fs::create_dir_all(&temp_dir).expect("create temp dir");
    let root = &temp_dir;
    
    // 创建普通文件
    fs::write(root.join("normal.txt"), "TARGET_KEYWORD_MATCH").expect("write normal");
    
    // 创建 target/, node_modules/, .git/ 并写入匹配内容
    let target_dir = root.join("target");
    fs::create_dir_all(&target_dir).expect("create target");
    fs::write(target_dir.join("build.txt"), "TARGET_KEYWORD_MATCH").expect("write target match");

    let nm_dir = root.join("node_modules");
    fs::create_dir_all(&nm_dir).expect("create node_modules");
    fs::write(nm_dir.join("pkg.txt"), "TARGET_KEYWORD_MATCH").expect("write nm match");

    let git_dir = root.join(".git");
    fs::create_dir_all(&git_dir).expect("create .git");
    fs::write(git_dir.join("config.txt"), "TARGET_KEYWORD_MATCH").expect("write git match");

    let engine = SearchEngine::new();
    let opts = SearchOptions::default();
    let result = engine.search_text("TARGET_KEYWORD_MATCH", root, &opts).expect("search text");

    let _ = fs::remove_dir_all(&temp_dir);

    assert_eq!(result.match_count, 1, "Only normal.txt should match, heavy dirs must be ignored");
    assert_eq!(result.matches[0].path, "normal.txt");
}

#[test]
fn test_tool_failure_signal_normalizes_free_text_description() {
    use pony_agent_core::agent::runtime::tool_failure_signal;
    use pony_agent_core::agent::tools::{ToolCall, ToolResult};
    use serde_json::json;

    let call1 = ToolCall {
        name: "workspace_run_command".to_string(),
        arguments: json!({
            "command": "git push origin main",
            "cwd": "/repo",
            "description": "第一次尝试推送"
        }),
        call_id: Some("call-1".to_string()),
        plan: None,
    };
    let call2 = ToolCall {
        name: "workspace_run_command".to_string(),
        arguments: json!({
            "command": "git push origin main",
            "cwd": "/repo",
            "description": "换个描述重试推送"
        }),
        call_id: Some("call-2".to_string()),
        plan: None,
    };
    let result = ToolResult {
        tool_name: "workspace_run_command".to_string(),
        status: "error".to_string(),
        duration_ms: 100,
        output: json!({
            "ok": false,
            "error": {
                "kind": "non_zero_exit",
                "message": "退出码 128"
            }
        }).to_string(),
    };

    let sig1 = tool_failure_signal(&call1, &result).expect("sig1 must exist");
    let sig2 = tool_failure_signal(&call2, &result).expect("sig2 must exist");

    assert_eq!(sig1.0, sig2.0, "Tool name must match");
    assert_eq!(sig1.1, sig2.1, "Normalized arguments fingerprint must match ignoring description");
    assert_eq!(sig1.2, sig2.2, "Error kind must match");
}

// ---------------------------------------------------------------------------
// wave-9 红相验收（P0-1 断路器指纹归一化 / P0-2 超时取证 / P1-3 Windows 内联 env）
// 契约来源：.dev-team/contract-matrix-wave9.json（T1 冻结）
// 本文件用例一经 Lead 冻结即只读，禁止 Executor 修改断言。
// ---------------------------------------------------------------------------

/// 构造一个 `error.kind == "timeout"` 的 Run 失败结果（output 为 JSON 字符串）。
fn wave9_timeout_result() -> pony_agent_core::agent::tools::ToolResult {
    use pony_agent_core::agent::tools::ToolResult;
    use serde_json::json;

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

/// 构造一个 Run 工具调用。
fn wave9_run_call(
    command: &str,
    timeout_ms: u64,
    description: &str,
    call_id: &str,
) -> pony_agent_core::agent::tools::ToolCall {
    use pony_agent_core::agent::tools::ToolCall;
    use serde_json::json;

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

/// P0-1：Run 类工具在 `timeout` 下的指纹必须对 `timeoutMs` 与行首内联 env 前缀归一化。
/// 红相：当前实现仅剔除 `description`，三次指纹互不相同。
#[test]
fn test_tool_failure_signal_normalizes_network_retry_timeout() {
    use pony_agent_core::agent::runtime::tool_failure_signal;

    let result = wave9_timeout_result();

    // 同一核心命令 `git push origin main`：仅 timeoutMs 不同。
    let call_a = wave9_run_call("git push origin main", 60_000, "第一次推送超时", "call-a");
    let call_b = wave9_run_call("git push origin main", 180_000, "加大超时重试推送", "call-b");
    // 仅行首 POSIX 内联 env 前缀不同（核心命令仍为 `git push origin main`）。
    let call_c = wave9_run_call(
        "GIT_TERMINAL_PROMPT=0 git push origin main",
        60_000,
        "关闭交互提示后重试",
        "call-c",
    );

    let sig_a = tool_failure_signal(&call_a, &result).expect("timeout 失败必须产生指纹信号");
    let sig_b = tool_failure_signal(&call_b, &result).expect("timeout 失败必须产生指纹信号");
    let sig_c = tool_failure_signal(&call_c, &result).expect("timeout 失败必须产生指纹信号");

    assert_eq!(sig_a.0, sig_b.0, "工具名必须一致");
    assert_eq!(sig_a.0, sig_c.0, "工具名必须一致");
    assert_eq!(sig_a.2, "timeout", "错误码必须为 timeout");
    assert_eq!(sig_b.2, "timeout", "错误码必须为 timeout");
    assert_eq!(sig_c.2, "timeout", "错误码必须为 timeout");

    assert_eq!(
        sig_a.1, sig_b.1,
        "仅 timeoutMs 不同的同一核心命令，指纹必须相同（否则模型加大超时重试即可绕过连续失败断路器）。\n a={}\n b={}",
        sig_a.1, sig_b.1
    );
    assert_eq!(
        sig_a.1, sig_c.1,
        "仅行首内联 env 前缀不同的同一核心命令，指纹必须相同（否则模型换前缀重试即可绕过断路器）。\n a={}\n c={}",
        sig_a.1, sig_c.1
    );
}

/// P0-1 反向护栏：不同核心命令在 timeout 下指纹必须保持不同（防归一化误杀）。
#[test]
fn test_tool_failure_signal_keeps_distinct_commands_distinct() {
    use pony_agent_core::agent::runtime::tool_failure_signal;

    let result = wave9_timeout_result();
    let call_push = wave9_run_call("git push origin main", 60_000, "推送", "call-push");
    let call_test = wave9_run_call("pnpm test", 60_000, "跑测试", "call-test");

    let sig_push = tool_failure_signal(&call_push, &result).expect("timeout 失败必须产生指纹信号");
    let sig_test = tool_failure_signal(&call_test, &result).expect("timeout 失败必须产生指纹信号");

    assert_eq!(sig_push.2, "timeout");
    assert_eq!(sig_test.2, "timeout");
    assert_ne!(
        sig_push.1, sig_test.1,
        "不同核心命令必须产生不同指纹，否则连续失败断路器会误杀正常推进的工作。\n push={}\n test={}",
        sig_push.1, sig_test.1
    );
}

/// P0-2：run_command 超时错误载荷必须保留已采集证据键。
/// 红相：当前 timeout 分支走 `error_result`，仅含 ok/tool/error{code,kind,message,hint}/summary。
#[test]
fn test_run_command_timeout_payload_keeps_evidence() {
    use pony_agent_core::agent::sandbox::TestSandboxBackend;
    use pony_agent_core::agent::tools::{ToolCall, ToolRouter};
    use serde_json::{json, Value};

    let workspace = std::env::temp_dir().join(format!(
        "wave9_timeout_evidence_{}",
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
            "timeoutMs": 200,
            "description": "wave9 超时取证"
        }),
        call_id: Some("call-timeout-evidence".to_string()),
        plan: None,
    };

    let result = router.execute(&call);
    let _ = std::fs::remove_dir_all(&workspace);

    assert_eq!(result.status, "error", "超时命令必须以 error 状态返回：{}", result.output);

    let payload: Value =
        serde_json::from_str(&result.output).expect("run_command output 必须是合法 JSON 字符串");
    let error = payload.get("error").expect("载荷必须含 error 对象");
    assert_eq!(
        error.get("code").and_then(Value::as_str),
        Some("timeout"),
        "超时错误码必须保持 timeout：{}",
        payload
    );
    assert_eq!(
        error.get("kind").and_then(Value::as_str),
        Some("timeout"),
        "超时 error.kind 必须保持 timeout：{}",
        payload
    );

    // P0-2 证据键必须落在载荷**根级**（契约 frozen_signatures.error_result_with_evidence：
    // 「把 evidence 对象的键并入错误载荷根级」）：
    // stdout/stderr/stdoutTruncated/stderrTruncated/exitCode/cleanupError/timeoutMs/command
    for key in [
        "stdout",
        "stderr",
        "stdoutTruncated",
        "stderrTruncated",
        "exitCode",
        "cleanupError",
        "timeoutMs",
        "command",
    ] {
        assert!(
            payload.get(key).is_some(),
            "超时错误载荷根级 {} 证据键缺失（P0-2 要求保留已采集证据）：{}",
            key,
            payload
        );
    }
    assert!(
        payload.get("stdout").and_then(Value::as_str).is_some(),
        "根级 stdout 必须是字符串：{}",
        payload
    );
    assert!(
        payload.get("stderr").and_then(Value::as_str).is_some(),
        "根级 stderr 必须是字符串：{}",
        payload
    );
    assert!(
        payload.get("stdoutTruncated").and_then(Value::as_bool).is_some(),
        "根级 stdoutTruncated 必须是布尔：{}",
        payload
    );
    assert!(
        payload.get("stderrTruncated").and_then(Value::as_bool).is_some(),
        "根级 stderrTruncated 必须是布尔：{}",
        payload
    );
    assert_eq!(
        payload.get("timeoutMs").and_then(Value::as_u64),
        Some(200),
        "根级 timeoutMs 必须回显实际生效超时：{}",
        payload
    );
    assert_eq!(
        payload.get("command").and_then(Value::as_str),
        Some(command),
        "根级 command 必须回显原始命令：{}",
        payload
    );
    // cleanupError 在无清理错误时允许为 null，但键必须在（契约明文）。
    let cleanup_error = payload.get("cleanupError").expect("cleanupError 键必须在");
    assert!(
        cleanup_error.is_null() || cleanup_error.as_str().is_some(),
        "cleanupError 必须是 null 或字符串：{}",
        payload
    );
}

/// P0-2 负例：命令在 deadline 前正常退出时，载荷不得出现伪造的 timeout 证据。
#[test]
fn test_run_command_success_payload_has_no_timeout_evidence() {
    use pony_agent_core::agent::sandbox::TestSandboxBackend;
    use pony_agent_core::agent::tools::{ToolCall, ToolRouter};
    use serde_json::{json, Value};

    let workspace = std::env::temp_dir().join(format!(
        "wave9_timeout_negative_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).expect("create temp workspace");

    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_sandbox_backend(TestSandboxBackend::available());

    let call = ToolCall {
        name: "workspace_run_command".to_string(),
        arguments: json!({
            "command": "echo wave9-success-probe",
            "cwd": workspace.display().to_string(),
            "timeoutMs": 10_000,
            "description": "wave9 超时负例"
        }),
        call_id: Some("call-timeout-negative".to_string()),
        plan: None,
    };

    let result = router.execute(&call);
    let _ = std::fs::remove_dir_all(&workspace);

    assert_eq!(
        result.status, "ok",
        "未超时的命令必须成功返回：{}",
        result.output
    );
    let payload: Value =
        serde_json::from_str(&result.output).expect("run_command output 必须是合法 JSON 字符串");
    assert!(
        payload.get("error").map(Value::is_null).unwrap_or(true),
        "未超时的载荷不得出现错误对象：{}",
        payload
    );
    assert!(
        payload
            .get("cleanupError")
            .map(Value::is_null)
            .unwrap_or(true),
        "未超时的载荷不得出现非 null 的 cleanupError 伪证据：{}",
        payload
    );
    assert!(
        payload
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("wave9-success-probe"),
        "未超时的载荷必须保留真实 stdout：{}",
        payload
    );
}

/// P0-1 反向护栏：非 timeout、以及非 Run 类工具必须完整回退到既有
/// `canonicalize_tool_argument_value`（`timeoutMs` 仍参与指纹），防止归一化过度扩大误杀正常推进的工作。
#[test]
fn test_tool_failure_signal_falls_back_for_non_timeout_and_non_run() {
    use pony_agent_core::agent::runtime::tool_failure_signal;
    use pony_agent_core::agent::tools::{ToolCall, ToolResult};
    use serde_json::json;

    let non_zero_exit = ToolResult {
        tool_name: "workspace_run_command".to_string(),
        status: "error".to_string(),
        duration_ms: 100,
        output: json!({
            "ok": false,
            "error": { "code": "non_zero_exit", "kind": "non_zero_exit", "message": "退出码 1" }
        })
        .to_string(),
    };
    let timeout = wave9_timeout_result();

    // (1) 非 timeout：同一 command 仅 timeoutMs 不同 → 必须保留 timeoutMs 参与指纹（两指纹不等）。
    let call_short = wave9_run_call("pnpm test", 1_000, "非超时变体 A", "fallback-a");
    let call_long = wave9_run_call("pnpm test", 60_000, "非超时变体 B", "fallback-b");
    let sig_short =
        tool_failure_signal(&call_short, &non_zero_exit).expect("non_zero_exit 必须产生指纹信号");
    let sig_long =
        tool_failure_signal(&call_long, &non_zero_exit).expect("non_zero_exit 必须产生指纹信号");
    assert_eq!(sig_short.2, "non_zero_exit");
    assert_ne!(
        sig_short.1, sig_long.1,
        "非 timeout 必须回退到 canonicalize_tool_argument_value，timeoutMs 仍须参与指纹：{}",
        sig_short.1
    );

    // (2) 非 Run 类工具在 timeout 下同样必须回退（不得被归一化）。
    let read_short = ToolCall {
        name: "read_file".to_string(),
        arguments: json!({ "path": "a.txt", "timeoutMs": 1_000 }),
        call_id: Some("fallback-read-a".to_string()),
        plan: None,
    };
    let read_long = ToolCall {
        name: "read_file".to_string(),
        arguments: json!({ "path": "a.txt", "timeoutMs": 60_000 }),
        call_id: Some("fallback-read-b".to_string()),
        plan: None,
    };
    let sig_read_short =
        tool_failure_signal(&read_short, &timeout).expect("read_file timeout 必须产生指纹信号");
    let sig_read_long =
        tool_failure_signal(&read_long, &timeout).expect("read_file timeout 必须产生指纹信号");
    assert_eq!(sig_read_short.2, "timeout");
    assert_ne!(
        sig_read_short.1, sig_read_long.1,
        "非 Run 类工具必须回退到 canonicalize_tool_argument_value：{}",
        sig_read_short.1
    );

    // (3) 非 Run 类工具的参数差异同样不得被归一化抹平（回退面完整）。
    let read_env = ToolCall {
        name: "read_file".to_string(),
        arguments: json!({ "path": "FOO=1 a.txt", "timeoutMs": 1_000 }),
        call_id: Some("fallback-read-c".to_string()),
        plan: None,
    };
    let sig_read_env =
        tool_failure_signal(&read_env, &timeout).expect("read_file timeout 必须产生指纹信号");
    assert_ne!(
        sig_read_short.1, sig_read_env.1,
        "非 Run 类工具的参数差异不得被归一化抹平：{}",
        sig_read_env.1
    );
}

/// P1-3：POSIX 行首内联 env 赋值形态的命令，在非 Windows 平台必须不被拒绝（回归护栏）。
///
/// [VERIFY DEGRADED: WINDOWS-ONLY] 本机为 Linux x86_64 且无 rustup，无法触发
/// `cfg!(windows)` 分支；Windows 下「拒绝并返回 error code = windows_inline_env_unsupported」
/// 的行为在本环境不可编译、不可运行、不可断言，属于已知残余风险，不计入已验证范围。
#[test]
fn test_posix_inline_env_rejected_on_windows_semantics() {
    use pony_agent_core::agent::sandbox::TestSandboxBackend;
    use pony_agent_core::agent::tools::{ToolCall, ToolRouter};
    use serde_json::{json, Value};

    let workspace = std::env::temp_dir().join(format!(
        "wave9_inline_env_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::create_dir_all(&workspace).expect("create temp workspace");

    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_sandbox_backend(TestSandboxBackend::available());

    let call = ToolCall {
        name: "workspace_run_command".to_string(),
        arguments: json!({
            "command": "WAVE9_INLINE_ENV=1 echo wave9-inline-env-probe",
            "cwd": workspace.display().to_string(),
            "timeoutMs": 10_000,
            "description": "wave9 内联 env 平台语义"
        }),
        call_id: Some("call-inline-env".to_string()),
        plan: None,
    };

    let result = router.execute(&call);
    let _ = std::fs::remove_dir_all(&workspace);
    let payload: Value =
        serde_json::from_str(&result.output).expect("run_command output 必须是合法 JSON 字符串");

    if cfg!(windows) {
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("windows_inline_env_unsupported"),
            "Windows 下 POSIX 内联 env 形态命令必须被拒绝：{}",
            payload
        );
    } else {
        // 非 Windows：该形态是合法 POSIX 语法，严禁被 Windows 专用拦截误伤。
        assert_ne!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("windows_inline_env_unsupported"),
            "非 Windows 平台严禁以 windows_inline_env_unsupported 拒绝合法 POSIX 命令：{}",
            payload
        );
        assert_eq!(
            result.status, "ok",
            "非 Windows 平台该形态命令必须正常执行：{}",
            payload
        );
        assert!(
            payload
                .get("stdout")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("wave9-inline-env-probe"),
            "内联 env 前缀命令必须真实执行并回显探针输出：{}",
            payload
        );
    }
}

#[test]
fn test_tool_execution_measures_physical_duration_ms() {
    use pony_agent_core::agent::tools::{ToolCall, ToolRouter};
    use serde_json::json;

    let router = ToolRouter::new();
    let call = ToolCall {
        name: "time_now".to_string(),
        arguments: json!({}),
        call_id: Some("call-time-1".to_string()),
        plan: None,
    };
    let result = router.execute(&call);
    assert_eq!(result.status, "ok");
    assert_eq!(result.tool_name, "time_now");
    // 验证物理执行耗时字段存在且合理（<= 5000ms）
    assert!(result.duration_ms <= 5000);
}
