# 2026-05-28 Session 41 PA-018 Runtime Store Summary Retrieval First

## 本轮目标

- 继续推进 `PA-018`
- 找出前端 runtime 默认状态里仍保留原始 `SessionSnapshot.summary` 的入口
- 把 store 级 `sessionSummary` 收口到 retrieval-first
- 同步回写任务文档和验证证据

## 本轮改动

- 更新：
  - `src/stores/runtime.ts`
  - `tests/runtime-store.spec.ts`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- `runtime store.applySessionSnapshot()` 现在会优先采用：
  - `retrieved.sessionContext.summary`
- 只有 retrieval summary 不可用时，才会回退：
  - 持久化 runtime state 里的 `sessionSummary`
  - 原始 `SessionSnapshot.summary`
- `createSnapshotFromRuntimeState()` 也开始优先使用：
  - `retrievedContext.sessionContext.summary`
- 这意味着前端 store 级 summary 不再长期停留在原始 snapshot summary，而是开始和 retrieval boundary 对齐

## 验证

已通过：

```powershell
npm exec vitest run tests/runtime-store.spec.ts tests/HomeSidebar.spec.ts
npm exec vue-tsc -- --noEmit
```

结果：

- `runtime-store.spec.ts` 共 `27` 条通过
- `HomeSidebar.spec.ts` 共 `8` 条通过
- 总计 `35` 条前端定向测试通过
- `vue-tsc` 通过

## 当前结果

- `PA-018` 在 `C. runtime 接入` 上又补了一处更深层收口，因为 retrieval summary 现在不只是显示在局部 UI，而是进入了 store 级默认状态
- 这让后续本地 snapshot / 持久化 / fallback 相关路径更容易继续沿着 retrieval boundary 对齐，而不是回滑到原始 `SessionSnapshot.summary`
- 当前仍不能宣布 `PA-018` 完成交付，因为 capability / bridge 层的系统迁移和更完整的稳定事实来源仍未收口

## 下一步动作

1. 继续找出 capability / bridge 层里仍默认读取原始 `session/run/checkpoint` 的入口
2. 继续扩展 `LongTermMemory` 的其他显式、保守、可审计稳定事实来源
3. 接近收口时，再做一轮正式 closeout audit，判断是否足以把 `PA-018` 从 `In Progress` 切到 `Done`

## 当前卡点

- 前端 store summary 已收口，但 capability / bridge 层还没有形成 retrieval-first 的系统证据
- 现有验证更强了，但还不足以支撑最终关单
