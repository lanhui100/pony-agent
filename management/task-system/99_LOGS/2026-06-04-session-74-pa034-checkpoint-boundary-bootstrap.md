# 2026-06-04 Session 74

## 主题

- `PA-034` checkpoint lifecycle boundary implementation 启动
- 把 `checkpointing` / `turn.checkpoint_persisted` 从 contract 推进为 runtime 真边界

## 本轮完成

1. 新建 `PA-034` 任务卡与 OpenSpec change：
   - [PA-034](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-034-implement-checkpoint-lifecycle-boundary.md)
   - [proposal.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-checkpoint-lifecycle-boundary-implementation/proposal.md)
   - [design.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-checkpoint-lifecycle-boundary-implementation/design.md)
   - [spec.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-checkpoint-lifecycle-boundary-implementation/specs/checkpoint-lifecycle-boundary/spec.md)
   - [tasks.md](/C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/add-checkpoint-lifecycle-boundary-implementation/tasks.md)
2. 完成一轮独立 spec 审核并采纳修订：
   - 审核记录见 [2026-06-04-pa034-spec-review.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa034-spec-review.md)
   - 已采纳“runtime / session / execution_control / control_plane / runtime store 是最小闭环”与“phase/status 优先级兼容必须显式约束”等意见
3. 完成第一轮 runtime 边界落地：
   - `src-tauri/src/agent/runtime.rs` 已在 no-tool completion 与 tool follow-up completion 链路补：
     - `turn:phase_changed` + `checkpointing`
     - `turn:checkpoint_persisted` + `checkpointing`
   - completed persisted trace timeline 已新增 `checkpoint_persist` evidence
4. 完成第一轮前端事件消费闭环：
   - `src/stores/runtime.ts` 已新增 `turn:phase_changed` 与 `turn:checkpoint_persisted` 监听
   - store 现在能把 checkpoint lifecycle boundary 消费为 `connecting` 投影，而不是直接跳过
5. 完成第一轮测试补强：
   - `runtime.rs` 现有 completion 测试已补 checkpoint boundary 顺序与 completed trace timeline 断言代码
   - `tests/runtime-store.spec.ts` 已补 checkpoint lifecycle event 消费断言
6. 完成第二轮 control-plane / runtime-view 收口：
   - `src-tauri/src/agent/control_plane.rs` 已新增 persisted trace -> `lifecycle_boundary` checkpoint 投影
   - `load_execution_checkpoint(...)` 现在在 `runtime_control` 与 graph `recovery` 都缺席时，可回退到 checkpoint lifecycle evidence
   - `load_session_runtime_view(...)` 现在可把该 checkpoint 暴露给会话读面，但仍保持 `recoveryMode=replay_required`、`resumable=false`、`replayable=false`
   - 已新增 control-plane 测试，覆盖“completed session 不应丢失 checkpoint lifecycle boundary，但也不应被提升为 recovery checkpoint”
7. 完成第三轮 submission plan 风险收口：
   - 已新增 control-plane 测试，覆盖“仅存在 `lifecycle_boundary` checkpoint 时，submission plan 仍回退到 `default -> start_graph_run_stream`”
   - 这确保 checkpoint lifecycle evidence 不会误触发 `resume_graph_run_stream` 或 `continue_graph_run_stream`
8. 完成第四轮 reload roundtrip 收口：
   - `src-tauri/src/agent/control_plane.rs` 已新增文件后端 roundtrip 测试
   - 该测试使用真实 `FileSessionBackend` 先写入带 `checkpoint_persist` evidence 的 trace，再重建 `AgentRuntime + HostControlPlane`
   - 已断言 reload 后：
     - `load_execution_checkpoint(...)` 仍返回 `checkpoint_kind=lifecycle_boundary`
     - `phase=checkpointing`
     - `projected_runtime_phase=connecting`
     - `resumable=false`
     - `replayable=false`
     - `submission_command=None`
     - `load_session_runtime_view(...)` 仍能暴露相同 checkpoint 投影

## 验证

- `npx vitest run tests/runtime-store.spec.ts`
  - 结果：通过，`51 passed`
- `npx vue-tsc --noEmit`
  - 结果：通过
- `cargo check --manifest-path src-tauri/Cargo.toml --tests --quiet`
  - 结果：通过
  - 备注：仍有 Windows incremental `os error 5` 警告，但未阻断编译
- `cargo test --manifest-path src-tauri/Cargo.toml file_backed_reload_restores_lifecycle_boundary_projection --lib -- --exact`
  - 结果：已通过独立 `CARGO_TARGET_DIR` 跑通 exact
- `cargo test --manifest-path src-tauri/Cargo.toml completed_session_can_project_checkpoint_lifecycle_boundary_without_recovery -- --exact`
  - 结果：已通过独立 `CARGO_TARGET_DIR` 跑通 exact
- `cargo test --manifest-path src-tauri/Cargo.toml lifecycle_boundary_checkpoint_does_not_override_default_submission_plan -- --exact`
  - 结果：已通过独立 `CARGO_TARGET_DIR` 跑通 exact
- `cargo test --manifest-path src-tauri/Cargo.toml agent::session::tests::file_backend_roundtrip_restores_checkpoint_persist_evidence --lib -- --exact`
  - 结果：已通过独立 `CARGO_TARGET_DIR` 跑通 exact
- `cargo test --manifest-path src-tauri/Cargo.toml agent::runtime::tests::start_turn_stream_emits_first_token_latency_on_reasoning_delta --lib -- --exact`
  - 结果：已通过独立 `CARGO_TARGET_DIR` 跑通 exact
  - 备注：以上 exact 结果说明 `PA-034` 已不再只停留在 `cargo check` 护栏，control-plane / session / 部分 runtime completion 路径已具备稳定执行证据
- `cargo test --manifest-path src-tauri/Cargo.toml agent::runtime::tests::start_turn_stream_completes_after_multi_hop_followup_stream --lib -- --exact`
  - 结果：已通过独立 `CARGO_TARGET_DIR` 跑通 exact
  - 备注：本轮同时修正了 stream 测试中过期的 initial decision mock 合同，确认此前“挂起”并非 checkpoint runtime 主逻辑回归

## 本轮补充收口

1. 定位并修复了 multi-hop exact 的假性挂起根因：
   - 当前 OpenAI native tool flow 会先走流式 initial decision
   - 旧测试仍把第一段 mock 写成 JSON decision，导致流式初判先消费错误响应，随后 `server.finish()` 卡在未消费响应
2. 已把相关 runtime stream 测试对齐到当前事实：
   - `start_turn_stream_completes_after_multi_hop_followup_stream`
   - `start_turn_stream_accumulates_token_usage_across_tool_followups`
   - `start_turn_stream_repairs_blank_tool_name_in_followup_stream`
3. 已完成 `PA-034` 最终验收补强：
   - 六条关键 Rust exact 用例本轮重新跑通
   - `npx vitest run tests/runtime-store.spec.ts` 本轮通过，`51 passed`
   - `npx vue-tsc --noEmit` 本轮通过
4. 已发起并完成 acceptance audit：
   - 见 [2026-06-04-pa034-acceptance-audit.md](/C:/Users/HUAWEI/Documents/pony-agent/management/task-system/02_REVIEWS/2026-06-04-pa034-acceptance-audit.md)
   - 结论：`PA-034` 可从 `In Progress` 移到 `Review`

## 当前边界

- 本轮只把 checkpoint lifecycle boundary 接到 normal completion 与 tool completion 的真实路径
- 暂未把 failed / cancelled 路径伪造成 `turn.checkpoint_persisted`
- 已把 execution checkpoint / session runtime view 的 checkpoint boundary 消费后端化到最小闭环
- 已完成 control-plane / session / 部分 runtime exact 验收
- `PA-034` 的 checkpoint lifecycle boundary 主逻辑与验收证据现已闭环
- hooks runtime 更完整的 boundary 发射面仍留在 `PA-033 / PA-022`

## 下一步

1. 推进 `PA-034` 在任务板上的最终关卡流转
2. 把主线重心切回 `PA-033`，继续 hooks runtime foundation 收口
3. 继续保持 OpenSpec / 任务系统 / 验收文档三边同步
