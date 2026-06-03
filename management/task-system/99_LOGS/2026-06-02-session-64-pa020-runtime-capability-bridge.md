# Session 64 - PA-020 Runtime Capability Bridge

## 本轮目标

- 在不改 planner 语义、不接真实 MCP 执行协议的前提下，把 `PA-020` 从 capability read-plane 骨架推进到 runtime 下方的 capability bridge 执行入口

## 已完成

- 扩展 `src-tauri/src/agent/capability_bridge.rs`
  - 新增 `CapabilityInvocationMode::as_str()`
  - 新增 `CapabilityFailureKind`
  - 新增 `CapabilityToolAction`
  - 新增 `CapabilityToolExecutionResult`
  - 新增 `CapabilityRegistry::resolve_tool_call()`
  - 新增 `CapabilityRegistry::capability_not_found_result()`
  - 支持把 dotted tool name（如 `time.now`）归一化匹配到 builtin capability（如 `builtin:time_now`）
- 扩展 `src-tauri/src/agent/runtime.rs`
  - `AgentRuntime` 新增 `capability_registry`
  - 新增 `execute_capability_tool_call()`
  - sync / stream 两条工具执行路径都改为先 resolve capability，再进入既有 `tool_executor`
  - 新增 capability 解析与 failure class 的 runtime log
- 新增 Rust 测试
  - `agent::capability_bridge::tests::registry_resolves_builtin_tool_calls_for_dotted_and_canonical_names`
  - `agent::runtime::tests::capability_bridge_resolves_dotted_builtin_tool_calls_before_execution`
  - `agent::runtime::tests::capability_bridge_returns_normalized_not_found_failure_for_unknown_tools`

## 验证

```powershell
cargo test registry_resolves_builtin_tool_calls_for_dotted_and_canonical_names --manifest-path src-tauri/Cargo.toml
cargo test capability_bridge_resolves_dotted_builtin_tool_calls_before_execution --manifest-path src-tauri/Cargo.toml
cargo test capability_bridge_returns_normalized_not_found_failure_for_unknown_tools --manifest-path src-tauri/Cargo.toml
cmd /c npm run test:unit -- runtime-store.spec.ts
```

结果：

- capability bridge registry 定向测试通过
- runtime capability bridge 两条新增定向测试通过
- 前端 `runtime-store` 定向测试保持通过：`38 passed`
- 前端 `runtime-store` 定向测试保持通过，并新增 capability read-plane fallback 覆盖：`39 passed`

## 当前结论

- `PA-020` 当前已覆盖三层：
  - capability registry 数据模型
  - host/Tauri/front-end read-plane
  - runtime 下方的 builtin capability resolve/execute 入口
- 仍未完成的主缺口：
  - 真实 MCP source 注册
  - `tool / resource / prompt_template` 三类真实映射
  - permission scope / failure normalization 完整合同
  - 前端 capability 调试 UI
