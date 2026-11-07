# TUI Ctrl+Z 挂起前后终端交还与能力重建闭环

Status: implemented

## Context

`pony` TUI 进入 raw mode 后，Ctrl+Z 的行为有三处实质缺陷，且互相掩盖：

1. **终端不交还**：SIGTSTP 走默认处置直接停进程，终端留在 raw mode + 备用屏 + kitty 键盘
   增强协议栈。用户回到 shell 后每个按键都被编码成 `ESC[117u` / `ESC[9;5u` 这类转义序列
   当正文回显，终端看起来"输入全是乱码"，只有 `fg` 触发 SIGCONT 才可能修复。
2. **恢复不完整**：旧 SIGCONT 路径只 `enable_raw_mode()`。备用屏没重进 ⇒ 后续帧直接画进
   用户 shell 的主屏滚屏区；鼠标 / bracketed paste / kitty 协议栈都没装回 ⇒ 能力单向丢失。
3. **Ctrl+Z 根本到不了信号层**：raw mode 由 `cfmakeraw` 建立，它清掉 `ISIG`，于是 Ctrl+Z
   不会产生 SIGTSTP，而是作为 `Char('z')+CONTROL` 从 crossterm 送到事件循环——接管 SIGTSTP
   只能覆盖「外部 `kill -TSTP`」与「进 raw 之前的窗口」，用户真正按键的路径完全没有拦截。

此外 `enable_raw_mode` 在「crossterm 自认为已是 raw」时直接返回 `Ok(())`；而信号处理器用
`tcsetattr` 绕过 crossterm 缓存，两者状态分叉 ⇒ `fg` 之后单发 `enable_raw_mode` 静默 no-op。

## Decision

1. **挂起 = 先还原、再停进程**：`handle_sigtstp_signal` 内调 `restore_terminal_for_signal()`
   （只 `write(2)` 还原串 + `tcsetattr` 回原 termios，async-signal-safe），随后把 SIGTSTP
   复位 `SIG_DFL` 并 `raise(SIGSTOP)`（不可捕获，必定停住）。
2. **按键路径主动挂起**：新增 `is_suspend_shortcut(code, modifiers)`（纯函数，只认恰好
   CONTROL + `Char('z')`，非 unix 恒 `false`）与 `suspend_tui()`；主循环 `InputKey` 分支在
   Ctrl+C 判定之前拦截。`suspend_tui` 用 `restore_terminal_modes()`（可 flush、可报错、会清
   crossterm 的 raw mode 缓存）再 `raise(SIGSTOP)`。
3. **恢复 = 一次性请求 + 显式计划**：SIGCONT 处理器只 `request_terminal_refresh()`（原子置位）
   并重新武装 SIGTSTP；主循环每 tick `take_refresh_request()`（单次 `swap`，防丢请求）后按
   `terminal_reentry_plan(mouse_capture)` 逐项重建，末尾 `ClearAndRedraw`。
4. **强制重进 raw mode**：`reenter_raw_mode()` 先 `disable_raw_mode()` 清 crossterm 缓存再
   `enable_raw_mode()`（对已被处理器还原的 termios 幂等）。
5. **不动用户选择**：F2 关过鼠标捕获时计划跳过 `MouseCapture`，恢复执行不得悄悄改回。

## Alternatives considered

- **只接管 SIGTSTP，不管按键路径**：改动最小，但 `ISIG` 被 `cfmakeraw` 清掉，用户按 Ctrl+Z
  压根不产生信号，缺陷仍在。这是本项目最初一版的真实错误。
- **挂起时不还原，靠 `fg` 后整屏重绘**：shell 在 raw + 备用屏里根本看不到提示符，用户无法输入
  `fg` 命令；且乱码回显发生在 TUI 恢复之前。
- **信号处理器里调 crossterm API**：crossterm 内部带锁与分配，信号上下文不安全（可能死锁）。
- **重入只 `enable_raw_mode()` 就继续画帧**：raw mode 因缓存分叉静默 no-op，且备用屏未重建，
  帧落进主屏滚屏区。
- **每次 tick 无条件重建终端**：`ClearAndRedraw` 每 tick 一次会持续闪屏，CPU 与视觉都不可接受；
  必须是一次性消费的请求位。
- **把 Ctrl+Z 绑成 undo**：与 POSIX 作业控制直觉冲突，且项目当前无 undo 栈。

## Consequences

- 改动集中在 `crates/pony-agent-core/src/bin/tui.rs`：`request_terminal_refresh` /
  `take_refresh_request` / `is_suspend_shortcut` / `suspend_tui` / `TerminalReentryStep` /
  `terminal_reentry_plan` / `apply_terminal_reentry` / `reenter_raw_mode`。
- `restore_terminal_for_signal` 重新获得真实调用点，`#[allow(dead_code)]` 已移除；两处失真注释
  （"信号处理器已就地还原"、"此刻绝不能再画帧"）同步修正为当前真实分工。
- 验收测试 `crates/pony-agent-core/tests/tui_signal_reentry.rs` R1–R6：一次性消费、多次置位合并、
  计划顺序、鼠标开关尊重、还原串与计划互逆、Ctrl+Z 只认裸组合。
- 已知残余：`Ctrl+Z` 在 Windows 不挂起（无 POSIX 作业控制，判定恒 false）；tmux/screen 下
  kitty 协议弹栈可能不被识别，仍靠 `=0u` 显式置零兜底（沿用 ADR 0075 的结论）。
