//! `workspace_run_command`：沙箱门禁、危险命令拦截、超时与 Windows 批处理封装。
use super::*;

/// P0-3：退出码分类的 7 条映射 + 边界（0 / 负数 / 普通失败码）。
///
/// 锁住「分类口径」本身：与 `run_exit_code_class_test`（真实进程黑盒）互补，
/// 这条纯函数单测保证常量表不被无意改掉，且兜底分支永不返回空串。
#[test]
fn classify_exit_code_maps_known_codes_and_falls_back() {
    let cases: [(i32, &str); 11] = [
        (101, "compile_or_panic"),
        (127, "command_not_found"),
        (126, "not_executable"),
        (130, "interrupted"),
        (137, "killed"),
        (143, "terminated"),
        (7, "general_failure"),
        (1, "general_failure"),
        (255, "general_failure"),
        (0, "general_failure"),  // 成功码不参与分类：兜底而非 panic
        (-1, "general_failure"), // 异常/平台自定义码同样兜底
    ];
    for (code, expected) in cases {
        let actual = classify_exit_code(code);
        eprintln!("[p0-3] classify_exit_code({code}) = {actual}");
        assert_eq!(actual, expected, "P0-3：退出码 {code} 分类不符");
        assert!(!actual.is_empty(), "P0-3：分类名不得为空串");
    }
}

#[test]
fn run_command_rejects_authorized_external_cwd() {
    // PA-080 code-review P1-1 回归：Run 的 cwd 必须是 workspace 内或受控 tmp，
    // 授权的外部目录不得作为命令工作目录。
    let workspace = temp_workspace();
    let external_dir =
        std::env::temp_dir().join(format!("pa080-run-cwd-external-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&external_dir);
    std::fs::create_dir_all(&external_dir).expect("create external dir");

    let authorize_store = crate::agent::path_permission::AuthorizeStore::new();
    authorize_store
        .grant(external_dir.clone())
        .expect("grant read authorization");
    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_authorize_store(std::sync::Arc::new(authorize_store))
        .with_sandbox_backend(crate::agent::sandbox::TestSandboxBackend::available());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
        arguments: json!({
            "command": "echo hi",
            "cwd": external_dir.display().to_string(),
            "timeoutMs": 5000
        }),
        plan: None,
    });
    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
    let code = payload
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert_eq!(
        code, "outside_workspace_write_denied",
        "run cwd must be rejected with outside_workspace_write_denied, got: {code}"
    );

    let _ = std::fs::remove_dir_all(&external_dir);
    let _ = std::fs::remove_dir_all(&workspace);
}

#[test]
fn run_command_returns_stdout_and_exit_code() {
    let workspace = temp_workspace();
    // Legacy Run fails closed without a sandbox backend (phase-5 review P1-2), so this
    // success path registers an explicitly available backend.
    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_sandbox_backend(crate::agent::sandbox::TestSandboxBackend::available());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
        arguments: json!({
            "command": "echo hello",
            "cwd": ".",
            "timeoutMs": 5000
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    assert_eq!(result.tool_name, TOOL_WORKSPACE_RUN_COMMAND);
    let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
    assert_eq!(
        payload.get("delegateTool").and_then(Value::as_str),
        Some("run_shell")
    );
    assert_eq!(payload.get("exitCode").and_then(Value::as_i64), Some(0));
    assert!(payload
        .get("stdout")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase()
        .contains("hello"));
}

#[test]
fn run_command_denies_high_risk_commands() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
        arguments: json!({
            "command": "rm -rf ."
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("command_denied")
    );
}

#[test]
fn run_command_denies_rm_with_split_flags() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
        arguments: json!({
            "command": "rm -r -f ."
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("command_denied")
    );
}

#[test]
fn run_command_returns_error_for_non_zero_exit_code() {
    let workspace = temp_workspace();
    // Legacy Run fails closed without a sandbox backend (phase-5 review P1-2), so this
    // success path registers an explicitly available backend.
    let router = ToolRouter::with_workspace_root(workspace)
        .with_sandbox_backend(crate::agent::sandbox::TestSandboxBackend::available());
    let command = if cfg!(windows) { "exit /b 7" } else { "exit 7" };

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
        arguments: json!({
            "command": command
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
    assert_eq!(payload.get("exitCode").and_then(Value::as_i64), Some(7));
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("non_zero_exit")
    );
}

#[test]
fn run_command_returns_timeout_error_when_command_exceeds_deadline() {
    // 契约 v5 / CT-P6：本用例锁的是「promote 关闭」时的波次一超时错误路径（到期必须
    // `error.code=timeout`），而 promote 默认开启会把同一输入变成 ok+promoted。
    // 故在用例内 pin 开关为关闭，并在退出（含 panic）时复位，避免污染同二进制的其它用例。
    struct PromoteOffPin;
    impl PromoteOffPin {
        fn new() -> Self {
            std::env::set_var(RUN_PROMOTE_ON_TIMEOUT_ENV, "0");
            Self
        }
    }
    impl Drop for PromoteOffPin {
        fn drop(&mut self) {
            std::env::remove_var(RUN_PROMOTE_ON_TIMEOUT_ENV);
        }
    }

    let _promote_off = PromoteOffPin::new();
    let workspace = temp_workspace();
    // Legacy Run fails closed without a sandbox backend (phase-5 review P1-2), so this
    // success path registers an explicitly available backend.
    let router = ToolRouter::with_workspace_root(workspace)
        .with_sandbox_backend(crate::agent::sandbox::TestSandboxBackend::available());
    let slow_command = if cfg!(windows) {
        "ping -n 6 127.0.0.1"
    } else {
        "sleep 5"
    };

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
        arguments: json!({
            "command": slow_command,
            "cwd": ".",
            "timeoutMs": 200
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("timeout")
    );
}

/// ADR 0055 P1-6：Windows 9009（命令不存在）必须附纠错提示，其它平台/退出码不得污染消息。
#[test]
fn windows_command_not_found_hint_only_for_9009_on_windows() {
    let hint = windows_command_not_found_hint(true, Some(9009), "grep -rn foo .")
        .expect("Windows + 9009 必须给出提示");
    assert!(hint.contains("9009"));
    assert!(hint.contains("findstr") && hint.contains("dir"));
    assert!(windows_command_not_found_hint(false, Some(9009), "grep -rn foo .").is_none());
    assert!(windows_command_not_found_hint(true, Some(1), "false").is_none());
    assert!(windows_command_not_found_hint(true, None, "false").is_none());
}

#[test]
fn classify_timeout_hint_classifies_network_commands_and_keeps_default() {
    let network_hint = classify_timeout_hint("git push origin main");
    for marker in [
        "网络/认证/代理",
        "git ls-remote",
        "git config --get remote.origin.url",
        "SSH 身份自检",
    ] {
        assert!(
            network_hint.contains(marker),
            "网络类超时 hint 必须含 `{marker}`：{network_hint}"
        );
    }
    for command in [
        "git fetch --all",
        "git clone https://example.com/x.git",
        "git pull",
        "git ls-remote origin",
    ] {
        assert!(
            classify_timeout_hint(command).contains("网络/认证/代理"),
            "`{command}` 必须命中网络类分类"
        );
    }

    let default_hint = classify_timeout_hint("sleep 5");
    assert_eq!(
        default_hint,
        "长任务请改用后台执行 `job_start`（`job_output` 取增量输出、`job_kill` 终止，无超时上限）；或显式传入更大的 timeoutMs（上限 600000）；仍不足时缩短或拆分命令。",
        "非网络类超时 hint 必须逐字等于 CT-5 默认文案（后台执行 + 上限 600000）"
    );
}

#[test]
fn posix_inline_env_assignment_reason_detects_only_leading_assignment_with_command() {
    for command in [
        "FOO=bar echo hi",
        "FOO=\"a b\" echo hi",
        "FOO='a b' echo hi",
    ] {
        let reason = posix_inline_env_assignment_reason(command)
            .unwrap_or_else(|| panic!("`{command}` 必须被识别为行首内联 env 赋值"));
        assert!(
            reason.contains("set VAR=value && cmd"),
            "纠正提示必须给出 `set VAR=value && cmd` 形式：{reason}"
        );
    }

    for command in [
        "echo FOO=bar",
        "FOO=bar",
        "--flag=value echo hi",
        "FOO=\"unterminated echo hi",
        "   ",
    ] {
        assert!(
            posix_inline_env_assignment_reason(command).is_none(),
            "`{command}` 不得被识别为行首内联 env 赋值后接命令"
        );
    }
}

#[test]
fn run_command_fails_closed_when_no_sandbox_backend_is_registered() {
    let workspace = temp_workspace();
    // Phase-5 review P1-2: a legacy Run with NO backend registered must fail closed
    // (`sandbox_denied` via `NoSandboxBackend`), never silently downgrade to a
    // full-parent-environment shell.
    let router = ToolRouter::with_workspace_root(workspace);

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
        arguments: json!({
            "command": "echo should-not-run",
            "cwd": ".",
            "timeoutMs": 5000
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("sandbox_denied")
    );
}

#[test]
fn run_command_fails_closed_when_a_sandbox_backend_is_registered_and_unavailable() {
    let workspace = temp_workspace();
    // With an explicitly registered unavailable sandbox backend the legacy Run path fails
    // closed (sandbox_denied). With no backend registered it also fails closed (P1-2); the
    // governed dispatcher enforces the sandbox gate for the governed path.
    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_sandbox_backend(crate::agent::sandbox::TestSandboxBackend::unavailable());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
        arguments: json!({
            "command": "echo hello",
            "cwd": ".",
            "timeoutMs": 5000
        }),
        plan: None,
    });

    assert_eq!(result.status, "error");
    let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("sandbox_denied")
    );
}

#[test]
fn run_command_executes_when_a_registered_sandbox_backend_is_available() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace)
        .with_sandbox_backend(crate::agent::sandbox::TestSandboxBackend::available());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
        arguments: json!({
            "command": "echo hello",
            "cwd": ".",
            "timeoutMs": 5000
        }),
        plan: None,
    });

    assert_eq!(result.status, "ok");
    let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
    assert_eq!(payload.get("exitCode").and_then(Value::as_i64), Some(0));
    assert!(payload
        .get("stdout")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_lowercase()
        .contains("hello"));
}

#[test]
fn windows_batch_script_emits_expected_lines_verbatim() {
    // A1：纯函数生成内容含 @echo off（首行）、cd /d "<cwd>"、命令原文（双引号原样保留）、
    // exit /b %errorlevel%。
    let script = build_windows_batch_script(
        "powershell -NoProfile -Command \"Write-Output 'hello'\"",
        r"C:\Users\test\work",
    );
    let lines: Vec<&str> = script.lines().collect();
    assert!(!lines.is_empty(), "脚本不能为空: {script:?}");
    assert_eq!(lines[0], "@echo off", "首行必须是 @echo off: {script:?}");
    assert!(
        script.contains("cd /d \"C:\\Users\\test\\work\""),
        "必须包含双引号包裹的 cd /d: {script:?}"
    );
    assert!(
        script.contains("powershell -NoProfile -Command \"Write-Output 'hello'\""),
        "命令必须原样保留（含双引号，不得改写为 \\\"）: {script:?}"
    );
    assert!(
        !script.contains("\\\""),
        "内嵌双引号不得被改写为 \\\"（CommandLineToArgvW 转义会损坏 cmd 解析）: {script:?}"
    );
    assert!(
        script.contains("exit /b %errorlevel%"),
        "末行必须为 exit /b %errorlevel%: {script:?}"
    );
}

#[test]
fn windows_batch_script_tolerates_consoleless_chcp() {
    // 弹窗修复配套：子进程经 CREATE_NO_WINDOW 隐藏后无控制台，chcp 失败会写 stderr；
    // 必须 `>nul 2>&1` 静默吞掉，避免污染工具输出（"chcp" 报错混进 stderr）。
    let script = build_windows_batch_script("echo ok", r"C:\w");
    assert!(
        script.contains("chcp 65001 >nul 2>&1"),
        "chcp 必须同时重定向 stderr: {script:?}"
    );
}

#[test]
fn windows_batch_script_keeps_real_world_powershell_cpu_mem_command_verbatim() {
    // 真实事故回归（2026-10-07）：用户用 Run 查询主机 CPU/内存的 PowerShell 命令。
    // 旧实现（cmd /C "cd /d … && <command>" 单参数内联）下，CommandLineToArgvW 把内嵌
    // 双引号转义为 \"，cmd 按自己的规则剥离 → 引号失衡、`|`/`&`/`;` 被 cmd 误切分：
    // PowerShell 收到残缺命令退出码 1（第一次调用），或等待 stdin 输入挂起到超时
    // （第二次调用）。批处理方案必须逐字保留该命令——含引号、管道、`%`、中文全部原样。
    let command = "powershell -NoProfile -Command \"$cpu = (Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average; $os = Get-CimInstance Win32_OperatingSystem; $totalMem = $os.TotalVisibleMemorySize / 1MB; $freeMem = $os.FreePhysicalMemory / 1MB; $usedMem = $totalMem - $freeMem; $usedPct = [math]::Round(($usedMem / $totalMem) * 100, 2); Write-Output ('CPU使用率: {0}%' -f $cpu); Write-Output ('内存: 总计 {0} GB, 已用 {1} GB, 空闲 {2} GB, 使用率 {3}%' -f [math]::Round($totalMem,2), [math]::Round($usedMem,2), [math]::Round($freeMem,2), $usedPct)\"";
    let script = build_windows_batch_script(command, r"D:\Documents\pony-agent\src-tauri");
    assert!(
        script.contains(command),
        "事故命令必须逐字保留（含引号/管道/中文/% 不变形）: {script:?}"
    );
    assert!(
        !script.contains("\\\""),
        "内嵌双引号不得被改写为 \\\"（CommandLineToArgvW 转义会损坏 cmd 解析）: {script:?}"
    );
    assert!(
        script.contains("cd /d \"D:\\Documents\\pony-agent\\src-tauri\" || exit /b 1"),
        "cwd 必须保持双引号包裹且 cd 失败短路退出: {script:?}"
    );
    // 行结构锁定：@echo off / chcp / cd / 命令 / exit —— 命令体绝不回到 cmd /C 行。
    assert_eq!(
        script.lines().count(),
        5,
        "批处理必须恰好 5 行（命令体只在批处理文件内）: {script:?}"
    );
}

#[test]
fn windows_batch_script_preserves_cmd_metacharacters_verbatim() {
    // A2：含 & | > " 的命令逐字保留（不做 cmd 转义、不 caret 转义、不改写引号）。
    let command = "dir \"C:\\My Dir\" & echo %ERRORLEVEL% | findstr /v \"x\" > out.txt";
    let script = build_windows_batch_script(command, r"C:\w");
    assert!(
        script.contains(command),
        "& | > \" 等元字符必须逐字保留: {script:?}"
    );
    assert!(!script.contains("^&"), "& 不得被 ^ 转义: {script:?}");
    assert!(!script.contains("^|"), "| 不得被 ^ 转义: {script:?}");
    assert!(!script.contains("^>"), "> 不得被 ^ 转义: {script:?}");
    assert!(!script.contains("\\\""), "\" 不得被改写为 \\\": {script:?}");
    assert!(
        script.contains("cd /d \"C:\\w\""),
        "cwd 保持双引号包裹: {script:?}"
    );
}

#[test]
fn workspace_command_parts_non_windows_branch_is_unchanged() {
    // A3 回归：非 Windows 平台（Linux/macOS sh -lc 分支）行为不变——
    // ("sh", ["-lc", "cd '<cwd>' && <command>"])。Windows 上本分支不生效，跳过。
    if cfg!(windows) {
        return;
    }
    let cwd = PathBuf::from("/tmp/pony-agent-some-workspace");
    let (program, args) = workspace_command_parts("echo hello", &cwd);
    assert_eq!(program, "sh");
    assert_eq!(
        args,
        vec![
            "-lc".to_string(),
            "cd '/tmp/pony-agent-some-workspace' && echo hello".to_string()
        ]
    );
}

/// CI windows-latest 集成测试（本机 Linux 不编译/不执行，需 CI 验证）：
/// Run 执行含双引号的 powershell 命令后，exitCode==0、stdout 为真实输出（含 hello）
/// 且不包含命令文本回显（不含 "Write-Output"）；临时 .cmd 批处理文件运行后被删除。
/// 稳定化说明：close_stdin（fa52bc6）已修确定性挂起；CI runner 负载相关的间歇性
/// 慢启动仍偶发，timeoutMs 上调至工具上限 120000 仅容忍负载方差，断言语义不变。
/// 若失败消息含"进程树清理失败：unknown process handle"，是 kill_after 计时器与
/// poll 死线双杀竞态的噪声（超时路径已 double-kill 兜底），非本测试问题。
#[cfg(windows)]
#[test]
fn windows_run_powershell_command_with_quotes_returns_real_stdout_and_cleans_batch() {
    let workspace = temp_workspace();
    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_sandbox_backend(crate::agent::sandbox::TestSandboxBackend::available());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
        arguments: json!({
            "command": "powershell -NoProfile -Command \"Write-Output 'hello'\"",
            "cwd": ".",
            "timeoutMs": 120000
        }),
        plan: None,
    });

    assert_eq!(
        result.status, "ok",
        "Run 应成功；output: {}",
        result.output
    );
    let payload: Value =
        serde_json::from_str(&result.output).expect("run output should be json");
    assert_eq!(payload.get("exitCode").and_then(Value::as_i64), Some(0));
    let stdout = payload.get("stdout").and_then(Value::as_str).unwrap_or("");
    assert!(
        stdout.contains("hello"),
        "stdout 必须包含真实输出 hello，实际: {stdout:?}"
    );
    assert!(
        !stdout.contains("Write-Output"),
        "stdout 不得回显命令文本（旧 bug 症状）: {stdout:?}"
    );

    // 临时 .cmd 批处理文件运行结束后必须被删除（受控 tmp <root>/.tmp/ 下）。
    let tmp_dir = workspace.join(".tmp");
    let leftovers: Vec<String> = if tmp_dir.exists() {
        std::fs::read_dir(&tmp_dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().to_string())
                    .filter(|name| name.ends_with(".cmd") || name.ends_with(".bat"))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    assert!(
        leftovers.is_empty(),
        "临时批处理文件必须在运行结束后被删除: {leftovers:?}"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

/// CI windows-latest 集成测试（本机 Linux 不编译/不执行，需 CI 验证）：
/// 含空格的 workspace 根目录下，cmd /C 引号形态必须仍正确（Bug A 回归）——
/// Run 含双引号的 powershell 命令 exit 0、stdout 含 space-ws 且无命令文本回显。
/// 稳定化说明：close_stdin（fa52bc6）已修确定性挂起；CI runner 负载相关的间歇性
/// 慢启动仍偶发，timeoutMs 上调至工具上限 120000 仅容忍负载方差，断言语义不变。
/// 若失败消息含"进程树清理失败：unknown process handle"，是 kill_after 计时器与
/// poll 死线双杀竞态的噪声（超时路径已 double-kill 兜底），非本测试问题。
#[cfg(windows)]
#[test]
fn windows_run_powershell_in_workspace_root_with_spaces_returns_real_stdout() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    // 目录名显式包含空格与连字符，锁定 `cd /d "<cwd>"` 引号包裹形态。
    let workspace = std::env::temp_dir().join(format!("pony agent spaced ws {unique}"));
    fs::create_dir_all(&workspace).expect("create spaced workspace");
    let workspace = workspace.canonicalize().unwrap_or(workspace);

    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_sandbox_backend(crate::agent::sandbox::TestSandboxBackend::available());

    let result = router.execute(&ToolCall {
        call_id: None,
        name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
        arguments: json!({
            "command": "powershell -NoProfile -Command \"Write-Output 'space-ws'\"",
            "cwd": ".",
            "timeoutMs": 120000
        }),
        plan: None,
    });

    assert_eq!(
        result.status, "ok",
        "含空格 workspace 下 Run 应成功；output: {}",
        result.output
    );
    let payload: Value =
        serde_json::from_str(&result.output).expect("run output should be json");
    assert_eq!(payload.get("exitCode").and_then(Value::as_i64), Some(0));
    let stdout = payload.get("stdout").and_then(Value::as_str).unwrap_or("");
    assert!(
        stdout.contains("space-ws"),
        "stdout 必须包含真实输出 space-ws，实际: {stdout:?}"
    );
    assert!(
        !stdout.contains("Write-Output"),
        "stdout 不得回显命令文本（旧 bug 症状）: {stdout:?}"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}
