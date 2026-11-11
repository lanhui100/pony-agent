# Agent Note: TUI 退出还原与 `pony resume <session-id>` 续接入口

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0062-tui-exit-restore-and-resume-entry.md` 之归档快照，仅供溯源。

## 背景

TUI 退出后用户在 shell 里输入任何键都变成 `ESC[117u`、`ESC[9;5u` 这类转义序列（终端看起来「输入全是乱码」）。
根因是进程退出时只撤了「自己确认 push 成功」的模式：`pop keyboard enhancement` 被一个
`keyboard_enhancement_pushed` 共享标志门控，而鼠标追踪（`?1000h/?1002h/?1003h/?1015h/?1006h`）、
bracketed paste（`?2004h`）、备用屏与光标显示分散在多条 `let _ = execute!(...)` 里；任一步失败或提前
返回（`?` / panic / SIGINT / SIGHUP），终端就停在 kitty 键盘协议态或 raw 态——shell 已回到行模式，
终端却继续把每个按键编码成 CSI 序列回显，于是「敲什么都乱码」。

同时会话是易失的：退出后 `session_id` 只存在于进程内存，用户没有可复制的续接入口，
下一次启动只能空会话，历史要靠 `/resume` 弹窗翻找。

## 候选方案

- **方案 A：只在退出时补发 `PopKeyboardEnhancementFlags`**。落选：只修了键盘协议一条，鼠标 any-motion /
  bracketed paste / 备用屏残留仍会让 shell 收到转义序列，且 panic、信号、`?` 提前返回等路径照旧漏还原。
- **方案 B：依赖 crossterm 的 Drop / `disable_raw_mode`**。落选：raw mode 只覆盖 termios，不管终端私有模式；
  进程被信号终止时不跑 Drop。
- **方案 C：退出时按「push 是否成功」分支还原**。落选：标志只反映 push 返回值，不反映终端真实状态
  （用户先手动 `printf '\e[>4;0m'`、或 push 半成功时判断为假 ⇒ 整段还原被跳过）。
- **方案 D（采纳）：无条件写全量还原串 + 三通道冗余**。输入侧还原串（kitty 弹栈两种写法、modifyOtherKeys、
  五级鼠标追踪、bracketed paste）与屏幕侧还原串（退出备用屏 + 显示光标）分开；raw mode 生效前快照 termios
  并预打开 `/dev/tty`，正常退出走 RAII 守卫、panic 走 hook、SIGINT/SIGTERM/SIGHUP 走 async-signal-safe
  的 `write(2)+tcsetattr(3)`。
- **方案 E：退出提示写成 `pony --resume <id>` 选项**。落选：与既有 `ponycli -r/--resume` 的旗标语义重复，
  且子命令形式（`pony resume <id>` / `pony resume` 取最近会话）更贴合「复制即执行」的提示诉求。
- **方案 F：只在会话已落库时打印提示**。落选：新会话未产生轮次时无持久化记录，用户退出后仍拿不到任何入口；
  采纳「提示恒打印 + 未知 id 以该 id 继续并显式说明」的语义，保证提示永不假承诺、也不留空窗。

## 决策

1. **还原串单源**：`TERMINAL_INPUT_MODES_RESET`（`ESC[<1u`、`ESC[<u`、`ESC[>4;0m`、`?1000l/?1002l/?1003l/?1006l/?1015l`、
   `?2004l`）与 `TERMINAL_SCREEN_RESET`（`?1049l`、`?25h`）是唯一定义；一律无条件写全，不看 push 结果。
2. **写入通道优先 `/dev/tty`**：stdout 被重定向或返回 EIO 时仍能送达；raw mode 之前预打开，信号处理里只用
   `write(2)` 与 `tcsetattr(3)`，不做任何分配。
3. **三条退出路径同一闭合**：`TerminalGuard`（Drop，覆盖 `?` 提前返回）→ panic hook（覆盖 unwind 前）→
   信号处理器（覆盖 SIGINT/SIGTERM/SIGHUP）。三者幂等，重复执行无副作用。
4. **退出提示在还原之后打印**：`继续该会话请运行：pony resume <session-id>` 经 `/dev/tty`（兜底 stdout）输出；
   先还原再打印，否则提示会落进备用屏。
5. **`pony resume [<session-id>]`**：缺省取最近一次会话；显式 id 一律接受——已存在则装载权威快照
   （history + turnTraceHistory + workspace 归属），不存在则沿用该 id 开新会话并在界面如实说明。
   装载逻辑与 TUI 内 `/resume` 共用 `hydrate_state_from_session`，不复制第二份。
6. **`/clear` 与启动恢复互不越界**：会话作用域复位仍由 `reset_session_scope` 负责，`hydrate` 只负责装载。

## 影响

- **用户可见行为**：退出 TUI（`/quit`、`/exit`、Ctrl-C、信号）后终端输入恢复正常；屏幕打印可直接复制的
  `pony resume <session-id>`；`pony resume` 能续接历史。
- **契约面**：`CliAction` 新增 `ResumeTui(Option<String>)`；`tui_event_loop` 返回最终 `session_id`；
  `resume_hint_line` 为提示文案的单源（可单测）。
- **跨平台**：非 unix 分支保留 crossterm 命令式还原（老 conhost 下 ANSI 直写不可靠），并补 `write_restore_bytes`
  的 stdout 兜底。
- **验收**：`crates/pony-agent-core/tests/tui_terminal_restore.rs`（还原串组成与入口形态，8 passed）、
  `crates/pony-agent-core/tests/tui_resume_cli.rs`（CLI 解析与提示文案，8 passed）；
  真实 PTY（`/quit`、Ctrl-C、SIGTERM 三路径 + kitty 应答 + 退出后输入）取证见
  `.dev-team/diagnosis/tui-repro.md`（本机 tmux/pty 原始捕获，非入库脚本）。

