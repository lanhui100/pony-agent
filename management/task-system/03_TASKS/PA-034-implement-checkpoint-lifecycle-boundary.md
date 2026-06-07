# PA-034 落地 checkpoint lifecycle boundary 与 persisted evidence

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## OpenSpec Change
- [add-checkpoint-lifecycle-boundary-implementation](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-checkpoint-lifecycle-boundary-implementation)

## Delta Spec
- [checkpoint-lifecycle-boundary/spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-checkpoint-lifecycle-boundary-implementation/specs/checkpoint-lifecycle-boundary/spec.md)

## Canonical Spec
- 交付后沉淀到 `openspec/specs/` 的 checkpoint lifecycle boundary canonical spec

## Spec 状态
- Proposal: `ready`
- Spec: `ready`
- Design: `ready`
- Tasks: `ready`

## 目标
把 `checkpointing` 从“只存在于 contract 中的名词”推进为 runtime、trace persistence、execution checkpoint 与前端读面都能观察到的真实 lifecycle boundary，并明确它不自动承诺 recovery capability。

## 输出
- checkpoint persist start/end 的 canonical lifecycle 发射面
- `turn.checkpoint_persisted` 的 persisted trace / snapshot evidence
- execution checkpoint 与 session hydration 对 checkpoint lifecycle 的统一读面
- hooks `before/after checkpoint persist` 可对齐的真实 runtime boundary
- Rust / 前端 / reload 验证矩阵

## 验收标准
- 正常完成的 turn 在 terminal 之前显式经过 `checkpointing`
- runtime 能发射与 contract 一致的 checkpoint boundary 事件，不再只在文档中声明
- persisted trace reload 后仍能读到 checkpoint lifecycle evidence
- execution checkpoint / session runtime view 能区分 checkpoint lifecycle boundary 与 recovery capability
- hooks `CheckpointPersistStart / End` 存在真实 lifecycle 锚点
- 测试覆盖 normal completion、tool follow-up completion、reload roundtrip 与“不把 checkpointing 误当 recovery”四类路径

## 当前进展
- 已完成母文档与前置 contract：
  - [PA-031](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-031-define-turn-lifecycle-and-event-contract.md)
  - [PA-032](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-032-stabilize-trace-persistence-and-recovery-contract.md)
  - [PA-033](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-033-build-agent-hooks-pipeline-foundation.md)
- 已完成独立 spec 审核并采纳修订，见：
  [2026-06-04-pa034-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa034-spec-review.md)
- 已确认当前缺口：
  - `CheckpointPersistStart / End` 已有 hooks contract，但 runtime 尚未形成真实发射面
  - `checkpointing` phase 在架构和 spec 中存在，但正常完成链路尚未形成稳定 persisted evidence
  - 这会让 hooks / trace / execution checkpoint / 前端读面继续各自猜测“持久化边界是否发生”
- 已完成第一轮实现落地：
  - `src-tauri/src/agent/runtime.rs` 已在 normal completion 与 tool follow-up completion 链路补 `turn:phase_changed(checkpointing)` 与 `turn:checkpoint_persisted`
  - completed trace timeline 已新增 `checkpoint_persist` evidence，确保 persisted trace / reload 能看见 checkpoint lifecycle boundary
  - `src/stores/runtime.ts` 已开始监听 `turn:phase_changed` 与 `turn:checkpoint_persisted`
  - `tests/runtime-store.spec.ts` 已补事件消费回归断言
- 已完成第二轮后端读面收口：
  - `src-tauri/src/agent/control_plane.rs` 已新增基于 persisted trace 的 `lifecycle_boundary` checkpoint 投影
  - `load_execution_checkpoint(...)` 现在会在 `runtime_control` 与 graph `recovery` 缺席时，回退到 checkpoint lifecycle evidence
  - `load_session_runtime_view(...)` 已能把这类 boundary checkpoint 暴露给上层读面，同时不把它误判成 recovery capability
  - 已新增 control-plane 测试，覆盖“completed session 暴露 lifecycle boundary checkpoint”的合同
- 已完成第三轮 submission-plan 风险收口：
  - 已新增 control-plane 测试，覆盖“仅存在 `lifecycle_boundary` checkpoint 时，submission plan 仍回退到 `default -> start_graph_run_stream`”
  - 这确保 checkpoint lifecycle evidence 只是读面事实，不会劫持 recovery / continue / resume 决策
- 已完成第四轮 reload roundtrip 补强：
  - `src-tauri/src/agent/control_plane.rs` 已新增文件后端 roundtrip 测试，覆盖“session trace 落盘并 reload 后，control plane 仍可投影 `lifecycle_boundary` checkpoint”
  - 该测试同时断言 `load_execution_checkpoint(...)` 与 `load_session_runtime_view(...)` 两个读面在 reload 后都还能恢复 `checkpointing -> connecting` 投影
  - 这把“persisted evidence 存在”推进成了“上层控制面可稳定读回 persisted evidence”
- 已完成本轮验证：
  - `npx vitest run tests/runtime-store.spec.ts` 通过，`51 passed`
  - `npx vue-tsc --noEmit` 通过
  - `cargo check --manifest-path src-tauri/Cargo.toml --tests --quiet` 通过
  - 已通过独立 `CARGO_TARGET_DIR` 跑通以下 exact Rust 用例：
    - `agent::control_plane::tests::completed_session_can_project_checkpoint_lifecycle_boundary_without_recovery`
    - `agent::control_plane::tests::lifecycle_boundary_checkpoint_does_not_override_default_submission_plan`
    - `agent::control_plane::tests::file_backed_reload_restores_lifecycle_boundary_projection`
    - `agent::session::tests::file_backend_roundtrip_restores_checkpoint_persist_evidence`
    - `agent::runtime::tests::start_turn_stream_emits_first_token_latency_on_reasoning_delta`
    - `agent::runtime::tests::start_turn_stream_completes_after_multi_hop_followup_stream`
  - 本轮已修正 multi-hop 流式测试中的过期 mock 合同：
    - OpenAI native tool flow 的 initial decision 现默认先走流式路径，测试第一段响应必须与 runtime 行为一致
    - 该修正消除了“旧 JSON decision mock 被流式初判先消费，随后 `server.finish()` 等待未消费响应”的假性挂起
- 已完成 acceptance audit：
  - 见 [2026-06-04-pa034-acceptance-audit.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa034-acceptance-audit.md)
  - 结论：`PA-034` 已满足任务卡定义的完成边界，可进入 `Review`

## 下一步动作
- 继续把 checkpoint lifecycle boundary 作为 hooks / recovery / monitor 的稳定输入面
- 若后续需要更细粒度 checkpoint boundary 或 non-terminal persistence phase，另拆新卡推进
- 继续保持 OpenSpec / 任务系统 / 验收文档同步

## 当前卡点
- 暂无；本卡已完成关闭

## 断点续跑提示
继续前先看：
- `docs/architecture/turn-lifecycle-hooks-and-recovery.md`
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/turn_flow.rs`
- `src-tauri/src/agent/session.rs`
- `src-tauri/src/agent/execution_control.rs`
- `src/stores/runtime.ts`
