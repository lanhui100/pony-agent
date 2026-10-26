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
//!   2) 有 spec → Unix `sh -c <command>` / Windows `cmd /C <command>`（产品双平台，见
//!      `hook_commands.rs:160-170`），stdin 收到
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
//! 时序/稳定性：临时目录仅用于「stdin payload 落盘」「hooks.json 落盘」等真实文件产物，
//! 每用例唯一目录 + Drop 清理；命令本身不再落脚本文件。
//!
//! Windows 兼容（本文件曾在此处失效）：产品是**双平台**的——Windows 走 `cmd /C <command>`，
//! 其它平台走 `sh -c <command>`。旧实现把 `#!/bin/sh` 脚本的**文件路径**当命令交给
//! `cmd /C`，Windows 上必然失败（且不能用 `#![cfg(unix)]` 砍覆盖，否则 Windows 零覆盖）。
//! 现改为「内联命令串 + 平台分支」（见 `decision_command` / `invalid_json_command` /
//! `nonzero_exit_command` 与 t5/t7 内的分支），**断言一条未改**，只替换「命令怎么构造」。
//!
//! t5 超时判定说明：Unix 用 `exec sleep <唯一小数标记>`——`exec` 让 sh 自身被替换为目标进程，
//! 于是「kill 直接子进程」等价于「kill 该命令」，且唯一小数标记可用 /proc cmdline 确定性地
//! 判定进程是否已消失（不依赖时间窗猜测）；Windows 用 `ping -n 6 127.0.0.1 >nul` 充当长命令
//! （对齐 `hook_commands.rs` 单元测试既有写法），超时语义（200ms + 2s 内返回）两平台一致。

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
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

}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Unix 分支：`sh -c` 收到命令串原样执行，故用 `cat >/dev/null` 抽干 stdin
/// （对齐真实 hook「读 payload」的行为；产品对「子进程不读 stdin」已用
/// `let _ = stdin.write_all(...)` 容错），再用 `printf` 精确输出文本（不追加换行）。
fn unix_output_command(text: &str) -> String {
    format!("cat >/dev/null; printf '{text}'")
}

/// Windows 分支：把文本放进数据文件，用 PowerShell 读回。
///
/// **为什么不能 `echo {"decision":"allow"}`**：产品以
/// `Command::new("cmd").arg("/C").arg(cmd)` 启动（`hook_commands.rs:159-161`），而 Rust std 在
/// Windows 上构造命令行时会给含空格/制表符的 argv 加外层引号，并且**无条件**把参数内每个 `"`
/// 转义成 `\"`（`library/std/src/sys/args/windows.rs::append_arg`：`Quote::Auto` 只在含空格时
/// 加引号，但 `escape` 恒为 `true`）。于是 `echo {"decision":"allow"}` 在 cmd 眼里是
/// `"echo {\"decision\":\"allow\"}"`，按 cmd `/C` 的「剥首尾引号」规则执行后变成
/// `echo {\"decision\":\"allow\"}`，echo 把反斜杠原样打印 → stdout 不是合法 JSON。
/// 结论：**Windows 命令串里不能出现任何 `"`**。故 JSON 一律先落数据文件，再用
/// `Get-Content -Raw` 读回；路径用 PowerShell 单引号包裹（`%TEMP%` 含空格也不会被拆词）。
fn windows_file_read_command(file: &str) -> String {
    powershell_encoded_command(&format!("Get-Content -Raw -LiteralPath '{file}'"))
}

/// 把 PowerShell 脚本编码成 `-EncodedCommand` 所需的 UTF-16LE base64 命令串。
///
/// 为什么不用 `-Command "<script>"`：产品以 `.arg()` 传命令串，Rust std 会把参数内每个 `"` 转义成
/// `\"`（见上），所以命令串必须完全不含双引号；`-Command` 的无引号形式虽可避开双引号，但仍需过
/// cmd 一遍（脚本里的 `|` `;` `(` 都得小心），且依赖 powershell.exe「拼接后续 argv」的隐式规则。
/// `-EncodedCommand` 的载荷只有 `[A-Za-z0-9+/=]`——不含双引号、不含任何 cmd 元字符，
/// 解析路径最短最确定。脚本内容仍在本文件内可读（t0 会解码回来逐字校验）。
fn powershell_encoded_command(script: &str) -> String {
    use base64::Engine as _;
    let utf16: Vec<u8> = script
        .encode_utf16()
        .flat_map(|unit| unit.to_le_bytes())
        .collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    format!("powershell -NoProfile -NonInteractive -EncodedCommand {encoded}")
}

/// 「输出一段固定文本」的 hook 命令（平台分支；Windows 侧顺带落数据文件）。
fn output_text_command(dir: &std::path::Path, tag: &str, text: &str) -> String {
    if cfg!(windows) {
        let file = dir.join(format!("{tag}.json"));
        fs::write(&file, text).expect("写入 W30 决策数据文件失败");
        windows_file_read_command(
            file.to_str().expect("W30 数据文件路径必须是合法 UTF-8"),
        )
    } else {
        unix_output_command(text)
    }
}

/// 输出一条 `{"decision":...}` 决策 JSON 的命令。
fn decision_command(dir: &std::path::Path, tag: &str, json_literal: &str) -> String {
    output_text_command(dir, tag, json_literal)
}

/// 协议违规：输出非法 JSON（两平台都不含 `"`，可直接用 echo）。
fn invalid_json_command() -> String {
    if cfg!(windows) {
        "echo this is not json".to_string()
    } else {
        unix_output_command("this is not json")
    }
}

/// 协议违规：非零退出码（Unix `exit 3`，Windows `exit /b 3`；两者都不含 `"`）。
fn nonzero_exit_command_for(windows: bool) -> String {
    if windows {
        "exit /b 3".to_string()
    } else {
        "cat >/dev/null; exit 3".to_string()
    }
}

fn nonzero_exit_command() -> String {
    nonzero_exit_command_for(cfg!(windows))
}

/// 超时用例的长命令（Unix `exec sleep <marker>`，Windows `ping ... >nul`；均不含 `"`）。
fn timeout_command_for(windows: bool, marker: &str) -> String {
    if windows {
        "ping -n 6 127.0.0.1 >nul".to_string()
    } else {
        format!("exec sleep {marker}")
    }
}

/// t7 的「stdin 落盘 + 回决策」命令（`decision_file` 仅在 Windows 分支使用）。
///
/// Windows：cmd 无 `cat`，用 PowerShell `[Console]::In.ReadToEnd()` 读完整 stdin；
/// `Set-Content -Encoding ascii -NoNewline` 以**无 BOM** 的 ASCII 原样落盘
/// （PS 默认 UTF-16 会带 BOM，污染 JSON 解析）；决策 JSON 同样从数据文件读回。
/// 整条脚本经 `-EncodedCommand` 编码，命令串只剩 base64 载荷（不含 `"` 与任何 cmd 元字符）。
fn stdin_capture_command_for(windows: bool, captured: &str, decision_file: &str) -> String {
    if windows {
        powershell_encoded_command(&format!(
            "$i=[Console]::In.ReadToEnd(); Set-Content -LiteralPath '{captured}' -Value $i -Encoding ascii -NoNewline; Get-Content -Raw -LiteralPath '{decision_file}'"
        ))
    } else {
        // Unix 侧不需要数据文件：决策 JSON 直接内联进 printf 的单引号里
        // （t7 的决策固定为 captured，保留字面量便于阅读）。
        let decision = r#"{"decision":"allow","summary":"captured"}"#;
        format!("cat > '{captured}'; printf '{decision}'")
    }
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
// t0：平台命令文本镜像 + cmd 安全不变量
//
// 目的：Linux 上无法执行 cmd/PowerShell，故用参数化构造函数锁定 Windows 分支的**确切文本**，
// 并把「echo/printf 的实参不含元字符」变成可执行不变量（防后人改 JSON 内容时引入 `>`/`&`/`'`）。
// cmd /C 实际收到的整串（逐条对应本文件各用例）：
//   echo {"decision":"allow","summary":"ok"}
//   echo {"decision":"deny","reason":"forbidden by w30"}
//   echo this is not json
//   echo {"summary":"missing decision"}
//   exit /b 3
//   ping -n 6 127.0.0.1 >nul
//   powershell -NoProfile -NonInteractive -Command "[Console]::In.ReadToEnd() | Set-Content -Path 'C:\Temp\stdin.json' -Encoding ascii -NoNewline" & echo {"decision":"allow","summary":"captured"}
// ─────────────────────────────────────────────────────────────────────────────
/// 把 `powershell ... -EncodedCommand <b64>` 的载荷解码回脚本文本。
///
/// 用途：Linux 上无法执行 cmd/PowerShell，但可以验证「Windows 分支最终会执行的脚本内容」——
/// base64/UTF-16LE 是**本文件内**可计算的事实，不依赖任何 Windows 行为。
fn decode_powershell_script(command: &str) -> String {
    use base64::Engine as _;
    let encoded = command
        .rsplit(' ')
        .next()
        .expect("W30 断言失败：命令串缺少 -EncodedCommand 载荷");
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .expect("W30 断言失败：-EncodedCommand 载荷必须是合法 base64");
    assert_eq!(bytes.len() % 2, 0, "UTF-16LE 载荷长度必须是偶数");
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    String::from_utf16(&units).expect("W30 断言失败：脚本必须是合法 UTF-16")
}

#[test]
fn t0_platform_command_strings_are_mirrorable_and_cmd_safe() {
    // (1) Unix 分支（本机实际执行）：命令串原样交给 `sh -c`。
    let allow = r#"{"decision":"allow","summary":"ok"}"#;
    assert_eq!(
        unix_output_command(allow),
        format!("cat >/dev/null; printf '{allow}'")
    );
    assert_eq!(nonzero_exit_command_for(false), "cat >/dev/null; exit 3");
    assert_eq!(
        timeout_command_for(false, "5.123456"),
        "exec sleep 5.123456"
    );
    assert!(stdin_capture_command_for(false, "/tmp/stdin.json", "/tmp/decision.json")
        .starts_with("cat > '/tmp/stdin.json'; printf '"));

    // (2) Windows 分支：命令串可镜像解码（Linux 上无法执行 cmd/PowerShell，故把 -EncodedCommand
    //     的载荷解码回脚本，逐字校验将要执行的内容）。
    assert_eq!(
        decode_powershell_script(&windows_file_read_command(r"C:\Temp\w30-decision.json")),
        r"Get-Content -Raw -LiteralPath 'C:\Temp\w30-decision.json'",
        "Windows 决策输出必须在 PowerShell 里按数据文件读回（且路径用单引号避免拆词）"
    );
    assert_eq!(
        decode_powershell_script(&stdin_capture_command_for(
            true,
            r"C:\Temp\stdin.json",
            r"C:\Temp\decision.json"
        )),
        r"$i=[Console]::In.ReadToEnd(); Set-Content -LiteralPath 'C:\Temp\stdin.json' -Value $i -Encoding ascii -NoNewline; Get-Content -Raw -LiteralPath 'C:\Temp\decision.json'",
        "t7 的 Windows 分支：读 stdin → 无 BOM（ascii）落盘 → 从数据文件读回决策"
    );
    assert!(windows_file_read_command(r"C:\Temp\x.json")
        .starts_with("powershell -NoProfile -NonInteractive -EncodedCommand "));
    assert_eq!(nonzero_exit_command_for(true), "exit /b 3");
    assert_eq!(
        timeout_command_for(true, "5.123456"),
        "ping -n 6 127.0.0.1 >nul",
        "Windows 超时命令与 marker 无关（marker 只服务 Linux 的 /proc 判定）"
    );

    // (3) **回归护栏**：Windows 命令串一律不得含双引号。
    // 原因见 `windows_file_read_command` 注释：产品用 `.arg()` 传命令串，Rust std 会把参数内每个
    // `"` 转义成 `\"`，而 cmd `/C` 只剥外层引号，反斜杠会留在 echo/stdout 里 → 断言全崩。
    for command in [
        windows_file_read_command(r"C:\Temp\colon.json"),
        nonzero_exit_command_for(true),
        "echo this is not json".to_string(),
        timeout_command_for(true, "5.000001"),
        stdin_capture_command_for(true, r"C:\Temp\a.json", r"C:\Temp\b.json"),
    ] {
        assert!(
            !command.contains('"'),
            "Windows 命令串不得含双引号：Rust std 会把它转义成反斜杠+引号，cmd 剥壳后反斜杠残留在 stdout。实际命令：{command}"
        );
    }

    // (4) Unix `printf '...'` 的实参安全不变量：不得含单引号或换行（否则引号被截断）。
    for literal in [
        allow,
        r#"{"decision":"deny","reason":"forbidden by w30"}"#,
        r#"{"summary":"missing decision"}"#,
        r#"{"decision":"allow","summary":"captured"}"#,
        "this is not json",
    ] {
        assert!(
            !literal
                .chars()
                .any(|c| matches!(c, '\'' | '\n' | '\r')),
            "sh printf 单引号内的字面量 {literal:?} 不得含单引号/换行，否则命令串被截断"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// t1：from_json_str 解析与空 command 拒绝
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn t1_json_config_parses_two_specs_and_rejects_empty_command() {
    // Arrange：两个 hook 名，各自指向「输出 allow 的内联命令」（两平台同一语义）
    let workspace = TempDir::new("t1-json");
    let allow_command = decision_command(
        &workspace.root,
        "t1-allow",
        r#"{"decision":"allow","summary":"ok"}"#,
    );
    let config = json!({
        "hooks": {
            // 15000ms：Windows 分支要冷启动 PowerShell；该值只作超时预算，不参与断言
            // （仍显式给出，以覆盖「配置里的 timeoutMs 被解析」这条契约）。
            "w30-alpha": { "command": allow_command.as_str(), "timeoutMs": 15000 },
            "w30-beta": { "command": allow_command.as_str() }
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
    let command = decision_command(
        &workspace.root,
        "t2-allow",
        r#"{"decision":"allow","summary":"ok"}"#,
    );
    let executor = single_spec("w30-allow", command, 5_000);
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
    let command = decision_command(
        &workspace.root,
        "t3-deny",
        r#"{"decision":"deny","reason":"forbidden by w30"}"#,
    );
    let executor = single_spec("w30-deny", command, 5_000);
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
    // Arrange：三类坏输出（内联命令，两平台同一语义；Windows 侧决策 JSON 落数据文件）
    let workspace = TempDir::new("t4-invalid");
    let cases: Vec<(&str, String)> = vec![
        ("非法 JSON", invalid_json_command()),
        ("非零退出码", nonzero_exit_command()),
        (
            "缺 decision",
            decision_command(
                &workspace.root,
                "t4-no-decision",
                r#"{"summary":"missing decision"}"#,
            ),
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
    // Arrange：长命令（内联，平台分支）。
    // Unix：`exec sleep <唯一小数标记>` —— `exec` 让 sh 自身被替换为目标进程，于是
    //   「kill 直接子进程」等价于「kill 该命令」；标记供 /proc cmdline 确定性判定是否已消失。
    // Windows：`ping -n 6 127.0.0.1 >nul`（约 5s，对齐 `hook_commands.rs` 单元测试的既有写法）。
    //   注：Windows 上产品只能 kill 直接子进程（cmd），孙进程可能残留，故超时路径依赖产品的
    //   「有界等待 drain」保证 2s 内返回（`hook_commands.rs::join_reader_until`）。
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0);
    let sleep_marker = format!("5.{:06}", nanos % 1_000_000);
    let long_command = timeout_command_for(cfg!(windows), &sleep_marker);
    let executor = single_spec("w30-timeout", long_command, 200);
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
    let command = decision_command(
        &workspace.root,
        "t6-allow",
        r#"{"decision":"allow","summary":"ok"}"#,
    );
    let executor = single_spec("w30-configured", command, 5_000);
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
    // Arrange：命令把 stdin 原样落盘，再回 allow（内联，平台分支）
    let workspace = TempDir::new("t7-payload");
    let captured = workspace.path("stdin.json");
    let captured_arg = captured
        .to_str()
        .expect("t7 失败：临时产物路径必须是合法 UTF-8")
        .to_string();
    let decision_file = workspace.path("decision.json");
    fs::write(&decision_file, r#"{"decision":"allow","summary":"captured"}"#)
        .expect("t7 失败：写入决策数据文件失败");
    let decision_file_arg = decision_file
        .to_str()
        .expect("t7 失败：决策数据文件路径必须是合法 UTF-8")
        .to_string();
    // 平台分支见 `stdin_capture_command_for` 的注释。
    let command = stdin_capture_command_for(cfg!(windows), &captured_arg, &decision_file_arg);
    // 15s 而非 5s：Windows 分支要冷启动 PowerShell（约 0.3~1.5s，CI 负载下更慢），
    // 该值只是超时预算、不参与任何断言；Unix 分支为毫秒级，收紧/放宽都不改变鉴别力。
    let executor = single_spec("w30-payload", command, 15_000);
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
    let command = decision_command(
        &workspace.root,
        "t8-allow",
        r#"{"decision":"allow","summary":"ok"}"#,
    );
    let config_path = workspace.path("hooks.json");
    fs::write(
        &config_path,
        // 15000ms：同上，Windows 侧 PowerShell 冷启动预算。
        json!({ "hooks": { "w30-file": { "command": command.as_str(), "timeoutMs": 15000 } } })
            .to_string(),
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
