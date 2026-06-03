# Session 63 - PA-020 Capability Registry Skeleton

## 本轮目标

- 在不改 runtime loop / planner 语义的前提下，为 `PA-020` 落第一批 capability-ingress 只读骨架
- 先统一 builtin tools 的 capability registry 视图，再把 host/Tauri/front-end 的最小读链路接通

## 已完成

- 新增后端 `src-tauri/src/agent/capability_bridge.rs`
  - 定义 `CapabilitySourceView / CapabilityView / CapabilityRegistry`
  - 先把 builtin tools 规范化为 `builtin-tools` source 下的 capability records
  - 预留 `register_mcp_source / register_mcp_capability` 供后续真实 MCP 接入
- 扩展 `HostControlPlane`
  - 新增 `list_capability_sources`
  - 新增 `list_capabilities`
  - 新增 `inspect_capability`
  - 新增 `inspect_capability_source`
- 扩展 Tauri command
  - `list_capability_sources`
  - `list_capabilities`
  - `inspect_capability`
  - `inspect_capability_source`
- 扩展前端 runtime 类型与 store
  - 新增 capability source / capability 类型
  - 新增 capabilitySources / capabilities 状态
  - 新增只读拉取与 inspect 方法
  - 浏览器预览模式下补 builtin capability fallback
- 补前端定向单测
  - `tests/runtime-store.spec.ts` 新增 unified capability read-plane 验证

## 验证

```powershell
cmd /c npm run test:unit -- runtime-store.spec.ts
cargo check --manifest-path src-tauri/Cargo.toml --lib
```

结果：

- `tests/runtime-store.spec.ts` 38 项通过
- Rust `cargo check` 通过
- 仍有少量既有 warning，包括 `register_mcp_source / register_mcp_capability` 尚未在生产路径使用，以及 Windows incremental finalize `os error 5` 告警

## 结论

- `PA-020` 已从 spec-ready 进入实现态
- 当前交付的是 capability registry 的只读骨架，不包含真实 MCP 执行接入
- 下一步应把真实 MCP source 注册与 runtime 下方的 bridge action resolve 补齐，继续保持 planner 只消费 capability facts
