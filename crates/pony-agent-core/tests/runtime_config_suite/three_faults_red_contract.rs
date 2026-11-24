//! 三故障黑盒验收契约（Test Agent 冻结，只读业务代码、只写本文件）。
//!
//! 故障 1（沙箱未注册）：`build_governed_executor` 产物 `governed_dispatcher()` 的
//! Execute-scope（`Run` → `workspace_run_command`）预检不得再报
//! "no sandbox backend is registered"。允许 `sandbox_unavailable` / `sandbox_denied` /
//! 正常通过，唯独不允许 no-backend 文案（生产链路未装配后端的指纹）。
//!
//! 故障 2/3（Watchdog/Trace）：后端 `turn:suspended` 终态已由
//! `crates/pony-agent-core/src/agent/runtime/tests.rs`
//! `governed_ask_stream_suspends_turn_and_binds_waiting_user_without_provider_followup`
//! 覆盖（断言 stream 发射终态 `turn:suspended`），本文件不再重复；前端侧由
//! `tests/turn-suspended-contract.spec.ts` 锁定 `initializeTurnEvents` 监听集合。
//!
//! 红相预期：本文件用例在修复前 FAIL（output 仍含 no-backend 文案），修复后 PASS。

use pony_agent_core::agent::governed_executor::build_governed_executor;
use pony_agent_core::agent::tools::{ToolCall, ToolExecutor};
use serde_json::json;

fn unique_workspace(tag: &str) -> std::path::PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("three-faults-{tag}-{}-{nanos}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp workspace");
    dir.canonicalize().unwrap_or(dir)
}

#[test]
fn contract_execute_scope_preflight_must_not_report_missing_backend() {
    let workspace = unique_workspace("sandbox");
    let governed = build_governed_executor(Some(workspace), None);
    let result = governed.execute(&ToolCall {
        call_id: None,
        name: "Run".to_string(),
        arguments: json!({ "command": "echo hi", "description": "contract" }),
        plan: None,
    });
    assert!(
        !result.output.contains("no sandbox backend is registered"),
        "Execute-scope preflight still reports missing backend (fail-closed misfire): {}",
        result.output
    );
}
