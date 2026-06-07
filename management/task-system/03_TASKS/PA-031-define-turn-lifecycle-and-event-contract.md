# PA-031 定义 turn lifecycle 与 event contract

## 状态
- Status: `Done`
- Priority: `P0`
- Owner: `Codex`

## OpenSpec Change
- [add-turn-lifecycle-event-contract](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-turn-lifecycle-event-contract)

## Delta Spec
- [turn-lifecycle-event-contract/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-turn-lifecycle-event-contract/specs/turn-lifecycle-event-contract/spec.md)

## Canonical Spec
- 交付后沉淀到 `openspec/specs/` 的 turn lifecycle canonical spec

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `ready`

## 目标
把 Pony Agent 当前分散在 runtime、stream、trace、checkpoint、history 与前端 store 中的 turn 生命周期语义正式收口成统一 contract，作为 trace、recovery 与 hooks 的共同底座。

## 输出
- canonical turn phase 状态机
- canonical turn event vocabulary 与最小 payload
- model hop / tool hop / terminal state 的统一表达
- Rust/TS 共享 contract 收口方案
- 生命周期母文档、任务系统与 OpenSpec 同步

## 验收标准
- 一个 turn 的 phase 与 terminal semantics 有唯一正式定义
- stream、trace、checkpoint、前端消费不得再各自发明冲突生命周期
- 一级事件名集合有显式 MUST 级约束，不再只是文档目标
- 事件流具备顺序号、可重放字段与终态一致性约束
- 多 hop 链路中，model hop 与 tool hop 的边界可以被稳定表达
- `checkpointing` 只表达生命周期边界，不单独承诺 recovery 能力
- 相关 Rust/前端测试覆盖正常完成、失败、取消与多 hop

## 当前进展
- 已完成架构母文档 `docs/architecture/turn-lifecycle-hooks-and-recovery.md`
- 已建立 OpenSpec change：`add-turn-lifecycle-event-contract`
- 已形成 phase / event / hook boundary 的第一版正式结论
- 已完成独立 spec 审核并采纳修订，见：
  [2026-06-03-pa031-pa032-pa033-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-03-pa031-pa032-pa033-spec-review.md)
- 已完成第一轮 contract 落地：
  - Rust `emit_event(...)` 统一补齐 `eventType / eventVersion / sequence / emittedAtMs`
  - Rust `TurnStreamEvent` 与 SSE 事件帧已补齐 `eventId / sessionId`
  - terminal 事件后端会清理 turn 级 sequence registry
  - TS `TurnStreamEvent`、lifecycle phase 与 event type 已对齐 canonical 字段
  - `src/stores/runtime.ts` 已开始按 canonical metadata 承接 phase，而不是只靠旧 `phase` 字段猜测
  - fallback trace timeline 已开始根据 canonical `eventType / phase` 判断 active hop，而不是只靠 `calling_model / calling_tool`
  - session restore / checkpoint restore 已补 canonical phase 归一化
- 已完成第二轮 lifecycle consumption 收口：
  - 后端 `ExecutionCheckpoint` 已显式补齐 `contractVersion / runId / projectedRuntimePhase / submissionCommand`
  - 前端 `runtime store` 现在会优先消费后端投影的 phase / submission contract，再回退到本地兼容推断
  - recovery checkpoint 在 `runState` 缺失场景下不再必须依赖前端 `activeRunId + recoveryMode + phase` 自行拼装恢复命令
- `cargo check --manifest-path src-tauri/Cargo.toml --tests` 已通过
- `cargo test --manifest-path src-tauri/Cargo.toml --lib sse_adapter::tests::format_sse_event_uses_standard_event_id_and_data_lines -- --exact` 已通过
- `npm run test:unit -- --run tests/runtime-store.spec.ts` 已通过
- `npx vue-tsc --noEmit` 已通过

## 下一步动作
- 后续实现卡继续直接消费 canonical lifecycle contract，不再在本卡内扩 recovery / hooks / session UX 范围
- 如未来需要继续收紧 UI runtime phase 投影，可在新 change 中推进，不回灌 `PA-031`
- 如需沉淀 canonical spec 归档，按后续 OpenSpec 流程单独执行

## 当前卡点
- 暂无；本卡已完成关闭

## 断点续跑提示
继续前先看：
- `docs/architecture/turn-lifecycle-hooks-and-recovery.md`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/turn_flow.rs`
- `src-tauri/src/agent/control_plane.rs`
- `src/types/runtime.ts`
- `src/stores/runtime.ts`
