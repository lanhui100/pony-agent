# Tasks: Add Checkpoint Lifecycle Boundary Implementation

## 1. Spec and Task-System Sync

- [x] 1.1 新建 `PA-034` 任务卡并同步到当前任务系统
- [x] 1.2 完成 `add-checkpoint-lifecycle-boundary-implementation` 的 proposal / design / spec 文档
- [x] 1.3 完成独立 spec 审核并采纳必要修订

## 2. Runtime Boundary Implementation

- [x] 2.1 在 runtime 正常完成链路补 `checkpointing` lifecycle boundary 发射
- [x] 2.2 让 persisted trace / session snapshot 保留 checkpoint lifecycle evidence
- [x] 2.3 让 execution checkpoint / session runtime view 能消费这组 evidence

## 3. Verification and Closeout

- [x] 3.1 为 normal completion / tool completion / reload roundtrip 补后端测试
  - 已完成：runtime completion 顺序断言、session file-backend roundtrip、control-plane file-backend reload projection
  - 已完成：`completed_session_can_project_checkpoint_lifecycle_boundary_without_recovery` exact
  - 已完成：`lifecycle_boundary_checkpoint_does_not_override_default_submission_plan` exact
  - 已完成：`file_backed_reload_restores_lifecycle_boundary_projection` exact
  - 已完成：`file_backend_roundtrip_restores_checkpoint_persist_evidence` exact
  - 已完成：`start_turn_stream_emits_first_token_latency_on_reasoning_delta` exact
  - 已完成：`start_turn_stream_completes_after_multi_hop_followup_stream` exact
  - 备注：本轮同时修正了过期测试合同，确保 OpenAI native tool flow 的 initial decision mock 与当前流式 runtime 行为一致
- [x] 3.2 为前端 hydration / runtime store 补回归测试
- [x] 3.3 回写任务卡、review 文档、日志与验收证据
