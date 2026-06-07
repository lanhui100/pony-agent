# 2026-06-04 PA-034 Spec Review

## 审核范围

- [PA-034 任务卡](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-034-implement-checkpoint-lifecycle-boundary.md)
- [proposal.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-checkpoint-lifecycle-boundary-implementation/proposal.md)
- [design.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-checkpoint-lifecycle-boundary-implementation/design.md)
- [spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-checkpoint-lifecycle-boundary-implementation/specs/checkpoint-lifecycle-boundary/spec.md)
- [tasks.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-checkpoint-lifecycle-boundary-implementation/tasks.md)

## 审核方式

- 子智能体只读审计
- 审计代理：`Heisenberg`（`019e8e84-25bb-7eb1-aad1-60226ff5210c`）

## 审核结论

- 方向正确，适合作为 `PA-031 / 032 / 033` 之后的独立可执行卡
- 关键实现缺口确实在后端 runtime boundary，而不是前端类型定义
- 最小可执行闭环应锁定：
  - `src-tauri/src/agent/runtime.rs`
  - `src-tauri/src/agent/session.rs`
  - `src-tauri/src/agent/execution_control.rs`
  - `src-tauri/src/agent/control_plane.rs`
  - `src/stores/runtime.ts`

## 采纳意见

1. 明确本卡的主战场是后端 runtime / persistence / control-plane，而不是先扩前端 UI。
2. 在 design 中补充 `phase / status / projectedRuntimePhase` 的兼容约束，避免 checkpoint boundary 被 terminal 提前吞没。
3. 在 implementation outline 中显式列出最小闭环文件，避免实现时范围漂移。
4. 在 verification strategy 中补充 backward-compatible default，确保历史 session 中缺失新 evidence 时仍能安全降级。
5. 保持“`checkpointing` 不等于 recovery checkpoint”作为最高优先级兼容边界。

## 未采纳意见

- “本轮同时覆盖所有 failed / cancelled 路径的 checkpoint persisted 事件”
  - 未采纳原因：当前更合理的 contract 是只在真实持久化提交边界发 `turn.checkpoint_persisted`，不能为了对称性伪造失败/取消的 persisted checkpoint 事实。

## 结果

- 本轮 spec 审核通过
- 已按采纳意见更新 `design.md`
