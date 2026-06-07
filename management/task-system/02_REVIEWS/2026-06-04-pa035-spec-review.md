# PA-035 Spec Review

## 审核范围

- `management/task-system/03_TASKS/PA-035-integrate-runtime-hook-dispatch-on-stable-boundaries.md`
- `openspec/changes/add-runtime-hook-dispatch-on-stable-boundaries/proposal.md`
- `openspec/changes/add-runtime-hook-dispatch-on-stable-boundaries/design.md`
- `openspec/changes/add-runtime-hook-dispatch-on-stable-boundaries/specs/runtime-hook-dispatch-on-stable-boundaries/spec.md`
- `openspec/changes/add-runtime-hook-dispatch-on-stable-boundaries/tasks.md`
- `management/task-system/03_TASKS/PA-033-build-agent-hooks-pipeline-foundation.md`

## 审核结论

总体通过，范围拆分合理：

- `PA-033` 收口为 foundation / no-op contract / binding / traceability 是正确方向
- `PA-035` 作为 stable-boundary runtime hook dispatch 实现卡，边界清楚，且没有回吞 `PA-022` 的 post-foundation 扩展范围

## 采纳修改

1. 明确 `PA-035` 首轮是 `trace-first integration`
   - 不要求本轮完成 patch / side-effect 的正式 contract applier
2. 明确 `PA-035` 依赖 `PA-034` 的 checkpoint stable boundary 事实源
   - 但不重复定义 checkpoint / recovery contract
3. 明确首轮 failure policy 以 `observe / non-blocking dispatch` 为主
   - 若纳入 blocking 行为，必须有 terminal outcome + trace evidence 的独立测试
4. 将 `tasks.md` 进一步拆细：
   - 区分普通 stable boundary 与 `CheckpointPersistEnd`
   - 区分 reload roundtrip 与 frontend hydration
   - 补“prepare/context build 不被 runtime 人造接线”的负例任务

## 结果

- 上述意见已采纳并回写到 `PA-035` 的任务卡、proposal、design、spec、tasks
- `PA-033` 的收口保留，不再把 runtime dispatch integration 继续算进 foundation 卡
