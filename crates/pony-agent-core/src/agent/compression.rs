//! 会话上下文压缩模块。
//!
//! 职责：
//! - 判断是否需要压缩（基于 token 水位阈值触发）
//! - 将历史拆分为"可压缩部分"和"保留原始部分"
//! - 构建压缩 prompt 供 LLM 生成摘要
//! - 将摘要应用回历史，替换旧消息
//! - **同时**给出原生 transcript 的裁剪轮数（两条记忆载体必须同步裁剪，否则
//!   native-tool-flow 会话的压缩等于没压：上下文组装只看 transcript）
//!
//! 设计原则：高内聚低耦合。
//! - 本模块不依赖 runtime 内部细节
//! - 不直接调用 LLM（由调用方传入摘要文本）
//! - 所有函数为纯数据变换
//!
//! 计量口径的唯一真源：`ContextPressure` / `compression_threshold_tokens` /
//! `retain_tokens_for`。调用方（自动压缩、手动 `/compact`）**不得**各自现算阈值
//! 或保留量——两处口径一旦分叉，同一会话会在不同入口得到不同的压缩边界。

use crate::agent::provider::ProviderManager;
use crate::agent::session::TurnHistoryMessage;
use serde_json::Value;

/// 压缩配置
#[derive(Clone, Debug)]
pub struct CompressionConfig {
    /// 触发压缩的阈值，占 context window 的比例（默认 0.8 = 80%）
    pub trigger_threshold: f64,
    /// 预留净空 token 数量（DSH headroom_tokens，用于容纳输出与请求净空）
    pub headroom_tokens: usize,
    /// provider 未声明 `max_output_tokens` 时的兜底输出预留
    /// （DSH reserved_completion_tokens）
    pub reserved_completion_tokens: usize,
    /// 保留最近上下文的 token 比例（DSH retain_ratio，默认 0.16）
    pub retain_ratio: f64,
    /// 摘要消息在历史中占位的 role
    pub summary_role: String,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            trigger_threshold: 0.8,
            headroom_tokens: 16_384,
            reserved_completion_tokens: 4_096,
            retain_ratio: 0.16,
            summary_role: "user".to_string(),
        }
    }
}

impl CompressionConfig {
    /// 计算触发压缩的有效 token 水位阈值（对齐 DSH thresholdTokens 逻辑）
    pub fn calculate_threshold_tokens(&self, context_window: usize) -> usize {
        calculate_threshold_tokens(context_window, self.reserved_completion_tokens, self)
    }

    /// 保留窗口的 token 预算（对齐 DSH `retainTokens = (cw - reserved) * retain_ratio`）。
    pub fn retain_tokens(&self, context_window: usize, reserved_completion_tokens: usize) -> usize {
        let usable = context_window.saturating_sub(reserved_completion_tokens);
        (usable as f64 * self.retain_ratio) as usize
    }
}

/// 计算触发压缩的有效 token 水位阈值（对齐 DSH thresholdTokens 逻辑）
pub fn calculate_threshold_tokens(
    context_window: usize,
    reserved_completion_tokens: usize,
    config: &CompressionConfig,
) -> usize {
    let message_budget = context_window.saturating_sub(reserved_completion_tokens);
    let pressure_budget = message_budget.saturating_sub(config.headroom_tokens);
    let ratio_threshold = (context_window as f64 * config.trigger_threshold) as usize;
    std::cmp::min(ratio_threshold, pressure_budget)
}

/// provider 未声明 context window 时的**保守输入预算**（token）。
///
/// 32k 的取值理由：主流模型里最小的常见窗口约 32k，按它设预算保证「任何声明缺失的
/// provider 都不会被我们发爆」；已知窗口的 provider 走精确预算，不受此常量影响。
/// （溢出是硬失败，宁可多裁一点。）
pub(crate) const FALLBACK_INPUT_BUDGET_TOKENS: usize = 32_000;

/// 上下文压力的唯一定义。
///
/// 两条记忆载体必须**一起**计入：native-tool-flow 会话的真实上下文是
/// `provider_native_transcript`（文本 `history` 不进 prompt），只估 `history`
/// 会让「一轮 90 个工具 hop」的增长永不触发压缩。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ContextPressure {
    /// 文本历史层估算 token
    pub history_tokens: usize,
    /// 原生 transcript 层估算 token
    pub transcript_tokens: usize,
    /// 上一请求 provider 真实上报的 input token（`provider/usage`）
    pub measured_input_tokens: Option<usize>,
}

impl ContextPressure {
    /// 会话内两条载体累加的估算值（不含 system / tools）。
    pub fn layered_tokens(&self) -> usize {
        self.history_tokens + self.transcript_tokens
    }

    /// 用于阈值比较的压力值。
    ///
    /// 真实计量（含 system prompt + 工具面）与分层估算（含尚未发出的新内容）
    /// 覆盖的口径不同，取二者较大者：任一侧已经到水位都说明该压缩了。
    pub fn effective_tokens(&self) -> usize {
        match self.measured_input_tokens {
            Some(measured) => measured.max(self.layered_tokens()),
            None => self.layered_tokens(),
        }
    }

    /// 从会话快照的两条载体构建压力值。
    pub fn from_layers(history: &[TurnHistoryMessage], transcript: &[Value]) -> Self {
        Self {
            history_tokens: estimate_history_tokens(history),
            transcript_tokens: estimate_native_transcript_tokens(transcript),
            measured_input_tokens: None,
        }
    }

    /// 附上上一请求的真实 input token（有则参与取大）。
    pub fn with_measured_input_tokens(mut self, measured: Option<usize>) -> Self {
        self.measured_input_tokens = measured;
        self
    }
}

/// provider 当前声明的输出预留：**优先**用 provider 的 `max_output_tokens`，
/// 未声明（0）时回退配置兜底。硬编码 2048 与配置里的 4096 曾长期分叉。
pub fn reserved_completion_tokens(provider: &ProviderManager, config: &CompressionConfig) -> usize {
    match provider.max_output_tokens() {
        0 => config.reserved_completion_tokens,
        declared => declared as usize,
    }
}

/// 压缩阈值（token）。provider 未声明 context window ⇒ `None`（无从判断）。
pub fn compression_threshold_tokens(
    provider: &ProviderManager,
    config: &CompressionConfig,
) -> Option<usize> {
    let context_window = provider.context_window_tokens()? as usize;
    Some(calculate_threshold_tokens(
        context_window,
        reserved_completion_tokens(provider, config),
        config,
    ))
}

/// 保留窗口的 token 预算。provider 未声明 context window ⇒ `None`。
pub fn retain_tokens_for(provider: &ProviderManager, config: &CompressionConfig) -> Option<usize> {
    let context_window = provider.context_window_tokens()? as usize;
    Some(config.retain_tokens(
        context_window,
        reserved_completion_tokens(provider, config),
    ))
}

/// 是否达到压缩水位。
pub fn should_compress_pressure(
    pressure: ContextPressure,
    provider: &ProviderManager,
    config: &CompressionConfig,
) -> bool {
    match compression_threshold_tokens(provider, config) {
        Some(threshold) => pressure.effective_tokens() > threshold,
        None => false,
    }
}

/// 估算整个历史消息的 token 总量
pub fn estimate_history_tokens(history: &[TurnHistoryMessage]) -> usize {
    history.iter().map(estimate_turn_message_tokens).sum()
}

/// 估算原生 transcript 的 token 总量（与上下文组装的估算同一口径）。
pub fn estimate_native_transcript_tokens(transcript: &[Value]) -> usize {
    transcript
        .iter()
        .map(estimate_native_message_tokens)
        .sum()
}

/// 压缩结果
#[derive(Clone, Debug)]
pub struct CompressionResult {
    /// 更新后的历史（摘要 + 保留的原始消息）
    pub history: Vec<TurnHistoryMessage>,
    /// 被压缩的原始消息数
    pub compressed_count: usize,
    /// 压缩后保留的原始消息数
    pub kept_count: usize,
    /// 保留窗口里的 user 轮数（原生 transcript 按同一轮数裁剪，保持两层口径一致）
    pub retained_user_turns: usize,
}

/// 将历史按 **token 预算** 拆分为"可压缩部分"和"保留原始部分"。
///
/// 返回 `(to_compress, to_keep)`：
/// - `to_keep`：从**某个 user 消息**开始的后缀，token 总量尽量贴近（不超过）
///   `retain_tokens`；
/// - `to_compress`：其之前的全部消息，交给摘要。
///
/// 两条硬约束：
/// 1. **切点必为 user 消息**：切在 assistant 中间会让保留窗口以 assistant 开头，
///    下一请求出现「assistant 开头」的非法序列；
/// 2. **最后一轮整轮保留**：单轮自身就超预算时不可再切（切了就不是"继续上一轮"），
///    宁可让窗口略超预算，由上下文组装的 input budget 兜底。
pub fn split_history_by_tokens(
    history: &[TurnHistoryMessage],
    retain_tokens: usize,
) -> (Vec<TurnHistoryMessage>, Vec<TurnHistoryMessage>) {
    match retained_window_start(history, retain_tokens) {
        Some(index) if index > 0 => (history[..index].to_vec(), history[index..].to_vec()),
        _ => (Vec::new(), history.to_vec()),
    }
}

/// 保留窗口的起始下标（必为 user 消息；`None` 表示无从切分）。
fn retained_window_start(history: &[TurnHistoryMessage], retain_tokens: usize) -> Option<usize> {
    if history.is_empty() {
        return None;
    }
    let mut accumulated = 0usize; // 当前候选窗口里「index 之后」的 token 数
    let mut best: Option<usize> = None;
    for index in (0..history.len()).rev() {
        let tokens = estimate_turn_message_tokens(&history[index]);
        if history[index].role == "user" {
            let window_tokens = accumulated + tokens;
            match best {
                None => {
                    // 最后一轮：无条件保留整轮（超预算也不切）。
                    best = Some(index);
                    if window_tokens > retain_tokens {
                        break;
                    }
                }
                Some(_) if window_tokens <= retain_tokens => best = Some(index),
                Some(_) => break,
            }
        }
        accumulated += tokens;
    }
    best
}

/// 保留窗口里的 user 轮数（供原生 transcript 按同一轮数裁剪）。
pub fn retained_user_turns(history: &[TurnHistoryMessage]) -> usize {
    history.iter().filter(|msg| msg.role == "user").count()
}

/// 按 **轮** 裁剪原生 transcript，保留最近 `keep_user_turns` 轮。
///
/// 以 user 消息为切点 ⇒ 天然满足「工具调用与结果成对闭合」：一轮从 user 开始，
/// 内部所有 `tool_calls` / `tool` 结果都在同一段里，切点两侧不会出现孤儿调用
/// （`sanitize_provider_native_transcript` 见到未闭合往返会清空整段 transcript）。
///
/// `keep_user_turns == 0` 或轮数不足 ⇒ 原样返回（不制造空 transcript）。
pub fn compact_native_transcript(transcript: &[Value], keep_user_turns: usize) -> Vec<Value> {
    if keep_user_turns == 0 || transcript.is_empty() {
        return transcript.to_vec();
    }
    let mut seen = 0usize;
    let mut start = 0usize;
    for (index, message) in transcript.iter().enumerate().rev() {
        if message.get("role").and_then(Value::as_str) == Some("user") {
            seen += 1;
            if seen == keep_user_turns {
                start = index;
                break;
            }
        }
    }
    if seen < keep_user_turns {
        // 轮数不足：整段保留（不做无意义裁剪）。
        return transcript.to_vec();
    }
    transcript[start..].to_vec()
}

/// 构建压缩系统 prompt
///
/// 这个 prompt 将作为 system 消息发送给 LLM，要求生成结构化摘要。
pub fn build_compression_system_prompt() -> String {
    r#"You are performing a context compression task. Create a structured summary of the following conversation for another AI agent to continue the work seamlessly.

Your summary MUST use the following XML format:

<session_summary>
  goal: What was the original task or request? What is the overall objective?
  progress: What key work has been completed? What was accomplished?
  decisions: What architectural decisions, technical choices, or user preferences were established?
  files: What files were created, modified, or examined? Include file paths and brief descriptions of changes.
  issues: What errors, bugs, or blockers were encountered? How were they resolved?
  pending: What explicitly requested tasks are still pending or incomplete?
  state: Exactly what was being worked on immediately before this summary? Where did the work leave off?
</session_summary>

Guidelines:
- Be specific and detailed. Include file paths, function names, error messages, and user preferences verbatim where relevant.
- Do not include code snippets — record what was done and why, not the full code.
- Focus on information that the next agent needs to continue without asking the user to repeat themselves.
- If the conversation is in Chinese, write the summary in Chinese. Otherwise, write in English."#.to_string()
}

/// 构建摘要前缀说明
///
/// 这个前缀将放在摘要之前，作为一条 user 消息注入历史，
/// 告诉后续模型这是前一轮对话的压缩摘要。
pub fn build_summary_prefix() -> String {
    r#"The following is a compressed summary of earlier conversation that was compacted to save context space. Use this to understand what has been done and continue the work without repeating steps.

<session_summary>"#.to_string()
}

/// 构建摘要后缀
pub fn build_summary_suffix() -> String {
    r#"</session_summary>

Continue directly from where the conversation left off based on the summary above. Do not acknowledge the summary or recap what happened — just pick up the work as if the break never happened."#.to_string()
}

/// 将 LLM 返回的摘要文本包装为历史消息
///
/// 从 LLM 响应中提取 `<session_summary>...</session_summary>` 内容，
/// 如果找不到标签，则使用原始响应文本。
pub fn wrap_summary_to_message(summary_text: &str) -> TurnHistoryMessage {
    let extracted = extract_summary_content(summary_text);
    let content = format!(
        "{}\n{}\n{}",
        build_summary_prefix(),
        extracted,
        build_summary_suffix(),
    );
    TurnHistoryMessage {
        role: "user".to_string(),
        content,
        ..Default::default()
    }
}

/// 从 LLM 响应中提取 `<session_summary>` 标签内的内容
fn extract_summary_content(response: &str) -> String {
    let start_tag = "<session_summary>";
    let end_tag = "</session_summary>";

    if let Some(start) = response.find(start_tag) {
        let content_start = start + start_tag.len();
        if let Some(end) = response[content_start..].find(end_tag) {
            return response[content_start..content_start + end].trim().to_string();
        }
    }

    // 没找到标签，返回原始响应
    response.trim().to_string()
}

/// 应用压缩结果到历史
///
/// 将可压缩部分替换为一条摘要消息，保留 `to_keep` 部分不变。
pub fn apply_compression(
    history: Vec<TurnHistoryMessage>,
    summary_message: TurnHistoryMessage,
    retain_tokens: usize,
) -> CompressionResult {
    let (to_compress, to_keep) = split_history_by_tokens(&history, retain_tokens);
    let compressed_count = to_compress.len();
    let kept_count = to_keep.len();
    let retained_user_turns = retained_user_turns(&to_keep);

    if compressed_count == 0 {
        return CompressionResult {
            history,
            compressed_count: 0,
            kept_count,
            retained_user_turns,
        };
    }

    let mut new_history = vec![summary_message];
    new_history.extend(to_keep);

    CompressionResult {
        history: new_history,
        compressed_count,
        kept_count,
        retained_user_turns,
    }
}

// ---- 内部辅助 ----

/// 估算单条 TurnHistoryMessage 的 token 数
fn estimate_turn_message_tokens(message: &TurnHistoryMessage) -> usize {
    4 + estimate_text_tokens(&message.content)
        + message
            .attachments
            .iter()
            .map(|a| {
                a.name
                    .as_deref()
                    .map(estimate_text_tokens)
                    .unwrap_or(0)
            })
            .sum::<usize>()
}

/// 估算文本的 token 数（粗略估算：每 4 **字符** ≈ 1 token）。
///
/// 按字符而非字节：中文一字三字节，按字节会把中文会话的估算放大三倍，
/// 导致压缩被远远提前触发。
pub(crate) fn estimate_text_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}

/// 估算单条 provider **原生**消息的 token 数（与上下文组装的预算裁剪同一口径）。
///
/// 家在本模块：压缩阈值与组装预算必须用同一个估算，否则「按 A 口径判定要压缩、
/// 按 B 口径算完仍超预算」会无限往返压缩。
pub(crate) fn estimate_native_message_tokens(message: &Value) -> usize {
    if let Some(content) = message.get("content").and_then(Value::as_str) {
        return 4 + estimate_text_tokens(content);
    }

    if let Some(content) = message.get("content").and_then(Value::as_array) {
        let content_tokens = content
            .iter()
            .map(estimate_native_content_block_tokens)
            .sum::<usize>();
        return 4 + content_tokens;
    }

    4 + estimate_text_tokens(&message.to_string())
}

/// 多模态内容块：图片按固定价（256 token）计，文本按字符数估。
fn estimate_native_content_block_tokens(block: &Value) -> usize {
    if block.get("type").and_then(Value::as_str) == Some("image")
        || block.get("type").and_then(Value::as_str) == Some("image_url")
    {
        return 256;
    }

    if let Some(text) = block.get("text").and_then(Value::as_str) {
        return estimate_text_tokens(text);
    }

    if let Some(text) = block
        .get("text")
        .and_then(|value| value.get("value"))
        .and_then(Value::as_str)
    {
        return estimate_text_tokens(text);
    }

    estimate_text_tokens(&block.to_string())
}

/// 压缩边界推导：`history/squash.base_seq` = **最后一个被压缩掉的折叠消息的 seq**。
///
/// 折叠语义是「保留 seq > base_seq」，因此边界只能由「保留窗口的第一条消息」反推。
/// 压缩后历史 = 摘要 + 保留窗口，而保留窗口是折叠结果的**尾部子序列**，所以用后缀
/// 对齐（role + content 逐条相等）从尾部找最长匹配即可。
///
/// 为什么要推导而不能「用当前水位当边界」：水位会把保留窗口一起压掉（折叠后只剩
/// 摘要）；为什么不能「靠 history 的 turn_id」：`append_turn` 落历史时不写 turn_id
/// （`TurnHistoryMessage::default()`），历史消息普遍没有轮次归属。
///
/// 退化行为（宁可多压缩，绝不复活被压缩内容）：
/// - 折叠结果为空 ⇒ 0（边界在地板，摘要照旧注入）；
/// - 保留窗口与折叠尾部一条都对不上（历史被外部改写 / 已按紧急上限裁剪）⇒ 返回最后
///   一条折叠消息的 seq（只留摘要）。这比让 400 条旧历史整段复活安全：摘要里已有被压缩
///   内容的结论，复活则会把上下文直接顶到预算上限。
pub(crate) fn derive_squash_base_seq(
    folded: &[(u64, TurnHistoryMessage)],
    retained: &[TurnHistoryMessage],
) -> u64 {
    if folded.is_empty() {
        return 0;
    }
    let mut matched = 0usize;
    while matched < retained.len() && matched < folded.len() {
        let kept = &retained[retained.len() - 1 - matched];
        let (_, candidate) = &folded[folded.len() - 1 - matched];
        if kept.role == candidate.role && kept.content == candidate.content {
            matched += 1;
        } else {
            break;
        }
    }
    if matched >= folded.len() {
        // 折叠结果全部落在保留窗口内（没有任何消息被压掉）：边界压到折叠结果之前，
        // 一条都不裁，摘要仍由 squash 事件注入。用**最小** seq 而不是首元素 seq——
        // 摘要被插在头部（seq 最大），用首元素会把整个历史压掉。
        return folded
            .iter()
            .map(|(seq, _)| *seq)
            .min()
            .unwrap_or(0)
            .saturating_sub(1);
    }
    folded[folded.len() - 1 - matched].0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_msg(role: &str, content: &str) -> TurnHistoryMessage {
        TurnHistoryMessage {
            role: role.to_string(),
            content: content.to_string(),
            ..Default::default()
        }
    }

    /// 构造 n 轮对话，每轮固定内容长度（token 量可预测）。
    fn make_turns(turns: usize, content_chars: usize) -> Vec<TurnHistoryMessage> {
        let mut history = Vec::new();
        for i in 0..turns {
            history.push(make_msg(
                "user",
                &format!("q{i}-{}", "x".repeat(content_chars)),
            ));
            history.push(make_msg(
                "assistant",
                &format!("a{i}-{}", "y".repeat(content_chars)),
            ));
        }
        history
    }

    #[test]
    fn test_estimate_text_tokens() {
        assert_eq!(estimate_text_tokens("hello"), 2);
        assert_eq!(estimate_text_tokens(""), 0);
        assert_eq!(estimate_text_tokens("a"), 1);
        assert_eq!(estimate_text_tokens("abcd"), 1);
        assert_eq!(estimate_text_tokens("abcde"), 2);
        // 中文按字符计：4 字 = 1 token（按字节会算成 3）。
        assert_eq!(estimate_text_tokens("压缩上下文"), 2);
    }

    #[test]
    fn test_split_history_by_tokens_keeps_recent_turns() {
        // 每条内容 20+2 字符 ⇒ 6 token，加 4 底盘 = 10 token/条，一轮 20 token。
        let history = make_turns(10, 20);
        let (to_compress, to_keep) = split_history_by_tokens(&history, 60);

        assert_eq!(to_keep.len(), 6, "保留 3 轮（20 token/轮 × 3 = 60）");
        assert_eq!(to_keep[0].role, "user", "切点必须是 user 消息");
        assert!(to_keep[0].content.starts_with("q7"));
        assert_eq!(to_compress.len(), 14);
        assert!(to_compress[0].content.starts_with("q0"));
    }

    #[test]
    fn test_split_history_by_tokens_keeps_whole_last_turn_when_over_budget() {
        let history = make_turns(5, 400);
        // 预算远小于最后一轮：整轮保留，绝不切在轮中间。
        let (to_compress, to_keep) = split_history_by_tokens(&history, 1);
        assert_eq!(to_keep.len(), 2);
        assert_eq!(to_keep[0].role, "user");
        assert!(to_keep[0].content.starts_with("q4"));
        assert_eq!(to_compress.len(), 8);
    }

    #[test]
    fn test_split_history_by_tokens_never_splits_everything() {
        let history = make_turns(3, 8);
        let (to_compress, to_keep) = split_history_by_tokens(&history, usize::MAX / 2);
        assert!(to_compress.is_empty(), "预算足够时不压缩");
        assert_eq!(to_keep.len(), history.len());
    }

    #[test]
    fn test_split_history_by_tokens_empty_history() {
        let (to_compress, to_keep) = split_history_by_tokens(&[], 100);
        assert!(to_compress.is_empty());
        assert!(to_keep.is_empty());
    }

    #[test]
    fn test_calculate_threshold_tokens() {
        let config = CompressionConfig {
            trigger_threshold: 0.8,
            headroom_tokens: 16_000,
            ..Default::default()
        };
        // context_window = 100_000, reserved = 2_000
        // ratio_threshold = 80_000
        // message_budget = 98_000, pressure_budget = 98_000 - 16_000 = 82_000
        // min(80_000, 82_000) = 80_000
        let threshold = calculate_threshold_tokens(100_000, 2_000, &config);
        assert_eq!(threshold, 80_000);

        // 当 headroom 很大时受 pressure_budget 限制：
        let high_headroom_config = CompressionConfig {
            trigger_threshold: 0.8,
            headroom_tokens: 30_000,
            ..Default::default()
        };
        // message_budget = 98_000, pressure_budget = 68_000
        // min(80_000, 68_000) = 68_000
        let threshold2 = calculate_threshold_tokens(100_000, 2_000, &high_headroom_config);
        assert_eq!(threshold2, 68_000);
    }

    #[test]
    fn test_retain_tokens_follows_ratio_of_usable_window() {
        let config = CompressionConfig::default();
        // (128_000 - 4_096) * 0.16 = 19_824.64 ⇒ 截断 19_824
        assert_eq!(config.retain_tokens(128_000, 4_096), 19_824);
    }

    #[test]
    fn test_context_pressure_prefers_larger_side() {
        // 分层估算 > 真实计量 ⇒ 认分层（还有未发出的新内容）。
        let layered_heavy = ContextPressure {
            history_tokens: 900,
            transcript_tokens: 6_000,
            measured_input_tokens: Some(2_000),
        };
        assert_eq!(layered_heavy.effective_tokens(), 6_900);

        // 真实计量 > 分层（system + 工具面不在分层里）⇒ 认真实。
        let measured_heavy = ContextPressure {
            history_tokens: 100,
            transcript_tokens: 200,
            measured_input_tokens: Some(50_000),
        };
        assert_eq!(measured_heavy.effective_tokens(), 50_000);

        // 无真实计量 ⇒ 纯分层。
        let blind = ContextPressure {
            history_tokens: 100,
            transcript_tokens: 200,
            measured_input_tokens: None,
        };
        assert_eq!(blind.effective_tokens(), 300);
    }

    #[test]
    fn test_extract_summary_content() {
        let response = r#"Some preamble
<session_summary>
  goal: Build a login page
  progress: Created the UI components
</session_summary>
Some trailing text"#;
        let extracted = extract_summary_content(response);
        assert!(extracted.contains("goal: Build a login page"));
        assert!(extracted.contains("progress: Created the UI components"));
    }

    #[test]
    fn test_extract_summary_content_no_tags() {
        let response = "Simple text without tags";
        assert_eq!(extract_summary_content(response), "Simple text without tags");
    }

    #[test]
    fn test_apply_compression() {
        let history = make_turns(6, 20);
        let summary_msg = make_msg("user", "summary content here");
        // 每轮 20 token；预算 60 ⇒ 保留 3 轮。
        let result = apply_compression(history, summary_msg, 60);

        assert_eq!(result.compressed_count, 6); // 前 3 轮 = 6 条被压缩
        assert_eq!(result.kept_count, 6); // 后 3 轮 = 6 条保留
        assert_eq!(result.retained_user_turns, 3);
        assert_eq!(result.history.len(), 7); // 1 摘要 + 6 保留
        assert_eq!(result.history[0].content, "summary content here");
    }

    #[test]
    fn test_apply_compression_without_compressible_prefix_is_noop() {
        let history = make_turns(2, 8);
        let result = apply_compression(history.clone(), make_msg("user", "s"), 1_000);
        assert_eq!(result.compressed_count, 0);
        assert_eq!(result.history.len(), history.len(), "不压缩时历史逐条不变");
    }

    fn native(role: &str, text: &str) -> Value {
        serde_json::json!({"role": role, "content": text})
    }

    fn native_turn(index: usize) -> Vec<Value> {
        vec![
            native("user", &format!("u{index}")),
            serde_json::json!({
                "role": "assistant",
                "content": "",
                "tool_calls": [{"id": format!("c{index}"), "type": "function",
                                "function": {"name": "noop", "arguments": "{}"}}],
            }),
            serde_json::json!({
                "role": "tool", "tool_call_id": format!("c{index}"), "content": "ok"
            }),
            native("assistant", &format!("done{index}")),
        ]
    }

    #[test]
    fn test_compact_native_transcript_keeps_whole_recent_turns() {
        let mut transcript = Vec::new();
        for index in 0..4 {
            transcript.extend(native_turn(index));
        }
        let compacted = compact_native_transcript(&transcript, 2);

        // 保留最近 2 轮 = 8 条，且必须从 user 开始（切点闭合，不留孤儿 tool 往返）。
        assert_eq!(compacted.len(), 8);
        assert_eq!(compacted[0]["role"], "user");
        assert_eq!(compacted[0]["content"], "u2");
        assert_eq!(compacted[7]["content"], "done3");
    }

    #[test]
    fn test_compact_native_transcript_noop_cases() {
        let transcript = [native("user", "u0"), native("assistant", "a0")];
        // 轮数为 0 ⇒ 不裁（避免把 transcript 清空）。
        assert_eq!(compact_native_transcript(&transcript, 0).len(), 2);
        // 保留轮数超过实际轮数 ⇒ 原样返回。
        assert_eq!(compact_native_transcript(&transcript, 9).len(), 2);
        // 空 transcript ⇒ 空。
        assert!(compact_native_transcript(&[], 3).is_empty());
    }

    #[test]
    fn test_retained_user_turns_counts_only_user() {
        let history = vec![
            make_msg("user", "u0"),
            make_msg("assistant", "a0"),
            make_msg("user", "u1"),
            make_msg("assistant", "a1"),
        ];
        assert_eq!(retained_user_turns(&history), 2);
        assert_eq!(retained_user_turns(&[]), 0);
    }

    #[test]
    fn test_estimate_native_transcript_tokens_counts_tool_payloads() {
        let transcript = native_turn(0);
        let tokens = estimate_native_transcript_tokens(&transcript);
        // 四条消息各至少 4 token 底盘 ⇒ 必大于 16。
        assert!(tokens > 16, "工具往返必须计入 transcript 压力: {tokens}");
    }

    #[test]
    fn test_derive_squash_base_seq_follows_retained_suffix() {
        let folded = vec![
            (10u64, make_msg("user", "u0")),
            (11, make_msg("assistant", "a0")),
            (12, make_msg("user", "u1")),
            (13, make_msg("assistant", "a1")),
        ];
        // 保留最后两条 ⇒ 边界 = 最后一个被压掉的消息（11），折叠后保留 12/13。
        assert_eq!(
            derive_squash_base_seq(
                &folded,
                &[make_msg("user", "u1"), make_msg("assistant", "a1")]
            ),
            11
        );
        // 全部保留（没有消息被压掉）⇒ 边界压到折叠结果之前，一条都不裁。
        let all_retained: Vec<TurnHistoryMessage> =
            folded.iter().map(|(_, message)| message.clone()).collect();
        assert_eq!(derive_squash_base_seq(&folded, &all_retained), 9);
        // 一条都不保留 ⇒ 边界 = 最后一条（只留摘要）。
        assert_eq!(derive_squash_base_seq(&folded, &[]), 13);
        // 保留窗口与折叠结果对不上（历史被外部改写）⇒ 退化为「只留摘要」，绝不复活。
        assert_eq!(
            derive_squash_base_seq(&folded, &[make_msg("user", "外部改写")]),
            13
        );
        // 折叠结果为空 ⇒ 边界在地板。
        assert_eq!(derive_squash_base_seq(&[], &[make_msg("user", "x")]), 0);
    }

    /// 默认压缩预算 = 256K 窗口：水位必须落在「远早于 256K 硬顶」的区间。
    ///
    /// 旧默认 1M 会让水位停在 ~800K，而主流模型真实窗口是 128K~256K ⇒ 压缩永远晚于
    /// 上游溢出（硬失败）触发，`RequiresRequestMutation` 之前直接终止本轮。
    #[test]
    fn test_default_context_window_is_256k_and_triggers_before_overflow() {
        let window = crate::agent::config::DEFAULT_CONTEXT_WINDOW_TOKENS as usize;
        assert_eq!(window, 262_144, "默认窗口真值必须是 256K");
        let threshold = calculate_threshold_tokens(window, 64_000, &CompressionConfig::default());
        assert!(
            threshold > 160_000 && threshold < 210_000,
            "默认水位应在 160K~210K（含 64K 输出预留时为 181_760），实际 {threshold}"
        );
        assert!(threshold < window);
    }
}
