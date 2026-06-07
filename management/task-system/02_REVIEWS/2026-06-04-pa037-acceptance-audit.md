# PA-037 Acceptance Audit

## 审核范围

- `management/task-system/03_TASKS/PA-037-build-session-control-surface-and-feedback-loop.md`
- `openspec/changes/add-session-control-surface-and-feedback-loop/specs/session-control-surface-and-feedback-loop/spec.md`
- `openspec/changes/add-session-control-surface-and-feedback-loop/tasks.md`
- `src/components/HomeSessionSidebar.vue`
- `src/components/HomeWorkspace.vue`
- `src/stores/runtime.ts`
- `tests/HomeSessionSidebar.spec.ts`
- `tests/HomeWorkspace.spec.ts`
- `tests/runtime-store.spec.ts`

## 审核口径

只按 `PA-037` 的完成边界判断：确认既有 runtime/control-plane/history graph 合同已经被前端收口成显式 session 控制交互、统一状态语言与可验证结果反馈；不把新的后端 resume/replay 命令、hooks runtime 扩展或更大范围的 history/recovery 合同改造重新算回本卡。

### 不在本审计内

- 新增 stop / resume / replay 后端命令
- hooks runtime boundary 扩展与 evidence model 继续演进：`PA-035`
- trace persistence / checkpoint / recovery contract 主体：`PA-032 / PA-034`

## 逐项结论

### A. explicit session control entrypoints

状态：`达成`

证据：

- `HomeWorkspace.vue` 已把发送区主 CTA 收口成显式 `恢复 / 继续 / 重新开始`
- `HomeWorkspace.vue` 在运行中已显示显式 `停止` CTA，并在 stop 请求发出后展示反馈
- CTA 文案直接绑定 `latestGraphRunSubmissionPlan / latestExecutionCheckpoint / phase`
- `tests/HomeWorkspace.spec.ts` 已覆盖：
  - paused run -> `恢复`
  - lifecycle boundary / replay-required -> `重新开始`
  - running -> `停止`

### B. checkout / restore result visibility

状态：`达成`

证据：

- `HomeSessionSidebar.vue` 已对 transcript-only checkout degrade 显示“仅恢复对话，未恢复工作区”与 degradation reason
- `HomeSessionSidebar.vue` 已在 restore / fork / branch switch 成功后展示 `branch / visible node / mode` 变化
- `tests/HomeSessionSidebar.spec.ts` 已覆盖：
  - checkout degrade feedback
  - restore success feedback
  - fork success feedback
  - branch switch success feedback

### C. unified user-facing vocabulary and disabled reasons

状态：`达成`

证据：

- `HomeSessionSidebar.vue` 已把 `live / historical / historical_dirty` 映射为统一用户状态语言
- `HomeSessionSidebar.vue` 已新增 session control 状态卡，把 `paused / recovery-capable / replay-required` 收口为用户可读标签
- disabled reason 已覆盖：
  - 运行中不可切换
  - 空白或缺少历史节点不可操作
  - 无需恢复到分支头
  - 不可切换分支
- `tests/HomeSessionSidebar.spec.ts` 已覆盖 `historical_dirty` 文案映射与运行中 disabled reason

### D. dispatch contract integrity

状态：`达成`

证据：

- `HomeWorkspace` 的显式 CTA 最终仍只派发到既有 `runtimeStore.submitTurn()` / `runtimeStore.stopTurn()` 边界
- `runtime-store.spec.ts` 既有回归继续覆盖：
  - `stop_graph_run`
  - `resume_graph_run_stream`
  - `continue_graph_run_stream`
  - `start_graph_run_stream`
- 本轮重新执行 `npx vitest run tests/runtime-store.spec.ts --pool=forks --maxWorkers=1`，结果 `55 passed`

### E. verification

状态：`达成`

本轮完成态验证：

```powershell
npx vitest run tests/HomeSessionSidebar.spec.ts --pool=forks --maxWorkers=1
npx vitest run tests/HomeWorkspace.spec.ts --pool=forks --maxWorkers=1
npx vitest run tests/runtime-store.spec.ts --pool=forks --maxWorkers=1
```

结果：

- `HomeSessionSidebar.spec.ts`：`24 passed`
- `HomeWorkspace.spec.ts`：`18 passed`
- `runtime-store.spec.ts`：`55 passed`

## 最终裁定

`PA-037` 已满足任务卡与 delta spec 定义的完成边界，可以从 `Review` 更新为 `Done`。

关闭理由：

1. stop / resume / continue / replay 的用户入口、历史反馈、状态语言与 disabled reason 已形成可见且可验收的前端闭环。
2. `HomeWorkspace / HomeSessionSidebar / runtime-store` 三层验证已经覆盖核心交互路径，不再只是“底层能力存在”。
3. 本卡只消费既有 runtime/history/control-plane 合同，没有越界扩成新的后端命令设计。
4. OpenSpec `tasks.md` 已全部完成，后续若要扩真正的专用 replay/resume 命令，应另立新卡。
