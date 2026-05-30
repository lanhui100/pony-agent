# 2026-05-28 Session 34 PA-018 UI Surfaces Active Task Fact

## 本轮目标

- 继续推进 `PA-018`
- 把 retrieval 中的显式当前任务焦点继续上浮到前端默认消费入口
- 补齐对应前端验证与任务文档回写

## 本轮改动

- 更新：
  - `src/types/runtime.ts`
  - `src/components/HomeSidebar.vue`
  - `src/components/HomeSessionSidebar.vue`
  - `src/components/HomeWorkspace.vue`
  - `tests/HomeSidebar.spec.ts`
  - `tests/HomeSessionSidebar.spec.ts`
  - `tests/HomeWorkspace.spec.ts`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/00_DASHBOARD.md`
  - `management/task-system/01_TASK_BOARD.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- 在前端类型层新增了 `extractActiveTaskFocus()` helper
- `HomeSidebar` 的 retrieval 状态面板现在会显示 `当前任务：PA-018`
- `HomeSessionSidebar` 的当前上下文卡片现在会显示 `当前任务：PA-018`
- `HomeWorkspace` 的顶部 retrieval 概览条现在也会显示 `当前任务：PA-018`
- 这让 `project_focus.active_task` 不再只停留在后端 memory/graph/planner，而是进入前端默认 retrieval 视图

## 验证

已通过：

```powershell
npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeSessionSidebar.spec.ts tests/HomeWorkspace.spec.ts
npm exec vue-tsc -- --noEmit
npm run verify
```

结果：

- 三个定向前端测试文件共 `26` 条测试全部通过
- `vue-tsc` 通过
- `npm run verify` 重新通过
- 当前结果为前端 `60` 个测试、`vite build` 与 Rust `cargo check` 全部通过

## 当前结果

- `PA-018` 在 `C. runtime 接入` 上继续前进，因为 retrieval 中的结构化项目事实已经进入三个默认 UI 入口
- retrieval boundary 更接近“上层默认消费稳定事实”，而不只是提供一个底层查询结果
- 现在从 memory 写入、retrieval 读取、graph/planner 消费到 UI 展示，`project_focus.active_task` 已经形成一条更完整的证据链

## 下一步动作

1. 继续找出仍默认依赖原始 `session/run/checkpoint` 原件的 capability 或 UI 入口
2. 继续扩展 `LongTermMemory` 的其他保守稳定事实来源
3. 逐项补齐 `PA-018` 收口前的最终验收证据

## 当前卡点

- capability / bridge 层的系统迁移还没完成
- 稳定事实来源虽然继续增多，但整体覆盖仍不足
- 现有验证已经很强，但还没有形成足以直接关单的终态证明
