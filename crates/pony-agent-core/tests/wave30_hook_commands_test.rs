//! W30a「外部命令 Hooks 执行器」红相验收测试。
//!
//! 背景：现有 `AgentHookExecutor` 只有 `NoopHookExecutor`（按 class 归一化），外部命令钩子
//! （claude-code `hooks/` / DSH `hooks-claude-code` 类比）完全缺失：无法用 shell 脚本在
//! 工具执行前后做校验/拦截/注入。W30a 要求新增 `agent::hook_commands::CommandHookExecutor`。
//!
//! 冻结契约（`docs/audit/agent-core-improvement-plan.md` §W30a）：
//! - `CommandHookSpec { command: String, timeout_ms: u64 }`；
//! - `CommandHookExecutor::{new(BTreeMap<String, CommandHookSpec>), from_json_str, from_file, is_empty}`
//!   （JSON 形态 `{"hooks":{"<hook名>":{"command":"...","timeoutMs":2000}}}`，`timeoutMs` 缺省 5000，
//!   空 command → Err）；
//! - `impl AgentHookExecutor`：
//!   1) descriptor.name 无对应 spec → 等价 Noop（`result_kind` 为 Allow/Observe 且 `blocked==false`），
//!      但 descriptor 不允许该 hook_point 时仍返回 Err；
//!   2) 有 spec → `sh -c <command>`，stdin 收到
//!      `{"hookName":...,"hookPoint":"<snake_case>","descriptorName":...}`，读取 stdout；
//!   3) `{"decision":"allow"}`→Allow/blocked=false；`{"decision":"deny"}`→Deny/blocked=true 且
//!      `structured_result` 为 `HookStructuredResult::Deny(_)`；`{"decision":"observe"}`→Observe/false；
//!   4) 非零退出码 / stdout 非法 JSON / 缺 decision → Err（message 非空）；
//!   5) 超时（> spec.timeout_ms）→ Err 且 message 含 timeout/超时，子进程必须被 kill；
//! - `build_default_hook_executor() -> Box<dyn AgentHookExecutor>`：无配置时等价 Noop。
//!
//! 红相说明：`agent::hook_commands` 模块在本文件写作时尚未被 `agent/mod.rs` 导出（实现并发落地中），
//! 首轮 `cargo test` 预期的红是**编译红**（`unresolved import ... hook_commands`）。
//! 本文件只引用冻结的公开路径与 `hooks.rs` 既有类型（不新增 descriptor 字段），
//! 以保证实现落地后立即成为有效断言。
//!
//! 时序/稳定性：临时脚本各用例独立落盘到唯一临时目录，Drop 清理；超时用例用
//! 「每 100ms 追加一行」的心跳脚本，既验证 2s 内返回 Err，也验证子进程确实被 kill
//! （返回后心跳文件不再增长），避免仅靠返回时延无法区分「kill」与「弃管」。
//!
//! 契约歧义（已回执）：t5 用「心跳循环 + sleep 0.1」替代契约示例的裸 `sleep 5`，
//! 以便确定性地断言「进程被 kill」；超时语义（200ms + 2s 内返回）不变。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pony_agent_core::agent::hook_commands::{
    build_default_hook_executor, CommandHookExecutor, CommandHookSpec,
};
use pony_agent_core::agent::hooks::{
    AgentHookDescriptor, AgentHookExecutor, HookClass, HookFailurePolicy,
    HookReplayRequirements, HookRecoveryMode, HookResultKind, HookSideEffectPersistenceRequirements,
    HookStructuredResult, HookTraceRequirements, NoopHookExecutor, TurnHookPoint,
};
use serde_json::{json, Value};

// ─────────────────────────────────────────────────────────────────────────────
// 夹具
// ─────────────────────────────────────────────────────────────────────────────

/// 唯一临时目录（Drop 清理，panic 路径也不残留脚本）。
struct TempDir {
    root: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let sequence = COUNTER.fetch_add(1, Ordering::SeqCst);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!(
            "pony-agent-w30-{tag}-{}-{nanos}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).expect("创建 W30 临时目录失败");
        let root = dir.canonicalize().unwrap_or(dir);
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    /// 写可执行 shell 脚本（executor 以 `sh -c <脚本路径>` 启动，需要可执行位）。
    fn write_script(&self, name: &str, body: &str) -> PathBuf {
        let path = self.path(name);
        fs::write(&path, body).expect("写入 W30 临时脚本失败");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&path)
                .expect("读取 W30 脚本元信息失败")
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&path, permissions).expect("设置 W30 脚本可执行位失败");
        }
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn script_arg(path: &Path) -> String {
    path.to_str()
        .expect("W30 临时脚本路径必须是合法 UTF-8")
        .to_string()
}

/// 与 `hooks.rs` 既有测试一致的 descriptor 构造（不新增字段）。
/// class=Guard + can_block=true，允许 Allow/Deny/Observe 三种结果。
fn guard_descriptor(name: &str, hook_point: TurnHookPoint) -> AgentHookDescriptor {
    AgentHookDescriptor {
        contract_version: "agent-hooks-v1".to_string(),
        name: name.to_string(),
        class: HookClass::Guard,
        priority: 100,
        timeout_ms: 5_000,
        allowed_hook_points: vec![hook_point],
        allowed_result_kinds: vec![
            HookResultKind::Allow,
            HookResultKind::Deny,
            HookResultKind::Observe,
        ],
        can_block: true,
        default_failure_policy: HookFailurePolicy::FailTurn,
        allowed_failure_policies: vec![HookFailurePolicy::FailTurn],
        default_recovery_mode: HookRecoveryMode::ReplayRequired,
        trace_requirements: HookTraceRequirements {
            include_name: true,
            include_hook_point: true,
            include_elapsed_ms: true,
            include_result_summary: true,
        },
        replay_requirements: HookReplayRequirements {
            include_hook_order: true,
            include_input_summary: true,
        },
        side_effect_persistence_requirements: HookSideEffectPersistenceRequirements {
            require_persistence_evidence: false,
            require_effect_summary: false,
        },
    }
}

fn single_spec(name: &str, command: String, timeout_ms: u64) -> CommandHookExecutor {
    let mut specs = BTreeMap::new();
    specs.insert(
        name.to_string(),
        CommandHookSpec {
            command,
            timeout_ms,
        },
    );
    CommandHookExecutor::new(specs)
}

/// 等待条件成立（10ms 轮询，不盲等）。
fn wait_until(deadline: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    loop {
        if predicate() {
            return true;
        }
        if started.elapsed() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn error_msg(outcome: &Result<pony_agent_core::agent::hooks::HookExecutionResult, String>) -> &str {
    match outcome {
        Ok(result) => panic!(
            "W30 断言失败：期望 Err，实际 Ok（result_kind={:?}, blocked={}）",
            result.result_kind, result.blocked
        ),
        Err(message) => message,
    }
}

fn is_timeout_message(message: &str) -> bool {
    let lower = message.to_lowercase();
    lower.contains("timeout") || message.contains("超时")
}

/// 是否存在 cmdline 含唯一标记的存活进程（Linux）：用于确定性断言「子进程已被 kill」。
/// 非 Linux 平台返回 false（跳过该平台专属断言）。
#[cfg(target_os = "linux")]
fn process_with_marker_exists(marker: &str) -> bool {
    let Ok(entries) = fs::read_dir("/proc") else {
        return false;
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|raw| raw.parse::<u32>().ok())
        else {
            continue;
        };
        if pid == std::process::id() {
            continue;
        }
        if let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline")) {
            if String::from_utf8_lossy(&cmdline).contains(marker) {
                return true;
            }
        }
    }
    false
}

#[cfg(not(target_os = "linux"))]
fn process_with_marker_exists(_marker: &str) -> bool {
    false
}

// ─────────────────────────────────────────────────────────────────────────────
// t1：from_json_str 解析与空 command 拒绝
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t1_json_config_parses_two_specs_and_rejects_empty_command() {
    // Arrange：两个 hook 名，各自指向「echo allow」脚本
    let workspace = TempDir::new("t1-json");
    let allow_script = script_arg(&workspace.write_script(
        "allow.sh",
        "#!/bin/sh\ncat >/dev/null\necho '{\"decision\":\"allow\",\"summary\":\"ok\"}'\n",
    ));
    let config = json!({
        "hooks": {
            "w30-alpha": { "command": allow_script, "timeoutMs": 2000 },
            "w30-beta": { "command": allow_script }
        }
    })
    .to_string();

    // Act 1：解析
    let executor = CommandHookExecutor::from_json_str(&config)
        .unwrap_or_else(|error| panic!("t1 失败：from_json_str 解析合法配置应成功，实际 Err({error})"));

    // Assert 1：非空（两条 spec 均被解析）
    assert!(
        !executor.is_empty(),
        "t1 失败：解析含 2 条 spec 的配置后 is_empty() 应为 false"
    );

    // Assert 2：两条 spec 都可执行（证明都被解析，而不是只留下一个）
    for name in ["w30-alpha", "w30-beta"] {
        let descriptor = guard_descriptor(name, TurnHookPoint::ToolCallStart);
        let outcome = executor.execute(&descriptor, TurnHookPoint::ToolCallStart);
        let result = outcome.unwrap_or_else(|error| {
            panic!("t1 失败：hook `{name}` 已配置 allow 脚本，execute 应成功，实际 Err({error})")
        });
        assert_eq!(
            result.result_kind,
            HookResultKind::Allow,
            "t1 失败：hook `{name}` 的 result_kind 应为 Allow，实际 {:?}",
            result.result_kind
        );
        assert!(
            !result.blocked,
            "t1 失败：hook `{name}` allow 决策不得 blocked=true"
        );
    }

    // Act 2 + Assert 3：空 command → Err
    let empty_command = json!({ "hooks": { "w30-empty": { "command": "   " } } }).to_string();
    assert!(
        CommandHookExecutor::from_json_str(&empty_command).is_err(),
        "t1 失败：command 为空/空白时必须返回 Err，实际 Ok"
    );
    let missing_command = json!({ "hooks": { "w30-missing": {} } }).to_string();
    assert!(
        CommandHookExecutor::from_json_str(&missing_command).is_err(),
        "t1 失败：缺少 command 字段时必须返回 Err，实际 Ok"
    );

    // Assert 4：空配置合法且为空执行器（未知顶层字段可忽略）
    let empty_executor = CommandHookExecutor::from_json_str(r#"{"hooks":{},"unknownTopLevel":123}"#)
        .expect("t1 失败：空 hooks 配置应解析成功");
    assert!(
        empty_executor.is_empty(),
        "t1 失败：空 hooks 配置解析后 is_empty() 应为 true"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t2：allow 决策
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t2_allow_decision_returns_allow_and_not_blocked() {
    // Arrange
    let workspace = TempDir::new("t2-allow");
    let script = script_arg(&workspace.write_script(
        "allow.sh",
        "#!/bin/sh\ncat >/dev/null\necho '{\"decision\":\"allow\",\"summary\":\"ok\"}'\n",
    ));
    let executor = single_spec("w30-allow", script, 5_000);
    let descriptor = guard_descriptor("w30-allow", TurnHookPoint::ToolCallStart);

    // Act
    let outcome = executor.execute(&descriptor, TurnHookPoint::ToolCallStart);
    println!("[W30][t2] outcome = {outcome:?}");

    // Assert
    let result = outcome.unwrap_or_else(|error| {
        panic!("t2 失败：allow 脚本执行应成功，实际 Err({error})")
    });
    assert_eq!(
        result.result_kind,
        HookResultKind::Allow,
        "t2 失败：{{\"decision\":\"allow\"}} 应映射为 result_kind=Allow，实际 {:?}",
        result.result_kind
    );
    assert!(
        !result.blocked,
        "t2 失败：allow 决策不得 blocked=true，实际 blocked={}",
        result.blocked
    );
    assert!(
        matches!(result.structured_result, HookStructuredResult::Allow { .. }),
        "t2 失败：structured_result 应为 HookStructuredResult::Allow，实际 {:?}",
        result.structured_result
    );
    assert_eq!(
        result.hook_name, descriptor.name,
        "t2 失败：hook_name 应回显 descriptor.name=\"{}\"，实际 \"{}\"",
        descriptor.name, result.hook_name
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t3：deny 决策
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t3_deny_decision_returns_blocked_deny_structured_result() {
    // Arrange
    let workspace = TempDir::new("t3-deny");
    let script = script_arg(&workspace.write_script(
        "deny.sh",
        "#!/bin/sh\ncat >/dev/null\necho '{\"decision\":\"deny\",\"reason\":\"forbidden by w30\"}'\n",
    ));
    let executor = single_spec("w30-deny", script, 5_000);
    let descriptor = guard_descriptor("w30-deny", TurnHookPoint::ToolCallStart);

    // Act
    let outcome = executor.execute(&descriptor, TurnHookPoint::ToolCallStart);
    println!("[W30][t3] outcome = {outcome:?}");

    // Assert
    let result = outcome.unwrap_or_else(|error| {
        panic!("t3 失败：deny 脚本应返回 Ok（Deny 是正常决策，不是错误），实际 Err({error})")
    });
    assert!(
        result.blocked,
        "t3 失败：deny 决策必须 blocked=true，实际 blocked={}，result_kind={:?}",
        result.blocked, result.result_kind
    );
    assert_eq!(
        result.result_kind,
        HookResultKind::Deny,
        "t3 失败：deny 决策应映射为 result_kind=Deny，实际 {:?}",
        result.result_kind
    );
    assert!(
        matches!(result.structured_result, HookStructuredResult::Deny(_)),
        "t3 失败：structured_result 必须是 HookStructuredResult::Deny(_)，实际 {:?}",
        result.structured_result
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t4：协议违规 —— 非法 JSON / 非零退出码 / 缺 decision 都必须 Err
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t4_invalid_protocol_outputs_return_errors() {
    // Arrange：三类坏输出
    let workspace = TempDir::new("t4-invalid");
    let cases: Vec<(&str, String)> = vec![
        (
            "非法 JSON",
            script_arg(&workspace.write_script(
                "bad_json.sh",
                "#!/bin/sh\ncat >/dev/null\necho 'this is not json'\n",
            )),
        ),
        (
            "非零退出码",
            script_arg(&workspace.write_script(
                "exit_nonzero.sh",
                "#!/bin/sh\ncat >/dev/null\nexit 3\n",
            )),
        ),
        (
            "缺 decision",
            script_arg(&workspace.write_script(
                "no_decision.sh",
                "#!/bin/sh\ncat >/dev/null\necho '{\"summary\":\"missing decision\"}'\n",
            )),
        ),
    ];

    let mut violations: Vec<String> = Vec::new();

    for (label, script) in cases {
        let hook_name = format!("w30-invalid-{}", label.replace(' ', "-"));
        let executor = single_spec(&hook_name, script, 5_000);
        let descriptor = guard_descriptor(&hook_name, TurnHookPoint::ToolCallStart);

        // Act
        let outcome = executor.execute(&descriptor, TurnHookPoint::ToolCallStart);
        println!("[W30][t4][{label}] outcome = {outcome:?}");

        // Assert：必须 Err 且 message 非空
        match outcome {
            Ok(result) => violations.push(format!(
                "[{label}] 期望 Err，实际 Ok（result_kind={:?}, blocked={}）",
                result.result_kind, result.blocked
            )),
            Err(message) => {
                if message.trim().is_empty() {
                    violations.push(format!("[{label}] 返回 Err 但 message 为空"));
                }
            }
        }
    }

    assert!(
        violations.is_empty(),
        "t4 失败：{} 类协议违规未按 W30a 契约返回 Err（message 非空）。\n实际违规：\n- {}",
        violations.len(),
        violations.join("\n- ")
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t5：超时 —— 2s 内返回 Err（message 含 timeout/超时），且子进程被 kill
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t5_timeout_returns_error_promptly_and_kills_child() {
    // Arrange：契约示例的 `sleep` 长命令；用 `exec` 让 sh 自身被替换为目标进程，
    // 于是「kill 直接子进程」等价于「kill 该命令」；sleep 时长带唯一小数标记，
    // 便于用 /proc cmdline 确定性地判定该进程是否已消失（不依赖时间窗猜测）。
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let sleep_marker = format!("5.{:06}", nanos % 1_000_000);
    let executor = single_spec("w30-timeout", format!("exec sleep {sleep_marker}"), 200);
    let descriptor = guard_descriptor("w30-timeout", TurnHookPoint::ToolCallStart);

    // Act
    let started = Instant::now();
    let outcome = executor.execute(&descriptor, TurnHookPoint::ToolCallStart);
    let elapsed = started.elapsed();
    println!(
        "[W30][t5] elapsed={elapsed:?} sleep_marker={sleep_marker} outcome={outcome:?}"
    );

    // Assert 1：2s 内返回（不得挂死）
    assert!(
        elapsed < Duration::from_secs(2),
        "t5 失败：spec.timeout_ms=200 时应在 2s 内返回，实际耗时 {elapsed:?}"
    );

    // Assert 2：错误信息含 timeout/超时
    let message = error_msg(&outcome).to_string();
    assert!(
        is_timeout_message(&message),
        "t5 失败：超时错误的 message 应含 \"timeout\" 或 \"超时\"，实际 message={message:?}"
    );

    // Assert 3：进程必须被 kill —— 返回后不得再观察到 `sleep <marker>` 进程存活
    // （非 Linux 平台该探测恒为 false，等于跳过此平台专属断言）
    assert!(
        wait_until(Duration::from_secs(1), || !process_with_marker_exists(&sleep_marker)),
        "t5 失败：超时返回 1s 后仍能观察到 `exec sleep {sleep_marker}` 进程存活，\
         说明超时只返回了 Err 而没有 kill 子进程"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t6：未知 hook 名等价 Noop（但 hook_point 校验仍在）
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t6_unknown_hook_name_falls_back_to_noop_semantics() {
    // Arrange：executor 只配置了 "w30-configured"，descriptor 用另一个名字
    let workspace = TempDir::new("t6-noop");
    let script = script_arg(&workspace.write_script(
        "allow.sh",
        "#!/bin/sh\ncat >/dev/null\necho '{\"decision\":\"allow\",\"summary\":\"ok\"}'\n",
    ));
    let executor = single_spec("w30-configured", script, 5_000);
    let descriptor = guard_descriptor("w30-unconfigured", TurnHookPoint::ToolCallStart);

    // Act 1：未知 hook 名 → Noop 等价
    let outcome = executor.execute(&descriptor, TurnHookPoint::ToolCallStart);
    let noop_outcome = NoopHookExecutor.execute(&descriptor, TurnHookPoint::ToolCallStart);
    println!("[W30][t6] command_executor={outcome:?} noop={noop_outcome:?}");

    // Assert 1：与 NoopHookExecutor 行为一致
    let result = outcome.unwrap_or_else(|error| {
        panic!("t6 失败：未配置 spec 的 descriptor 应走 Noop 路径并返回 Ok，实际 Err({error})")
    });
    let noop_result = noop_outcome.expect("t6 前置失败：NoopHookExecutor 对同一 descriptor 应返回 Ok");
    assert!(
        !result.blocked,
        "t6 失败：未知 hook 名不得 blocked=true，实际 blocked={}",
        result.blocked
    );
    assert!(
        matches!(result.result_kind, HookResultKind::Allow | HookResultKind::Observe),
        "t6 失败：未知 hook 名的 result_kind 应为 Allow 或 Observe（Noop 归一化），实际 {:?}",
        result.result_kind
    );
    assert_eq!(
        result.result_kind, noop_result.result_kind,
        "t6 失败：未知 hook 名应等价 NoopHookExecutor —— 期望 result_kind={:?}，实际 {:?}",
        noop_result.result_kind, result.result_kind
    );
    assert_eq!(
        result.blocked, noop_result.blocked,
        "t6 失败：未知 hook 名应等价 NoopHookExecutor —— 期望 blocked={}，实际 {}",
        noop_result.blocked, result.blocked
    );

    // Act 2：未知 hook 名 + 不允许的 hook_point → 仍必须 Err（沿用 Noop 校验）
    let mut disallowed = guard_descriptor("w30-unconfigured", TurnHookPoint::ToolCallStart);
    disallowed.allowed_hook_points = vec![TurnHookPoint::ModelCallStart];
    let disallowed_outcome = executor.execute(&disallowed, TurnHookPoint::ToolCallStart);

    // Assert 2
    assert!(
        disallowed_outcome.is_err(),
        "t6 失败：descriptor 不允许该 hook_point 时必须返回 Err（沿用 Noop 校验），实际 Ok（result_kind={:?}）",
        disallowed_outcome.as_ref().map(|result| format!("{:?}", result.result_kind)).unwrap_or_else(|_| "-".to_string())
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// t7：stdin payload 契约（hookName / descriptorName / hookPoint）
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t7_stdin_payload_contains_hook_name_and_snake_case_hook_point() {
    // Arrange：脚本把 stdin 原样落盘，再回 allow
    let workspace = TempDir::new("t7-payload");
    let captured = workspace.path("stdin.json");
    let captured_arg = script_arg(&captured);
    let script = script_arg(&workspace.write_script(
        "capture.sh",
        &format!(
            "#!/bin/sh\ncat > '{captured_arg}'\necho '{{\"decision\":\"allow\",\"summary\":\"captured\"}}'\n"
        ),
    ));
    let executor = single_spec("w30-payload", script, 5_000);
    let descriptor = guard_descriptor("w30-payload", TurnHookPoint::ToolCallStart);

    // Act
    let outcome = executor.execute(&descriptor, TurnHookPoint::ToolCallStart);
    let result = outcome.unwrap_or_else(|error| {
        panic!("t7 失败：payload 捕获脚本应正常返回 allow，实际 Err({error})")
    });
    assert_eq!(
        result.result_kind,
        HookResultKind::Allow,
        "t7 前置失败：捕获脚本应回 allow，实际 {:?}",
        result.result_kind
    );

    // Assert 1：stdin 文件必须被写入（数据流必须真的送达子进程 stdin）
    assert!(
        wait_until(Duration::from_secs(2), || captured.exists() && fs::metadata(&captured).map(|meta| meta.len() > 0).unwrap_or(false)),
        "t7 失败：脚本未把 stdin 写入文件 {}（期望 executor 向子进程 stdin 写入 JSON payload）",
        captured.display()
    );
    let raw = fs::read_to_string(&captured).expect("t7 失败：读取 stdin payload 文件失败");
    println!("[W30][t7] stdin payload = {raw:?}");
    let payload: Value = serde_json::from_str(raw.trim()).unwrap_or_else(|error| {
        panic!("t7 失败：stdin payload 不是合法 JSON（{error}）：{raw:?}")
    });

    // Assert 2：hookName == descriptor.name
    assert_eq!(
        payload.get("hookName").and_then(Value::as_str),
        Some(descriptor.name.as_str()),
        "t7 失败：stdin payload.hookName 应为 descriptor.name=\"{}\"，实际 payload={}",
        descriptor.name,
        payload
    );

    // Assert 3：descriptorName == descriptor.name
    assert_eq!(
        payload.get("descriptorName").and_then(Value::as_str),
        Some(descriptor.name.as_str()),
        "t7 失败：stdin payload.descriptorName 应为 \"{}\"，实际 payload={}",
        descriptor.name,
        payload
    );

    // Assert 4：hookPoint 为字符串（snake_case）
    let hook_point = payload.get("hookPoint").and_then(Value::as_str);
    match hook_point {
        Some(label) => {
            assert!(
                !label.trim().is_empty(),
                "t7 失败：payload.hookPoint 不得为空字符串，实际 payload={}",
                payload
            );
            assert!(
                label.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_'),
                "t7 失败：payload.hookPoint 应为 snake_case 字符串，实际 \"{label}\""
            );
            assert_eq!(
                label, "tool_call_start",
                "t7 失败：TurnHookPoint::ToolCallStart 的 snake_case 标签应为 \"tool_call_start\"，实际 \"{label}\""
            );
        }
        None => panic!(
            "t7 失败：payload.hookPoint 缺失或不是字符串，实际 payload={}",
            payload
        ),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// t8：from_file 与 build_default_hook_executor 接线
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t8_from_file_and_default_executor_wiring() {
    // Arrange：临时 hooks.json（唯一 hook 名，避免与环境里的 .pony/hooks.json 冲突）
    let workspace = TempDir::new("t8-file");
    let script = script_arg(&workspace.write_script(
        "allow.sh",
        "#!/bin/sh\ncat >/dev/null\necho '{\"decision\":\"allow\",\"summary\":\"ok\"}'\n",
    ));
    let config_path = workspace.path("hooks.json");
    fs::write(
        &config_path,
        json!({ "hooks": { "w30-file": { "command": script, "timeoutMs": 2000 } } }).to_string(),
    )
    .expect("t8 失败：写入临时 hooks.json 失败");

    // Act 1：from_file
    let from_file = CommandHookExecutor::from_file(&config_path).unwrap_or_else(|error| {
        panic!(
            "t8 失败：from_file({}) 应成功，实际 Err({error})",
            config_path.display()
        )
    });

    // Assert 1：从文件加载的 spec 可执行
    assert!(
        !from_file.is_empty(),
        "t8 失败：from_file 加载含 1 条 spec 的配置后 is_empty() 应为 false"
    );
    let descriptor = guard_descriptor("w30-file", TurnHookPoint::ToolCallStart);
    let result = from_file
        .execute(&descriptor, TurnHookPoint::ToolCallStart)
        .unwrap_or_else(|error| {
            panic!("t8 失败：from_file 加载的 hook 应可执行 allow 脚本，实际 Err({error})")
        });
    assert_eq!(
        result.result_kind,
        HookResultKind::Allow,
        "t8 失败：from_file 加载的 hook 应返回 Allow，实际 {:?}",
        result.result_kind
    );

    // Act 2：build_default_hook_executor（无配置时应等价 Noop）
    let default_executor: Box<dyn AgentHookExecutor> = build_default_hook_executor();
    let unconfigured_name = format!(
        "w30-default-probe-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0)
    );
    let probe = guard_descriptor(&unconfigured_name, TurnHookPoint::ToolCallStart);
    let probe_outcome = default_executor.execute(&probe, TurnHookPoint::ToolCallStart);

    // Assert 2：默认执行器对未配置的 hook 走 Noop 语义（Ok + blocked=false）
    let probe_result = probe_outcome.unwrap_or_else(|error| {
        panic!("t8 失败：build_default_hook_executor() 对未配置 hook 应等价 Noop（Ok），实际 Err({error})")
    });
    assert!(
        !probe_result.blocked,
        "t8 失败：默认执行器 Noop 路径不得 blocked=true，实际 blocked={}",
        probe_result.blocked
    );
    assert!(
        matches!(
            probe_result.result_kind,
            HookResultKind::Allow | HookResultKind::Observe
        ),
        "t8 失败：默认执行器 Noop 路径的 result_kind 应为 Allow/Observe，实际 {:?}",
        probe_result.result_kind
    );
}
