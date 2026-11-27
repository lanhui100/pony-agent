//! 一次 turn 的提示词**内容单源**：稳定/半稳定层只渲染一次，两个 provider 视图只做角色投影。
//!
//! 历史背景（backlog B133）：同一批内容要同时进 normalized（`ProviderMessage`）与 native
//! （OpenAI 原生 JSON）两个视图，旧实现把它们各拼一遍——易变上下文一 turn 渲染 4 次
//! （其中 2 次只为估算预算）、长期记忆渲染 2 次、native 侧靠 `len()-1` 推导 domain 插入下标。
//! 易变段还会读全局目标/待办存储，重复渲染等于重复读取同一份事实。
//!
//! 现在：内容在 [`TurnPromptContent`] 里构造一次，`normalized_prefix` / `native_prefix` /
//! `system_messages_of` 只做角色映射。历史（conversation carry）不在这里：normalized 用文本历史、
//! native 用 provider 原生 transcript，来源不同且各自独立截断（ADR 0123）。

use serde_json::{json, Value};

use crate::agent::context::VolatileNotes;
use crate::agent::provider::ProviderMessage;

pub(crate) const SYSTEM_REMINDER_OPEN: &str = "<system-reminder>";
pub(crate) const SYSTEM_REMINDER_CLOSE: &str = "</system-reminder>";

/// 将易变运行时提示包装为 `<system-reminder>`；空串原样返回（调用方跳过空消息）。
pub(crate) fn wrap_system_reminder(body: &str) -> String {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    format!("{SYSTEM_REMINDER_OPEN}\n{trimmed}\n{SYSTEM_REMINDER_CLOSE}")
}

pub(crate) fn is_system_reminder_message(content: &str) -> bool {
    content.trim_start().starts_with(SYSTEM_REMINDER_OPEN)
}

/// 一次 turn 的**内容单源**：稳定/半稳定层只渲染一次，两个 provider 视图只做角色投影。
///
/// 为什么要有这一层：同一批内容要同时进 normalized（`ProviderMessage`）与 native（OpenAI
/// 原生 JSON）两个视图，历史实现把它们各拼一遍——易变上下文一 turn 渲染 4 次（其中 2 次只为
/// 估算预算）、长期记忆渲染 2 次、native 侧靠 `len()-1` 推导 domain 插入下标。易变段还会读全局
/// 目标/待办存储，重复渲染等于重复读取。
///
/// 历史（conversation carry）**不进**本结构：normalized 用文本历史、native 用 provider 原生
/// transcript，来源不同且各自独立截断（ADR 0123）。
pub(crate) struct TurnPromptContent {
    /// BASE 及其后紧跟的稳定层段（团队策略段仅在接线时存在）。
    pub(crate) base: Vec<String>,
    /// domain profile（窗口过小时为空）。
    pub(crate) domain_profile: Vec<String>,
    /// 半稳定段：会话标签 + 工作区宪法 + 技能清单。
    pub(crate) semistable: Vec<String>,
    /// 长期记忆段。
    pub(crate) memory: Vec<String>,
    /// 易变段：分段渲染，按视图决定插入哪条截断提示。
    pub(crate) volatile: VolatileNotes,
}

impl TurnPromptContent {
    /// normalized 预留前缀：稳定层 + 当前输入 + 易变提醒，顺序与 `flatten_layered_input_messages` 一致。
    pub(crate) fn normalized_prefix(
        &self,
        truncation_note: Option<&str>,
        user: ProviderMessage,
    ) -> Vec<ProviderMessage> {
        let mut messages = Vec::new();
        messages.extend(self.base.iter().cloned().map(ProviderMessage::system));
        messages.extend(self.domain_profile.iter().cloned().map(ProviderMessage::developer));
        messages.extend(self.semistable.iter().cloned().map(ProviderMessage::developer));
        messages.extend(self.memory.iter().cloned().map(ProviderMessage::developer));
        messages.push(user);
        let reminder = wrap_system_reminder(&self.volatile.render(truncation_note));
        if !reminder.is_empty() {
            messages.push(ProviderMessage::developer(reminder));
        }
        messages
    }

    /// native 预留前缀：同一个内容单源，稳定层使用 `system`，当前用户输入后置 `user` 级 `<system-reminder>`。
    pub(crate) fn native_prefix(&self, truncation_note: Option<&str>, user: Value) -> Vec<Value> {
        let mut messages = Vec::new();
        messages.extend(system_messages_of(&self.base));
        messages.extend(system_messages_of(&self.domain_profile));
        messages.extend(system_messages_of(&self.semistable));
        messages.extend(system_messages_of(&self.memory));
        messages.push(user);
        messages.extend(self.native_volatile(truncation_note));
        messages
    }

    pub(crate) fn native_volatile(&self, truncation_note: Option<&str>) -> Vec<Value> {
        let reminder = wrap_system_reminder(&self.volatile.render(truncation_note));
        if reminder.is_empty() {
            return Vec::new();
        }
        vec![json!({
            "role": "user",
            "content": reminder,
        })]
    }
}

/// 把一段文本按 native 稳定层口径包装（`role = system`）。
pub(crate) fn system_messages_of(texts: &[String]) -> Vec<Value> {
    texts
        .iter()
        .map(|text| json!({ "role": "system", "content": text }))
        .collect()
}

/// 将 `ProviderMessage` 切片转换为 OpenAI/DeepSeek 兼容的 `native_messages` JSON 数组：
/// 用于 Ask 恢复等原本 `native_messages` 为空、但需要追加原生 `[assistant(tool_call), tool(result)]` 的场景。
pub(crate) fn native_messages_from_provider_input(
    input: &[crate::agent::provider::ProviderMessage],
) -> Vec<Value> {
    use crate::agent::provider::ProviderRole;
    input
        .iter()
        .map(|message| {
            if matches!(message.role, ProviderRole::Developer) {
                if let Some(reply) = message.content.strip_prefix("Previous assistant reply: ") {
                    return json!({ "role": "assistant", "content": reply });
                }
                if is_system_reminder_message(&message.content) {
                    return json!({ "role": "user", "content": message.content });
                }
            }
            let role = match message.role {
                ProviderRole::System | ProviderRole::Developer => "system",
                ProviderRole::User => "user",
            };
            json!({ "role": role, "content": message.content })
        })
        .collect()
}
