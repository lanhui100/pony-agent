//! 批6（task-15）验收测试：按消息粒度布局缓存 / 视口虚拟化 / 惰性全量文本 / 选区。
//!
//! 契约真值源：`.dev-team/contract-matrix-slash-commands.md` 修订11（a）~（e）（Lead 批准·批6）。
//! 沿用 `tests/tui_render_layout.rs` 的 bin-as-module + `TestBackend` harness：`src/bin/tui.rs` 是
//! `[[bin]]` 目标，只能引入**同一份真实源码**才能在编译期断言其行为。
//!
//! ## 证据口径（如实声明）
//! - **R1 逐格一致**：参考路径 = `tui::message_lines_for(msg, width)` 逐条拼接（同一份生产布局函数，
//!   契约 (d) 要求参考路径与缓存路径共用它），被测路径 = `ui()` 真实渲染进 `TestBackend` 的
//!   chat 区域（由 `state.last_chat_rect` 精确定位）。逐行比较**去掉行尾补白**后的整行文本；
//!   行尾空白是 ratatui 把整行填满宽度产生的填充，不属于"渲染内容"。宽字符（CJK）在缓冲里占两格、
//!   其后是一格填充，故 harness 用最小宽度表重建行文本（否则会假红成「帮 我 检 查」）。
//!   另有唯一一处允许的差异：双宽字符正好卡在正文区右边界时 ratatui 整字省略（不拆格），
//!   `assert_row_matches` 把该例外限定为「恰好缺一个宽字符且行宽+2 超过可用列宽」。可见窗口以外的参考行若多于
//!   可见行，只比对可见部分（其余行由 R5 的滚动窗口用例覆盖）。
//! - **R4 上界**：`laid_out_lines` 按 Lead 定义取「本帧真正做布局的行数」，上界用
//!   `可见高度 + max(单条消息布局行数)`——后者由 `message_lines_for` 实测得出，而不是硬编码常数；
//!   若实现把 `total_lines` 也计入 `laid_out_lines`，`laid_out_lines < total_lines` 会立刻失败，
//!   此时以实现语义为准并在本注释登记（不在实现未落地前假定）。
//! - **R6 选区**：`MouseSelectionState` / `TextCoord` 的字段都是 pub（已核对），因此不退化：
//!   完整断言「`ensure_full_text()` 重建的全量文本 == 缓存统计的 total_lines」+「跨行选区提取出的
//!   文本包含被选中的哨兵内容」。
//!
//! 铁律：真实断言、无 `#[ignore]`/skip；不修改实现代码；不触碰真实 `~/.pony-agent`。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use ratatui::backend::TestBackend;
use ratatui::Terminal;

// ── harness ────────────────────────────────────────────────────────────────────

fn screen(state: &mut tui::TuiState, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("create test backend");
    terminal
        .draw(|frame| tui::ui(frame, state))
        .expect("draw frame");
    terminal.backend().buffer().clone()
}

/// 宽字符占位宽度：镜像 unicode-width 的 East Asian Wide 规则（仅覆盖 fixture 里出现的字符集：
/// CJK 统一表意文字、CJK 标点、全角标点、制表/块元素为窄）。测试不能直接用 unicode-width crate
/// （非本 crate 的直接依赖），故这里给出一张最小宽度表，只用于**重建**终端视觉行文本。
fn char_width(ch: char) -> usize {
    match ch as u32 {
        0x1100..=0x115F
        | 0x2E80..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

/// 把一行缓冲单元重建为字符串：宽字符占两格，其后续填充格必须跳过，否则 CJK 行会变成
/// “帮 我 检 查 …”（每字之间多一个空格），逐格比对会假红。
fn row_cells_text(buf: &ratatui::buffer::Buffer, rect: ratatui::layout::Rect, y: u16) -> String {
    let mut out = String::new();
    let mut x = rect.x;
    while x < rect.x + rect.width {
        let symbol = buf[(x, y)].symbol();
        let mut chars = symbol.chars();
        match chars.next() {
            Some(ch) => {
                out.push(ch);
                // 多字符 grapheme 视为宽度 1（本 fixture 不出现组合字符）。
                let width = if symbol.chars().count() > 1 { 1 } else { char_width(ch) };
                x = x.saturating_add(width.max(1) as u16);
            }
            None => {
                // 空 symbol（被宽字符覆盖的填充格）⇒ 前进一格。
                x = x.saturating_add(1);
            }
        }
    }
    out
}

/// chat 正文区的可见行文本（按 `last_chat_rect` 精确切片，包含左侧轨道/缩进，不做任何裁剪）。
fn chat_rows(state: &tui::TuiState, buf: &ratatui::buffer::Buffer) -> Vec<String> {
    let rect = state
        .last_chat_rect
        .expect("ui() 必须记录 last_chat_rect（渲染 harness 依赖它定位正文区）");
    (rect.y..rect.y + rect.height)
        .map(|y| row_cells_text(buf, rect, y))
        .collect()
}

fn line_text(line: &ratatui::text::Line<'static>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>()
}

/// 逐行比较：正常情形要求整行文本完全一致；唯一允许的差异是**宽字符正好卡在右边界**被裁掉
/// （ratatui 不会把双宽字符拆成两格：末格放不下时整字省略）。该例外被严格限定：
/// 缺的必须恰好是一个宽字符，且实际行宽 + 2 必须超过可用列宽——否则视为渲染错位。
fn assert_row_matches(label: &str, index: usize, actual: &str, expected: &str, available_cells: usize) {
    let actual_text = actual.trim_end();
    let expected_text = expected.trim_end();
    if actual_text == expected_text {
        return;
    }
    if let Some(suffix) = expected_text.strip_prefix(actual_text) {
        let mut chars = suffix.chars();
        if let Some(clipped) = chars.next() {
            assert!(
                chars.next().is_none(),
                "{label}: 第 {index} 行缺少多个字符（不是宽字符边界裁剪）：actual={actual_text:?} expected={expected_text:?}"
            );
            assert_eq!(
                char_width(clipped),
                2,
                "{label}: 第 {index} 行缺少的字符必须是宽字符（双宽字符边界裁剪）才允许，实际缺的是 {clipped:?}"
            );
            let actual_width: usize = actual_text.chars().map(char_width).sum();
            assert!(
                actual_width + char_width(clipped) > available_cells,
                "{label}: 第 {index} 行末尾宽字符被裁掉，但行宽 {actual_width}+2 未超过可用列宽 {available_cells}（说明是丢内容而非边界裁剪）"
            );
            return;
        }
    }
    panic!("{label}: 第 {index} 行与参考布局不一致（逐格比对失败）：actual={actual_text:?} expected={expected_text:?}");
}

/// 参考路径：逐条消息调用生产布局函数并拼接（不复制任何排版逻辑到测试里）。
fn reference_lines(messages: &[tui::ChatMessage], width: usize) -> Vec<String> {
    messages
        .iter()
        .flat_map(|message| {
            tui::message_lines_for(message, width)
                .into_iter()
                .map(|line| line_text(&line))
        })
        .collect()
}

fn message(role: tui::MessageRole, content: &str) -> tui::ChatMessage {
    tui::ChatMessage {
        role,
        content: content.to_string(),
        timestamp_ms: 1_700_000_000_000,
    }
}

/// R1/R3 固定 fixture：长 markdown + CJK 超长行 + 代码块（三种排版压力各一条）。
fn fixture_messages() -> Vec<tui::ChatMessage> {
    vec![
        message(
            tui::MessageRole::User,
            "帮我检查这段中文超长行在窄终端里的折行是否稳定：这是一个刻意写得很长的中文句子，\
用来验证按显示宽度折行的实现不会在中文字符中间劈开，也不会把标点留在行首导致悬挂。",
        ),
        message(
            tui::MessageRole::Assistant,
            "## 结论\n\n- 第一点：折行按显示宽度进行\n- 第二点：CJK 字符宽度为 2\n- 第三点：标点悬挂回上一行\n\n接下来是代码：\n\n```rust\nfn main() {\n    println!(\"hello world from a fairly long line of code\");\n}\n```\n\n以上。",
        ),
        message(
            tui::MessageRole::Assistant,
            "补充一段说明：缓存按消息粒度复用，只有最后一条消息在流式期间需要重算布局。",
        ),
    ]
}

/// 只保留非空白字符，用于「换行位置是否变化」这类结构性比较。
fn squeezed(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

// ── R1：虚拟化后与参考布局逐格一致（修订11(e)） ─────────────────────────────────

#[test]
fn r1_visible_rows_match_reference_layout_cell_for_cell() {
    let mut state = tui::TuiState::default();
    // 关掉贴底阻尼（否则每帧会按步长推进 scroll_offset，固定 scroll=0 的前提被破坏）。
    state.auto_scroll = false;
    state.scroll_offset = 0;
    state.messages = fixture_messages();

    let buf = screen(&mut state, 80, 40);
    let rect = state.last_chat_rect.expect("last_chat_rect");
    let width = rect.width as usize;
    let expected = reference_lines(&state.messages, width);
    let rows = chat_rows(&state, &buf);

    assert!(
        !expected.is_empty(),
        "R1 前置条件：参考布局不得为空（否则逐格比对没有意义）"
    );
    assert_eq!(
        rows.len(),
        rect.height as usize,
        "R1: chat 可见行数必须等于 last_chat_rect.height"
    );

    let mut compared = 0usize;
    for (index, row) in rows.iter().enumerate() {
        match expected.get(index) {
            Some(expected_row) => {
                assert_row_matches("R1", index, row, expected_row, width);
                compared += 1;
            }
            None => {
                assert!(
                    row.trim().is_empty(),
                    "R1: 参考布局已结束但第 {index} 行仍有内容（多渲染了行）：{row:?}"
                );
            }
        }
    }
    assert!(
        compared >= 10,
        "R1: 至少应有 10 行参与逐格比对（fixture 足够长以覆盖可见区），实际 {compared}"
    );
}

// ── R2：按消息粒度的缓存命中（修订11(a)） ──────────────────────────────────────

#[test]
fn r2_second_frame_reuses_cache_and_only_rebuilds_last_message() {
    let mut state = tui::TuiState::default();
    state.auto_scroll = false;
    state.messages = fixture_messages();
    let total_messages = state.messages.len();
    assert!(
        total_messages >= 3,
        "R2 前置条件：至少 3 条消息才能区分「全量重建」与「仅末条重算」"
    );

    // 第一帧：缓存为空 ⇒ 全量构建。
    let _ = screen(&mut state, 80, 40);
    let first = state.render_cache_stats();
    assert_eq!(
        first.rebuilt_messages, total_messages,
        "R2: 首帧必须全量构建（缓存为空）"
    );
    assert_eq!(first.reused_messages, 0, "R2: 首帧复用数必须为 0");
    assert!(first.total_lines > 0, "R2: 总行数必须为正");

    // 第二帧：宽度与消息数都没变 ⇒ 仅最后一条重算，其余复用。
    let _ = screen(&mut state, 80, 40);
    let second = state.render_cache_stats();
    assert!(
        second.rebuilt_messages <= 1,
        "R2: 第二帧重建条数必须 <= 1（仅末条），实际 {}",
        second.rebuilt_messages
    );
    assert_eq!(
        second.reused_messages,
        total_messages - 1,
        "R2: 第二帧必须复用其余 n-1 条消息的布局"
    );
    assert_eq!(
        second.total_lines, first.total_lines,
        "R2: 同宽度同消息数下总行数必须稳定（缓存不得改变布局结果）"
    );
}

// ── R3：宽度变化触发全量重建（修订11(a)） ──────────────────────────────────────

#[test]
fn r3_width_change_invalidates_whole_cache_and_rewraps_lines() {
    let mut state = tui::TuiState::default();
    state.auto_scroll = false;
    state.messages = fixture_messages();
    let total_messages = state.messages.len();

    let buf80 = screen(&mut state, 80, 40);
    let text80 = chat_rows(&state, &buf80).join("\n");
    assert!(
        !squeezed(&text80).is_empty(),
        "R3 前置条件：80 列必须渲染出内容"
    );

    let buf60 = screen(&mut state, 60, 40);
    let stats60 = state.render_cache_stats();
    let text60 = chat_rows(&state, &buf60).join("\n");

    assert_eq!(
        stats60.rebuilt_messages, total_messages,
        "R3: 宽度变化必须全量重建全部 {total_messages} 条消息的布局"
    );
    assert_eq!(stats60.reused_messages, 0, "R3: 宽度变化后不得复用旧宽度缓存");
    assert_ne!(
        squeezed(&text80),
        squeezed(&text60),
        "R3: 80 列与 60 列的折行位置必须不同（否则说明宽度变化没有真正重排）"
    );
}

// ── R4：视口虚拟化的布局上界（修订11(b)） ──────────────────────────────────────

#[test]
fn r4_layout_work_is_bounded_by_viewport_and_single_message() {
    let mut state = tui::TuiState::default();
    state.auto_scroll = false;
    state.messages = (0..500)
        .map(|index| {
            message(
                if index % 2 == 0 {
                    tui::MessageRole::User
                } else {
                    tui::MessageRole::Assistant
                },
                &format!("虚拟化压力消息 #{index}：内容长度适中，用于制造大量视觉行。"),
            )
        })
        .collect();
    assert_eq!(state.messages.len(), 500, "R4 前置条件：500 条消息");

    // 第一帧拿到 max_scroll_offset，再滚到中部。
    let _ = screen(&mut state, 100, 30);
    let max_offset = state.max_scroll_offset;
    assert!(max_offset > 0, "R4 前置条件：500 条消息必须可滚动");
    state.scroll_offset = max_offset / 2;

    let _ = screen(&mut state, 100, 30);
    let stats = state.render_cache_stats();
    let rect = state.last_chat_rect.expect("last_chat_rect");
    let visible = rect.height as usize;

    // 单条消息布局行数上界：由生产布局函数实测，不硬编码。
    let width = rect.width as usize;
    let max_single_message_lines = state
        .messages
        .iter()
        .map(|message| tui::message_lines_for(message, width).len())
        .max()
        .unwrap_or(0);
    assert!(
        max_single_message_lines > 0,
        "R4 前置条件：单条消息布局行数必须为正"
    );

    assert!(
        stats.laid_out_lines <= visible + max_single_message_lines,
        "R4: 本帧布局行数必须 <= 可见高度 {visible} + 最高单条消息行数 {max_single_message_lines}，实际 {}",
        stats.laid_out_lines
    );
    assert!(
        stats.laid_out_lines < stats.total_lines,
        "R4: 虚拟化后布局行数必须严格小于总行数（{} vs {}）",
        stats.laid_out_lines,
        stats.total_lines
    );
    assert!(
        stats.total_lines >= 500,
        "R4: 500 条消息的总视觉行数必须 >= 500，实际 {}",
        stats.total_lines
    );

    // max_scroll_offset 语义不变：总行数 − 可见高度。
    assert_eq!(
        state.max_scroll_offset,
        (stats.total_lines - visible) as u32,
        "R4: max_scroll_offset 必须仍等于 total_lines - 可见高度（语义零变更）"
    );
}

// ── R5：滚动边界（0 / max / max+100 钳制） ─────────────────────────────────────

#[test]
fn r5_scroll_boundaries_render_correct_window_without_panic() {
    let mut state = tui::TuiState::default();
    state.auto_scroll = false;
    state.messages = (0..200)
        .map(|index| {
            message(
                tui::MessageRole::User,
                &format!("滚动边界消息 #{index}：用于验证 offset 窗口推导。"),
            )
        })
        .collect();

    state.scroll_offset = 0;
    let buf = screen(&mut state, 100, 30);
    let max_offset = state.max_scroll_offset;
    let rect = state.last_chat_rect.expect("last_chat_rect");
    let visible = rect.height as usize;
    assert!(max_offset > 0, "R5 前置条件：内容必须可滚动");

    // offset = 0：首行必须是从全量文本第 0 行开始的窗口。
    state.ensure_full_text();
    let full = state.rendered_lines_text.clone();
    assert!(
        full.len() as u32 >= max_offset + visible as u32,
        "R5 前置条件：全量文本长度必须 >= max_offset + 可见高度"
    );
    let rows = chat_rows(&state, &buf);
    let layout_width = rect.width as usize;
    for index in 0..visible {
        assert_row_matches("R5(offset=0)", index, &rows[index], &full[index], layout_width);
    }

    // offset = max：滚到底部，首行必须是 full[max]。
    state.scroll_offset = max_offset;
    let buf_max = screen(&mut state, 100, 30);
    assert_eq!(
        state.scroll_offset, max_offset,
        "R5: offset=max 时必须保持 max（不得被阻尼改动）"
    );
    state.ensure_full_text();
    let full_max = state.rendered_lines_text.clone();
    let rows_max = chat_rows(&state, &buf_max);
    for index in 0..visible {
        assert_row_matches(
            "R5(offset=max)",
            index,
            &rows_max[index],
            &full_max[max_offset as usize + index],
            layout_width,
        );
    }

    // offset = max + 100：必须钳制到 max（不得 panic、不得渲染空窗口）。
    state.scroll_offset = max_offset + 100;
    let buf_clamped = screen(&mut state, 100, 30);
    assert_eq!(
        state.scroll_offset, max_offset,
        "R5: 越界 offset 必须钳制到 max_scroll_offset"
    );
    state.ensure_full_text();
    let full_clamped = state.rendered_lines_text.clone();
    let rows_clamped = chat_rows(&state, &buf_clamped);
    for index in 0..visible {
        assert_row_matches(
            "R5(offset clamped)",
            index,
            &rows_clamped[index],
            &full_clamped[max_offset as usize + index],
            layout_width,
        );
    }
}

// ── R6：惰性全量文本不破坏鼠标框选/复制（修订11(c)） ─────────────────────────────

#[test]
fn r6_lazy_full_text_keeps_selection_extraction_intact() {
    let mut state = tui::TuiState::default();
    state.auto_scroll = false;
    state.messages = vec![
        message(tui::MessageRole::User, "SENTINEL_ONE 选区测试第一行"),
        message(tui::MessageRole::Assistant, "SENTINEL_TWO 选区测试第二行"),
        message(tui::MessageRole::Assistant, "SENTINEL_THREE 选区测试第三行"),
    ];

    let _ = screen(&mut state, 100, 30);
    let stats = state.render_cache_stats();
    assert!(
        stats.total_lines > 0,
        "R6 前置条件：必须有可见布局行"
    );

    // 惰性重建：虚拟化后 rendered_lines_text 不再每帧重建，必须按需恢复为**全量**文本。
    state.ensure_full_text();
    assert_eq!(
        state.rendered_lines_text.len(),
        stats.total_lines,
        "R6: ensure_full_text() 后 rendered_lines_text 长度必须等于缓存统计的 total_lines（少一行都会让框选漏字）"
    );

    // 构造跨行选区（锚点在第 1 行行首，头在最后一行）。
    let last_line = state.rendered_lines_text.len().saturating_sub(1);
    state.mouse_selection = tui::MouseSelectionState {
        active: true,
        anchor: Some(tui::TextCoord {
            line: 0,
            col: 0,
        }),
        head: Some(tui::TextCoord {
            line: last_line,
            col: 4096, // 远超任何可见行宽度 ⇒ 选区到底
        }),
        debounce_until: None,
        copied_range: None,
    };

    let (anchor, head) = state
        .mouse_selection
        .normalized_range()
        .expect("R6: 选区已设置，normalized_range 必须返回区间");
    let selected = tui::extract_selected_text(&state.rendered_lines_text, anchor, head);

    assert!(
        !selected.trim().is_empty(),
        "R6: 跨行选区提取结果不得为空（惰性重建不得破坏复制）"
    );
    for sentinel in ["SENTINEL_ONE", "SENTINEL_TWO", "SENTINEL_THREE"] {
        assert!(
            selected.contains(sentinel),
            "R6: 全量选区必须包含 {sentinel}，实际提取 = {selected:?}"
        );
    }
    assert!(
        selected.contains('\n'),
        "R6: 跨行选区提取结果必须保留换行（否则复制出来会粘成一行）"
    );
}
