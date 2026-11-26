//! wave-62 **C5（端到端，追加判据）** 黑盒验收 —— 红相先写。
//!
//! 契约真值源：`.dev-team/contracts/wave-62-tool-cancel.md` §5 补遗 v2（Lead 一票裁决）。
//!
//! 背景（双路 L3 独立复现的 BLOCKING）：`GovernedToolExecutor → GovernedDispatcher →
//! RouterPrimitiveHandler` 在桥接处用 `ToolExecutionContext::default()` 重建上下文，
//! `PrimitiveToolHandlerRequest` 又不带取消字段 ⇒ `turn_stream.rs` 注入的 turn 令牌在桥接层
//! **丢失**，真实 app 里 stop 之后 `Run` 仍跑完。直连 `ToolRouter` 的 C3 用例覆盖不到该桥接，
//! 故本文件改用**生产默认执行器** `build_governed_executor_with_sandbox_backend(...)`。
//!
//! 判据（C5）：经生产默认执行器执行 `Run`（`sleep 1.5; echo leaked > marker`），
//! `DispatchContext.turn_cancellation = Some(token)`；执行 200ms 后 `token.cancel()` ⇒
//! **≤1000ms** 返回、`status == "aborted"`、错误码 `tool_cancelled`、
//! **marker 文件不存在**（副作用未发生）。
//!
//! 纪律：真实断言、AAA、禁止 `#[ignore]`/吞异常/空断言；用例自带 `recv_timeout` 上界，**永不挂死**。

use pony_agent_core::agent::budget::CancellationToken;
use pony_agent_core::agent::dispatcher::DispatchContext;
use pony_agent_core::agent::governed_executor::build_governed_executor_with_sandbox_backend;
use pony_agent_core::agent::sandbox::{SandboxBackend, TestSandboxBackend};
use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolExecutor};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

/// 契约 C5 / NFR：cancel → 返回 aborted 的硬上界。
const TURN_CANCEL_EXIT_BOUND_MS: u64 = 1_000;
/// 命令自身的副作用落点时刻（1.5s）之后才判定 marker，避免"提前观测"造成的假绿。
const SIDE_EFFECT_SETTLE_MS: u64 = 1_600;

fn temp_workspace(tag: &str) -> PathBuf {
    // wave-61-team Windows 可移植性修复：ThreadId 的 `( )` 是 cmd 无法安全引用的字符，
    // 拼进临时目录名会让 Run 工具 fail-closed（spawn_failed）。规约为纯数字后缀。
    let thread_suffix: String = format!("{:?}", std::thread::current().id())
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    let path = std::env::temp_dir().join(format!(
        "pony-wave62-e2e-{tag}-{}-{thread_suffix}",
        std::process::id()
    ));
    std::fs::create_dir_all(&path).expect("create e2e temp workspace");
    path
}

/// 生产默认执行器（与 app 运行期同一条构造路径）。
fn production_executor(workspace: &PathBuf) -> impl ToolExecutor {
    build_governed_executor_with_sandbox_backend(
        Some(workspace.clone()),
        None,
        Some(Arc::new(TestSandboxBackend::available()) as Arc<dyn SandboxBackend>),
    )
}

fn run_call(command: String) -> ToolCall {
    ToolCall {
        call_id: Some("call-c5".to_string()),
        name: "Run".to_string(),
        arguments: json!({
            "command": command,
            "description": "wave62 C5 e2e probe",
            "timeoutMs": 10_000
        }),
        plan: None,
    }
}

fn cancelled_side_effect_command(marker: &PathBuf) -> String {
    if cfg!(windows) {
        format!(
            "ping -n 4 127.0.0.1 > nul & echo leaked > \"{}\"",
            marker.display()
        )
    } else {
        // 1.5s 后写副作用：若取消未真正杀进程，marker 必被写出（可观测证据）。
        format!("sleep 1.5; echo leaked > '{}'", marker.display())
    }
}

/// C5：生产默认执行器上，turn 令牌 cancel ⇒ `Run` 被真正终止（≤1000ms / aborted /
/// `tool_cancelled`），且被取消的命令不得产生副作用。
#[test]
fn c5_production_executor_run_is_killed_by_turn_cancellation() {
    // Arrange
    let workspace = temp_workspace("kill");
    let marker = workspace.join("c5-leaked-side-effect.txt");
    let executor = production_executor(&workspace);
    let token = Arc::new(CancellationToken::new());
    let context = ToolExecutionContext {
        workspace_root: Some(workspace.clone()),
        session_id: Some("session-c5".to_string()),
        dispatch_context: Some(DispatchContext {
            session_id: Some("session-c5".to_string()),
            run_id: Some("run-c5".to_string()),
            turn_id: Some("turn-c5".to_string()),
            workspace_root: Some(workspace.display().to_string()),
            turn_cancellation: Some(Arc::clone(&token)),
            ..Default::default()
        }),
        ..Default::default()
    };
    let call = run_call(cancelled_side_effect_command(&marker));
    let (result_tx, result_rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = result_tx.send(executor.execute_with_context(&call, &context));
    });

    // Act：命令真正开始执行后（200ms 量级）取消 turn 令牌。
    std::thread::sleep(Duration::from_millis(200));
    let cancelled_at = Instant::now();
    token.cancel();
    let tool_result = result_rx
        .recv_timeout(Duration::from_millis(TURN_CANCEL_EXIT_BOUND_MS + 2_000))
        .expect("C5：cancel 后生产默认执行器必须返回（3s 观测上界；超时=令牌在桥接层丢失且无轮询）");
    let exit_ms = cancelled_at.elapsed().as_millis() as u64;

    // Assert 1（语义优先）：必须返回 aborted，而不是命令自然跑完
    assert_eq!(
        tool_result.status, "aborted",
        "C5：经生产默认执行器（GovernedToolExecutor→GovernedDispatcher→RouterPrimitiveHandler）\
         执行的 Run 在 turn 令牌 cancel 后必须返回 status=\"aborted\"；\
         实际 status={:?}、cancel→返回 {}ms、output={}",
        tool_result.status, exit_ms, tool_result.output
    );

    // Assert 2：错误码沿用既有 `tool_cancelled`（不得新增码）
    let payload =
        serde_json::from_str::<Value>(&tool_result.output).expect("C5：Run 输出必须为 JSON");
    let code = payload
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("");
    assert_eq!(
        code, "tool_cancelled",
        "C5：错误码必须沿用既有 `tool_cancelled`（不得新增码），实际 = {code:?}\
         ；诊断：status={:?}、cancel→返回 {}ms、error={:?}",
        tool_result.status,
        exit_ms,
        payload.get("error")
    );

    // Assert 3：时延上界（cancel → 返回 ≤1000ms）
    assert!(
        exit_ms <= TURN_CANCEL_EXIT_BOUND_MS,
        "C5：cancel → 返回必须 ≤{TURN_CANCEL_EXIT_BOUND_MS}ms，实际 {exit_ms}ms"
    );

    // Assert 4：副作用未发生 —— 等过命令原本要写 marker 的时刻（1.5s），marker 必须不存在
    std::thread::sleep(Duration::from_millis(SIDE_EFFECT_SETTLE_MS));
    assert!(
        !marker.exists(),
        "C5：取消必须真正终止子进程（否则 {:?} 会在 1.5s 后写出副作用）",
        marker
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

/// C5 回归：生产默认执行器上未取消的 `Run` 行为不变（正常完成 + 输出回传）。
#[test]
fn c5_production_executor_run_completes_normally_without_cancellation() {
    // Arrange
    let workspace = temp_workspace("normal");
    let executor = production_executor(&workspace);
    let token = Arc::new(CancellationToken::new());
    let context = ToolExecutionContext {
        workspace_root: Some(workspace.clone()),
        session_id: Some("session-c5-normal".to_string()),
        dispatch_context: Some(DispatchContext {
            session_id: Some("session-c5-normal".to_string()),
            run_id: Some("run-c5-normal".to_string()),
            turn_id: Some("turn-c5-normal".to_string()),
            workspace_root: Some(workspace.display().to_string()),
            turn_cancellation: Some(Arc::clone(&token)),
            ..Default::default()
        }),
        ..Default::default()
    };

    // Act：不取消
    let result = executor.execute_with_context(&run_call("echo hi".to_string()), &context);

    // Assert
    assert_eq!(
        result.status, "ok",
        "C5 回归：未取消的 Run 必须正常完成，实际 status={:?} output={}",
        result.status, result.output
    );
    assert!(
        result.output.contains("hi"),
        "C5 回归：命令输出必须回传，实际 = {}",
        result.output
    );
    assert!(
        !token.is_cancelled(),
        "C5 回归：正常完成路径不得自我置位取消"
    );

    let _ = std::fs::remove_dir_all(&workspace);
}
