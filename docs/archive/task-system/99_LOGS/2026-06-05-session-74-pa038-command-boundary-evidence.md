# 2026-06-05 Session 74 - PA-038 Command Boundary Evidence

## 主题

- 把 `PA-038` 的下一最小实现切口从“结果态 boundary”推进到“命令边界 evidence”
- 避免 `stop_requested / run_resume` 继续隐没在 `GraphRunEvent` 的结果态里

## 本轮结论

1. `PA-038` 的下一步不该先做通用 run-hook executor
   - 当前更稳的切口是先把 `stop_requested / run_resume` 做成独立 command-boundary evidence
   - 这样可以把“发起了什么控制命令”与“graph 最终进入什么结果态”拆开

2. command-boundary evidence 已开始进入 persisted graph truth-source
   - `GraphRun / GraphRunCheckpoint` 已新增 `control_boundary_evidence`
   - `stop_graph_run(...)` 现在会记录 `stop_requested`
   - `resume_graph_run(...)` 与 `prepare_resume_graph_run_stream(...)` 现在会记录 `run_resume`
   - `GraphRunControlResponse / GraphRunStreamStartResponse` 已暴露当前命令边界 evidence

3. 当前边界仍保持收敛
   - 本轮没有让 run hooks 变成第二 arbitration source
   - `submission_plan` 仍是入口仲裁真源
   - `GraphRunEvent` 仍只表达结果态 boundary；命令证据走单独载体

## 执行命令

```powershell
cargo check --manifest-path src-tauri/Cargo.toml --tests --quiet
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::graph::tests::persistent_graph_run_store_roundtrips_checkpointable_state -- --exact --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib agent::control_plane::tests::graph_run_can_stop_resume_and_expose_checkpoint -- --exact --nocapture
```

## 验证结果

- `cargo check --tests --quiet`：通过
- `persistent_graph_run_store_roundtrips_checkpointable_state`：通过
  - 证明新增 `control_boundary_evidence` 可随 `GraphRunStore` 持久化并 reload
- `graph_run_can_stop_resume_and_expose_checkpoint`：通过
  - 证明 `stop_requested` 与 `run_resume` 已进入 graph/control-plane 的真实后端路径
- `graph_run_stream_can_start_continue_and_resume`：
  - 本轮在本机环境下长时间卡住，暂未形成可用通过/失败结论
  - 已确认此前存在多组超时遗留 `cargo` 进程占锁；清理后该用例仍未在本轮拿到稳定结果，需要下一轮单独诊断
- Windows incremental finalize `os error 5` warning 仍存在，但不影响已通过项判定

## 后续建议

- 下一步优先补 `runtime view / session control` 对 `control_boundary_evidence` 的统一消费
- 再决定是否把这批 run-level evidence 投影进更正式的 persisted trace，而不是只停留在 graph-run/read-model
- 单独排查 `graph_run_stream_can_start_continue_and_resume` 的挂起原因，避免把环境锁竞争与真实逻辑问题混淆

## 追加进展（runtime view 统一读面）

- `SessionRuntimeView` 已新增 `control_boundary_evidence`
- `load_session_runtime_view(...)` 现在会把当前 `GraphRun` 上的命令边界 evidence 一并投影给前端
- 前端 runtime store 已新增 `latestGraphRunControlBoundaryEvidence`
  - session 初始化 / 切换时会从 `SessionRuntimeView` hydrate
  - `stop_graph_run` 与 graph run stream start/resume/continue 的响应也会即时刷新这份 evidence

## 本轮补充验证

- `npx vitest run tests/runtime-store.spec.ts -t "hydrates submissionPlan from session runtime view and reuses its runId|hydrates default submissionPlan without reviving an active run id"`：通过
  - 证明前端 runtime store 已能稳定读回 `submissionPlan + controlBoundaryEvidence`
- `npx vitest run tests/HomeWorkspace.spec.ts tests/HomeSessionSidebar.spec.ts --reporter=dot`：通过
  - 证明 `HomeWorkspace / HomeSessionSidebar` 已开始展示最近一条 `control_boundary_evidence`
  - 当前命令边界证据不再只停留在 runtime view / store，而是已经进入 session control 前端反馈面
- Rust 定向测试与 `cargo check --tests`：
  - 本轮在本机编译环境下 4-5 分钟仍未跑完，被超时截断，暂未形成新的通过结论
  - 已清理残留 `cargo` 进程，下一轮需要单独处理 Rust 编译耗时 / 挂起诊断
