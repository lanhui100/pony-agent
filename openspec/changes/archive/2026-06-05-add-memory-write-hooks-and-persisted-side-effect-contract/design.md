## Context

memory write 与 side-effect 是 hooks 体系里最容易破坏 recovery truth 的一层：一旦副作用已经发生，系统必须能回答“已持久化了什么”“reload 后如何判断是否重放”。

turn/runtime hooks 已经建立了 trace evidence 链，但还没有正式覆盖 memory write。

## Goals / Non-Goals

**Goals**

- 给 memory write 建立 normalized hook input/output
- 给 `persisted_effect / replay_required` 建立最小 evidence envelope
- 打通 reload / recovery / read-plane

**Non-Goals**

- 本轮不接 graph run / execution-control
- 本轮不接 planner / capability mediation
- 本轮不允许 hooks 直接写 session store
- 本轮不允许 memory hooks 反向定义 recovery truth-source

## Decisions

### 1. memory hooks 只消费 write intent

hooks 面向的是被规范化的 write intent，而不是低层具体 store API。

### 2. persisted effect 必须声明 evidence

任何会留下真实效果的 hook，都必须同时定义“持久化了什么证据”“replay 怎样判定”。

### 3. replay_required 默认优先于隐式恢复

如果 hook 没有足够 persisted evidence，系统默认回到 `replay_required`，而不是乐观假设 side-effect 已可恢复。

### 4. memory-write hook trace 先进入 session truth-source

memory-write hook 自身的决策轨迹，不应先强塞进 checkpoint 模型。

先保证：

- hook 结果能被投影为独立 trace record
- trace record 能进入 `SessionState / SessionSnapshot / HistoryNode`
- reload 与 history checkout 都能读回正确节点范围内的 hook traces

checkpoint/read-plane 是否额外投影与当前恢复边界相关的 hook trace 摘要，放到后续实现阶段再决定。

### 5. cross-card exclusion matrix 必须固定

- `PA-039` 不触碰 execution arbitration
- `PA-039` 不吞 planner/capability mediation
- `PA-039` 只处理 memory write 与 persisted side-effect

## Implementation Outline

1. 定义 normalized memory write intent / hook point / result envelope
2. 在 session/control-plane 补 persisted evidence 与 recovery projection
3. 先把 hook trace 接进 session truth-source，再补 reload/read-plane/traceability 测试

## Verification Strategy

- Rust 单测覆盖 allow/deny/transform/persisted_effect/replay_required
- Rust roundtrip 覆盖 evidence reload
- control-plane / runtime view 覆盖 recovery read-plane
