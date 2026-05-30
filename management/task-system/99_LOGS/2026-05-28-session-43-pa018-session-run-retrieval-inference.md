# 2026-05-28 Session 43 PA-018 Session Run Retrieval Inference

## 本轮目标

- 继续推进 `PA-018`
- 找出宿主 retrieval 查询面里仍要求显式 `run_id` 的默认入口
- 让 retrieval 能按 `session_id` 推断当前非终态 graph run
- 让 `submitTurn()` 不再把 `inspect_host` 当成默认 run 决策兜底
- 同步回写任务文档与验证证据

## 本轮改动

- 更新：
  - `src-tauri/src/agent/control_plane.rs`
  - `src/stores/runtime.ts`
  - `tests/runtime-store.spec.ts`
  - `docs/architecture/context-state-subsystem.md`
  - `management/task-system/02_REVIEWS/2026-05-28-pa018-acceptance-audit.md`
  - `management/task-system/03_TASKS/PA-018-build-context-state-subsystem-and-retrieval-boundary.md`

## 本轮完成

- `HostControlPlane.load_retrieved_context()` 现在在只给 `session_id`、没有显式 `run_id` 时，也会：
  - 选择当前 session 最新的非终态 graph run
  - 把这条 run 的 `RunState` 一起带进 retrieval 结果
- `HostControlPlane.inspect()` 在：
  - `include_run`
  - `include_retrieved`
  场景下，也复用了同一条 session-aware run 推断逻辑
- `runtime store.submitTurn()` 现在会：
  - 先消费内存里的 `retrievedContext.runState`
  - 不足时先刷新宿主原生 `load_retrieved_context`
  - 如果刷新后的 retrieval 仍没有活跃 run，则直接新建 graph run
- 这意味着：
  - `submitTurn()` 不再把 `inspect_host` 当成默认 run 决策兜底
  - retrieval boundary 开始更像真正的 session/run 主读面，而不是“只有显式 `run_id` 才完整”的半成品查询面

## 验证

已通过：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib retrieved_context_can_infer_active_graph_run_from_session_id -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib inspection_can_infer_session_run_without_explicit_run_id -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib retrieved_context_queries_flow_through_control_plane -- --nocapture
npm exec vitest run tests/runtime-store.spec.ts
npm exec vue-tsc -- --noEmit
```

结果：

- 三条控制面定向测试通过，直接证明：
  - `load_retrieved_context(session_id)` 已能推断当前非终态 graph run
  - `inspect(session_id)` 已能在 `include_run / include_retrieved` 场景下复用同一条推断逻辑
- `runtime-store.spec.ts` 共 `28` 条通过
- `vue-tsc` 通过
- 其中新增前端用例还直接覆盖：
  - retrieval 刷新后若仍无活跃 run
  - `submitTurn()` 应直接走 `start_graph_run_stream`
  - `inspect_host` 不应再成为默认决策兜底

## 当前结果

- `PA-018` 在 `C. runtime 接入` 上又补了一条更深的 retrieval-first 证据：
  - 宿主 retrieval 查询面开始拥有 session-aware 的活跃 run 推断能力
  - 默认提交主链不再把原始 host inspection 当作 run 决策主入口
- 当前仍不能宣布 `PA-018` 完成交付，因为：
  - `HomeSidebar` / `HomeWorkspace` 的 run 历史面板在 retrieval 仍不足时仍会按需回退 `inspect_host`
  - capability / bridge 层还没有系统性收口到 retrieval 首选读面

## 下一步动作

1. 继续找出 capability / bridge 邻接层里仍默认读取原始 `session/run/checkpoint` 的公共入口
2. 继续扩展 `LongTermMemory` 的其他显式、保守、可审计稳定事实来源
3. 接近收口时，再做一轮正式 closeout audit，判断是否足以把 `PA-018` 从 `In Progress` 切到 `Done`

## 当前卡点

- 默认提交主链已经更接近 retrieval-first，但 run 历史面板和 capability / bridge 层还没有完全收口
- 现有证据更强了，但还不足以支撑最终关单
