// compaction_ops: 手动上下文压缩（斜杠命令 /compact 的 Rust 核心实现）。
//
// 复用 `agent/compression.rs` 的纯数据变换；本模块只负责编排与 IO 纪律：
// - **锁纪律**（契约 C-1 语义 1 / 审查 F2a）：网络调用期间**不持有** `sessions` 读写锁，
//   控制面也不得在持有 `runtime` 读锁时发起调用 ⇒ 流程拆成「短读快照 → 无锁摘要 → 短写回写」。
// - **时长上界**（审查 F2b）：摘要调用在独立线程执行，调用方以**墙钟 deadline** 兜底
//   （半开连接也会按时返回 Err，TUI 不会永久卡在「正在压缩上下文…」）；真流式 provider
//   同时把同一 deadline 接进 `ProviderStreamGuard`，可协作取消。
// - **空摘要**（审查 F3）：摘要为空/仅空白一律 `Err`，绝不用空摘要替换历史报成功。
use super::*;
use crate::agent::compression::{
    apply_compression, build_compression_system_prompt, build_summary_prefix, build_summary_suffix,
    estimate_history_tokens, should_compress, split_history, wrap_summary_to_message,
    CompressionConfig,
};
use crate::agent::provider::ProviderStreamGuard;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::mpsc;
use std::time::Duration;

/// 手动压缩摘要调用的墙钟上界（秒）。
///
/// 取值依据：手动压缩是**用户主动等待并阻塞交互**的前台动作，必须显著短于 turn 的
/// 缺省流式上界（`ProviderStreamGuard::DEFAULT_MAX_ELAPSED` = 30min）；provider 侧建连预算
/// 15s、读空闲 180s，60s 既覆盖正常摘要生成（`max_output_tokens = 2048`），又不让半开连接
/// 把 TUI 挂在「正在压缩上下文…」。
pub const COMPACTION_TIMEOUT_SECS: u64 = 60;

/// 超时错误码（调用方可按前缀辨识；不是静默截断，也不谎报成功）。
pub const COMPACTION_TIMEOUT_ERROR_CODE: &str = "compaction_deadline_exceeded";

/// 空摘要错误码（审查 F3：摘要为空 ⇒ 中止，不得改写历史）。
pub const COMPACTION_EMPTY_SUMMARY_ERROR_CODE: &str = "empty_summary";

/// 历史在压缩期间被并发改写（非同源追加）时的错误码：放弃写入而不是覆盖丢数据。
pub const COMPACTION_HISTORY_CHANGED_ERROR_CODE: &str = "compaction_history_changed";

/// 手动压缩结果（前端 / Tauri 序列化契约，camelCase）。
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualCompactionOutcome {
    pub compacted: bool,
    /// "compacted" | "history_too_short" | "below_threshold"
    pub reason: String,
    pub compressed_count: usize,
    pub kept_count: usize,
    pub before_tokens: usize,
    pub after_tokens: usize,
}

impl ManualCompactionOutcome {
    fn untouched(reason: &str, before_tokens: usize, kept_count: usize) -> Self {
        Self {
            compacted: false,
            reason: reason.to_string(),
            compressed_count: 0,
            kept_count,
            before_tokens,
            after_tokens: before_tokens,
        }
    }
}

/// 契约 C-7 / 审查 F4：把压缩结果映射为用户可见的 System 消息文案（纯函数，TUI/CLI 共用）。
///
/// 语义（可被测试直接断言）：
/// - 成功：含 `compressed_count` / `kept_count`，且含"已压缩"；
/// - `history_too_short` / `below_threshold`：含"无需压缩"，**不得**出现"已压缩"；
/// - `Err`：含错误摘要，**不得**出现"已压缩"。
pub fn compact_outcome_message(result: &Result<ManualCompactionOutcome, String>) -> String {
    match result {
        Ok(outcome) if outcome.compacted => format!(
            "已压缩 {} 条消息，保留 {} 条（tokens {} → {}）",
            outcome.compressed_count,
            outcome.kept_count,
            outcome.before_tokens,
            outcome.after_tokens
        ),
        Ok(outcome) => {
            let reason = match outcome.reason.as_str() {
                "history_too_short" => "历史过短，无需压缩",
                "below_threshold" => "上下文未达到压缩阈值，无需压缩",
                _ => "无需压缩",
            };
            format!("{reason}（保留 {} 条）", outcome.kept_count)
        }
        Err(error) => {
            let summary: String = error.chars().take(200).collect();
            format!("上下文压缩失败: {summary}")
        }
    }
}

impl AgentRuntime {
    /// 手动压缩用的 provider（解析策略与生产 turn 一致：默认 provider selection）。
    ///
    /// 单独抽出是为了让控制面只在**解析 provider 期间**持 `runtime` 读锁（毫秒级），
    /// 不再把读锁跨到网络调用上（审查 F2a）。
    pub(crate) fn compaction_provider(&self) -> ProviderManager {
        let selection = self
            .provider_resolver
            .resolve_provider_selection(None, None);
        // PA-172 §3.5 W1：压缩摘要属同步 `decide_with_tools` 路径 ⇒ 继承同一策略与链
        // （矩阵 §3.4：该 path 的 provider 就是装链的 manager）。
        let chain = self.provider_resolver.resolve_failover_chain(&selection);
        ProviderManager::new(selection)
            .with_policy(self.provider_resolver.upstream_policy())
            .with_failover(crate::agent::provider::ProviderFailoverChain::new(chain))
    }
}

/// 手动压缩的唯一实现（契约 C-1；控制面 `HostControlPlane::compact_session` 由这里进入）。
///
/// 三段式：短读快照 → 无锁摘要 → 短写回写。控制面只在解析 provider 时持 `runtime` 读锁，
/// 之后带着 `sessions` 的 `Arc` 直接调用本函数（审查 F2a：网络调用期间零持锁）。
pub(crate) fn compact_session_with_sessions(
    sessions: &Arc<RwLock<SessionStore>>,
    provider: ProviderManager,
    session_id: &str,
    force: bool,
) -> Result<ManualCompactionOutcome, String> {
    let config = CompressionConfig::default();

    // 1) 短读锁：取历史快照后立刻释放。
    let base_history = read_session_history(sessions, session_id);
    let before_tokens = estimate_history_tokens(&base_history);

    // 2) 非强制模式下未达阈值：不改写历史，且**零 LLM 调用**。
    if !force && !should_compress(&base_history, &provider, &config) {
        return Ok(ManualCompactionOutcome::untouched(
            "below_threshold",
            before_tokens,
            base_history.len(),
        ));
    }

    // 3) 切分；没有可压缩区间时同样不改写历史（也**零 LLM 调用**）。
    let (to_compress, to_keep) = split_history(&base_history, &config);
    if to_compress.is_empty() {
        return Ok(ManualCompactionOutcome::untouched(
            "history_too_short",
            before_tokens,
            to_keep.len(),
        ));
    }

    // 4) 无锁段：发起摘要调用（带墙钟上界，见 `summarize_with_deadline`）。
    let request = build_compaction_request(&provider, &to_compress);
    let summary_text = summarize_with_deadline(provider, request)?;
    let summary_message = wrap_summary_to_message(&summary_text);
    // F3：空摘要（空串/仅空白/仅有 `<session_summary></session_summary>` 壳）一律中止，
    // 不得用空摘要替换历史，也不得报成功。
    if summary_body(&summary_message.content).is_empty() {
        return Err(format!(
            "{COMPACTION_EMPTY_SUMMARY_ERROR_CODE}: compaction aborted (model returned an empty summary)"
        ));
    }

    // 5) 短写锁：回写。压缩期间并发追加的消息必须保留，绝不被覆盖丢失。
    let mut store = sessions.write().unwrap_or_else(|e| {
        eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
        e.into_inner()
    });
    let current_history = store
        .snapshot_at_readonly(Some(session_id), None, &[])
        .history;
    let appended = concurrent_suffix(&base_history, &current_history)?;
    let result = apply_compression(base_history, summary_message, &config);
    if result.compressed_count == 0 {
        // 防御：切分结果为空时不得谎报已压缩。
        return Ok(ManualCompactionOutcome::untouched(
            "history_too_short",
            before_tokens,
            result.kept_count,
        ));
    }

    let compressed_count = result.compressed_count;
    let appended_count = appended.len();
    let mut new_history = result.history;
    new_history.extend(appended);
    let kept_count = result.kept_count + appended_count;
    let after_tokens = estimate_history_tokens(&new_history);
    store.replace_session_history(Some(session_id), new_history);
    drop(store);

    runtime_log(format!(
        "compact:session={} compressed={} kept={} before_tokens={} after_tokens={}",
        session_id, compressed_count, kept_count, before_tokens, after_tokens
    ));

    Ok(ManualCompactionOutcome {
        compacted: true,
        reason: "compacted".to_string(),
        compressed_count,
        kept_count,
        before_tokens,
        after_tokens,
    })
}

/// 读取会话历史快照（短读锁，锁毒化按既有惯例恢复）。
fn read_session_history(
    sessions: &Arc<RwLock<SessionStore>>,
    session_id: &str,
) -> Vec<TurnHistoryMessage> {
    let store = sessions.read().unwrap_or_else(|e| {
        eprintln!("[pony-agent] sessions rwlock poisoned: {e}, recovering");
        e.into_inner()
    });
    store
        .snapshot_at_readonly(Some(session_id), None, &[])
        .history
}

/// 构建摘要请求（与 `runtime/turn_prep.rs` 的自动压缩口径一致：
/// system prompt + 待压缩消息逐条 user、temperature 0.3、max_output_tokens 2048）。
fn build_compaction_request(
    provider: &ProviderManager,
    to_compress: &[TurnHistoryMessage],
) -> ProviderRequest {
    let mut messages = vec![ProviderMessage::system(build_compression_system_prompt())];
    for message in to_compress {
        messages.push(ProviderMessage::user(&message.content));
    }
    ProviderRequest {
        model: provider.model().to_string(),
        input: messages,
        images: Vec::new(),
        native_messages: Vec::new(),
        observation: ProviderRequestObservation::default(),
        temperature: 0.3,
        max_output_tokens: 2048,
    }
}

/// 带**确定性墙钟上界**的摘要调用（审查 F2b）。
///
/// 为什么需要外层 deadline：`ProviderStreamGuard` 的 deadline 只在收到帧时检查，半开连接下
/// 永不触发（只能等 provider 的 180s 读空闲预算）；因此把调用放进 worker 线程，由调用方
/// `recv_timeout` 兜底。真流式 provider 同时接入同一 deadline + 取消位，超时后能协作退出；
/// 非流式 provider（含 anthropic 同步路径）由外层 deadline 兜底。
fn summarize_with_deadline(
    provider: ProviderManager,
    request: ProviderRequest,
) -> Result<String, String> {
    let deadline = Duration::from_secs(COMPACTION_TIMEOUT_SECS);
    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_worker = Arc::clone(&cancel);
    let streamable = provider.supports_true_streaming_decision();

    std::thread::spawn(move || {
        let _guard = crate::agent::runtime_helper::TestRuntimeGuard::new();
        let result = if streamable {
            let cancel_check = || cancel_for_worker.load(AtomicOrdering::Relaxed);
            let stream_guard = ProviderStreamGuard::new(
                deadline,
                ProviderStreamGuard::DEFAULT_CANCEL_TICK,
                Some(&cancel_check),
            );
            provider
                .decide_with_tools_stream_bounded(&stream_guard, &request, &[], |_chunk| {})
                .map(|decision| decision.output_text)
        } else {
            provider
                .decide_with_tools(&request, &[])
                .map(|decision| decision.output_text)
        };
        // 接收端可能已因超时返回：发送失败是可接受的（结果已无人等待）。
        let _ = tx.send(result);
    });

    match rx.recv_timeout(deadline) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // 协作取消：流式 provider 在下一次帧/守卫检查点退出。
            cancel.store(true, AtomicOrdering::Relaxed);
            Err(format!(
                "{COMPACTION_TIMEOUT_ERROR_CODE}: manual compaction summarization timed out after {}s",
                deadline.as_secs()
            ))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("compaction: summarization worker exited without a result".to_string())
        }
    }
}

/// 取出「wrap 后的摘要消息」里属于模型输出的正文（剥掉固定前后缀）。
///
/// 用于 F3 判定：模型只回一个 `<session_summary></session_summary>` 壳时，wrap 后的消息
/// 仍然非空（前后缀在里面），但**实质摘要为空**，不得据此改写历史。
fn summary_body(wrapped_content: &str) -> String {
    let prefix = build_summary_prefix();
    let suffix = build_summary_suffix();
    let trimmed = wrapped_content.trim();
    let without_prefix = trimmed.strip_prefix(prefix.trim()).unwrap_or(trimmed);
    let body = without_prefix
        .strip_suffix(suffix.trim())
        .unwrap_or(without_prefix);
    body.trim().to_string()
}

/// 压缩期间被并发追加的消息（历史快照基线之后新增的部分）。
///
/// - 基线未被改写（`current` 与 `base` 逐条相等）⇒ 返回新增尾部，回写时原样保留；
/// - 基线被改写（分支切换 / 清空 / 重写）⇒ 返回 `Err`，放弃写入而不是覆盖丢数据。
fn concurrent_suffix(
    base: &[TurnHistoryMessage],
    current: &[TurnHistoryMessage],
) -> Result<Vec<TurnHistoryMessage>, String> {
    if current.len() < base.len() {
        return Err(format!(
            "{COMPACTION_HISTORY_CHANGED_ERROR_CODE}: session history shrank during compaction; \
             refusing to overwrite (base={}, current={})",
            base.len(),
            current.len()
        ));
    }
    if current[..base.len()] != *base {
        return Err(format!(
            "{COMPACTION_HISTORY_CHANGED_ERROR_CODE}: session history was rewritten during \
             compaction; refusing to overwrite {} messages",
            current.len()
        ));
    }
    Ok(current[base.len()..].to_vec())
}

#[cfg(test)]
mod tests {
    //! 手动压缩的纯逻辑单测（F1~F4 的可测面；验收测试在 `tests/slash_commands_acceptance.rs`）。
    use super::*;

    fn turn(role: &str, content: &str) -> TurnHistoryMessage {
        TurnHistoryMessage {
            role: role.to_string(),
            content: content.to_string(),
            ..Default::default()
        }
    }

    fn outcome(compacted: bool, reason: &str) -> ManualCompactionOutcome {
        ManualCompactionOutcome {
            compacted,
            reason: reason.to_string(),
            compressed_count: if compacted { 18 } else { 0 },
            kept_count: 6,
            before_tokens: 9_000,
            after_tokens: if compacted { 2_400 } else { 9_000 },
        }
    }

    #[test]
    fn compact_message_success_carries_both_counts() {
        let message = compact_outcome_message(&Ok(outcome(true, "compacted")));
        assert!(message.contains("已压缩"), "{message}");
        assert!(message.contains("18"), "{message}");
        assert!(message.contains("6"), "{message}");
    }

    #[test]
    fn compact_message_not_compacted_never_claims_compaction() {
        for reason in ["history_too_short", "below_threshold"] {
            let message = compact_outcome_message(&Ok(outcome(false, reason)));
            assert!(message.contains("无需压缩"), "{reason}: {message}");
            assert!(!message.contains("已压缩"), "{reason}: {message}");
        }
    }

    #[test]
    fn compact_message_error_carries_summary_without_fake_success() {
        let message = compact_outcome_message(&Err("boom: provider unreachable".to_string()));
        assert!(message.contains("provider unreachable"), "{message}");
        assert!(!message.contains("已压缩"), "{message}");
    }

    #[test]
    fn summary_body_detects_empty_summaries() {
        // 空串 / 仅空白
        assert!(summary_body(&wrap_summary_to_message("").content).is_empty());
        assert!(summary_body(&wrap_summary_to_message("   \n\t ").content).is_empty());
        // 只有标签壳：实质摘要为空
        assert!(summary_body(
            &wrap_summary_to_message("<session_summary>  </session_summary>").content
        )
        .is_empty());
        // 有实质内容
        assert!(!summary_body(
            &wrap_summary_to_message("<session_summary>goal: 压缩上下文</session_summary>").content
        )
        .is_empty());
        assert!(!summary_body(&wrap_summary_to_message("无标签的原始摘要").content).is_empty());
    }

    #[test]
    fn concurrent_suffix_preserves_appended_and_rejects_rewrites() {
        let base = vec![turn("user", "a"), turn("assistant", "b")];

        // 未变：无新增
        assert!(concurrent_suffix(&base, &base).expect("unchanged").is_empty());

        // 追加：新增尾部必须原样返回（压缩期间到来的新轮次不能被覆盖丢失）
        let mut extended = base.clone();
        extended.push(turn("user", "c"));
        let suffix = concurrent_suffix(&base, &extended).expect("extended");
        assert_eq!(suffix, vec![turn("user", "c")]);

        // 变短 / 被改写：不得覆盖，返回可辨识错误
        let shrunk = vec![turn("user", "a")];
        let error = concurrent_suffix(&base, &shrunk).expect_err("shrunk must fail");
        assert!(error.contains(COMPACTION_HISTORY_CHANGED_ERROR_CODE), "{error}");

        let rewritten = vec![turn("user", "a"), turn("assistant", "ZZZ")];
        let error = concurrent_suffix(&base, &rewritten).expect_err("rewritten must fail");
        assert!(error.contains(COMPACTION_HISTORY_CHANGED_ERROR_CODE), "{error}");
    }
}
