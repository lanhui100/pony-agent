# PonySentry 错误采集与全链路遥测接入架构报告

> 制定者: Protocol Agent (L2-P)  
> 状态: 契约已冻结 (Phase 1 交付物)  
> 对应任务: dev-team task-1

---

## 1. 架构目标与总体拓扑

本项目全面接入 PonySentry 遥测错误采集网关，实现对 **Agent 运行时、Tauri 桌面外壳与 Vue 前端工作台** 全链路未捕获崩溃与运行时故障的主动收集与智能聚合，彻底消除静默失败与错误盲区。

### 1.1 系统拓扑图

```
┌─────────────────────────────────────────────────────────────┐
│                    Pony-Agent Client                        │
│                                                             │
│  [Vue 3 SPA]                                                │
│    - errorHandler / unhandledrejection                      │
│    - safeInvoke IPC Error Interceptor                       │
│    - Breadcrumb Ring Buffer (max 64)                        │
│         │ (IPC: ponysentry_capture)                         │
│         ▼                                                   │
│  [Tauri Desktop Shell]                                      │
│    - std::panic::set_hook (Global Panic Capture)            │
│    - Tauri Command Error Wrappers                           │
│         │ (In-process Rust call)                            │
│         ▼                                                   │
│  [Rust Agent Core (pony_agent_core::agent::ponysentry)]     │
│    - Turn Lifecycle Errors (fail_stream_turn / watchdog)    │
│    - LLM Provider Network & Retry Exhaustion                │
│    - Tool Execution & Process / Sandbox Failures            │
│    - Zero-Trust Client Sanitizer (Path & Secret Redaction)  │
│    - Bounded Queue (1024) & Fire-and-Forget Async Worker    │
└──────────────────────────────┬──────────────────────────────┘
                               │ HTTP POST /api/v1/ingest
                               │ Timeout <= 3s, X-Client-Token
                               ▼
┌─────────────────────────────────────────────────────────────┐
│                   PonySentry Ingest Cluster                 │
│                                                             │
│   - k3s Cluster: http://10.43.94.160:3000                   │
│   - Ingress Endpoint: https://sentry.ponyjob.top/api/v1/ingest│
│   - Server-Side Sanitization Pipeline                       │
│   - Fingerprint & Deduplication Engine (Sha256)             │
│   - PostgreSQL Storage & Issue Lifecycle Machine            │
└─────────────────────────────────────────────────────────────┘
```

---

## 2. 接口契约与数据模型规范

### 2.1 Ingest 上报 Payload 契约

端点: `POST /api/v1/ingest`  
请求头: `Content-Type: application/json`，可选 `X-Client-Token: <token>`

```json
{
  "platform": "rust | tauri | vue",
  "release": "0.1.109",
  "environment": "production | staging | dev",
  "message": "可选的错误摘要文本",
  "exception": {
    "error_type": "UpstreamTimeout | Panic | ToolExecutionError | ...",
    "value": "脱敏后的错误具体详情",
    "stacktrace": [
      {
        "filename": "crates/pony-agent-core/src/agent/provider/mod.rs",
        "function": "post_openai_json",
        "lineno": 1752,
        "in_app": true
      }
    ]
  },
  "tags": {
    "session_id": "...",
    "turn_id": "...",
    "provider": "deepseek",
    "model": "deepseek-chat"
  },
  "extra": {
    "error_code": "upstream_timeout",
    "retry_attempts": 3,
    "elapsed_ms": 3000
  },
  "breadcrumbs": [
    {
      "category": "provider",
      "message": "requesting initial turn decision",
      "data": { "model": "deepseek-chat" }
    }
  ]
}
```

### 2.2 响应契约
```json
{
  "issue_id": "171e1147-909b-40f7-a45e-8d7cd0957ca1",
  "event_id": "5982f450-e618-480c-bb0b-3c83e613a0c4",
  "fingerprint": "283288c9e22111bc24b9ff816c2e7f7956f2e9d5db39913f280ea61d25aba4c2",
  "status": "unresolved",
  "count": 1
}
```

---

## 3. 全量扫描报告与接入点清单

基于对 `crates/pony-agent-core`、`src-tauri` 及 `src/` 的全面扫描，提取如下关键埋点接入清单：

### 3.1 P0 级严重故障与崩溃捕获点 (Crash & Panic Vectors)

| 序号 | 代码位置 | 错误表面 | 错误类型 | 提取上下文字段 | 优先级 |
|---|---|---|---|---|---|
| P0-1 | 全局进程入口 (`src-tauri/src/main.rs`) | `std::panic::set_hook` | `ProcessPanic` | 崩溃位置、线程名、panic payload、breadcrumbs | **P0** |
| P0-2 | `runtime/stream_support.rs:234` | `build_provider_call_cache_record` panic | `ProviderCachePanic` | `provider_name`, `model`, `turn_id` | **P0** |
| P0-3 | `hooks.rs:1259-1262` | `canonical_lifecycle_binding` panic | `HookBindingPanic` | `hook_id`, `lifecycle_point` | **P0** |
| P0-4 | `tools.rs:3055, 3175` | 子工具无隔离 panic 逃逸 | `ToolChildPanic` | `tool_name`, `workspace_root`, `turn_id` | **P0** |
| P0-5 | `sqlite_session.rs:2635` | `persist_commands_batch` 写入失败静默丢数据 | `SqliteWriteLoss` | `session_id`, `batch_size`, `sqlite_err` | **P0** |

### 3.2 P1 级核心业务与运行时故障接入点 (Runtime Failures)

| 序号 | 代码位置 | 错误表面 | 错误类型 | 提取上下文字段 | 优先级 |
|---|---|---|---|---|---|
| P1-1 | `runtime/turn_persist.rs:280` | `fail_stream_turn_with_hook_dispatch` (单一流式失败收口点) | `TurnStreamFailed` | `turn_id`, `session_id`, `error`, `step` | **P1** |
| P1-2 | `turn_flow.rs:846 / 481` | `emit_turn_failed` / `emit_stream_failed` | `TurnFailed` | `session_id`, `turn_id`, `provider`, `model` | **P1** |
| P1-3 | `src-tauri/src/lib.rs:168` | `fail_turn_for_watchdog` (看门狗超时收口) | `WatchdogTimeout` | `session_id`, `turn_id`, `error_detail` | **P1** |
| P1-4 | `provider/mod.rs:3404` | `retry_provider_scoped_with_report` 终态重试耗尽 | `UpstreamRetryExhausted` | `provider_name`, `model`, `attempts`, `last_error` | **P1** |
| P1-5 | `provider/mod.rs:1752, 1796` | `post_openai_json` / `post_anthropic_json` 请求与状态错误 | `UpstreamNetworkError` | `status_code`, `elapsed_ms`, `url`, `error_type` | **P1** |
| P1-6 | `tools.rs:6771` | `error_result` 工具中心错误生成器 | `ToolExecutionError` | `tool_name`, `code`, `message`, `hint` | **P1** |
| P1-7 | `process.rs:389, 407` | 进程管理 `kill_after` / `shutdown` 失败 | `ProcessTerminationError` | `handle`, `pid`, `err` | **P1** |
| P1-8 | `src/lib/tauri.ts:19` | `safeInvoke` IPC 核心调用捕获 | `IpcCommandError` | `command`, `args` (脱敏后), `error_message` | **P1** |
| P1-9 | `src/main.ts` | Vue 全局 `app.config.errorHandler` | `VueUnhandledError` | `component`, `lifecycle_info`, `stack` | **P1** |
| P1-10| `src/main.ts` | 浏览器 `unhandledrejection` | `UnhandledPromiseRejection` | `reason`, `stack` | **P1** |

### 3.3 P2 级重试、降级与静默恢复追踪点 (Recovery & Degradation)

| 序号 | 代码位置 | 错误表面 | 错误类型 | 提取上下文字段 | 优先级 |
|---|---|---|---|---|---|
| P2-1 | `tools.rs:6446` | `retry_tool_timeout` 工具超时重试丢弃记录点 | `ToolTimeoutRetried` | `tool_name`, `attempt`, `preview_error` | **P2** |
| P2-2 | `turn_stream.rs:1768` | `stream_initial_decision` 失败静默回退同步 `decide_sync` | `StreamFallbackSync` | `turn_id`, `stream_err` | **P2** |
| P2-3 | `runtime/session_ops.rs` 等 | 互斥锁中毒恢复 (`unwrap_or_else`) | `LockPoisonRecovered` | `lock_name`, `poison_error` | **P2** |
| P2-4 | `workspace.rs:308` | 工作区路径解析失败导致静默封阻 | `WorkspaceResolveFailed` | `raw_path`, `error` | **P2** |

---

## 4. 零信任脱敏安全规范 (Sanitization Contract)

遵循 `sanitizer.rs` 与 checklist 最佳实践，客户端必须在网络发出前执行一级深度脱敏（零信任双保险）：

1. **敏感凭据替换**:
   - 正则匹配 `token`, `password`, `secret`, `api_key`, `apikey`, `authorization`, `cookie`, `access_token` 等键名，将其值全量替换为 `[REDACTED_SECRET]`。
   - 识别 `Bearer <token>` / `Basic <base64>` 模式并整体替换。
2. **绝对物理路径脱敏**:
   - `/home/<username>/` -> `[USER_HOME]/`
   - `/Users/<username>/` -> `[USER_HOME]/`
   - `C:\Users\<username>\` -> `[USER_HOME]\`
3. **递归脱敏与深度防溢出**:
   - 对 `extra`, `breadcrumbs`, `tags` 递归清洗，递归深度上限硬锁为 32。
4. **禁止个人 PII 数据采集**:
   - 绝不采集用户名、明文提示词中包含的私钥与凭证文件全文。

---

## 5. NFR 商业基准对齐规范

已落盘于 `.dev-team/nfr-baseline.json`：
- **外部调用超时**: `external_call_timeout_ms <= 3000`（所有针对 PonySentry Ingest 的 HTTP 请求硬超时锁死为 3000ms）。
- **非阻塞式发射**: 采用有界队列 (`capacity: 1024`) 与独立 `tokio::spawn` / 后台线程，队列满时丢弃，绝不拖慢 Agent 主执行与前端界面渲染。
- **重试退避**: 最多重试 1 次（backoff 1000ms），严禁死循环重试造成雪崩。
- **并发与锁**: 内部状态与面包屑环形缓冲采用标准 `Mutex` 保护，上限 64 条环形淘汰。
- **格式与追踪**: 上报包含结构化字段，必要时携带 `trace_id` / `session_id`。
