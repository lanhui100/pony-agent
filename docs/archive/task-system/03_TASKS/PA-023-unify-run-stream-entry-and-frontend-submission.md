# PA-023 统一 run-stream 正式入口与前端主提交流

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`
- Completed At: `2026-05-26`

## 目标
在 `PA-012 / PA-013 / PA-014 / PA-015 / PA-019` 已落地的前提下，把当前“同步 graph run 入口”和“流式 turn 入口”收敛成统一的正式宿主入口：
- 前端主输入不再直接调用裸 `start_turn_stream`
- 主提交流程统一进入 graph run 的 streaming 入口
- 保持真实后端 delta stream，不退回“整轮结束后再一次性渲染”
- 保持 `Trace -> Run -> Turn` 的分层语义

## 本次完成
- 后端新增 graph run streaming 入口：
  `start_graph_run_stream / continue_graph_run_stream / resume_graph_run_stream`
- `HostControlPlane` 新增 run-stream 预处理与执行链路：
  `prepare_*_graph_run_stream`、`execute_graph_run_stream`、`begin_graph_run_stream`
- Tauri 宿主新增正式 run-stream 命令入口，并通过 `tauri_adapter` 后台执行
- 前端 `runtimeStore.submitTurn()` 统一改为：
  先 `inspect_host`，再按当前 session 中最近非终态 run 自动选择 `start / continue / resume`
- 前端停止逻辑优先停止 `run`，无法定位活动 run 时再回退到 `turn`
- `Trace` 侧边栏通过 `inspect_host` 主动刷新，保证新的 `run -> turn` 归组及时出现

## 关键实现点
- `run` 先进入 `running + active_turn_id`，再启动真实 turn stream
- graph 层仍只消费完整收口后的 `TurnResult / GraphTurnHandoff`
- 为避免改坏 runtime 主签名，control plane 通过 `RecordingTurnEventSink` 从终态事件重建 `TurnResult`
- 修复了 streamed turn 终态与 checkpoint 短暂不一致时，graph 被错误保留在 `Running` 的问题
- 为 control plane 补上本地 mock provider 测试夹具，确保 run-stream 回归测试不依赖真实网络

## 验收结果
- 主对话输入不再直接调用裸 `start_turn_stream`
- 首次输入可创建 run 并立即启动当前 turn 的真实后端 stream
- 后续输入可继续或恢复当前 session 下最近的非终态 run
- `stop_graph_run` 与 `stop_turn` 职责分层保持明确
- 回归测试已覆盖图片附件提交、run-stream 提交链路、`Trace -> Run -> Turn` 结构与 control plane run-stream 执行

## 验证
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`
- `npx vitest run tests/runtime-store.spec.ts tests/HomeWorkspace.spec.ts tests/HomeSidebar.spec.ts tests/HomeSessionSidebar.spec.ts`
- `npx vue-tsc --noEmit --pretty false`
- `npm run build`

## 关联
- `PA-012`
- `PA-013`
- `PA-014`
- `PA-015`
- `PA-019`
