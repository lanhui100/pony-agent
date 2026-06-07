# 2026-06-04 Session 74

## 主题

- 归档第一批已完成的 OpenSpec changes，并同步 canonical specs

## 本轮完成

1. 确认归档前置条件
   - 对应任务卡均已 `Done`
   - acceptance audit / closeout 证据已齐
   - change `tasks.md` 均无未勾选项
2. 完成 canonical spec 同步
   - 将以下 change 的 `specs/<capability>/spec.md` 同步到 `openspec/specs/<capability>/spec.md`
   - `add-skills-registry-bridge`
   - `add-trace-panel-call-model-observability`
   - `add-turn-lifecycle-event-contract`
   - `add-trace-persistence-and-recovery-contract`
   - `add-agent-hooks-pipeline-foundation`
   - `add-checkpoint-lifecycle-boundary-implementation`
   - `add-runtime-hook-dispatch-on-stable-boundaries`
   - `add-terminal-trace-envelope-and-monitor-truth-source`
   - `add-session-control-surface-and-feedback-loop`
3. 完成 archive 迁移
   - 上述 9 个 change 已移入 `openspec/changes/archive/2026-06-04-*`
   - 当前 `openspec/changes/` 活跃目录已只剩 `archive/`

## 结果

- `openspec/specs/` 已新增：
  - `skills-registry-bridge`
  - `trace-panel-call-model-observability`
  - `turn-lifecycle-event-contract`
  - `trace-persistence-and-recovery-contract`
  - `agent-hooks-pipeline-foundation`
  - `checkpoint-lifecycle-boundary`
  - `runtime-hook-dispatch-on-stable-boundaries`
  - `terminal-trace-envelope-and-monitor-truth-source`
  - `session-control-surface-and-feedback-loop`
- lifecycle / recovery / hooks / session UX 这条近线的第一批 changes 已从“已完成未归档”切到“已完成已归档”

## 当前判断

- 当前 spec 系统与任务系统重新对齐，阶段性 closeout 已经从实现层推进到变更管理层
- 下一步最合适的是决定新的近线 change：优先评估 `PA-022` 是否要正式拆分为更小的 post-foundation 扩展卡

## 回写

- [Dashboard](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/00_DASHBOARD.md)
- [OpenSpec Specs](/C:/Users/HUAWEI/Documents/pony-agent/openspec/specs)
- [OpenSpec Archive](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/archive)
