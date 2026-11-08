//! 模型面板统一导航空间（ADR 0100）：优选栏前置 + 全部模型，索引与渲染行一一对应。
//!
//! 旧实现里优选行仅展示、`selected_model_index` 只覆盖 `model_list`，导致优选模型
//! 看得见选不中。本模块把索引空间收敛为「优选栏行数 + 全部模型行数」，高亮 / ↵ 确认 /
//! Ctrl+D / Ctrl+F 共用同一映射；同一模型允许在两栏各出现一次。
//!
//! 消费纪律：面板选中一律经 `model_panel_item_at` / `model_panel_index_of` 解释，
//! 禁止再把 `selected_model_index` 直接当 `model_list` 下标。

use super::*;

/// 统一导航空间总行数。
pub fn model_panel_len(state: &TuiState) -> usize {
    state.model_list.len() + preferred_count(state)
}

/// 优选栏行数（不克隆条目，供导航边界与索引偏移使用）。
fn preferred_count(state: &TuiState) -> usize {
    state
        .model_list
        .iter()
        .filter(|item| state.model_preferences.is_preferred(&item.id))
        .count()
}

/// 统一索引 → 模型条目：小于优选行数命中优选栏，否则偏移后命中全部模型列表。
pub fn model_panel_item_at(state: &TuiState, index: usize) -> Option<ModelItem> {
    let pref_count = preferred_count(state);
    if index < pref_count {
        model_preferred_items(state).get(index).cloned()
    } else {
        state.model_list.get(index - pref_count).cloned()
    }
}

/// 模型 id → 统一索引（优选栏优先）：Ctrl+F 切换优选 / 列表刷新后重锚选中项，
/// 保证索引空间变化时选中身份不漂移。
pub fn model_panel_index_of(state: &TuiState, id: &str) -> Option<usize> {
    let preferred = model_preferred_items(state);
    if let Some(pos) = preferred.iter().position(|item| item.id == id) {
        return Some(pos);
    }
    state
        .model_list
        .iter()
        .position(|item| item.id == id)
        .map(|pos| preferred.len() + pos)
}