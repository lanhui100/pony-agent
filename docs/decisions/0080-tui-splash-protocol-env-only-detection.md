# 0080 TUI 品牌标图形协议改为零 I/O 环境判定：禁止启动期 stdio 能力查询

Status: implemented

> 品牌标布局尺寸（本文第 5 条的 26×15 字符矩形）已由 [0105](0105-tui-splash-logo-scale-third.md)
> 修订为 9×5；零 I/O 环境判定、图形协议白名单与字符画回退部分仍现行。

## 背景

[0069](superseded/0069-tui-splash-image-protocol.md) 决定「终端图形协议 + 位图资产 + 字符画回退」，并把协议
选择交给 `ratatui-image` 的 `Picker::from_query_stdio()` 在启动时探测。上线后该探测被证实是**真实缺陷源**，
而非单纯的偏好问题：

1. **启动期吃键**。`from_query_stdio()` 内部另起线程对 stdin 做阻塞 `read`（超时 1s），超时后线程不被回收，
   继续从 stdin 偷字节。用户在启动瞬间敲入的字符被直接吞掉（strace 实证：读走了 `abc`），且这类丢失对用户
   不可见、不可恢复。
2. **破坏 raw mode**。同一路径会自行调用 `disable_raw_mode()`，把上层刚建立的 raw mode 关掉，事件循环被降级
   到行缓冲——与 [0075](0075-tui-kitty-keyboard-protocol-restore-hardening.md) 修好的键盘协议路径互相踩踏。
3. **探测结果本身不可信**。在复用器 / pty 代理层内，能力查询的回包可能被吞或错配，探测结论与实际渲染能力
   常常相反（0069 后果节已记录 tmux 内误报）。

结论：品牌标是**装饰性内容**，不值得用输入可靠性去换；探测这件事应该被彻底移出启动路径，而不是加超时、
加线程回收补丁。

## 候选方案

- **方案 A：保留 stdio 探测，加更短超时 + 线程 abort**。落选：线程卡在阻塞 `read` 上无法安全 abort，
  仍会吃掉探测窗口内的真实按键；缩短窗口只会降低概率，不消除缺陷。
- **方案 B：启动前暂停输入队列，探测后回灌**。落选：需要接管 crossterm 事件源并缓存未解析字节，
  复杂度高且仍然要读 stdin；一旦回灌顺序出错，等于制造更难查的输入错位。
- **方案 C：完全删除图形协议路径，回到纯字符画**。落选：支持图形协议的终端上锯齿问题真实存在（0069 背景），
  为了一个可绕过的探测缺陷牺牲已落地的能力，属过度收缩。
- **方案 D：环境判定 + 保守白名单 + 显式覆盖入口（采纳）**。判定只读环境变量与终端拓扑，零 I/O；
  默认 fail-closed 回退字符画；用户可用 env 强制指定协议与字符格像素。
- **方案 E：用 `TIOCGWINSZ` 像素字段替代 stdio 探测来做能力判定**。落选：像素尺寸只能说明字符格大小，
  不能说明终端实现哪种**图形编码**，不作为协议判据（只用于位图尺寸换算，见决策 4）。

## 决策

1. **删除启动期 stdio 能力查询**。`init_splash_image()` 不再调用 `Picker::from_query_stdio()`；
   源码级回归守卫 `splash_gradient_tests::splash_probe_never_reads_stdin` 禁止该调用形态重新出现。
2. **唯一决策点 `splash_protocol_for_env(&SplashEnv) -> Option<ProtocolType>`**，纯函数、无 I/O：
   - 命中复用器 / pty 代理层（`HERDR`/`TMUX`/`ZELLIJ`/`STY`/`ABDUCO` 任一非空）⇒ `None`（回退字符画）；
   - kitty 优先：`KITTY_WINDOW_ID`、`TERM` 含 `kitty`、`TERM_PROGRAM` 含 `ghostty`/`wezterm`；
   - 其次 iTerm2：`TERM_PROGRAM` / `LC_TERMINAL` 含 `iterm`；
   - 最后 sixel：`TERM` 含 `sixel`，或 `TERM` 含 `foot`/`contour`/`mlterm`/`yaft`/`mintty`；
   - 其余 ⇒ `None`。
   顺序即风险排序：kitty 的 APC 序列在不支持的终端上被静默忽略（最坏只保留字符画），sixel 误判会喷像素乱码，
   故 kitty 先命中、sixel 只认明确声明支持的终端。
3. **显式覆盖入口**（优先级高于自动判定）：`PONY_TUI_SPLASH_IMAGE_PROTOCOL=kitty|sixel|iterm2|halfblocks|off`。
   `off` 关闭图形路径；`halfblocks` 强制半块位图协议（母版真色渲染，ADR 0098 起不再等同 `off`）；
   `auto`/缺省走决策 2。映射实现为纯函数 `splash_protocol_override`（零 I/O，可独立单测）。
4. **字符格像素尺寸零 I/O 获取** `splash_font_px()`：`PONY_TUI_SPLASH_FONT_PX=WxH` > `TIOCGWINSZ`
   的 `ws_xpixel/ws_ypixel` 换算 > 默认 8×16。`Picker::from_fontsize()` 不发送任何查询序列。
5. **布局不变**：图形路径占 26×15 字符矩形（`SPLASH_IMAGE_COLS × SPLASH_IMAGE_ROWS`，ADR 0098 起
   由 28×11 改为 26×15 以对齐母版宽高比），位图矩形与周围 Canvas 无缝衔接；字符串位置与引导块布局沿用 0069。
6. **验收契约**：
   - 单测 `splash_protocol_detection_is_env_only_and_fail_closed`（13 例表驱动）+ 源码级守卫
     `splash_probe_never_reads_stdin`；
   - 黑盒 `crates/pony-agent-core/tests/tui_startup_input_guard.rs`（真实二进制 + python PTY）：
     `g1` 启动期零能力查询字节、`g2` 启动瞬间键入 `abc` 不被吞、`g0` 负向对照（假程序故意发 `CSI c` 必须被抓到）；
   - `tools/tui-audit/check_shift_enter.py` 断言 `a2.6_startup_sends_no_capability_query`
     并记录 `capability_probe_bytes`。

## 后果

- 启动路径**零探测字节**：终端不会收到 `CSI ? u` / `CSI c`，启动瞬间的按键不再被吞，raw mode 不再被降级。
- 图形协议判定从「问终端」变成「读环境」：误判代价被压到「支持却用字符画」（可见但不伤功能），
  反向风险（不支持却发 sixel）用保守白名单 + kitty 优先规避。
- 复用器内默认回退字符画；需要图形直通时用 `PONY_TUI_SPLASH_IMAGE_PROTOCOL` 显式开启（配合复用器的
  passthrough 开关）。这是把「猜测」换成「用户声明」的取舍。
- 失去对「实现了 sixel 但 `TERM` 不含白名单关键字」终端的自动识别，需 env 兜底；新增终端支持时改白名单表。
- 取代 [0069](superseded/0069-tui-splash-image-protocol.md) 的决策 3（探测）与后果节的探测相关结论；
  0069 其余决定（图形协议 + 位图资产 + 字符画回退 + 显式覆盖入口）由本篇承接重述。

## 验证回执（2026-11-07）

- 单测：`cargo test -p pony-agent-core --bin pony splash` → 8 passed / 0 failed
  （含 13 例表驱动 `splash_protocol_detection_is_env_only_and_fail_closed` 与源码级守卫 `splash_probe_never_reads_stdin`）。
- 黑盒：`cargo test -p pony-agent-core --test tui_startup_input_guard` → 2 passed / 0 failed
  （`g0` 负向对照：假程序故意发 `CSI c` 必须被抓；`g1` 启动期零能力查询字节；`g2` 启动瞬间键入 `abc` 不被吞）。
- 审计：`python3 tools/tui-audit/check_shift_enter.py` → `pass=6 fail=0 exit=0`，
  `a2.6_startup_sends_no_capability_query` 的 `capability_probe_bytes=[]`。
- 图形路径未回归（强制 `PONY_TUI_SPLASH_IMAGE_PROTOCOL=kitty`，40×140 PTY）：kitty APC `\x1b_G` × 52、
  composer 正常渲染、无 panic、能力查询字节 0 —— 说明 env-only 判定没有削掉既有图形能力。
- 观测归属：`docs/observability.yaml` 追加 `wave-tui-splash-no-probe` 归属登记（四维归属不变，metrics/logs 结构性 N-A）；
  `audit-diff.sh --decl-approved` → exit 0（`[OK] audit-diff: 无越界实现、收口点完整`）；
  `check-declaration.sh .` → exit 2，缺口为 tracing `T4:PARTIAL`（「接了却从未到过数据」），
  **用 `HEAD` 版声明复跑结果完全相同**，即该 FAIL 属既有基线，与本篇无关。
- 回归面（同一二进制）：`tui_terminal_restore` 38 passed、`tui_six_fixes_acceptance` 36 passed、
  `tui_startup_input_guard` 2 passed、`tui_enter_key_wave40` 31 passed；`tui_render_layout` 35 passed / 2 failed、
  `tui_overlay_cancel_acceptance` 38 passed / 5 failed。失败断言全部指向**并行未提交的 header `~/` 与指标行布局改动**
  （代码注释称 ADR 0078，ADR 文件尚未落库），分别为「指标行左侧须为折叠 cwd」「row0 逐格不变 `~/…`」
  「Ask 面板高度 4..=12，实际 16（cap 已改为 40/24）」「恢复视图 description 优先」；
  其实现 hunk（`workspace_header_label_with_home`、`metrics_line`、`inline_ask_height`、`message_lines_for`）
  与本篇改动 hunk（`init_splash_image`/`splash_*`）不重叠，故不属本篇引入，也未在本篇顺手修改。
