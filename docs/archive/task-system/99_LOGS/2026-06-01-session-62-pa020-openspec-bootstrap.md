# 2026-06-01 Session 62 - PA-020 OpenSpec 基线建立

## 本次目标
- 为 `PA-020 建立 MCP capability bridge` 建立正式 OpenSpec 入口。
- 将当前仓库从“已完成 change 全部归档”推进到“下一张 backlog 卡具备 spec-first 执行条件”。

## 本次产出
- 新建 change：
  - [add-mcp-capability-bridge](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-mcp-capability-bridge)
- 新建 proposal / design / tasks / delta spec：
  - [proposal.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-mcp-capability-bridge/proposal.md)
  - [design.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-mcp-capability-bridge/design.md)
  - [tasks.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-mcp-capability-bridge/tasks.md)
  - [mcp-capability-bridge/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-mcp-capability-bridge/specs/mcp-capability-bridge/spec.md)

## 关键边界
- MCP 被定义为 capability-ingress layer，而不是 scheduler / runtime layer。
- planner 只能消费 normalized capability facts，不直接消费 MCP wire details。
- `PA-021` skills bridge 与 `PA-022` hooks pipeline 被明确保留为下游范围，不并入 `PA-020`。

## 任务系统同步
- [PA-020-build-mcp-capability-bridge.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-020-build-mcp-capability-bridge.md) 已补齐 OpenSpec Change、Delta Spec 与 Spec 状态。
- [01_TASK_BOARD.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md) 已将 `PA-020` 放入 `Ready`。
- [00_DASHBOARD.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/00_DASHBOARD.md) 已将下一步主线更新为 `PA-020`。

## 说明
- 本地环境不可用 `openspec` CLI，因此本次 change 目录与文档采用手工创建方式补齐。
- 本次仅建立 spec 基线，不包含 `PA-020` 的代码实现。
