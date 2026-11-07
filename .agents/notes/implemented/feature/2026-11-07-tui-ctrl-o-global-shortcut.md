# Agent Note: TUI `Ctrl+O`「隐藏 Agent 过程」升级为全局快捷键

Status: implemented

## Problem

`/settings` 面板与 `Ctrl+O` 都切换同一个 `hide_process` 偏好字段（契约 C12）。但 `Ctrl+O`
的判定当时落在事件循环的「面板类快捷键块」里，而该块排在**所有模态分支之后**：只要设置面板、
会话/模型/Provider 选择器、审批、问答等模态打开，模态分支就先把按键吸收并 `continue`，
`Ctrl+O` 根本走不到。用户诉求是：**面板/选择器打开时按 `Ctrl+O` 也要能切换过程显示**。

## Decision

- 新增纯判定 `is_hide_process_shortcut(code, modifiers)`（`crates/pony-agent-core/src/bin/tui/input.rs`，
  经 `pub use self::input::*;` 重导出），只认「恰好 `CONTROL + Char('o')`」。
- 在 `UiEvent::InputKey` 分发的最前面（与 F2/F3 同级、**所有模态分支之前**）消费该键：
  切换 `hide_process` → `persist_tui_preferences` 落盘 → `set_status_notice` 一次性提示 → `continue`。
- 删除原先面板快捷键块里那份 `KeyCode::Char('o')` 分支（避免死代码/双份真值）。
- 契约矩阵 C12 记「修订 R2（全局化）」；`docs/observability.yaml` 追加归属登记（四维不变、无新增收口点）。

## Alternatives considered

- **新增独立组合键（如 `Ctrl+Shift+O`）**：会引入第二个键位记忆负担，且用户明确要求沿用 `Ctrl+O`。
- **在每个模态分支内部各自加一份 `Ctrl+O` 处理**：N 个模态 × 一份逻辑，必然漂移；且新增模态时容易漏。
- **保持现状、只在文档里说明「面板内不可用」**：与用户诉求（面板打开时也要生效）直接冲突。
- 选「前置单一判定」：一处判定覆盖全部模态，新增模态自动继承，且顺序可用源码护栏机械校验。

## Consequences

- 任意模态下 `Ctrl+O` 均即时切换过程显示；行为与 `/settings` 同字段、同落盘。
- 新增验收 `crates/pony-agent-core/tests/tui_ctrl_o_global_shortcut.rs`：纯函数真值表 +
  **源码顺序护栏**（消费点必须早于全部模态分支）+ settings 不吞 `Char('o')` + 字段同源。
- 无新增网络 IO / 后台任务 / LLM 调用 / 依赖；metrics / logs 维持结构性 N-A。
