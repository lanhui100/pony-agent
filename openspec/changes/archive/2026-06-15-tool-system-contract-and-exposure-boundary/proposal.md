# Proposal: Tool System Contract And Exposure Boundary

## Why

Pony Agent 当前已经有可工作的工具执行底座：`ToolRouter`、`ToolCall`、`ToolResult`、`ToolPlan`、`capability bridge`、`telemetry` 与 `workspace_root` 注入都已经存在，说明系统并不是“还没有工具层”，而是“已经有实现，但缺少一份正式的统一合同”。

当前最大的问题不是工具能力不足，而是工具协议尚未被明确收口：

- 模型可见工具名与内部执行原语名还没有分层，现有 `workspace_*` 工具名更像底层实现名而不是长期模型协议
- 还没有明确三层标识模型：哪个标识用于模型协议，哪个用于跨来源聚合，哪个用于最终执行原语
- `ToolResult`、工具失败语义、复合工具结果与展示元数据还没有形成稳定结构化合同
- builtin tool、capability-backed tool、skill-composed tool 与 composite tool 还没有被要求复用同一套工具定义/结果/失败模型
- 前端、trace、planner、runtime 目前能消费工具，但消费口径尚未有统一 spec 真相源，后续扩展很容易各自发明字段

如果不先把工具系统写成稳定协议，后面的 workspace 合同、权限审批、首批工具面、前端展示与迁移实现都会在各自任务里重复发明命名、字段和边界，最终把工具系统变成“有很多实现，但没有统一语义”的状态。

## What Changes

- 建立 `tool system contract and exposure boundary` 的正式 spec
- 明确区分“模型可见工具名”和“内部执行原语名”，要求底层工具能力可以继续复用，但不应直接暴露为长期产品级模型协议
- 明确三层标识模型：模型可见工具名、跨来源聚合的 canonical tool name、运行时解析后的 execution primitive
- 为 Pony Agent 定义统一 `ToolDefinition / ToolCall / ToolResult / ToolFailureKind` 合同
- 定义统一工具分类体系，至少覆盖 `read / search / write / execute / plan / interactive / composite / external`
- 定义统一暴露策略，至少覆盖 `model-visible / internal / deferred`
- 定义中文短显示名与前端展示元数据属于展示层，而不是底层工具标识
- 要求 capability / skill / builtin / composite tool 复用同一套工具结果与失败语义，而不是各自发明第二套协议
- 为后续审批与权限合同预留策略元数据位，但不在本 change 中提前冻结完整审批语义

## Impact

- 后续 `PA-046 ~ PA-049` 都能建立在同一份工具母合同之上推进
- 首批基础工具面可以用短英文名和简洁中文短显示名对外暴露，同时继续复用已有 `workspace_*` 底层实现
- planner、trace、monitor、session drilldown 与前端工具活动都能消费稳定的工具定义与结果结构
- capability bridge / skills registry 可以继续保留主体逻辑，但它们的工具表达口径会被要求统一

## Tracking

- Task card: `PA-045`
- OpenSpec Change: `tool-system-contract-and-exposure-boundary`
