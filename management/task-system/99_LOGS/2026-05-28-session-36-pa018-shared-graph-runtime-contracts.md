# 2026-05-28 Session 36 PA-018 Shared Graph Runtime Contracts

## 本轮目标

- 继续推进 `PA-018`
- 把前端 graph/retrieval 邻接 contract 从多份局部定义收敛到共享类型
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

- 在 `src/types/runtime.ts` 中正式导出了共享的：
  - `GraphRunPhase`
  - `GraphDecision`
  - `GraphStep`
  - `GraphTurnHandoff`
  - `GraphRun`
  - `GraphRunCheckpoint`
  - `GraphRunEvent`
  - `GraphRunTurnResponse`
  - `GraphRunControlResponse`
  - `GraphRunStreamStartResponse`
  - `HostInspectionSnapshot`
- `runtime store`、`HomeSidebar`、`HomeWorkspace` 已开始复用这些共享类型
- 这次类型收敛还顺带暴露并修复了一个真实 contract 缺口：
  - 前端派生的最小 `lastHandoff` 之前缺少 `recentAttachmentAssetCount / longTermMemoryStatus / longTermMemoryEntryCount`
  - 现在这些字段已补齐，前端最小 handoff 不再明显落后于后端真实 contract

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

- `PA-018` 的“稳定 contract”不再只停留在 Rust 侧，前端 graph/retrieval 邻接边界也开始有统一 contract
- retrieval boundary 在前端被局部漂移、局部缩水的风险更低了
- 这让后续继续推进更多 UI / capability 入口迁移到 retrieval 视图时，边界更容易保持一致

## 下一步动作

1. 继续找出仍默认依赖原始 `session/run/checkpoint` 原件的 capability 或 UI 入口
2. 继续扩展 `LongTermMemory` 的其他保守稳定事实来源
3. 逐项补齐 `PA-018` 收口前的最终验收证据

## 当前卡点

- capability / bridge 层的系统迁移还没完成
- 稳定事实来源虽然继续增多，但整体覆盖仍不足
- 现有验证已经很强，但还没有形成足以直接关单的终态证明
