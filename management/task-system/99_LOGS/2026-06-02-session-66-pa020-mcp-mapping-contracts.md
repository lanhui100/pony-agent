# Session 66 - PA-020 MCP Mapping Contracts

## 本轮目标

- 在没有真实 MCP server 接入的前提下，先把 `PA-020` 的 capability model / mapping / invocation contract 补到可以承接真实来源的程度

## 已完成

- 扩展 `src-tauri/src/agent/capability_bridge.rs`
  - `CapabilityView` 新增 `permission_scope`
  - 新增 `CapabilityResourceAction`
  - 新增 `CapabilityPromptTemplateAction`
  - 新增 `CapabilityBridgeAction`
  - 新增 `CapabilityInvocationRequest`
  - 新增统一 `resolve_invocation()` 合同
  - 新增：
    - `register_mcp_tool_capability()`
    - `register_mcp_resource_capability()`
    - `register_mcp_prompt_template_capability()`
  - `resolve_tool_call()` 已改为复用统一 `resolve_invocation()` 路径
- 扩展前端类型与消费
  - `src/types/runtime.ts` 的 `CapabilityView` 新增 `permissionScope`
  - `ModelMonitorPage` capability inspect 区块已展示 `permissionScope`
- 扩展测试
  - Rust：
    - `registry_resolves_mcp_tool_resource_and_prompt_template_actions`
  - 前端：
    - `runtime-store.spec.ts` 保持通过
    - `ModelMonitorPage.spec.ts` 保持通过

## 验证

```powershell
cargo test registry_resolves_mcp_tool_resource_and_prompt_template_actions --manifest-path src-tauri/Cargo.toml
cargo test registry_resolves_builtin_tool_calls_for_dotted_and_canonical_names --manifest-path src-tauri/Cargo.toml
cmd /c npm run test:unit -- runtime-store.spec.ts
cmd /c npm run test:unit -- ModelMonitorPage.spec.ts
cargo check --manifest-path src-tauri/Cargo.toml --lib
```

结果：

- 三类 capability 映射 resolve 定向测试通过
- builtin dotted/canonical resolve 回归测试通过
- `runtime-store.spec.ts` 39 项通过
- `ModelMonitorPage.spec.ts` 6 项通过
- Rust `cargo check` 通过

## 对应 OpenSpec 进展

- `2.3` 已有代码字段与测试证据
- `3.1 / 3.2 / 3.3` 已有映射函数与 resolve 测试证据
- `4.2` 已有统一 invocation contract 与 runtime builtin 复用证据

## 当前剩余主缺口

- 真实 MCP source 注册进入生产路径
- `source unavailable / permission denied / malformed response` 三类 failure 进入实际 host/runtime 合同
- planner/runtime 边界验收测试
