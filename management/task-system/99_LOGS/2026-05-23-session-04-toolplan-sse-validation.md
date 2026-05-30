# 2026-05-23 Session 04 - ToolPlan / SSE Adapter / Live Validation

## 本轮目标

- 把 `PA-008` 的组合工具计划从隐式 nested result 推进到显式 `ToolPlan`
- 为 `PA-007` 补第二 adapter 验证，而不是直接上完整 HTTP/SSE 服务层
- 完成一轮真实 provider 的长链路人工回归，并把结论写回任务系统

## 已完成

- 在 `src-tauri/src/agent/tools.rs` 中新增 `ToolPlan / ToolPlanStep`
- `workspace_batch / workspace_gather_context` 结果中已显式输出 `plan`
- 在 `src-tauri/src/agent/planner.rs` 中，显式多路径批量请求已写入 `toolPlan`
- 在 `src-tauri/src/agent/telemetry.rs` 中，tool activity 已优先消费显式 `toolPlan`
- 在 `src-tauri/src/agent/config.rs` 与 `src/stores/providers.ts` 中引入集中式 capability catalog
- 新增 `src-tauri/src/sse_adapter.rs`
- 新增 `src-tauri/src/bin/sse_turn_probe.rs`
- 对 OpenAI 兼容 reasoning 模型补了“stream follow-up 失败时回退 sync follow-up”的保守兜底

## 验证结果

- `cargo test --manifest-path src-tauri/Cargo.toml --lib` 通过（41/41）
- `cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check` 通过
- `npm run verify` 通过
- `cargo run --manifest-path src-tauri/Cargo.toml --bin direct_turn_probe -- followup-fallback`
  - 真实 provider 两轮均走 `provider_mode=live`
  - 未触发 fallback
- `cargo run --manifest-path src-tauri/Cargo.toml --bin sse_turn_probe -- "当前文件夹中有哪些文件？"`
  - 成功输出 `turn:started -> turn:tool -> turn:delta -> turn:completed`
  - 证明第二 adapter 可消费同一套 core 事件模型

## 新发现

- 真实 provider 在“大体积工具结果”的 follow-up 阶段仍可能返回 400，例如 provider 侧报 `No tool output found for function call ...`
- 该问题更像 provider 兼容性 / 大结果承载问题，而不是 adapter 边界问题
- 当前已用“stream follow-up 失败后回退 sync follow-up”降低风险，但后续仍需要考虑：
- 对大工具结果做摘要/裁剪
- 区分“工具结果过大”与“adapter/transport 问题”

## 影响文件

- `src-tauri/src/agent/tools.rs`
- `src-tauri/src/agent/planner.rs`
- `src-tauri/src/agent/telemetry.rs`
- `src-tauri/src/agent/provider.rs`
- `src-tauri/src/agent/config.rs`
- `src-tauri/src/sse_adapter.rs`
- `src-tauri/src/bin/sse_turn_probe.rs`
- `src/stores/providers.ts`
- `docs/architecture/runtime.md`

## 下一步建议

1. 继续收敛 `ToolPlan` 是否要上提成 runtime 级通用计划对象
2. 针对大工具结果补摘要/压缩策略，减少 provider-native follow-up 失败率
3. 若继续看第二 adapter，优先保持“同一套 core 事件模型 + 薄宿主映射”原则，不要过早长出完整 HTTP 服务层
## 2026-05-23 二次收口补记
- `ToolPlan` 已进一步从 `planner.rs` 里塞进 `arguments.toolPlan` 的过渡方案，收束为 `ToolCall.plan` 一等字段。
- `LocalTurnPlanner` + `runtime` 已支持“显式多路径计划优先于 provider 单路径工具决策”，因此 `direct_turn_probe -- multipath-context` 已能稳定命中 `workspace_batch`。
- OpenAI follow-up 现在除了“大结果压缩”外，又补了“sync / stream follow-up 失败时返回本地兜底整合结果”的保护，因此：
- `direct_turn_probe -- large-result` 可在 `provider_mode=fallback` 下完成
- `sse_turn_probe -- adapter-large-result --raw` 可在 `provider_mode=fallback` 下输出 `turn:completed`
- 本轮最终验证结果：
- `cargo test --manifest-path src-tauri/Cargo.toml`
- `npm run verify`
- `cargo run --manifest-path src-tauri/Cargo.toml --bin direct_turn_probe -- multipath-context`
- `cargo run --manifest-path src-tauri/Cargo.toml --bin direct_turn_probe -- large-result`
- `cargo run --manifest-path src-tauri/Cargo.toml --bin sse_turn_probe -- adapter-multipath`
- `cargo run --manifest-path src-tauri/Cargo.toml --bin sse_turn_probe -- adapter-large-result --raw`
