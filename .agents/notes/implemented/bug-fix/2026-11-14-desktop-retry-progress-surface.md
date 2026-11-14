# Agent Note: 桌面端接入上游重试进度真值面（`turn:retry` → 状态行）

Status: implemented

## Problem

上游重试的真值（`agent/retry.rs::RetryNotice`：分子 = 已失败次数，分母 = 本跳尝试总数）此前只
接到了 TUI 与 CLI，**桌面端（`src-tauri`）没有接线**：

1. 三个流式命令入口（`start/continue/resume_graph_run_stream`）最终都落到
   `tauri_adapter::run_stream_task` 的 `spawn_blocking` 闭包里执行回合，但闭包内从未
   `install_retry_observer` ⇒ 桌面进程里观察者恒为空 ⇒ 退避重试期间前端 0 信号。
2. 前端状态行因此只能走降级路径：`src/lib/runtime/status.ts` 的 retry 分支在没有 `retryMax` 时
   渲染「正在重试…」，`src/stores/runtime.ts` 的 getter 也刻意**不转发** `retryAttempt`
   （见 `tests/acceptance/status-flow-contract.spec.ts` 的锁定注释）。用户只能看到一个不动的
   占位文案，无法判断是第几次重试、还剩几次。
3. 前次决策（[`2026-11-14-upstream-retry-count-and-status-surface`](2026-11-14-upstream-retry-count-and-status-surface.md)）
   已把「桌面端尚未安装同一观察者」明确登记为遗留项，并写明接线点是 `run_stream_task` 的
   `spawn_blocking` 闭包 —— 本卡即收口该遗留。

## Decision

1. **装配点钉在回合阻塞线程内**（`src-tauri/src/tauri_adapter.rs::run_stream_task`）：闭包内
   `install_turn_retry_observer(sink, turn_id, session_id)` 安装，返回的 `RetryObserverGuard`
   生命周期 = 该闭包（回合结束即摘除）。理由与库层取舍同源：观察者是 thread_local，一次 provider
   调用（含内部重试）与回合执行同步在同一线程 —— 装在线程池外层（`spawn` 之前）等于装错线程，
   观察者永不触发。这里不加第二个装配点：三个流式入口共用这条路径。
2. **重试进度走独立事件名 `turn:retry`，但不进 `TurnStreamEvent`**：负载
   `TurnRetryPayload { turnId, sessionId, attempt, maxAttempts, delayMs, errorClass }`
   （serde camelCase）。发射口复用**同一个** `TauriTurnEventSink` 对象的 `emit_retry`（本文件仍是
   桌面侧唯一的 `app.emit` 落点），却刻意不复用 `TurnStreamEvent` 的线缆形状：重试进度是**瞬时
   状态位**，不参与回合生命周期状态机（不改变终态判定、不参与事件游标去重）。这与 TUI 的
   `UiEvent::ProviderRetry` 是对称结构，不是第二条回合事件通道。
3. **前端只做落地与展示，不推算**：`src/stores/runtime.ts` 新增 state `retryProgress`
   （会话内瞬时态，不持久化）+ action `applyTurnRetryProgress`（归属 fail-closed：`sessionId`
   缺失/非本会话 ⇒ 丢弃；分母 < 分子等物理不可信负载 ⇒ 丢弃），`agentRuntimeStatus` getter 在
   进度非空时转发 `retryAttempt` / `retryMax` ⇒ 状态行呈现 **真实** 的 `正在重试（n/m）...`。
4. **清零点与既有占位清零同点**：`clearTimeoutRetryPlaceholder`（新轮 `turn:started`、首个
   正文/思考增量、`turn:tool`/`output_end`、终态）内一并把 `retryProgress` 置 `null`，不引入 TTL
   猜测，也不新增第二套生命周期。
5. **不按 turnId 门禁前端落地**：沿用 D-59-9 的既有事实（本地 requestId 与后端回传 turnId 可能
   不同），按 id 卡会让重试态再次不可达；过期由第 4 条的统一点位收口。

## Alternatives considered

- **新增 `TurnStreamEvent` 的 `retry` kind（走 sink 的 `emit` 正规通道）**：线缆更「统一」，但要给
  40+ 字段的扁平结构再加 3~4 个仅此事件有值的可选字段，且 `turn:retry` 完全不参与回合状态机——
  为一个瞬时状态位扩大公共线缆契约（还要同步 `ts-contract.snapshot` 与前端联合类型），收益是形式
  统一，代价是每次改重试语义都动公共协议。前次决策也记录过「事件走 sink」在库层不可行（sink 无法
  存进 `'static` 的 thread_local）；桌面侧虽有 sink 在手，也没有必要为此改公共形状。
- **让前端自行推算分母（读 `upstream_policy.decision_max_attempts`）**：等于把「配置上限」冒充
  「本跳实际分母」——限流长退避、换手、预算耗尽都会让实际尝试次数与配置不同，这正是历史上伪造
  `重试中(1/5)` 的同一类错误。真值必须由后端通知携带。
- **在 `register_turn_sink`（命令线程）里安装观察者**：那里确实能拿到 sink 与 turn_id，但线程是
  Tauri 的命令处理线程，不是执行回合的阻塞线程 ⇒ 观察者永不触发，还会随下一轮覆盖安装形成假接线。
- **前端用 TTL 自动清空（如 5s 未更新即视为结束）**：重试间隔本身可达 8s（退避封顶），TTL 会把
  真实重试误清成「已结束」，属于猜测式状态；改用既有生命周期点位后无此问题。
- **把重试进度写成消息流里的一行 System 消息**：9 次重试会堆 9 行噪音，且「重试中」是当前状态而
  非历史事实（与 TUI 侧同一裁决）。

## Consequences

- 桌面 App 状态行在退避重试期间显示真实进度（`正在重试（2/10）...`），与 TUI 口径一致；无重试事件
  时行为不变（仍降级为「正在重试…」，不虚构分母）。
- 新增对外事件 `turn:retry`：前端消费者只有 `src/stores/runtime.ts` 一处；`TurnRetryEvent` 类型在
  `src/types/runtime.ts`。该事件不在 `TurnStreamEvent` 契约内，故 `docs/contracts/ts-contract.snapshot.json`
  无需重生成（`npm run contract:check` 判据只查 Rust `TurnStreamEvent` 系）。
- 新增回归用例：
  - `src-tauri/src/tauri_adapter.rs::tests`（3 条）：camelCase 线缆键名锁定、事件名字面量锁定、
    **观察者只在本线程触发**（锁定「装错线程 ⇒ 0 信号」这一装配不变量）。
  - `tests/turn-retry-progress.spec.ts`（6 条）：真实分母走线、跨会话丢弃、缺 sessionId fail-closed、
    不可信分母丢弃、首个增量清零、无事件不嘘分母。
- 门禁：本卡改了 `docs/observability.yaml`（补归属说明）⇒ 提交信息须带 `[decl-change-approved]`；
  账本留痕 `[backlog:B17x]`（见 `backlog/tasks.jsonl`）。
- 仍属 N/A 的部分不变：metrics / logs 维持结构性 N-A（桌面端无常驻监听地址、无 `/metrics`）；
  tracing 收口点仍是 `agent/otel/mod.rs`，本卡不新增 span，也不新增 enforcement_point。
