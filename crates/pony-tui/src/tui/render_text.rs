//! 消息区渲染的纯排版层（B1795831358336 Phase 2）。
//!
//! 从 `tui.rs` 整块切出：样式取样（`ink`/`muted`/`faint`）、行拼装（`push_*`/`wrap_line`）、
//! 字形常量与输入区布局计算（`layout_input`）。全部是**无状态纯函数**——不碰 `TuiState`、
//! 不做 IO，因此可在无 TTY 环境独立断言（见 `tui/tests/` 的 composer/table 用例）。
//!
//! 切出契约：函数名、签名、可见性语义逐字保留（`pub(crate)` 用于跨 `tui` 模块调用）。

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::tui::text::{char_width, is_no_line_start, line_display_width};
use crate::tui::{theme, RAIL};

/// 极简字形集：只用三种标记，避免图标噪音。
pub(crate) const GLYPH_RUNNING: &str = "◦";
pub(crate) const GLYPH_DONE: &str = "✓";
pub(crate) const GLYPH_FAIL: &str = "✕";
pub(crate) const CARET: &str = "▏";

pub(crate) fn ink() -> Style {
    Style::default().fg(theme::INK)
}

pub(crate) fn ink_soft() -> Style {
    Style::default().fg(theme::INK_SOFT)
}

pub(crate) fn muted() -> Style {
    Style::default().fg(theme::MUTED)
}

pub(crate) fn faint() -> Style {
    Style::default().fg(theme::FAINT)
}




/// 1px 发丝分隔线（不是边框，只是一条极淡的横线）。
pub(crate) fn hairline(width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "▁".repeat(width.max(1)),
        Style::default().fg(theme::HAIRLINE),
    ))
}

pub(crate) fn blank() -> Line<'static> {
    Line::raw("")
}

/// 宽松行距的块边界判定：ATX 标题行（`#`~`######` 后跟空格）。
///
/// 口径与 `markdown::render_markdown_line` 的标题分支严格一致（同一判据的两处表达，
/// 改动必须同提交对齐）：标题是段落级视觉节点，只有它在宽松行距下拿到前后各 1 行呼吸；
/// 正文相邻行一律不插行。
pub(crate) fn is_markdown_heading_line(raw: &str) -> bool {
    let trimmed = raw.trim_start();
    let hash_count = trimmed.chars().take_while(|&c| c == '#').count();
    hash_count > 0 && trimmed[hash_count..].starts_with(' ')
}

/// 按显示宽度折行，保留 span 样式；用于所有正文（CJK 不会错位）。
pub(crate) fn wrap_line(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    if line.spans.is_empty() {
        return vec![blank()];
    }
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut col = 0usize;
    for span in &line.spans {
        if span.content.is_empty() {
            continue;
        }
        let style = span.style;
        let mut buf = String::new();
        for ch in span.content.chars() {
            let w = char_width(ch);
            if col + w > width && col > 0 && !is_no_line_start(ch) {
                if !buf.is_empty() {
                    current.push(Span::styled(std::mem::take(&mut buf), style));
                }
                out.push(Line::from(std::mem::take(&mut current)));
                col = 0;
            }
            buf.push(ch);
            col += w;
        }
        if !buf.is_empty() {
            current.push(Span::styled(buf, style));
        }
    }
    if !current.is_empty() || out.is_empty() {
        out.push(Line::from(current));
    }
    out
}

/// 保证末尾恰好有 n 个空行（用于区分「段间距 1 行」与「轮间距 2 行」的节拍）。
pub(crate) fn push_gap(lines: &mut Vec<Line<'static>>, n: usize) {
    if lines.is_empty() {
        return;
    }
    let mut blanks = 0usize;
    for line in lines.iter().rev() {
        if line.spans.iter().all(|s| s.content.trim().is_empty()) {
            blanks += 1;
        } else {
            break;
        }
    }
    for _ in blanks..n {
        lines.push(blank());
    }
}

pub(crate) fn push_wrapped(lines: &mut Vec<Line<'static>>, line: Line<'static>, width: usize) {
    let mut wrapped = wrap_line(&line, width);
    // 行首禁则（跨行）：模型自带换行后若行首是标点，悬挂回上一行行尾。
    if !lines.is_empty() {
        let mut pulled: Option<(char, Style)> = None;
        if let Some(first) = wrapped.first_mut() {
            if let Some(span) = first.spans.first_mut() {
                if let Some(ch) = span.content.chars().next() {
                    if is_no_line_start(ch) {
                        span.content = span.content.chars().skip(1).collect();
                        let style = span.style;
                        pulled = Some((ch, style));
                    }
                }
            }
        }
        if let Some((ch, style)) = pulled {
            if let Some(prev) = lines.last_mut() {
                if let Some(last) = prev.spans.last_mut() {
                    let merged = format!("{}{}", last.content, ch);
                    last.content = merged.into();
                } else {
                    prev.spans.push(Span::styled(ch.to_string(), style));
                }
            }
        }
    }
    lines.extend(wrapped);
}

/// 只在不重复的前提下插入空行，保持「行间距呼吸」而不产生连续空行噪音。
pub(crate) fn push_blank(lines: &mut Vec<Line<'static>>) {
    if let Some(last) = lines.last() {
        let is_blank = last.spans.iter().all(|s| s.content.trim().is_empty());
        if !is_blank {
            lines.push(blank());
        }
    }
}


pub(crate) fn pad_line_with_bg(mut line: Line<'static>, width: usize, bg: ratatui::style::Color) -> Line<'static> {
    let used = line_display_width(&line);
    if used < width {
        line.spans.push(Span::styled(
            " ".repeat(width - used),
            Style::default().bg(bg),
        ));
    }
    line
}

/// 轨道缩进折行：首行用 first 前缀、续行用 cont 前缀，正文宽度统一。
pub(crate) fn push_prefixed(
    lines: &mut Vec<Line<'static>>,
    first: Vec<Span<'static>>,
    cont: Vec<Span<'static>>,
    body: Line<'static>,
    body_width: usize,
) {
    for (i, line) in wrap_line(&body, body_width.max(1)).into_iter().enumerate() {
        let mut spans = if i == 0 { first.clone() } else { cont.clone() };
        spans.extend(line.spans);
        lines.push(Line::from(spans));
    }
}

pub(crate) fn blank_rail(style: Style) -> Vec<Span<'static>> {
    vec![Span::styled(" ".repeat(RAIL), style)]
}






/// 用户消息：低对比表面块 + 左侧骨色竖条。用「材质」而不是文字标签区分角色。
/// 块内含 1 行上/下呼吸留白，正文回到与助手/工具相同的文本列。
pub(crate) fn push_user_block(lines: &mut Vec<Line<'static>>, content: &str, width: usize) {
    // 用户轮**不再画左侧竖线**：竖线会被终端选中复制带走，粘到剪贴板里污染正文。
    // 仍然用 `RAIL` 列缩进 + SURFACE 底色区分角色，复制出来的就是纯正文。
    let bg = theme::SURFACE;
    let fill_style = Style::default().bg(bg);
    let gutter = || Span::styled(" ".repeat(RAIL), fill_style);
    let body_w = width.saturating_sub(RAIL + 1).max(1);

    let block_pad = |lines: &mut Vec<Line<'static>>| {
        lines.push(pad_line_with_bg(Line::from(vec![gutter()]), width, bg));
    };

    block_pad(lines);
    let mut wrote_body = false;
    for raw in content.lines() {
        let body = Line::from(Span::styled(
            raw.to_string(),
            Style::default().fg(theme::INK_SOFT).bg(bg),
        ));
        for wrapped in wrap_line(&body, body_w) {
            let mut spans = vec![gutter()];
            spans.extend(
                wrapped
                    .spans
                    .into_iter()
                    .map(|s| Span::styled(s.content, s.style.bg(bg))),
            );
            lines.push(pad_line_with_bg(Line::from(spans), width, bg));
            wrote_body = true;
        }
    }
    if !wrote_body {
        let mut spans = vec![gutter()];
        spans.push(Span::styled(String::new(), Style::default().bg(bg)));
        lines.push(pad_line_with_bg(Line::from(spans), width, bg));
    }
    block_pad(lines);
}

/// 工具行分层：动作名保留可读性，参数/结果退到次级墨色。
pub(crate) fn tool_line_spans(
    glyph: &str,
    glyph_style: Style,
    head_style: Style,
    content: &str,
) -> (Vec<Span<'static>>, Vec<Span<'static>>) {
    // 工具描述常带行内 markdown 记号；工具行不是富文本，直接去掉反引号/加粗星号。
    let cleaned = content.replace('`', "").replace("**", "");
    let (head, detail) = cleaned
        .split_once(" · ")
        .or_else(|| content.split_once("  "))
        .map(|(h, d)| (h.to_string(), d.to_string()))
        .unwrap_or_else(|| (content.to_string(), String::new()));
    let prefix = vec![Span::styled(format!("{} ", glyph), glyph_style)];
    let mut body = vec![Span::styled(head, head_style)];
    if !detail.is_empty() {
        // 参数/结果属于中间过程，压到最暗一档。
        body.push(Span::styled(format!(" · {}", detail), faint()));
    }
    (prefix, body)
}

/// 输入框视觉折行：按显示宽度硬折行 + 尊重显式换行符，返回字符区间。
pub(crate) fn layout_input(text: &str, width: usize) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let width = width.max(1);
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    let mut col = 0usize;
    for (i, ch) in chars.iter().enumerate() {
        if *ch == '\n' {
            ranges.push((start, i));
            start = i + 1;
            col = 0;
            continue;
        }
        let w = char_width(*ch);
        if col + w > width && col > 0 {
            ranges.push((start, i));
            start = i;
            col = 0;
        }
        col += w;
    }
    ranges.push((start, chars.len()));
    ranges
}
