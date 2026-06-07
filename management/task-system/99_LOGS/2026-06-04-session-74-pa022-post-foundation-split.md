# 2026-06-04 Session 74

## 主题

- 将 `PA-022` 从 hooks 总筐拆成下一轮可执行的 post-foundation 子卡

## 本轮完成

1. 复核 `PA-022` 当前范围
   - 确认它仍保留 `run / memory write / planner / skills / MCP` 等 post-foundation hooks 范围
   - 确认继续把这些内容混在一张 Backlog 卡里，不利于形成下一轮可验收闭环
2. 新建三张 `Ready` 卡
   - `PA-038`：run hooks 与 execution-control boundary
   - `PA-039`：memory-write hooks 与 persisted side-effect contract
   - `PA-040`：planner 与 capability-mediation hooks
3. 为三张卡补齐 OpenSpec change 骨架
   - `proposal.md`
   - `design.md`
   - `tasks.md`
   - `specs/<capability>/spec.md`
4. 更新任务系统口径
   - `PA-022` 现在保留为总入口与分流说明，不再直接承载实现
   - Task Board 已把 `PA-038 / PA-039 / PA-040` 放入 `Ready`
   - Dashboard 已更新为“下一轮从三张 Ready 卡中选择启动”

## 当前判断

- 经过这轮拆分，hooks 主线已经从“大方向清晰但实现太宽”推进到“有明确下一跳”
- 下一步不再需要重新讨论 `PA-022` 是什么，而是只需要决定 `PA-038 / PA-039 / PA-040` 的近线优先级

## 回写

- [PA-022](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-022-build-lifecycle-hooks-pipeline.md)
- [PA-038](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-038-build-run-hooks-and-execution-control-boundaries.md)
- [PA-039](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-039-build-memory-write-hooks-and-persisted-side-effect-contract.md)
- [PA-040](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-040-build-planner-and-capability-mediation-hooks.md)
- [Task Board](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/01_TASK_BOARD.md)
- [Dashboard](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/00_DASHBOARD.md)
