# 2026-05-28 Session 24 PA-018 Home Workspace Retrieval Strip

## 本轮目标

- 继续推进 `PA-018`
- 把 retrieval 事实再推进到主工作区层
- 让 `HomeWorkspace` 也拥有真实的 retrieval UI 消费点
- 补测试与任务文档

## 本轮改动

- 更新：
  - `src/components/HomeWorkspace.vue`
  - `tests/HomeWorkspace.spec.ts`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/00_DASHBOARD.md`
  - `management/task-system/01_TASK_BOARD.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- `HomeWorkspace` 顶部新增 retrieval 上下文概览条
- 该概览条直接消费 `runtimeStore.retrievedContext`
- 主工作区现在会展示 retrieval 中的结构化事实：
  - 当前 summary
  - history 数量
  - attachment 数量
  - long-term memory 数量
  - run goal
  - last referenced file
- 前端 retrieval UI 消费点现在已经覆盖：
  - `HomeSidebar`
  - `HomeSessionSidebar`
  - `HomeWorkspace`

## 验证

已通过：

```powershell
npm exec vitest run tests/HomeWorkspace.spec.ts tests/HomeSessionSidebar.spec.ts tests/HomeSidebar.spec.ts tests/runtime-store.spec.ts
npm exec vue-tsc -- --noEmit
git diff --check -- src/components/HomeWorkspace.vue tests/HomeWorkspace.spec.ts src/components/HomeSessionSidebar.vue tests/HomeSessionSidebar.spec.ts src/components/HomeSidebar.vue tests/HomeSidebar.spec.ts src/stores/runtime.ts src/types/runtime.ts tests/runtime-store.spec.ts
```

结果：

- `tests/HomeWorkspace.spec.ts`、`tests/HomeSessionSidebar.spec.ts`、`tests/HomeSidebar.spec.ts` 与 `tests/runtime-store.spec.ts` 共 `46` 个测试全部通过
- 新增 workspace 测试验证：
  - retrieval summary 渲染
  - retrieval run goal 渲染
  - retrieval last referenced file 渲染
  - retrieval memory 数量渲染
- `vue-tsc --noEmit` 通过
- `git diff --check` 无空白错误，只有仓库现有 LF/CRLF 提示

## 当前结果

- retrieval boundary 的前端消费已经从 store 扩展到主工作区、会话导航和右侧状态栏三类 UI
- 这说明 `PA-018` 的前端默认上层入口正在逐步从原始 session artifacts 收口到结构化 retrieval facts
- 但当前还不能视为完成，因为更深层的 graph/checkpoint/capability 展示链路和 `LongTermMemory` 稳定事实来源还没有收口到位

## 下一步动作

1. 继续评估 `HomeWorkspace` 中 graph 状态、checkpoint 视图和继续/恢复提示哪些适合进一步收口到 retrieval facts
2. 找出仍直接依赖原始 `SessionSnapshot` 的前端状态或 capability 展示链路
3. 继续扩展 `LongTermMemory` 的保守稳定事实来源，并补对应 retrieval 展示与验证
4. 接近收口前，按 `PA-018` 验收标准逐条核对完成度

## 当前卡点

- 虽然前端已有三条 retrieval UI 消费链路，但更多深层 capability / graph 视图还未系统迁移
- `LongTermMemory` 仍主要覆盖显式用户偏好与显式 note，稳定事实来源仍不充分
- 仍不能把 `PA-018` 标记为完成，因为验收标准要求的“更多上层默认拿 retrieval 或稳定衍生 contract”还未达到足够覆盖
