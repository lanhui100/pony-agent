//! P0/P1/P2 回归：TUI header 指标**双口径**（↑in 当前上下文 / ↓out 与缓存会话累计）。
//!
//! 口径契约（与桌面端 `HomeSidebar.vue` 同源）：
//! - `↑in` = **当前上下文**输入（ADR 0035 S1：最近一次调用，禁止跨轮累加）；
//! - `↓out` = **会话累计** Σ per-call 输出（`SessionUsageLedger`，不随轮次清零）；
//! - `缓存 x%` = **会话累计**命中率 Σ命中/Σ上下文输入（fail-closed，越界不 clamp）；
//! - `上下文 x%` = 当前上下文 / 窗口；快照缺窗口时回退调用方解析的真实窗口（P2）；
//! - 轮首（新快照未到）沿用上一轮末值并追加「（上轮）」自描述（P1）。
//!
//! 取行口径：ADR 0078 起指标簇在 header(row 0)；底面最后一行只承载模型身份。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use pony_agent_core::agent::metrics_snapshot::{build_turn_metrics_snapshot, TurnMetricsSnapshot};
use pony_agent_core::agent::session::SessionUsageTotals;
use pony_agent_core::agent::telemetry::{ProviderCallCacheRecord, ProviderRequestKind};
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

fn header_row(state: &mut tui::TuiState) -> String {
    let buf = screen(state, 110, 30);
    row_text(&buf, 0)
}

/// 一轮快照夹具：per-call 段 + 轮级 Σ 段（会话台账只吃 Σ 段）。
fn turn_snapshot(
    current_context: u64,
    turn_context: u64,
    turn_cache: u64,
    turn_output: u64,
    cache_ratio: Option<f64>,
) -> TurnMetricsSnapshot {
    TurnMetricsSnapshot {
        context_input_tokens: Some(current_context),
        context_window_tokens: Some(100_000),
        context_ratio: Some(current_context as f64 / 100_000.0),
        cache_hit_ratio: cache_ratio,
        output_tokens: Some(turn_output),
        turn_output_tokens: turn_output,
        turn_context_input_tokens: turn_context,
        turn_cache_hit_input_tokens: turn_cache,
        turn_calls: 1,
        turn_calls_with_cache: u32::from(turn_cache > 0),
        provider: "fixture-provider".to_string(),
        model: "fixture-model-unpriced".to_string(),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// P0-1：跨轮 —— ↑in 跟随当前上下文，↓out 是会话累计（不得退回末次调用）
// ---------------------------------------------------------------------------
#[test]
fn p0_header_output_is_session_cumulative_while_input_tracks_current_context() {
    let mut state = tui::TuiState::default();

    // 第 1 轮：上下文 10k、输出 500、命中 4k（40%）
    state.apply_live_metrics(turn_snapshot(10_000, 10_000, 4_000, 500, Some(0.4)));
    let first = header_row(&mut state);
    assert!(first.contains("↑10k"), "第 1 轮 ↑in = 当前上下文 10k，实际 {first:?}");
    assert!(first.contains("↓500"), "第 1 轮 ↓out = 会话累计 500，实际 {first:?}");

    // 转轮：轮首结算本轮 Σ（不清零），新快照到达后再吸收本轮 Σ
    state.begin_turn_usage();
    state.apply_live_metrics(turn_snapshot(20_000, 20_000, 5_000, 700, Some(0.25)));

    let second = header_row(&mut state);
    assert!(
        second.contains("↑20k"),
        "第 2 轮 ↑in 必须是当前上下文 20k（不是跨轮累加 30k），实际 {second:?}"
    );
    assert!(
        second.contains("↓1.2k"),
        "第 2 轮 ↓out 必须是**会话累计** 500+700=1200 ⇒ 1.2k（而非末次调用 700），实际 {second:?}"
    );
    let totals = state.session_usage.totals();
    assert_eq!(totals.output_tokens, 1_200, "会话累计输出必须逐轮累加");
    assert_eq!(totals.input_tokens, 30_000, "台账 Σ上下文输入用于命中率分母");
}

// ---------------------------------------------------------------------------
// P0-2：缓存段是**会话累计**命中率（Σ命中/Σ输入），不是末次调用比
// ---------------------------------------------------------------------------
#[test]
fn p0_cache_segment_uses_session_cumulative_ratio() {
    let mut state = tui::TuiState::default();
    state.apply_live_metrics(turn_snapshot(10_000, 10_000, 8_000, 100, Some(0.8)));
    state.begin_turn_usage();
    // 第 2 轮单次比 0.05，但会话累计 = (8000+500)/(10000+10000) = 42.5% ⇒ 43%
    state.apply_live_metrics(turn_snapshot(10_000, 10_000, 500, 100, Some(0.05)));

    let header = header_row(&mut state);
    assert!(
        header.contains("缓存 43%"),
        "缓存段必须是会话累计 8500/20000 ⇒ 43%（不是末次调用 5%），实际 {header:?}"
    );
    assert!(
        !header.contains("缓存 5%"),
        "禁止用末次调用比冒充会话命中率，实际 {header:?}"
    );
}

// ---------------------------------------------------------------------------
// P0-3：会话命中率越界必须 fail-closed（不得 clamp 成 100%）
// ---------------------------------------------------------------------------
#[test]
fn p0_session_cache_ratio_fails_closed_instead_of_clamping() {
    let mut state = tui::TuiState::default();
    // 人为构造矛盾数据（Σ命中 > Σ输入）：会话比不可信 ⇒ 不展示该段（回退到快照单次比 0.3）
    state.session_usage.restore(SessionUsageTotals {
        input_tokens: 100,
        cache_hit_input_tokens: 140,
        output_tokens: 10,
        calls: 1,
        calls_with_cache: 1,
    });
    state.live_metrics = None;
    state.total_input_tokens = 0;
    let header = header_row(&mut state);
    assert!(
        !header.contains("缓存 100%"),
        "越界比必须 fail-closed，绝不能 clamp 成 100%，实际 {header:?}"
    );
    assert!(
        header.contains("↓10"),
        "↓out 仍应展示会话累计（数据矛盾只影响比率），实际 {header:?}"
    );
}

// ---------------------------------------------------------------------------
// P1-1：轮首保留上一轮末值 + 「（上轮）」自描述；新快照到达即复位
// ---------------------------------------------------------------------------
#[test]
fn p1_turn_start_keeps_last_values_and_marks_them_stale() {
    let mut state = tui::TuiState::default();
    state.apply_live_metrics(turn_snapshot(8_000, 8_000, 2_000, 400, Some(0.25)));
    assert!(
        !header_row(&mut state).contains("（上轮）"),
        "实时快照不得带 stale 标注"
    );

    state.begin_turn_usage();
    let stale = header_row(&mut state);
    assert!(
        stale.contains("↓400") && stale.contains("↑8k"),
        "轮首必须保留上一轮末值（整行消失会造成抖动），实际 {stale:?}"
    );
    assert!(
        stale.contains("（上轮）"),
        "沿用上一轮末值时必须自描述（否则被当成当前轮读数），实际 {stale:?}"
    );

    state.apply_live_metrics(turn_snapshot(9_000, 9_000, 3_000, 100, Some(1.0 / 3.0)));
    let fresh = header_row(&mut state);
    assert!(
        !fresh.contains("（上轮）"),
        "新快照到达后 stale 标注必须消失，实际 {fresh:?}"
    );
    assert!(fresh.contains("↑9k"), "新快照 ↑in 必须换成当前上下文，实际 {fresh:?}");
    assert!(
        fresh.contains("↓500"),
        "↓out 跨轮累计 400+100=500，实际 {fresh:?}"
    );
}

// ---------------------------------------------------------------------------
// P1-2：会话恢复回填（`/resume` 走同一路径）—— 累计与上下文都要有值
// ---------------------------------------------------------------------------
#[test]
fn p1_restored_session_renders_cumulative_usage_and_context() {
    let mut state = tui::TuiState::default();
    // 模拟 hydrate_state_from_session 的回填：会话累计取历史 per-call Σ，
    // 当前上下文取末次已结算调用，窗口取注册表真实能力。
    state.session_usage.restore(SessionUsageTotals {
        input_tokens: 50_000,
        cache_hit_input_tokens: 25_000,
        output_tokens: 3_000,
        calls: 4,
        calls_with_cache: 4,
    });
    state.total_input_tokens = 12_000;
    state.last_context_tokens = 12_000;
    state.context_window_tokens = 100_000;

    let header = header_row(&mut state);
    assert!(header.contains("↑12k"), "恢复后 ↑in = 末次调用上下文 12k，实际 {header:?}");
    assert!(header.contains("↓3k"), "恢复后 ↓out = 会话累计 3k，实际 {header:?}");
    assert!(header.contains("缓存 50%"), "恢复后缓存 = 25000/50000 = 50%，实际 {header:?}");
    assert!(header.contains("上下文 12%"), "恢复后上下文 = 12000/100000 = 12%，实际 {header:?}");
}

// ---------------------------------------------------------------------------
// P2：快照缺窗口 ⇒ 回退调用方解析的真实窗口（比例仍由 core 计算）
// ---------------------------------------------------------------------------
#[test]
fn p2_snapshot_without_window_falls_back_to_resolved_window() {
    let mut snapshot = turn_snapshot(25_000, 25_000, 0, 1_000, None);
    snapshot.context_window_tokens = None;
    snapshot.context_ratio = None;

    // 窗口未知：不得猜窗口，也不得出占位符
    let mut unknown = tui::TuiState::default();
    unknown.apply_live_metrics(snapshot.clone());
    let without = header_row(&mut unknown);
    assert!(
        !without.contains("上下文"),
        "快照缺窗口且调用方无窗口时必须无值（禁止猜 200k 常量），实际 {without:?}"
    );

    // 调用方解析出真实窗口 ⇒ 用同一 core 口径回退出上下文占用
    let mut resolved = tui::TuiState::default();
    resolved.context_window_tokens = 100_000;
    resolved.apply_live_metrics(snapshot);
    let with = header_row(&mut resolved);
    assert!(
        with.contains("上下文 25%"),
        "回退真实窗口后 25000/100000 ⇒ 25%，实际 {with:?}"
    );
}

// ---------------------------------------------------------------------------
// P0-4：快照生产侧必须给出**轮级 Σ**（会话累计的唯一原料）
// ---------------------------------------------------------------------------
fn record(
    input: Option<u64>,
    context_input: Option<u64>,
    cache: Option<u64>,
    output: Option<u64>,
) -> ProviderCallCacheRecord {
    ProviderCallCacheRecord {
        request_kind: ProviderRequestKind::default(),
        input_tokens: input,
        context_input_tokens: context_input,
        cache_hit_input_tokens: cache,
        output_tokens: output,
        turn_duration_ms: Some(100),
        ..Default::default()
    }
}

#[test]
fn p0_snapshot_exposes_turn_level_sums_for_session_ledger() {
    let records = vec![
        record(Some(100), Some(120), Some(80), Some(10)),
        // 「未知缓存」调用：计入 Σ输入，但不计入命中分母
        record(Some(200), None, None, Some(20)),
    ];
    let snapshot = build_turn_metrics_snapshot(&records, Some(100_000), None)
        .expect("非空 records 必须产出快照");

    assert_eq!(
        snapshot.turn_context_input_tokens, 320,
        "Σ上下文输入必须逐条相加（120 + 200），不是末次调用 200"
    );
    assert_eq!(
        snapshot.turn_cache_hit_input_tokens, 80,
        "Σ缓存命中 = 80（未知命中不得当 0 也不得伪造）"
    );
    assert_eq!(
        snapshot.turn_calls_with_cache, 1,
        "只有上报了命中的调用计入命中率分母"
    );

    // 端到端：台账据此给出会话命中率 80/320 = 25%
    let mut state = tui::TuiState::default();
    state.apply_live_metrics(snapshot);
    let header = header_row(&mut state);
    assert!(header.contains("缓存 25%"), "会话命中率必须 = 80/320 ⇒ 25%，实际 {header:?}");
    assert!(header.contains("↑200"), "↑in 仍是末次调用 200，实际 {header:?}");
    assert!(header.contains("↓30"), "↓out = 本轮 Σ 10+20 = 30，实际 {header:?}");
}

// ---------------------------------------------------------------------------
// P0-4：诚实百分比 —— 部分命中不得被整数四舍五入谎报成 100%
// （ADR `.agents/notes/implemented/bug-fix/2026-11-12-metrics-percent-honesty.md`）
// ---------------------------------------------------------------------------
#[test]
fn p0_partial_cache_hit_never_displays_as_full_hit() {
    let mut state = tui::TuiState::default();
    state.session_usage.restore(SessionUsageTotals {
        input_tokens: 20_000,
        cache_hit_input_tokens: 19_990,
        output_tokens: 10,
        calls: 1,
        calls_with_cache: 1,
    });
    state.live_metrics = None;
    state.total_input_tokens = 0;

    let header = header_row(&mut state);
    assert!(
        header.contains("缓存 99.95%"),
        "19990/20000 必须显示 99.95%（两位小数向下取整），实际 {header:?}"
    );
    assert!(
        !header.contains("缓存 100%"),
        "部分命中绝不能显示成 100%，实际 {header:?}"
    );
}

#[test]
fn p0_full_cache_hit_displays_as_100() {
    let mut state = tui::TuiState::default();
    state.session_usage.restore(SessionUsageTotals {
        input_tokens: 20_000,
        cache_hit_input_tokens: 20_000,
        output_tokens: 10,
        calls: 1,
        calls_with_cache: 1,
    });
    state.live_metrics = None;
    state.total_input_tokens = 0;

    let header = header_row(&mut state);
    assert!(
        header.contains("缓存 100%"),
        "真全命中（分子 == 分母）必须显示 100%，实际 {header:?}"
    );
}
