/// wave-tui-session-resilience C1：DSML 工具调用协议原文的**唯一**清洗入口。
/// 二元隔离在 SSE 解析层完成，流式增量、最终累计文本与历史回放共用同一语义；
/// 纯函数、不新增依赖、不误伤 markdown 代码块 / 普通含尖括号正文。
const DSML_PLACEHOLDER: &str = "［工具调用］";

/// DSML 标记的规范写法的「归一化签名」：大小写不敏感 + 全角/半角竖线统一。
/// 只用于判定，绝不改写用户正文（输出仍逐字保真原字符）。
fn dsml_signature(text: &str) -> String {
    text.to_ascii_lowercase().replace('｜', "|")
}

/// 尾部「已识别 DSML 协议前缀」的半截形态：只认已知标签的合法前缀，
/// 普通正文以单个 `<` 结尾 / markdown 代码块 / `<tag>` 一律不判命中。
fn dsml_partial_tail_len(text: &str) -> Option<usize> {
    let trimmed = text.trim_end_matches(['\r', '\n', ' ', '\t']);
    let known_tags = [
        "<||dsml|| calls>",
        "</||dsml|| calls>",
        "<||dsml|| invoke",
        "</||dsml|| invoke>",
        "<||dsml|| parameter",
        "</||dsml|| parameter>",
        "<||dsml||",
        "|dsml||",
        "||ds",
        "<|",
    ];
    // 只检查尾部有限窗口内、合法 UTF-8 字符边界上的候选，
    // 绝不按裸 byte offset 切片；单个 `<` 属普通正文，不判命中。
    let max_tag_len = known_tags.iter().map(|tag| tag.len()).max().unwrap_or(0) * 3 + 4;
    for (start, _) in trimmed.char_indices().rev() {
        if trimmed.len() - start > max_tag_len {
            break;
        }
        let tail = &trimmed[start..];
        if tail.contains('>') || tail.len() <= "<".len() {
            continue;
        }
        // `<|` / `<｜` 这类最短前缀只在行首或换行后判为协议碎片；
        // 行中普通正文（如 `a <| 半角竖线`）逐字保真。
        if start > 0 && !trimmed[..start].ends_with('\n') && !tail.starts_with("<｜") {
            continue;
        }
        let signature = dsml_signature(tail);
        if known_tags.iter().any(|tag| tag.starts_with(&signature)) {
            return Some(text.len() - start);
        }
    }
    None
}

/// 协议行判定：归一化后必须含 `|dsml|`（覆盖大小写 / 半角 / 全角 / 混合变体），
/// 普通 `<tag>`、markdown 代码块、含 `<｜不是 DSML` 之类正文不命中。
fn is_dsml_protocol_line(line: &str) -> bool {
    dsml_signature(line).contains("|dsml|")
}

/// 协议**碎片**行判定（RC3 / C5）：跨 chunk 或截断折叠后可能残留不带 `|dsml|` 标记的
/// 纯参数/标签残片（如 `parameter name="command">cargo test`）。
///
/// **只在协议块内 / 非 fenced 代码块内生效**（见 [`strip_dsml_protocol_text`] 的单遍状态机）：
/// 正文里以 `parameter name=` 开头的普通行、以及 fenced 代码块内的示例，一律逐字保真
/// （L3-B 反例：` ```\nparameter name="command">cargo test\n``` ` 必须原样保留）。
fn is_dsml_fragment_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    const FRAGMENT_PREFIXES: [&str; 4] = [
        "parameter name=",
        "invoke name=",
        "antml:parameter",
        "antml:invoke",
    ];
    FRAGMENT_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
        || (lower.starts_with("</") && lower.contains("dsml"))
}

/// 协议状态机（L3-B）：区分「正文」/「fenced 代码块」/「DSML 协议块」三种上下文。
///
/// 只有**协议块内**才允许丢弃碎片行；fenced 代码块内一律逐字保真。协议块由带 `|dsml|`
/// 的开标签行进入（`<…DSML…calls>` / `<…DSML…invoke`），由闭标签行或正文边界退出。
#[derive(Default)]
struct DsmlScanState {
    in_fence: bool,
    in_protocol_block: bool,
}

impl DsmlScanState {
    /// 处理一行，返回 `true` 表示该行是协议内容、应当丢弃。
    fn observe(&mut self, line: &str) -> bool {
        let trimmed = line.trim_start();
        // 1) fenced 代码块边界：``` 行本身保留，块内一律保真（含形似协议的内容）。
        if trimmed.starts_with("```") {
            self.in_fence = !self.in_fence;
            return false;
        }
        if self.in_fence {
            return false;
        }
        // 2) 带 `|dsml|` 标记的协议行：整行丢弃，并据开/闭标签更新块状态。
        if is_dsml_protocol_line(line) {
            let lower = dsml_signature(trimmed);
            let is_closing = lower.starts_with("</") || lower.contains("</");
            if is_closing {
                self.in_protocol_block = false;
            } else if lower.contains("calls") || lower.contains("invoke") {
                self.in_protocol_block = true;
            }
            return true;
        }
        // 3) 协议块内的无标记残片（参数值/未闭合尾巴）：只在块内丢弃。
        if self.in_protocol_block && is_dsml_fragment_line(line) {
            return true;
        }
        // 4) 空行不改变块状态（协议块内允许空行，仍属协议区）。
        if self.in_protocol_block && !trimmed.is_empty() {
            // 非空、非碎片、非协议行 ⇒ 正文开始，协议块结束（未闭合块就地收口）。
            self.in_protocol_block = false;
        }
        false
    }
}

/// 占位符最多出现一次（C5：旧脏行的占位符不得被放大/连排）。
///
/// 判据是「**连续**的占位符段」而非整行相等：旧库脏行既有整行 `［工具调用］` 的重复行，
/// 也有 `［工具调用］［工具调用］` 这种行内连排；两者都必须收敛为一个。
/// 占位符之间只允许空白（含换行），遇到其它正文即视为新的一段。
fn collapse_dsml_placeholders(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut rest = text;
    let mut previous_was_placeholder = false;
    while let Some(index) = rest.find(DSML_PLACEHOLDER) {
        let (before, after) = rest.split_at(index);
        let after = &after[DSML_PLACEHOLDER.len()..];
        kept.push_str(before);
        if before.chars().any(|ch| !ch.is_whitespace()) {
            previous_was_placeholder = false;
        }
        if !previous_was_placeholder {
            kept.push_str(DSML_PLACEHOLDER);
            previous_was_placeholder = true;
        }
        rest = after;
    }
    kept.push_str(rest);
    kept
}

/// 持久化侧清洗（RC3 / C5）：**整块丢弃** DSML 协议行与协议碎片行，且**不补占位符**。
///
/// 单遍状态机（L3-B）：
/// - fenced 代码块（```）内一律逐字保真（示例代码不得被当协议删掉）；
/// - 带 `|dsml|` 标记的协议行整行丢弃，并按开/闭标签维护"协议块"状态；
/// - 协议块内的无标记残片（参数值/未闭合尾巴）丢弃；块外同形正文逐字保真；
/// - 旧库"孤立残片"行（无块上下文的 `parameter name=…`，非 fenced）按只读清理丢弃，不回写放大。
///
/// 对干净正文幂等（`strip(strip(x)) == strip(x)`）。
pub fn strip_dsml_protocol_text(text: &str) -> String {
    let body = match dsml_partial_tail_len(text) {
        Some(len) => &text[..text.len() - len],
        None => text,
    };
    let mut scan = DsmlScanState::default();
    let mut cleaned = String::with_capacity(body.len());
    for line in body.split_inclusive('\n') {
        if scan.observe(line) {
            continue;
        }
        if !scan.in_fence && is_dsml_fragment_line(line) {
            continue;
        }
        cleaned.push_str(line);
    }
    if let Some(len) = dsml_partial_tail_len(&cleaned) {
        cleaned.truncate(cleaned.len() - len);
    }
    collapse_dsml_placeholders(&cleaned)
}

/// 展示侧清洗（C4/C5 展示腿）：沿用既有折叠（保留一个占位符），额外丢弃**孤立**协议残片行
/// （fenced 内不丢），并把连排占位符收敛为一个。
///
/// 持久化正文**不得**经本函数（那会把占位符写回落库）；调用方必须传**整段正文**，
/// 不得逐 delta 传（逐 delta 时 delta 起点会被误当行首，L3-B）。
pub fn display_dsml_protocol_text(text: &str) -> String {
    let folded = sanitize_dsml_protocol_text(text);
    let mut scan = DsmlScanState::default();
    let mut kept = String::with_capacity(folded.len());
    for line in folded.split_inclusive('\n') {
        // 折叠后协议块已被删除，此处只需 fenced 判定 + 孤立残片过滤。
        let _ = scan.observe(line);
        if !scan.in_fence && is_dsml_fragment_line(line) {
            continue;
        }
        kept.push_str(line);
    }
    collapse_dsml_placeholders(&kept)
}

pub fn sanitize_dsml_protocol_text(text: &str) -> String {
    // 跨 chunk 被切断的半截标记先截掉：分片累加时不得残留 `<｜` / `｜DSML` 等前缀。
    let body = match dsml_partial_tail_len(text) {
        Some(len) => &text[..text.len() - len],
        None => text,
    };
    // 整行折叠：DSML 协议块（含大小写/半角/混合变体）按行剔除，
    // 块外正文逐字保真；至少折叠一次时给一个面向用户的占位。
    let mut cleaned = String::with_capacity(body.len());
    let mut removed_protocol = false;
    for line in body.split_inclusive('\n') {
        if is_dsml_protocol_line(line) {
            removed_protocol = true;
            continue;
        }
        cleaned.push_str(line);
    }
    if removed_protocol && !cleaned.contains(DSML_PLACEHOLDER) {
        cleaned.push_str(DSML_PLACEHOLDER);
    }
    // 收尾：孤立半截尾巴（如 `｜DSML`）再截一次。
    if let Some(len) = dsml_partial_tail_len(&cleaned) {
        cleaned.truncate(cleaned.len() - len);
    }
    cleaned
}
