# Agent Note: TUI 状态行按真实事件给出上游静默心跳（5s / 15s 两档）

Status: implemented

## Problem

现场会话 `pa_01a1df2f-8315` 末段单次 provider 交互静默约 180s（`providerCallRecords` 可证），期间 TUI 状态行只显示「生成中 + 轮次总耗时」：

- 没有任何**阶段**信息：无法区分「正在建连」「已请求、等首字节」「正在解码增量」；
- 没有**最近一次活动时间**：轮次总耗时只说明「这一轮跑了多久」，不能说明「上游已经多久没动静」；
- 450ms 节拍只是在重画同一个状态词与同一秒数（`tui.rs` 主循环 `tick_ms`），用户无法把「慢」与「死」区分开。

`TurnStreamEvent.phase` 早已存在，`turn:started` 与 `turn:trace` 都携带 `phase="calling_model"`（真实「模型调用开始」事件），`turn:delta` 也是真实增量事件——但 TUI 的 `TuiTurnSink::emit` 把 `turn:trace` 落在 `_ => {}` 丢弃，且 `TuiState` 没有「最后活动时刻」字段。

## Decision

在纯逻辑层（`bin/tui/status_dwell.rs`）新增上游活动状态机，在呈现层只做接线：

1. `UpstreamPhase { RequestSent, FirstByte, Delta }` + `UpstreamIdle { phase, last_activity_ms }`：
   - `note_request_sent(now_ms)`：`turn:trace` 且 `phase == "calling_model"` 时调用（每次模型调用/retry 都是新的请求）；
   - `note_delta(now_ms)`：`turn:delta` 到达时调用；首次 delta 把阶段推进到 `FirstByte`，其后为 `Delta`；
   - `reset()`：`turn:started` 与终态事件（completed/failed/cancelled/suspended）复位；
   - `silent_ms(now_ms)` / `hint(now_ms)`：时间由调用方注入（与 `StatusDwell` 同一口径，单测可喂假时间）。
2. 纯函数 `stall_hint(phase, silent_ms) -> Option<String>`，两档阈值常量：
   - `< 5s`（`UPSTREAM_STALL_HINT_MS`）⇒ `None`：正常增量间隔不制造噪音；
   - `5s..<15s` ⇒ 提示档：给出阶段（等待首字节能/读流中）与静默秒数；
   - `≥ 15s`（`UPSTREAM_STALL_ALERT_MS`）⇒ 告警档：文案与提示档可区分，明确「上游已静默 Ns」，提示可中断。
3. 呈现层只在 `TurnLifecycle::Running` 时把 hint 拼到状态行（与 ADR 2026-11-16-tui-turn-lifecycle-status-line 的生命周期门同源）；hint 不参与 `StatusDwell` 的驻留比较（否则秒数变化会拖住状态词切换）。

## Alternatives considered

- **A. 只在 `elapsed_label` 后追加「已等待 N 秒」**：落选。轮次总耗时已每帧刷新，同义文案是噪音；缺失的是**阶段**与**静默时长**两个不同量。
- **B. 后端新增 `turn:first_byte` 事件**：落选（本轮）。需要改事件持久化契约与四端消费；现有真实事件已足够表达阶段：`calling_model`（请求已发出）+ 首个 `delta`（首字节已到）。若将来需要精确 TTFT 心跳，再按事件演进门禁加。
- **C. 用墙钟时间戳判断静默**：落选。墙钟会被 NTP 跳变/休眠影响，且违反「纯逻辑时间注入」约定；进程内单调时钟已满足。
- **D. 固定单阈值（如 10s）**：落选。5s/15s 两档区分「偶发慢」与「真的没动静」，单阈值要么太吵要么太晚；两档常量集中在 `status_dwell.rs`，调参只改一处。
- **E. 在渲染点直接读 `Instant::now()` 计算**：落选。渲染点无法单测阈值边界；时间注入是既有 `StatusDwell` 的既定做法。

## Consequences

- 长静默期状态行从「生成中 16:52」变成「生成中 16:52 · 仍在等待上游首字节…（已静默 8s）」，15s 后升级为告警档；用户可据此决定等或中断。
- 阶段只来自真实事件（`turn:trace(calling_model)` / `turn:delta`），无事件时 `hint` 恒 `None`——不伪造静默、不伪造阶段。
- 阈值边界与阶段切换由 `status_dwell.rs` 模块内单测覆盖（4999/5000/14999/15000），不依赖渲染快照。
- 不改 provider 超时与重试预算；本卡只解决「看得见」。
- 无新增对外接口、网络 IO、线程或后台任务；`docs/observability.yaml` 四维归属不变。
