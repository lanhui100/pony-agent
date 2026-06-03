# Session 67 - PA-020 Permission and Failure Normalization

## 本轮目标

- 把 `PA-020` 的 permission / failure 合同从静态字段推进到 capability resolve 逻辑里

## 已完成

- 扩展 `src-tauri/src/agent/capability_bridge.rs`
  - `resolve_invocation()` 现在会先检查 source availability
  - `unreachable / disabled` source 会统一返回 `CapabilityFailureKind::SourceUnavailable`
  - `requires_approval = true` 且 `host_mediated = false` 的 capability 会统一返回 `CapabilityFailureKind::PermissionDenied`
  - source 缺失会统一返回 `CapabilityFailureKind::MalformedResponse`
- `resolve_tool_call()` 已改为内部复用统一 `resolve_invocation()` 路径
- 新增 Rust 定向测试：
  - `registry_normalizes_source_unavailable_and_permission_denied_failures`

## 验证

```powershell
cargo test registry_normalizes_source_unavailable_and_permission_denied_failures --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml --lib
```

结果：

- permission / source unavailable 归一化定向测试通过
- Rust `cargo check` 通过

## 当前结论

- `PA-020` 的 `5.1 / 5.3` 现在不再只是字段骨架，已经进入 capability resolve 合同
- 仍未闭合的部分：
  - host/runtime 对 `PermissionDenied / SourceUnavailable / MalformedResponse` 的可见输出与观测记录
  - 真实 MCP source 注册进入生产路径
