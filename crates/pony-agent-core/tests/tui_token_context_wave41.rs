// wave-41: TUI 输入 token 与上下文计算口径验收测试
// 契约：.dev-team/contract-matrix-tui-token-context-wave41.md 对齐 ADR 0035 S1 规范

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use pony_agent_core::agent::runtime::TurnStreamEvent;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;
use serde_json::json;

fn event_from(value: serde_json::Value) -> TurnStreamEvent {
    serde_json::from_value(value).expect("TurnStreamEvent 反序列化失败")
}

fn screen(state: &mut tui::TuiState, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| tui::ui(f, state)).unwrap();
    terminal.backend().buffer().clone()
}

fn disp_width(ch: char) -> usize {
    let cp = ch as u32;
    let wide = matches!(cp,
        0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xA960..=0xA97F
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x3FFFD);
    if wide { 2 } else { 1 }
}

/// 一行在真实终端上的视觉字符串（跳过宽字符右侧的占位 cell）。
fn row_text(buf: &Buffer, y: u16) -> String {
    let mut out = String::new();
    let mut skip_placeholder = false;
    for x in 0..buf.area.width {
        if skip_placeholder {
            skip_placeholder = false;
            continue;
        }
        let sym = buf[(x, y)].symbol();
        out.push_str(sym);
        let ch = sym.chars().next().unwrap_or(' ');
        if disp_width(ch) == 2 {
            skip_placeholder = true;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// A1: 多跳调用时 stream_usage_event 必须按 S1 提取最后一次调用的 input_tokens
// ---------------------------------------------------------------------------
#[test]
fn a1_multihop_stream_usage_extracts_last_call_input() {
    // 模拟多跳工具轮次：
    // call #0: input 1000, output 100
    // call #1: input 1500, output 200
    // 累加 input 字段为 2500，但 records[1].input_tokens 才是 1500
    let payload = event_from(json!({
        "turnId": "turn-multihop",
        "kind": "output_end",
        "inputTokens": 2500,
        "outputTokens": 300,
        "cacheHitInputTokens": 200,
        "totalTokens": 2800,
        "providerCallRecords": [
            {
                "requestKind": "initial_request",
                "inputTokens": 1000,
                "outputTokens": 100,
                "totalTokens": 1100
            },
            {
                "requestKind": "tool_followup",
                "inputTokens": 1500,
                "outputTokens": 200,
                "totalTokens": 1700
            }
        ]
    }));

    let event = tui::stream_usage_event("turn:output_end", &payload)
        .expect("必须产出 usage 事件");

    match event {
        tui::UiEvent::StreamUsage {
            input_tokens,
            output_tokens,
            total_tokens,
            ..
        } => {
            // S1 口径：当前上下文输入必须是最近一次调用的 1500，严禁使用整轮累加值 2500
            assert_eq!(input_tokens, 1500, "多跳调用必须取末条 call 记录的 1500，而不是累加值 2500");
            assert_eq!(output_tokens, 300, "输出 token 仍保持整轮累加");
            assert_eq!(total_tokens, 2800, "总 token 仍保持透传");
        }
        _ => panic!("必须返回 StreamUsage 变体"),
    }
}

// ---------------------------------------------------------------------------
// A2: 无 providerCallRecords 时回退 payload.input_tokens
// ---------------------------------------------------------------------------
#[test]
fn a2_single_call_without_records_fallbacks_to_payload_input() {
    let payload = event_from(json!({
        "turnId": "turn-single",
        "kind": "output_end",
        "inputTokens": 800,
        "outputTokens": 150,
        "cacheHitInputTokens": 100,
        "totalTokens": 950
    }));

    let event = tui::stream_usage_event("turn:output_end", &payload)
        .expect("必须产出 usage 事件");

    match event {
        tui::UiEvent::StreamUsage { input_tokens, .. } => {
            assert_eq!(input_tokens, 800, "无 records 时正确回退 payload.input_tokens");
        }
        _ => panic!("必须返回 StreamUsage 变体"),
    }
}

// ---------------------------------------------------------------------------
// A3: 跨轮 apply_turn_usage 时，当前上下文输入不应跨轮累加，而应作为当前最新上下文覆盖
// ---------------------------------------------------------------------------
#[test]
fn a3_apply_turn_usage_updates_context_input_without_cross_turn_sum() {
    let mut state = tui::TuiState::default();

    // 轮次 1：单次输入 1000, 输出 100
    state.apply_turn_usage(1000, 100, 50, 1100);
    assert_eq!(state.total_input_tokens, 1000, "第 1 轮当前上下文输入为 1000");
    assert_eq!(state.total_output_tokens, 100, "输出为 100");

    // 轮次 2：单次输入 1200, 输出 200
    state.begin_turn_usage();
    state.apply_turn_usage(1200, 200, 100, 1400);

    // S1 口径核心断言：当前输入上下文是当前轮最新输入 1200，绝非 1000 + 1200 = 2200！
    assert_eq!(
        state.total_input_tokens, 1200,
        "跨轮输入 token 必须代表当前最新上下文输入 1200，绝不可累加成 2200"
    );
    // 输出 token 维持跨轮会话累计：100 + 200 = 300
    assert_eq!(state.total_output_tokens, 300, "输出 token 保持跨轮累加");
    // 缓存命中维持跨轮会话累计：50 + 100 = 150
    assert_eq!(state.total_cache_hit_tokens, 150, "缓存命中保持跨轮累加");
}

// ---------------------------------------------------------------------------
// A4: 上下文占用百分比必须以当前上下文输入 token 计算
// ---------------------------------------------------------------------------
#[test]
fn a4_context_percentage_is_based_on_current_context_input() {
    let mut state = tui::TuiState::default();
    // 假设当前上下文输入 25_000，模型上下文窗口 100_000，输出 5_000，总用量 30_000
    state.apply_turn_usage(25_000, 5_000, 0, 30_000);
    state.context_window_tokens = 100_000;

    // 断言分子是 25_000 (25%)，而非 30_000 (30%)
    let buf = screen(&mut state, 110, 30);
    let last = row_text(&buf, 29);

    assert!(
        last.contains("上下文 25%"),
        "上下文百分比必须是当前上下文输入 25000 / 100000 = 25%，而非包含输出后的 30%，实际：{last}"
    );
    assert!(
        last.contains("↑25k"),
        "输入指标必须显示当前上下文占用 25k，实际：{last}"
    );
}
