//! 纯文本度量工具（原 `tui.rs` 内联实现，机械外移）。
//! 东亚宽字符宽度、截断/省略、对齐补齐、粘贴规范化——全部无 IO、无状态。

use super::*;

pub fn format_elapsed_seconds(ms: u64) -> String {
    let secs = ms / 1000;
    if secs < 60 {
        format!("{}秒", secs)
    } else {
        let mins = secs / 60;
        let rem_secs = secs % 60;
        if rem_secs > 0 {
            format!("{}分{}秒", mins, rem_secs)
        } else {
            format!("{}分钟", mins)
        }
    }
}

pub fn clean_tool_name_display(name: &str) -> String {
    let base = name.split('·').next().unwrap_or(name).trim();
    let cleaned = base
        .trim_start_matches('●')
        .trim_start_matches('✔')
        .trim();
    cleaned.to_string()
}

/// 契约 F-C：粘贴文本归一化——CRLF / CR 统一为 LF，其余字符原样保留。
///
/// 终端 pasting 会把换行按平台习惯送上（Windows CRLF / 老终端 CR）；直接在缓冲里插入
/// `\r` 会让后续按行渲染与提交判定出现幽灵空行，故在入口一次性归一。
pub fn normalize_paste(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// 东亚宽字符按 2 列计宽，保证中文/emoji 折行与滚动偏移精确。
pub fn char_width(c: char) -> usize {
    let cp = c as u32;
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
    if wide {
        2
    } else {
        1
    }
}

pub fn str_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// 行首禁则：这些标点不允许出现在行首（悬挂到上一行末尾）。
pub fn is_no_line_start(c: char) -> bool {
    matches!(
        c,
        '，' | '。'
            | '、'
            | '；'
            | '：'
            | '？'
            | '！'
            | '）'
            | '】'
            | '》'
            | '」'
            | '』'
            | '…'
            | ','
            | '.'
            | ';'
            | ':'
            | '?'
            | '!'
            | ')'
            | ']'
            | '%'
    )
}

pub fn line_display_width(line: &Line<'static>) -> usize {
    line.spans.iter().map(|s| str_width(&s.content)).sum()
}

pub fn truncate_width(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = char_width(ch);
        if used + w > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

/// 按显示宽度截断，并以省略号收尾（头部元信息/面板右列专用）。
pub fn ellipsize_width(text: &str, width: usize) -> String {
    if str_width(text) <= width {
        return text.to_string();
    }
    if width == 0 {
        return String::new();
    }
    let mut out = truncate_width(text, width.saturating_sub(1));
    out.push('…');
    out
}

/// wave-39 R7：token 数字缩写 —— `<1000` 原样；`<1e6` 一位小数 `k`（`.0` 省略）；
/// 否则一位小数 `M`（`.0` 省略）。
pub fn fmt_token_count(n: u64) -> String {
    fn one_decimal(value: f64) -> String {
        let rounded = (value * 10.0).round() / 10.0;
        let text = format!("{:.1}", rounded);
        text.strip_suffix(".0").unwrap_or(&text).to_string()
    }
    if n < 1_000 {
        return n.to_string();
    }
    if n < 1_000_000 {
        let k = n as f64 / 1_000.0;
        // 999_999 四舍五入后会变成 1000k：改以 M 表达，避免「1000k」这种破格写法。
        if (k * 10.0).round() >= 10_000.0 {
            return format!("{}M", one_decimal(k / 1_000.0));
        }
        return format!("{}k", one_decimal(k));
    }
    format!("{}M", one_decimal(n as f64 / 1_000_000.0))
}

/// 按「字符」截断（绝不按字节切分，避免 CJK 边界 panic），超长时补省略号。
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        out.push('…');
    }
    out
}

pub fn pad_display(text: &str, width: usize) -> String {
    let used = str_width(text);
    if used >= width {
        text.to_string()
    } else {
        format!("{}{}", text, " ".repeat(width - used))
    }
}

/// 把「鼠标命中的显示列」吸附到所在字形的起始列。
///
/// 宽字符（CJK / emoji）占 2 个显示列，点在右半格时命中的列落在字形中间：
/// 若直接用该列做选区端点，`起 < 字形起点`、`止 > 字形起点`，高亮与复制的边界
/// 就会各说各话（复制会带上半个字的判定、续选会从字中间开始）。吸附后：
/// 同一个字形内的任意列都归约到该字起点，行尾之后归约到行宽（允许延伸选到行尾）。
pub fn snap_col_to_glyph_start(text: &str, col: usize) -> usize {
    let mut display = 0usize;
    for ch in text.chars() {
        let width = char_width(ch).max(1);
        if col < display + width {
            return display;
        }
        display += width;
    }
    display
}
