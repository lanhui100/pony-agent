//! [W2 / task-2] TUI 消费验收：指标行必须取自 `TurnMetricsSnapshot`（快照为唯一来源）。
//!
//! 契约源（只读）：`.dev-team/contract-matrix-metrics-snapshot.md` §1 I6 / I7，§0「两端消费约束」。
//! 冻结 API 面（Lead 裁决，不得改名）：`TuiState::live_metrics` / `TuiState::apply_live_metrics`。
//!
//! 纪律：沿用既有 bin-as-module + ratatui `TestBackend` 渲染取行的写法
//! （辅助函数照抄 `tests/tui_token_context_wave41.rs`），固定夹具、无 sleep、无 `#[ignore]`。
//! 红相锚定：`apply_live_metrics` / `live_metrics` 尚未落地 ⇒ 本文件编译失败（预期红相）。
//!
//! 本文件由 Test Agent（task-2）独占写域。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use pony_agent_core::agent::metrics_snapshot::TurnMetricsSnapshot;
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

/// 底部指标行（既有回归测试 tui_token_context_wave41 / tui_dialog_wave39 的同一取行位置）。
fn metrics_row(state: &mut tui::TuiState) -> String {
    let buf = screen(state, 110, 30);
    row_text(&buf, 29)
}

/// 契约夹具：缓存命中 60%、上下文占比 25%、上下文输入 25k、输出 5k、轮速度 120 t/s、TTFT 800ms。
fn snapshot_fixture() -> TurnMetricsSnapshot {
    TurnMetricsSnapshot {
        context_input_tokens: Some(25_000),
        context_window_tokens: Some(100_000),
        cache_hit_ratio: Some(0.6),
        context_ratio: Some(0.25),
        output_tokens: Some(5_000),
        turn_output_tokens: 5_000,
        turn_speed_tokens_per_second: Some(120.0),
        first_token_latency_ms: Some(800),
        turn_calls: 1,
        provider: "fixture-provider".to_string(),
        model: "fixture-model-unpriced".to_string(),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// C1：快照为唯一来源 —— 缓存 60% / 上下文 25%，绝不出现 100%
// ---------------------------------------------------------------------------

#[test]
fn c1_metrics_line_renders_percentages_from_snapshot() {
    // Arrange：TuiState 保持默认（旧路径计量全为 0），因此任何百分比只可能来自快照。
    let mut state = tui::TuiState::default();
    assert!(
        state.live_metrics.is_none(),
        "新建 TuiState 不得预置快照（旧事件路径必须回退既有行为）"
    );

    // Act
    state.apply_live_metrics(snapshot_fixture());

    // Assert
    assert!(
        state.live_metrics.is_some(),
        "apply_live_metrics 必须把快照写入 live_metrics"
    );
    let last = metrics_row(&mut state);
    assert!(
        last.contains("缓存 60%"),
        "I6：缓存百分比必须 = round(cacheHitRatio * 100) = 60%，实际：{last:?}"
    );
    assert!(
        last.contains("上下文 25%"),
        "I3/I6：上下文百分比必须 = round(contextRatio * 100) = 25%，实际：{last:?}"
    );
    assert!(
        !last.contains("100%"),
        "I6：0.6 的缓存比绝不能被渲染成 100%（clamp 掩盖），实际：{last:?}"
    );
}

// ---------------------------------------------------------------------------
// C2：I6 反例守卫 —— 实现严禁「现场重算」缓存百分比
// ---------------------------------------------------------------------------

#[test]
fn c2_tui_never_recomputes_cache_ratio_from_raw_tokens() {
    // Arrange：ratio = 0.6，但若实现拿 cache_hit_input_tokens / context_input_tokens 自算，
    // 会得到 99_000 / 1_000 = 9900%（或被 clamp 成 100%）——这正是 I6 禁止的路径。
    let mut state = tui::TuiState::default();
    let snapshot = TurnMetricsSnapshot {
        cache_hit_input_tokens: Some(99_000),
        cache_miss_input_tokens: Some(1_000),
        context_input_tokens: Some(1_000),
        context_window_tokens: Some(100_000),
        cache_hit_ratio: Some(0.6),
        context_ratio: Some(0.25),
        output_tokens: Some(5_000),
        turn_output_tokens: 5_000,
        turn_speed_tokens_per_second: Some(120.0),
        first_token_latency_ms: Some(800),
        provider: "fixture-provider".to_string(),
        model: "fixture-model-unpriced".to_string(),
        ..Default::default()
    };

    // Act
    state.apply_live_metrics(snapshot);

    // Assert：只认快照字段，不对原始 token 做任何二次运算
    let last = metrics_row(&mut state);
    assert!(
        last.contains("缓存 60%"),
        "I6：缓存百分比必须直接取自快照的 cacheHitRatio = 0.6 ⇒ 60%，实际：{last:?}"
    );
    assert!(
        !last.contains("9900%"),
        "I6：禁止用 cache_hit_input_tokens / context_input_tokens 现场重算（99000/1000 = 9900%），\
         实际：{last:?}"
    );
    assert!(
        !last.contains("6000%"),
        "I6：禁止任何把比值当百分数的重复换算（0.6 被再乘 100），实际：{last:?}"
    );
    assert!(
        !last.contains("缓存 100%"),
        "I6：禁止用 clamp(0,100) 把越界值掩盖成恒 100%，实际：{last:?}"
    );
}

// ---------------------------------------------------------------------------
// C3：I7 wire 向后兼容 —— 无快照时旧路径语义逐字不回归
// ---------------------------------------------------------------------------

#[test]
fn c3_legacy_path_without_snapshot_is_unchanged() {
    // Arrange：旧事件路径（无 metrics）——与 tui_token_context_wave41 A4 完全同构
    let mut state = tui::TuiState::default();
    state.apply_turn_usage(25_000, 5_000, 0, 30_000);
    state.context_window_tokens = 100_000;
    assert!(
        state.live_metrics.is_none(),
        "I7：旧路径下 live_metrics 必须保持 None"
    );

    // Act
    let last = metrics_row(&mut state);

    // Assert：既有语义（↑25k / 上下文 25%）逐字不变
    assert!(
        last.contains("↑25k"),
        "I7：旧路径输入指标必须仍显示当前上下文占用 ↑25k，实际：{last:?}"
    );
    assert!(
        last.contains("上下文 25%"),
        "I7：旧路径上下文百分比必须仍为 25000/100000 = 25%，实际：{last:?}"
    );
}

// ---------------------------------------------------------------------------
// C4：速度口径自描述（token 速度口径 v2 / I9 / I10）
// ---------------------------------------------------------------------------

/// 口径夹具：速度 120 t/s + 指定 basis。
fn speed_fixture(basis: Option<pony_agent_core::agent::turn_flow::TurnSpeedBasis>) -> TurnMetricsSnapshot {
    TurnMetricsSnapshot {
        turn_speed_tokens_per_second: Some(120.0),
        turn_speed_basis: basis,
        turn_output_tokens: 240,
        turn_generation_ms: 2_000,
        turn_calls: 1,
        provider: "fixture-provider".to_string(),
        model: "fixture-model-unpriced".to_string(),
        ..Default::default()
    }
}

#[test]
fn c4_speed_segment_renders_basis_prefix() {
    use pony_agent_core::agent::turn_flow::TurnSpeedBasis;

    // ① stream_decode（可比解码速度）⇒ 裸值，不带 `≈`（避免把可比速度标成估计值）。
    let mut decoded = tui::TuiState::default();
    decoded.apply_live_metrics(speed_fixture(Some(TurnSpeedBasis::StreamDecode)));
    let decoded_row = metrics_row(&mut decoded);
    assert!(
        decoded_row.contains("120 t/s"),
        "I9：stream_decode 必须渲染裸速度 120 t/s，实际：{decoded_row:?}"
    );
    assert!(
        !decoded_row.contains("≈"),
        "I9：stream_decode 是可比口径，不得加 `≈`（否则与端到端混为一谈），实际：{decoded_row:?}"
    );

    // ② end_to_end（含排队+prefill，跨调用不可比）⇒ 必须带 `≈` 前缀。
    let mut end_to_end = tui::TuiState::default();
    end_to_end.apply_live_metrics(speed_fixture(Some(TurnSpeedBasis::EndToEnd)));
    let e2e_row = metrics_row(&mut end_to_end);
    assert!(
        e2e_row.contains("≈120 t/s"),
        "I10：end_to_end 必须渲染为 `≈120 t/s`（口径不可与前直接比较），实际：{e2e_row:?}"
    );

    // ③ basis 缺席（旧快照/legacy wire）⇒ 仍渲染原值（向后兼容，不伪造口径）。
    let mut legacy = tui::TuiState::default();
    legacy.apply_live_metrics(speed_fixture(None));
    let legacy_row = metrics_row(&mut legacy);
    assert!(
        legacy_row.contains("120 t/s"),
        "向后兼容：basis 缺席的旧快照仍必须显示速度 120 t/s，实际：{legacy_row:?}"
    );
}
