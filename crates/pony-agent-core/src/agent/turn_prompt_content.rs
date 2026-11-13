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
    /// normalized 预留前缀：稳定层 + 易变段 + 当前输入，顺序与 `flatten_layered_input_messages` 一致。
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
        messages.push(ProviderMessage::developer(self.volatile.render(truncation_note)));
        messages.push(user);
        messages
    }

    /// native 预留前缀：同一个内容单源，只换角色（native 侧稳定层全为 `system`）。
    pub(crate) fn native_prefix(&self, truncation_note: Option<&str>, user: Value) -> Vec<Value> {
        let mut messages = Vec::new();
        messages.extend(system_messages_of(&self.base));
        messages.extend(system_messages_of(&self.domain_profile));
        messages.extend(system_messages_of(&self.semistable));
        messages.extend(system_messages_of(&self.memory));
        messages.push(json!({
            "role": "system",
            "content": self.volatile.render(truncation_note),
        }));
        messages.push(user);
        messages
    }

    pub(crate) fn native_volatile(&self, truncation_note: Option<&str>) -> Vec<Value> {
        vec![json!({
            "role": "system",
            "content": self.volatile.render(truncation_note),
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

