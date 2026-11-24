//! Ask（`ask_user`）内联问答面板：行口径、选项折行、多选标记与渲染（原 `tui.rs` 内联实现，机械外移）。
//!
//! 一处口径：面板高度（[`inline_ask_height`]）与渲染（[`render_ask_inline`]）共用
//! [`ask_option_rows`] / [`ask_custom_input_label`] / [`ask_key_hint`]，行数不会分叉。

use super::*;

/// Ask 面板的呼吸节奏：2 帧交替（每帧约 450ms），只在问答等待期生效。
pub fn ask_pulse_glyph(tick: usize) -> &'static str {
    if tick % 2 == 0 {
        "◆"
    } else {
        "◇"
    }
}

/// 内联 Ask 面板高度：分隔线 + 标题 + 选项(含自定义输入) + 键位提示；无问答时为 0。
/// Ask 面板问题正文换行：首行 `first_width`、续行 `rest_width`（CJK 宽度感知）。
///
/// 与 `wrap_line` 同一套行首避让规则（标点不落行首）；**绝不**用省略号截断 ——
/// 问题必须完整可读。空问题返回单条空串，保证调用方至少有一行可渲染。
pub fn wrap_ask_question(question: &str, first_width: usize, rest_width: usize) -> Vec<String> {
    let first_limit = first_width.max(1);
    let rest_limit = rest_width.max(1);
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut col = 0usize;
    let mut limit = first_limit;
    for ch in question.chars() {
        let w = char_width(ch);
        if col + w > limit && col > 0 && !is_no_line_start(ch) {
            out.push(std::mem::take(&mut current));
            col = 0;
            limit = rest_limit;
        }
        current.push(ch);
        col += w;
    }
    out.push(current);
    out
}

/// Ask 面板选项行的固定前缀宽度：` 轨标 + 标记 + 空格` = 5 列。
/// 折行后的续行按同一缩进对齐，保证文字始终落在标签列、绝不越过面板右缘。
pub const ASK_OPTION_INDENT: usize = 5;

/// Ask 面板右侧留白：正文不与面板右缘贴边（与左侧 RAIL 一起构成对称呼吸）。
pub const ASK_RIGHT_GUTTER: usize = 1;

/// 面向用户的分组标签：运行时内部 id（`builtin:ask_user`）不得上屏。
pub fn ask_display_header(question: &AskUserQuestion) -> Option<String> {
    question
        .header
        .as_ref()
        .map(|header| header.trim())
        .filter(|header| !header.is_empty())
        .filter(|header| !header.contains(':') && !header.starts_with("builtin"))
        .map(str::to_string)
}

/// 选项标记：多选用勾选框（已勾 `▣` / 未勾 `▢`），单选用圆点（焦点 `●` / 其余 `○`）。
pub fn ask_option_marker(question: &AskUserQuestion, ask: &AskUserState, index: usize) -> &'static str {
    if question.multi_select {
        if ask.multi_selected_indices.contains(&index) {
            "▣"
        } else {
            "▢"
        }
    } else if index == ask.selected_option_index {
        "●"
    } else {
        "○"
    }
}

/// 「自定义输入」行的文本（焦点态带呼吸光标）。
///
/// 高度计算与渲染**共用**本函数：光标是一个 1 列字符，各自算一次会让「预留高度」与实际
/// 渲染行数在边界宽度上分叉（面板被裁掉最后一行）。
pub fn ask_custom_input_label(ask: &AskUserState, focused: bool, tick: usize) -> String {
    let caret = if focused && tick % 2 == 0 { CARET } else { "" };
    if ask.custom_input.is_empty() {
        if focused {
            format!("自定义输入{caret}")
        } else {
            "自定义输入…".to_string()
        }
    } else if focused {
        format!("{}{caret}", ask.custom_input)
    } else {
        ask.custom_input.clone()
    }
}

/// Ask 面板的键位提示：按当前状态给最短的一组（esc = 忽略此提问，不是「过期」）。
pub fn ask_key_hint(ask: &AskUserState) -> &'static str {
    let has_options = ask
        .current_question()
        .map(|question| !question.options.is_empty())
        .unwrap_or(false);
    let multi = ask
        .current_question()
        .map(|question| question.multi_select)
        .unwrap_or(false);
    if !has_options {
        "输入回答 · ↵ 提交 · esc 忽略此提问"
    } else if ask.is_custom_input_active {
        "输入自定义回答 · ↵ 提交 · esc 忽略此提问"
    } else if multi {
        "↑↓ 移动 · space 勾选 · ↵ 提交 · esc 忽略此提问"
    } else {
        "↑↓ 移动 · ↵ 确认 · esc 忽略此提问"
    }
}

/// 一条选项渲染成**若干行**：长标签 / 长说明按显示宽度折行，绝不省略、绝不越过面板右缘。
///
/// 行结构 ` 轨标 标记 标签  说明`；续行按 [`ASK_OPTION_INDENT`] 缩进对齐标签列，焦点行整块
/// `SURFACE_SEL` 铺底（含续行），让「当前焦点」在折行后仍是一个完整色块而不是半行。
pub fn ask_option_rows(
    focused: bool,
    marker: &str,
    label: &str,
    description: &str,
    width: usize,
) -> Vec<Line<'static>> {
    let bg = if focused {
        theme::SURFACE_SEL
    } else {
        theme::CANVAS
    };
    let bar_style = if focused {
        Style::default().fg(theme::ACCENT).bg(bg)
    } else {
        Style::default().bg(bg)
    };
    let label_style = if focused {
        Style::default()
            .fg(theme::INK)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::INK_SOFT).bg(bg)
    };
    let marker_style = if focused {
        Style::default().fg(theme::ACCENT).bg(bg)
    } else {
        Style::default().fg(theme::MUTED).bg(bg)
    };
    let desc_style = if focused {
        Style::default().fg(theme::INK_SOFT).bg(bg)
    } else {
        Style::default().fg(theme::MUTED).bg(bg)
    };

    let width = width.max(ASK_OPTION_INDENT + 4);
    let text_width = width
        .saturating_sub(ASK_OPTION_INDENT + ASK_RIGHT_GUTTER)
        .max(4);
    let mut content: Vec<Span<'static>> = vec![Span::styled(label.to_string(), label_style)];
    if !description.trim().is_empty() {
        content.push(Span::styled("  ", Style::default().bg(bg)));
        content.push(Span::styled(description.trim().to_string(), desc_style));
    }

    let mut rows: Vec<Line<'static>> = Vec::new();
    for (index, line) in wrap_line(&Line::from(content), text_width)
        .into_iter()
        .enumerate()
    {
        let mut spans: Vec<Span<'static>> = Vec::with_capacity(line.spans.len() + 4);
        if index == 0 {
            spans.push(Span::raw(" "));
            spans.push(Span::styled(if focused { "▌" } else { " " }, bar_style));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(format!("{marker} "), marker_style));
        } else {
            spans.push(Span::styled(
                " ".repeat(ASK_OPTION_INDENT),
                Style::default().bg(bg),
            ));
        }
        let used = ASK_OPTION_INDENT + line_display_width(&line);
        spans.extend(line.spans);
        let pad = width.saturating_sub(used);
        if pad > 0 {
            spans.push(Span::styled(" ".repeat(pad), Style::default().bg(bg)));
        }
        rows.push(Line::from(spans));
    }
    rows
}

/// 内联 Ask 面板高度（渲染与几何计算的**唯一**口径）。
///
/// 结构自上而下：发丝线 1 + 上内边距 1 + 标题 1 + 问题正文(≥1) + 问答间距 1 +
/// 选项行(可折行) + 键位提示 1 + 下内边距 1。上限只防极端长文本吞掉整屏：`ui()` 仍会按
/// 消息区几何二次收缩（不得越过 header / composer）。
pub fn inline_ask_height(state: &TuiState, width: usize) -> u16 {
    if state.mode != InteractionMode::AskUserQuestion {
        return 0;
    }
    let Some(ask) = &state.ask_user_state else {
        return 0;
    };
    let width = width.max(1);
    let prompt = ask
        .current_question()
        .map(|q| q.question.trim().replace('\n', " "))
        .unwrap_or_default();
    let question_width = width.saturating_sub(RAIL + ASK_RIGHT_GUTTER).max(8);
    let question_rows = wrap_ask_question(&prompt, question_width, question_width)
        .len()
        .max(1);
    let option_rows = ask
        .current_question()
        .map(|question| {
            let mut rows = 0usize;
            for (index, option) in question.options.iter().enumerate() {
                let focused = index == ask.selected_option_index;
                rows += ask_option_rows(
                    focused,
                    ask_option_marker(question, ask, index),
                    &option.label,
                    option.description.as_deref().unwrap_or(""),
                    width,
                )
                .len();
            }
            let focused = ask.selected_option_index == question.options.len();
            let label = ask_custom_input_label(ask, focused, state.spinner_tick);
            let description = if focused { "↵ 提交" } else { "" };
            rows + ask_option_rows(focused, ">", &label, description, width).len()
        })
        .unwrap_or(0);
    let needed = 6 + question_rows + option_rows;
    let cap = if width >= 60 { 40 } else { 24 };
    (needed as u16).min(cap).max(4)
}

/// 信息流底部的内联问答面板（无边框、无遮罩，与正文同一 CANVAS 底色）。
///
/// 版式（每块之间都有呼吸行，不再把所有东西挤成一段）：
///
/// ```text
/// ▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁▁
///
///   ◆ 提问  验收分支  1/2
///   请选择本次交付的执行架构…
///
///   ▌ ▣ 分支 A  先做冒烟
///     ▢ 分支 B  直接全量（长文本会折行，续行与标签列对齐）
///   ▌ > 自定义输入…
///   ↑↓ 移动 · space 勾选 · ↵ 提交 · esc 忽略此提问
///
/// ```
pub fn render_ask_inline(f: &mut Frame, area: Rect, state: &TuiState) {
    let Some(ask) = &state.ask_user_state else {
        return;
    };
    fill(f, area, theme::CANVAS);
    let width = area.width as usize;
    let budget = area.height as usize;
    let question_style = Style::default()
        .fg(theme::TERRACOTTA_BRIGHT)
        .add_modifier(Modifier::BOLD);

    // ── head：发丝线 + 上内边距 + 标题 + 问题正文 + 问答间距 ──────────────────
    let mut head: Vec<Line<'static>> = Vec::new();
    if budget >= 2 {
        head.push(hairline(width));
    }
    head.push(blank());

    let pulse = ask_pulse_glyph(state.spinner_tick);
    let mut title_spans: Vec<Span<'static>> = vec![
        Span::raw(" ".repeat(RAIL)),
        Span::styled(
            pulse,
            Style::default()
                .fg(theme::TERRACOTTA_BRIGHT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            "提问",
            Style::default()
                .fg(theme::TERRACOTTA_BRIGHT)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(header) = ask.current_question().and_then(ask_display_header) {
        title_spans.push(Span::raw("  "));
        title_spans.push(Span::styled(header, muted()));
    }
    let total = ask.questions.len();
    if ask.current_question().is_some() && total > 1 {
        title_spans.push(Span::raw("  "));
        title_spans.push(Span::styled(
            format!("{}/{}", ask.current_question_index + 1, total),
            faint(),
        ));
    }
    head.push(Line::from(title_spans));

    // 问题正文**独占整块**：与「提问」chip 分行的原因是长问题折行后仍要能按段落阅读，
    // 挤在标题行里会让每一行都变成「chip + 半句问题」的碎读。
    let question = ask
        .current_question()
        .map(|q| q.question.trim().replace('\n', " "))
        .unwrap_or_default();
    let question_indent = " ".repeat(RAIL);
    let question_width = width.saturating_sub(RAIL + ASK_RIGHT_GUTTER).max(8);
    if question.is_empty() {
        head.push(Line::from(vec![
            Span::raw(question_indent),
            Span::styled("无等待确认的问题", faint()),
        ]));
    } else {
        for segment in wrap_ask_question(&question, question_width, question_width) {
            head.push(Line::from(vec![
                Span::raw(question_indent.clone()),
                Span::styled(segment, question_style),
            ]));
        }
    }
    // 问题与选项之间的呼吸行：提问与可点选项不再贴在一起。
    head.push(blank());

    // ── body：选项 + 自定义输入（长文本折行，焦点行整块高亮）──────────────────
    let mut body: Vec<Line<'static>> = Vec::new();
    if let Some(question) = ask.current_question() {
        for (index, option) in question.options.iter().enumerate() {
            let focused = index == ask.selected_option_index;
            body.extend(ask_option_rows(
                focused,
                ask_option_marker(question, ask, index),
                &option.label,
                option.description.as_deref().unwrap_or(""),
                width,
            ));
        }
        let focused = ask.selected_option_index == question.options.len();
        let label = ask_custom_input_label(ask, focused, state.spinner_tick);
        let description = if focused { "↵ 提交" } else { "" };
        body.extend(ask_option_rows(focused, ">", &label, description, width));
    }

    // ── tail：键位提示 + 下内边距 ────────────────────────────────────────────
    let mut tail: Vec<Line<'static>> = vec![
        Line::from(vec![
            Span::raw(" ".repeat(RAIL + 1)),
            Span::styled(ask_key_hint(ask), faint()),
        ]),
        blank(),
    ];

    // 空间不足时**先丢选项尾部**：问题与键位提示（含 esc 出口）优先保留。
    let mut lines = head;
    let fixed = lines.len() + tail.len();
    if budget > fixed {
        lines.extend(body.into_iter().take(budget - fixed));
    }
    lines.append(&mut tail);
    if lines.len() > budget {
        lines.truncate(budget);
    }

    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::CANVAS)),
        area,
    );
}
