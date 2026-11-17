//! ACP 委派最终正文归约（wave-acp-delegate C0.1–C0.4，**W1 签名骨架**）。
//!
//! 真源：`.dev-team/contract-matrix-acp-delegate-p0p2.md` C0.1–C0.4。
//! 本文件在 W1（协议冻结）只声明常量 / 类型 / 函数签名，函数体为 `unimplemented!()`；
//! 实现归 W2a（写域 `crates/pony-agent-core/src/acp/**`）。
//!
//! 实现侧必须满足的契约（此处仅冻结形状，不含任何逻辑）：
//! - [`reduce_final_text`] 按**到达顺序**拼接 `SessionUpdate::AgentMessageChunk` 的文本块；
//! - 归约结果超过 [`ACP_DELEGATE_FINAL_TEXT_MAX_CHARS`] ⇒ 保留前缀并置 `truncated = true`
//!   （上限是单一常量真源，禁止在别处再写字面量）；
//! - 无 `AgentMessageChunk` ⇒ `text` 为空串、`truncated = false`（不得伪造内容，C0.2）。
//!
//! 空桩期（W1）显式放行 `dead_code`：消费者在 W2a 才接入，而发版构建带 `-D warnings`；
//! 实现落地后该放行随「函数被调用」自然失效（不是永久豁免，也不是绕过门禁）。
#![allow(dead_code)]

use crate::acp::protocol::SessionUpdate;

/// `acp_delegate` 最终正文的长度上限（字符数，单一真源）。
///
/// 语义：结果 JSON 的 `finalText` 最多保留该长度的前缀；超出部分丢弃并把
/// `finalTextTruncated` 置 `true`（C0.3）。取值与工具结果落库上限
/// （`agent/telemetry.rs::TOOL_RESULT_TEXT_MAX_CHARS = 32 * 1024`）同量级，
/// 避免一条委派把 session blob 撑爆。
pub const ACP_DELEGATE_FINAL_TEXT_MAX_CHARS: usize = 32_000;

/// `acp_delegate` 最终正文的归约产物（C0.1 / C0.3）。
#[derive(Debug, Clone, Default)]
pub(crate) struct DelegateFinalText {
    /// 按到达顺序拼接的正文（超长时为前缀，可能为空串）。
    pub text: String,
    /// 是否因超过 [`ACP_DELEGATE_FINAL_TEXT_MAX_CHARS`] 被截断。
    pub truncated: bool,
}

/// 从 ACP `session/update` 序列归约最终正文（签名冻结；实现归 W2a）。
#[allow(unused_variables)]
pub(crate) fn reduce_final_text(updates: &[SessionUpdate]) -> DelegateFinalText {
    unimplemented!()
}
