## Context

当前系统具备 turn 流式事件、trace 持久化、execution checkpoint、history graph 等零件，但没有一套稳定的 lifecycle contract 约束这些零件。现状导致：

1. 前端 store 必须从 `messages + traceTimeline + providerCallRecords + fallback` 中恢复运行语义
2. 后端只有局部阶段名，缺少统一 event vocabulary
3. `completed / failed / cancelled` 的终态语义存在多处重复表达
4. hooks、recovery、monitoring 都没有可靠挂接点

本变更只收口 lifecycle contract，不直接扩展 hooks、history checkout 细节或完整恢复实现。

## Goals / Non-Goals

**Goals**

- 建立 canonical turn phase 状态机
- 建立 canonical turn event vocabulary
- 定义 model hop / tool hop 在 lifecycle 中的表示方式
- 定义前后端最小共享 contract 与消费原则
- 为后续 trace persistence 与 hooks pipeline 提供稳定底座

**Non-Goals**

- 立即重写全部前端 runtime store
- 立即实现持久化 recovery checkpoint
- 立即交付完整 hooks registry/plugin 生态
- 重新设计 graph run 的全部状态机

## Decisions

### 1. 生命周期以 turn 为最小执行单元

原因：

- trace、checkpoint、历史回放、UI 展示都以 turn 为主要观察窗口
- 一个 turn 内可容纳多次 model/tool hop，但不应再拆成多个一线执行对象

### 2. phase 与 event 分离

做法：

- phase 描述当前状态机所处阶段
- event 描述生命周期边界上发生的事实

原因：

- phase 适合前端展示和 checkpoint
- event 适合 trace、审计、重放和 hooks boundary

### 3. 事件必须可重放且带顺序号

做法：

- 所有事件都具备 `eventId / turnId / sessionId / sequence / emittedAtMs`
- 事件流按单 turn 单调递增排序

原因：

- 后续 recovery、trace rebuild、hooks 审计都依赖可重放事件

### 4. 前端不再发明生命周期真相

做法：

- 前端只能消费 canonical lifecycle contract
- `PA-031` 只收 `phase / event vocabulary / ordering / terminal semantics`
- 若后端未提供足够 contract，应在后端补齐，而不是前端继续推导更多隐式 lifecycle 语义

原因：

- 当前指标漂移与 trace 消失问题的核心风险就是状态源分裂

### 5. `checkpointing` 只定义生命周期边界，不承诺恢复能力

做法：

- `checkpointing` 表示 turn 进入检查点或持久化提交边界
- `checkpointing` 不等同于 `recovery checkpoint`
- 哪些 checkpoint 具备 `replayable / resumable` 语义，由 `PA-032` 定义

原因：

- 避免 `PA-031` 抢走 `PA-032` 的恢复合同边界

## Implementation Outline

1. 明确 Rust 与 TS 共享 phase vocabulary
2. 明确一级事件枚举与最小 payload
3. 在 runtime / turn_flow / control_plane 之间收口事件出口
4. 在前端类型层对齐 canonical event/phase contract
5. 为后续任务记录验证矩阵

## Verification Strategy

- Rust 单元测试验证 phase 转移、终态和 sequence 稳定性
- 流式 turn 测试验证事件顺序与终态一致
- 前端 store 测试验证消费 canonical lifecycle 字段时不再发明冲突 lifecycle 语义
- 文档与任务系统必须同步更新
