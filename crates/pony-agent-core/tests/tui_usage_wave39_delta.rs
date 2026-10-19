// wave-39-delta L2-T（契约 v2 §5 A11）：`stream_usage_event` 事件口径冻结验收
// —— Test Agent 所有，Executor 只读；**不影响、不修改 A1–A10 冻结文件**
// （`tests/tui_dialog_wave39.rs` 保持原样）。
//
// 契约来源：.dev-team/contract-matrix-tui-dialog-wave39.md §5 R8' / A11。
// 缺陷根因：`turn:output_end` 与 `turn:completed` 携带同一 `persisted` token 值，若两者都发
// `UiEvent::StreamUsage`，主循环各 apply 一次；当用户在新旧轮交叠时提交新轮且新轮失败/取消
// （无 usage 回执）时，旧轮 usage 会以「本轮回执」身份被二次计入累计 → 指标永久多算。
// 因此 usage 只允许由 `turn:output_end` 发射，`turn:completed` 不得再发。
//
// 红相目标：Protocol 已落盘空桩 `stream_usage_event(...) { unimplemented!() }`，
// 本文件必须「编译通过（Exit 0）+ 运行失败（unimplemented panic）」。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use pony_agent_core::agent::runtime::TurnStreamEvent;
use serde_json::json;

// ---------------------------------------------------------------------------
// 载荷构造：TurnStreamEvent 字段极多（约 30 个 Option），用 serde 反序列化构造，
// 只声明本契约关心的字段；未声明字段全部为 None（与真实 wire 载荷同构）。
// ---------------------------------------------------------------------------

fn event_from(value: serde_json::Value) -> TurnStreamEvent {
    serde_json::from_value(value).expect("TurnStreamEvent 反序列化失败（字段名/类型与 wire 契约不一致）")
}

/// 完整 usage 载荷：input=1200 / output=345 / cache_hit=600 / total=1545。
fn full_usage_event() -> TurnStreamEvent {
    event_from(json!({
        "turnId": "turn-wave39-delta",
        "kind": "output_end",
        "inputTokens": 1200,
        "outputTokens": 345,
        "cacheHitInputTokens": 600,
        "totalTokens": 1545,
    }))
}

// ---------------------------------------------------------------------------
// A11-1：turn:output_end + 全量 token 字段 → Some(StreamUsage)，四值逐一一致
// ---------------------------------------------------------------------------
#[test]
fn a11_output_end_yields_usage() {
    // Arrange
    let payload = full_usage_event();

    // Act
    let event = tui::stream_usage_event("turn:output_end", &payload)
        .expect("turn:output_end 且任一 token 字段为 Some 时必须产出 usage 事件");

    // Assert：变体与四个数值逐一比对（UiEvent 无 PartialEq，按字段断言）
    match event {
        tui::UiEvent::StreamUsage {
            input_tokens,
            output_tokens,
            cache_hit_input_tokens,
            total_tokens,
        } => {
            assert_eq!(input_tokens, 1200, "input_tokens 必须来自载荷 inputTokens");
            assert_eq!(output_tokens, 345, "output_tokens 必须来自载荷 outputTokens");
            assert_eq!(
                cache_hit_input_tokens, 600,
                "cache_hit_input_tokens 必须来自载荷 cacheHitInputTokens"
            );
            assert_eq!(total_tokens, 1545, "total_tokens 必须来自载荷 totalTokens");
        }
        _ => panic!("stream_usage_event 必须返回 UiEvent::StreamUsage 变体"),
    }
}

// ---------------------------------------------------------------------------
// A11-2：turn:completed + 同一载荷 → None（双发去重 = 缺陷根因）
// ---------------------------------------------------------------------------
#[test]
fn a11_completed_yields_none() {
    // Arrange：与 A11-1 完全相同的 persisted token 载荷（真实 wire 上 completed 亦携带它）
    let payload = full_usage_event();

    // Act
    let event = tui::stream_usage_event("turn:completed", &payload);

    // Assert
    assert!(
        event.is_none(),
        "turn:completed 与 turn:output_end 双发同一 persisted token：completed 必须返回 None，\
         否则主循环二次 apply 会造成累计永久多算"
    );
}

// ---------------------------------------------------------------------------
// A11-3：turn:delta / turn:failed / turn:tool → None（非 usage 权威事件）
// ---------------------------------------------------------------------------
#[test]
fn a11_other_events_yield_none() {
    // Arrange：带全量 token 字段的载荷，确保「事件名」是唯一的过滤条件
    let payload = full_usage_event();

    // Act + Assert
    for name in ["turn:delta", "turn:failed", "turn:tool"] {
        match tui::stream_usage_event(name, &payload) {
            None => {}
            Some(_) => panic!("{name} 不是 usage 权威事件，必须返回 None"),
        }
    }
}

// ---------------------------------------------------------------------------
// A11-4：缺失字段补齐 0 —— 仅 output_tokens=Some(5) → Some，其余三值 0
// ---------------------------------------------------------------------------
#[test]
fn a11_missing_fields_default_zero() {
    // Arrange：只带一个 token 字段（部分 provider 只回报 output/total）
    let payload = event_from(json!({
        "turnId": "turn-wave39-delta-partial",
        "kind": "output_end",
        "outputTokens": 5,
    }));

    // Act
    let event = tui::stream_usage_event("turn:output_end", &payload)
        .expect("任一 token 字段为 Some 即必须产出 usage 事件");

    // Assert
    match event {
        tui::UiEvent::StreamUsage {
            input_tokens,
            output_tokens,
            cache_hit_input_tokens,
            total_tokens,
        } => {
            assert_eq!(output_tokens, 5, "存在字段必须原样透传");
            assert_eq!(input_tokens, 0, "缺失 inputTokens 必须补 0");
            assert_eq!(
                cache_hit_input_tokens, 0,
                "缺失 cacheHitInputTokens 必须补 0"
            );
            assert_eq!(total_tokens, 0, "缺失 totalTokens 必须补 0");
        }
        _ => panic!("stream_usage_event 必须返回 UiEvent::StreamUsage 变体"),
    }
}
