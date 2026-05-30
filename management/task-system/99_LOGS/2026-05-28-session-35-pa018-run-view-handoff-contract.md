# 2026-05-28 Session 35 PA-018 Run View Handoff Contract

## 本轮目标

- 继续推进 `PA-018`
- 把 `GraphTurnHandoff.active_task_focus` 的事实链继续补到前端局部 contract 与 run/checkpoint 视图
- 补齐对应前端验证与任务文档回写

## 本轮改动

- 更新：
  - `src/components/HomeSidebar.vue`
  - `src/components/HomeWorkspace.vue`
  - `src/types/runtime.ts`
  - `tests/HomeSidebar.spec.ts`
  - `tests/HomeWorkspace.spec.ts`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/00_DASHBOARD.md`
  - `management/task-system/01_TASK_BOARD.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- 补齐了前端局部 `GraphTurnHandoff` 类型的 `activeTaskFocus`
- `HomeSidebar` 的 trace-run 组头现在会显示 `当前任务：PA-018`
- `HomeWorkspace` 的 run/checkpoint strip 现在也会显示 `当前任务：PA-018`
- 由 retrieval `runState` 派生的最小 run 视图也会把当前任务焦点补进 `lastHandoff`
- 这让 `active_task_focus` 不再只停留在上下文卡片，而是进入 run 视图 contract

## 验证

已通过：

```powershell
npm exec vitest run tests/HomeSidebar.spec.ts tests/HomeWorkspace.spec.ts
npm exec vue-tsc -- --noEmit
npm run build
cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check
```

结果：

- 两个定向前端测试文件共 `19` 条测试全部通过
- `vue-tsc` 通过
- `npm run build` 通过
- Rust `cargo check` 通过

## 当前结果

- `PA-018` 在 `C. runtime 接入` 上继续前进，因为 `active_task_focus` 已经进入前端 run/checkpoint 视图
- retrieval -> handoff -> run view 的结构化事实链更完整了
- 前端局部 contract 不再落后于后端 `GraphTurnHandoff` 的关键字段

## 下一步动作

1. 继续找出仍默认依赖原始 `session/run/checkpoint` 原件的 capability 或 UI 入口
2. 继续扩展 `LongTermMemory` 的其他保守稳定事实来源
3. 逐项补齐 `PA-018` 收口前的最终验收证据

## 当前卡点

- capability / bridge 层的系统迁移还没完成
- 稳定事实来源虽然继续增多，但整体覆盖仍不足
- 现有验证已经很强，但还没有形成足以直接关单的终态证明
