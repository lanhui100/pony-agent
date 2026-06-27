# Rust 智能体开发指南

## 项目结构

```
pony-agent/
├── crates/pony-agent-core/   # 智能体核心库（不依赖 Tauri）
│   └── src/agent/            # 22 个模块
├── src-tauri/                # Tauri 桌面适配器（依赖 pony-agent-core）
│   └── tests/                # 回归测试
└── src/                      # Vue 3 前端
```

### Workspace 成员

- `crates/pony-agent-core` — 核心库，包名 `pony_agent_core`，不含 Tauri 依赖
- `src-tauri` — Tauri 桌面适配器，包名 `pony_agent`，通过 path 依赖引入 core

## 模块架构（22 模块）

### 层依赖方向

`control_plane` → `runtime` / `graph` → `session` / `context` / `provider` / `tools` → `telemetry` / `hooks` / `config`

| 模块 | 职责 |
|------|------|
| `app_settings` | 应用级设置（workspace mode 等），JSON 文件存储 |
| `capability_bridge` | 能力注册中心：内置工具 / MCP / skill 的统一视图与调用路由 |
| `config` | Provider 注册表（providers.json）、模型能力预设、选择器 |
| `context` | 上下文构建与检索：TurnContext / SessionContext / RunState / LongTermMemory |
| `control_plane` | 宿主控制面：统一 `turn / run / session / checkpoint / diagnostics` 命令 |
| `execution_control` | Turn 级执行控制：checkpoint 注册、stop 命令、协作式取消 |
| `frontend_diagnostics` | 前端诊断数据收集：SQLite 跟踪事件存储与查询 |
| `graph` | Graph Run 编排：run 状态机、checkpoint、persist、stop/resume |
| `hooks` | 生命周期钩子管线：turn 钩子、run 钩子、memory write 钩子、patch 系统 |
| `input` | 用户输入数据结构（TurnInputImage） |
| `planner` | 决策规划：TurnPlanner（单轮预检/工具选择）、GraphPlanner（跨轮继续策略） |
| `provider` | Provider 协议抽象：OpenAI/Anthropic 兼容、请求构建、流式解析、重试 |
| `retry` | Provider 重试策略：退避算法、预算、升降级逻辑 |
| `runtime` | AgentRuntime / AgentRuntimeBuilder：单轮执行循环、流式输出、状态管理 |
| `runtime/turn_runner` | 实际 turn 执行骨架：hook 调度、上下文构造、模型调用、工具跟进 |
| `runtime_helper` | Tokio 运行时辅助：`block_on` 与 `TestRuntimeGuard` |
| `secret_store` | 密钥存储抽象：系统密钥链 / 文件 / Composite 策略 |
| `session` | 会话存储：SessionStore、Attachment 生命周期、History 分支与回溯 |
| `sqlite_session` | SQLite 后端实现：持久化会话、附件资产、JSON 回迁 |
| `sse_adapter` | SSE 格式序列化：将 TurnStreamEvent 转 BufferingSseTurnEventSink |
| `telemetry` | 遥测数据结构：TurnTraceStep、ToolActivity、CapabilityInvocationRecord |
| `tools` | 工具系统：定义、执行器、路由器（内置 18+ 工具） |

### 模块间依赖关系

```
control_plane
  ├── runtime         → execution_control, context, provider, tools, session, hooks, telemetry, turn_flow, planner
  │   └── turn_runner → hooks, capability_bridge, context, provider, tools, planner, session
  ├── graph           → planner, context, hooks, runtime (TurnResult)
  └── frontend_diagnostics

context    → session, provider, input, graph, capability_bridge, execution_control
provider   → config, input, tools, retry, runtime_helper
tools      → runtime_helper
hooks      → (独立，被 runtime/graph 消费)
session    → capability_bridge, hooks, input, provider, telemetry
```

## 关键 Trait 与抽象

### SessionStore / SessionBackend

```rust
// session.rs
pub trait SessionBackend: Send + Sync {
    fn load_store(&self) -> Result<PersistedStore, String>;
    fn save_store(&self, store: &PersistedStore) -> Result<(), String>;
    // + attachment 资产 CRUD
}

pub struct SessionStore { ... }  // 封装 backend，提供会话级操作接口
```

两个后端：`FileSessionBackend`（JSON 文件）和 `SqliteSessionBackend`（SQLite WAL 模式）。

### AgentRuntime / HostControlPlane

`AgentRuntime` 在 `runtime/mod.rs` 中定义，通过 Builder 模式构造，负责单轮执行核心：
- `start_turn_stream()` / `stop_turn()` — 流式执行 / 取消
- 消费 `planner`、`provider`、`tool_executor`、`session_store`
- 输出 `TurnStreamEvent`

`HostControlPlane` 在 `control_plane.rs` 中定义，作为宿主统一入口：
- `run_turn` / `start_turn_stream` / `stop_turn`
- `start_graph_run` / `continue_graph_run` / `stop_graph_run` / `resume_graph_run`
- `session_snapshot` / `list_sessions` / `remove_session`
- `frontend_trace_query` / `frontend_diagnostics_path`
- 使用 `HostControlPlaneBuilder` 构造，支持非 Tauri 宿主注入

### Provider 协议抽象

```rust
// provider.rs
pub enum ProviderProtocol { OpenAi, Anthropic }
pub struct ProviderManager { ... }     // 执行实际 API 调用
pub trait ProviderSelectionResolver {  // 选择 provider 配置
    fn resolve_provider_selection(...) -> ResolvedProviderSelection;
}
```

Provider 负责：请求构建、base64 图片注入、流式 chunk 解析、重试/退避。

### 工具系统

```rust
// tools.rs
pub trait ToolExecutor { fn execute(&self, call: &ToolCall) -> ToolResult; }
pub struct ToolRouter { ... } // 路由到具体实现
pub struct ToolCall { name, arguments, ... }
pub struct ToolResult { status, output, ... }
```

内置工具约定使用 `tool_` 前缀常量（如 `TOOL_WORKSPACE_READ_FILE`），`ToolKind` 枚举分类（Read/Search/Write/Execute/Plan/Interactive/Composite/External）。

### Hooks 管线

```rust
// hooks.rs
pub enum TurnHookPoint { TurnPrepareStart, ContextBuildStart, ModelCallStart, ... }
pub enum CanonicalTurnEventType { TurnCreated, TurnPhaseChanged, ... }
pub trait AgentHookExecutor { fn execute(&self, descriptor, hook_point) -> ... }
pub struct AgentHookRegistry { ... }
```

生命周期事件分为 turn 级（`CanonicalTurnEventType` / `TurnHookPoint`）、run 级（`CanonicalGraphRunEventType` / `RunHookPoint`）、memory write 级。支持 patch 操作（`HookPatchOperation`）和冲突策略（`HookPatchConflictPolicy`）。

### Context / Retrieval 子系统

```rust
// context.rs
pub struct TurnContext { user_message, images, ... }
pub struct SessionContext { conversation_id, recent_history, ... }
pub struct RunState { run_id, goal, phase, ... }
pub struct RetrievedContextState { turn, session, run, long_term_memory, ... }
```

分层定义：TurnContext（当前轮）→ SessionContext（当前会话）→ RunState（当前运行）→ LongTermMemory（跨会话记忆）。`DefaultTurnContextBuilder` 执行实际的 context 组装。

### Graph / Planner

```rust
// graph.rs
pub struct GraphRun { run_id, goal, phase(Vec<GraphStep>), ... }
pub struct GraphRunCheckpoint { ... }
pub trait GraphEngine { ... }          // start/continue/stop/resume
pub struct GraphRunner { ... }         // 实际编排

// planner.rs
pub trait TurnPlanner: Send + Sync {   // 单轮决策
    fn preflight_decision(...);
    fn select_tool_call(...);
}
pub trait GraphPlanner: Send + Sync {  // 跨轮决策
    fn decide_after_turn(...) -> GraphDecision;
}
pub struct DefaultGraphPlanner { ... } // 默认策略：ask user / auto-continue
```

### 其他重要 Trait

| Trait | 位置 | 职责 |
|-------|------|------|
| `SecretStore` | `secret_store.rs` | `get/set/delete` 密钥，支持系统密钥链 + 文件 fallback |
| `TurnEventSink` | `turn_flow.rs` | `emit(name, payload)` 流式事件输出 |
| `TurnContextBuilder` | `context.rs` | `build_turn_context()` 组装上下文的策略接口 |
| `TurnTelemetryBuilder` | `telemetry.rs` | telemetry 数据收集策略接口 |
| `ToolExecutor` | `tools.rs` | `execute(&ToolCall) -> ToolResult` |

## 开发工作流

### 构建命令

| 命令 | 用途 | target 目录 |
|------|------|-------------|
| `npm run cargo:check:shared` | 类型检查（所有 workspace member） | `target-check/` |
| `npm run cargo:test:shared` | 运行所有测试 | `target-test/` |
| `npm run cargo:test:exact -- --lib <name>` | 精确运行单测 | `target-test-exact-a/` |
| `npm run dev:tauri` | Tauri 开发模式 | `target/` |
| `npm run verify` | test:unit + build + cargo:check:shared | 综合验证 |

> **必须通过 npm script 运行 cargo**，禁止直接 `cargo check/test` 以免污染 `target/` 构建缓存。

### 清理

- `npm run clean:tauri:light` — 清理 check/test target
- `npm run clean:tauri:deep` — 额外清理 `target/`（全量重编译）

### 添加新模块

1. 在 `crates/pony-agent-core/src/agent/` 下创建 `your_module.rs`
2. 在 `crates/pony-agent-core/src/agent/mod.rs` 中添加 `pub mod your_module;`
3. 遵循依赖方向：下层模块不引用上层模块
4. 如果涉及新的 trait，优先在模块内部定义默认实现

## 编码规约

### 错误处理

- 模块内部使用 `Result<_, String>` 传递错误，避免引入自定义错误类型
- 调用 `runtime_helper::block_on()` 包裹 async 调用（provider、tool 中的网络调用）
- 关键检查点记录 `runtime_log()`（用于调试跟踪）
- provider 重试：`ProviderRetryPolicy` + `RetryBudget` + `compute_delay()` 退避

### 测试模式

- 单元测试使用 `#[cfg(test)] mod tests { ... }` 内联在模块文件底部
- 回归测试放在 `src-tauri/tests/` 下，作为独立 `#[test]` 文件
- 使用临时目录（`std::env::temp_dir()` + 时间戳）避免测试间污染
- async 测试用 `TestRuntimeGuard` 提供 tokio 上下文
- 环境变量敏感的测试用 `env_lock()` + `lock_env()` 序列化

```rust
// 单元测试模式
#[cfg(test)]
mod tests {
    #[test]
    fn test_something() {
        // arrange
        // act
        // assert
    }
}

// 回归测试模式
#[test]
fn feature_regression_test() {
    let temp = temp_something();
    // 建立隔离环境
    // 执行操作
    // 断言结果
    let _ = fs::remove_dir_all(temp);
}
```

### 文档标准

- 公共 API 使用 `///` doc comment 说明职责
- 每个模块在文件头用 `//!` doc comment 描述模块职责
- 复杂领域逻辑在 `docs/architecture/` 中归档
- 架构决策记录（ADR）在 `openspec/` 中维护

## 测试策略

### Rust 测试分类

| 类型 | 位置 | 命令 |
|------|------|------|
| 单元测试 | 各模块 `mod tests` | `npm run cargo:test:shared` |
| 集成测试 | `src-tauri/tests/` | `npm run cargo:test:regression` |
| 非 Tauri 集成测试 | `crates/pony-agent-core/src/bin/non_tauri_harness.rs` | 在 CI 中作为 binary 运行 |

### 回归测试（3 个）

- `session_regression.rs` — SessionStore 持久化生命周期、历史管理、附件资产 cleanup
- `provider_registry_regression.rs` — ProviderRegistryStore 保存/加载、环境变量集成、配置规范化
- `tool_router_regression.rs` — ToolRouter 各工具边界、文件搜索/读/写/编辑/batch

### 前端测试

- `npm run test:unit` — Vitest 单元测试
- `npm run test:e2e` — Playwright E2E 测试
- `npm run test:ui-guard` — UI 组件 guard 测试

### 系统测试

- `npm run verify` — 前端测试 + 构建 + Rust 类型检查的组合验证
- Tauri smoke 测试：`npm run test:tauri:smoke`

## 关键设计原则

1. **依赖方向**：`control_plane → runtime → execution_control / context / provider / tools / session → telemetry / hooks`
2. **分层清晰**：core 不依赖 Tauri，`src-tauri` 只是桌面适配器
3. **最小闭环**：先做单 turn，再做 graph 多 turn，再做 hooks 横切
4. **状态机优先**：所有执行阶段用枚举表达（`GraphRunPhase`、`CanonicalTurnPhase`）
5. **File → SQLite**：JSON 文件作为早期后端，SQLite 作为稳定后端，支持自动回迁
6. **host 可注入**：`AgentRuntimeBuilder` / `HostControlPlaneBuilder` 支持 builder 模式，非 Tauri 宿主（`non_tauri_harness`）可复用全套 core
