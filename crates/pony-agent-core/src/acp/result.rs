//! ACP 委派最终正文归约（wave-acp-delegate C0.1–C0.4 / C2.1）。
//!
//! 真源：`.dev-team/contract-matrix-acp-delegate-p0p2.md` C0.1–C0.4 与 C2.1。
//!
//! 契约（实现已落地）：
//! - [`reduce_final_text`] 按**到达顺序**拼接 `SessionUpdate::AgentMessageChunk` 的文本块；
//! - 归约结果超过 [`ACP_DELEGATE_FINAL_TEXT_MAX_CHARS`] ⇒ 保留前缀并置 `truncated = true`
//!   （上限是单一常量真源，禁止在别处再写字面量）；
//! - 无 `AgentMessageChunk` ⇒ `text` 为空串、`truncated = false`（不得伪造内容，C0.2）；
//! - [`reduce_usage`] 把 `usage_update` 的 used/size 与可选 cost 归约成结果 JSON 的 `usage`
//!   （C2.1；**不写** `usage_ledger`，Non-Goal 3）。
//!
//! 纯函数、无 I/O：本文件不发射事件、不写日志（日志归调用点 `acp/session.rs`）。

use serde_json::{json, Value};

use crate::acp::protocol::{ContentBlock, Cost, SessionUpdate};

/// `acp_delegate` 最终正文的长度上限（字符数，单一真源）。
///
/// 语义：结果 JSON 的 `finalText` 最多保留该长度的前缀；超出部分丢弃并把
/// `finalTextTruncated` 置 `true`（C0.3）。取值与工具结果落库上限
/// （`agent/telemetry.rs::TOOL_RESULT_TEXT_MAX_CHARS = 32 * 1024`）同量级，
/// 避免一条委派把 session blob 撑爆。
pub(crate) const ACP_DELEGATE_FINAL_TEXT_MAX_CHARS: usize = 32_000;

/// `acp_delegate` 最终正文的归约产物（C0.1 / C0.3）。
#[derive(Debug, Clone, Default)]
pub(crate) struct DelegateFinalText {
    /// 按到达顺序拼接的正文（超长时为前缀，可能为空串）。
    pub text: String,
    /// 是否因超过 [`ACP_DELEGATE_FINAL_TEXT_MAX_CHARS`] 被截断。
    pub truncated: bool,
}

/// 从 ACP `session/update` 序列归约最终正文（C0.1 / C0.3）。
///
/// - 只吃 `AgentMessageChunk` 的 **文本** 块（图片/资源等非文本块不进正文）；
/// - 顺序 = 到达顺序（`updates` 是接收序，不得重排/去重）；
/// - 越过上限即截断并**停止累积**（尾标记不得出现在结果里，C0.3）。
pub(crate) fn reduce_final_text(updates: &[SessionUpdate]) -> DelegateFinalText {
    let mut reduced = DelegateFinalText::default();
    let mut length = 0usize;
    for update in updates {
        if reduced.truncated {
            break;
        }
        let SessionUpdate::AgentMessageChunk { content, .. } = update else {
            continue;
        };
        let ContentBlock::Text { text } = content else {
            continue;
        };
        let chunk_chars = text.chars().count();
        let remaining = ACP_DELEGATE_FINAL_TEXT_MAX_CHARS.saturating_sub(length);
        if chunk_chars > remaining {
            reduced.text.extend(text.chars().take(remaining));
            reduced.truncated = true;
            length = ACP_DELEGATE_FINAL_TEXT_MAX_CHARS;
        } else {
            reduced.text.push_str(text);
            length += chunk_chars;
        }
    }
    reduced
}

/// `acp_delegate` 结果 JSON 的 `usage` 面（C2.1：`{used, size, cost?}`）。
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct DelegateUsage {
    /// 当前上下文内 token 数（`usage_update.used`）。
    pub used: u64,
    /// 上下文窗口总大小（`usage_update.size`）。
    pub size: u64,
    /// 会话累计 cost（可选；`usage_update.cost` 透传 `{amount, currency}`）。
    pub cost: Option<Cost>,
}

impl DelegateUsage {
    /// 结果 JSON 的 `usage` 对象（`cost` 缺失 ⇒ 键缺席，不用 null/0 占位）。
    pub(crate) fn to_json(&self) -> Value {
        let mut usage = serde_json::Map::new();
        usage.insert("used".to_string(), json!(self.used));
        usage.insert("size".to_string(), json!(self.size));
        if let Some(cost) = &self.cost {
            usage.insert(
                "cost".to_string(),
                json!({ "amount": cost.amount, "currency": cost.currency }),
            );
        }
        Value::Object(usage)
    }
}

/// 归约 `usage_update`（C2.1）：ACP 的 used/size 是**快照**语义 ⇒ 后到者覆盖；
/// cost 是会话累计值且本就可选 ⇒ 后到者缺省时沿用上一个已知值（不得凭空归零）。
///
/// 无 `usage_update` ⇒ `None`（结果 JSON 不出现 `usage` 键，C0.1 既有键面不膨胀）。
pub(crate) fn reduce_usage(updates: &[SessionUpdate]) -> Option<DelegateUsage> {
    let mut latest: Option<DelegateUsage> = None;
    for update in updates {
        let SessionUpdate::UsageUpdate(usage) = update else {
            continue;
        };
        let carried_cost = usage
            .cost
            .clone()
            .or_else(|| latest.as_ref().and_then(|previous| previous.cost.clone()));
        latest = Some(DelegateUsage {
            used: usage.used,
            size: usage.size,
            cost: carried_cost,
        });
    }
    latest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text_chunk(text: &str) -> SessionUpdate {
        SessionUpdate::AgentMessageChunk {
            content: ContentBlock::Text {
                text: text.to_string(),
            },
            message_id: None,
        }
    }

    /// C0.1：多块正文按到达顺序拼接，且未越上限 ⇒ 不截断。
    #[test]
    fn reduce_final_text_keeps_arrival_order() {
        let updates = [
            text_chunk("ALPHA-1|"),
            text_chunk("BETA-2|"),
            text_chunk("GAMMA-3|"),
        ];
        let reduced = reduce_final_text(&updates);
        assert_eq!(reduced.text, "ALPHA-1|BETA-2|GAMMA-3|");
        assert!(!reduced.truncated);
    }

    /// C0.2：无正文 ⇒ 空串 + 未截断（不得伪造内容）。
    #[test]
    fn reduce_final_text_is_empty_without_chunks() {
        let reduced = reduce_final_text(&[]);
        assert_eq!(reduced.text, "");
        assert!(!reduced.truncated);
    }

    /// C0.3：越上限 ⇒ 恰好保留上限字符的前缀并标记截断。
    #[test]
    fn reduce_final_text_truncates_to_limit_prefix() {
        let oversize = format!("BEGIN-{}", "y".repeat(ACP_DELEGATE_FINAL_TEXT_MAX_CHARS));
        let reduced = reduce_final_text(&[text_chunk(&oversize)]);
        assert!(reduced.truncated);
        assert_eq!(
            reduced.text.chars().count(),
            ACP_DELEGATE_FINAL_TEXT_MAX_CHARS
        );
        assert!(reduced.text.starts_with("BEGIN-"));
    }

    /// C2.1：快照语义（后到者覆盖 used/size）+ cost 缺失时沿用上一已知值。
    #[test]
    fn reduce_usage_keeps_latest_snapshot_and_carries_cost() {
        let with_cost = SessionUpdate::UsageUpdate(crate::acp::protocol::UsageUpdate {
            used: 100,
            size: 200_000,
            cost: Some(Cost {
                amount: 0.5,
                currency: "USD".to_string(),
                meta: None,
            }),
            meta: None,
        });
        let without_cost = SessionUpdate::UsageUpdate(crate::acp::protocol::UsageUpdate {
            used: 12_345,
            size: 200_000,
            cost: None,
            meta: None,
        });
        let usage = reduce_usage(&[with_cost, without_cost]).expect("有 usage_update");
        assert_eq!(usage.used, 12_345);
        assert_eq!(usage.size, 200_000);
        assert_eq!(
            usage.cost.as_ref().map(|cost| cost.amount),
            Some(0.5),
            "cost 是可选的累计值：后到者缺省时必须沿用上一个已知值"
        );
        assert!(
            reduce_usage(&[]).is_none(),
            "无 usage_update ⇒ 结果无 usage 键"
        );
    }
}
