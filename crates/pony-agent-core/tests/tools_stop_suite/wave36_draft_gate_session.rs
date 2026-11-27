//! W36（2026-11-27 重写）：**Plan 模式写门禁**必须使用运行时注入的会话 id。
//!
//! 旧契约针对 `Plan` 步骤机的 `Draft` 态门禁（`plan_draft_gate`）。步骤机已下线
//! （ADR `2026-11-27-plan-mode-dsh-alignment-and-task-count`），门禁改为「会话处于
//! Plan 模式」；**会话判据不变**——只认 runtime 经 `set_context` 注入的
//! `DispatchContext.session_id`，绝不回落工具参数/环境变量/常量 `"default"`。
//!
//! 断言面（每条都是当年真实缺陷的回归栅栏）：
//! 1. 注入会话处于 Plan 模式 ⇒ 写/执行工具被 `plan_mode_gate` 拒绝，且**零副作用**；
//! 2. 另一会话不在 Plan 模式 ⇒ 写工具照常放行（不会跨会话误锁）；
//! 3. 未注入会话（`session_id = None`）⇒ 按「未激活」处理，交由只读闸门兜底；
//! 4. Plan 模式下读类工具照常放行（只锁写/执行）；
//! 5. 退出 Plan 模式（批准或 `/plan off`）⇒ 写工具立刻解锁。
//!
//! 走**生产装配路径**（`build_governed_executor` + `set_context` + `execute`）。

use pony_agent_core::agent::dispatcher::DispatchContext;
use pony_agent_core::agent::dispatcher_composites::GovernedToolExecutor;
use pony_agent_core::agent::governed_executor::build_governed_executor;
use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolExecutor, ToolResult};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_workspace() -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "pony-wave36-plan-mode-{}-{seq}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create plan-mode test workspace");
    root
}

fn session(name: &str) -> String {
    format!("wave36-plan-mode-{name}-{}", uuid::Uuid::new_v4())
}

fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        call_id: Some(format!("call-{name}")),
        name: name.to_string(),
        arguments,
        plan: None,
    }
}

/// 以指定会话上下文执行一次调用（会话事实只来自 `ToolExecutionContext.dispatch_context`）。
fn execute_in_session(
    executor: &GovernedToolExecutor,
    session_id: Option<&str>,
    call: &ToolCall,
) -> ToolResult {
    let exec_context = ToolExecutionContext {
        dispatch_context: Some(DispatchContext {
            session_id: session_id.map(str::to_string),
            ..DispatchContext::default()
        }),
        ..ToolExecutionContext::default()
    };
    executor.execute_with_context(call, &exec_context)
}

fn write_call(path: &str) -> ToolCall {
    call(
        "workspace_write_file",
        json!({ "path": path, "content": "hello", "description": "写入测试文件" }),
    )
}

fn run_call() -> ToolCall {
    call(
        "workspace_run_command",
        json!({ "command": "echo plan-mode-gate", "description": "跑一条命令" }),
    )
}

fn error_code(result: &ToolResult) -> Option<String> {
    let parsed: Value = serde_json::from_str(&result.output).ok()?;
    parsed
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn error_message(result: &ToolResult) -> String {
    let parsed: Value = serde_json::from_str(&result.output).unwrap_or(Value::Null);
    parsed
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

#[test]
fn plan_mode_gate_blocks_write_and_execute_for_the_runtime_injected_session() {
    let workspace = temp_workspace();
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let active = session("active");
    pony_agent_core::agent::plan_mode::set_active(&active, true);

    let write = execute_in_session(&executor, Some(&active), &write_call("blocked.txt"));
    assert_eq!(write.status, "error", "Plan 模式下写工具必须被拒绝");
    assert_eq!(error_code(&write).as_deref(), Some("permission_denied"));
    assert!(
        error_message(&write).contains("plan_mode_gate"),
        "拒绝必须来自 Plan 模式门禁（来源可追溯）: {}",
        write.output
    );
    assert!(
        !workspace.join("blocked.txt").exists(),
        "fail-closed 必须在副作用之前生效：拒绝时不得落盘"
    );

    let run = execute_in_session(&executor, Some(&active), &run_call());
    assert_eq!(run.status, "error", "Plan 模式下执行工具必须被拒绝");
    assert_eq!(error_code(&run).as_deref(), Some("permission_denied"));

    pony_agent_core::agent::plan_mode::set_active(&active, false);
    let _ = fs::remove_dir_all(&workspace);
}

#[test]
fn plan_mode_gate_does_not_cross_block_other_sessions() {
    let workspace = temp_workspace();
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let active = session("other-active");
    let passive = session("passive");
    pony_agent_core::agent::plan_mode::set_active(&active, true);

    let result = execute_in_session(&executor, Some(&passive), &write_call("allowed.txt"));
    assert_eq!(
        result.status, "ok",
        "别的会话进入 Plan 模式不得锁住本会话的写工具: {}",
        result.output
    );
    assert!(workspace.join("allowed.txt").exists());

    pony_agent_core::agent::plan_mode::set_active(&active, false);
    let _ = fs::remove_dir_all(&workspace);
}

#[test]
fn plan_mode_gate_does_not_guess_a_session_when_context_is_missing() {
    let workspace = temp_workspace();
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let active = session("unbound");
    pony_agent_core::agent::plan_mode::set_active(&active, true);

    // 未注入会话 ⇒ 无从证明处于 Plan 模式：按「未激活」处理，不得凭工具参数/环境变量猜。
    let result = execute_in_session(&executor, None, &write_call("unbound.txt"));
    assert_eq!(
        result.status, "ok",
        "无注入会话时不得凭猜测锁定写工具（会话判据只能来自 DispatchContext）: {}",
        result.output
    );

    pony_agent_core::agent::plan_mode::set_active(&active, false);
    let _ = fs::remove_dir_all(&workspace);
}

#[test]
fn plan_mode_gate_allows_read_tools() {
    let workspace = temp_workspace();
    fs::write(workspace.join("readme.md"), "hello").expect("seed file");
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let active = session("read");
    pony_agent_core::agent::plan_mode::set_active(&active, true);
    let result = execute_in_session(
        &executor,
        Some(&active),
        &call(
            "Read",
            json!({ "path": "readme.md", "description": "读取文件" }),
        ),
    );
    assert_eq!(
        result.status, "ok",
        "Plan 模式只锁写/执行，读类工具必须放行: {}",
        result.output
    );

    pony_agent_core::agent::plan_mode::set_active(&active, false);
    let _ = fs::remove_dir_all(&workspace);
}

#[test]
fn leaving_plan_mode_unlocks_writes_immediately() {
    let workspace = temp_workspace();
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let active = session("leave");
    pony_agent_core::agent::plan_mode::set_active(&active, true);
    assert_eq!(
        execute_in_session(&executor, Some(&active), &write_call("first.txt")).status,
        "error"
    );

    // 用户批准方案 / `/plan off`：模式关闭后写工具立刻解锁。
    pony_agent_core::agent::plan_mode::set_active(&active, false);
    let after = execute_in_session(&executor, Some(&active), &write_call("second.txt"));
    assert_eq!(
        after.status, "ok",
        "退出 Plan 模式后写工具必须立刻解锁: {}",
        after.output
    );
    assert!(workspace.join("second.txt").exists());

    let _ = fs::remove_dir_all(&workspace);
}

/// Plan 模式的**退出审查工具**（primitive `plan_control` / 模型名 `ExitPlanMode`）
/// 自身必须豁免门禁——否则模式永远解不开（自锁死防护）。
#[test]
fn plan_mode_gate_keeps_exit_plan_mode_exempt() {
    let workspace = temp_workspace();
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let active = session("exempt");
    pony_agent_core::agent::plan_mode::set_active(&active, true);
    let result = execute_in_session(
        &executor,
        Some(&active),
        &call(
            "ExitPlanMode",
            json!({ "plan": "# 方案\n- 步骤", "description": "提交方案请用户批准" }),
        ),
    );
    assert_eq!(
        result.status, "ok",
        "ExitPlanMode 必须能在 Plan 模式内调用（唯一退出路径）: {}",
        result.output
    );
    let parsed: Value = serde_json::from_str(&result.output).expect("valid json");
    assert_eq!(
        parsed.get("awaitingReview").and_then(Value::as_bool),
        Some(true),
        "提交方案后必须请求宿主审核: {parsed}"
    );

    pony_agent_core::agent::plan_mode::set_active(&active, false);
    let _ = fs::remove_dir_all(&workspace);
}

#[test]
fn exit_plan_mode_fails_closed_outside_plan_mode() {
    let workspace = temp_workspace();
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let passive = session("outside");
    let result = execute_in_session(
        &executor,
        Some(&passive),
        &call(
            "ExitPlanMode",
            json!({ "plan": "# 方案", "description": "非 Plan 模式下调用" }),
        ),
    );
    assert_eq!(result.status, "error", "非 Plan 模式下必须 fail-closed");
    assert_eq!(
        error_code(&result).as_deref(),
        Some("plan_mode_inactive"),
        "错误码必须是 plan_mode_inactive: {}",
        result.output
    );

    let _ = fs::remove_dir_all(&workspace);
}

#[test]
fn exit_plan_mode_requires_a_markdown_heading() {
    let workspace = temp_workspace();
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let active = session("heading");
    pony_agent_core::agent::plan_mode::set_active(&active, true);
    let result = execute_in_session(
        &executor,
        Some(&active),
        &call(
            "ExitPlanMode",
            json!({ "plan": "随便写写", "description": "无标题方案" }),
        ),
    );
    assert_eq!(result.status, "error");
    assert_eq!(
        error_code(&result).as_deref(),
        Some("invalid_plan"),
        "无一级标题的方案必须被拒绝: {}",
        result.output
    );

    pony_agent_core::agent::plan_mode::set_active(&active, false);
    let _ = fs::remove_dir_all(&workspace);
}

#[test]
fn tool_execution_context_alias_still_compiles() {
    // 说明：本测试文件同时锁住「旧 `ToolExecutionContext` 通道仍可用」——
    // Plan 模式门禁只新增判定，不改既有调用面（compile-time guard）。
    let _ = ToolExecutionContext::default();
}
