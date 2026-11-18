//! ACP 委派最终正文归约（wave-acp-delegate C0.1–C0.4 / C2.1）。
//!
//! 真源：`.dev-team/contract-matrix-acp-delegate-p0p2.md` C0.1–C0.4 与 C2.1。
//!
//! 契约（实现已落地）：
//! - [`reduce_final_text`] 按**到达顺序**拼接 `SessionUpdate::AgentMessageChunk` 的文本块；
//! - 归约结果超过 [`ACP_DELEGATE_FINAL_TEXT_MAX_BYTES`] ⇒ 保留前缀并置 `truncated = true`
//!   （上限是单一常量真源，禁止在别处再写字面量）；
//! - 无 `AgentMessageChunk` ⇒ `text` 为空串、`truncated = false`（不得伪造内容，C0.2）；
//! - [`reduce_usage`] 把 `usage_update` 的 used/size 与可选 cost 归约成结果 JSON 的 `usage`
//!   （C2.1；**不写** `usage_ledger`，Non-Goal 3）。
//!
//! 纯函数、无 I/O：本文件不发射事件、不写日志（日志归调用点 `acp/session.rs`）。

use serde_json::{json, Value};

use crate::acp::protocol::{ContentBlock, Cost, SessionUpdate};

/// `acp_delegate` 最终正文的长度上限（**UTF-8 字节数**，单一真源）。
///
/// 语义：结果 JSON 的 `finalText` 最多保留该字节数的前缀（切口回退到最近的
/// 字符边界，绝不切出半个 UTF-8 码点）；超出部分丢弃并把 `finalTextTruncated`
/// 置 `true`（C0.3）。
///
/// B240 口径统一：此前这里是 **32_000 字符**，而下游落库截断
/// （`agent/telemetry.rs::TOOL_RESULT_TEXT_MAX_CHARS = 32 * 1024`，按**字节**且字界安全）
/// 是 32 KiB。两者不同量纲 ⇒ 32 000 个 CJK 字符 ≈ 96 KB，会在下游被静默再切一刀，
/// 而 `finalTextTruncated` 仍为 `false`（谎报「正文完整」）。
/// 现在两者同一量纲且 `32_000 < 32_768`：`finalText` 正文**必然整体活过**下游截断，
/// 标志位与实际保留内容永远一致（ASCII 下字节数 == 字符数 ⇒ 既有 32 000 断言不变）。
pub(crate) const ACP_DELEGATE_FINAL_TEXT_MAX_BYTES: usize = 32_000;

/// `acp_delegate` 最终正文的归约产物（C0.1 / C0.3）。
#[derive(Debug, Clone, Default)]
pub(crate) struct DelegateFinalText {
    /// 按到达顺序拼接的正文（超长时为前缀，可能为空串）。
    pub text: String,
    /// 是否因超过 [`ACP_DELEGATE_FINAL_TEXT_MAX_BYTES`] 被截断。
    pub truncated: bool,
}

/// 从 ACP `session/update` 序列归约最终正文（C0.1 / C0.3）。
///
/// - 只吃 `AgentMessageChunk` 的 **文本** 块（图片/资源等非文本块不进正文）；
/// - 顺序 = 到达顺序（`updates` 是接收序，不得重排/去重）；
/// - 越过上限即截断并**停止累积**（尾标记不得出现在结果里，C0.3）。
pub(crate) fn reduce_final_text(updates: &[SessionUpdate]) -> DelegateFinalText {
    let mut reduced = DelegateFinalText::default();
    let mut length = 0usize; // 已累积的**字节**数
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
        let chunk_bytes = text.len();
        let remaining = ACP_DELEGATE_FINAL_TEXT_MAX_BYTES.saturating_sub(length);
        if chunk_bytes > remaining {
            // B240：按字节上限截断，但切口必须落在 UTF-8 字界上
            // （CJK / emoji 是多字节码点，直切会切出半个字符 ⇒ 后续 `serde_json`
            // 序列化或前端解码得到替换符，正文被静默污染）。
            let mut cut = remaining;
            while cut > 0 && !text.is_char_boundary(cut) {
                cut -= 1;
            }
            reduced.text.push_str(&text[..cut]);
            reduced.truncated = true;
            length = ACP_DELEGATE_FINAL_TEXT_MAX_BYTES;
        } else {
            reduced.text.push_str(text);
            length += chunk_bytes;
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

    /// C0.3：越上限 ⇒ 恰好保留上限字节的前缀并标记截断。
    #[test]
    fn reduce_final_text_truncates_to_limit_prefix() {
        let oversize = format!("BEGIN-{}", "y".repeat(ACP_DELEGATE_FINAL_TEXT_MAX_BYTES));
        let reduced = reduce_final_text(&[text_chunk(&oversize)]);
        assert!(reduced.truncated);
        assert_eq!(reduced.text.len(), ACP_DELEGATE_FINAL_TEXT_MAX_BYTES);
        assert!(reduced.text.starts_with("BEGIN-"));
    }

    /// B240 回归：上限是**字节**口径 ⇒ 多字节正文不得被下游 32 KiB 字节截断再切一刀，
    /// 且切口必须落在 UTF-8 字界（不产生半个码点 / 替换符）。
    #[test]
    fn reduce_final_text_is_byte_bounded_and_utf8_safe() {
        // 三字节 CJK 码点：字符数远小于上限，字节数必须恰好受控。
        let body = "界".repeat(ACP_DELEGATE_FINAL_TEXT_MAX_BYTES);
        let reduced = reduce_final_text(&[text_chunk(&body)]);
        assert!(reduced.truncated, "超字节上限必须标记截断");
        assert!(
            reduced.text.len() <= ACP_DELEGATE_FINAL_TEXT_MAX_BYTES,
            "字节数不得超过上限"
        );
        assert_eq!(
            reduced.text.len() % 3,
            0,
            "切口必须在 3 字节 CJK 码点边界上"
        );
        assert!(
            std::str::from_utf8(reduced.text.as_bytes()).is_ok(),
            "截断结果必须是合法 UTF-8"
        );
        assert!(
            reduced.text.chars().all(|c| c == '界'),
            "不得混入替换符或半个码点"
        );
        // 下游（工具结果落库）按 32 KiB 字节截断：本上限必须严格小于它，
        // 否则 `finalTextTruncated=false` 却仍被下游改写的旧缺陷会回来。
        assert!(
            reduced.text.len() < 32 * 1024,
            "finalText 字节上限必须小于下游 32 KiB 字节预算"
        );
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
