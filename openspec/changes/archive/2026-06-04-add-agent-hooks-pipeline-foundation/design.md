## Context

仓库已有一个早期任务卡 `PA-022` 指向 lifecycle hooks pipeline，但当时 graph/runtime/context/capability 边界尚未稳定，过早实现 hooks 容易把主流程打散。现在已经明确：

- lifecycle 必须先成为事实源
- trace / recovery 必须先建立稳定边界
- hooks 必须作为受控扩展层，而不是新的事实源

本变更把 `PA-022` 中与 turn lifecycle 直接相关、且必须在 lifecycle/recovery 稳定后才能落地的那一层 hooks foundation 提炼成可执行任务；它不是 `PA-022` 的全部替代品。

## Goals / Non-Goals

**Goals**

- 定义 hook 分类与能力边界
- 定义允许挂接的 lifecycle boundary
- 定义 hook 执行顺序、超时、失败策略与恢复语义
- 定义 hook traceability 与持久化交互原则
- 为后续最小实现提供清晰骨架

**Non-Goals**

- 一次性交付复杂插件生态
- 允许 hook 任意修改内部 store
- 在本卡内实现 workflow 编排系统

## Relationship to PA-022

- `PA-033` 是 `PA-022` 的 turn-lifecycle foundation 子集
- `PA-033` 完成不自动关闭 `PA-022`
- `PA-022` 中 `run / memory write / planner / skills / MCP` 相关 hooks 仍保留为 foundation 之后的扩展范围
- 后续若继续推进更大范围的 hooks 平台，应重写或拆分 `PA-022` 为 post-foundation expansion 任务卡

## Decisions

### 1. hooks 只允许挂在稳定边界

允许的一线 boundary：

- `before/after turn prepare`
- `before/after context build`
- `before/after model call`
- `before/after tool call`
- `before/after checkpoint persist`
- `before/after turn finalize`

原因：

- 只有边界稳定，trace、checkpoint 和恢复语义才可控

### 2. hooks 必须分类与分权

分类：

- `observe`
- `guard`
- `transform`
- `side_effect`

原因：

- 不同 hook 的权限、失败策略和恢复语义不同，不能混在一起

### 3. hook 不直接改内部 store，也不成为新调度层

做法：

- hook 通过结构化结果返回 allow/deny/patch/side-effect-request
- canonical runtime contract applier 再决定如何应用

原因：

- 否则 trace 和 recovery 会失控

附加禁止项：

- hooks 不得创建新 lifecycle phase
- hooks 不得递归调度 turn
- hooks 不得绕开 canonical runtime path 直接触发新的 model/tool hop
- `side-effect-request` 只允许交给既有 runtime/host capability 执行，不构成独立调度循环

### 4. hooks 合同必须达到可直接编码粒度

每个 hook 至少定义：

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

补充约束：

- 同一 boundary 的 hook 顺序必须稳定且可预测
- transform patch 冲突必须有固定裁决规则
- `guard deny` 必须映射到结构化 turn/result 语义
- `observe` 与 `side_effect` 超时默认不得改变 turn 终态，除非显式声明阻断
- `persisted_effect` 必须有可验证的持久化证据，否则默认按 `replay_required` 处理

### 5. hook 必须显式进入 trace

原因：

- 工业项目里，hook 的可观测性与可审计性是上线前提

## Implementation Outline

1. 基于 `PA-031` phase/event contract 定义 hook points
2. 基于 `PA-032` recovery 语义定义 hook recovery modes
3. 定义 registry / executor / result normalization / contract applier 骨架
4. 定义 hook trace step、持久化证据与验收矩阵

## Verification Strategy

- 合同测试覆盖 hook 分类、边界、失败策略与恢复模式
- runtime 定向测试覆盖 hook 执行顺序与 traceability
- 文档、任务卡与 review 文件同步更新
