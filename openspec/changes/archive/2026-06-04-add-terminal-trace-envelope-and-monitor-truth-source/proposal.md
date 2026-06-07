# Proposal: Add Terminal Trace Envelope And Monitor Truth Source

## Why

`PA-031` 到 `PA-035` 已经把 turn lifecycle、trace persistence、checkpoint boundary 与 runtime hook dispatch 站稳，但 terminal trace envelope 仍有一个剩余灰区：

- streamed graph-run 路径会把 terminal event metadata 回写到 persisted trace
- sync `run_turn()` persisted trace 还没有同等粒度的 terminal envelope
- monitor / control-plane 聚合已开始消费 persisted trace，但“terminal envelope 一致性”还没有单独的实现卡和验收矩阵

这会让系统进入一种危险状态：

- trace 看起来存在
- reload 后 drilldown 也能打开
- 但某些 sync/failed/cancelled 路径上的 terminal metadata 与聚合口径并没有被正式证明为同构

因此需要单独落一张实现卡，把 terminal envelope 与 monitor 真相源继续收紧。

## What Changes

- 为 sync terminal persisted trace 补齐 canonical terminal event metadata
- 统一 completed / failed / cancelled persisted trace 的 terminal envelope 口径
- 收紧 monitor / control-plane 对 terminal trace 的聚合事实源
- 为 failed / cancelled drilldown 补 provider/tool/hook evidence 保真验证

## Out Of Scope

- 新的 lifecycle contract 设计
- 新的 recovery checkpoint 语义
- hooks patch / side-effect applier 正式上线
- session 控制 UX 交互改版

