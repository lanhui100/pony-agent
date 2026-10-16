# wave-20 契约冻结矩阵：模型工具面净化与长程目标 Goal 工具真实可达

## 1. 现状与诊断
- 虚假模型能力：`subagent`, `workflow`, `spawn_teammate`, `list_agents`, `send_message`, `interrupt_agent`, `drain_inbox`, `team_task_create`, `team_task_update`, `team_task_list` 目前作为 `ToolExposure::ModelVisible` 暴露给模型，但执行器内部仅返回 mock 字符串或内存桩，导致模型产生严重委派幻觉。
- 目标工具失联：`create_goal`, `get_goal`, `update_goal` 虽已实现执行逻辑，但未加入 `canonical_tool_name` 别名表，且 exposure 默认回退到 `ToolExposure::Internal`，导致模型在被引导创建目标时无法成功下发调用。

## 2. 目标契约 (C1 ~ C2)

### C1: 10 个未就绪空桩工具模型面下线
- 以下 primitive 的 `tool_exposure_for_primitive` 从 `ModelVisible` 调整为 `Internal`：
  - `subagent`
  - `workflow`
  - `spawn_teammate`
  - `list_agents`
  - `send_message`
  - `interrupt_agent`
  - `drain_inbox`
  - `team_task_create`
  - `team_task_update`
  - `team_task_list`
- `provider_contract_views()` 不再向外部模型下发上述工具 schema。

### C2: 3 个 Goal 目标工具完全接入模型面
- 在 `canonical_tool_name` 中增加匹配：
  - `TOOL_CREATE_GOAL | "create_goal" | "create.goal"` => `Some(TOOL_CREATE_GOAL)`
  - `TOOL_GET_GOAL | "get_goal" | "get.goal"` => `Some(TOOL_GET_GOAL)`
  - `TOOL_UPDATE_GOAL | "update_goal" | "update.goal"` => `Some(TOOL_UPDATE_GOAL)`
- 在 `tool_exposure_for_primitive` 中将 `TOOL_CREATE_GOAL | TOOL_GET_GOAL | TOOL_UPDATE_GOAL` 映射为 `ToolExposure::ModelVisible`。
- 模型下发工具定义中将正式包含 Goal 系列工具。
