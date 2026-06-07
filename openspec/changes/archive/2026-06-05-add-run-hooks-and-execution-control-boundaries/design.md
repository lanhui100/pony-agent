## Context

当前 hooks 主线已经分成三层：

1. `PA-033`：foundation / no-op contract / binding / traceability
2. `PA-035`：turn stable-boundary runtime dispatch
3. `PA-037`：session control UX

最缺的是 run / execution-control 这层的中间闭环：用户已经可以 `停止 / 恢复 / 继续 / 重新开始`，但系统还没有给这些 run-level boundary 提供统一 hooks 面。

## Goals / Non-Goals

**Goals**

- 给 graph run / execution control 提供最小 hooks 边界
- 保证 stop / resume / submission-plan 仍以 control-plane 与 graph truth-source 为准
- 让 run-level hook evidence 进入 persisted/read-plane

**Non-Goals**

- 本轮不扩展到 memory write / persisted side-effect
- 本轮不扩展到 planner / skills / MCP capability mediation
- 本轮不让 hooks 直接改 graph run store 或 runtime turn store
- 本轮不允许 hooks 定义新的 execution command 或新的 run arbitration source

## Decisions

### 1. run-level hooks 只消费 normalized control envelope

hook 输入必须是规范化的 run/control envelope，而不是 graph 内部临时状态或前端私有 flags。

### 2. stop / resume / submission-plan 仍由既有 truth-source 裁决

hooks 可以 observe / guard / transform 结构化输入，但不能绕开 `resolve_graph_run_submission_plan(...)`、execution checkpoint 或 graph decision truth-source。

### 3. evidence 必须能穿透 reload / control-plane

run-level hooks 不只要“执行过”，还必须在 persisted trace、runtime view、session drilldown 中可读回。

### 4. cross-card exclusion matrix 必须固定

- `PA-038` 不触碰 memory write truth-source
- `PA-038` 不吞 planner/capability mediation
- `PA-038` 只处理 run / execution control boundary

## Implementation Outline

1. 定义 `GraphRunHookPoint / ExecutionControlHookPoint` 与 canonical binding
2. 为 submission-plan / stop / resume / wait-user 相关 boundary 定义 normalized envelope
3. 在 graph / execution_control / control_plane 补最小 hook dispatch 与 trace evidence
4. 补 reload/read-plane/前端控制回归

## Verification Strategy

- Rust 单测覆盖 run start、wait_user、resume、stop/cancel 与 submission-plan arbitration
- Rust roundtrip 覆盖 persisted hook evidence reload
- 前端 store / control-plane 覆盖 runtime view 与 session control read-plane
