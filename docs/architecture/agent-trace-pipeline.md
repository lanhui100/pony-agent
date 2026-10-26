# Agent Trace 全链路（客户端 → 公网 OTLP 入口 → Tempo / Langfuse）

> 本文是 agent trace 上报的**拓扑与契约真源**（人读）。决策缘由见
> [ADR 0047](../decisions/0047-agent-trace-otel-langfuse-migration.md)；集群侧实现见
> `cluster-infra`（`manifests/observability/otel-collector*/`、`.agents/notes/implemented/architecture/2026-10-24-public-otel-ingress.md`）。

## 1. 链路拓扑

```
┌────────────────────────────┐
│ 桌面端 / CLI（用户机器）      │  默认**不上报**：配置页「全局 Agent Trace 上报」开关 false
│  agent/otel（OTel Rust SDK）│  开启后：OTLP/HTTP-protobuf + Authorization: Bearer <token>
└──────────────┬─────────────┘
               │ HTTPS  POST https://otel.ponyjob.top/v1/traces   （pathType: Exact）
               ▼
┌────────────────────────────┐
│ Traefik（aliyun 节点 443）   │  redirect-https → ratelimit → buffering
└──────────────┬─────────────┘
               ▼
┌────────────────────────────┐
│ otel-collector-edge         │  bearertokenauth/public（多 token 文件，逐枚常量时间比对）
│ （observability 命名空间）    │  transform/sanitize：
│                             │    ① client.service.name ← 原始 service.name
│                             │    ② service.name ← "desktop-client"
│                             │    ③ ingest.source ← "public"
└──────────────┬─────────────┘
               │ gRPC 明文（集群内）
               ▼
┌────────────────────────────┐
│ otel-collector（内部）       │  transform/route：命中 gen_ai.*/langfuse.* → pony.trace.route=genai
│                             │   traces/system → otlp/tempo（全部 span）
│                             │   traces/genai  → otlphttp/langfuse（只送 genai 副本，basicauth 由 collector 持有）
└───────┬─────────────┬──────┘
        ▼             ▼
     Tempo        Langfuse（语义树：SPAN/GENERATION/TOOL）
```

## 2. 客户端契约（本仓实现）

| 项 | 契约 | 代码位置 |
| :--- | :--- | :--- |
| 端点 | `OTEL_EXPORTER_OTLP_ENDPOINT`；集群内缺省 `http://otel-collector.observability.svc.cluster.local:4318`，桌面端 release 用 `https://otel.ponyjob.top` | `agent/otel/mod.rs::from_env` |
| 服务名 | `OTEL_SERVICE_NAME`（缺省 `pony-agent`）；**公网入口会把它改写成 `desktop-client`**，原值保留在 `client.service.name` | 同上 |
| 令牌 | `PONY_OTEL_TOKEN` = **裸 token**；客户端自动组装成 `Authorization: Bearer <token>`（已带 scheme 则不重复） | `authorization_header_value` |
| 采样 | `OTEL_TRACES_SAMPLER_ARG`（缺省 `1.0`） | 同上 |
| 门禁 | `AppSettings.agentTraceReporting`（默认 `false`）→ `set_reporting_enabled`；关闭时 `GatedSampler` 直接 Drop，导出器逐 span 只放行 `pony.otel.forced` | `agent/otel/mod.rs` |
| 超时 | 导出 3000ms、`force_flush` ≤3000ms；不重试 | `EXPORT_TIMEOUT_MS` / `MAX_FLUSH_WAIT_MS` |
| 脱敏 | 所有字符串型 span 属性统一过 `ponysentry::sanitize` | `push_str` / `set_phase` |
| 明文 HTTP 防线 | 有令牌 + `http://` + 非回环/非集群内主机 ⇒ `init` 直接 Err | `is_plaintext_remote` |

### 2.1 Span 语义

| Span | 关键属性 | Langfuse 侧呈现 |
| :--- | :--- | :--- |
| `agent.turn`（root） | `langfuse.session.id` / `langfuse.trace.name` / `gen_ai.operation.name=agent.turn` / `agent.turn.id|sequence|phase` | SPAN（trace 根） |
| `llm.chat` | `gen_ai.system` / `gen_ai.request.model` + `langfuse.observation.model.name` / `gen_ai.usage.input_tokens|output_tokens` | **GENERATION**（带 model 与 token 用量） |
| `tool.call` | `gen_ai.tool.name` / `langfuse.observation.type=tool` / `agent.tool.status|duration_ms|error` | **TOOL** |

父 span 解析：按 `tokio::task::Id` 索引 + `thread_local` 兜底（非 tokio 上下文如 TUI/CLI 同步路径）。

## 3. 手工验证（上线后复跑）

```bash
# ① 无令牌必须 401（不是 404，也不是 200）
curl -sS -o /dev/null -w '%{http_code}\n' -X POST https://otel.ponyjob.top/v1/traces \
  -H 'content-type: application/x-protobuf' --data-binary ''
# ② 裸 token 也必须 401（edge 用 bearertokenauth，认 scheme）
# ③ Bearer + token 必须 200
TOKEN=$(kubectl -n observability get secret otel-public-ingest-tokens -o jsonpath='{.data.tokens\.txt}' | base64 -d | head -1)
curl -sS -o /dev/null -w '%{http_code}\n' -X POST https://otel.ponyjob.top/v1/traces \
  -H "authorization: Bearer $TOKEN" -H 'content-type: application/x-protobuf' --data-binary ''
# ④ 真实 span（用本仓 otel 模块发一条 turn+llm+tool），再查两条轨：
#    Tempo：  { resource.client.service.name="pony-agent" }  → 有 trace
#    Langfuse：GET /api/public/observations?traceId=<id>     → agent.turn/SPAN、llm.chat/GENERATION、tool.call/TOOL
```

巡检（skill）：`audit-project.sh /home/dm/pony-agent --project pony-agent`
—— tracing 维度的 T4 会自动先查 `service.name=<svc>`，未命中再回退 TraceQL
`{ resource.client.service.name="<svc>" }`（公网 edge 入口项目的正确查法）。

## 4. 运维要点

| 事项 | 做法 |
| :--- | :--- |
| 令牌轮换 | 改 Secret `otel-public-ingest-tokens` 的 `tokens.txt`（每行一枚）→ `kubectl -n observability rollout restart deploy/otel-collector-edge`（不依赖热重载） |
| 新增设备令牌 | 同上追加一行；旧令牌保留一个过渡窗口后删除 |
| 限流/体积 | Ingress middlewares `otel-public-ratelimit` / `otel-public-buffering`；批量 5s/1024 条 |
| 数据归因 | 用 `ingest.source=public`（链路）+ `client.service.name`（真实生产者）+ `service.name=desktop-client`（隔离桶）三层过滤，不要指望从 `service.name` 认出设备 |
| 关闭上报 | 桌面端配置页关闭开关；关闭后零 span 分配、零网络出口（有单测与 wire 级证明） |

## 5. 故障排查

| 现象 | 最可能原因 | 一步验证 |
| :--- | :--- | :--- |
| 401 `missing or empty authorization header` | 没带 token，或只发了裸 token（缺 `Bearer `） | 上面 ②/③ 两条 curl |
| 200 但 Tempo 无 trace | 采样率为 0 / 开关关闭 / 时区与 `since` 不匹配 | 客户端 `force_flush` 返回值；`audit-project --json` 看 T4 |
| Tempo 有、Langfuse 无 | span 没带 `gen_ai.system` 或任意 `langfuse.*` 属性 ⇒ 未命中语义轨谓词 | Tempo 里看 `pony.trace.route` 是否 `genai` |
| 只有 `desktop-client` 找不到自己的服务 | 未按 `client.service.name` 过滤 | TraceQL `{ resource.client.service.name="<svc>" }` |
| 所有设备混在一起 | 未用 `client.service.name` / `ingest.source` 归因 | 同上 |
| 会话在 Langfuse 里被拆成多条 trace | turn 结束即关闭 root span，跨 turn 不续接（设计如此） | 按 `langfuse.session.id` 聚合查看 |
