# PA-007 拆分独立接入层（Tauri / HTTP-SSE adapter）

## 状态
- Status: `Done`
- Priority: `P1`
- Owner: `Codex`

## 目标
把当前主要通过 Tauri command/event 暴露的 agent runtime，进一步收束成“核心 runtime + 薄 adapter”的结构，为未来 Web 应用、Linux 服务和独立部署做准备。

## 输出
- 更明确的 adapter 边界
- Tauri adapter 的职责收敛说明
- 未来 HTTP/SSE adapter 的最小契约草案
- 可复用的 turn 事件模型

## 验收标准
- `runtime / provider / tools / session` 不再隐式依赖 Tauri UI 语义
- 能清楚区分“核心 turn 执行逻辑”和“桌面端事件转发逻辑”
- 为未来 HTTP/SSE 暴露保留稳定输入输出契约，而不是重写一套 runtime

## 当前进展
- `AgentRuntime` 已经成为当前主执行入口
- `start_turn_stream()` 已形成最小事件驱动链路
- 当前事件模型已经包含 `turn:started / turn:delta / turn:trace / turn:tool / turn:completed / turn:failed`
- `SessionStore`、`ProviderManager` 和 `ToolRouter` 都已经具备最小独立边界
- 目前真正还偏“接入层耦合”的部分，主要是 Tauri command/event 与 runtime 之间的桥接方式
- 2026-05-23 已完成一次后审计，结论是：
- 同步 core 已能脱离 Tauri 单独复用，`direct_turn_probe.rs` / `decision_probe.rs` 已证明 `run_turn()` 和 provider 决策可在非 Tauri 探针中直接运行
- 当前最强耦合点集中在 `start_turn_stream()`、`handle_stream_tool_turn()` 以及 `turn_flow.rs` 中对 `AppHandle` 的直接依赖
- 现阶段适合先抽“turn event sink / adapter”边界，不适合直接落完整 HTTP/SSE 服务层，也不适合冻结 provider registry / session lifecycle / native transcript 的外部 API 语义
- 本轮已完成最小代码重构：
- `turn_flow.rs` 已引入通用 `TurnEventSink`
- `runtime.rs` 与 `start_turn_stream()` 已改为面向 sink 发事件，不再直接接收 `AppHandle`
- Tauri 侧事件投递已收束到 `src-tauri/src/tauri_adapter.rs`
- 已新增 runtime 单测，验证空输入失败路径会通过 sink 发出 `turn:failed`
- 本轮继续完成第二 adapter 验证：
- 新增 `src-tauri/src/sse_adapter.rs`，把 `TurnStreamEvent` 映射成标准 SSE `event/id/data` 帧
- 新增 `src-tauri/src/bin/sse_turn_probe.rs`，可在非 Tauri 环境下直接验证 SSE 宿主对 core 事件流的消费
- 已补 `sse_adapter` 单测，覆盖事件格式化与 runtime 失败态输出

## 本轮验证
- `cargo test --manifest-path src-tauri/Cargo.toml --lib` 通过（41/41）
- `npm run verify` 通过
- `cargo run --manifest-path src-tauri/Cargo.toml --bin sse_turn_probe -- "当前文件夹中有哪些文件？"` 通过

## 下一步动作
- 继续梳理 provider registry、session lifecycle 与 `provider_native_transcript` 哪些仍留在桌面端语义，哪些值得进一步抽成跨宿主边界
- 继续观察真实 provider 下“大工具结果 + provider-native follow-up”对 SSE 宿主的稳定性

## 当前卡点
- 当前最小 adapter 边界已经抽出；后续真正的难点不再是 `AppHandle`，而是哪些宿主相关语义值得继续外提、哪些应继续留在桌面端实现里

## 审核记录
- `management/task-system/02_REVIEWS/2026-05-23-pa007-adapter-boundary-audit.md`

## 断点续跑提示
继续前先看：
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/turn_flow.rs`
- `docs/architecture/runtime.md`
- `docs/learning/0012-stream-runtime-and-future-architecture.md`
