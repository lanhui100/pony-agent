//! Markdown 行渲染与代码带高亮（原 `tui.rs` 内联实现，机械外移）。

use super::*;

pub fn render_markdown_line(line: &str) -> Vec<Span<'static>> {
    let trimmed = line.trim_start();
    let hash_count = trimmed.chars().take_while(|&c| c == '#').count();
    if hash_count > 0 && trimmed[hash_count..].starts_with(' ') {
        // 标题不用符号噪音，只用字重与微小的色阶差异表达层级。
        let clean_text = trimmed[hash_count..].trim_start();
        let style = match hash_count {
            1 => Style::default().fg(theme::INK).add_modifier(Modifier::BOLD),
            2 => Style::default().fg(theme::INK).add_modifier(Modifier::BOLD),
            3 => Style::default().fg(theme::INK_SOFT).add_modifier(Modifier::BOLD),
            _ => Style::default().fg(theme::INK_SOFT),
        };
        return vec![Span::styled(clean_text.to_string(), style)];
    }

    if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
        let mut spans = vec![Span::styled("  · ", faint())];
        spans.extend(parse_inline_formatting(rest));
        return spans;
    }

    // 数字有序列表例如 "1. ", "2. "
    if let Some(dot_idx) = trimmed.find(". ") {
        let num_part = &trimmed[..dot_idx];
        if !num_part.is_empty() && num_part.chars().all(|c| c.is_ascii_digit()) {
            let mut spans = vec![Span::styled(format!("  {}. ", num_part), faint())];
            spans.extend(parse_inline_formatting(&trimmed[dot_idx + 2..]));
            return spans;
        }
    }

    // 分割线 --- 或 ***：一条安静的发丝线
    if trimmed == "---" || trimmed == "***" || trimmed == "___" {
        return vec![Span::styled(
            "─".repeat(48),
            Style::default().fg(theme::HAIRLINE),
        )];
    }

    // 代码块围栏 ```lang 只显示语言标签；裸围栏保持安静（不占视觉噪音）
    if trimmed.starts_with("```") {
        let lang = trimmed.trim_start_matches('`').trim();
        if lang.is_empty() {
            return vec![Span::raw("")];
        }
        return vec![Span::styled(lang.to_string(), faint())];
    }

    parse_inline_formatting(line)
}

pub(super) fn parse_inline_formatting(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text;
    let plain = |s: &str| Span::styled(s.to_string(), ink_soft());

    while !rest.is_empty() {
        let bold = rest.find("**");
        let code = rest.find('`');
        // 按「先出现的标记」处理，避免先遇到加粗就把前面的行内代码整段吞成纯文本。
        let use_bold = match (bold, code) {
            (Some(b), Some(c)) => b <= c,
            (Some(_), None) => true,
            _ => false,
        };
        if use_bold {
            let b = bold.unwrap();
            if let Some(off) = rest[b + 2..].find("**") {
                if b > 0 {
                    spans.push(plain(&rest[..b]));
                }
                spans.push(Span::styled(
                    rest[b + 2..b + 2 + off].to_string(),
                    Style::default()
                        .fg(theme::EMPHASIS)
                        .add_modifier(Modifier::BOLD),
                ));
                rest = &rest[b + 2 + off + 2..];
                continue;
            }
        }
        if let Some(c) = code {
            if let Some(off) = rest[c + 1..].find('`') {
                if c > 0 {
                    spans.push(plain(&rest[..c]));
                }
                spans.push(Span::styled(
                    rest[c + 1..c + 1 + off].to_string(),
                    Style::default().fg(theme::VSCODE_STRING).bg(theme::CODE_FILL),
                ));
                rest = &rest[c + 1 + off + 1..];
                continue;
            }
        }
        spans.push(plain(rest));
        break;
    }
    spans
}

/// 代码块色带的右端内缩量：色块不贴内容列右缘，留出呼吸。
pub const CODE_BAND_INSET: usize = 2;

/// 色带内部左右内边距：代码文字与色块边缘之间留 3 列内边距，提升视觉质感。
pub const CODE_BAND_PAD: usize = 3;

/// VS Code 风格的高亮代码行着色器（无重依赖的极速词法识别）
pub fn highlight_code_line(raw: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let trimmed = raw.trim_start();
    let leading_spaces = raw.len() - trimmed.len();
    if leading_spaces > 0 {
        spans.push(Span::raw(" ".repeat(leading_spaces)));
    }

    // 单行注释
    if trimmed.starts_with("//") || trimmed.starts_with('#') {
        spans.push(Span::styled(
            trimmed.to_string(),
            Style::default().fg(theme::VSCODE_COMMENT).add_modifier(Modifier::ITALIC),
        ));
        return spans;
    }

    // 属性宏例如 #[derive(...)]
    if trimmed.starts_with("#[") || trimmed.starts_with('@') {
        spans.push(Span::styled(
            trimmed.to_string(),
            Style::default().fg(theme::VSCODE_ATTR),
        ));
        return spans;
    }

    let mut rest = trimmed;
    while !rest.is_empty() {
        // 查找字符串 "..." 或 '...'
        if let Some(quote_idx) = rest.find('"') {
            if let Some(next_quote) = rest[quote_idx + 1..].find('"') {
                let before = &rest[..quote_idx];
                if !before.is_empty() {
                    spans.extend(tokenize_code_segment(before));
                }
                let str_content = &rest[quote_idx..=quote_idx + 1 + next_quote];
                spans.push(Span::styled(
                    str_content.to_string(),
                    Style::default().fg(theme::VSCODE_STRING),
                ));
                rest = &rest[quote_idx + 2 + next_quote..];
                continue;
            }
        }
        spans.extend(tokenize_code_segment(rest));
        break;
    }
    spans
}

pub fn tokenize_code_segment(segment: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut word = String::new();

    let flush_word = |spans: &mut Vec<Span<'static>>, w: &mut String| {
        if w.is_empty() {
            return;
        }
        let token = w.clone();
        w.clear();

        // 关键字
        let is_keyword = matches!(
            token.as_str(),
            "fn" | "let" | "mut" | "pub" | "struct" | "enum" | "impl" | "use" | "mod"
                | "const" | "static" | "type" | "trait" | "async" | "await" | "return"
                | "if" | "else" | "match" | "for" | "in" | "while" | "loop" | "break"
                | "continue" | "def" | "class" | "import" | "from" | "as" | "function"
                | "var" | "val" | "export" | "default"
        );
        if is_keyword {
            spans.push(Span::styled(
                token,
                Style::default().fg(theme::VSCODE_KEYWORD).add_modifier(Modifier::BOLD),
            ));
            return;
        }

        // 控流特殊词
        if matches!(token.as_str(), "mut" | "ref") {
            spans.push(Span::styled(
                token,
                Style::default().fg(theme::VSCODE_CONTROL),
            ));
            return;
        }

        // 布尔与空值
        if matches!(token.as_str(), "true" | "false" | "None" | "Some" | "Ok" | "Err" | "null" | "nil" | "self" | "Self") {
            spans.push(Span::styled(
                token,
                Style::default().fg(theme::VSCODE_VARIABLE).add_modifier(Modifier::ITALIC),
            ));
            return;
        }

        // 类型 (首字母大写)
        if token.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
            spans.push(Span::styled(
                token,
                Style::default().fg(theme::VSCODE_TYPE),
            ));
            return;
        }

        // 数字
        if token.chars().all(|c| c.is_ascii_digit() || c == '.' || c == '_') {
            spans.push(Span::styled(
                token,
                Style::default().fg(theme::VSCODE_NUMBER),
            ));
            return;
        }

        // 常规函数名 / 标识符：如果后面紧跟着括号，或者作为常用命名
        spans.push(Span::styled(token, Style::default().fg(theme::VSCODE_VARIABLE)));
    };

    for c in segment.chars() {
        if c.is_alphanumeric() || c == '_' {
            word.push(c);
        } else {
            flush_word(&mut spans, &mut word);
            let symbol_style = match c {
                '(' | ')' | '{' | '}' | '[' | ']' => Style::default().fg(theme::VSCODE_ATTR),
                ':' | ';' | ',' | '.' => Style::default().fg(theme::FAINT),
                '=' | '+' | '-' | '*' | '/' | '%' | '&' | '|' | '!' | '<' | '>' | '^' => {
                    Style::default().fg(theme::VSCODE_CONTROL)
                }
                _ => Style::default().fg(theme::INK_SOFT),
            };
            spans.push(Span::styled(c.to_string(), symbol_style));
        }
    }
    flush_word(&mut spans, &mut word);
    spans
}

/// 向代码块色带追加一行整行背景留白，形成顶部或底部的代码块内边距（内边距仍带有 CODE_FILL 底色）。
pub fn push_code_band_padding(lines: &mut Vec<Line<'static>>, width: usize) {
    let band_w = width
        .saturating_sub(RAIL + CODE_BAND_INSET)
        .max(CODE_BAND_PAD * 2 + 1);
    let mut spans = blank_rail(Style::default());
    spans.push(Span::styled(" ", Style::default().bg(theme::CODE_FILL)));
    lines.push(pad_line_with_bg(
        Line::from(spans),
        RAIL + band_w,
        theme::CODE_FILL,
    ));
}

/// 代码块材质：语言标签行与代码行共用同一条 `CODE_FILL` 陶土暗底色带，无竖线、无边框。
/// 色带从轨道后起排，右端内缩 `CODE_BAND_INSET` 列；超出宽度的代码按显示宽度折行，
/// 内部增加 CODE_BAND_PAD 列内边距，提升呼吸感与专业代码编辑器质感。
pub fn push_code_band_line(lines: &mut Vec<Line<'static>>, body: Line<'static>, width: usize) {
    let band_w = width
        .saturating_sub(RAIL + CODE_BAND_INSET)
        .max(CODE_BAND_PAD * 2 + 1);
    let body_w = band_w.saturating_sub(CODE_BAND_PAD * 2).max(1);
    let bg = Style::default().bg(theme::CODE_FILL);
    for wrapped in wrap_line(&body, body_w) {
        let mut spans = blank_rail(Style::default());
        spans.push(Span::styled(" ".repeat(CODE_BAND_PAD), bg));
        spans.extend(
            wrapped
                .spans
                .into_iter()
                .map(|span| Span::styled(span.content, span.style.bg(theme::CODE_FILL))),
        );
        lines.push(pad_line_with_bg(
            Line::from(spans),
            RAIL + band_w,
            theme::CODE_FILL,
        ));
    }
}
