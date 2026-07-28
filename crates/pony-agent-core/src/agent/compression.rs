//! 会话上下文压缩模块。
//!
//! 职责：
//! - 判断是否需要压缩（基于阈值触发）
//! - 将历史拆分为"可压缩部分"和"保留原始部分"
//! - 构建压缩 prompt 供 LLM 生成摘要
//! - 将摘要应用回历史，替换旧消息
//!
//! 设计原则：高内聚低耦合。
//! - 本模块不依赖 runtime 内部细节
//! - 不直接调用 LLM（由调用方传入摘要文本）
//! - 所有函数为纯数据变换

use crate::agent::provider::ProviderManager;
use crate::agent::session::TurnHistoryMessage;

/// 压缩配置
#[derive(Clone, Debug)]
pub struct CompressionConfig {
    /// 触发压缩的阈值，占 context window 的比例（默认 0.8 = 80%）
    pub trigger_threshold: f64,
    /// 保留原始消息的最近 turn 数（默认 10，业界研究表明 10 轮是最佳平衡点）
    pub keep_recent_turns: usize,
    /// 摘要消息在历史中占位的 role
    pub summary_role: String,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            trigger_threshold: 0.8,
            keep_recent_turns: 10,
            summary_role: "user".to_string(),
        }
    }
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
}

/// 判断是否需要压缩
///
/// 当历史消息的估算 token 总数超过 `context_window * trigger_threshold` 时返回 true。
pub fn should_compress(
    history: &[TurnHistoryMessage],
    provider: &ProviderManager,
    config: &CompressionConfig,
) -> bool {
    let Some(context_window) = provider.context_window_tokens() else {
        return false;
    };
    let threshold_tokens = (context_window as f64 * config.trigger_threshold) as usize;
    let total_tokens: usize = history.iter().map(estimate_turn_message_tokens).sum();
    total_tokens > threshold_tokens
}

/// 将历史拆分为"可压缩部分"和"保留原始部分"
///
/// 返回 `(to_compress, to_keep)`：
/// - `to_compress`：最旧的消息，将被压缩为摘要
/// - `to_keep`：最近的 `keep_recent_turns` 轮消息，保留原始内容
///
/// 拆分以 turn 边界对齐（从尾部向前数 `keep_recent_turns` 个 user 消息）。
pub fn split_history(
    history: &[TurnHistoryMessage],
    config: &CompressionConfig,
) -> (Vec<TurnHistoryMessage>, Vec<TurnHistoryMessage>) {
    if history.len() <= config.keep_recent_turns * 2 {
        // 历史太短，不值得压缩
        return (Vec::new(), history.to_vec());
    }

    // 从尾部向前找到第 keep_recent_turns 个 user 消息的位置
    let mut user_count = 0;
    let split_idx = history.iter().rposition(|msg| {
        if msg.role == "user" {
            user_count += 1;
            user_count == config.keep_recent_turns
        } else {
            false
        }
    });

    match split_idx {
        Some(idx) => {
            let to_compress = history[..idx].to_vec();
            let to_keep = history[idx..].to_vec();
            (to_compress, to_keep)
        }
        None => {
            // 不足 keep_recent_turns 个 user 消息，全部保留
            (Vec::new(), history.to_vec())
        }
    }
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
/// 将 `to_compress` 部分替换为一条摘要消息，保留 `to_keep` 部分不变。
pub fn apply_compression(
    history: Vec<TurnHistoryMessage>,
    summary_message: TurnHistoryMessage,
    config: &CompressionConfig,
) -> CompressionResult {
    let (to_compress, to_keep) = split_history(&history, config);
    let compressed_count = to_compress.len();
    let kept_count = to_keep.len();

    if compressed_count == 0 {
        return CompressionResult {
            history,
            compressed_count: 0,
            kept_count,
        };
    }

    let mut new_history = vec![summary_message];
    new_history.extend(to_keep);

    CompressionResult {
        history: new_history,
        compressed_count,
        kept_count,
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

/// 估算文本的 token 数（粗略估算：每 4 字符 ≈ 1 token）
fn estimate_text_tokens(text: &str) -> usize {
    (text.len() + 3) / 4
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

    #[test]
    fn test_estimate_text_tokens() {
        assert_eq!(estimate_text_tokens("hello"), 2);
        assert_eq!(estimate_text_tokens(""), 0);
        assert_eq!(estimate_text_tokens("a"), 1);
        assert_eq!(estimate_text_tokens("abcd"), 1);
        assert_eq!(estimate_text_tokens("abcde"), 2);
    }

    #[test]
    fn test_split_history_keeps_recent_turns() {
        let mut history = Vec::new();
        // 10 轮对话 = 20 条消息（user + assistant）
        for i in 0..10 {
            history.push(make_msg("user", &format!("question {}", i)));
            history.push(make_msg("assistant", &format!("answer {}", i)));
        }

        let config = CompressionConfig {
            keep_recent_turns: 3,
            ..Default::default()
        };
        let (to_compress, to_keep) = split_history(&history, &config);

        // 应保留最后 3 轮 = 6 条消息
        assert_eq!(to_keep.len(), 6);
        assert_eq!(to_keep[0].content, "question 7");
        assert_eq!(to_compress.len(), 14);
        assert_eq!(to_compress[0].content, "question 0");
    }

    #[test]
    fn test_split_history_short_history() {
        let mut history = Vec::new();
        for i in 0..3 {
            history.push(make_msg("user", &format!("q {}", i)));
            history.push(make_msg("assistant", &format!("a {}", i)));
        }

        let config = CompressionConfig {
            keep_recent_turns: 5,
            ..Default::default()
        };
        let (to_compress, to_keep) = split_history(&history, &config);

        // 历史太短，全部保留
        assert!(to_compress.is_empty());
        assert_eq!(to_keep.len(), 6);
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
        let mut history = Vec::new();
        for i in 0..6 {
            history.push(make_msg("user", &format!("q {}", i)));
            history.push(make_msg("assistant", &format!("a {}", i)));
        }

        let config = CompressionConfig {
            keep_recent_turns: 2,
            ..Default::default()
        };
        let summary_msg = make_msg("user", "summary content here");
        let result = apply_compression(history, summary_msg, &config);

        assert_eq!(result.compressed_count, 8); // 前 4 轮 = 8 条被压缩
        assert_eq!(result.kept_count, 4); // 后 2 轮 = 4 条保留
        assert_eq!(result.history.len(), 5); // 1 摘要 + 4 保留
        assert_eq!(result.history[0].content, "summary content here");
    }
}