//! GFM 管道表格块级渲染（列宽拟合 / 折行 / 对齐）

use super::*;

/// 单列宽度下限：再窄的列已无可读性，宁可整体退回原样逐行渲染。
const MIN_COL_WIDTH: usize = 4;

/// 列间固定 2 空格。绝不画 `|`：终端选中复制出来必须是干净文本。
const COL_GAP: usize = 2;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Align {
    Left,
    Center,
    Right,
}

/// 按未转义 `|` 拆分单元格：外覆的管道符丢弃，`\|` 还原为字面量 `|`，两端留白修剪。
fn split_cells(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let body = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let mut cells: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut chars = body.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => match chars.next() {
                Some('|') => cur.push('|'),
                Some(other) => {
                    cur.push('\\');
                    cur.push(other);
                }
                None => cur.push('\\'),
            },
            '|' => cells.push(std::mem::take(&mut cur)),
            _ => cur.push(ch),
        }
    }
    // 行尾 `|` 之后剩下的空尾巴是收尾符号，不是一列。
    if !cur.is_empty() || !trimmed.ends_with('|') {
        cells.push(cur);
    }
    cells.into_iter().map(|c| c.trim().to_string()).collect()
}

/// 分隔行单元格判定：`:?-+:?` —— 去两端各一个冒号后必须全为 `-` 且至少 1 个。
fn is_separator_cell(cell: &str) -> bool {
    let trimmed = cell.trim();
    let trimmed = trimmed.strip_prefix(':').unwrap_or(trimmed);
    let trimmed = trimmed.strip_suffix(':').unwrap_or(trimmed);
    !trimmed.is_empty() && trimmed.chars().all(|c| c == '-')
}

fn align_of(cell: &str) -> Align {
    let trimmed = cell.trim();
    let left = trimmed.starts_with(':');
    let right = trimmed.len() > 1 && trimmed.ends_with(':');
    match (left, right) {
        (true, true) => Align::Center,
        (false, true) => Align::Right,
        _ => Align::Left,
    }
}

/// 去掉行内标记后的纯文本，仅用于测宽（渲染仍走 `parse_inline_formatting`）。
fn plain_text(cell: &str) -> String {
    cell.replace("**", "").replace('`', "")
}

/// 一段已识别的表格块：表头、各列对齐、数据行、以及它吃掉的源代码行数。
struct TableBlock {
    header: Vec<String>,
    aligns: Vec<Align>,
    rows: Vec<Vec<String>>,
    consumed: usize,
}

/// 识别并收集表格块；任何一条 GFM 形态不满足就返回 `None`（调用方按现状逐行渲染）。
fn parse_block(lines: &[&str]) -> Option<TableBlock> {
    if lines.len() < 2 {
        return None;
    }
    let header = split_cells(lines[0]);
    if header.len() < 2 {
        return None;
    }
    let sep = split_cells(lines[1]);
    if sep.len() != header.len() || !sep.iter().all(|cell| is_separator_cell(cell)) {
        return None;
    }
    let cols = header.len();
    let aligns: Vec<Align> = sep.iter().map(|cell| align_of(cell)).collect();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut consumed = 2usize;
    for raw in &lines[2..] {
        let trimmed = raw.trim();
        // 空行 / 不含管道符的行 / 新的代码围栏都意味着表格块在此收尾。
        if trimmed.is_empty() || !trimmed.contains('|') || trimmed.starts_with("```") {
            break;
        }
        let mut cells = split_cells(raw);
        cells.resize(cols, String::new());
        cells.truncate(cols);
        rows.push(cells);
        consumed += 1;
    }
    Some(TableBlock {
        header,
        aligns,
        rows,
        consumed,
    })
}

/// 自然列宽：表头与各数据行去标记后的最大显示宽度（宽字符按 2 列计）。
fn natural_widths(table: &TableBlock) -> Vec<usize> {
    let mut widths: Vec<usize> = table
        .header
        .iter()
        .map(|cell| str_width(&plain_text(cell)).max(1))
        .collect();
    for row in &table.rows {
        for (index, cell) in row.iter().enumerate() {
            widths[index] = widths[index].max(str_width(&plain_text(cell)));
        }
    }
    widths
}

fn total_width(widths: &[usize]) -> usize {
    widths.iter().sum::<usize>() + COL_GAP * widths.len().saturating_sub(1)
}

/// 反复收窄最宽列直到塞进 `width`；单列下限 `MIN_COL_WIDTH`，仍放不下返回 `None`。
fn fit_widths(mut widths: Vec<usize>, width: usize) -> Option<Vec<usize>> {
    loop {
        if total_width(&widths) <= width {
            return Some(widths);
        }
        let widest = widths
            .iter()
            .enumerate()
            .max_by_key(|(_, value)| **value)
            .map(|(index, value)| (index, *value))?;
        if widest.1 <= MIN_COL_WIDTH {
            return None;
        }
        widths[widest.0] = widest.1 - 1;
    }
}

/// 单元格行内样式：数据行沿用 `parse_inline_formatting`，表头追加 BOLD + `INK`。
fn cell_line(cell: &str, header: bool) -> Line<'static> {
    let mut spans = parse_inline_formatting(cell);
    if header {
        for span in spans.iter_mut() {
            span.style = span.style.fg(theme::INK).add_modifier(Modifier::BOLD);
        }
    }
    Line::from(spans)
}

/// 把一段内容按对齐补到列宽；最后一列不补尾随空格（复制出来不留脏空白）。
fn push_padded(
    out: &mut Vec<Span<'static>>,
    spans: Vec<Span<'static>>,
    used: usize,
    width: usize,
    align: Align,
    last: bool,
) {
    if used >= width {
        out.extend(spans);
        return;
    }
    let fill = width - used;
    let (lead, trail) = match align {
        Align::Left => (0, fill),
        Align::Right => (fill, 0),
        Align::Center => (fill / 2, fill - fill / 2),
    };
    if lead > 0 {
        out.push(Span::raw(" ".repeat(lead)));
    }
    out.extend(spans);
    if trail > 0 && !last {
        out.push(Span::raw(" ".repeat(trail)));
    }
}

/// 渲染一行：逐单元格按列宽折行，再按视觉行拼接（列间 2 空格，无竖线）。
fn render_row(cells: &[String], widths: &[usize], aligns: &[Align], header: bool) -> Vec<Line<'static>> {
    let mut wrapped: Vec<Vec<Line<'static>>> = Vec::with_capacity(cells.len());
    for (index, cell) in cells.iter().enumerate() {
        let line = cell_line(cell, header);
        wrapped.push(wrap_line(&line, widths[index].max(1)));
    }
    let height = wrapped.iter().map(|chunks| chunks.len()).max().unwrap_or(1).max(1);
    let mut out: Vec<Line<'static>> = Vec::with_capacity(height);
    for row_index in 0..height {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (index, chunks) in wrapped.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw(" ".repeat(COL_GAP)));
            }
            let part: Vec<Span<'static>> = chunks
                .get(row_index)
                .map(|line| line.spans.clone())
                .unwrap_or_default();
            let used: usize = part.iter().map(|span| str_width(&span.content)).sum();
            let last = index + 1 == cells.len();
            push_padded(&mut spans, part, used, widths[index], aligns[index], last);
        }
        out.push(Line::from(spans));
    }
    out
}

/// 辅助消息正文中的表格块入口。
///
/// 返回「渲染好的行 + 吃掉的源代码行数」；形态不成立或终端太窄（收窄到底仍放不下）
/// 时返回 `None`，调用方按现状逐行渲染，行为逐字节不变。
pub fn render_markdown_table(lines: &[&str], width: usize) -> Option<(Vec<Line<'static>>, usize)> {
    let table = parse_block(lines)?;
    let widths = fit_widths(natural_widths(&table), width)?;
    let mut out: Vec<Line<'static>> = Vec::new();
    out.extend(render_row(&table.header, &widths, &table.aligns, true));
    // 表头下方一条发丝线，宽度 = 表格实际占用宽度（不画整块边框）。
    out.push(Line::from(Span::styled(
        "─".repeat(total_width(&widths).min(width).max(1)),
        Style::default().fg(theme::HAIRLINE),
    )));
    for row in &table.rows {
        out.extend(render_row(row, &widths, &table.aligns, false));
    }
    Some((out, table.consumed))
}

#[cfg(test)]
mod table_tests {
    use super::*;

    fn plain(lines: &[Line<'static>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| line.spans.iter().map(|span| span.content.as_ref()).collect::<String>())
            .collect()
    }

    #[test]
    fn renders_header_hairline_and_rows() {
        let src = vec!["| 名称 | 值 |", "| --- | --- |", "| alpha | 1 |", "| beta | 2 |"];
        let (lines, used) = render_markdown_table(&src, 60).expect("2x2 表格应被识别");
        assert_eq!(used, 4, "应吃掉表头+分隔行+2 数据行");
        // 1 表头 + 1 发丝线 + 2 数据行：不再原样打印 `|---|---|`。
        assert_eq!(lines.len(), 4);
        let texts = plain(&lines);
        assert!(texts[0].contains("名称") && texts[0].contains("值"));
        assert!(texts[1].chars().all(|c| c == '─'));
        // 同一行同时可见两列的文本（块级排版，而不是逐行原样输出）。
        assert!(texts[2].contains("alpha") && texts[2].contains('1'));
        assert!(!texts.iter().any(|text| text.contains('|')));
    }

    #[test]
    fn missing_separator_returns_none() {
        let src = vec!["| a | b |", "| c | d |"];
        assert!(render_markdown_table(&src, 60).is_none());
    }

    #[test]
    fn narrow_terminal_keeps_every_line_within_width() {
        let src = vec![
            "| 一个很长很长的表头 | B |",
            "| --- | --- |",
            "| 很长很长很长的内容文本 | second column |",
        ];
        let (lines, _) = render_markdown_table(&src, 20).expect("收窄到 4 列下限后仍应可渲染");
        for line in &lines {
            assert!(
                line_display_width(line) <= 20,
                "行宽超限: {}",
                line.spans.iter().map(|s| s.content.as_ref()).collect::<String>()
            );
        }
    }

    #[test]
    fn inline_code_loses_backticks() {
        let src = vec!["| 字段 | 说明 |", "| --- | --- |", "| `id` | 主键 |"];
        let (lines, _) = render_markdown_table(&src, 60).expect("表格应被识别");
        let joined: String = plain(&lines).concat();
        assert!(!joined.contains('`'), "行内代码的反引号不得残留: {joined}");
        assert!(joined.contains("id"));
    }
}
