# PA-044 Acceptance Audit

## 审核对象

- [PA-044 任务卡](</C:/Users/HUAWEI/Documents/pony-agent/management/task-system/03_TASKS/PA-044-harden-agent-core-infrastructure-boundary.md>)
- [OpenSpec change](</C:/Users/HUAWEI/Documents/pony-agent/openspec/changes/harden-agent-core-infrastructure-boundary>)
- [agent core crate](</C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core>)
- [Tauri adapter crate](</C:/Users/HUAWEI/Documents/pony-agent/src-tauri>)
- [runtime architecture doc](</C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/runtime.md>)
- [architecture overview](</C:/Users/HUAWEI/Documents/pony-agent/docs/architecture/overview.md>)

## 结论

`PA-044` 已完成并可交付。

本轮已把 agent core 从 `src-tauri` 所有权边界中拆出，建立独立 `pony-agent-core` crate；Tauri 现在作为 desktop adapter 依赖并 re-export core。核心构造入口、host preset、workspace/storage 注入与 non-Tauri harness 都已落地，能够证明 Pony Agent 的 agent core 没有继续偏向只为 Tauri 桌面端构建。

## 验收项

### 1. Core package boundary

- 已新增 workspace root `Cargo.toml`
- 已新增 `crates/pony-agent-core/Cargo.toml`
- `agent` 模块树已迁入 `crates/pony-agent-core/src/agent`
- `src-tauri` 通过 `pony-agent-core = { path = "../crates/pony-agent-core" }` 依赖 core
- `src-tauri/src/lib.rs` re-export `pony_agent_core::agent`

### 2. Construction boundary

- 已实现 `AgentRuntimeBuilder`
- 已实现 `HostControlPlaneBuilder`
- 已实现 `DesktopRuntimePreset`
- 已实现 `DesktopHostPreset`
- `AgentRuntime::with_dependencies` 已从 crate-only 入口改为公开入口
- `HostControlPlane::with_runtime` 已从 test-only 入口改为公开入口
- `ToolRouter::with_workspace_root` 已从 test-only 入口改为公开入口
- `GraphRunStore::new` 已从 test-only 入口改为公开入口

### 3. Host preset and adapter boundary

- desktop 默认构造继续保留在 `DesktopRuntimePreset / DesktopHostPreset`
- Tauri command/event contract 未改写
- `tauri_adapter.rs` 仍只负责 Tauri event delivery
- `sse_adapter.rs` 改为依赖 `pony_agent_core::agent` 类型，不再依赖 Tauri crate 内部 `crate::agent`
- 旧 probe 已改为通过 `pony_agent_core::agent` 使用 core 类型

### 4. Second host proof

已新增：

- [non_tauri_harness.rs](</C:/Users/HUAWEI/Documents/pony-agent/crates/pony-agent-core/src/bin/non_tauri_harness.rs>)

覆盖：

- builder 显式构造 core
- injected `ProviderSelectionResolver`
- injected `SessionStore`
- injected `GraphRunStore`
- injected workspace root
- sync turn
- stream turn
- graph run checkpoint persistence
- workspace tool execution
- non-Tauri `TurnEventSink`

## 验证证据

### Core package does not contain Tauri-only tokens

```powershell
Get-ChildItem -Recurse crates\pony-agent-core -File | Select-String -Pattern 'tauri::|AppHandle|State<|Emitter|use tauri|tauri =|tauri-build'
```

结果：无命中。

### Core-only check

```powershell
cargo check -p pony-agent-core --target-dir target-check-core-final
```

结果：通过。

### Core-only tests

```powershell
cargo test -p pony-agent-core --lib --target-dir target-test-pa044-core -- --test-threads=1
```

结果：通过，304 passed。

### Tauri desktop adapter check

```powershell
cargo check --manifest-path src-tauri/Cargo.toml --target-dir target-check-pa044
```

结果：通过。

### Tauri desktop adapter tests

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --target-dir target-check-pa044
```

结果：通过，包含 `sse_adapter`、probe binaries、`provider_registry_regression`、`session_regression`、`tool_router_regression`。

说明：Windows 上多次出现 incremental compilation session directory `拒绝访问 (os error 5)` 警告，但命令最终退出码为 0，测试结果通过。

### Frontend command/event contract regression

```powershell
npm run test:unit -- --run tests/runtime-store.spec.ts
```

结果：通过，67 passed。测试期间输出了预期的 cache telemetry stderr 诊断，不影响断言结果。

### Non-Tauri harness smoke

```powershell
cargo run -p pony-agent-core --bin non_tauri_harness --target-dir target-pa044-harness
```

结果：通过，输出 `non_tauri_harness ok`。

说明：该命令同样可能在 Windows 上输出 incremental compilation `拒绝访问 (os error 5)` 警告；harness 进程输出 `non_tauri_harness ok`，退出码为 0。

### OpenSpec validation

```powershell
npm run openspec -- validate harden-agent-core-infrastructure-boundary --type change --strict --no-interactive
```

结果：通过。

### OpenSpec status

```powershell
npm run openspec -- status --change harden-agent-core-infrastructure-boundary --json
```

结果：`isComplete: true`，proposal / design / specs / tasks 均为 `done`。

## 残余风险

- provider transport 仍保留 blocking reqwest 实现；本卡已将其收口为 provider transport implementation，并通过 builder/preset seam 为后续 service host 替换保留接口空间。
- 当前 `non_tauri_harness` 是 smoke proof，不是完整 HTTP/SSE server；这符合 PA-044 范围，后续如要落正式 HTTP/SSE host，应新开任务承接。

## 最终判断

验收通过。`PA-044` 可标记为完成，并可在后续归档时同步 canonical spec。
