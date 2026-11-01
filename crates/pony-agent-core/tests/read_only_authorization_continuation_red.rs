//! T7 验收：`read_only` 会话的写入/执行类工具必须被 Deny —— 含「目标自动续轮」的第 2 轮。
//!
//! 缺陷形状（已复核）：goal 自动续轮时宿主自造的 `TurnInput` 曾把
//! `tool_authorization_mode` 写成 `None`（`control_plane/mod.rs`），而授权模式闸门是
//! `if let Some(auth_mode) = …`（`governed_executor.rs`）——`None` 直接跳过检查 ⇒
//! 只读会话在续轮里拿到 **Allow**（fail-open）。
//!
//! 本文件钉住可独立判定的两半契约：
//!   (a) **执行闸门半**：`read_only` 上下文下 Write / Edit / Bash 类调用必须被拒且拒绝原因
//!       指向只读模式（本文件断言，走生产装配 `build_governed_executor`）；
//!   (b) **续轮继承半**：宿主自动续轮必须继承原会话的授权模式，缺失时回落 `read_only`
//!       而**绝不**是 `None`。
//!
//! 覆盖边界（显式记录，避免"看着绿就以为全覆盖"）：(b) 的唯一可观察点在
//! `HostControlPlane::execute_graph_run_stream` 的 goal 续轮循环内部，其输入由宿主自造、
//! 无公开注入缝；集成层验证需要在测试里重建私有的 `MockHttpServer` + provider resolver
//! 才能驱动一轮真实 turn。故本文件当前只断言 (a)，(b) 需实现侧提供测试缝（或授权补 mock
//! provider 夹具）后再冻结 —— 见回报说明。

use pony_agent_core::agent::dispatcher::DispatchContext;
use pony_agent_core::agent::governed_executor::build_governed_executor;
use pony_agent_core::agent::tools::{
    ToolCall, ToolExecutionContext, ToolExecutor, ToolResult,
};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

const SESSION: &str = "t7-read-only-session";

fn temp_workspace() -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "pony-t7-read-only-{}-{seq}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create t7 workspace");
    root
}

fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        call_id: None,
        name: name.to_string(),
        arguments,
        plan: None,
    }
}

/// `read_only` 会话的 per-call 上下文（与运行时下发的形状一致）。
fn read_only_context() -> ToolExecutionContext {
    ToolExecutionContext {
        session_id: Some(SESSION.to_string()),
        dispatch_context: Some(DispatchContext {
            session_id: Some(SESSION.to_string()),
            tool_authorization_mode: Some("read_only".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn payload(result: &ToolResult) -> Value {
    serde_json::from_str(&result.output).unwrap_or_else(|_| Value::String(result.output.clone()))
}

#[test]
fn read_only_session_denies_write_edit_and_execute() {
    let workspace = temp_workspace();
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let context = read_only_context();

    // (1) Write：必须 Deny，且拒绝原因指向只读模式；文件不得被创建。
    let write = executor.execute_with_context(
        &call(
            "Write",
            json!({
                "path": "read-only-should-not-exist.txt",
                "content": "leak",
                "description": "T7 read_only 写入探针"
            }),
        ),
        &context,
    );
    assert_ne!(
        write.status, "ok",
        "T7：read_only 会话的 Write 必须被拒（fail-open 会让只读会话写盘），实际 output={}",
        write.output
    );
    let write_payload = payload(&write);
    assert!(
        write.output.contains("read_only") || write.output.contains("只读"),
        "T7：拒绝原因必须指向只读授权模式，实际 {write_payload}"
    );
    assert!(
        !workspace.join("read-only-should-not-exist.txt").exists(),
        "T7：被拒的 Write 不得落盘"
    );

    // (2) Edit：同样必须 Deny（先造一个真实文件，避免因"文件不存在"而假绿）。
    fs::write(workspace.join("editable.txt"), "alpha\nbeta\n").expect("seed editable file");
    let edit = executor.execute_with_context(
        &call(
            "Edit",
            json!({
                "path": "editable.txt",
                "oldText": "beta",
                "newText": "gamma",
                "description": "T7 read_only 编辑探针"
            }),
        ),
        &context,
    );
    assert_ne!(
        edit.status, "ok",
        "T7：read_only 会话的 Edit 必须被拒，实际 output={}",
        edit.output
    );
    assert!(
        edit.output.contains("read_only") || edit.output.contains("只读"),
        "T7：Edit 的拒绝原因必须指向只读授权模式（不得靠参数/schema 错误假绿），实际 {}",
        payload(&edit)
    );
    assert_eq!(
        fs::read_to_string(workspace.join("editable.txt")).expect("read seeded file"),
        "alpha\nbeta\n",
        "T7：被拒的 Edit 不得修改文件内容"
    );

    // (3) Execute（模型名 `Run` → builtin:workspace_run_command）：必须 Deny，且不得产生副作用。
    let execute = executor.execute_with_context(
        &call(
            "Run",
            json!({
                "command": "echo t7-should-not-run > read-only-executed.txt",
                "cwd": ".",
                "timeoutMs": 5000,
                "description": "T7 read_only 执行探针"
            }),
        ),
        &context,
    );
    assert_ne!(
        execute.status, "ok",
        "T7：read_only 会话的 Execute 必须被拒，实际 output={}",
        execute.output
    );
    assert!(
        execute.output.contains("read_only") || execute.output.contains("只读"),
        "T7：Execute 的拒绝原因必须指向只读授权模式，实际 {}",
        payload(&execute)
    );
    assert!(
        !workspace.join("read-only-executed.txt").exists(),
        "T7：被拒的 Execute 不得产生副作用"
    );

    let _ = fs::remove_dir_all(workspace);
}

/// 反向钉子：同一批工具在 `full_access` 下不得被授权闸门误杀（防"一刀切拒绝"）。
#[test]
fn full_access_session_still_allows_write() {
    let workspace = temp_workspace();
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let context = ToolExecutionContext {
        session_id: Some(SESSION.to_string()),
        dispatch_context: Some(DispatchContext {
            session_id: Some(SESSION.to_string()),
            tool_authorization_mode: Some("full_access".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };

    let write = executor.execute_with_context(
        &call(
            "Write",
            json!({
                "path": "full-access-ok.txt",
                "content": "ok",
                "description": "T7 full_access 写入探针"
            }),
        ),
        &context,
    );
    assert_eq!(
        write.status, "ok",
        "T7：full_access 会话的 Write 必须放行（授权闸门不得误杀），实际 output={}",
        write.output
    );
    assert!(workspace.join("full-access-ok.txt").exists());

    let _ = fs::remove_dir_all(workspace);
}
