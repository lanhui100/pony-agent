# Agent Note: TUI 首帧先绘制、控制面后台装载（ADR 0115 实施记录）

Status: implemented

- 日期：2026-11-10
- 决策真源：[`docs/decisions/0115-tui-first-frame-before-control-plane.md`](../../../docs/decisions/0115-tui-first-frame-before-control-plane.md)
- 交付面：`pony`（TUI 二进制）

## Problem

用户报告「TUI 初始化页很慢」。实测首帧：空库 462/484ms、真实库 3649/3699ms、真实库 + 真实
session-logs 4988/5150ms。根因不是渲染：`run_tui` 在 `tui_event_loop` 前**同步**跑
`build_control_plane()` → `SessionStore::new()` → `load_store()`（整库装载 + 会话日志 fold），
装载期间终端停在备用屏空白态。`phase.txt` 采样证明读盘 2404ms 结束（769MB）、首帧 3471ms 才出现，
即「首帧时刻 = 装载结束时刻」。

## Decision

1. `bin/tui.rs` 新增 `build_control_plane_after_first_frame(terminal, state)`：
   - 起 `pony-tui-control-plane` 线程跑 `build_control_plane`，结果经 `mpsc` 回主线程；
   - 主线程先 `terminal.draw(|f| ui(f, state))` 画初始化页（`TuiState::default()` 的
     `messages` 为空 ⇒ 天然走 splash 分支）；
   - 等待期 `recv_timeout(80ms)` 超时即重绘并刷新 `正在装载会话库…` 状态提示（每帧续期，避免
     `STATUS_NOTICE_TTL_MS` 提前吞掉提示）；
   - 收到结果清空提示后返回；`Disconnected`（线程 panic）⇒ 启动失败，走既有 `TerminalGuard` 还原。
2. `run_tui` 的调用点改为该函数；`build_control_plane` 本体零改动（演示/审计入口仍直接调用）。
3. 不并入库体积/WAL 议题（`B046`）：A/B 实测 freelist 空洞对首帧读取字节零影响。

## Alternatives considered

详见 ADR 0115「候选方案」：
- B 定向查询 `load_store_normalized`（**实测否决**：156 行全部为 `__index_only`，日志 fold 只覆盖
  当前 cwd 分组，`index_projection_filled: 152~155` ⇒ targets ≈ 全量，收益 ≈ 2% 却新增查询变体）；
- C 只画一帧再同步装载（装载期界面冻结，改动量相同收益更小）；
- D 完全懒装载（收益最大 −4.5s，但推翻「索引可读」契约，需逐消费方改造，不能一次吞下）；
- E 预加载/守护进程（引入跨进程生命周期，与本缺陷无关）。

## Consequences

- 机器回执（pty + tmux，最终二进制，同沙箱各三轮）：真实库 + 真实日志首帧
  **52/53/53ms**（基线 4988~5150ms），装载完成 4353/4451/4383ms；真实库无日志 **50/57ms**
  （基线 3649~3699ms）；空库 **60ms**（基线 462~484ms）。单测 `cargo test -p pony-agent-core
  --bin pony`：112 passed / 0 failed。
- 冒烟：9s 后 `正在装载会话库…` 已消失、splash 正常、输入回显 ok、运行日志无 `panicked`。
- 当前未做（已登记 backlog）：延迟 trace / 懒装载（预计再省 ~1.1s、~220MB 读取，需先解「索引可读」
  契约）；`B046` 处理 freelist 空洞与 WAL。
- 残余风险（靠 review）：等待期重绘与装载线程同进程，存在内存分配器/页缓存竞争的可能；
  三类沙箱实测未见装载变慢，低配机器仍需观察。
