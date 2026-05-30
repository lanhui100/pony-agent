# PA-007 Adapter Boundary Audit

## 审核范围

- `src-tauri/src/agent/runtime.rs`
- `src-tauri/src/agent/turn_flow.rs`
- `src-tauri/src/lib.rs`
- `src/stores/runtime.ts`
- `src/types/runtime.ts`
- `src-tauri/src/bin/direct_turn_probe.rs`
- `src-tauri/src/bin/decision_probe.rs`

## 审核结论

- 当前适合启动 `PA-007`，但适合做的是“抽离 stream delivery adapter/sink 边界”，而不是直接落完整 HTTP/SSE 服务层。
- 同步 core 与 provider 决策已经能在非 Tauri 探针里独立运行，说明 `runtime / provider / tools / session` 的主体边界已经基本成立。
- 当前最强耦合点集中在流式入口和事件发射：`start_turn_stream()`、`handle_stream_tool_turn()` 和 `turn_flow.rs` 里的 `emit_* / stream_*chunks` 仍直接依赖 `AppHandle`。

## 适合立即推进的动作

1. 抽出统一的 `TurnEvent` / `TurnEventSink` 或等价回调接口。
2. 让 `runtime` 只产出事件，不再直接持有或感知 `AppHandle`。
3. 让 Tauri adapter 负责把 core 事件映射成现有前端事件名。
4. 在 sink 边界稳定后，再决定是否补最小 HTTP/SSE demo。

## 暂不建议冻结的边界

- `ProviderRegistryStore`
- `SessionStore` 的外部生命周期语义
- `provider_native_transcript`
- 当前全局 `Mutex<AgentRuntime>` 的宿主并发模型

这些部分仍更接近当前桌面端与现有 provider 协议的内部实现，不适合作为未来多接入层的稳定外部 API 直接承诺。

## 审核后优化动作

- 把 `PA-007` 从任务板 `Blocked` 调整为 `Ready`
- 在架构文档中明确“先抽 stream delivery adapter/sink，再评估 HTTP/SSE”这条路径
- 保持当前前端事件契约不动，避免 adapter 抽离同时放大 UI 联调成本
