# 2026-05-26 Session 12 Closeout

## 本轮目标
- 收口正式 graph-run stream 宿主入口
- 统一前端主提交链路到 graph-run stream
- 保持真实 delta stream，不退回整轮结束后再一次性渲染
- 对齐 `Trace -> Run -> Turn` 的前端归组结构

## 本轮实现
- 后端新增 `start_graph_run_stream / continue_graph_run_stream / resume_graph_run_stream`
- `HostControlPlane` 新增 run-stream 预处理与执行链路：
  - `prepare_*_graph_run_stream`
  - `execute_graph_run_stream`
  - `begin_graph_run_stream`
- Tauri 宿主新增正式 run-stream 命令入口，并通过 `tauri_adapter` 执行
- 前端 `runtimeStore.submitTurn()` 统一改为：
  - 先 `inspect_host`
  - 再按当前 session 中最近非终态 run 自动选择 `start / continue / resume`
- 前端停止逻辑优先停止 `run`，只有找不到活跃 run 时才回退到 `turn`
- `Trace` 侧边栏通过 `inspect_host` 主动刷新，保证新的 `run -> turn` 归组及时出现

## 关键边界
- graph 层继续只消费完整收口后的 `TurnResult / GraphTurnHandoff`
- `start_turn_stream` 继续作为底层 turn streaming primitive，而不是主工作流入口
- 为避免改坏 runtime 主签名，control plane 通过 `RecordingTurnEventSink` 从终态事件重建 `TurnResult`

## 验证
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`
- `npx vitest run tests/runtime-store.spec.ts tests/HomeWorkspace.spec.ts tests/HomeSidebar.spec.ts tests/HomeSessionSidebar.spec.ts`
- `npx vue-tsc --noEmit --pretty false`
- `npm run build`

## 产出文件
- `src-tauri/src/agent/control_plane.rs`
- `src-tauri/src/lib.rs`
- `src/stores/runtime.ts`
- `src/components/HomeWorkspace.vue`
- `src/components/HomeSidebar.vue`
- `tests/runtime-store.spec.ts`
- `tests/HomeWorkspace.spec.ts`
- `tests/HomeSidebar.spec.ts`
- `tests/HomeSessionSidebar.spec.ts`

## 当前结果
- 主对话输入不再直接调用裸 `start_turn_stream`
- graph-run stream 已成为正式主提交入口
- `Trace -> Run -> Turn` 的前端归组已与新的执行链路对齐

## 下一步动作
- 回到 `PA-018`，继续处理 context/state subsystem 与 retrieval boundary
- 等 memory 边界稳定后，再推进 `PA-020 / PA-021`

## 断点续跑提示
- 若后续继续改提交链路，优先检查 `runtimeStore.submitTurn()`、`inspect_host` 与 `GraphRunStreamStartResponse` 三处是否仍保持一致
