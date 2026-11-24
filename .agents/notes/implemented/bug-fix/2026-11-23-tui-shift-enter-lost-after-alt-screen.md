# Agent Note: TUI Shift+Enter/Ctrl+Enter 失效 —— 键盘增强 push 早于备用屏，被 pty 代理层作废

Status: implemented

## Problem

现场（herdr 0.7.5 pane 内运行 `pony`）：输入框里 **Shift+Enter 与 Ctrl+Enter 既不换行也不发送**，
按键被完全吞掉；裸 Enter 提交正常，Ctrl+J 换行正常。

机器证据分三步取得（均在 herdr 真实 pane 内，`HERDR_ENV=1` 生效）：

1. **代理层实际下发的字节**（把 pane 前台进程换成只 `os.read` 并落盘的探针）：
   - `Shift+Enter` → `1b 5b 32 37 3b 32 3b 31 33 7e` = `CSI 27;2;13~`（xterm **modifyOtherKeys**）
   - `Ctrl+Enter` → `CSI 27;5;13~`
   - 裸 `Enter` → `0d`；`Ctrl+J` → `0a`
2. **crossterm 0.28.1 不解析 `CSI 27;mod;code~`**：`event/sys/unix/parse.rs` 对 `~` 结尾只认
   `1|7`→Home、`2`→Insert、`3`→Delete、`4|8`→End、`5|6`→PgUp/PgDn、`11..=15|17..=24|25..` → F(n)、
   `200|201` → 括号粘贴；首位参数 `27` 落入 `_ => None` ⇒ **整串被丢弃**（所以既不换行也不提交）。
   升级 crossterm 0.29.0 亦无该分支（已下载 crate 源码核对），故「升级依赖」不是选项。
3. **对照实验定位触发条件**（同一 pane 内换探针，逐项只改一个变量）：

   | 探针写入序列 | 代理随后对 Shift+Enter 的编码 |
   |---|---|
   | `CSI >1u`（push，无备用屏） | `CSI 13;2u`（kitty，**正确**） |
   | `CSI >1u` + `CSI ?1000h`（鼠标） | `CSI 13;2u`（正确） |
   | `CSI >1u` + `CSI ?1049h`（备用屏） | `CSI 27;2;13~`（**modifyOtherKeys，被丢弃**） |
   | `CSI ?1049h` + `CSI ?1000h` + `CSI ?2004h` + `CSI >1u`（备用屏在前） | `CSI 13;2u`（**正确**） |

   即：**pty 代理层在 pane 进入备用屏的瞬间清掉它逐 pane 记录的 kitty 标志**；而 `pony` 的启动序列是
   `push → EnterAlternateScreen`，push 恰好被这次清空作废，代理退回 modifyOtherKeys。
   这也解释了 Ctrl+Enter 同时失效（同一条退回路径），以及裸 Enter / Ctrl+J 不受影响（不依赖修饰键消歧）。

## Decision

1. **顺序不变量**：键盘增强 push 必须晚于 `EnterAlternateScreen`。
   - `tui.rs` 的两个入口（`run_tui` 与演示入口）：`execute!(EnterAlternateScreen, EnableMouseCapture,
     EnableBracketedPaste)` 之后再 `PushKeyboardEnhancementFlags`。
   - `tui/terminal.rs::terminal_reentry_plan`：`RawMode → AlternateScreen → KeyboardEnhancement →
     MouseCapture → BracketedPaste → ClearAndRedraw`（挂起恢复时同样不能把 push 放在备用屏之前）。
2. **Enter 语义统一**：`is_newline_enter` = `Enter` 且 modifiers 含 `SHIFT / ALT / CONTROL` 任一 ⇒ 换行；
   裸 Enter ⇒ 提交。Shift+Enter 是对外宣称键位；Alt+Enter 是传统终端兜底；Ctrl+Enter 在 kitty 协议下
   上报为 `CSI 13;5u`，若仍按提交处理，用户「想换行」的手势会被直接发出去。流式期裁决
   （`streaming_key_disposition`）复用同一判定，避免两处分叉。
3. **回归守卫**：
   - `tools/tui-audit/check_shift_enter.py` 新增 `a2.7_keyboard_enhancement_push_after_alternate_screen`
     （真实 PTY：push 字节偏移必须晚于 `ESC[?1049h`）；
   - `tests/tui_enter_key_wave40.rs` 补 `Ctrl+Enter → Newline`；
   - `tests/tui_signal_reentry.rs` 断言重入计划里 `AlternateScreen` 早于 `KeyboardEnhancement`。

## Alternatives considered

- **A：升级 crossterm 到 0.29 让 `CSI 27;2;13~` 被解析**。落选：0.29.0 的 `parse.rs` 同样没有
  modifyOtherKeys 分支（已核对源码），升级不解决问题且会牵动 ratatui 后端矩阵。
- **B：自研 stdin 解析器（或 `dup2` 管道前置改写 `CSI 27;mod;code~` → `CSI 13;modu`）替代/包裹
  crossterm 事件源**。落选：要重新实现鼠标、括号粘贴、resize、DECCKM 等解析，故障面远大于收益；
  且 `enable_raw_mode` 依赖真实 tty，管道化会让 raw mode 与 winsize 一起失配。
- **C：herdr 下干脆不 push 键盘增强**。落选：Shift+Enter 会退化成裸 Enter（＝直接发送），
  用删功能换稳定，与 ADR 0075 的取舍同样不可接受。
- **D：只改 herdr（让代理在备用屏期间保留 kitty 标志）**。落选：herdr 属外部工具、不随本仓发版；
  且「push 放在能力重建的最后一步」对任何 pty 代理层都更稳，没有理由依赖代理侧修复。
- **E：保留 Ctrl+Enter = 提交**。落选：kitty 协议下 Ctrl+Enter 与「换行」手势无法区分意图，
  而提交已有裸 Enter 与队列自动提交两条路径，多这一个键只会误发。

## Consequences

- 用户可见：herdr（及其他会重置 pane 键盘标志的 pty 代理层）内 Shift+Enter 恢复换行，Ctrl+Enter 亦换行；
  裸 Enter 仍是唯一提交键；挂起（Ctrl+Z）恢复后能力不丢。
- 代价：push 与备用屏的相对顺序成为必须人工维护的不变量 —— 已用 PTY 脚本 + 两个 Rust 测试钉住。
- 残余风险：终端若**完全不支持**键盘增强协议（老式 xterm、部分 Windows 终端），Shift+Enter 仍等价于
  裸 Enter；该场景保留 Alt+Enter / Ctrl+J 两条换行回退（协议限制，应用层无法区分）。
