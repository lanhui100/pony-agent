# Tasks: Add Trace Persistence and Recovery Contract

## 1. Spec and Task-System Sync

- [x] 1.1 新建 `PA-032` 任务卡并同步 dashboard / task board 状态
- [x] 1.2 完成 `add-trace-persistence-and-recovery-contract` 的 proposal / design / spec 文档

## 2. Contract Definition

- [x] 2.1 明确 trace persistence source-of-truth 边界
- [x] 2.2 明确 runtime checkpoint 与 recovery checkpoint 的区分
- [x] 2.3 明确 history checkout / workspace rollback 的结果与降级口径

## 3. Implementation and Verification

- [x] 3.1 在后端读写面与前端消费面收第一版 contract
- [x] 3.2 为重启恢复、trace roundtrip 与 degrade path 增补定向测试
- [x] 3.3 回写任务卡、日志与验收证据
- [x] 3.4 补齐 replay / resume 执行入口对 recovery contract 的最终仲裁
  - 已完成：后端已新增 `submission_plan_starts_fresh_run_when_recovery_contract_requires_replay`
  - 已完成：后端已新增 `submission_plan_switches_with_session_checkpoint_boundary`
  - 已完成：通过独立 `CARGO_TARGET_DIR` 运行 exact Rust 单测，拿到稳定逻辑执行结论
- [x] 3.5 完成 session 级 checkpoint 切换、执行入口仲裁与 hydration 验收的最终收口记录
  - 已完成：`load_session_runtime_view(...)` 暴露 `submissionPlan`，前端 hydration 消费 `submissionPlan`，reload 后 completed/lifecycle-boundary session 默认回到 `start_graph_run_stream`
  - 已完成：session 级 checkpoint 切换、submission plan 仲裁与 reload/hydration 证据已回写任务卡与会话日志
