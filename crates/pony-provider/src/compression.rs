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

use crate::provider::ProviderManager;
use pony_protocol::session_types::TurnHistoryMessage;
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
pub const FALLBACK_INPUT_BUDGET_TOKENS: usize = 32_000;

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
    split_history_with_transcript(history, &[], retain_tokens)
}

/// 将历史与原生转录一起按 **token 预算** 拆分为「可压缩部分」与「保留原始部分」。
///
/// 原生工具流会话的主要体积落在 `provider_native_transcript`（工具调用 + 思考链），
/// 文本 `history` 往往只有数千 token；切分只看 `history` 会让重工具会话长期返回
/// `history_too_short` 而无法触发压缩。当 `transcript` 非空时，保留最近 `k` 轮的代价
/// 取 `max(history_tail_tokens, transcript_tail_tokens)`，与 `compact_native_transcript`
/// 保持同一轮次边界口径。
pub fn split_history_with_transcript(
    history: &[TurnHistoryMessage],
    transcript: &[Value],
    retain_tokens: usize,
) -> (Vec<TurnHistoryMessage>, Vec<TurnHistoryMessage>) {
    match retained_window_start_with_transcript(history, transcript, retain_tokens) {
        Some(index) if index > 0 => (history[..index].to_vec(), history[index..].to_vec()),
        _ => (Vec::new(), history.to_vec()),
    }
}

/// 估算原生转录末尾 `keep_user_turns` 轮的 token 占用（按 `role == "user"` 切轮）。
fn transcript_tail_tokens_for_turns(transcript: &[Value], keep_user_turns: usize) -> usize {
    if transcript.is_empty() || keep_user_turns == 0 {
        return 0;
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
        start = 0;
    }
    estimate_native_transcript_tokens(&transcript[start..])
}

/// 保留窗口的起始下标（必为 user 消息；`None` 表示无从切分）。
fn retained_window_start_with_transcript(
    history: &[TurnHistoryMessage],
    transcript: &[Value],
    retain_tokens: usize,
) -> Option<usize> {
    if history.is_empty() {
        return None;
    }
    let mut accumulated_history = 0usize; // 当前候选窗口里「index 之后」的 history token 数
    let mut user_turns_kept = 0usize;
    let mut best: Option<usize> = None;
    for index in (0..history.len()).rev() {
        let tokens = estimate_turn_message_tokens(&history[index]);
        if history[index].role == "user" {
            user_turns_kept += 1;
            let history_window_tokens = accumulated_history + tokens;
            let transcript_window_tokens =
                transcript_tail_tokens_for_turns(transcript, user_turns_kept);
            let window_tokens = history_window_tokens.max(transcript_window_tokens);
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
        accumulated_history += tokens;
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
    apply_compression_with_transcript(history, &[], summary_message, retain_tokens)
}

/// 与 `split_history_with_transcript` 同源的应用入口：保证计划切分与回写切分使用同一口径。
pub fn apply_compression_with_transcript(
    history: Vec<TurnHistoryMessage>,
    transcript: &[Value],
    summary_message: TurnHistoryMessage,
    retain_tokens: usize,
) -> CompressionResult {
    let (to_compress, to_keep) = split_history_with_transcript(&history, transcript, retain_tokens);
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

/// 溢出自救兜底：在保持「user 锚点 + 已闭合工具往返」前提下，把原生转录压到目标 token 预算内。
///
/// 适用场景：当整段上下文仅剩最后一轮（`history_too_short`）或摘要调用自身不可用时，
/// 先就地修剪超长 tool 输出，再从最近一轮 `user` 锚点之后按完整 `[assistant(tool_calls), tool...]`
/// 原子单元从旧到新丢弃，保留最近闭合的工具步骤，绝不产生孤儿 `tool` 或缺 `tool_result`。
pub fn trim_native_transcript_to_token_budget(transcript: &[Value], max_tokens: usize) -> Vec<Value> {
    if transcript.is_empty() {
        return Vec::new();
    }
    let mut working = transcript.to_vec();
    crate::tool_pruner::prune_native_messages_in_place(&mut working);
    if estimate_native_transcript_tokens(&working) <= max_tokens {
        return working;
    }

    let anchor_idx = working
        .iter()
        .rposition(|msg| msg.get("role").and_then(Value::as_str) == Some("user"));
    let (anchor, tail) = match anchor_idx {
        Some(idx) => (Some(working[idx].clone()), &working[idx + 1..]),
        None => (None, working.as_slice()),
    };

    // 将 tail 切成协议不可拆分的原子单元（带 tool_calls 的 assistant 必须与后续连续 tool 同进同退）。
    let mut units: Vec<Vec<Value>> = Vec::new();
    let mut i = 0usize;
    while i < tail.len() {
        let has_tool_calls = tail[i].get("role").and_then(Value::as_str) == Some("assistant")
            && tail[i]
                .get("tool_calls")
                .and_then(Value::as_array)
                .is_some_and(|calls| !calls.is_empty());
        if has_tool_calls {
            let mut end = i + 1;
            while end < tail.len()
                && tail[end].get("role").and_then(Value::as_str) == Some("tool")
            {
                end += 1;
            }
            units.push(tail[i..end].to_vec());
            i = end;
        } else if tail[i].get("role").and_then(Value::as_str) == Some("tool") {
            // 孤儿 tool 消息丢弃，不进入自救结果。
            i += 1;
        } else {
            units.push(vec![tail[i].clone()]);
            i += 1;
        }
    }

    let anchor_tokens = anchor
        .as_ref()
        .map_or(0, estimate_native_message_tokens);
    let mut remaining = max_tokens.saturating_sub(anchor_tokens);
    let mut kept_units: Vec<Vec<Value>> = Vec::new();
    for unit in units.into_iter().rev() {
        let unit_tokens = estimate_native_transcript_tokens(&unit);
        if unit_tokens <= remaining {
            remaining -= unit_tokens;
            kept_units.push(unit);
        } else {
            break;
        }
    }
    kept_units.reverse();

    let mut result = Vec::new();
    if let Some(user_msg) = anchor {
        result.push(user_msg);
    }
    for unit in kept_units {
        result.extend(unit);
    }
    if crate::tool_pruner::needs_tool_roundtrip_repair(&result) {
        crate::tool_pruner::recover_incomplete_tool_roundtrips(&result)
    } else {
        result
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

pub use pony_protocol::estimate::{derive_squash_base_seq, estimate_text_tokens};

/// 估算单条 provider **原生**消息的 token 数（与上下文组装的预算裁剪同一口径）。
///
/// 家在本模块：压缩阈值与组装预算必须用同一个估算，否则「按 A 口径判定要压缩、
/// 按 B 口径算完仍超预算」会无限往返压缩。
///
/// 除 `content` 外同时计入 `reasoning_content` / `reasoning` / `tool_calls` / `tool_call_id`，
/// 避免 DeepSeek 等推理+重工具会话因只算正文而系统性低估体积。
pub fn estimate_native_message_tokens(message: &Value) -> usize {
    let overhead = 4;
    let extra = estimate_native_message_extra_tokens(message);
    if let Some(content) = message.get("content").and_then(Value::as_str) {
        return overhead + estimate_text_tokens(content) + extra;
    }

    if let Some(content) = message.get("content").and_then(Value::as_array) {
        let content_tokens = content
            .iter()
            .map(estimate_native_content_block_tokens)
            .sum::<usize>();
        return overhead + content_tokens + extra;
    }

    overhead + estimate_text_tokens(&message.to_string()) + extra
}

/// 除 `content` 之外会进请求体的字段（`reasoning_content` / `tool_calls` / `tool_call_id`）。
///
/// 只在字段**存在**时计入，故对不含这些字段的历史消息零影响（既有预算断言不变）。
fn estimate_native_message_extra_tokens(message: &Value) -> usize {
    let mut extra = 0;
    for key in ["reasoning_content", "reasoning"] {
        if let Some(text) = message.get(key).and_then(Value::as_str) {
            extra += estimate_text_tokens(text);
        }
    }
    if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
        extra += calls
            .iter()
            .map(|call| estimate_text_tokens(&call.to_string()))
            .sum::<usize>();
    }
    if let Some(call_id) = message.get("tool_call_id").and_then(Value::as_str) {
        extra += estimate_text_tokens(call_id);
    }
    extra
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

    /// B1795506800002：`reasoning_content` 与 `tool_calls` 必须计入同一条消息的估算。
    ///
    /// 缺陷现场：旧实现只算 `content`，而 DeepSeek 等推理模型的思考正文可达正文同量级、
    /// 一个 patch 类工具调用的参数 JSON 上万字符 —— 漏算让预算裁剪与压缩阈值系统性低估，
    /// 是「token 预算看着没超、实际请求体已 300KB+」的成因之一。
    #[test]
    fn test_estimate_native_message_tokens_counts_reasoning_and_tool_calls() {
        let content_only = serde_json::json!({
            "role": "assistant",
            "content": "x".repeat(4_000),
        });
        let base = estimate_native_message_tokens(&content_only);

        // 同 content + 4k 字符思考正文 ⇒ 至少多 1000 token（chars/4 口径）。
        let with_reasoning = serde_json::json!({
            "role": "assistant",
            "content": "x".repeat(4_000),
            "reasoning_content": "r".repeat(4_000),
        });
        let with_reasoning_tokens = estimate_native_message_tokens(&with_reasoning);
        assert!(
            with_reasoning_tokens >= base + 1_000,
            "reasoning_content 必须计入估算：base={base} with_reasoning={with_reasoning_tokens}"
        );

        // 同 content + 一个参数 JSON 很大的工具调用 ⇒ 必须显著抬高。
        let with_tool_calls = serde_json::json!({
            "role": "assistant",
            "content": "x".repeat(4_000),
            "tool_calls": [{
                "id": "call_1",
                "type": "function",
                "function": {
                    "name": "workspace.write_file",
                    "arguments": serde_json::json!({ "content": "y".repeat(4_000) }).to_string(),
                }
            }],
        });
        let with_tool_calls_tokens = estimate_native_message_tokens(&with_tool_calls);
        assert!(
            with_tool_calls_tokens >= base + 1_000,
            "tool_calls 参数 JSON 必须计入估算：base={base} with_tool_calls={with_tool_calls_tokens}"
        );

        // 无这些字段的消息估算必须逐字不变（既有预算断言不得漂移）。
        let plain = serde_json::json!({ "role": "user", "content": "hello world" });
        assert_eq!(
            estimate_native_message_tokens(&plain),
            4 + estimate_text_tokens("hello world"),
            "不含 reasoning/tool_calls 的消息必须保持原口径"
        );
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

    /// 默认压缩预算 = 1M 窗口（2026-11-20 用户裁决）：水位必须仍严格早于窗口硬顶。
    ///
    /// 历史包袱：2026-11-10 曾把默认窗口设为 256K，理由是「1M 默认 ⇒ 水位 ~800K，而主流
    /// 模型真实窗口 128K~256K ⇒ 压缩永远晚于上游溢出」。现在默认值回到 1M 后，这条风险
    /// 的处置方式改为**逐模型声明真实窗口**（TUI `/models` 的 `w`，声明值优先于默认值），
    /// 而不是压低全局默认：真实窗口未知时按 1M 计，声明后就按真实容量推导水位。
    #[test]
    fn test_default_context_window_is_1m_and_triggers_before_overflow() {
        let window = crate::config::DEFAULT_CONTEXT_WINDOW_TOKENS as usize;
        assert_eq!(window, 1_000_000, "默认窗口真值必须是 1M（十进制口径）");
        let threshold = calculate_threshold_tokens(window, 64_000, &CompressionConfig::default());
        assert_eq!(
            threshold, 800_000,
            "1M 默认下水位取触发比例 0.8（压力预算 919_616 更晚），实际 {threshold}"
        );
        assert!(threshold < window, "水位必须严格早于窗口硬顶");
    }
}
