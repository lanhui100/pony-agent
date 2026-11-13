//! header 右槽指标溢出回归（ADR 2026-11-14-tui-header-metrics-overflow）。
//!
//! 缺陷：`decode n=<样本量>` 落地后指标簇变长，中等宽度终端仍走
//! `ellipsize_width(&right_meta, budget)` 的**右截断**——它把最右端的「首token <ms>」
//! 先砍掉，甚至砍成 `首t…`，用户看到的最后一项永远是半截文字。
//!
//! 契约（本文件锁定）：
//! - 宽度不足时**整段**让位（从最左开始），绝不输出半段、绝不留悬挂分隔符；
//! - 右端的轮级实时读数（速度含口径/样本量、首token）最后退出；
//! - 有让位时以 `… ` 自描述；
//! - 只有连最后一段都放不下（极端窄屏）才允许段内省略。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use pony_agent_core::agent::metrics_snapshot::TurnMetricsSnapshot;
use pony_agent_core::agent::turn_flow::TurnSpeedBasis;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

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
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x3FFFD);
    if wide {
        2
    } else {
        1
    }
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

fn header_row(state: &mut tui::TuiState, width: u16) -> String {
    let buf = screen(state, width, 30);
    row_text(&buf, 0)
}

/// 覆盖全部指标段的夹具：累计量 + 比率 + 带口径/样本量/剔突发的速度 + 首token。
fn state_with_all_metrics() -> tui::TuiState {
    let mut state = tui::TuiState::default();
    state.workspace = "/home/dm/pony-agent".into();
    state.home_dir = Some("/home/dm".to_string());
    state.apply_live_metrics(TurnMetricsSnapshot {
        context_input_tokens: Some(25_000),
        context_window_tokens: Some(100_000),
        cache_hit_ratio: Some(0.6),
        context_ratio: Some(0.25),
        output_tokens: Some(5_000),
        turn_output_tokens: 5_000,
        turn_speed_tokens_per_second: Some(120.0),
        turn_speed_basis: Some(TurnSpeedBasis::StreamDecode),
        turn_calls: 1,
        turn_burst_excluded: Some(2),
        first_token_latency_ms: Some(800),
        provider: "fixture-provider".to_string(),
        model: "fixture-model-unpriced".to_string(),
        ..Default::default()
    });
    state
}

// ---------------------------------------------------------------------------
// C1：宽度压力下「首token」整段存活（旧实现会先砍掉它 / 砍成 `首t…`）
// ---------------------------------------------------------------------------
#[test]
fn c1_first_token_segment_survives_width_pressure() {
    for width in [60u16, 72, 80, 88, 92, 96, 100] {
        let mut state = state_with_all_metrics();
        let header = header_row(&mut state, width);
        assert!(
            header.contains("首token 800ms"),
            "w={width}：首token 必须整段可见（不得被速度段挤掉），实际 {header:?}"
        );
        assert!(
            !header.contains("首t…") && !header.contains("首token…"),
            "w={width}：禁止输出半截首token 段，实际 {header:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// C2：中等宽度 —— 整段让位 + `… ` 自描述，速度段（含 decode/n=/剔突发）不被切半
// ---------------------------------------------------------------------------
#[test]
fn c2_overflow_drops_whole_segments_and_keeps_speed_self_description() {
    let mut full_state = state_with_all_metrics();
    let full = header_row(&mut full_state, 160);
    assert!(
        !full.contains('…'),
        "宽屏必须全量展示、不得出现让位标记，实际 {full:?}"
    );
    for needle in [
        "↑25k ↓5k",
        "缓存 60%",
        "上下文 25%",
        "120 t/s decode n=1 剔突发2",
        "首token 800ms",
    ] {
        assert!(full.contains(needle), "宽屏缺少指标段 {needle:?}：{full:?}");
    }

    let mut state = state_with_all_metrics();
    let header = header_row(&mut state, 96);
    assert!(
        header.contains("… "),
        "发生让位时必须用 `… ` 自描述，实际 {header:?}"
    );
    assert!(
        header.contains("120 t/s decode n=1 剔突发2"),
        "速度段的口径/样本量/剔突发后缀是契约字面量，不得被拆散，实际 {header:?}"
    );
    assert!(
        header.contains("首token 800ms"),
        "首token 必须保留在右端，实际 {header:?}"
    );
    for orphan in [" ·  · ", "· …", "… ·"] {
        assert!(
            !header.contains(orphan),
            "让位后不得出现悬挂分隔符 {orphan:?}，实际 {header:?}"
        );
    }
    assert!(
        !header.trim_end().ends_with('·'),
        "让位后不得以分隔符收尾，实际 {header:?}"
    );
}

// ---------------------------------------------------------------------------
// C3：极端窄屏 —— 允许段内省略，但绝不 panic、绝不把整簇丢掉后残留孤立分隔符
// ---------------------------------------------------------------------------
#[test]
fn c3_extreme_narrow_degrades_to_single_ellipsized_segment() {
    for width in [40u16, 44, 48] {
        let mut state = state_with_all_metrics();
        let header = header_row(&mut state, width);
        // 该区间仍进入「中等宽度」档（pad > left_w + 12），必须至少保留首token 的段首。
        assert!(
            header.contains("首token"),
            "w={width}：极端窄屏也必须保留最右读数的段首，实际 {header:?}"
        );
        assert!(
            !header.contains("… ·"),
            "w={width}：省略后不得留下孤立分隔符，实际 {header:?}"
        );
    }
}
