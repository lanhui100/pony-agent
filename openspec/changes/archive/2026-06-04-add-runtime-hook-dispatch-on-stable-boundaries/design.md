## Context

当前仓库的 hooks 能力分成两层：

1. `PA-033` 已交付 foundation：
   - `TurnHookPoint`
   - canonical lifecycle binding
   - `AgentHookRegistry`
   - `NoopHookExecutor`
   - `HookExecutionResult / HookTraceRecord`
2. runtime 已真实发射一部分 canonical turn boundary：
   - `ModelCallStart`
   - `ToolCallStart`
   - `ToolCallEnd`
   - `CheckpointPersistStart / End`
   - `TurnFinalizeEnd`

最缺的不是更多 hooks vocabulary，而是把这两层连起来，让 hooks 至少能在稳定边界上被 runtime 调度，并生成真实 trace evidence。

## Goals / Non-Goals

**Goals**

- 为稳定 boundary 提供最小 runtime hook dispatch
- 保证 hook ordering / failure policy / trace evidence 在 runtime 中真正落地
- 让实时 turn event 与 persisted turn trace 都能看到 hook trace records
- 保持 hooks 是 lifecycle boundary 的受控扩展，而不是新的调度层

**Non-Goals**

- 本轮不补 prepare/context build 早期 boundary 的真实 runtime 事件面
- 本轮不实现 patch/side-effect 的正式 contract applier；这轮只要求 trace-first integration 与受控 evidence 产物
- 本轮不扩展到 `run / memory write / planner / skills / MCP`
- 本轮不引入嵌套 turn、额外 model hop 或独立 side-effect 调度循环

## Decisions

### 1. 首轮只接稳定 boundary

首轮 runtime dispatch 只覆盖已经有真实 runtime 发射的 boundary：

- `ModelCallStart`
- `ToolCallStart`
- `ToolCallEnd`
- `CheckpointPersistEnd`
- `TurnFinalizeEnd`

原因：

- 这些边界已经具备可验证的 canonical event / phase 锚点
- 不需要先改 prepare/context build 的事件面

### 2. 首轮以 trace-first dispatch 为主

首轮目标是“可调度、可排序、可观察、可持久化”，不是一次性实现所有 hook 结果对主执行链的影响。

做法：

- runtime 按 hook point 取 registry 列表
- 通过 executor 执行 hook
- 记录 `HookExecutionResult -> HookTraceRecord`
- 将记录附着到事件与 persisted trace

原因：

- 先把第一条真实 evidence 链打通，能显著降低后续 guard / transform / side-effect 接线风险

### 3. 受控结果仍不允许直接改 store

即使 runtime 已开始执行 hooks，也仍保持：

- hook 不能直接改内部 store
- hook 不能创建新 lifecycle phase
- hook 不能绕开 canonical model/tool path

原因：

- 避免 hooks foundation 在第一次接 runtime 时就滑向新的隐式调度层

### 4. failure policy 首轮以 non-destructive 路径优先

首轮默认以 observe / non-blocking dispatch 为主，优先验证 ordering、traceability 与 failure surface；若纳入 blocking 行为，必须有独立测试覆盖 terminal outcome 与 trace evidence。

原因：

- 这能把“runtime 接线”与“正式干预主链”拆开，让验收边界更清楚

## Implementation Outline

1. 在 `src-tauri/src/agent/runtime.rs` 引入 hook registry / executor 持有与 boundary dispatch helper
2. 为稳定 boundary 的关键发射点补 hook dispatch 调用与 trace record 聚合
3. 让 `TurnStreamEvent / TurnResult / TurnTraceRecord / SessionSnapshot` 真正携带 runtime 产出的 hook trace records
4. 为 runtime exact / roundtrip / frontend hydration 增补稳定 boundary dispatch 验证

## Verification Strategy

- Rust 单测覆盖 stable-boundary dispatch 的 ordering / failure policy / trace evidence
- Rust roundtrip 测试覆盖 hook trace records 落盘与 reload
- 前端 store 测试覆盖 hook trace records 的 hydration / clone 消费
- 明确负例测试：
  - 不为 prepare/context build 自动补假 boundary
  - 不让 hook 创建新 phase
  - 不让 hook 直接绕过 canonical runtime path
