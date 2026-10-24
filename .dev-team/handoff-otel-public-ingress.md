# 交接单：OTLP 公网入口（`otel.ponyjob.top`）—— 留给下一会话

> 用户裁决（2026-10-24）：**仍按 `otel.ponyjob.top` 实现，但不在本次计划内**。本会话只负责把
> **接口与配置面准备好**，上线时把 endpoint 与 token 填进去即可。B1 已从本波次移出，不阻塞代码交付。

## A. 客户端侧（本会话已完成，无需再动）

| 项 | 值 / 位置 | 说明 |
| :--- | :--- | :--- |
| 端点来源 | env `OTEL_EXPORTER_OTLP_ENDPOINT` | 代码零硬编码；集群内缺省 `http://otel-collector.observability.svc.cluster.local:4318` |
| 服务名 | env `OTEL_SERVICE_NAME`，缺省 `pony-agent` | Tempo 检索主键 |
| 采样 | env `OTEL_TRACES_SAMPLER_ARG`，缺省 `1.0` | 桌面端显式授权 + 单机低频 |
| 令牌 | env `PONY_OTEL_TOKEN` → `authorization` 头 | 代码只透传，不校验内容、不落盘 |
| 开关 | `AppSettings.agentTraceReporting`（默认 `false`） | 关闭时 GatedSampler Drop，零 span 分配 / 零 egress |

**上线时唯一需要的动作**：在发布配置里设置
`OTEL_EXPORTER_OTLP_ENDPOINT=https://otel.ponyjob.top` 与 `PONY_OTEL_TOKEN=<预留令牌>`。
无需改代码。

## B. 基础设施侧（下一会话在 `/home/dm/cluster-infra` 完成）

> 该仓现状：G1「collector 只准 ClusterIP、零公网入口」与 G5「ingress 无 tempo/langfuse/otel」写死在
> `docs/architecture/system-design.md:16,20`；6 组验收断言把「零 otel 入口」当承诺：
> `exposure_test.sh` E4(:142)/E5(:166)/E6(:183)/E10(:221-231)、`observability_alerts_test.sh` D14(:474)/D15(:481)、
> `postflight_test.sh` F13(:280)。此外 `otel.ponyjob.top` 当前**不解析**，EdgeOne CDN → aliyun 443 的链路不在任何仓内。

必须完成的工作清单：

1. **ADR + 蓝图修订（同提交）**：在 `.agents/notes/` 立新决策记录（含 `## Alternatives considered`），
   修订 `docs/architecture/system-design.md` 的 G1/G5，明确「collector 仍零公网入口；公网只开放
   `/v1/traces` 的**认证型** OTLP 转发路径」。**不得**只是新增 Ingress 而不改不变量。
2. **鉴权（硬前置）**：collector receiver 无 auth、无 TLS（`10-otel-collector-configmap.yaml:25-32`，
   tempo exporter 还 `insecure: true`）。必须二选一：
   - (i) 在 collector 加 **auth 扩展**（bearer/basicauth）并只对公网路径生效；或
   - (ii) 前置一层认证中继（Traefik `basicauth` middleware 需新建；集群当前 0 个 basicauth 实例）+ `rateLimit`。
   **禁止**裸露 4318 到公网（= 匿名可写 spans）。
3. **Ingress 精确化**：`host: otel.ponyjob.top`、`path: /v1/traces`、`pathType: Exact`、TLS 走 cert-manager
   `letsencrypt-prod`（照 `pony-sentry/deploy/k3s/app.yaml:85-115` 范式）。**禁止** `Prefix /`（会把
   `8888/metrics` 一起放出去）；4317 gRPC 走 Ingress 不生效，需 `IngressRouteTCP`（集群 0 个、模板已废弃，别复活）。
4. **限流与体积**：加 rateLimit（注意 EdgeOne 后 `ipStrategy.depth:1` 拿到的是 CDN IP，会把全量用户合成一个源，
   需按 token/头部限流而非 IP）+ 请求体上限（OTLP batch 可能超过 sentry 现用的 512KB buffering）。
5. **DNS/CDN**：确认 `*.ponyjob.top` 是否泛解析；若需 EdgeOne 侧新增 host，走对应变更流程。
6. **验收断言同步**：更新上述 6 组脚本的「零 otel 入口」断言为「otel 入口存在但需认证」，
   并保留「collector Service 恒 ClusterIP」这一条不变（这才是真正的安全边界）。
7. **契约重录**：`bash ~/.agents/skills/observability-onboarding/scripts/record-contract.sh --confirm`
   把公网 OTLP 端点与鉴权方式写进 `references/contract.json → dimensions.tracing`，再跑
   `check-drift.sh` 与 `selftest.sh` 双 Exit 0；集群仓库侧 `scripts/observability/check-skill-contract.sh` 同 PR 绿。

## C. 上线后本仓的回读验收（下一会话或上线时执行）

```bash
# 1) 公网入口可达且需认证（无 token 应 401/403，不是 404 也不是 200）
curl -sS -o /dev/null -w '%{http_code}\n' -X POST https://otel.ponyjob.top/v1/traces \
  -H 'content-type: application/x-protobuf' --data-binary ''
# 2) 带 token 上报一帧 OTLP，Tempo 能查到 service.name=pony-agent
# 3) Langfuse 能按 session 看到 agent.turn → llm.chat / tool.call 的观察树
# 4) 关闭全局开关后重复 2)，Tempo/Langfuse 均无新数据（零 egress 证明）
```
