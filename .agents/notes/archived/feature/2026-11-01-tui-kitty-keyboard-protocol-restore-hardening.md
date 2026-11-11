# Agent Note: TUI 键盘协议还原加固：显式置零、复用器最小化 push、失败可告知

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0075-tui-kitty-keyboard-protocol-restore-hardening.md` 之归档快照，仅供溯源。

## 背景

ADR [0062](0062-tui-exit-restore-and-resume-entry.md) 已把「退出 TUI 后终端输入变乱码」的还原收敛为
「无条件写全量还原串 + 三通道冗余」。上线后仍在真实环境复现，现场证据：

1. 用户在 **herdr 0.7.5** 的 pane 里运行 pony。herdr 自己不解析 kitty 键盘协议，只把字节转发给外层
   真实终端：`push`（`CSI > 9 u`）能透传，`pop`（`CSI < 1 u`）是否被对称转发不受 pony 控制。
2. 退出后 shell 里按任意键都被编码成 `CSI <code>;<mods> u` 并当正文回显 —— 表现为「按 Enter 得到 `u`，
   按 3 得到 `3u`，按 7 得到 `7u`」。这正是 `REPORT_ALL_KEYS_AS_ESCAPE_CODES`(8) 的编码特征。
3. 存在还原串**整串丢失**的路径：写 `/dev/tty` 返回 EIO，退回 stdout 同样 EIO（pane 已被关闭 /
   SIGHUP），日志里 `Input/output error (os error 5)` 是实证。

旧还原串的两个结构性缺口：

- **只有栈语义的 pop**（`ESC[<1u` / `ESC[<u`）：栈是相对状态，push/pop 被代理层错配一次，弹多少都清不干净，
  且没有一条「与栈无关」的绝对还原。
- **push 的键位面过大**：`DISAMBIGUATE(1) | REPORT_ALL_KEYS(8)` 会让**每个**按键都变成 CSI 序列。
  Shift+Enter 只需要 DISAMBIGUATE；REPORT_ALL_KEYS 把「pop 丢失」的后果从「修饰键异常」放大成「整个键盘乱码」。

## 候选方案

- **方案 A：只把 pop 重复几次**。落选：仍是相对操作，栈深未知时次数只能靠猜；代理层吞掉全部 pop 时无效。
- **方案 B：退出时按 push 结果决定 pop 次数**。落选：push 的返回值只反映本地 `execute!` 是否成功，
  不反映经代理层到达外层终端的真实栈状态。
- **方案 C（部分采纳）：`CSI = 0 u` 显式置零**。与栈无关地直接把键盘协议标志设为 0，是唯一的绝对还原；
  但它不能撤销鼠标追踪 / bracketed paste / raw mode，所以仍需与既有还原串并存。
- **方案 D：检测到复用器就完全不 push**。落选：Shift+Enter 会退化成裸 Enter（＝直接发送），
  用普通键位上报的代价换掉一个真实可用的功能，不可接受。
- **方案 E：还原失败时静默 + 只写日志**。落选：用户此刻正卡在「敲不出命令」的终端里，
  日志在他看不到的文件里；不给一条可复制的恢复命令，等于把排障成本转嫁给他。
- **方案 F（采纳）**：置零与重复弹栈并存 + 复用器下最小化 push + 还原失败可告知。

## 决策

1. **还原串「弹栈兜底 + 置零收尾」**：`TERMINAL_INPUT_MODES_RESET` 改为
   `ESC[<1u`×3 → `ESC[<u` → **`ESC[=0u`** → `ESC[>4;0m` → `?1000l/?1002l/?1003l/?1006l/?1015l` → `?2004l`。
   置零必须排在弹栈**之后**：弹栈会把栈里保存的旧标志写回终端，顺序反了等于没清。
   重复三次是兜底（代理层可能吞掉或错配一次 pop），弹栈不足时由置零绝对收尾。
2. **push 按终端拓扑分级**：`keyboard_enhancement_push_flags_for(term, multiplexer)` 是唯一决策点。
   - `TERM` 缺失 / `dumb` ⇒ 不 push；
   - 复用器 / pty 代理层（`HERDR`、`TMUX`、`ZELLIJ`、`STY`、`ABDUCO` 任一非空）⇒ **只**
     `DISAMBIGUATE_ESCAPE_CODES`(1)：Shift+Enter 照常可用，而「pop 丢失」的后果从「全键盘乱码」降级为
     「修饰键异常」；
   - 直连终端 ⇒ `DISAMBIGUATE | REPORT_ALL_KEYS`（保留既有全键位上报能力）。
   环境判定走纯函数内核 `terminal_multiplexer_for_env`，可在单测注入环境视图，不改全局 `env`。
3. **还原失败必须可告知**：`restore_terminal_modes` 用 `TERMINAL_RESTORE_INCOMPLETE` 记录「输入侧还原串有没有
   送达」，写通道在「预打开 `/dev/tty` fd → 重开 `/dev/tty` → stdout」三级间降级；退出提示由纯函数
   `resume_hint_block(session_id, restore_incomplete)` 生成：续接行恒在，**仅当**还原不完整时追加一条
   可直接执行的手工恢复命令（`reset`，应急 `printf '\033[<1u\033[=0u\033[>4;0m'`，用 POSIX printf 不用 `$'...'`）。
   正常退出不会多印这行 —— 否则会误导用户怀疑终端坏了。

## 影响

- **用户可见行为**：herdr/tmux 等复用器下退出 TUI 后不再出现「按键变 `u`/`3u`」；万一还原串仍没送达，
  退出提示会给出可复制的恢复命令而不只是一句失败日志。Shift+Enter 换行在所有拓扑下保持可用。
- **契约面**：`TERMINAL_INPUT_MODES_RESET` 字节内容变更（顺序与条目都参与断言）；新增
  `keyboard_enhancement_push_flags_for` / `keyboard_enhancement_push_flags` / `terminal_multiplexer_for_env` /
  `terminal_multiplexer` / `terminal_recovery_hint_line` / `resume_hint_block` / `terminal_restore_incomplete`。
  `resume_hint_line` 语义不变（单行、含 `pony resume <id>`），仍为提示文案单源。
- **跨平台**：Windows 分支逐条 crossterm 命令无可靠失败回传，统一标记「已完整还原」，不附恢复命令。
- **验收**：`crates/pony-agent-core/tests/tui_terminal_restore.rs` C1–C10（置零顺序、弹栈次数、复用器分级、
  环境探测、恢复文案单行可执行、提示块行序与条件出现、还原幂等）；关联套件
  `tui_enter_key_wave40` / `tui_six_fixes_acceptance` / `tui_resume_cli` / `tui_session_resilience_acceptance` 全绿。

## 验证收据

- `cargo test -p pony-agent-core --test tui_terminal_restore` ⇒ 25 passed / 0 failed。
- `cargo test -p pony-agent-core --test tui_enter_key_wave40 --test tui_six_fixes_acceptance --test tui_resume_cli --test tui_session_resilience_acceptance`
  ⇒ 18 / 19 / 26 / 23 passed，0 failed。
- `cargo check -p pony-agent-core --bin pony` ⇒ 0 warning / 0 error。
- 真实 pty 抓包（`TERM=xterm-256color`，分别置 `TMUX` 与不置）：push 序列在复用器下为 `ESC[>1u`、
  直连为 `ESC[>9u`，退出送回 `ESC[<1u`×3 + `ESC[=0u`，见 `.dev-team/diagnosis/tui-repro.md` 同法取证。

