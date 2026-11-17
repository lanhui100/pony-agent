# Agent Note: followup 三段静默窗口用「阶段通道」披露（与重试计数正交）

Status: implemented

## Problem

`pa_01a1df2f-8315-7090-8a4c-978934c4d9cb` 的实测证据（`tui-runtime.log`）：

- `followup:stream-fallback reason=读取 provider SSE 流失败 ... elapsed=75520ms`（已解析 71882 字节后断流）；
- 随后 `followup_sync_fallback attempt=1` 连接失败 `elapsed=31000ms`；
- `attempt=2..5` 各 `elapsed=21161ms` → `Escalate(attempt_limit_exhausted)`；
- 最终由 `local_tool_followup_fallback_response` 合成兜底正文，全程约 **190s**。

界面在这 190s 内只有 `provider_retry` 的「正在重试（n/5）」，缺失三段真实事实：

1. **阶段切换**：流式已开始产出后断流、改为同步兜底（此时并没有“重试计数”可显示）；
2. **首次同步尝试**：`RetryNotifier` 只在 `RetryDecision::Retry` 分支派发，首次尝试无计数可报；
3. **降级收尾**：本地兜底摘要被当作正常 assistant 正文，用户无法区分“模型回答”与“兜底拼接”。

B220 的 `stall_hint` 只回答“静默多久”（时间维度），补不上“卡在哪个阶段”（流程维度）；
B219 已修“降级轮仍记 completed”，但过程中间仍无阶段文案——两者互补，不可互相替代。

## Decision

在库层新增一条**与重试正交**的阶段通道，事实与措辞分离：

1. **事实（库层）**：`agent::retry::ProviderPhase`（`StreamFallback` / `SyncAttempt{attempt}` /
   `LocalFallback`）+ `install_phase_observer` / `notify_provider_phase`，
   与既有 `RetryObserver` **逐字同构**：`thread_local`、非阻塞、未安装即 no-op。
   调用点只在真实分支内（流式失败转同步前、首次同步请求发出前、本地兜底前），不伪造进度。
   阶段枚举刻意不含 URL / elapsed / provider 名——库层不承载 UI 文案。
2. **措辞（展示层）**：`bin/tui/status_dwell.rs::followup_phase_text` 纯函数，
   三档固定中文短语，带单测断言“不含 URL / elapsed、三档可区分”。
3. **接线（已落地，B198-b）**：`UiEvent::ProviderPhase(ProviderPhase)` 变体由
   `install_tui_phase_observer` 在轮次线程与重试观察者**成对安装**；主循环写入
   `TuiState::provider_phase`，`status_word_base` 在「重试计数之后」呈现阶段短语
   （计数更精确，故计数优先）；`turn_lifecycle` 把阶段位与重试位同权纳入 `Running`，
   保证 `runtime_mode` 已回终态时仍在途可见；清空复用 `clear_provider_retry()`
   （阶段位与重试位生命周期同源，避免只清一半的陈旧残留）。
   关键取舍：**不新增第二条 UI 通道**——阶段枚举随既有 `UiEvent` 单通道流动，
   `status_dwell::followup_phase_text` 是唯一措辞点。

## Alternatives considered

- **扩展 `RetryNotice` 加阶段字段**：重试通知的语义是“已失败且即将重试”，把首次尝试与阶段切换
  塞进去会让“重试计数”失真（`attempt/max_attempts` 会被同一次尝试重复消费），且 UI 需要再拆解语义。
  否决。
- **库层直接产出用户文案**：库层将被打上 UI 措辞依赖，CLI/桌面端无法各自措辞；且违背“同一事实一个家”
  （事实在库层、措辞在展示层）。否决。
- **在库层直接 `send` 新的 UI 事件通道**：重试/阶段循环位于库层深处，拿不到 turn 层的
  `TurnEventSink`（sink 以 `&impl` 逐层传参，无法存进 `'static` thread_local）；
  且新增线程/通道会破坏既有单线程同步执行假设。否决。
- **仅靠 B220 的 5s/15s 心跳**：心跳只能表达“静默了多久”，无法表达“已从流式降级到同步兜底”，
  用户仍会把降级等待误读为“模型在慢慢生成”。否决。

## Consequences

- 观察者只在**宿主轮次线程**安装才生效；CLI / 测试 / 子代理零副作用（与 `RetryObserver` 一致）。
- 该通道只覆盖 followup 路径；decision 路径的阶段披露如需要，按同一模式复用，不另建机制。
- 用户可见效果已随 B198-b 接线交付：断流 / 首次同步 / 本地兜底三阶段各有固定短语，
  且与 B220 的 5s/15s 静默心跳叠加（阶段在前、时长在后），不再出现「整段无反馈」。
- 回归口径：`cargo test -p pony-agent-core --bin pony running_status_visibility_tests`（含 `b198_*` 3 例）。

## Related

- `2026-11-14-upstream-retry-count-and-status-surface.md`（重试计数通道，本条的**正交**补充）
- `2026-11-16-tui-upstream-stall-heartbeat.md`（静默时长心跳，本条的**流程维度**补充）
- `2026-11-16-degraded-turn-reason-and-trace.md`（降级终态，本条披露降级的**过程**）
- `2026-11-16-provider-fallback-body-layering.md`（兜底正文分层，本条不改正文形态）
