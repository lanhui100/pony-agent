# 结构性技术债提案（2026-11-01 架构评估产出）

> 定位：本页只登记**已机器复核存在、但本轮不实施**的结构性问题，给出优先级与拆分轴。
> 本轮只修可闭包的 P0/P1 缺陷；结构性重构属独立立项，需按 `dev-team` 波次单独开单，禁止夹带在缺陷修复提交里。

## 1. 上帝模块拆分（P1，建议单独立项）

| 对象 | 规模 | 拆分轴建议 |
| --- | --- | --- |
| `crates/pony-agent-core/src/agent/tools.rs` | 13589 行；`ToolRouter` impl 约 L1286–5239 | `registry/`（描述符单一源）+ `catalog/`（lookup）+ `handlers/{workspace,web,process,agents}` |
| `crates/pony-agent-core/src/agent/control_plane/mod.rs` | 9962 行（L3201 之后为测试） | 按限界上下文：`TurnService` / `GraphService` / `HistoryService` / `CapabilityService` / `AskPlanService` + 外层 facade |
| `crates/pony-agent-core/src/agent/provider/mod.rs` | 6868 行 | 每协议一个 adapter（请求体构建 / 响应解析 / SSE），`ProviderManager` 只编排与重试 |
| `src/stores/runtime.ts` | 5430 行，state 约 80 字段 | `session` / `stream` / `history` / `trace` / `queue` 拆 store，事件路由抽纯函数 reducer |
| `crates/pony-agent-core/src/bin/tui.rs` | 6581 行 | 界面/协议/命令三层 |

**先决条件**：新增工具的写入点仍为 8~11 处（`tools.rs` 内 11 处 + `present.rs` + `mod.rs` + 测试快照），拆分前需先把"描述符单一真值源"做掉，否则拆分只是搬家。

## 2. 依赖环（P1）

已复核的相互依赖：

- `runtime` ↔ `turn_flow`、`runtime` ↔ `graph`（`graph.rs` 用 `runtime::TurnResult`，`runtime` 又持 `GraphEngine`/`GraphRunStore`）、`graph` ↔ `planner`、`tools` ↔ `tool_runtime`；
- 能力层经 `tools.rs` → `subagent_runtime` 反向依赖 `runtime::AgentRuntimeBuilder`。

建议解法的前置动作：把 `TurnResult` / `TurnStreamEvent` 等共享类型下沉到独立 `types` 模块，改"graph 驱动 runtime"而非 runtime 持有 graph。**注意**：`runtime` 不 import `control_plane`，这个方向是单向的，不要一起动。

## 3. sync / stream 双实现（P1）

`turn_sync.rs`（约 1068 行）与 `turn_stream.rs`（约 2794 行）各自实现一遍工具循环与 hop 上限判定；`turn_runner.rs` 内还有一份无外部调用的自由函数副本。sync 路径**零取消检查**，而桌面 `run_turn` 命令（`src-tauri/src/lib.rs:60`）与子代理（`subagent_runtime.rs`）都在用它。

建议：抽出统一 `TurnLoop`，sync 实现为"stream + 收集型 sink"；删除 `turn_runner.rs` 的重复函数。

补充（wave-55 L3-P 复核发现，**wave-56 已闭环**）：provider 流式读循环内既无取消检查也无总时长上界的问题，
已由 `ProviderStreamGuard`（`provider/mod.rs` L517-545）修掉：180s 读空闲 + 30min 可注入总时长上界 +
每 250ms 观测既有取消源（`ExecutionControl::is_stop_requested`，接线于 `runtime/turn_stream.rs:1659`），
超限分别以 `provider_stream_deadline_exceeded` / `provider_stream_cancelled` 退出。
**仍待办**：sync 路径（`turn_sync.rs`）零取消检查问题未动（本波只覆盖流式路径）。

## 4. TS/Rust 契约手写（P1）

`src/types/runtime.ts`（1546 行）与 Rust DTO 全靠人工同步，已出现漂移（本轮已修 `TurnStreamEvent.step`）。建议引入 `ts-rs` 或 `specta` 生成类型 + CI diff 门禁。

## 5. 测试体系缺口（P1）

- 覆盖率门禁只覆盖 `App.vue`、`HomeWorkspace`、`HomeSessionSidebar`，`runtime.ts`（最关键的 store）不在门禁内；
- Playwright 跑在 vite preview 上并伪造 `__TAURI_INTERNALS__`，无真 Tauri E2E；
- 无 IPC 契约测试（命令入参/返回值序列化往返）。

## 6. 可观测性管道收敛（P2）

token 用量至少存 4 份（`TurnTrace`、`turn_events`、`usage_ledger`、OTEL gen_ai 属性）；日志三种风格并存。建议明确 `usage_ledger` 为计费真源，其余标为派生，并统一过 sanitizer。

## 7. 仓库卫生（P2）

`tmp/` 下 14 个临时产物已被 git 跟踪；`docs/imports-archived/` 内仍有 Windows 绝对路径（归档件，按归档语义保留，不再改写）。

## 8. Tauri 命令层：86 个同步命令在 IPC 处理线程内联执行（P1）

**机器事实**：`src-tauri/src/lib.rs` 共 92 个 `#[tauri::command]`，其中仅 6 个是 `async fn`。
Tauri 2.11.5 的 IPC 自定义协议处理器对同步命令**内联执行、无 spawn**：
`src/ipc/protocol.rs:314` → `webview/mod.rs:1742 on_message` → `manager/mod.rs:471 run_invoke_handler`，
同步命令宏展开后走 `.block(value, resolver)`（`src/ipc/command.rs:210-216`）。
桌面端该回调由 WebView 主线程驱动，因此 `list_sessions` / `load_session_*` 这类**同步 SQLite 读**会阻塞 UI 事件循环；
这与 ADR-0032 修过的主线程卡死属同一类风险，只是入口从 `trace_persistence` 换成了命令层。

**建议**：按热路径优先把读命令改为 `async fn` + `BlockingHelper::spawn`（`src-tauri/src/blocking_helper.rs` 已提供统一入口），
先覆盖 `list_sessions`、`load_session_traces`、`load_session_runtime_view`、`health_check`。
**不要**一次性把 86 个全改（爆炸半径过大，且多数命令本身是轻量参数翻译）。
