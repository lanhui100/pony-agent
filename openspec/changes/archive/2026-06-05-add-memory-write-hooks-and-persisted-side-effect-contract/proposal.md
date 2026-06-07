## Why

hooks 要从 turn/runtime 扩到真正高风险的持久化路径，memory write 与 persisted side-effect 是必须单独接住的一层：

- `PA-018` 已稳定 retrieval / long-term memory 边界
- `PA-032` 已稳定 recovery / replay contract
- `PA-033` 已定义 `persisted_effect / replay_required` 等 foundation 语义

但目前还没有正式的“memory write intent -> hook -> persistence evidence -> replay decision”合同。如果不单独拆卡，后续很容易重新走回隐式 store mutation。

## What Changes

- 定义 normalized memory-write hook point 与 write intent
- 为 persisted side-effect 增加最小证据与 recovery/replay 口径
- 让 write hook evidence 进入 persisted trace / control-plane / recovery 读面
- 明确本卡不扩展到 planner / capability / run-level mediation

## Capabilities

### New Capabilities

- `memory-write-hooks-and-persisted-side-effect-contract`

### Modified Capabilities

- `agent-hooks-pipeline-foundation`
- `trace-persistence-and-recovery-contract`
