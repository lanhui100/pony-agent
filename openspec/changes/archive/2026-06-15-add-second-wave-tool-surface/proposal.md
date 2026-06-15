# Proposal: Add Second Wave Tool Surface

## Why

Pony Agent 已经通过 `PA-045 ~ PA-049` 完成了工具系统第一轮合同闭环：

- 工具定义、调用、结果与失败语义已经正式收口
- workspace 边界、权限合同与前端展示合同已经稳定
- 首批产品级工具面已经冻结为：
  - `Plan`
  - `Read`
  - `Search`
  - `List`
  - `Edit`
  - `Write`
  - `Run`
  - `Ask`

但当前真实实现与已冻结工具面之间仍有明显差距：

- 当前内建 primitive 仍主要对应 `Run / Ask / Read / List / Search / Plan`
- `Edit / Write` 已进入产品级合同，但尚未成为真实内建工具
- `Run` 当前主要映射到极轻量 primitive，尚未形成真正的受控命令执行边界

同时，和本仓库中作为对照基线的 Codex / Claude Code 相比，Pony Agent 仍缺少几类高价值基础能力：

- 面向代码库探索的 `Glob / Grep`
- 面向真实执行闭环的 shell / command 执行
- 面向外部知识获取的 `WebFetch / WebSearch`
- 面向能力桥接的 `MCP Resource Read`
- 面向大工具池治理的 `ToolSearch`

如果不把“下一步该补哪些工具、哪些属于近线、哪些后置”写成正式 spec，后续实现容易再次碎片化：有人只补 `Edit`，有人先补 `WebSearch`，有人把 `Run` 做成任意执行入口，最终会破坏刚建立的工具协议稳定性。

## What Changes

- 建立 `second-wave-tool-surface` 的正式 spec
- 明确第二批工具能力的候选清单、优先级与分阶段范围
- 对齐“首批合同已冻结但尚未真实实现”的缺口，优先收口 `Edit / Write / RunShell`
- 明确 `Glob / Grep / WebFetch / WebSearch / MCP Resource / ToolSearch` 的进入条件和稳定边界
- 约束哪些能力属于当前近线基础工具面，哪些仍属于后续扩展能力
- 通过独立 spec review 收紧范围，避免第二批工具面再次漂移

## Impact

- Pony Agent 后续工具实现将从“零散补能力”变成“按正式 spec 分阶段补齐”
- `Edit / Write / Run` 将拥有比当前更清晰的产品级边界
- 后续实现 `Glob / Grep / WebFetch / WebSearch / MCP Resource / ToolSearch` 时会直接复用统一工具合同，而不是各自发明协议
- 任务系统与 OpenSpec 将能明确区分：
  - 已冻结但未实装的首批工具缺口
  - 真正的新第二批能力
  - 暂不进入近线的扩展工具

## Tracking

- Task card: `PA-050`
- OpenSpec Change: `add-second-wave-tool-surface`
