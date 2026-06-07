## Why

`PA-033` 和 `PA-035` 已经把 turn-level hooks foundation 与 stable-boundary runtime dispatch 收口，但 graph run / execution control 这一层仍没有统一 hooks 边界：

- `start / wait_user / stop / resume / submission-plan` 仍散落在 graph、runtime 与 control-plane 里
- session 控制 UX 已经落地，但缺少受控 hook 扩展面
- 后续如果要做 run 级治理、审批、拦截或审计，当前没有稳定挂接点

因此需要把 `PA-022` 拆出一张专门的 run/execution-control hooks 卡，只在 graph run 与 execution control 的稳定边界上接 hooks，不回吞 memory / planner / capability 范围。

## What Changes

- 定义 graph run / execution control hook point 与 canonical binding
- 为 stop / resume / checkpoint selection / submission-plan 增加结构化 mediation envelope
- 让 hook evidence 进入 persisted trace / runtime view / control-plane 读面
- 明确本卡不扩展到 memory write、planner facts 或 capability ingress

## Capabilities

### New Capabilities

- `run-hooks-and-execution-control-boundaries`

### Modified Capabilities

- `agent-hooks-pipeline-foundation`
- `turn-lifecycle-event-contract`
- `trace-persistence-and-recovery-contract`
