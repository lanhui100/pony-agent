# Turn Lifecycle、Hooks 与 Recovery 架构基线

## 目的

本文件把当前围绕 trace、session 可控、checkpoint/recovery 与 hooks 可拓展性的讨论结论收束为正式架构基线，用于指导后续 OpenSpec、任务拆分与实现顺序。

状态注记：

- 本文件主要覆盖 `PA-031 ~ PA-034` 的 foundation 基线
- post-foundation hooks 扩展现已拆入 `PA-038 / PA-039 / PA-040`
- 当前执行顺序、验收口径与具体边界，以对应任务卡和 OpenSpec change 为准

## 核心结论

### 1. 生命周期是事实源，hooks 是受控扩展

- `prompt` 负责软控制
- `hooks` 负责硬控制
- `turn lifecycle` 是底座
- `trace`、`checkpoint`、`history`、`rollback`、前端 UI 都必须消费同一份 lifecycle contract

约束语句：

> Lifecycle is the source of truth; hooks are controlled extensions on lifecycle boundaries.

### 2. 当前问题的根因不是“没有 hooks”，而是缺少统一的生命周期契约

当前仓库已经有：

- Rust runtime turn 执行链路
- stream event sink
- trace 持久化
- execution checkpoint
- history node / branch / restore / checkout

但这些能力还没有被统一的 `turn lifecycle + event contract` 收口，所以前后端都在各自补状态、推状态、兼容历史结构，导致：

- trace 指标漂移
- 重启后状态恢复不稳定
- session/history UI 很难验证
- hooks 无法安全扩展

### 3. 必须显式拆开 4 个子系统

1. `Turn Execution`
   一次用户输入驱动的一次执行。
2. `Execution Control`
   stop / cancel / pause / resume / checkpoint。
3. `Session History`
   turn record / branch / fork / checkout / restore。
4. `Workspace Rollback`
   transcript-only 与 transcript+workspace 两类恢复。

### 4. 术语必须统一

后续 spec、任务卡、实现与验收统一使用以下术语：

- `canonical lifecycle contract`
- `canonical trace metrics`
- `runtime checkpoint`
- `recovery checkpoint`
- `transcript-only`
- `transcript+workspace`
- `degraded_to_transcript_only`

## Turn 生命周期基线

### 状态

- `created`
- `preparing`
- `building_context`
- `calling_model`
- `streaming_response`
- `executing_tool`
- `tool_result_integrating`
- `checkpointing`
- `completed`
- `failed`
- `cancelled`

### 基本原则

- 一个 `turn` 是最小执行单元
- 一个 `turn` 内允许多次 model hop 与 tool hop
- `streaming_response` 是 canonical phase，而不是可选实现细节
- `turn` 的终态只能是：
  - `completed`
  - `failed`
  - `cancelled`

### 典型链路

`created -> preparing -> building_context -> calling_model -> streaming_response -> executing_tool -> tool_result_integrating -> calling_model -> streaming_response -> checkpointing -> completed`

## 统一事件契约基线

### 最小公共字段

- `eventId`
- `eventType`
- `eventVersion`
- `emittedAtMs`
- `sessionId`
- `runId`
- `turnId`
- `sequence`

### 一级事件

- `turn.created`
- `turn.phase_changed`
- `turn.context_built`
- `turn.model_call_started`
- `turn.first_token`
- `turn.output_delta`
- `turn.tool_call_started`
- `turn.tool_call_completed`
- `turn.trace_updated`
- `turn.checkpoint_persisted`
- `turn.completed`
- `turn.failed`
- `turn.cancelled`

### 约束

- 所有事件必须可重放
- 所有事件必须具备稳定顺序
- 指标必须由后端事件/持久化给出，而不是前端二次推导
- 前端只能消费 canonical state，不再自行补生命周期真相

## Trace 责任边界

`Trace = Turn 生命周期事件的观测视图`

trace 负责：

- 展示 phase / step / hop
- 展示 provider 与 tool 活动
- 展示标准指标
- 展示最终汇总

trace 不负责：

- 定义恢复语义
- 决定 session 是否可恢复
- 从 message/history 反推 canonical timeline

推荐的数据形态：

`Session -> TurnRecord[] -> StepRecord[]`

## Checkpoint 与 Recovery 边界

必须区分：

1. `Ephemeral Runtime Checkpoint`
   仅服务运行中 stop/cancel/pause。
2. `Persisted Recovery Checkpoint`
   已落盘、可恢复、可回放、可在重启后重新加载。

恢复语义也必须区分：

- `replayable`
- `resumable`

不能因为存在 checkpoint 字段就默认系统支持断点续跑。

额外约束：

- `checkpointing` phase 只表示 turn 生命周期内发生了检查点或持久化提交边界
- `checkpointing` 本身不承诺该结果已经成为 `recovery checkpoint`
- `recovery checkpoint` 的能力口径由恢复合同单独定义

## Hooks 可拓展性基线

### 原则

- hooks 只能挂在稳定的 lifecycle boundary 上
- hooks 不能直接接管状态机
- hooks 不能直接随意改内部 store
- hook 的副作用必须进入 trace
- hook 的恢复语义必须预先声明
- hook 的可拓展性建立在 contract 扩展上，而不是 runtime 特判扩展上

### 可拓展性治理约束

- 新增 hook 点，必须先升级 lifecycle contract，而不是先改 runtime 再补文档
- 新增 hook 能力，必须先落结构化结果类型，而不是返回自由文本或隐式布尔值
- 新增 hook 副作用，必须先定义 persistence evidence，再允许进入恢复口径
- 新增 hook 恢复优化，必须证明不破坏 `replay_required` 默认回退
- hook foundation 可以扩展 registry / normalization / trace projection，但不得自行长出独立调度循环

### Hook 分类

1. `observe`
   只读，不改执行结果。
2. `guard`
   允许阻断，不允许随意改数据。
3. `transform`
   可以改输入/输出，但必须声明改动面。
4. `side_effect`
   可以触发额外动作，但不能直接篡改 lifecycle 主状态。

### 推荐 hook 点

- `before_turn_prepare`
- `after_turn_prepare`
- `before_context_build`
- `after_context_build`
- `before_model_call`
- `after_model_response`
- `before_tool_call`
- `after_tool_call`
- `before_checkpoint_persist`
- `after_checkpoint_persist`
- `before_turn_finalize`
- `after_turn_finalize`

这些 hook 点的附着规则必须固定为：

- `start/end` 只是 boundary 命名，不得自行引入新的 canonical phase vocabulary
- 每个 hook 点都必须绑定到既有 canonical `eventType` 与 canonical `phase`
- 若某个 hook 点找不到稳定的 canonical lifecycle 锚点，则先收紧 hooks contract，而不是让 runtime 继续猜语义

推荐的最小绑定表：

- `TurnPrepareStart` -> `turn.created | turn.phase_changed` + `created | preparing`
- `TurnPrepareEnd` -> `turn.phase_changed` + `preparing`
- `ContextBuildStart` -> `turn.phase_changed` + `building_context`
- `ContextBuildEnd` -> `turn.context_built` + `building_context`
- `ModelCallStart` -> `turn.model_call_started` + `calling_model`
- `ModelResponseEnd` -> `turn.first_token | turn.output_delta` + `streaming_response`
- `ToolCallStart` -> `turn.tool_call_started` + `executing_tool`
- `ToolCallEnd` -> `turn.tool_call_completed` + `tool_result_integrating`
- `CheckpointPersistStart` -> `turn.phase_changed` + `checkpointing`
- `CheckpointPersistEnd` -> `turn.checkpoint_persisted` + `checkpointing`
- `TurnFinalizeStart` -> `turn.phase_changed` + `checkpointing`
- `TurnFinalizeEnd` -> `turn.completed | turn.failed | turn.cancelled` + `completed | failed | cancelled`

### Hook 合同最小表

每个 hook 至少声明：

- `class`
- `allowedBoundaries`
- `allowedResultKinds`
- `canBlock`
- `defaultFailurePolicy`
- `allowedFailurePolicies`
- `defaultRecoveryMode`
- `traceRequirements`
- `replayRequirements`
- `sideEffectPersistenceRequirements`

每个 hook 结果至少要能被归一化为以下结构之一：

- `observe`
- `allow`
- `deny`
- `patch`
- `side-effect-request`

### 恢复语义

每个 hook 至少声明：

- `failurePolicy`
- `timeoutMs`
- `recoveryMode`

其中 `recoveryMode` 最少区分：

- `replay_required`
- `persisted_effect`

附加约束：

- `persisted_effect` 若没有持久化证据，则不得宣称可跳过重放
- 缺少恢复证据时，默认按 `replay_required` 处理
- `side_effect` hook 不能创建新 phase、不能递归调度 turn、不能直接触发新的 model/tool hop
- `side-effect-request` 只允许提交给 canonical runtime path 处理，不构成独立调度循环
- `deny` 必须携带结构化原因码；`patch` 必须携带目标与操作摘要；`side-effect-request` 必须声明是否要求持久化证据

## 任务拆分与执行顺序

### PA-031：Turn Lifecycle 与 Event Contract

目标：

- 定义 canonical phase / event / ordering / terminal semantics
- 收口后端与前端共享 contract

这是所有后续工作的前置卡。

### PA-032：Trace Persistence 与 Recovery Contract

目标：

- 让 trace / checkpoint / history / rollback 的职责边界可持久化、可恢复、可验收

依赖：

- `PA-031`

### PA-033：Agent Hooks Pipeline Foundation

目标：

- 基于稳定 lifecycle boundary 建立 hook registry / execution / traceability 基线

依赖：

- `PA-031`
- `PA-032` 的恢复语义结论

### PA-034：Checkpoint Lifecycle Boundary Implementation

目标：

- 把 `checkpointing`、`turn.checkpoint_persisted` 与 `before/after checkpoint persist` 从 contract 推进为 runtime、trace persistence、reload 与前端读面都能观察到的真实边界

依赖：

- `PA-031`
- `PA-032`
- `PA-033` 的 boundary binding 结论

## 实施顺序

1. 先完成 `PA-031`
2. 再完成 `PA-032`
3. 再把 `PA-033` 的 turn-lifecycle foundation 做实
4. 最后补 `PA-034`，把 checkpoint boundary 接到真实 runtime / persistence

允许并行的部分：

- spec 起草
- spec 审核
- UI 验收方案设计
- 测试矩阵设计
- `PA-034` 的只读代码审计

不应并行的部分：

- 在 `PA-031` 未定稿前直接实现复杂 hooks
- 在 `PA-032` 未定稿前给 checkpoint/recovery 打补丁

## 与现有代码的直接结论

- `src-tauri/src/agent/runtime.rs` 是当前 turn 执行事实中心
- `src-tauri/src/agent/turn_flow.rs` 是事件发射面
- `src-tauri/src/agent/session.rs` 是 session/history/trace 落盘面
- `src-tauri/src/agent/execution_control.rs` 当前仍以进程内 checkpoint 为主
- `src/stores/runtime.ts` 当前承担了过多推导与兼容逻辑，应在后续改造中逐步降级为消费层

## 本文件的用途

本文件是本轮任务拆分的母文档。后续实现应以对应 OpenSpec change 与任务卡为直接执行入口，并以这里的边界定义作为冲突裁决依据。
