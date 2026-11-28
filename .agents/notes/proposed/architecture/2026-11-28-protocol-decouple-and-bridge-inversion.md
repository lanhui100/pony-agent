# Agent Note: L0 协议层净化与全局 OnceLock 桥接显式 DI 反转

Status: proposed

## Problem

在 [2026-11-25-build-test-layering-and-workspace-crate-split.md](../../implemented/architecture/2026-11-25-build-test-layering-and-workspace-crate-split.md) 完成 6 层 10-crate 拆分后，名义上作为纯契约/类型底座的 L0 `pony-protocol` 仍残留大量运行时、全局可变状态与反向服务定位器：

1. **L0 携带运行态与全局业务状态**：
   - `crates/pony-protocol/src/runtime_helper.rs:8,25` 持有两处全局 `OnceLock<tokio::runtime::Runtime>`，迫使 L0 依赖 `tokio(rt-multi-thread)` 与 `dirs`；
   - `crates/pony-protocol/src/todo.rs:58` 持有全局 `TODO_STORE: Mutex<Option<HashMap<String, Vec<TodoItem>>>>`；
   - `crates/pony-protocol/src/cost.rs:469` 持有全局 `BUDGET_USD_BITS: AtomicU64`。
2. **8 处全局 `OnceLock` 反向服务定位器造成隐式耦合**：
   - `crates/pony-protocol/src/host_hooks.rs:6-7`（`GIT_IDENTITY_ENV_PROVIDER`, `SUBAGENT_WIRED_PROBE`）；
   - `crates/pony-protocol/src/turn_flow_registry.rs:12,137,172,230,233,318`（6 个 `OnceLock<Mutex<...>>` 回调注册表）；
   - `crates/pony-tools/src/tools/mod.rs:597`（`ORCHESTRATION_BRIDGE: OnceLock<Arc<dyn OrchestrationToolBridge>>`）。
   - 以上桥接仅依赖 `crates/pony-agent-core/src/agent/mod.rs:111-127` 的 `ensure_runtime_bridges()` 在 4 个特定入口惰性注册。任何绕过这 4 个入口的调用方或单元测试都会静默退化为空实现。
3. **关联全局污染与分配器绑定**：`crates/pony-agent-core/src/lib.rs:10-12` 在库 crate 中定义 `#[global_allocator] MiMalloc`，`crates/pony-agent-core/src/agent/orchestration.rs:174-720` 仍持有 12 个进程级 `static Mutex`。

## Proposal

建议通过以下四步完成 L0 净化与依赖反转（需人工裁决是否采用显式 DI 方案 A）：

1. **剥离 L0 运行时与状态**：将 `runtime_helper.rs` 移入运行时基础设施层（`pony-agent-core` 或各宿主二进制），将 `TODO_STORE` 与 `BUDGET_USD_BITS` 收敛为 `pony-storage` / `SessionStore` 或显式状态句柄，从 `crates/pony-protocol/Cargo.toml` 移除 `tokio` 运行时特性与 `dirs`。
2. **以显式 Trait 参数注入取代 8 处全局 `OnceLock`**：
   - 将 `host_hooks`、`turn_flow_registry`、`OrchestrationToolBridge` 定义为显式上下文/建造器 trait（如 `TurnEventSink` / `HostHookProvider` / `OrchestrationPort`），经 `AgentRuntimeBuilder`、`HostControlPlaneBuilder` 与 `ToolRouter` 构造期传入；
   - 同步移除 `crates/pony-agent-core/src/agent/mod.rs:111-127` `ensure_runtime_bridges()` 与 `crates/pony-agent-core/src/agent/tools/mod.rs` 的桥接薄包装壳。
3. **移除库级 `#[global_allocator]`**：将 `MiMalloc` 声明从 `pony-agent-core/src/lib.rs` 下移至各自二进制入口（`src-tauri/src/main.rs`、`pony-tui/src/main.rs`、`pony-cli/src/main.rs`）。
4. **并行性收口**：同步将 `orchestration.rs` 的 12 个全局 `static Mutex` 逐步实例化到 `AgentRuntime` / `HostControlPlane` 作用域，为多会话并发与无进程隔离单元测试扫清障碍。

## Alternatives considered

- **方案 A：显式构造期 DI Trait 参数注入（推荐）**。彻底消除全局服务定位器与静默退化风险，L0 `pony-protocol` 回归纯数据结构与 trait 定义，并可独立做无全局状态并发单测。代价是需修改约 20+ 处构造/测试签名，必须按「L0 剥离 → trait 注入 → 删桥接」分批推进。
- **方案 B：保留全局 `OnceLock` 注册表，仅把未注册行为改为 panic（fail-closed）**。改动小，能暴露漏调用 `ensure_runtime_bridges()` 的路径；但落选，因为仍无法解决多会话并发与单元测试并行隔离（全局单例相互污染）的根本架构缺陷。
- **方案 C：回退部分 crate 合并回 `pony-agent-core`**。彻底消除跨 crate 桥接问题；但落选，会摧毁 10-crate 拆分带来的 ≤12s 叶子 crate 增量编译收益（见上游 ADR）。

## Acceptance criteria

1. `crates/pony-protocol/Cargo.toml` 不再依赖 `tokio`（仅保留纯同步类型/协议依赖，或按需最小特性）与 `dirs`，源码内零 `OnceLock<Runtime>`、`TODO_STORE`、`BUDGET_USD_BITS`、`host_hooks`、`turn_flow_registry` 全局可变静态量。
2. `crates/pony-tools/src/tools/mod.rs` 移除 `ORCHESTRATION_BRIDGE` 全局静态量；`crates/pony-agent-core/src/agent/mod.rs` 移除 `ensure_runtime_bridges()`。
3. `#[global_allocator]` 仅存在于二进制 crate，不在 `pony-agent-core` 库中定义。
4. 全 workspace `npm run cargo:check:shared`、`npm run test:l3` 与 `nextest` 并发运行全部通过，无需靠进程级隔离掩盖全局状态互染。

## Risks

- 构造器签名变更面较广（涉及 `runtime/builder.rs`、`subagent_runtime.rs`、`tools` 与集成测试夹具），未按批次切分易造成大面积合并冲突。
- 在移除 `OnceLock` 桥接过渡期内，若测试夹具注入不全，可能暴露此前被 `host_hooks` 默认分支掩盖的隐藏调用链。
