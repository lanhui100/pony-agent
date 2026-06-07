## Context

planner 与 capability mediation 是系统里最容易长出“第二大脑”的层：

- planner 已经负责稳定、可审计的系统级裁决
- capability registry 已经负责统一 ingress / read-plane / execution boundary

hooks 若要进入这两层，必须只消费 normalized facts/envelope，并且不能绕开既有 truth-source。

## Goals / Non-Goals

**Goals**

- 给 planner / capability mediation 定义最小 hooks boundary
- 保证 hooks 只基于 normalized facts/envelope 做 observe / guard / transform
- 让 evidence 进入 trace / control-plane / monitor

**Non-Goals**

- 本轮不回到 turn/runtime dispatch
- 本轮不处理 memory write 或 persisted side-effect
- 本轮不让 hooks 变成第二 scheduler 或第二 registry
- 本轮不允许 planner hooks 与 capability hooks 共用一套未区分的 transform 白名单

## Decisions

### 1. planner hooks 只消费 normalized planner facts

不得让 hooks 依赖 provider raw response、前端状态猜测或 graph 内部临时结构。

### 2. capability hooks 只消费 mediation envelope

能力中介层的 hooks 必须建立在 capability bridge / skills ingress 的规范化 envelope 上。

### 3. transform 只允许改规范化高层输入输出

允许改的只是 plan / capability mediation 的规范化结构，不是底层 runtime/tool store。

### 4. planner 与 capability 必须分别限权

- planner hooks 的可改动面单独定义
- capability hooks 的可改动面单独定义
- 两者都必须有只读字段与禁止字段

## Implementation Outline

1. 定义 planner / capability hook point 与 normalized envelope
2. 在 planner / capability_bridge / control_plane 落最小 trace-first mediation 闭环
3. 补 read-plane 与回归验证

## Verification Strategy

- Rust 单测覆盖 planner preflight、graph decision、capability ingress、skill mediation
- Rust/read-plane 覆盖 trace evidence 与 reload
- monitor/control-plane 覆盖 hooks 聚合与 drilldown
