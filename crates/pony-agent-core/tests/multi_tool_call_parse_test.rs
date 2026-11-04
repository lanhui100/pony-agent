//! D 卡（P1-1 最小切片）红相：provider **多工具调用不再截断**。
//!
//! 契约真源：trace 报告 P1-1（一次响应含多个 tool_use / tool_calls 时，运行时只执行第一个，
//! 其余被静默丢弃）。Lead 裁决 **方案 A**：新增复数版 **pub 纯函数**，单数版保留给现有调用点：
//!
//! - `pub fn extract_openai_tool_calls(message: &Value) -> Vec<ToolCall>`
//! - `pub fn extract_anthropic_tool_calls(content: &[Value]) -> Vec<ToolCall>`
//!
//! 验收：
//! 1. OpenAI：2 个 `tool_calls` ⇒ 2 个调用，**顺序与数组一致**，name/arguments/id 正确；
//!    1 个 ⇒ 1 个；0 个 / 畸形 ⇒ 空 vec（不 panic）。
//! 2. Anthropic：2 个 `tool_use` ⇒ 2 个，顺序按出现顺序；夹杂 `text` 块不影响结果；畸形 ⇒ 空 vec。
//! 3. 回归锁：单数版 `extract_openai_tool_call` **当前仍是私有**（`provider/mod.rs`），
//!    集成测试不可达 ⇒ 按 Lead 授权跳过「与复数版首元素一致」这条，改由「复数版首元素自身
//!    字段正确」+「1 个调用时复数版恰好返回 1 个、字段与单数版语义一致」间接锁定。
//!    若 executor 顺带把单数版改 pub，此文件可加一条 `calls[0] == single` 的强断言。
//!
//! 每条用例都 `println!` 真实 `Vec<ToolCall>` 摘要，便于人工复核。

use pony_agent_core::agent::provider::{extract_anthropic_tool_calls, extract_openai_tool_calls};
use pony_agent_core::agent::tools::ToolCall;
use serde_json::{json, Value};

/// 打印调用摘要（name / call_id / arguments），便于回执时人工核对顺序与字段。
fn describe(calls: &[ToolCall]) -> String {
    let rendered: Vec<String> = calls
        .iter()
        .map(|call| {
            format!(
                "{{name={:?}, call_id={:?}, arguments={}}}",
                call.name, call.call_id, call.arguments
            )
        })
        .collect();
    format!("len={} [{}]", calls.len(), rendered.join(", "))
}

fn openai_tool_call(id: &str, name: &str, arguments: Value) -> Value {
    json!({
        "id": id,
        "type": "function",
        "function": {
            "name": name,
            "arguments": arguments.to_string()
        }
    })
}

// ── OpenAI：多调用不被截断 ────────────────────────────────────────────────

#[test]
fn openai_multiple_tool_calls_are_all_returned_in_order() {
    // Arrange
    let message = json!({
        "role": "assistant",
        "tool_calls": [
            openai_tool_call("call_1", "Read", json!({ "path": "a.txt" })),
            openai_tool_call("call_2", "List", json!({})),
        ]
    });

    // Act
    let calls = extract_openai_tool_calls(&message);
    println!("[D] openai 2 calls ⇒ {}", describe(&calls));

    // Assert：数量（不得只返回第一个）、顺序、字段。
    assert_eq!(
        calls.len(),
        2,
        "P1-1：2 个 tool_calls 必须返回 2 个 ToolCall（不得截断为 1），实际 {}",
        describe(&calls)
    );
    assert_eq!(calls[0].name, "Read", "P1-1：首个调用 name 必须为 Read");
    assert_eq!(
        calls[0].call_id.as_deref(),
        Some("call_1"),
        "P1-1：首个调用 call_id 必须为 call_1"
    );
    assert_eq!(
        calls[0].arguments,
        json!({ "path": "a.txt" }),
        "P1-1：首个调用 arguments 必须解析为对象"
    );
    assert_eq!(
        calls[1].name, "List",
        "P1-1：第二个调用 name 必须为 List（顺序与数组一致）"
    );
    assert_eq!(
        calls[1].call_id.as_deref(),
        Some("call_2"),
        "P1-1：第二个调用 call_id 必须为 call_2"
    );
}

#[test]
fn openai_single_tool_call_is_unchanged() {
    // Arrange
    let message = json!({
        "tool_calls": [openai_tool_call("call_only", "Read", json!({ "path": "b.txt" }))]
    });

    // Act
    let calls = extract_openai_tool_calls(&message);
    println!("[D] openai 1 call ⇒ {}", describe(&calls));

    // Assert（回归锁：单调用的字段语义与现状一致）
    assert_eq!(calls.len(), 1, "P1-1：单个调用必须恰好返回 1 个");
    assert_eq!(calls[0].name, "Read");
    assert_eq!(calls[0].call_id.as_deref(), Some("call_only"));
    assert_eq!(calls[0].arguments, json!({ "path": "b.txt" }));
    assert!(calls[0].plan.is_none(), "P1-1：解析结果不得携带 plan");
}

#[test]
fn openai_absent_or_malformed_tool_calls_yield_empty_vec() {
    // Act + Assert：四种无/畸形输入都不得 panic，且返回空 vec。
    for (label, message) in [
        ("no tool_calls key", json!({ "role": "assistant" })),
        ("empty array", json!({ "tool_calls": [] })),
        ("non-array", json!({ "tool_calls": "not-an-array" })),
        ("non-object message", json!(42)),
    ] {
        let calls = extract_openai_tool_calls(&message);
        println!("[D] openai {label} ⇒ {}", describe(&calls));
        assert!(
            calls.is_empty(),
            "P1-1：{label} 必须返回空 vec（不得 panic、不得伪造调用），实际 {}",
            describe(&calls)
        );
    }
}

// ── Anthropic：多 tool_use 不被截断 ───────────────────────────────────────

fn anthropic_blocks() -> Vec<Value> {
    vec![
        json!({ "type": "text", "text": "先看一下目录，再读文件。" }),
        json!({ "type": "tool_use", "id": "toolu_1", "name": "List", "input": {} }),
        json!({ "type": "tool_use", "id": "toolu_2", "name": "Read", "input": { "path": "b.txt" } }),
    ]
}

#[test]
fn anthropic_multiple_tool_use_blocks_are_all_returned_in_order() {
    // Arrange
    let content = anthropic_blocks();

    // Act
    let calls = extract_anthropic_tool_calls(&content);
    println!("[D] anthropic 2 tool_use（夹杂 text）⇒ {}", describe(&calls));

    // Assert
    assert_eq!(
        calls.len(),
        2,
        "P1-1：2 个 tool_use 必须返回 2 个 ToolCall（不得截断为 1、不得 Err），实际 {}",
        describe(&calls)
    );
    assert_eq!(calls[0].name, "List", "P1-1：首个调用须为 List（按出现顺序）");
    assert_eq!(calls[0].call_id.as_deref(), Some("toolu_1"));
    assert_eq!(calls[1].name, "Read", "P1-1：第二个调用须为 Read");
    assert_eq!(calls[1].call_id.as_deref(), Some("toolu_2"));
    assert_eq!(
        calls[1].arguments,
        json!({ "path": "b.txt" }),
        "P1-1：第二个调用 arguments 必须为 input 原样"
    );
}

#[test]
fn anthropic_empty_or_malformed_content_yields_empty_vec() {
    // Act + Assert
    let cases: [(&str, Vec<Value>); 3] = [
        ("empty content", Vec::new()),
        ("only text blocks", vec![json!({ "type": "text", "text": "hi" })]),
        ("malformed entries", vec![json!(42), json!("tool_use")]),
    ];
    for (label, content) in cases {
        let calls = extract_anthropic_tool_calls(&content);
        println!("[D] anthropic {label} ⇒ {}", describe(&calls));
        assert!(
            calls.is_empty(),
            "P1-1：{label} 必须返回空 vec（不得 panic），实际 {}",
            describe(&calls)
        );
    }
}
