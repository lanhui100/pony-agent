# 2026-05-28 Session 22 PA-018 Home Sidebar Retrieval Consumption

## 本轮目标

- 继续推进 `PA-018`
- 在前端找到一个真实上层入口消费 retrieval 视图
- 补测试与任务文档，避免 `retrievedContext` 只停留在 store 状态

## 本轮改动

- 更新：
  - `src/components/HomeSidebar.vue`
  - `tests/HomeSidebar.spec.ts`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/00_DASHBOARD.md`
  - `management/task-system/01_TASK_BOARD.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- `HomeSidebar` 的状态面板现在会直接消费 `runtimeStore.retrievedContext`
- 状态面板现在优先显示 retrieval 中的 `sessionContext.summary`
- 状态面板现在会展示 retrieval 中的结构化事实：
  - `recent_history` 数量
  - `recent_attachment_assets` 数量
  - `long_term_memory` 状态与条目预览
  - `last_referenced_file`
  - `run goal / run phase`
- 这意味着 `PA-018` 在前端侧已经不只是“store 里存在 retrieval 结果”
- 已经有一个真实 UI 面板开始把 retrieval 视图作为默认上层事实来源

## 验证

已通过：

```powershell
npm exec vitest run tests/HomeSidebar.spec.ts tests/runtime-store.spec.ts
npm exec vue-tsc -- --noEmit
git diff --check -- src/components/HomeSidebar.vue tests/HomeSidebar.spec.ts src/stores/runtime.ts src/types/runtime.ts tests/runtime-store.spec.ts
```

结果：

- `tests/HomeSidebar.spec.ts` 与 `tests/runtime-store.spec.ts` 共 `29` 个测试全部通过
- 新增 sidebar 测试验证：
  - retrieval summary 渲染
  - long-term memory 条目渲染
  - retrieval summary 覆盖旧的 legacy session summary
- `vue-tsc --noEmit` 通过
- `git diff --check` 无空白错误，只有仓库现有 LF/CRLF 提示

## 当前结果

- 前端 retrieval 收口已经从 store 状态推进到了真实 UI 消费
- `HomeSidebar` 成为第一条明确消费 `RetrievedContextState` 的前端展示链路
- 这让后续继续把其它 UI / capability 面板迁移到 retrieval 视图时，有了可复用模式

## 下一步动作

1. 继续评估 `HomeWorkspace / HomeSessionSidebar` 哪些状态面板最适合继续收口到 retrieval facts
2. 找出仍直接依赖原始 `SessionSnapshot` 的前端会话摘要或状态展示链路
3. 继续扩展 `LongTermMemory` 的保守稳定事实来源，并补对应 retrieval 显示与验证
4. 在接近收口前，按 `PA-018` 验收标准逐条核对完成度

## 当前卡点

- 目前只有 `HomeSidebar` 已经形成明确的 retrieval UI 消费点，其它前端面板还未系统迁移
- `LongTermMemory` 仍主要覆盖显式用户偏好与显式 note，缺少更完整但仍可审计的稳定事实来源
- 仍不能把 `PA-018` 视为完成，因为 capability / bridge / 更多上层 UI 还没有全面收口到 retrieval boundary
