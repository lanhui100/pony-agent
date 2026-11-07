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

## 5. 契约勘误（Test Agent 物理验证，2026-10-19）

1. **push 序列为 `ESC[>1u`**，不是 §3 写的 `>2u`：`KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES` bits=1（crossterm-0.28.1 `event.rs:293,494-496`）；`>2u` 是 `REPORT_EVENT_TYPES`。A2 断言改为「push 存在且 disambiguate 位为 1」并记录实际 flags/hex。
2. **PTY 对 `ESC[?u` 的应答必须为 `ESC[?1u` + 主设备属性** `ESC[?1;2c`；裸 `ESC[c` 只是查询，回写会让 crossterm 探测等待 8s 超时。一次性 crossterm 探针已实证：回写 `ESC[?1u`+`ESC[?1;2c` 后 `supports_keyboard_enhancement()==true`。
3. 红相期间断言 2–4（换行/提交/退出）原本已通过：crossterm 对 `CSI 13;2u` 的解析不依赖 push，且既有 Shift/Alt 内联分支本就可达；本波次真正缺失的是 R3 的「探测 + push + 退出 pop」。故 A2 的判别性断言是 #1（probe/push），其余用于防回归。
4. **pop 序列为 `ESC[<1u`**（crossterm-0.28.1 `event.rs:533 csi!("<1u")`）；A2 断言 4 的「push 过必须 pop」应以 `re: \x1b\[<[0-9]*u` 匹配，不得只认裸 `<u`。
5. **A2 断言 1 已改为「无探测 + 无条件 push + 分级」（ADR 0075，2026-11-07）**：应用不再发 `ESC[?u`（`supports_keyboard_enhancement()` 探测会误判并被复用器吞掉），只要 `TERM ≠ dumb` 就在启动期 push；直连终端 push `ESC[>9u`（DISAMBIGUATE|REPORT_ALL_KEYS=9），复用器（`HERDR`/`TMUX`/`ZELLIJ`/`STY`/`ABDUCO` 任一非空）只 push `ESC[>1u`（1）。故 §3 A2 的「等探测 → 回写 `ESC[?1u`+`ESC[?1;2c`」步骤与勘误 2 **整体作废**；现行断言为 `a2.1_no_probe_and_unconditional_push_direct_tier`（无 `ESC[?u` 且 flags==9）与 `a2.5_multiplexer_tier_pushes_disambiguate_only`（`HERDR=0.7.5` 子会话 flags==1）。
6. **harness 曾以 `PONY_TUI_SPLASH_IMAGE_PROTOCOL=off` 运行**（**已作废，见勘误 7**）：`ratatui-image` 的 `Picker::from_query_stdio()` 在自己的探测窗口内直接读 stdin 并丢弃用户按键（实测：启动后首段输入被吞），会让 A2 的输入断言随机变红。当时判为独立产品缺陷（启动期吃键）、不在本契约范围内另行跟踪；该缺陷已于 2026-11-07 修复，规避手段同步删除。
7. **启动期吃键已修复，A2 增补断言 6（ADR 0080，2026-11-07）**：`init_splash_image()` 不再调用 `from_query_stdio()`，改为**零 I/O 环境判定** `splash_protocol_for_env()`——命中复用器/pty 代理层（`HERDR`/`TMUX`/`ZELLIJ`/`STY`/`ABDUCO`）或未知终端 ⇒ 回退字符画；已知终端按风险排序 kitty → iTerm2 → sixel 白名单（`foot`/`contour`/`mlterm`/`yaft`/`mintty` 或 `TERM` 含 `sixel`）。字符格像素 `splash_font_px()` 只读 `PONY_TUI_SPLASH_FONT_PX` 与 `TIOCGWINSZ` 的 `ws_xpixel/ws_ypixel`，同样零探测字节。故：
   - harness 的 `PONY_TUI_SPLASH_IMAGE_PROTOCOL=off` 规避与勘误 6 的「另行跟踪」**整体作废**；
   - 现行断言 `a2.6_startup_sends_no_capability_query`：启动期 stdout 不得出现能力查询字节（`ESC[?u`、`ESC[c`、`ESC[>c`、`ESC[?1;2c` 等），回执字段 `capability_probe_bytes` 必须为空串；
   - 回归守卫：`crates/pony-agent-core/tests/tui_startup_input_guard.rs`（`g0` 负向对照假程序故意发 `CSI c` 必须被抓、`g1` 启动期零能力查询字节、`g2` 启动瞬间键入 `abc` 不被吞）+ 源码级 `splash_gradient_tests::splash_probe_never_reads_stdin`（禁止 `from_query_stdio()` 形态回流）+ 单测 `splash_protocol_detection_is_env_only_and_fail_closed`（13 例表驱动，fail-closed）。
8. **布局锚点与恢复视图折叠态口径变更（ADR 0078 / 0079 / 0081，2026-11-07）**：三条并行未提交特性改动了本契约相关断言的**观察锚点**（不是被测行为），断言随实现口径同步，判据的判别性不变：
   - **指标行锚点 `row33` → `row0`（ADR 0078）**：数值指标从 composer 下底面行迁入 header 右槽，底面行改承载模型身份（右对齐）。故 `tui_overlay_cancel_acceptance` 的 `o1_metrics_fixture_precondition_closed_frame_has_token_metrics` / `o1_history_and_question_share_one_frame_without_wiping_scrollback` 的「指标可见」前置条件与遮挡判据改锚 `row_text(&buf, 0)`；`row33` 的逐字不变量保留，但语义改为「底面行（模型身份）与基线逐字一致」。`tui_render_layout` 的 `header_workspace_label_is_home_folded_folder_name` 同步改为「header 载工作区名、末行载模型身份」，浮层态末行须为空画布行、关闭后恢复模型身份。工作区路径全屏只出现一次（`~/<文件夹名称>`）。
     行尾留白口径（实测校准）：内容列按 `GUTTER=3` 居中，右对齐目标是**内容列**右边缘，故底面行原始文本必然带 3 列外沟槽留白——断言必须用 `trim()` 取「唯一内容是模型身份」+「行尾留白 ≤ 4 列（外沟槽）」证明右对齐，`assert_eq!(row.trim_end(), model)` 这类 raw 比较会把外沟槽误判成左对齐回归（本轮已修）。
   - **同轮修掉一个真实实现缺陷**：header 原先每帧重读 `std::env::var("HOME")`，同进程内 `set_var("HOME")`（`wave65_approval_unit_tests` 隔离沙箱）会让相邻两帧在 `~/x` 与 `x` 之间跳变，导致 O1/O2 的逐格不变量随机变红。现改为 `TuiState::home_dir` 构造期冻结（`Default` 不读 env，`run_tui` / 预览入口注入 `detect_home_dir()`）。**该失败是产品缺陷不是测试缺陷**，已修实现而非放宽断言。
   - **`j1` 恢复视图折叠态口径（ADR 0079）**：`TuiPreferences::default()` = `hide_process: true`（Agent 过程默认隐藏）。同一份 `sessions.db` 下，HEAD 二进制显示「收集路径…」而当前实现不显示——差异来自折叠默认值，不是摘要选择优先级回归。`j1_resume_summary_prefers_description_and_hides_structured_json` 的 harness 因此先按 `Ctrl+O`（`\x0f`）展开过程再抓屏，并新增前置断言 `resume_process_expanded`（屏幕须出现「已显示Agent过程」），把「折叠导致的空屏」与「摘要优先级回归」两类失败区分开。
   - **Ask 面板高度上限 cap 12/8 → 40/24（ADR 0081）**：`o3_ask_panel_survives_wide_narrow_and_tiny_terminals` 的区间断言改为 `(4..=40)`（≥60 列）/ `(4..=24)`（<60 列），并新增「宽终端长问题必须突破旧 12 行上限」的正向断言。旧断言「保留 4..=12 上限」已作废。
