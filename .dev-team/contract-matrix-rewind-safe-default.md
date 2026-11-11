# 契约矩阵: 撤回面板安全默认项改造 (ISSUE-20261111-005)

## 1. 契约条款清单

| 编号 | 模块 | 承诺/规则 | 验证方式 |
| :--- | :--- | :--- | :--- |
| **C-REWIND-01** | `tui::rewind` | `RewindState::new` 初始化后的 `scope()` 必须为 `RewindMode::ConversationOnly`（安全默认，最小破坏性）。 | 单元测试：`scope_rows_default_to_safe_conversation_only` |
| **C-REWIND-02** | `tui::rewind` | `REWIND_SCOPE_ROWS` 首项（行下标 0）必须为「仅恢复会话」(`ConversationOnly`)，次项（行下标 1）必须为「恢复代码与会话」(`ConversationAndFiles`)。 | 单元测试：`scope_rows_order_is_safe_first` |
| **C-REWIND-03** | `tui::rewind` | 在范围选择阶段直接按回车（`KeyCode::Enter`），确认执行的撤回模式必须是 `RewindMode::ConversationOnly`。 | 单元测试：`scope_stage_enter_defaults_to_conversation_only` |
| **C-REWIND-04** | `tui::rewind` | 在范围选择阶段按数字键 `1` 选择并确认「仅恢复会话」，按数字键 `2` 选择并确认「恢复代码与会话」。 | 单元测试：`scope_stage_number_keys_pick_and_confirm` |
| **C-REWIND-05** | `tui::rewind` | 按方向键下移（`KeyCode::Down` / `'j'`）能主动切换到「恢复代码与会话」，并在回车后以 `RewindMode::ConversationAndFiles` 确认。 | 单元测试：`scope_stage_down_arrow_selects_destructive_mode` |
| **C-REWIND-06** | `docs` | 同步修订 `docs/decisions/0113-tui-esc-double-press-rewind.md` 的 D2 描述，将默认项更新为安全项，消除事实腐化。 | 文档审查与决策一致性校验 |
