# 2026-05-28 Session 23 PA-018 Home Session Sidebar Retrieval Context

## 本轮目标

- 继续推进 `PA-018`
- 在前端再补一条真实 retrieval UI 消费链路
- 让会话导航层也开始直接显示 retrieval facts
- 补测试与任务文档

## 本轮改动

- 更新：
  - `src/components/HomeSessionSidebar.vue`
  - `tests/HomeSessionSidebar.spec.ts`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/00_DASHBOARD.md`
  - `management/task-system/01_TASK_BOARD.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- `HomeSessionSidebar` 新增“当前上下文”卡片
- 该卡片直接消费 `runtimeStore.retrievedContext`
- 会话导航侧栏现在会展示 retrieval 中的结构化事实：
  - 当前 summary
  - recent history 数量
  - recent attachment 数量
  - long-term memory 状态
  - run goal
  - last referenced file
- 这意味着前端 retrieval UI 消费点已经从：
  - `HomeSidebar`
  扩展到：
  - `HomeSidebar`
  - `HomeSessionSidebar`

## 验证

已通过：

```powershell
npm exec vitest run tests/HomeSessionSidebar.spec.ts tests/HomeSidebar.spec.ts tests/runtime-store.spec.ts
npm exec vue-tsc -- --noEmit
git diff --check -- src/components/HomeSessionSidebar.vue tests/HomeSessionSidebar.spec.ts src/components/HomeSidebar.vue tests/HomeSidebar.spec.ts src/stores/runtime.ts src/types/runtime.ts tests/runtime-store.spec.ts
```

结果：

- `tests/HomeSessionSidebar.spec.ts`、`tests/HomeSidebar.spec.ts` 与 `tests/runtime-store.spec.ts` 共 `36` 个测试全部通过
- 新增 session sidebar 测试验证：
  - 当前 retrieval summary 渲染
  - retrieval run goal 渲染
  - retrieval last referenced file 渲染
  - long-term memory 状态渲染
- `vue-tsc --noEmit` 通过
- `git diff --check` 无空白错误，只有仓库现有 LF/CRLF 提示

## 当前结果

- 前端 retrieval 收口已经不只是单一状态面板
- 会话导航层也已经开始直接显示 retrieval facts
- 这使得后续继续把 `HomeWorkspace` 或其它 capability 入口迁到 retrieval 视图时，有了第二个更靠近导航/摘要层的复用模式

## 下一步动作

1. 继续评估 `HomeWorkspace` 中哪些会话状态、graph 状态或摘要视图最适合迁到 retrieval facts
2. 找出仍直接依赖原始 `SessionSnapshot` 的前端状态展示链路
3. 继续扩展 `LongTermMemory` 的保守稳定事实来源，并补对应 retrieval 展示与验证
4. 在接近收口前，按 `PA-018` 验收标准逐条核对完成度

## 当前卡点

- `HomeSidebar` 与 `HomeSessionSidebar` 已有真实 retrieval UI 消费点，但 `HomeWorkspace` 和更多 capability 入口还未系统迁移
- `LongTermMemory` 仍主要覆盖显式用户偏好与显式 note，稳定事实来源不够完整
- 仍不能把 `PA-018` 标记为完成，因为 retrieval boundary 的前端/能力层收口还没有达到验收标准要求的充分覆盖
