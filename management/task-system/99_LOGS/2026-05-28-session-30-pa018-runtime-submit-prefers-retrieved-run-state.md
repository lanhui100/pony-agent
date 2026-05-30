# 2026-05-28 Session 30 PA-018 Runtime Submit Prefers Retrieved Run State

## 本轮目标

- 继续推进 `PA-018`
- 找到一个仍直接依赖原始 run 元件的真实上层入口
- 把它优先收口到 retrieval boundary，并补更强验证证据

## 本轮改动

- 更新：
  - `src/stores/runtime.ts`
  - `tests/runtime-store.spec.ts`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/00_DASHBOARD.md`
  - `management/task-system/01_TASK_BOARD.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- 把前端 `runtime store.submitTurn()` 的 graph stream 提交流程继续收口到 retrieval boundary
- 当前在决定：
  - `start_graph_run_stream`
  - `continue_graph_run_stream`
  - `resume_graph_run_stream`
  时，会优先消费 `retrievedContext.runState`
- 只有当 retrieval facts 不足时，才回退到 `inspect_host` 读取原始 run 列表
- 新增两条单测，覆盖：
  - 通过 `retrievedContext.runState` 继续已有 run
  - 通过 `retrievedContext.runState` 恢复已暂停 run

## 验证

已通过：

```powershell
npm exec vitest run tests/runtime-store.spec.ts tests/HomeWorkspace.spec.ts tests/HomeSidebar.spec.ts tests/HomeSessionSidebar.spec.ts
npm exec vue-tsc -- --noEmit
npm run verify
git diff --check -- src/stores/runtime.ts tests/runtime-store.spec.ts
```

结果：

- `tests/runtime-store.spec.ts`、`tests/HomeWorkspace.spec.ts`、`tests/HomeSidebar.spec.ts` 与 `tests/HomeSessionSidebar.spec.ts` 共 `49` 个测试全部通过
- `runtime-store.spec.ts` 从 `23` 个测试增长到 `25` 个，新增长的两条 runState 提交流程断言通过
- `npm run verify` 重新通过，当前结果为前端 `58` 个测试、`vite build` 与 Rust `cargo check` 全部通过
- `git diff --check` 无空白错误，只有仓库现有 LF/CRLF 提示

## 当前结果

- `PA-018` 的 runtime 接入证据进一步增强，不再只是“loadSessionState 会加载 retrieval”
- 前端主提交流程也开始直接消费 retrieval facts，而不是默认先去 inspection 拉原始 run 列表
- 这让 `PA-018` 在验收项 `C. runtime 接入` 上更接近“默认上层消费 retrieval boundary”的目标

## 下一步动作

1. 继续找出仍默认依赖原始 `run/checkpoint/session` 原件的 capability 或 UI 入口
2. 继续扩展 `LongTermMemory` 的保守、可审计稳定事实来源
3. 在接近收口前，按 `PA-018` 验收标准逐项补齐最终证据，尤其是更广范围的 Rust/整仓级证明

## 当前卡点

- 仍有部分更深层 capability / bridge 层未系统迁移到 retrieval boundary
- `LongTermMemory` 稳定事实来源依然偏少
- 现有验证虽更强，但仍未形成足以直接关单的逐项终态证明
