# Agent Note: TUI 首帧先绘制、控制面后台装载（启动感知时间与真实装载时间解耦）

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0115-tui-first-frame-before-control-plane.md` 之归档快照，仅供溯源。

日期：2026-11-10
实施说明：[`.agents/notes/implemented/architecture/2026-11-10-tui-first-frame-before-control-plane.md`](../../.agents/notes/implemented/architecture/2026-11-10-tui-first-frame-before-control-plane.md)。
关联：[0106](superseded/0106-session-store-single-writer-fail-closed-and-startup-budget.md)（启动装载预算）/ [0107](0107-session-scoped-writer-lease-and-log-first-durability.md)（每次装载重新起算自愈预算）语义不变；[0112](0112-tui-splash-logo-legibility-floor.md)（初始化页渲染口径）不变。

## 背景

用户报告「TUI 启动时初始化页面很慢」。实测（release 二进制、真实库 1.2GB / 153 会话 / 88k 事件、
真实 session-logs 652MB、pty 内测量）**首帧**耗时：

| 场景 | 首帧 |
|---|---|
| 空 HOME + 空库 | 462 / 484 ms |
| 真实库（无日志目录） | 3649 / 3699 ms |
| 真实库 + 真实 session-logs | 5150 / 4988 ms |

根因不在渲染：`run_tui` 在 `tui_event_loop` 之前**同步**执行 `build_control_plane()` →
`SessionStore::new()` → `load_store()`，把整库装载 + 会话日志 fold 全部做完才画第一帧。
装载期间终端停留在备用屏的空白态，用户把「装载慢」感知成「初始化页慢」。

`phase.txt` 物证（每 213ms 采样 `rchar` + 是否已绘制）：读盘在 2404ms 停在 769MB，直到 3471ms
才 `painted` —— 最后约 1s 是反序列化/折叠的 CPU，随后首帧才出现。也就是说**首帧时刻 = 装载结束时刻**。

## 候选方案

- **A（采纳）首帧先画 + 控制面后台线程装载**：主线程先 `terminal.draw(ui)` 出初始化页，装载在
  `pony-tui-control-plane` 线程里跑；等待期每 80ms 重绘（响应 resize、保持界面活性）并刷新
  「正在装载会话库…」状态提示；收到结果即进入 `tui_event_loop` 并清掉提示。
  装载失败的语义与同步版**一致**：线程 panic ⇒ 通道断开 ⇒ 启动失败；不降级成空控制面、不重试。
- **B（落选，本 ADR 记录否决证据）把 `fill_index_only_rows_from_index_projection` 的
  `load_store_normalized` 改为按 `session_id` 定向查询**：实测无效。`sessions` 表 156 行**全部**是
  `__index_only` 轻量行，而日志 fold 只覆盖**当前 cwd 分组**的日志（8 个分组中 1 个），
  运行日志 `index_projection_filled: 152~155` ⇒ targets ≈ 全量。定向查询最多省 1~4 个会话（约 2%），
  却要新增一套过滤 SQL 变体（维护面 + 回归面），违背「不为零收益改共享查询」。
  真正的下一刀是**延迟/按需装载**（见「后续」），属独立决策。
- **C（落选）只在装载前画一帧、然后同步装载**：首帧同样 <100ms，但装载期界面完全冻结
  （连 resize 都不响应），用户看到的是一张不动的图 4~5s；与 A 的改动量相同，收益更小。
- **D（落选）把 `SessionStore` 改成完全懒装载（启动只读索引行，打开会话再读内容）**：收益最大
  （实测清空 `normalized_turn_traces` + `session_turn_traces` + `turn_events` 后首帧 462ms，
  即 −4.5s），但会推翻「索引可读」契约（`load_store` 的降级分支与
  `fifty_logged_sessions_load_within_budget_and_degrade_keeps_the_index_complete` 用例守着它），
  且 `SessionStore::sessions` 是列表/标题/摘要的唯一数据面，需要逐消费方改造。不是一次提交能安全吞下的改动。
- **E（落选）预加载到内存缓存/守护进程**：引入跨进程生命周期与失效协议，与本缺陷（启动顺序）无关。

## 决策

1. `crates/pony-agent-core/src/bin/tui.rs` 新增 `build_control_plane_after_first_frame()`：
   装载线程 + 首帧立即绘制 + 80ms 重绘节拍 + 「正在装载会话库…」提示（每帧续期，TTL 不会提前吞掉）。
2. `run_tui` 的 `build_control_plane(...)` 调用点改为该函数；`build_control_plane` 本体一字未改
   （仍是唯一真值源，演示/审计入口继续直接调用）。
3. **总装载时间不在本 ADR 的验收面内**：本 ADR 只把「感知时间」与「真实装载时间」解耦。
   装载本身仍约 4.3~4.7s（与基线持平，说明后台化没有引入 CPU/IO 回归）。
4. freelist 空洞与 WAL 膨胀**不并入**本决策：A/B 实测空洞对首帧读取字节零影响（同一快照
   1.2GB vs VACUUM 后 730MB，首帧前累计读盘均为 3083MB），属磁盘占用项，登记为 `B046`。

## 后果

- 机器回执（pty + tmux，最终二进制，同一沙箱各三轮）：真实库 + 真实日志首帧
  **52 / 53 / 53 ms**（基线 4988~5150ms），装载完成 4353 / 4451 / 4383 ms；
  真实库无日志首帧 **50 / 57 ms**（基线 3649~3699ms）；空库 **60 ms**（基线 462~484ms）。
  开发期中间版本同口径测得 56~111ms，量级一致。
- 回归守护：装载线程 panic 时 `RecvTimeoutError::Disconnected` ⇒ 启动失败并走既有 `TerminalGuard`
  还原路径；`PONY_TUI_RUNTIME_LOG` 内的 `panicked` 在 9s 冒烟中为 0。
- 残余风险（靠 review）：等待期重绘与装载线程共享同一进程（内存分配器/页缓存竞争）；
  实测在三类沙箱上未出现装载变慢（4.3~4.7s 与串行基线持平），但极端低配机器上仍建议观察。
- 后续（已登记 backlog）：延迟 trace / 懒装载（预计再省 ~1.1s 与 ~220MB 读取），
  需先解「索引可读」契约；`B046` 处理库体积与 WAL。

