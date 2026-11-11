# 2026-05-28 Session 29 PA-018 Home Workspace Run Checkpoint Strip

## 本轮目标

- 继续推进 `PA-018`
- 验证 `HomeWorkspace` 新增的更深层 run/checkpoint strip
- 把这轮验收证据与当前完成度回写到任务系统

## 本轮改动

- 更新：
  - `tests/HomeSessionSidebar.spec.ts`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/00_DASHBOARD.md`
  - `management/task-system/01_TASK_BOARD.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- 确认 `HomeWorkspace` 顶部新增的更深层 run/checkpoint strip 已进入真实验收证据
- 当前主工作区顶部除了 retrieval summary 外，还能显示：
  - `phase`
  - `checkpoint status`
  - `checkpoint phase`
  - `resume count`
  - `resumable`
  - `last decision`
  - `active turn`
  - `last completed turn`
- 为了让 `PA-018` 的前端定向验证重新稳定，给 `HomeSessionSidebar` 的删除确认测试补了最小 `ConfirmPopover` stub，避免 portal/focus 行为把侧栏语义测试拖成超时
- 已把这轮真实完成度回写到：
  - `PA-018` 任务卡
  - `Dashboard`
  - `Task Board`
  - 架构文档

## 验证

已通过：

```powershell
npm exec vitest run tests/HomeSessionSidebar.spec.ts tests/HomeWorkspace.spec.ts tests/HomeSidebar.spec.ts tests/runtime-store.spec.ts
npm exec vue-tsc -- --noEmit
git diff --check -- src/components/HomeWorkspace.vue tests/HomeWorkspace.spec.ts tests/HomeSessionSidebar.spec.ts
```

结果：

- `tests/HomeSessionSidebar.spec.ts`、`tests/HomeWorkspace.spec.ts`、`tests/HomeSidebar.spec.ts` 与 `tests/runtime-store.spec.ts` 共 `47` 个测试全部通过
- `HomeWorkspace` 新增的“更深层 run/checkpoint 状态”断言已通过
- `vue-tsc --noEmit` 通过
- `git diff --check` 无空白错误，只有仓库现有 LF/CRLF 提示

## 当前结果

- `PA-018` 的前端 retrieval 消费证据已经不只是“看得到 summary”
- `HomeWorkspace` 现在也开始消费更深层的运行态结构化事实，说明 retrieval boundary 正在向 graph/checkpoint 邻接 UI 收口
- 但 `PA-018` 仍不能关单，因为更深层 runtime / capability 默认消费链路与更完整的稳定事实写入来源还没补齐

## 下一步动作

1. 继续梳理哪些 runtime / graph / capability 入口还在直接消费原始 session/run/checkpoint 原件
2. 继续扩展 `LongTermMemory` 的保守、可审计稳定事实来源
3. 在接近收口前，围绕 `PA-018` 验收标准逐项补齐最终证据

## 当前卡点

- 更深层 capability / bridge 层还没有系统迁移到 retrieval boundary
- `LongTermMemory` 的稳定事实来源仍然偏少
- 整体验收闭环仍缺少足够广的最终收口证据
