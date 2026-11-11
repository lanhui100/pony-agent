# 2026-05-28 Session 32 PA-018 Sidebar Workspace RunState Preferred

## 本轮目标

- 继续推进 `PA-018`
- 把组件层仍直接依赖 `inspect_host` 的 run 视图继续收口到 retrieval boundary
- 补齐对应前端验证与任务文档回写

## 本轮改动

- 更新：
  - `src/components/HomeSidebar.vue`
  - `src/components/HomeWorkspace.vue`
  - `tests/HomeSidebar.spec.ts`
  - `tests/HomeWorkspace.spec.ts`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/00_DASHBOARD.md`
  - `management/task-system/01_TASK_BOARD.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- 把 `HomeSidebar` 的 run 面板改成优先消费 `retrievedContext.runState`
- 把 `HomeWorkspace` 的 graph run 刷新改成优先消费 `retrievedContext.runState`
- 当前会先用 retrieval facts 构造最小 run 视图，只有 retrieval facts 不足时才回退到 `inspect_host`
- `HomeWorkspace` 还会直接用 retrieval 中的 `runId` 去加载 checkpoint，而不是默认先拉原始 run 列表
- 新增两条前端单测，覆盖：
  - `HomeSidebar` 不默认依赖 `inspect_host`
  - `HomeWorkspace` 不默认依赖 `inspect_host`，并直接用 retrieval `runId` 加载 checkpoint

## 验证

已通过：

```powershell
npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts tests/HomeSessionSidebar.spec.ts tests/runtime-store.spec.ts
npm exec vue-tsc -- --noEmit
npm run verify
git diff --check -- src/components/HomeSidebar.vue src/components/HomeWorkspace.vue tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts
```

结果：

- `tests/HomeSidebar.spec.ts`、`tests/HomeWorkspace.spec.ts`、`tests/HomeSessionSidebar.spec.ts` 与 `tests/runtime-store.spec.ts` 共 `51` 个测试全部通过
- `HomeSidebar.spec.ts` 从 `6` 条增长到 `7` 条
- `HomeWorkspace.spec.ts` 从 `11` 条增长到 `12` 条
- `npm run verify` 重新通过，当前结果为前端 `60` 个测试、`vite build` 与 Rust `cargo check` 全部通过
- `git diff --check` 无空白错误，只有仓库现有 LF/CRLF 提示

## 当前结果

- `PA-018` 在 `C. runtime 接入` 上继续前进，因为组件层的 run 视图不再默认先吃原始 host inspection
- 现在从提交主链、右侧 run 面板到主工作区 run 刷新，都已经开始优先消费 `retrievedContext.runState`
- 这让 retrieval boundary 更接近“默认上层入口”，而不只是附加信息

## 下一步动作

1. 继续找出仍默认依赖原始 `session/run/checkpoint` 原件的 capability 或 UI 入口
2. 继续扩展 `LongTermMemory` 的其他保守稳定事实来源
3. 在接近收口前，按 `PA-018` 验收标准逐项补齐最终证据

## 当前卡点

- capability / bridge 层的系统迁移还没完成
- 稳定事实来源虽然增多，但整体覆盖仍不足
- 现有验证很强，但还没有形成足以直接关单的逐项终态证明
