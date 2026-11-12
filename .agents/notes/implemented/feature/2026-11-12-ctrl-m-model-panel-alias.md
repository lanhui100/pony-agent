# Agent Note: TUI 模型面板快捷键 Ctrl+R 增设 Ctrl+M 别名（kitty 协议终端生效）

Status: implemented

- 日期：2026-11-12
- 交付面：CLI/TUI（`crates/pony-agent-core/src/bin/tui.rs`）
- 收口点：`tui.rs` 的「面板类快捷键」分支（`KeyCode::Char('r') | KeyCode::Char('m')`）
- 相关决策：[herdr 复用器变量名漂移与 DISAMBIGUATE 分级 push](../bug-fix/2026-11-08-tui-enter-key-drift-and-session-row-heal.md)
  —— 本别名只在真正 push 且被终端遵守 `DISAMBIGUATE` 时生效；herdr 等不解析 kitty 协议的复用器下
  `Ctrl+M` 仍按该条的「分级 push」口径落在 Enter（回退），不构成本条的新承诺。

## Problem

模型面板的打开快捷键只有 `Ctrl+R`（wave-35b，与 `/models` 同路径）。部分用户向评审反馈：
`R` 位于键盘右侧、单手操作不如左手区顺手，希望补一个更顺手、且不被"以 r 开头的输入"占用的
别名键。但任何"新快捷键"候选都必须先过一道硬约束——**`Ctrl+M` 与 `Enter` 在大多数终端里
是同一个字节（0x0D）**：终端层不区分二者时，按下 `Ctrl+M` 等于回车提交，若把它绑定成面板
快捷键会在这些终端上"吞掉回车"，属于不可接受的回归。

## Decision

给模型面板快捷键增加 **`Ctrl+M` 别名，保留 `Ctrl+R` 主键**；`Ctrl+M` 仅在**支持 kitty 键盘
协议（`DISAMBIGUATE_ESCAPE_CODES`）的终端**生效，其余终端维持"Ctrl+M = Enter（提交输入）"
的既有字节语义，即天然回退、不吞键、不额外探测。

实现细节（`tui.rs` 面板快捷键分支，Normal / 流式期共用同一路径，C3 流式守卫对 `Forward` 放行）：

```rust
KeyCode::Char('r') | KeyCode::Char('m') => { /* 打开模型面板并远程刷新 */ }
```

生效机制与回退口径：

- 本 TUI 已在启动时按 `terminal_multiplexer()` / `TERM` 决定是否 push `DISAMBIGUATE_ESCAPE_CODES`
  （`tui/input.rs::keyboard_enhancement_push_flags`，Shift+Enter 换行同依赖该标志）。
  kitty 协议兼容终端（kitty、wezterm、foot 等）在该标志下把 `Ctrl+M` 与 `Enter` 区分上报为
  `Char('m')` + `KeyModifiers::CONTROL`，本分支据此命中别名；
- 其余终端（或复用器/内嵌终端对标志静默忽略）仍把 `Ctrl+M` 发送为 0x0D，crossterm 解析为
  `KeyCode::Enter` → 走既有 Enter 提交/排队语义。**不会出现"按 Ctrl+M 却弹出模型面板"或
  "回车被吞"**——"不支持即回退为 Enter"由终端层字节事实保证，不需要运行时能力探测。
- 不带 Shift/Alt 限制的必要性：kitty 终端上 `Ctrl+Shift+M` 上报为 `Char('M')` + CONTROL|SHIFT，
  用 `Char('m')` 字面匹配天然排除，不会误触发。
- 不改动面板内 `r`/`R` 刷新键（`tui.rs` SelectingModel 分支 9036 处）与 `Ctrl+T` 任务看板键。

## Alternatives considered

- **只保留 Ctrl+R，不做改动**：满足"沿用已有习惯"，但不解决"R 不顺手的操作面"诉求，且用户
  明确要求补别名，落选。
- **改用 Alt+M（所有终端可用）**：`Alt` 与小键盘/组合键在部分终端被布局或用例（如 Meta 键、
  终端特权键）占用；更重要的是本 TUI 已把裸 `Alt+Enter`/`Alt+↑/↓` 等纳入语义（换行/轮跳转），
  `Alt+M` 与"以 m 结尾的快速组合"无冲突但引入新修饰键族，收益不如"复用 kitty 协议既有
  能力"直接。落选。
- **运行时探测"终端是否真支持 DISAMBIGUATE"再决定是否注册别名**：探测本身要再发一轮 "CSI ? u"
  往返（正是旧 `supports_keyboard_enhancement()` 被弃的理由），且探测结果与按键上报由两次往返
  各自决定，时序上可能漂移；"由终端字节事实隐式门控"更简单且与既有 Shift+Enter 的实现对齐。落选。
- **在 flow 层做"是否已成功 push 标志"的软状态拦截**：push 成功不等于终端遵守，软状态会给出
  误导性"支持"结论；信任终端层对 0x0D vs Char('m')+CONTROL 的区分本身就是最可靠的门。落选。

## Consequences

- 用户可见：kitty 协议终端上 `Ctrl+M` 与 `Ctrl+R` 均可打开模型面板（流式期同样可用）；其余
  终端 `Ctrl+M` 行为不变（Enter 提交）。`/help` 与 `/models` 命令描述已同步标注别名与 kitty 前提。
- 观察面：纯按键语义变更，无新增网络 IO、后台任务、环境变量、依赖、指标或日志出口；
  四维观测归属维持结构性 N-A，`docs/observability.yaml` 已追加非遥测登记段。
- 测试：`tui_streaming_input_acceptance.rs` C3 断言扩展为 `['t', 'r', 'm']`，锁定"流式中
  `Char('m')`+CONTROL 必须 `Forward` 到面板分支、不得被当提交/换行吞掉"。
- 机器可验承诺的对账命令：
  - `cargo test -p pony-agent-core --test tui_streaming_input_acceptance a3_panel_and_session_mutating_command_classification -- --exact`
  - `cargo check -p pony-agent-core --bin tui`
  - `bash .agents/skills/write-adr/verify-note.sh`