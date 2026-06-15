# Proposal: Tool Observability And Frontend Presentation Contract

## Why

工具系统不只是后端协议问题。即使工具合同、workspace、权限、首批工具面都定义好了，如果 trace、monitor、session drilldown 与前端工具活动没有统一展示合同，最终仍会出现：

- 同一个工具在不同读面展示不同名字
- 复合工具与子步骤结果各自发明渲染结构
- 中文短显示名无法稳定进入前端读面
- 权限拒绝、部分成功、附件对象、子步骤结果在 UI 中缺乏统一语义

因此需要一张独立 spec，把工具系统的“可观测与可展示读面”正式收口。

## What Changes

- 建立 `tool observability and frontend presentation contract` 的正式 spec
- 定义工具活动、trace、monitor、session drilldown 的统一展示字段
- 定义中文短显示名的前端消费规则
- 定义复合工具、部分成功、审批拒绝、失败与附件对象的展示口径
- 建立工具协议迁移后的验收矩阵
- 定义旧工具到新工具迁移时的展示兼容与回退边界

## Impact

- 前端和观测读面不再依赖各自推导工具语义
- 新旧工具迁移时能保持展示层稳定
- 复合工具和权限路径能被用户清晰理解
- 中文短显示名、canonical 名、子步骤与附件展示将有统一锚点

## Tracking

- Task card: `PA-049`
- OpenSpec Change: `tool-observability-and-frontend-presentation-contract`
