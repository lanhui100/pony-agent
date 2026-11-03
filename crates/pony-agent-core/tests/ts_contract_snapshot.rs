//! W57-A：TS/Rust 契约漂移门禁的 **Rust 侧快照生成器**。
//!
//! 背景：wave-55 已实证一次真实漂移 —— Rust `TurnStreamEvent.step`
//! （`crates/pony-agent-core/src/agent/runtime/types.rs`）在 `src/types/runtime.ts` 里缺失，
//! 前端静默丢字段，只能靠人工评审发现。手写契约表不可持续。
//!
//! 做法：把 DTO **真实 serde 线缆键名**（`serde_json::to_value` 产出的对象键）落盘为快照
//! `docs/contracts/ts-contract.snapshot.json`，由 `scripts/check-ts-contract.mjs` 与 TS 声明比对。
//! 键名一律来自真实序列化结果，**禁止手抄**（本文件只构造实例，不列举线缆键名）。
//!
//! 两种运行模式：
//!   - 默认（校验）：快照缺失或与当前 serde 键集合不一致 ⇒ 测试失败，提示如何重生成；
//!   - `UPDATE_CONTRACT_SNAPSHOT=1 cargo test -p pony-agent-core --test ts_contract_snapshot`
//!     ⇒ 重写快照文件（提交前必须跑 `npm run contract:check` 复核）。
//!
//! 覆盖面说明：`TurnInput` 是 `#[derive(Deserialize)]`（**无 Serialize**），无法用
//! `to_value` 取真实键名，故不在本快照内；如未来需要覆盖它，请先给它加 `Serialize`
//! 或在实现侧导出键名真源。

use pony_agent_core::agent::runtime::{TurnInput, TurnResult, TurnStreamEvent};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

/// 快照文件（相对仓库根）。
fn snapshot_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/contracts/ts-contract.snapshot.json")
}

fn sorted_keys(value: &Value) -> Vec<String> {
    match value {
        Value::Object(map) => {
            let mut keys = map.keys().cloned().collect::<Vec<_>>();
            keys.sort();
            keys
        }
        other => panic!("契约快照对象必须是 JSON object，实际 {}", other),
    }
}

/// `TurnStreamEvent` 的代表性样本：所有 `skip_serializing_if = "Option::is_none"` 字段
/// 都必须填 `Some`，否则键会从线缆上消失、快照漏键。
fn turn_stream_event_sample() -> Value {
    let event = TurnStreamEvent {
        event_id: Some("contract-event-1".to_string()),
        session_id: Some("contract-session".to_string()),
        turn_id: "contract-turn-1".to_string(),
        // skip_serializing_if is_none：必须给值
        trace_id: Some("4bf92f3577b34da6a3ce929d0e0e4736".to_string()),
        cost_usd: Some(0.5),
        kind: "delta".to_string(),
        event_type: Some("turn.output_delta".to_string()),
        event_version: Some("1.0".to_string()),
        sequence: Some(1),
        emitted_at_ms: Some(1_700_000_000_000),
        phase: Some("calling_model".to_string()),
        text: Some("contract probe".to_string()),
        reasoning_content: Some("contract reasoning".to_string()),
        error: Some("contract error".to_string()),
        provider_requested_name: Some("OpenAI".to_string()),
        provider_name: Some("OpenAI".to_string()),
        provider_protocol: Some("openai-completions".to_string()),
        provider_model: Some("gpt-contract".to_string()),
        provider_source: Some("primary".to_string()),
        provider_mode: Some("standard".to_string()),
        fallback_reason: Some("contract fallback".to_string()),
        build_context_observation: None,
        input_tokens: Some(1),
        cache_hit_input_tokens: Some(1),
        reasoning_tokens: Some(1),
        output_tokens: Some(1),
        total_tokens: Some(1),
        first_token_latency_ms: Some(1),
        turn_duration_ms: Some(1),
        trace_steps: Some(Vec::new()),
        trace_timeline: Some(Vec::new()),
        tool_activities: Some(Vec::new()),
        provider_call_records: Some(Vec::new()),
        hook_trace_records: Some(Vec::new()),
        session_summary: Some("contract summary".to_string()),
        step: Some(1),
        // wave-60：追加 wire 字段（`#[serde(default)]`，非 skip_serializing_if，
        // 但按本夹具「必须给值，否则快照漏键」的约定填 Some，覆盖线缆键 stopReason）。
        stop_reason: Some("user".to_string()),
    };
    serde_json::to_value(&event).expect("TurnStreamEvent must serialize")
}

fn turn_result_sample() -> Value {
    let result = TurnResult {
        event_id: Some("contract-event-1".to_string()),
        event_type: Some("turn.completed".to_string()),
        event_version: Some("1.0".to_string()),
        sequence: Some(1),
        emitted_at_ms: Some(1_700_000_000_000),
        phase: "completed".to_string(),
        provider_requested_name: "OpenAI".to_string(),
        provider_name: "OpenAI".to_string(),
        provider_protocol: "openai-completions".to_string(),
        provider_model: "gpt-contract".to_string(),
        provider_source: "primary".to_string(),
        provider_mode: "standard".to_string(),
        fallback_reason: None,
        build_context_observation: None,
        input_tokens: Some(1),
        cache_hit_input_tokens: Some(1),
        reasoning_tokens: Some(1),
        output_tokens: Some(1),
        total_tokens: Some(1),
        first_token_latency_ms: Some(1),
        turn_duration_ms: Some(1),
        user_message: "contract user".to_string(),
        assistant_message: "contract assistant".to_string(),
        trace_steps: Vec::new(),
        trace_timeline: Vec::new(),
        tool_activities: Vec::new(),
        provider_call_records: Vec::new(),
        hook_trace_records: Vec::new(),
        session_summary: "contract summary".to_string(),
    };
    serde_json::to_value(&result).expect("TurnResult must serialize")
}

fn turn_input_sample() -> Value {
    let input = TurnInput {
        message: "contract message".to_string(),
        display_message: Some("contract display".to_string()),
        provider_id: Some("openai".to_string()),
        model_id: Some("gpt-4o".to_string()),
        reasoning_effort: None,
        workspace_mode: Some("default".to_string()),
        tool_authorization_mode: Some("read_only".to_string()),
        session_id: Some("session-contract".to_string()),
        node_id: Some("node-contract".to_string()),
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: Some("ws-contract".to_string()),
        plan_execution_approval: None,
    };
    serde_json::to_value(&input).expect("TurnInput must serialize")
}

/// 构建完整快照文档：`types.<TypeName>.serdeKeys` 为排序后的真实线缆键。
fn build_snapshot() -> Value {
    let mut types = Map::new();
    for (type_name, sample) in [
        ("TurnStreamEvent", turn_stream_event_sample()),
        ("TurnResult", turn_result_sample()),
        ("TurnInput", turn_input_sample()),
    ] {
        types.insert(
            type_name.to_string(),
            json!({
                "rustSource": rust_source_of(type_name),
                "serdeKeys": sorted_keys(&sample),
            }),
        );
    }

    json!({
        "schemaVersion": 1,
        "generatedBy": "cargo test -p pony-agent-core --test ts_contract_snapshot (UPDATE_CONTRACT_SNAPSHOT=1)",
        "notes": [
            "键名来自 serde_json::to_value 的真实输出，禁止手工编辑本文件。",
            "tsDeclarations 指向 TS 侧声明文件；检查脚本只报「Rust 有、TS 无」的漏声明。"
        ],
        "tsDeclarations": {
            "TurnStreamEvent": "src/types/runtime.ts",
            "TurnResult": "src/types/runtime.ts",
            "TurnInput": "src/types/runtime.ts"
        },
        "types": Value::Object(types)
    })
}

fn rust_source_of(type_name: &str) -> &'static str {
    match type_name {
        "TurnStreamEvent" => "crates/pony-agent-core/src/agent/runtime/types.rs:TurnStreamEvent",
        "TurnResult" => "crates/pony-agent-core/src/agent/runtime/types.rs:TurnResult",
        "TurnInput" => "crates/pony-agent-core/src/agent/runtime/types.rs:TurnInput",
        _ => "unknown",
    }
}

fn read_committed_snapshot() -> Option<Value> {
    let path = snapshot_path();
    let raw = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str::<Value>(&raw).ok()
}

fn write_snapshot(snapshot: &Value) {
    let path = snapshot_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create docs/contracts");
    }
    let mut serialized =
        serde_json::to_string_pretty(snapshot).expect("snapshot must serialize as JSON");
    serialized.push('\n');
    std::fs::write(&path, serialized).expect("write contract snapshot");
}

/// 快照必须与当前 serde 线缆键一致（键集合逐类型比对）。
/// 不一致时测试失败并给出重生成指令 —— 这是"实现改了字段名但忘了同步契约"的机械拦截点。
#[test]
fn serde_wire_keys_match_committed_contract_snapshot() {
    let current = build_snapshot();

    if std::env::var("UPDATE_CONTRACT_SNAPSHOT").as_deref() == Ok("1") {
        write_snapshot(&current);
        eprintln!(
            "[contract] 已重写快照 {}；请运行 `npm run contract:check` 复核 TS 侧声明",
            snapshot_path().display()
        );
        return;
    }

    let committed = read_committed_snapshot().unwrap_or_else(|| {
        panic!(
            "W57-A：契约快照缺失或非法 JSON（{}）。请运行 \
             `UPDATE_CONTRACT_SNAPSHOT=1 cargo test -p pony-agent-core --test ts_contract_snapshot` 重生成",
            snapshot_path().display()
        )
    });

    let committed_types = committed
        .get("types")
        .and_then(Value::as_object)
        .expect("快照缺少 types");
    let current_types = current
        .get("types")
        .and_then(Value::as_object)
        .expect("当前快照缺少 types");

    for (type_name, current_entry) in current_types {
        let expected_keys = current_entry
            .get("serdeKeys")
            .and_then(Value::as_array)
            .expect("serdeKeys must be an array");
        let committed_keys = committed_types
            .get(type_name)
            .and_then(|entry| entry.get("serdeKeys"))
            .and_then(Value::as_array)
            .unwrap_or_else(|| {
                panic!("W57-A：快照缺少类型 `{type_name}`，请重生成快照")
            });

        assert_eq!(
            committed_keys, expected_keys,
            "W57-A：`{type_name}` 的 serde 线缆键与快照不一致 —— 实现改了字段/序列化属性后必须\
             重生成快照（UPDATE_CONTRACT_SNAPSHOT=1），并同步检查 TS 声明"
        );
    }
}
