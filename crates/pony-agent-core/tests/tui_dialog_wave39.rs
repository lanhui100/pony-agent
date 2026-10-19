// wave-39 L2-T：TUI 输入对话框(Composer)收边 + 底部指标行 —— 冻结验收契约（Test Agent 所有，Executor 只读）。
//
// 契约来源：.dev-team/contract-matrix-tui-dialog-wave39.md §3（A1–A10）。
// 红相目标：Protocol 已落盘 `UiEvent::StreamUsage` / `TuiState` 指标字段 /
// `begin_turn_usage` / `apply_turn_usage` / `resolve_context_window_tokens` 空桩（`unimplemented!()`），
// 但 R4–R8（去细线 / 内边距几何 / 去 hint / 指标行 / usage 接线）尚未实现，因此本文件在红相
// 阶段必须「可编译但运行失败（EXIT≠0）」，且失败只能是断言失败或 `unimplemented!()` panic。
//
// 本文件把 `src/bin/tui.rs` 以 bin-as-module 方式 include（与 tui_render_layout.rs 同模式），
// 用 ratatui TestBackend 做真实渲染后断言字符缓冲；bin 的 main 不会被调用，allow 仅用于
// 消除 bin-as-module 固有的 dead_code 噪声，不放宽任何断言。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;

// 与 tui.rs 保持一致的布局常量（只在断言里复算几何，不参与实现）。
const GUTTER: u16 = 3;
const RAIL: usize = 2;
const MAX_CONTENT_WIDTH: u16 = 120;

// ---------------------------------------------------------------------------
// 渲染与取值工具
// ---------------------------------------------------------------------------

fn screen(state: &mut tui::TuiState, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| tui::ui(f, state)).unwrap();
    terminal.backend().buffer().clone()
}

/// 与 tui.rs `char_width` 同构的东亚宽字符判定：缓冲区里宽字符右侧有一格占位空格，
/// 断言中文时必须按「视觉字符串」还原，否则 `"缓存 50%"` 会变成 `"缓 存 50%"` 而假失败。
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
        let mut chars = sym.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            if disp_width(ch) == 2 {
                skip_placeholder = true;
            }
        }
    }
    out
}

fn all_text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 每个 cell 一个 String（宽字符右半格是空格），用于按「列」而不是「字符」定位。
fn row_cells(buf: &Buffer, y: u16) -> Vec<String> {
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol().to_string())
        .collect()
}

/// 内容列几何：(x, width)。与 tui.rs `ui()` 的 content 计算同构。
fn content_geometry(terminal_width: u16) -> (u16, u16) {
    let width = terminal_width
        .saturating_sub(GUTTER * 2)
        .min(MAX_CONTENT_WIDTH);
    ((terminal_width - width) / 2, width)
}

/// 最后一行（wave-39 起 = 指标行）从 `content.x + RAIL` 起的文本；右端 trim。
fn last_row_left(buf: &Buffer) -> String {
    let (cx, _) = content_geometry(buf.area.width);
    let cells = row_cells(buf, buf.area.height - 1);
    cells[cx as usize + RAIL..].concat().trim_end().to_string()
}

/// 三条主体沟槽列（左 0/1，右 W-2/W-1）在任何一行都不得出现内容。
fn assert_gutters_blank(buf: &Buffer, width: u16) {
    let cols = [0u16, 1, width - 2, width - 1];
    for y in 0..buf.area.height {
        for x in cols {
            let sym = buf[(x, y)].symbol();
            assert!(
                sym.trim().is_empty(),
                "第 {y} 行第 {x} 列（沟槽）出现内容 {sym:?}"
            );
        }
    }
}

/// 正文态 + 流式状态词的夹具：用于证明最后一行不再是「状态短语行」。
fn streaming_state(mode: tui::RuntimeMode) -> tui::TuiState {
    let mut state = tui::TuiState::default();
    state.add_user_message("分析一下这个仓库的重试逻辑");
    state.append_assistant_chunk("好的，我先看一下 retry.rs 的实现。");
    state.is_streaming = true;
    state.transition_runtime_mode(mode);
    state.turn_started_at = Some(std::time::Instant::now());
    state
}

// ---------------------------------------------------------------------------
// A1：色带顶部细线去除 —— 全屏只剩 header 一条整行 "▁"
// ---------------------------------------------------------------------------
#[test]
fn a1_only_the_header_rule_keeps_the_hairline_glyph() {
    // Arrange
    let mut state = tui::TuiState::default();
    let (cx, cw) = content_geometry(110);

    // Act
    let buf = screen(&mut state, 110, 30);

    // Assert：全屏含 "▁" 的行必须恰好是第 1 行（header 发丝线）
    let rule_rows: Vec<u16> = (0..buf.area.height)
        .filter(|y| row_text(&buf, *y).contains('▁'))
        .collect();
    assert_eq!(
        rule_rows,
        vec![1u16],
        "色带顶部细线必须删除：全屏 \"▁\" 行只能是 header 发丝线（y=1），实际 {rule_rows:?}"
    );

    // Assert：header 发丝线仍然整行、与内容列同宽同起点
    let cells = row_cells(&buf, 1);
    let dashes = cells.iter().filter(|c| c.as_str() == "▁").count();
    assert_eq!(
        dashes, cw as usize,
        "header 发丝线必须独占整行内容宽（{cw} 列），实际 {dashes} 列"
    );
    assert_eq!(cells[cx as usize], "▁", "header 发丝线必须从内容列起点开始");
    assert_eq!(
        cells[cx as usize + cw as usize - 1],
        "▁",
        "header 发丝线必须铺满到内容列最后一列"
    );

    // Assert：色带上方（y >= 2）任何一行都不允许再出现 "▁"
    for y in 2..buf.area.height {
        let row = row_text(&buf, y);
        assert!(
            !row.contains('▁'),
            "色带上方第 {y} 行仍残留 \"▁\" 细线: {row:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// A2：底部快捷键提示全部消失（正常态）
// ---------------------------------------------------------------------------
#[test]
fn a2_composer_shortcut_hints_are_gone() {
    // Arrange：覆盖 5 种会切换旧 hint 文案的正常态
    let mut states: Vec<(&str, tui::TuiState)> = Vec::new();
    states.push(("empty", tui::TuiState::default()));

    let mut typed = tui::TuiState::default();
    typed.set_input("请分析重试逻辑");
    states.push(("typed", typed));

    let mut scrolled = tui::TuiState::default();
    scrolled.add_user_message("hello");
    scrolled.auto_scroll = false;
    states.push(("scrolled", scrolled));

    let mut streaming = tui::TuiState::default();
    streaming.add_user_message("hello");
    streaming.append_assistant_chunk("world");
    streaming.is_streaming = true;
    streaming.transition_runtime_mode(tui::RuntimeMode::Responding);
    states.push(("streaming", streaming));

    let mut menu = tui::TuiState::default();
    menu.set_input("/");
    states.push(("slash-menu", menu));

    const FORBIDDEN: [&str; 4] = ["Enter 发送", "Esc 中断", "↑↓ 选择", "Shift+Enter"];

    for (label, state) in states.iter_mut() {
        // Act
        let buf = screen(state, 110, 30);
        let text = all_text(&buf);

        // Assert
        for needle in FORBIDDEN {
            assert!(
                !text.contains(needle),
                "[{label}] 正常态仍渲染已废弃的快捷键提示 {needle:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// A3：最后一行不再承载任何状态词（状态词只留在正文活动行）
// ---------------------------------------------------------------------------
#[test]
fn a3_metrics_row_never_renders_status_words() {
    // Arrange：覆盖所有 RuntimeMode 分支
    let mut states: Vec<(&str, tui::TuiState)> = vec![("ready", tui::TuiState::default())];
    states.push(("responding", streaming_state(tui::RuntimeMode::Responding)));
    states.push(("thinking", streaming_state(tui::RuntimeMode::Thinking)));
    states.push((
        "executing-tool",
        streaming_state(tui::RuntimeMode::ExecutingTool("Read  src/main.rs".into())),
    ));
    states.push((
        "waiting-approval",
        streaming_state(tui::RuntimeMode::WaitingApproval),
    ));
    states.push(("completed", streaming_state(tui::RuntimeMode::Completed)));
    states.push((
        "failed",
        streaming_state(tui::RuntimeMode::Failed("401 unauthorized".into())),
    ));

    const FORBIDDEN: [&str; 7] = [
        "就绪", "完成", "失败", "待确认", "写入中", "思考中", "工具 ·",
    ];

    for (label, state) in states.iter_mut() {
        // Act
        let buf = screen(state, 110, 30);
        let last = row_text(&buf, buf.area.height - 1);

        // Assert
        for word in FORBIDDEN {
            assert!(
                !last.contains(word),
                "[{label}] 最后一行（指标行）不得再出现状态词 {word:?}: {last:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// A4：指标行左侧 = workspace 路径标签（HOME 前缀折叠为 ~）
// ---------------------------------------------------------------------------
#[test]
fn a4_metrics_row_left_shows_workspace_label() {
    let (cx, _cw) = content_geometry(110);

    // 用例 1（Unix）：HOME 前缀 → 折叠为 ~/<basename>。
    #[cfg(unix)]
    {
        let home = std::env::var("HOME").unwrap_or_default();
        assert!(
            !home.is_empty(),
            "Unix 环境必须提供 HOME 才能验证 ~ 折叠契约"
        );
        let mut state = tui::TuiState::default();
        state.workspace = std::path::PathBuf::from(&home).join("pony-agent");

        let buf = screen(&mut state, 110, 30);
        let label = last_row_left(&buf);
        assert!(
            label.starts_with("~/pony-agent"),
            "HOME 下的 workspace 必须折叠为 \"~/pony-agent\"，实际标签 {label:?}"
        );
        let cells = row_cells(&buf, 29);
        assert_eq!(
            cells[cx as usize + RAIL],
            "~",
            "路径标签必须从 content.x+RAIL({}) 列起排",
            cx as usize + RAIL
        );
    }

    // 用例 2：非 HOME 前缀 → 完整 display，basename 必须出现在左侧。
    let mut state = tui::TuiState::default();
    state.workspace = std::env::temp_dir().join("wave39-ws-alpha");
    let buf = screen(&mut state, 110, 30);
    let label = last_row_left(&buf);
    assert!(
        label.contains("wave39-ws-alpha"),
        "指标行左侧必须显示 workspace basename，实际标签 {label:?}"
    );
    assert!(
        label.starts_with('/') || label.starts_with('~'),
        "workspace 标签必须以路径形式起排，实际标签 {label:?}"
    );
    let cells = row_cells(&buf, 29);
    let first = cells[cx as usize + RAIL].as_str();
    assert!(
        first == "/" || first == "~",
        "路径标签首字符必须是路径根（/ 或 ~），实际 {first:?}"
    );
}

// ---------------------------------------------------------------------------
// A5：指标行右簇 = ↑in ↓out · 缓存 x% · 上下文 y%，以数字/百分号收尾
// ---------------------------------------------------------------------------
#[test]
fn a5_metrics_row_carries_up_down_cache_and_context() {
    // Arrange：注入非退化数值，保证右端以百分号结束。
    let mut state = tui::TuiState::default();
    state.total_input_tokens = 1200;
    state.total_output_tokens = 345;
    state.total_cache_hit_tokens = 600;
    state.last_context_tokens = 50_000;
    state.context_window_tokens = 200_000;

    // Act
    let buf = screen(&mut state, 110, 30);
    let last = row_text(&buf, 29);

    // Assert：四个契约 token 全部在最后一行
    for needle in ["↑", "↓", "缓存", "上下文"] {
        assert!(last.contains(needle), "指标行缺少 {needle:?}: {last:?}");
    }

    // Assert：右簇顶到内容列最后一列，且以数字/百分号结尾
    let (cx, cw) = content_geometry(110);
    let tail = buf[(cx + cw - 1, 29)].symbol();
    let tail_ch = tail.chars().next().unwrap_or(' ');
    assert!(
        tail_ch.is_ascii_digit() || tail_ch == '%',
        "指标行必须终止于内容列最后一列（x={}）且以数字/百分号结尾，实际 {tail:?}；行文本 {last:?}",
        cx + cw - 1
    );
    let trimmed = last.trim_end();
    let end_ch = trimmed.chars().next_back().unwrap_or(' ');
    assert!(
        end_ch.is_ascii_digit() || end_ch == '%',
        "指标行去尾空白后必须以数字/百分号结尾，实际 {end_ch:?}；行文本 {last:?}"
    );
}

// ---------------------------------------------------------------------------
// A6：110×30 空输入几何 —— 提示符 y=28 / 指标行 y=29 / 顶部内边距 y=27 空白
// ---------------------------------------------------------------------------
#[test]
fn a6_composer_geometry_is_input_row_plus_metrics_row() {
    // Arrange
    let mut state = tui::TuiState::default();
    let (cx, cw) = content_geometry(110);

    // Act
    let buf = screen(&mut state, 110, 30);

    // Assert：提示符行唯一且位于 y=28
    let prompt_rows: Vec<u16> = (0..buf.area.height)
        .filter(|y| row_text(&buf, *y).contains('›'))
        .collect();
    assert_eq!(
        prompt_rows,
        vec![28u16],
        "110×30 空输入的提示符行必须唯一且位于 y=28（input_h=1, bottom_h=3），实际 {prompt_rows:?}"
    );
    assert_eq!(
        buf[(cx, 28)].symbol(),
        "›",
        "提示符必须落在输入区内容列起点 x={cx}"
    );

    // Assert：y=29 是指标行（非空白，含指标簇）
    let metrics = row_text(&buf, 29);
    assert!(
        !metrics.trim().is_empty(),
        "110×30 的 y=29 必须是指标行，不允许整行空白"
    );
    assert!(
        metrics.contains("缓存") && metrics.contains("上下文"),
        "y=29 必须是指标行（含 缓存/上下文），实际 {metrics:?}"
    );

    // Assert：y=27 是色带顶部内边距，必须整行空白（bottom_h=3 的直接证据）
    let pad = row_text(&buf, 27);
    assert!(
        pad.trim().is_empty(),
        "y=27 必须是色带顶部空白内边距（bottom_top=27），实际 {pad:?}"
    );

    // Assert：三条主体沟槽列无内容
    assert_gutters_blank(&buf, 110);

    // Assert：输入行右侧（指标行之外）不得贴到内容列最后一列
    let input_row = row_text(&buf, 28);
    let input_cells = row_cells(&buf, 28);
    assert!(
        input_cells[cx as usize + cw as usize - 1].trim().is_empty(),
        "输入行最后一列 x={} 必须留白，实际 {:?}；行文本 {input_row:?}",
        cx + cw - 1,
        input_cells[cx as usize + cw as usize - 1]
    );
}

// ---------------------------------------------------------------------------
// A7：指标数字格式化 —— 1.2k / 345 / 缓存 50% / 上下文 25%
// ---------------------------------------------------------------------------
#[test]
fn a7_metrics_row_formats_compact_numbers_and_ratios() {
    // Arrange：来自契约 §2 R7 的注入值
    let mut state = tui::TuiState::default();
    state.total_input_tokens = 1200;
    state.total_output_tokens = 345;
    state.total_cache_hit_tokens = 600;
    state.last_context_tokens = 50_000;
    state.context_window_tokens = 200_000;

    // Act
    let buf = screen(&mut state, 110, 30);
    let last = row_text(&buf, 29);

    // Assert
    assert!(
        last.contains("1.2k"),
        "1200 输入 token 必须格式化为 \"1.2k\"，实际 {last:?}"
    );
    assert!(
        last.contains("345"),
        "345 输出 token 必须原样显示，实际 {last:?}"
    );
    assert!(
        last.contains("缓存 50%"),
        "600/1200 缓存命中必须显示 \"缓存 50%\"，实际 {last:?}"
    );
    assert!(
        last.contains("上下文 25%"),
        "50000/200000 上下文占用必须显示 \"上下文 25%\"，实际 {last:?}"
    );
}

// ---------------------------------------------------------------------------
// A8：未知态占位符 —— in=0 → 缓存 —；window=0 → 上下文 —
// ---------------------------------------------------------------------------
#[test]
fn a8_metrics_row_renders_unknown_placeholders() {
    // 用例 1：输入未知（in=0）→ 缓存占用率未知
    let mut state = tui::TuiState::default();
    state.total_input_tokens = 0;
    state.total_cache_hit_tokens = 0;
    state.last_context_tokens = 10_000;
    state.context_window_tokens = 200_000;
    let buf = screen(&mut state, 110, 30);
    let last = row_text(&buf, 29);
    assert!(
        last.contains("缓存 —"),
        "in=0 时缓存占用率必须显示占位符 \"缓存 —\"，实际 {last:?}"
    );
    assert!(
        last.contains("上下文 5%"),
        "10000/200000 上下文仍有值时必须显示 \"上下文 5%\"，实际 {last:?}"
    );

    // 用例 2：上下文窗口未知（window=0）→ 上下文占用率未知
    let mut state = tui::TuiState::default();
    state.total_input_tokens = 500;
    state.total_cache_hit_tokens = 0;
    state.last_context_tokens = 0;
    state.context_window_tokens = 0;
    let buf = screen(&mut state, 110, 30);
    let last = row_text(&buf, 29);
    assert!(
        last.contains("上下文 —"),
        "window=0 时上下文占用率必须显示占位符 \"上下文 —\"，实际 {last:?}"
    );
    assert!(
        last.contains("缓存 0%"),
        "0/500 缓存命中必须显示 \"缓存 0%\"，实际 {last:?}"
    );
}

// ---------------------------------------------------------------------------
// A9：apply_turn_usage 本轮幂等覆盖 + 跨轮累加
// ---------------------------------------------------------------------------
#[test]
fn a9_apply_turn_usage_is_idempotent_per_turn_and_accumulates_across_turns() {
    // Arrange
    let mut state = tui::TuiState::default();

    // Act 1：本轮首次回执
    state.apply_turn_usage(1, 1, 0, 2);

    // Assert 1：首轮回执直接计入
    assert_eq!(state.total_input_tokens, 1, "首轮回执 input 必须计入");
    assert_eq!(state.total_output_tokens, 1, "首轮回执 output 必须计入");
    assert_eq!(state.total_cache_hit_tokens, 0, "首轮回执 cache 必须计入");
    assert_eq!(
        state.last_context_tokens, 2,
        "last_context_tokens 必须跟随最近一轮 total"
    );

    // Act 2：同一轮重复回执（幂等覆盖，不是累加）
    state.apply_turn_usage(3, 2, 1, 5);

    // Assert 2：同一轮二次回执必须覆盖旧值 —— 3 而非 4
    assert_eq!(
        state.total_input_tokens, 3,
        "同一轮二次回执必须幂等覆盖（期望 3，累加会得到 4）"
    );
    assert_eq!(
        state.total_output_tokens, 2,
        "同一轮二次回执必须幂等覆盖 output"
    );
    assert_eq!(
        state.total_cache_hit_tokens, 1,
        "同一轮二次回执必须幂等覆盖 cache"
    );
    assert_eq!(
        state.last_context_tokens, 5,
        "同一轮二次回执后 last_context_tokens 必须为最新 total"
    );

    // Act 3：新一轮（begin 清空本轮回执）后再次回执
    state.begin_turn_usage();
    state.apply_turn_usage(10, 4, 2, 14);

    // Assert 3：跨轮累加 = 3+10 / 2+4 / 1+2，last_context 跟随最新一轮
    assert_eq!(
        state.total_input_tokens, 13,
        "跨轮 input 必须累加（3 + 10）"
    );
    assert_eq!(
        state.total_output_tokens, 6,
        "跨轮 output 必须累加（2 + 4）"
    );
    assert_eq!(
        state.total_cache_hit_tokens, 3,
        "跨轮 cache 必须累加（1 + 2）"
    );
    assert_eq!(
        state.last_context_tokens, 14,
        "last_context_tokens 必须跟随最新一轮 total（14）"
    );
}

// ---------------------------------------------------------------------------
// A10：退化宽度 + 浮层态 —— 不 panic、不越沟槽、浮层不渲染色带
// ---------------------------------------------------------------------------
#[test]
fn a10_degenerate_widths_and_overlay_mode_degrade_without_panic() {
    // 用例 1/2：窄终端仍渲染指标行且不越沟槽
    for (w, h) in [(60u16, 20u16), (80u16, 24u16)] {
        let mut state = tui::TuiState::default();
        let buf = screen(&mut state, w, h);

        assert_eq!(buf.area.width, w, "{w}×{h} 渲染宽度不符");
        assert_eq!(buf.area.height, h, "{w}×{h} 渲染高度不符");
        assert_gutters_blank(&buf, w);

        let last = row_text(&buf, h - 1);
        assert!(
            last.contains("↑") && last.contains("↓"),
            "{w}×{h} 最后一行必须仍是指标行（↑/↓），实际 {last:?}"
        );
        // 窄终端不得把指标行挤到内容列外（右端必须收在内容列内）
        let (cx, cw) = content_geometry(w);
        let right_edge = row_cells(&buf, h - 1);
        assert!(
            right_edge[(cx + cw) as usize].trim().is_empty(),
            "{w}×{h} 指标行越过内容列右边界（x={}）",
            cx + cw
        );
    }

    // 用例 3：浮层态（mode != Normal）composer + 指标行必须整体收起
    let mut state = tui::TuiState::default();
    state.total_input_tokens = 1200;
    state.total_output_tokens = 345;
    state.total_cache_hit_tokens = 600;
    state.last_context_tokens = 50_000;
    state.context_window_tokens = 200_000;
    state.open_session_selector(vec![tui::SessionItem {
        id: "session-wave39".into(),
        title: "Wave 39".into(),
        updated_at: "0".into(),
    }]);
    assert_ne!(
        state.mode,
        tui::InteractionMode::Normal,
        "夹具必须进入浮层态"
    );

    let buf = screen(&mut state, 110, 30);
    let last = row_text(&buf, 29);
    assert!(
        !last.contains("缓存") && !last.contains("上下文") && !last.contains("↑"),
        "浮层态最后一行不得渲染指标簇，实际 {last:?}"
    );
    let text = all_text(&buf);
    assert!(
        !text.contains('›'),
        "浮层态不得渲染色带输入提示符，实际屏幕: {text:?}"
    );
    assert_gutters_blank(&buf, 110);
}
