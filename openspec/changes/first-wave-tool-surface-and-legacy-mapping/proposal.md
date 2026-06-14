# Proposal: First Wave Tool Surface And Legacy Mapping

## Why

当前 Pony Agent 已经具备真实底层工具能力，但这些能力主要以 `workspace_*` 形式存在，更像执行原语而不是长期产品级工具面。

在前两张合同卡已经收口工具协议与 workspace/权限边界之后，必须尽快定义首批模型可见工具面，否则：

- 模型仍会直接看到内部实现名
- 前端工具活动会继续沿用技术味过重的底层名字
- 复合工具与旧工具别名的兼容路径会长期模糊
- `Read / Search / List / Edit / Write / Run / Ask / Plan` 这类产品级工具很难进入稳定协议

## What Changes

- 建立 `first wave tool surface and legacy mapping` 的正式 spec
- 定义首批模型可见工具面与正式工具清单
- 定义旧 `workspace_*` 工具到新工具面的映射与兼容策略
- 定义复合工具与显式 `ToolPlan` 的产品层暴露规则
- 定义模型可见名、canonical 产品名与 execution primitive 名的关系

## Impact

- Pony Agent 将从“有底层工具实现”推进到“有正式第一层工具产品协议”
- 前端、trace、planner 与 runtime 可以围绕统一首批工具面继续演化
- 旧工具名不会立刻消失，但会被正式降级为内部原语或兼容别名
- `Plan` 与 `Ask` 这类容易在实现中漂移的交互能力也会进入正式产品合同

## Tracking

- Task card: `PA-048`
- OpenSpec Change: `first-wave-tool-surface-and-legacy-mapping`
