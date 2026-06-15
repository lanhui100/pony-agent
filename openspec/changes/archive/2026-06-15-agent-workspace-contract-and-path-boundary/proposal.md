# Proposal: Agent Workspace Contract And Path Boundary

## Why

当前 Pony Agent 已经不是“完全没有 workspace 概念”的状态：

- `AgentRuntimeBuilder.workspace_root(...)` 已经提供工作区注入入口
- `ToolRouter::with_workspace_root(...)` 已经让工具执行可以绑定显式 workspace root
- 工具层已经具备 canonicalize、越界防护、相对路径展示与路径修复逻辑

这说明 workspace 已经是工具系统的现实边界，而不是未来再讨论的产品功能。

但目前这些能力还没有被写成正式合同，因此存在几个风险：

- 路径解析、canonicalize、越界拒绝、相对路径展示与路径修复还主要体现在实现里，而非稳定协议
- 后续 `Read / Search / List / Edit / Write / Run` 很容易各自带一套路径语义
- future Tauri / HTTP-SSE / CLI / service host 可能各自发明 workspace policy
- 权限与 trace 后续会依赖 workspace scope，但当前还没有正式真相源

如果不先把 workspace 写成母合同，后面的权限、首批工具面与前端展示就会建立在“实现已经大概这样做了”的弱前提上，而不是稳定边界上。

## What Changes

- 建立 `agent workspace contract and path boundary` 的正式 spec
- 明确 agent run 的 workspace 边界与注入方式
- 定义路径解析、canonicalize、越界拒绝、相对路径展示与路径修复规则
- 定义 `Run` 默认工作目录与执行类工具的 workspace 约束边界
- 定义 workspace 作为 runtime / tool / trace / permission scope 的共同上下文
- 为 future host preset 与 workspace policy 保留稳定 seam

## Impact

- 文件与目录类工具将有统一、可审计的路径语义
- 执行类工具也将有明确的 workspace 默认边界，而不是游离在合同外
- 权限合同可以复用稳定的 workspace scope，而不是各自补定义
- future host 可以在统一 contract 下替换 workspace policy，而不是复制工具层逻辑
- 前端、trace 与 session drilldown 可以消费稳定的相对路径显示规则与失败语义

## Tracking

- Task card: `PA-046`
- OpenSpec Change: `agent-workspace-contract-and-path-boundary`
