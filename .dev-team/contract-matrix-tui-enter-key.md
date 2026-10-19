# 契约矩阵 —— TUI Composer Shift+Enter 换行（wave-40）

## 0. 症状与根因（Lead 已核实）
- 用户实测：TUI 输入框里 `Shift+Enter` 会**发送**而不是换行。
- 代码里已有分支：`crates/pony-agent-core/src/bin/tui.rs:3559-3566`
  ```rust
  if key.code == KeyCode::Enter
      && (key.modifiers.contains(KeyModifiers::SHIFT)
          || key.modifiers.contains(KeyModifiers::ALT))
  { state.insert_newline(); continue; }
  ```
  → 分支本身正确，**但默认终端不启用 kitty 键盘增强协议**：多数终端在未收到 `CSI > 2 u` 时把 Shift+Enter 与 Enter 都编码为裸 `\r`，crossterm 解析为无修饰 Enter，于是走提交路径。
- 证据：`tui.rs:2987-2991` 仅 `enable_raw_mode` + `EnterAlternateScreen` + `EnableMouseCapture`，**无** `supports_keyboard_enhancement()` / `PushKeyboardEnhancementFlags` / `PopKeyboardEnhancementFlags`（`grep` 0 命中）。
- crossterm 0.28.1 提供：`terminal::supports_keyboard_enhancement() -> io::Result<bool>`（unix，2s 超时，终端不支持时返回 Err）、`event::PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)`、`PopKeyboardEnhancementFlags`；CSI u 序列解析不依赖是否 push（`event/sys/unix/parse.rs:203,497`），因此可用 PTY 注入做真实 E2E。

## 1. 契约面（Protocol Agent 唯一编写，纯空桩）
```rust
/// wave-40 R1：Composer 中 Enter 键语义（可测纯函数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerEnterAction { Submit, Newline }
pub fn composer_enter_action(key: &crossterm::event::KeyEvent) -> ComposerEnterAction;

/// wave-40 R2：根据终端支持探测结果决定是否 push 键盘增强标志（可测纯函数）。
pub fn keyboard_enhancement_flags(supported: bool) -> Option<crossterm::event::KeyboardEnhancementFlags>;
```
- `composer_enter_action`：`KeyCode::Enter` 且 modifiers 含 `SHIFT` 或 `ALT` → `Newline`；`KeyCode::Enter` 其他 → `Submit`；非 Enter → `Submit`（调用点只用于 Enter 分支，语义固定便于断言）。
- `keyboard_enhancement_flags(true) == Some(DISAMBIGUATE_ESCAPE_CODES)`；`false == None`。

## 2. 实现契约（Executor）
### R3 启用/恢复终端键盘增强
- `run()`（`tui.rs:2987` 附近）：`enable_raw_mode()` + `EnterAlternateScreen/EnableMouseCapture` 之后：
  - `let enhancement = keyboard_enhancement_flags(crossterm::terminal::supports_keyboard_enhancement().unwrap_or(false));`
  - 若 `Some(flags)` → `execute!(terminal.backend_mut(), PushKeyboardEnhancementFlags(flags))`（失败仅忽略，不 panic、不阻塞启动）。
  - 环境变量 `PONY_KEYBOARD_ENHANCEMENT=0/1` 强制关闭/开启探测结果（0 = 不 push；1 = 直接 push DISAMBIGUATE，不探测）——用于不响应探测的终端。
- 退出恢复（`tui.rs:3018` 附近，与 `LeaveAlternateScreen/DisableMouseCapture` 同处）：若已 push → `execute!(..., PopKeyboardEnhancementFlags)`；必须位于 `disable_raw_mode` 之前或同批次，任何退出路径（Ctrl-C、/quit、panic catch）都要执行。
- 不改既有 Shift+Enter/Alt+Enter 分支语义；不改 Ctrl+J 换行；不改 Ctrl+C 语义。
### R4 事件循环接线
- 把 `tui.rs:3559-3566` 的内联条件替换为 `if composer_enter_action(&key) == ComposerEnterAction::Newline { state.insert_newline(); continue; }`，行为不变。

## 3. 验收契约（Test Agent）
### A1 单元矩阵（新文件 `crates/pony-agent-core/tests/tui_enter_key_wave40.rs`）
- `Enter`（无修饰）→ `Submit`
- `Shift+Enter` → `Newline`
- `Alt+Enter` → `Newline`
- `Ctrl+Shift+Enter` → `Newline`（modifiers 含 SHIFT）
- `Shift+Char('x')` → `Submit`（非 Enter 不触发换行）
- `keyboard_enhancement_flags(true)` → `Some(DISAMBIGUATE_ESCAPE_CODES)`；`false` → `None`
### A2 真实 PTY E2E（`tools/tui-audit/check_shift_enter.py`，物理凭证）
- 用 `pty` 启动 `target/debug/pony`（隔离 HOME/XDG），读取输出；
- 当收到探测序列 `ESC[?u`（可伴随 `ESC[c`）时，回写 kitty 支持响应 `ESC[?1u` + `ESC[c`（主设备属性响应）；
- 断言 1：随后应用输出含 `ESC[>2u`（push DISAMBIGUATE）；
- 断言 2：发送 `abc` + `ESC[13;2u`（Shift+Enter）后，屏幕文本仍含 `abc` 且 composer 出现第二行（未提交、未清空）；
- 断言 3：随后发送 `\r`（裸 Enter）后，composer 清空/进入提交路径（不再保留 `abc` 在输入区）；
- 断言 4：发送 `Ctrl+C` 两次后进程退出、终端恢复、无 panic 文本。
- 脚本 Exit 0 = 全断言通过；输出 `.dev-team/evidence/wave40-shift-enter/{screen-before.txt,screen-after-shift-enter.txt,screen-after-enter.txt,report.json}`。
- 注：PTY 内的终端响应由脚本伪造，等价于支持 kitty 协议的终端；不依赖真实 tmux/桌面终端。

## 4. Non-Goals
- 不改桌面端（Tauri/Vue）键盘处理；不改 IME 组合态逻辑。
- 不改其它键位（Ctrl+J、Esc、Tab）。
- 不支持「终端完全不支持键盘增强协议」时识别 Shift+Enter（硬件/协议限制，无法区分）；该情形保留 Alt+Enter 换行与 `PONY_KEYBOARD_ENHANCEMENT=1` 兜底。
