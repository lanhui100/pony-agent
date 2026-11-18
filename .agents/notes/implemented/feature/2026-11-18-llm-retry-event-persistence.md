# Agent Note: provider 重试事实持久化为会话事件（llm/retry）

Status: implemented

工单：B214。影响文件：`agent/turn_event.rs`、`agent/retry.rs`、`agent/retry_projection.rs`（新）、
`agent/turn_flow.rs`、`agent/turn_flow/turn_flow_registry.rs`、`agent/runtime/{mod,turn_stream,turn_sync}.rs`、
`tests/b214_retry_event_persistence.rs`（新）。

## Problem

provider 重试进度此前只经 `thread_local RETRY_OBSERVER` 投给 TUI 的 `UiEvent::ProviderRetry`：
进程内一次性、不落盘、不跨界面，重启即丢；桌面端/CLI/会话重载后完全不可见，排障只能翻运行日志。
B237/C5.1 已把 attempts 落进 trace 记录（`ProviderCallRetryMeta`），但**会话事件流**仍无重试事实，
客户端无法在重载后投影「正在重试模型请求（2/5）· 3s」。

## Decision

新增 additive 会话事件 `llm/retry`（`TurnEvent::LlmRetry`），字段：

| 字段 | 语义 |
|---|---|
| `turn_id` | 所属轮次（与 `turn/start` 同 id） |
| `request` | provider 调用名（decision_stream / followup …） |
| `scope` | `decision` / `followup` |
| `failure_code` | timeout / rate_limited / quota / network / auth / bad_request / other |
| `retry` | 已失败尝试数（1 起，即「第 N 次重试」分子） |
| `max_attempts` | 本跳允许尝试总数（含首次，分母） |
| `delay_ms` | 本次退避毫秒数 |

- **写路径**：`retry.rs` 在 `notify_provider_retry` 派发实时观察者**之后**独立落盘（观察者缺失不影响
  持久化），经 `turn_flow_registry::emit_turn_scoped_event` 走既有 `dispatch_event_persist` 通道；
  `event_times_ms` 复用既有逐事件时间戳语义，**不新增第二套持久化通道**。
- **上下文注入**：轮入口 `AgentRuntime::enter_turn()` 统一登记「轮次序号 + 重试事件上下文守卫」，
  TUI 实时通道保留（不删）。
- **读路径**：`agent/retry_projection.rs` 提供 `ProviderRetryProjectionState`（`latest()` /
  `retries_for_turn()`，`>=` 水位），客户端从事件日志投影重试展示。
- 不实现 `llm/retry-started`（保持最小闭包；当前需求只需失败后退避的可见性）。

## Alternatives considered

- **A. 只把 thread_local 观察者改成全局静态。** 落选：仍不落盘、不跨重启，且无法跨进程/界面。
- **B. 复用 `turn:provider_call` 事件加 attempts 字段。** 落选：该事件是**结算后**的单条摘要，
  重试进行中的「第 N 次 + 退避时长」在它落地前不可见；且会改变既有事件语义。
- **C. 新起独立 durable 文件/表。** 落选：会话事件流已有逐事件时间戳与重载投影，重复造通道
  会引入第二份真源。

## Verification

- `cargo-lane test --lib retry` 66 passed、`--lib turn_event` 13 passed；
- `cargo-lane test --test b214_retry_event_persistence` 1 passed（写入 → 落盘 → 新 backend 重载 →
  投影断言：`turn/start` + 两条 `llm/retry`、event_type、每事件 `created_at_ms`、投影 2/5、3000ms）；
- `npm run rust:lint` 3870/3870 新增 0；`size:check` 0 new。
