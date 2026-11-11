# ISSUE-20261111-005 交付报告 (Delivery Receipt)

## 1. 缺陷概述与根因
- **Issue ID**: `ISSUE-20261111-005`
- **标题**: 撤回面板默认高亮「恢复代码与会话」（破坏性最强项），只想改代码的用户直接回车即回滚了工作区文件
- **根因**: `REWIND_SCOPE_ROWS` 中下标 0 被硬编码为 `("恢复代码与会话", RewindMode::ConversationAndFiles)`，`RewindState::new()` 默认 `selected_scope = 0`，导致两段式确认中连击 Enter 直接执行工作区代码物理回退。

## 2. 变更内容
- **实现文件**: `crates/pony-agent-core/src/bin/tui/rewind.rs`
  - 调整 `REWIND_SCOPE_ROWS` 首项为 `("仅恢复会话", RewindMode::ConversationOnly)`，次项为 `("恢复代码与会话", RewindMode::ConversationAndFiles)`。
  - 调整 `RewindState::scope()` 的 fallback 默认模式为 `RewindMode::ConversationOnly`。
  - 键盘操作完全一致：Enter 默认安全模式；数字 1 对应安全模式，数字 2 对应文件还原模式；Down 键可主动下移选择文件还原模式。
- **单测增强**: `crates/pony-agent-core/src/bin/tui/tests/rewind_tests.rs`
  - 新增/重构 4 个用例，严格锚定安全行序、直接回车确认模式、方向键切换模式及数字快捷键映射。
- **文档同步**: `docs/decisions/0113-tui-esc-double-press-rewind.md`
  - 同步修订 D2 决策说明，更新默认项为安全项。

## 3. 验收结果
- **红相证据**: `.dev-team/rewind-red-receipts.md`（4 项断言精准失败，Exit 101）
- **绿相证据**: `.dev-team/rewind-green-receipts.md`（14/14 单元测试通过，Exit 0）
- **回归测试**: `tests/tui_rewind_acceptance.rs`（3/3 端到端用例全绿，Exit 0）
- **决策门禁**: `npm run decisions:check`（119 篇 ADR 一致，Exit 0）
