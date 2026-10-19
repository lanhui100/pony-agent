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
use ratatui::style::Color;
use ratatui::Terminal;

// 与 tui.rs 保持一致的布局常量（只在断言里复算几何，不参与实现）。
const GUTTER: u16 = 3;
const RAIL: usize = 2;
const MAX_CONTENT_WIDTH: u16 = 120;

// 契约 v3 §6 R9/R10 主题字面值（`tui::theme` 为私有模块，无法从测试引用；按现实现值冻结）。
// CANVAS = 正文/指标行背景；SURFACE = 色带背景；GHOST = 指标行前景（比 FAINT 更暗一档）。
const CANVAS: Color = Color::Rgb(18, 16, 14);
const SURFACE: Color = Color::Rgb(27, 25, 22);
const GHOST: Color = Color::Rgb(104, 98, 90);

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
        "就绪", "完成", "失败", "待确认", "生成中", "思考中", "执行 ·",
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
// A5：有值时右簇 = ↑in ↓out · 缓存 x% · 上下文 y%；无值时右簇为空（v3 §6 R11）
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

    // Assert 1：四个契约 token 全部在最后一行
    for needle in ["↑", "↓", "缓存", "上下文"] {
        assert!(last.contains(needle), "指标行缺少 {needle:?}: {last:?}");
    }

    // Assert 2：右簇顶到内容列最后一列，且以数字/百分号结尾
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

    // Assert 3（v3 R11）：默认全零状态 → 右簇整段为空，只剩左侧 workspace 标签
    let mut zero = tui::TuiState::default();
    let buf = screen(&mut zero, 110, 30);
    let row = row_text(&buf, 29);
    for needle in ["↑", "↓", "缓存", "上下文", "—"] {
        assert!(
            !row.contains(needle),
            "无值状态不得渲染指标段 {needle:?}（v3 R11：无值不显示）: {row:?}"
        );
    }
    let ws = zero
        .workspace
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();
    assert!(
        row.contains(&ws),
        "无值状态仍必须显示左侧 workspace 标签 {ws:?}: {row:?}"
    );
}

// ---------------------------------------------------------------------------
// A6：110×30 单行输入几何 v4 —— bottom_top=26 / 提示符 26..27 / 色带呼吸行 28 SURFACE / 指标行 29
// ---------------------------------------------------------------------------
#[test]
fn a6_composer_geometry_is_band_padding_input_spacing_and_metrics() {
    // Arrange
    let mut state = tui::TuiState::default();
    let (cx, cw) = content_geometry(110);
    let ws = state
        .workspace
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();

    // Act
    let buf = screen(&mut state, 110, 30);

    // Assert 1（R13：bottom_top=26）：y=25 是正文画布，y=26 是色带顶部内边距（SURFACE 空白）
    assert!(
        row_text(&buf, 25).trim().is_empty(),
        "y=25 应为正文区空白，实际 {:?}",
        row_text(&buf, 25)
    );
    assert_eq!(
        buf[(cx, 25)].bg, CANVAS,
        "y=25 必须仍是正文画布 CANVAS（v4 色带不得从 25 行开始）"
    );
    assert!(
        row_text(&buf, 26).trim().is_empty(),
        "y=26 色带顶部内边距必须整行空白，实际 {:?}",
        row_text(&buf, 26)
    );
    assert_eq!(
        buf[(cx, 26)].bg, SURFACE,
        "y=26 必须是色带顶部内边距（SURFACE）—— 即 bottom_top=26"
    );

    // Assert 2（R13：input_h=2）：提示符行唯一，位于输入区 26..=27
    let prompt_rows: Vec<u16> = (0..buf.area.height)
        .filter(|y| row_text(&buf, *y).contains('›'))
        .collect();
    assert_eq!(
        prompt_rows.len(),
        1,
        "单行输入必须恰好一个提示符行，实际 {prompt_rows:?}"
    );
    let prompt_y = prompt_rows[0];
    assert!(
        (26..=27).contains(&prompt_y),
        "v4 几何下提示符必须位于 26..=27（input_h=2），实际 y={prompt_y}"
    );
    assert_eq!(
        buf[(cx, prompt_y)].symbol(),
        "›",
        "提示符必须落在输入区内容列起点 x={cx}（y={prompt_y}）"
    );

    // Assert 3（R13：间距减半 —— 独立 CANVAS 间距行已删除）：
    // y=28 是**色带内**呼吸行：整行空白且 bg=SURFACE（不再是 CANVAS 间距行）。
    let breathing = row_text(&buf, 28);
    assert!(
        breathing.trim().is_empty(),
        "y=28 色带呼吸行必须整行空白，实际 {breathing:?}"
    );
    assert_eq!(
        buf[(cx, 28)].bg, SURFACE,
        "y=28 必须是色带内呼吸行（SURFACE）；v4 已删除独立 CANVAS 间距行"
    );
    // 输入正文行（27）→ 指标行（29）之间只隔 1 行空白（v3 为 2 行）。
    assert_eq!(
        prompt_y + 2,
        29,
        "输入正文行与指标行之间必须只隔 1 行（呼吸行 28），实际 prompt_y={prompt_y}"
    );

    // Assert 4（R13：指标行 29，紧贴色带）：CANVAS 背景 + GHOST 前景 + workspace 仍在
    let metrics = row_text(&buf, 29);
    assert!(
        !metrics.trim().is_empty(),
        "y=29 必须是指标行（至少 workspace 标签），不允许整行空白"
    );
    assert!(
        metrics.contains(&ws),
        "y=29 指标行必须显示 workspace basename {ws:?}，实际 {metrics:?}"
    );
    assert_eq!(
        buf[(cx, 29)].bg, CANVAS,
        "y=29 指标行必须是 CANVAS 背景（非色带 SURFACE）"
    );
    assert_eq!(
        buf[(cx, 29)].fg, GHOST,
        "y=29 指标行字形前景必须是 GHOST Rgb(104,98,90)"
    );

    // Assert 5：三条主体沟槽列无内容
    assert_gutters_blank(&buf, 110);

    // Assert 6：输入行右侧（指标行之外）不得贴到内容列最后一列
    let input_row = row_text(&buf, prompt_y);
    let input_cells = row_cells(&buf, prompt_y);
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
// A8：无值不显示（v3 §6 R11）—— in=0 / window=0 时不出现对应段，也不出现占位符 "—"
// ---------------------------------------------------------------------------
#[test]
fn a8_metrics_row_hides_segments_without_values() {
    let ws = tui::TuiState::default()
        .workspace
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string();

    // 用例 1：全零（in=0 / out=0 / window=0）→ 右簇整段为空，只剩 workspace
    let mut state = tui::TuiState::default();
    let buf = screen(&mut state, 110, 30);
    let last = row_text(&buf, 29);
    for needle in ["↑", "↓", "缓存", "上下文", "—"] {
        assert!(
            !last.contains(needle),
            "全零状态不得出现指标段/占位符 {needle:?}（v3 R11）：{last:?}"
        );
    }
    assert!(
        last.contains(&ws),
        "全零状态仍必须显示 workspace {ws:?}: {last:?}"
    );

    // 用例 2：in=0 但 window>0 → 只有「上下文」段；缓存比不可算，不得出现 缓存/↑/↓
    let mut state = tui::TuiState::default();
    state.total_input_tokens = 0;
    state.total_output_tokens = 0;
    state.total_cache_hit_tokens = 0;
    state.last_context_tokens = 50_000;
    state.context_window_tokens = 200_000;
    let buf = screen(&mut state, 110, 30);
    let last = row_text(&buf, 29);
    assert!(
        last.contains("上下文 25%"),
        "window>0 时上下文段必须显示（50000/200000=25%）：{last:?}"
    );
    for needle in ["缓存", "↑", "↓", "—"] {
        assert!(
            !last.contains(needle),
            "in=0 时不得显示 {needle:?}（缓存比不可算，也不再用占位符）：{last:?}"
        );
    }

    // 用例 3：window=0 且 last_ctx>0 → 上下文段不得显示；in>0 时 ↑/↓/缓存 正常
    let mut state = tui::TuiState::default();
    state.total_input_tokens = 500;
    state.total_output_tokens = 0;
    state.total_cache_hit_tokens = 0;
    state.last_context_tokens = 12_345;
    state.context_window_tokens = 0;
    let buf = screen(&mut state, 110, 30);
    let last = row_text(&buf, 29);
    assert!(
        !last.contains("上下文"),
        "window=0 时上下文段不得显示：{last:?}"
    );
    assert!(
        last.contains("↑500") && last.contains("↓0"),
        "in>0 时 ↑/↓ 段必须显示：{last:?}"
    );
    assert!(
        last.contains("缓存 0%"),
        "in>0 时缓存段必须显示（0/500=0%）：{last:?}"
    );
    assert!(!last.contains('—'), "v3 起不再使用占位符 —：{last:?}");
}

// ---------------------------------------------------------------------------
// A9：apply_turn_usage 本轮幂等覆盖 + 跨轮累加
// ---------------------------------------------------------------------------
#[test]
fn a9_apply_turn_usage_is_idempotent_per_turn_and_accumulates_across_turns() {
    // Arrange
    let mut state = tui::TuiState::default();

    // Act 1：本轮首次回执 (input=1, output=1, cache=0, total=2)
    state.apply_turn_usage(1, 1, 0, 2);

    // Assert 1：首轮回执直接计入，last_context_tokens 遵循 S1 单次上下文输入
    assert_eq!(state.total_input_tokens, 1, "首轮回执 input 必须计入");
    assert_eq!(state.total_output_tokens, 1, "首轮回执 output 必须计入");
    assert_eq!(state.total_cache_hit_tokens, 0, "首轮回执 cache 必须计入");
    assert_eq!(
        state.last_context_tokens, 1,
        "last_context_tokens 遵循 S1 必须记录当前上下文输入"
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
        state.last_context_tokens, 3,
        "同一轮二次回执后 last_context_tokens 必须为最新上下文输入 3"
    );

    // Act 3：新一轮（begin 清空本轮回执）后再次回执
    state.begin_turn_usage();
    state.apply_turn_usage(10, 4, 2, 14);

    // Assert 3：wave-41 对齐 ADR 0035 S1 规范：当前输入为单次上下文输入 10（非跨轮累加 13）；输出与缓存保持跨轮累加
    assert_eq!(
        state.total_input_tokens, 10,
        "wave-41：输入 token 对齐 ADR 0035 S1 当前单次上下文输入（10，非 3+10）"
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
        state.last_context_tokens, 10,
        "last_context_tokens 必须跟随最新一轮上下文输入（10）"
    );
}

// ---------------------------------------------------------------------------
// A10：退化宽度 + 浮层态 —— 不 panic、不越沟槽、浮层不渲染色带
// ---------------------------------------------------------------------------
#[test]
fn a10_degenerate_widths_and_overlay_mode_degrade_without_panic() {
    // 用例 1/2：窄终端仍渲染指标行且不越沟槽
    // （v3 R11：无值不显示，故注入有值状态，右簇才有 ↑/↓ 段可断言）
    for (w, h) in [(60u16, 20u16), (80u16, 24u16)] {
        let mut state = tui::TuiState::default();
        state.total_input_tokens = 12;
        state.total_output_tokens = 3;
        state.total_cache_hit_tokens = 6;
        state.last_context_tokens = 50_000;
        state.context_window_tokens = 200_000;
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

// ---------------------------------------------------------------------------
// A12（v3 §6 R9/R10）：指标行 = GHOST 前景 + CANVAS 背景；间距行 28 全空白 + CANVAS
// ---------------------------------------------------------------------------

/// v3 R11：段间分隔符必须恰好是 `" · "`；只检测**真正的孤立分隔符**。
///
/// 仅检查：行首/簇首 `·`、簇尾 `·`、连续分隔符（`"· ·"` / `"·  ·"`）。合法的段间
/// `" · "` 前缀会自然以 `·` 结尾，不能据此判为孤立（原实现误杀 `"↑500 ↓0 · 缓存 20%"`）。
fn assert_no_orphan_separator(row: &str, label: &str) {
    // 右簇 = 行内第一个指标段到行尾（workspace 路径不可能含 ↑/缓存/上下文）。
    let cluster_start = ["↑", "缓存", "上下文"]
        .iter()
        .filter_map(|token| row.find(token))
        .min()
        .unwrap_or(row.len());
    let cluster = row[cluster_start..].trim();
    if cluster.is_empty() {
        // 无值不显示：右簇为空时由各用例的存在性断言负责，这里无结构可查。
        return;
    }
    assert!(
        !cluster.starts_with('·'),
        "[{label}] 指标右簇不得以孤立分隔符开头: {cluster:?}"
    );
    assert!(
        !cluster.ends_with('·'),
        "[{label}] 指标右簇不得以孤立分隔符结尾: {cluster:?}"
    );
    assert!(
        !cluster.contains("· ·") && !cluster.contains("·  ·"),
        "[{label}] 指标段之间不得出现连续孤立分隔符: {cluster:?}"
    );
    // 结构断言：段间恰好一个 " · "，各段非空且不含首尾空格（空段会同时暴露为 "· ·"）。
    for part in cluster.split(" · ") {
        assert!(
            !part.is_empty(),
            "[{label}] 分隔符之间存在空段: {cluster:?}"
        );
        assert_eq!(
            part,
            part.trim(),
            "[{label}] 指标段不得含首尾空格: {part:?}（簇 {cluster:?}）"
        );
    }
}

#[test]
fn a12_metrics_row_is_ghost_on_canvas_and_breathing_row_is_band_surface() {
    // Arrange：注入非零值，保证指标行右簇有字形（左簇 workspace 始终有字形）
    let mut state = tui::TuiState::default();
    state.total_input_tokens = 1200;
    state.total_output_tokens = 345;
    state.total_cache_hit_tokens = 600;
    state.last_context_tokens = 50_000;
    state.context_window_tokens = 200_000;
    let (cx, cw) = content_geometry(110);

    // Act
    let buf = screen(&mut state, 110, 30);

    // Assert 1（R10）：指标行**仅在内容列内**断言 —— 非续格 bg=CANVAS、字形 fg=GHOST。
    //
    // 用例缺陷修正（v4b）：原实现遍历终端整行，迫使实现把呼吸行 SURFACE 铺满整行，
    // 会在对话框底部形成比对话框更宽的横条，破坏 ADR 0038 的沟槽纪律。指标行/呼吸行
    // 始终与内容列同宽，故只断言 `content.x..content.x+content.width`。
    // （v3 test-fix 的宽字符续格跳过逻辑保留：ratatui 0.29 `Buffer::diff` 对续格置 to_skip，
    // TestBackend 首帧该格为 `Cell::default`，不承载独立 fg/bg 契约。）
    let mut glyph_cells = 0usize;
    let mut skip_continuation = false;
    for x in cx..cx + cw {
        if skip_continuation {
            skip_continuation = false;
            continue;
        }
        let cell = &buf[(x, 29)];
        assert_eq!(
            cell.bg, CANVAS,
            "指标行内容列第 {x} 列背景必须是 CANVAS（非色带 SURFACE），实际 {:?}",
            cell.bg
        );
        let symbol = cell.symbol();
        if !symbol.trim().is_empty() {
            glyph_cells += 1;
            assert_eq!(
                cell.fg,
                GHOST,
                "指标行内容列第 {x} 列字形前景必须是 GHOST Rgb(104,98,90)，实际 {:?}（symbol={:?}）",
                cell.fg,
                symbol
            );
        }
        let mut chars = symbol.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            if disp_width(ch) == 2 {
                skip_continuation = true;
            }
        }
    }
    assert!(
        glyph_cells > 0,
        "指标行必须至少渲染一个字形（workspace 标签 / 指标段）"
    );

    // Assert 2（R13/R14）：y=28 是**色带内**呼吸行 —— **内容列**逐格空白且 bg=SURFACE。
    // v4 已删除独立 CANVAS 间距行；但色带与内容列同宽，不得因呼吸行把 SURFACE 抹到沟槽。
    for x in cx..cx + cw {
        let cell = &buf[(x, 28)];
        assert!(
            cell.symbol().trim().is_empty(),
            "色带呼吸行内容列第 {x} 列不得有字符，实际 {:?}",
            cell.symbol()
        );
        assert_eq!(
            cell.bg, SURFACE,
            "色带呼吸行内容列第 {x} 列背景必须是 SURFACE（v4：呼吸行属于色带），实际 {:?}",
            cell.bg
        );
    }

    // Assert 3（ADR 0038 沟槽纪律）：沟槽列（0/1/W-2/W-1）在呼吸行（28）与指标行（29）
    // 必须保持 CANVAS 背景且无字形 —— 色带/指标不得越出内容列。
    assert_eq!(cx, 3, "110×30 的内容列起点应为 3（GUTTER=3）");
    for x in [0u16, 1, buf.area.width - 2, buf.area.width - 1] {
        for y in [28u16, 29u16] {
            let cell = &buf[(x, y)];
            assert!(
                cell.symbol().trim().is_empty(),
                "沟槽列 x={x}（y={y}）不得有字形，实际 {:?}",
                cell.symbol()
            );
            assert_eq!(
                cell.bg, CANVAS,
                "沟槽列 x={x}（y={y}）背景必须是 CANVAS（呼吸行/指标行不得铺满整行），实际 {:?}",
                cell.bg
            );
        }
    }
}

// ---------------------------------------------------------------------------
// A13（v3 §6 R11）：分段隐藏组合 —— 有值才显示对应段，且不得出现孤立分隔符
// ---------------------------------------------------------------------------
#[test]
fn a13_metrics_segments_hide_independently() {
    // 用例 1：仅 in>0（out=0 / window=0）→ ↑/↓/缓存 有，上下文 无
    let mut state = tui::TuiState::default();
    state.total_input_tokens = 500;
    state.total_output_tokens = 0;
    state.total_cache_hit_tokens = 100;
    state.last_context_tokens = 0;
    state.context_window_tokens = 0;
    let buf = screen(&mut state, 110, 30);
    let last = row_text(&buf, 29);
    assert!(
        last.contains("↑500") && last.contains("↓0"),
        "in>0 时 ↑/↓ 段必须显示：{last:?}"
    );
    assert!(
        last.contains("缓存 20%"),
        "in>0 时缓存段必须显示（100/500=20%）：{last:?}"
    );
    assert!(
        !last.contains("上下文"),
        "window=0 时上下文段必须隐藏：{last:?}"
    );
    assert!(!last.contains('—'), "v3 起不再使用占位符：{last:?}");
    assert_no_orphan_separator(&last, "A13-1 in>0");

    // 用例 2：仅 window>0（in=0 / out=0）→ 仅「上下文」一段
    let mut state = tui::TuiState::default();
    state.total_input_tokens = 0;
    state.total_output_tokens = 0;
    state.total_cache_hit_tokens = 0;
    state.last_context_tokens = 50_000;
    state.context_window_tokens = 200_000;
    let buf = screen(&mut state, 110, 30);
    let last = row_text(&buf, 29);
    assert!(
        last.contains("上下文 25%"),
        "window>0 时上下文段必须显示（50000/200000=25%）：{last:?}"
    );
    for needle in ["↑", "↓", "缓存", "—"] {
        assert!(
            !last.contains(needle),
            "in=0/out=0 时不得出现 {needle:?}：{last:?}"
        );
    }
    assert_no_orphan_separator(&last, "A13-2 window>0");

    // 用例 3：in=0 且 out>0（window=0）→ ↑/↓ 有，缓存/上下文本无
    let mut state = tui::TuiState::default();
    state.total_input_tokens = 0;
    state.total_output_tokens = 250;
    state.total_cache_hit_tokens = 0;
    state.last_context_tokens = 0;
    state.context_window_tokens = 0;
    let buf = screen(&mut state, 110, 30);
    let last = row_text(&buf, 29);
    assert!(
        last.contains("↑0") && last.contains("↓250"),
        "out>0 时 ↑/↓ 段必须显示：{last:?}"
    );
    assert!(
        !last.contains("缓存"),
        "in=0 时缓存段必须隐藏（缓存比不可算）：{last:?}"
    );
    assert!(
        !last.contains("上下文"),
        "window=0 时上下文段必须隐藏：{last:?}"
    );
    assert!(!last.contains('—'), "v3 起不再使用占位符：{last:?}");
    assert_no_orphan_separator(&last, "A13-3 out>0");
}
