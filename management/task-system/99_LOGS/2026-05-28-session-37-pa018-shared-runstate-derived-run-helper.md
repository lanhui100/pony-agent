# 2026-05-28 Session 37 PA-018 Shared RunState Derived Run Helper

## 本轮目标

- 继续推进 `PA-018`
- 把前端 `runState -> 最小 GraphRun` 的派生逻辑收敛到共享 helper
- 补齐对应验证与任务文档回写

## 本轮改动

- 更新：
  - `src/types/runtime.ts`
  - `src/stores/runtime.ts`
  - `src/components/HomeSidebar.vue`
  - `src/components/HomeWorkspace.vue`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/00_DASHBOARD.md`
  - `management/task-system/01_TASK_BOARD.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- 在 `src/types/runtime.ts` 中新增了共享 helper：
  - `normalizeGraphRunPhase()`
  - `deriveGraphRunFromRunState()`
- `HomeSidebar` 与 `HomeWorkspace` 不再各自手写一份 `retrievedContext.runState -> GraphRun` 映射
- 共享 helper 还统一补齐了前端最小 `lastHandoff` 的最小字段集合：
  - `recentAttachmentAssetCount`
  - `longTermMemoryStatus`
  - `longTermMemoryEntryCount`
- 这让前端不仅共享了 contract 类型，也开始共享 contract 的解释逻辑

## 验证

已通过：

```powershell
npm exec vue-tsc -- --noEmit
npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts tests/runtime-store.spec.ts
npm run build
```

结果：

- `vue-tsc` 通过
- 三个定向测试文件共 `44` 条测试全部通过
- 前端构建通过

## 当前结果

- `PA-018` 在前端的 retrieval boundary 更稳定了，因为“共享类型一致但共享映射逻辑分叉”的问题开始收口
- `HomeSidebar`、`HomeWorkspace` 与 `runtime store` 围绕 graph/retrieval 的解释模型更一致
- 这让后续继续迁移更多 UI / capability 入口到 retrieval 视图时，重复漂移的风险更低

## 下一步动作

1. 继续找出仍默认依赖原始 `session/run/checkpoint` 原件的 capability 或 UI 入口
2. 继续扩展 `LongTermMemory` 的其他保守稳定事实来源
3. 逐项补齐 `PA-018` 收口前的最终验收证据

## 当前卡点

- capability / bridge 层的系统迁移还没完成
- 稳定事实来源虽然继续增多，但整体覆盖仍不足
- 现有验证已经很强，但还没有形成足以直接关单的终态证明
