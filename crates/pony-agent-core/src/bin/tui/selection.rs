//! 鼠标文本选择与复制（原 `tui.rs` 内联实现，机械外移）。
//! 坐标换算、跨行提取、OSC52 复制——全部纯函数，无渲染副作用。

use super::*;

/// 文本字符坐标（行、字符索引）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TextCoord {
    pub line: usize,
    pub col: usize,
}

/// 鼠标拖拽选择状态机
#[derive(Debug, Clone, Default)]
pub struct MouseSelectionState {
    pub active: bool,
    pub anchor: Option<TextCoord>,
    pub head: Option<TextCoord>,
    pub debounce_until: Option<std::time::Instant>,
    pub copied_range: Option<(TextCoord, TextCoord)>,
}

/// 鼠标命中 → 选区坐标（`None` = 不在任何可选择区域内）。
///
/// 坐标空间统一：正文用 0 起始的绝对渲染行号；输入框用 `INPUT_LINE_BASE + 逻辑行号`。
pub fn selection_point_at(state: &TuiState, x: u16, y: u16) -> Option<TextCoord> {
    if let Some(rect) = state.last_input_rect {
        if x >= rect.x
            && x < rect.x.saturating_add(rect.width)
            && y >= rect.y
            && y < rect.y.saturating_add(rect.height)
        {
            let line = INPUT_LINE_BASE + state.last_input_view_start + (y - rect.y) as usize;
            return Some(TextCoord {
                line,
                col: snap_hit_col(state, line, (x - rect.x) as usize),
            });
        }
    }
    let chat_rect = state.last_chat_rect?;
    if x >= chat_rect.x
        && x < chat_rect.x.saturating_add(chat_rect.width)
        && y >= chat_rect.y
        && y < chat_rect.y.saturating_add(chat_rect.height)
    {
        let line = (y - chat_rect.y) as usize + state.effective_scroll_offset as usize;
        return Some(TextCoord {
            line,
            col: snap_hit_col(state, line, (x - chat_rect.x) as usize),
        });
    }
    None
}

/// 拖拽时的命中：越出正文矩形后吸附到可视区首/末行与左右边界，保证连续扩选不中断。
pub fn selection_drag_point(state: &TuiState, x: u16, y: u16) -> Option<TextCoord> {
    if let Some(coord) = selection_point_at(state, x, y) {
        return Some(coord);
    }
    let chat_rect = state.last_chat_rect?;
    if chat_rect.width == 0 || chat_rect.height == 0 {
        return None;
    }
    let x_clamped = x
        .max(chat_rect.x)
        .min(chat_rect.x.saturating_add(chat_rect.width).saturating_sub(1));
    let last_row = chat_rect.height.saturating_sub(1) as usize;
    let row = (y as usize)
        .saturating_sub(chat_rect.y as usize)
        .min(last_row);
    let line = row + state.effective_scroll_offset as usize;
    Some(TextCoord {
        line,
        col: snap_hit_col(state, line, (x_clamped - chat_rect.x) as usize),
    })
}

/// 跨区域（正文 ↔ 输入框）选区提取：按坐标行号拆分两段，避免构造百万级空行占位。
pub fn extract_selection_across(
    chat_lines: &[String],
    input_lines: &[String],
    anchor: TextCoord,
    head: TextCoord,
) -> String {
    let (start, end) = if anchor <= head {
        (anchor, head)
    } else {
        (head, anchor)
    };
    if start.line >= INPUT_LINE_BASE {
        let from = TextCoord {
            line: start.line - INPUT_LINE_BASE,
            col: start.col,
        };
        let to = TextCoord {
            line: end.line - INPUT_LINE_BASE,
            col: end.col,
        };
        return extract_selected_by_display_cols(input_lines, from, to);
    }
    if end.line < INPUT_LINE_BASE {
        return extract_selected_by_display_cols(chat_lines, start, end);
    }
    let chat_tail = extract_selected_by_display_cols(
        chat_lines,
        start,
        TextCoord {
            line: chat_lines.len().saturating_sub(1),
            col: usize::MAX,
        },
    );
    let input_part = extract_selected_by_display_cols(
        input_lines,
        TextCoord { line: 0, col: 0 },
        TextCoord {
            line: end.line - INPUT_LINE_BASE,
            col: end.col,
        },
    );
    match (chat_tail.is_empty(), input_part.is_empty()) {
        (true, _) => input_part,
        (false, true) => chat_tail,
        (false, false) => format!("{chat_tail}\n{input_part}"),
    }
}

/// 将文本编码为 OSC 52 终端剪贴板序列
pub fn encode_osc52_copy(text: &str) -> String {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
    format!("\x1b]52;c;{}\x07", b64)
}

/// 判断给定坐标是否落在由 anchor 和 head 围成的选区内
pub fn is_coord_selected(coord: TextCoord, anchor: TextCoord, head: TextCoord) -> bool {
    let (start, end) = if anchor <= head {
        (anchor, head)
    } else {
        (head, anchor)
    };
    if coord.line < start.line || coord.line > end.line {
        return false;
    }
    if start.line == end.line {
        coord.col >= start.col && coord.col <= end.col
    } else if coord.line == start.line {
        coord.col >= start.col
    } else if coord.line == end.line {
        coord.col <= end.col
    } else {
        true
    }
}

/// 从多行纯文本切片中截取选区内的文本内容
pub fn extract_selected_text(lines: &[String], anchor: TextCoord, head: TextCoord) -> String {
    let (start, end) = if anchor <= head {
        (anchor, head)
    } else {
        (head, anchor)
    };
    if lines.is_empty() || start.line >= lines.len() {
        return String::new();
    }
    let end_line = end.line.min(lines.len() - 1);
    if start.line == end_line {
        let line = &lines[start.line];
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() || start.col >= chars.len() {
            return String::new();
        }
        let end_col = end.col.min(chars.len().saturating_sub(1));
        if start.col > end_col {
            return String::new();
        }
        chars[start.col..=end_col].iter().collect()
    } else {
        let mut result = Vec::new();
        for line_idx in start.line..=end_line {
            let line = &lines[line_idx];
            let chars: Vec<char> = line.chars().collect();
            if line_idx == start.line {
                if !chars.is_empty() && start.col < chars.len() {
                    result.push(chars[start.col..].iter().collect::<String>());
                } else {
                    result.push(String::new());
                }
            } else if line_idx == end_line {
                if !chars.is_empty() {
                    let end_col = end.col.min(chars.len().saturating_sub(1));
                    result.push(chars[..=end_col].iter().collect::<String>());
                } else {
                    result.push(String::new());
                }
            } else {
                result.push(line.clone());
            }
        }
        result.join("\n")
    }
}

/// 显示列语义的精确选区提取（鼠标选区的真实路径）。
///
/// - `anchor` / `head` 的 `col` 是**终端显示列**（宽字符占 2 列），不是 char 索引；
/// - 端点语义为「包含该列所在的字符」，拖到宽字符右半格也整字选中；
/// - 每行右侧的底色填充空格是渲染产物，复制时裁掉，避免粘贴出尾随空白。
pub fn extract_selected_by_display_cols(
    lines: &[String],
    anchor: TextCoord,
    head: TextCoord,
) -> String {
    let (start, end) = if anchor <= head {
        (anchor, head)
    } else {
        (head, anchor)
    };
    if lines.is_empty() || start.line >= lines.len() {
        return String::new();
    }
    let end_line = end.line.min(lines.len() - 1);
    let mut out: Vec<String> = Vec::new();
    for line_idx in start.line..=end_line {
        let line = &lines[line_idx];
        let col_from = if line_idx == start.line { start.col } else { 0 };
        let col_to = if line_idx == end.line {
            end.col
        } else {
            usize::MAX
        };
        let mut buf = String::new();
        let mut display_col = 0usize;
        for ch in line.chars() {
            let width = char_width(ch);
            let ch_start = display_col;
            let ch_end = display_col + width - 1;
            display_col += width;
            if ch_end >= col_from && ch_start <= col_to {
                buf.push(ch);
            } else if !buf.is_empty() {
                break;
            }
        }
        out.push(buf.trim_end().to_string());
    }
    out.join("\n")
}

/// 鼠标选区高亮：选区用**绝对行号 + 显示列**表达，窗口可能只是全量行的一段，故需要 `first_line` 偏移。
pub fn apply_selection_highlight(
    lines: &mut [Line<'static>],
    first_line: usize,
    range: Option<(TextCoord, TextCoord)>,
) {
    let Some((start, end)) = range else {
        return;
    };
    let highlight_style = Style::default().add_modifier(Modifier::REVERSED);
    let last_line = first_line + lines.len().saturating_sub(1);
    for line_idx in start.line..=end.line.min(last_line) {
        let Some(local_idx) = line_idx.checked_sub(first_line) else {
            continue;
        };
        let Some(old_line) = lines.get(local_idx) else {
            continue;
        };
        // 本行参与的显示列窗口（起行取 start.col，止行取 end.col，中间行全选）。
        let col_from = if line_idx == start.line { start.col } else { 0 };
        let col_to = if line_idx == end.line {
            end.col
        } else {
            usize::MAX
        };
        let mut new_spans: Vec<Span<'static>> = Vec::new();
        let mut display_col = 0usize;
        for span in &old_line.spans {
            let base_style = span.style;
            let mut segment = String::new();
            let mut in_sel = false;
            for ch in span.content.chars() {
                // 宽字符占 2 列：旧实现每 char 只 +1 列，导致中文/emoji 行选区整体左漂。
                let width = char_width(ch);
                let ch_start = display_col;
                let ch_end = display_col + width - 1;
                display_col += width;
                let is_sel = ch_end >= col_from && ch_start <= col_to;
                if segment.is_empty() {
                    in_sel = is_sel;
                    segment.push(ch);
                } else if in_sel == is_sel {
                    segment.push(ch);
                } else {
                    let style = if in_sel {
                        base_style.patch(highlight_style)
                    } else {
                        base_style
                    };
                    new_spans.push(Span::styled(std::mem::take(&mut segment), style));
                    in_sel = is_sel;
                    segment.push(ch);
                }
            }
            if !segment.is_empty() {
                let style = if in_sel {
                    base_style.patch(highlight_style)
                } else {
                    base_style
                };
                new_spans.push(Span::styled(segment, style));
            }
        }
        lines[local_idx] = Line::from(new_spans);
    }
}

/// 命中列 → 选区列：正文行与输入框行都按「字形起点」归约（宽字符整字语义）。
pub fn snap_hit_col(state: &TuiState, line: usize, raw_col: usize) -> usize {
    if line >= INPUT_LINE_BASE {
        state
            .last_input_lines
            .get(line - INPUT_LINE_BASE)
            .map(|text| snap_col_to_glyph_start(text, raw_col))
            .unwrap_or(raw_col)
    } else {
        chat_line_plain_text(state, line)
            .map(|text| snap_col_to_glyph_start(&text, raw_col))
            .unwrap_or(raw_col)
    }
}
