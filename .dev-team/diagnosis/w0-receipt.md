# W0 本地 trace/issue 接入 — 物理收据

时间：2026-11-05 17:33–17:36 CST
配置：~/.config/pony-agent/settings.json（权限 600；已备份 /tmp/pony-settings-before-w0.json）
  - otelEndpoint=https://otel.ponyjob.top
  - otelServiceName=pony-agent-dev
  - otelToken=<from observability/otel-public-ingest-tokens tokens.txt，已脱敏>
  - ponysentryIngestUrl=http://127.0.0.1:3000
  - ponysentryClientToken=<from pony-sentry/pony-sentry-secrets CLIENT_TOKEN，已脱敏>
  - agentTraceReporting=true

## 验证 1：OTel token 对准公网 edge
POST https://otel.ponyjob.top/v1/traces（Authorization: Bearer <otelToken>）→ HTTP 200

## 验证 2：真实 CLI turn 上传
命令：target/debug/ponycli -p "只回答：1+1等于几？不要调用工具。"
（provider 连接本地网关失败但 turn 完成并落 trace）
traceId=3476d3d72a1be57806ce3a64400b8a53
Tempo: GET http://10.43.51.135:3200/api/traces/3476d3d72a1be57806ce3a64400b8a53 → HTTP 200
resource: service.name=desktop-client（edge 改写），client.service.name=pony-agent-dev，service.version=0.1.154
spans: agent.turn / llm.chat / governance.decide / tool.call
Langfuse ClickHouse: select count() from traces where id='3476d3d72a1be57806ce3a64400b8a53' → 1
时钟：2026-11-05 09:33:34Z（17:33 CST）

## 验证 3：PonySentry issue
POST http://127.0.0.1:3000/api/v1/ingest（X-Client-Token: <ponysentryClientToken>）→ HTTP 201
issue_id=54a22e96-218b-4ba5-b305-163a04c35f11（dev 环境探针，非产品 issue）

## 验证 4：日志
`~/.local/state/pony-agent/logs/pony-agent.log` 最后一条「未配置鉴权 token」= 2026-11-05T09:13:17Z（早于配置 09:33Z）；配置后无新增。
