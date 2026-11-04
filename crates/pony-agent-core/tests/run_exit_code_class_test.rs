//! P0-3 红相验收：`non_zero_exit` 载荷携带**退出码分类** `exitCodeClass`（trace 可诊断的第一刀）。
//!
//! 契约真源：trace 报告 P0-3（失败轮保留正文稍后单独做；本卡只做退出码分类）。
//!
//! 新增字段位置：**错误载荷内**（`payload.error.exitCodeClass`），与既有 `error.code =
//! "non_zero_exit"` 一起返回；既有字段（`code`/`kind`/`message`/`exitCode`）**不得改写**。
//!
//! 分类口径（本卡冻结的最小集合）：
//! - `101` ⇒ `compile_or_panic`（cargo/编译失败、Rust panic 的典型码）
//! - `127` ⇒ `command_not_found`
//! - `126` ⇒ `not_executable`
//! - `130` ⇒ `interrupted`（SIGINT，128+2）
//! - `137` ⇒ `killed`（SIGKILL，128+9）
//! - 其余（示例 `7`）⇒ `general_failure`（兜底）
//!
//! 平台：仅 Unix（本文件用 `sh -c` 造真实退出码；Windows 分支不写）。
//! 每条用例都 `println!` 真实载荷，便于人工复核。

#![cfg(unix)]

use std::path::PathBuf;

use pony_agent_core::agent::sandbox::TestSandboxBackend;
use pony_agent_core::agent::tools::{ToolCall, ToolRouter};
use serde_json::{json, Value};

/// 每个用例独立临时 workspace（谁创建谁清理）。
fn unique_workspace(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock must be after UNIX_EPOCH")
        .as_nanos();
    std::env::temp_dir().join(format!("run_exit_class_{}_{}", tag, nanos))
}

/// 经真实 `ToolRouter` 执行 `workspace_run_command`，返回 `(status, 载荷)`。
fn run_command(command: &str, tag: &str) -> (String, Value) {
    let workspace = unique_workspace(tag);
    std::fs::create_dir_all(&workspace).expect("create temp workspace");
    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_sandbox_backend(TestSandboxBackend::available());
    let call = ToolCall {
        name: "workspace_run_command".to_string(),
        arguments: json!({
            "command": command,
            "cwd": workspace.display().to_string(),
            "timeoutMs": 60_000,
            "description": "P0-3 退出码分类验收",
        }),
        call_id: Some(format!("call-{tag}")),
        plan: None,
    };
    let result = router.execute(&call);
    let _ = std::fs::remove_dir_all(&workspace);
    let payload: Value = serde_json::from_str(&result.output).unwrap_or_else(|error| {
        panic!(
            "Run 输出必须是合法 JSON（{}）: {}",
            error, result.output
        )
    });
    (result.status, payload)
}

/// 断言某条命令的退出码分类，并**打印真实载荷**供复核。
///
/// 同时锁死既有契约（P0-3 第 7 条）：`error.code` 仍为 `non_zero_exit`、`exitCode` 仍为原值。
fn assert_exit_code_class(
    command: &str,
    expected_exit_code: i64,
    expected_class: &str,
    rationale: &str,
    tag: &str,
) {
    // Act
    let (status, payload) = run_command(command, tag);
    println!(
        "[P0-3] case=`{}` expected=({}, {}) rationale={} ⇒ status={} payload={}",
        command, expected_exit_code, expected_class, rationale, status, payload
    );

    // Assert 1（兼容锁）：仍是波次一的 non_zero_exit 错误路径。
    assert_eq!(
        status, "error",
        "P0-3：非零退出必须返回 error 状态（case=`{command}`），实际 status=`{status}` payload={payload}"
    );
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("non_zero_exit"),
        "P0-3：既有错误码不得改写（case=`{command}`），payload={payload}"
    );
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("kind"))
            .and_then(Value::as_str),
        Some("non_zero_exit"),
        "P0-3：既有 error.kind 不得改写（case=`{command}`），payload={payload}"
    );

    // Assert 2（兼容锁）：退出码原值必须保留（根级 + error 内各一份）。
    assert_eq!(
        payload.get("exitCode").and_then(Value::as_i64),
        Some(expected_exit_code),
        "P0-3：根级 exitCode 必须保留原值 {}（case=`{command}`），payload={payload}",
        expected_exit_code
    );
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("exitCode"))
            .and_then(Value::as_i64),
        Some(expected_exit_code),
        "P0-3：error.exitCode 必须保留原值 {}（case=`{command}`），payload={payload}",
        expected_exit_code
    );

    // Assert 3（新增）：退出码分类字段。
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("exitCodeClass"))
            .and_then(Value::as_str),
        Some(expected_class),
        "P0-3：`error.exitCodeClass` 必须为 `{}`（{}；case=`{}`），payload={}",
        expected_class,
        rationale,
        command,
        payload
    );
}

#[test]
fn exit_101_is_compile_or_panic() {
    assert_exit_code_class(
        "sh -c 'exit 101'",
        101,
        "compile_or_panic",
        "cargo/编译失败与 Rust panic 的典型退出码",
        "101",
    );
}

#[test]
fn exit_127_is_command_not_found() {
    assert_exit_code_class(
        "sh -c 'exit 127'",
        127,
        "command_not_found",
        "shell「命令不存在」约定码",
        "127",
    );
}

#[test]
fn exit_126_is_not_executable() {
    assert_exit_code_class(
        "sh -c 'exit 126'",
        126,
        "not_executable",
        "shell「找到但不可执行」约定码",
        "126",
    );
}

#[test]
fn exit_130_is_interrupted() {
    assert_exit_code_class(
        "sh -c 'exit 130'",
        130,
        "interrupted",
        "128+SIGINT(2)：用户中断",
        "130",
    );
}

#[test]
fn exit_137_is_killed() {
    assert_exit_code_class(
        "sh -c 'exit 137'",
        137,
        "killed",
        "128+SIGKILL(9)：被强杀",
        "137",
    );
}

#[test]
fn other_exit_codes_fall_back_to_general_failure() {
    assert_exit_code_class(
        "sh -c 'exit 7'",
        7,
        "general_failure",
        "兜底分类",
        "7",
    );
}
