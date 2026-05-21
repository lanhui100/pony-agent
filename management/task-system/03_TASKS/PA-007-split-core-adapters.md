# PA-007 拆分独立接入层（Tauri / HTTP-SSE adapter）

## 状态
- Status: `Ready`
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

## 下一步动作
- 画清楚 `core runtime`、`Tauri adapter`、`future HTTP/SSE adapter` 的职责分界
- 提炼复用同一套 turn 事件模型所需的最小接口
- 明确哪些数据应该是 runtime 原生产物，哪些只是某个 adapter 的表现形式
- 先从文档与边界重构入手，再决定是否落最小 HTTP/SSE demo

## 当前卡点
- 现在代码已经可用，重构时要避免把“未来接口想象”反向压坏当前可联调链路
- 若过早落地完整 HTTP 服务层，容易和仍在演进的 session/history 策略互相牵扯

## 断点续跑提示
继续前先看：
- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/turn_flow.rs`
- `docs/architecture/runtime.md`
- `docs/learning/0012-stream-runtime-and-future-architecture.md`
