# Proposal: Add Third Wave Default Tool Alignment

## Why

Pony Agent 已经完成第二波工具面的规划与实现闭环，但默认工具层仍存在一个明显的“半完成区”：

- `Plan / Ask` 已进入默认工具面与前端默认目录，但尚未完全收口为稳定的真实执行或宿主中介边界
- `MCP Resource / ToolSearch` 已有最小入口实现，但还没有和“默认工具对齐”视角统一收口
- `Run` 虽已具备受控命令执行 primitive，但还没有作为第三波默认工具合同升级的一部分被正式写清楚

如果不把这 5 个工具拉到同一张实现卡和同一份 spec 中处理，下一轮很容易再次出现：

- `Plan` 留在展示层
- `Ask` 留在模糊的人机交互占位
- `MCP Resource / ToolSearch` 继续停留在“已经有入口，但没有默认工具地位”的灰区
- `Run` 的默认工具语义与真实 shell 执行语义继续并行漂移

这会让 Pony Agent 在“默认工具体验”上继续与 Codex / Claude Code 产生割裂。

## What Changes

- 建立 `third-wave-default-tool-alignment` 正式 spec
- 把以下 5 个工具作为同一波实现任务统一收口：
  - `Plan`
  - `Ask`
  - `MCP Resource`
  - `ToolSearch`
  - `Run`
- 明确 `Plan / Ask` 的真实边界、宿主中介语义与非目标
- 明确 `MCP Resource / ToolSearch` 在默认工具面对齐中的角色
- 明确 `Run` 在本波次中的定位是默认工具合同升级，而不是新的产品名
- 通过独立 spec review 收紧范围、顺序和安全边界

## Why This Grouping

这 5 个工具之所以要同波收口，不是因为同类，而是因为它们都处在“默认工具体验未完全收口”的同一层：

- `Plan / Ask` 仍停留在计划和宿主中介的半完成态
- `MCP Resource / ToolSearch` 已有入口，但默认工具地位还未正式收口
- `Run` 虽然是执行 primitive，但它与 `Plan / Ask` 存在直接的编排与审批联动

## Impact

- Pony Agent 将从“第二波工具功能已具备，但默认工具体验仍有断层”进入“默认工具能力与实现语义一致”的下一阶段
- `Plan / Ask` 不再只是目录层工具名，而会具备稳定的执行/交互边界
- `MCP Resource / ToolSearch` 将从桥接入口提升为默认工具面中的正式成员
- `Run` 将在不改 canonical tool name 的前提下，进一步对齐 shell-command 风格默认能力体验

## Tracking

- Task card: `PA-055`
- OpenSpec Change: `add-third-wave-default-tool-alignment`
