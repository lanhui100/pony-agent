//! W56-B 验收：`BatchExecute` 的**整体预校验**（鲁棒性改进，不是安全修复）。
//!
//! Lead 裁决（2026-11-01）：写/执行类子调用受**描述符级白名单**约束
//! （`dispatcher.rs` 的 `allowed_child_ids` → `child_not_allowed`），结构上不可能作为 batch
//! 子项派发，因此 read_only **不能**经 batch 绕过 —— 上一轮"逐子调用过闸 ⇒ 安全缺口"的记录
//! 已更正。本卡的真实缺口是**部分执行语义**：批量的"允许性"只在派发每个子调用时才判定，
//! 于是出现「前几个子调用已执行、到第 N 个才失败」。
//!
//! 本文件冻结的契约（机器可判定）：
//!   1. **mixed batch**（白名单内只读子调用 + 白名单外写子调用）→ 整体拒绝：`results` 为空
//!      （一个子调用都不派发）、零副作用、顶层错误指向具体原因；
//!   2. 正向钉子：`read_only` + **纯只读** batch → 正常执行（整体判否不得变成无谓的能力损失）；
//!   3. 正向钉子：`full_access` + 纯只读 batch → 正常完成。
//!
//! 说明：写类子调用作为 batch 子项"按设计就是不允许"（`src-tauri/tests/tool_router_regression.rs`
//! 的 `batch_rejects_write_child_with_unsupported_composite_child` 已冻结该语义），故本文件
//! 不对"full_access 下写子调用成功"提任何要求。

use pony_agent_core::agent::dispatcher::DispatchContext;
use pony_agent_core::agent::governed_executor::build_governed_executor;
use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolExecutor, ToolResult};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

const SESSION: &str = "w56b-batch-session";
const WRITE_TARGET: &str = "batch-should-not-exist.txt";

fn temp_workspace() -> PathBuf {
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "pony-w56b-batch-{}-{seq}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create w56b workspace");
    root
}

fn context_with_mode(mode: &str) -> ToolExecutionContext {
    ToolExecutionContext {
        session_id: Some(SESSION.to_string()),
        dispatch_context: Some(DispatchContext {
            session_id: Some(SESSION.to_string()),
            tool_authorization_mode: Some(mode.to_string()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn batch_call(calls: Value) -> ToolCall {
    ToolCall {
        call_id: None,
        name: "BatchExecute".to_string(),
        arguments: json!({
            "description": "W56-B 批量整体预校验探针",
            "calls": calls
        }),
        plan: None,
    }
}

/// 白名单内的只读子调用（`workspace_list_files`），确保红相来自"整体预校验缺失"，
/// 而不是来自子调用结构性非法。
fn read_child(path: &str) -> Value {
    json!({
        "name": "workspace_list_files",
        "arguments": {
            "path": path,
            "description": "W56-B batch 的只读子调用"
        }
    })
}

/// 白名单外的写子调用（按设计不可作为 batch 子项）。
fn write_child(path: &str) -> Value {
    json!({
        "name": "Write",
        "arguments": {
            "path": path,
            "content": "batch side effect",
            "description": "W56-B batch 的写子调用"
        }
    })
}

fn payload(result: &ToolResult) -> Value {
    serde_json::from_str(&result.output).unwrap_or_else(|_| Value::String(result.output.clone()))
}

/// "一个子调用都没派发"的判据：复合输出里不得出现任何 results 条目。
fn child_results(payload: &Value) -> Vec<Value> {
    payload
        .get("results")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn assert_no_write_side_effect(workspace: &Path) {
    assert!(
        !workspace.join(WRITE_TARGET).exists(),
        "W56-B：被整体拒绝的 batch 不得产生任何写副作用（{} 不应存在）",
        workspace.join(WRITE_TARGET).display()
    );
}

/// (1) mixed batch：整体预校验 ⇒ 零派发、零副作用。
/// 红相（当前实现）：先派发只读子调用并真的执行，再让写子调用以 `child_not_allowed` 失败
/// ⇒ `results` 非空的部分执行语义。
#[test]
fn mixed_batch_is_rejected_as_a_whole_before_any_child_is_dispatched() {
    let workspace = temp_workspace();
    fs::write(workspace.join("seed.txt"), "seed\n").expect("seed file");
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let context = context_with_mode("read_only");

    let result = executor.execute_with_context(
        &batch_call(json!([read_child("."), write_child(WRITE_TARGET)])),
        &context,
    );
    let body = payload(&result);

    assert_ne!(
        result.status, "ok",
        "W56-B：含白名单外子调用的 batch 必须整体被拒，实际 status={} output={}",
        result.status, result.output
    );
    assert!(
        child_results(&body).is_empty(),
        "W56-B：整体预校验必须在派发任何子调用**之前**发生（results 必须为空/缺席，杜绝部分执行），实际 {body}"
    );
    assert_no_write_side_effect(&workspace);

    let _ = fs::remove_dir_all(workspace);
}

/// (2) 正向钉子：read_only + 纯只读 batch 必须正常执行（不得因整体判否损失能力）。
#[test]
fn read_only_pure_read_batch_still_executes() {
    let workspace = temp_workspace();
    fs::write(workspace.join("seed.txt"), "seed\n").expect("seed file");
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let context = context_with_mode("read_only");

    let result = executor.execute_with_context(&batch_call(json!([read_child(".")])), &context);
    let body = payload(&result);

    assert_eq!(
        result.status, "ok",
        "W56-B：read_only 下的**纯只读** batch 必须正常执行（整体判否不得变成无谓的能力损失），实际 output={}",
        result.output
    );
    assert_eq!(
        child_results(&body).len(),
        1,
        "W56-B：纯只读 batch 的子调用必须真的被执行，实际 {body}"
    );

    let _ = fs::remove_dir_all(workspace);
}

/// (3) 正向钉子：full_access + 纯只读 batch 正常完成（反向控制，防一刀切拒绝）。
#[test]
fn full_access_pure_read_batch_still_completes() {
    let workspace = temp_workspace();
    fs::write(workspace.join("seed.txt"), "seed\n").expect("seed file");
    let executor = build_governed_executor(Some(workspace.clone()), None);
    let context = context_with_mode("full_access");

    let result = executor.execute_with_context(
        &batch_call(json!([read_child("."), read_child(".")])),
        &context,
    );
    let body = payload(&result);

    assert_eq!(
        result.status, "ok",
        "W56-B：full_access 下纯只读 batch 必须正常完成（反向钉子），实际 output={}",
        result.output
    );
    assert_eq!(
        child_results(&body).len(),
        2,
        "W56-B：full_access 下每个只读子调用都必须被执行，实际 {body}"
    );

    let _ = fs::remove_dir_all(workspace);
}
