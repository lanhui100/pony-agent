// title_ops: AI 会话标题生成（斜杠命令 `/title` 与首轮自动生成共用核心实现）。
//
// 与 `compaction_ops.rs` 同构的 IO 纪律：
// - **锁纪律**：网络调用期间**不持** `sessions` 读写锁 —— 拆成「短读快照 → 无锁生成 → 短写回写」。
// - **时长上界**：生成调用在独立线程执行，调用方以墙钟 deadline 兜底，TUI 不会永久卡在
//   「正在生成标题…」；真流式 provider 同一 deadline 接入 `ProviderStreamGuard` 可协作取消。
// - **空输出**：模型只回空白 / 壳一律 `Err`，绝不用空标题覆盖历史（不谎报成功）。
//
// 标题来源（`TitleSource`）三线并存：用户改名（title_override）> AI（ai_title）> 派生（title）。
// 本模块只写 `ai_title`（并经 `SessionStore::set_ai_title` 镜像到冻结基准 `title`）。
use crate::agent::provider::{
    ProviderManager, ProviderMessage, ProviderRequest, ProviderRequestObservation,
    ProviderStreamGuard,
};
use crate::agent::session::is_host_continuation_prompt;
use crate::agent::session::{SessionStore, SessionSnapshot};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::mpsc;
use std::sync::{Arc, RwLock};
use std::time::Duration;

/// AI 标题生成调用的墙钟上界（秒）。标题是短任务，30s 足够覆盖建连 + 生成，且明显短于
/// turn 的缺省流式上界，避免半开连接把 TUI 挂在「正在生成标题…」。
pub const TITLE_TIMEOUT_SECS: u64 = 30;

/// 超时错误码（调用方可按前缀辨识）。
pub const TITLE_TIMEOUT_ERROR_CODE: &str = "title_deadline_exceeded";
/// 无可用于生成标题的真实用户轮。
pub const TITLE_NO_USER_MESSAGE_ERROR_CODE: &str = "no_user_message";
/// 模型输出为空 / 仅壳。
pub const TITLE_EMPTY_OUTPUT_ERROR_CODE: &str = "empty_title";
/// 用于喂给模型的对话片段条数上限（只取最早若干轮，标题取决于会话「开场」）。
const TITLE_CONTEXT_MAX_MESSAGES: usize = 8;
/// 单条消息喂给模型的字符上限（避免长文把标题 prompt 撑爆）。
const TITLE_CONTEXT_MESSAGE_CHARS: usize = 600;
/// AI 标题字符上限（比派生标题的 28 更宽松，留给「动作+对象」表达；仍受 store 的 64 校验约束）。
pub const AI_TITLE_MAX_CHARS: usize = 40;

/// AI 标题生成结果（前端 / Tauri 序列化契约，camelCase）。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionTitleOutcome {
    /// 是否真的写入了一个新标题。
    pub generated: bool,
    /// 生效的标题（generated=false 时为原值 / 空）。
    pub title: String,
    /// 标题来源（"ai" / "user" / "derived"）。
    pub source: String,
    /// 未生成时的原因码（generated=true 时为空）。
    pub reason: String,
}

/// 标题生成的系统提示（纯字符串，便于测试断言与漂移防控）。
pub fn build_title_system_prompt() -> String {
    "你是会话标题生成器。请阅读用户与助手的对话片段，用一句不超过 20 个字的中文短语概括\
     这次会话在做什么，作为侧边栏标题。要求：只输出标题本身，不要引号、不要句号、不要\
     「标题：」等前缀、不要解释，也不要换行。"
        .to_string()
}

/// 把会话历史压成一段喂给模型的对话片段。只取最早的若干真实用户轮与其后的助手回复；
/// 若整段历史不含任何真实用户轮（只有宿主续轮提示 / 空），返回 `None`。
pub fn format_title_context(history: &[crate::agent::session::TurnHistoryMessage]) -> Option<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut saw_user = false;
    for message in history {
        let role = message.role.as_str();
        if role != "user" && role != "assistant" {
            continue;
        }
        if role == "user" {
            if is_host_continuation_prompt(&message.content) {
                continue;
            }
            saw_user = true;
        } else if !saw_user {
            // 跳过开场 assistant（无对应真实用户轮之前的内容）
            continue;
        }
        let content: String = message
            .content
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let clipped: String = content.chars().take(TITLE_CONTEXT_MESSAGE_CHARS).collect();
        if clipped.trim().is_empty() {
            continue;
        }
        let label = if role == "user" { "用户" } else { "助手" };
        lines.push(format!("{label}：{clipped}"));
        if lines.len() >= TITLE_CONTEXT_MAX_MESSAGES {
            break;
        }
    }
    if !saw_user || lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// 归一化模型原始输出为标题：取首个非空行、剥常见前后缀与包裹引号、折叠内部空白、
/// 截断到 [`AI_TITLE_MAX_CHARS`]。空白 / 仅标点壳返回 `None`。
pub fn normalize_title_output(raw: &str) -> Option<String> {
    let first_line = raw
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    // 剥离常见前缀「标题：」「Title:」等。
    let mut text = first_line;
    for prefix in ["标题：", "标题:", "Title:", "title:", "标题"] {
        if let Some(rest) = text.strip_prefix(prefix) {
            text = rest.trim();
            break;
        }
    }
    // 剥离成对包裹引号（中英文）。
    let trimmed = text.trim();
    let unquoted = trimmed
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| trimmed.strip_prefix('“').and_then(|s| s.strip_suffix('”')))
        .or_else(|| trimmed.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(trimmed)
        .trim();
    let collapsed = unquoted.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() || collapsed.chars().all(|c| c.is_ascii_punctuation()) {
        return None;
    }
    let mut out = String::new();
    let mut count = 0;
    for ch in collapsed.chars() {
        if count >= AI_TITLE_MAX_CHARS {
            out.push('…');
            break;
        }
        out.push(ch);
        count += 1;
    }
    let out = out.trim().trim_end_matches(['。', '.']).trim().to_string();
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 构建标题生成请求（system prompt + 单条 user 对话片段，temperature 0.3，max_output_tokens 64）。
pub fn build_title_request(provider: &ProviderManager, context: &str) -> ProviderRequest {
    ProviderRequest {
        model: provider.model().to_string(),
        input: vec![
            ProviderMessage::system(build_title_system_prompt()),
            ProviderMessage::user(context),
        ],
        images: Vec::new(),
        native_messages: Vec::new(),
        observation: ProviderRequestObservation::default(),
        temperature: 0.3,
        max_output_tokens: 64,
    }
}

/// 读取会话快照（短读锁，锁毒化按既有惯例恢复）。
fn read_session_snapshot(
    sessions: &Arc<RwLock<SessionStore>>,
    session_id: &str,
) -> SessionSnapshot {
    let store = sessions.read().unwrap_or_else(|e| {
        tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
        e.into_inner()
    });
    store.snapshot_at_readonly(Some(session_id), None, &[])
}

/// 带确定性墙钟上界的模型调用（与 `compaction_ops::summarize_with_deadline` 同款纪律）。
fn call_with_deadline(provider: ProviderManager, request: ProviderRequest) -> Result<String, String> {
    let deadline = Duration::from_secs(TITLE_TIMEOUT_SECS);
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
        let _ = tx.send(result);
    });

    match rx.recv_timeout(deadline) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            cancel.store(true, AtomicOrdering::Relaxed);
            Err(format!(
                "{TITLE_TIMEOUT_ERROR_CODE}: title generation timed out after {}s",
                deadline.as_secs()
            ))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("title: generation worker exited without a result".to_string())
        }
    }
}

/// AI 标题生成唯一实现：短读快照 → 无锁生成 → 短写回写。
///
/// 返回 `Err(TITLE_NO_USER_MESSAGE_ERROR_CODE:…)` 表示没有可用真实用户轮（调用方按需静默）；
/// 其余 `Err` 为真实生成失败（超时 / 空输出 / 网络错误）。
pub(crate) fn generate_session_title_with_sessions(
    sessions: &Arc<RwLock<SessionStore>>,
    provider: ProviderManager,
    session_id: &str,
) -> Result<SessionTitleOutcome, String> {
    let snapshot = read_session_snapshot(sessions, session_id);
    if snapshot.conversation_id.is_empty() {
        return Err(format!("会话不存在：{session_id}"));
    }
    let context = format_title_context(&snapshot.history).ok_or_else(|| {
        format!("{TITLE_NO_USER_MESSAGE_ERROR_CODE}: session {session_id} has no real user turn")
    })?;
    let request = build_title_request(&provider, &context);
    let raw = call_with_deadline(provider, request)?;
    let title = normalize_title_output(&raw)
        .ok_or_else(|| format!("{TITLE_EMPTY_OUTPUT_ERROR_CODE}: model returned empty title"))?;

    {
        let mut store = sessions.write().unwrap_or_else(|e| {
            tracing::warn!(target: "control_plane", error = %e, "[pony-agent] sessions rwlock poisoned");
            e.into_inner()
        });
        store.set_ai_title(session_id, &title)?;
    }

    Ok(SessionTitleOutcome {
        generated: true,
        title,
        source: "ai".to_string(),
        reason: String::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_strips_prefix_quotes_and_period() {
        assert_eq!(normalize_title_output("标题：修复登录超时。"), Some("修复登录超时".to_string()));
        assert_eq!(normalize_title_output("\"重构会话列表\""), Some("重构会话列表".to_string()));
        assert_eq!(normalize_title_output("  Title: Add AI titles  "), Some("Add AI titles".to_string()));
        assert_eq!(normalize_title_output("“中文引号标题”"), Some("中文引号标题".to_string()));
    }

    #[test]
    fn normalize_rejects_empty_and_punctuation_only() {
        assert_eq!(normalize_title_output(""), None);
        assert_eq!(normalize_title_output("   \n  "), None);
        assert_eq!(normalize_title_output("..."), None);
    }

    #[test]
    fn normalize_truncates_to_limit() {
        let long = "一".repeat(AI_TITLE_MAX_CHARS + 10);
        let out = normalize_title_output(&long).unwrap();
        // 上限 + 省略号
        assert_eq!(out.chars().count(), AI_TITLE_MAX_CHARS + 1);
        assert!(out.ends_with('…'));
    }

    fn msg(role: &str, content: &str) -> crate::agent::session::TurnHistoryMessage {
        crate::agent::session::TurnHistoryMessage {
            role: role.to_string(),
            content: content.to_string(),
            attachments: Vec::new(),
            ..Default::default()
        }
    }

    #[test]
    fn context_requires_real_user_turn() {
        assert!(format_title_context(&[]).is_none());
        // 只有助手消息
        assert!(format_title_context(&[msg("assistant", "hi")]).is_none());
        // 只有宿主续轮提示
        let cont = "（已作答，继续上一轮未完成的步骤）";
        assert!(crate::agent::session::is_host_continuation_prompt(cont));
        assert!(format_title_context(&[msg("user", cont)]).is_none());
        // 有真实用户轮
        let ctx = format_title_context(&[msg("user", "帮我修复登录 bug"), msg("assistant", "好的")])
            .unwrap();
        assert!(ctx.contains("用户：帮我修复登录 bug"));
        assert!(ctx.contains("助手：好的"));
    }
}
