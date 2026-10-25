# Wave-5 可观测性链路复审（真实轮次驱动）

- 复审时间：2026-10-25（Asia/Shanghai）
- 与 Wave-4 审计的关键差异：**本轮用真实 LLM 轮次驱动公网入口**，而不是合成 span 探针。
- 命令：`./target/debug/ponycli --prompt "只回复：pong"`，env 注入 `PONY_OTEL_TOKEN`（集群已签发设备 token）与 `OTEL_EXPORTER_OTLP_ENDPOINT=https://otel.ponyjob.top`，`settings.json` 中 `agentTraceReporting=true`。
- 说明：provider 网关 `10.42.0.67:8080` 在本机不可达（connect error）⇒ 该轮**失败**；失败轮同样产生完整链路数据，证据更强。

---

## 0. 结论

**桌面端（Tauri）链路在 Wave-4 修复后已真实贯通**；**本轮开发过程中又发现 CLI/TUI 交付形态完全缺引导（已修）**。当前仍存 **4 项** 需要优化（1 项 infra 待核实、2 项设计取舍、1 项 UNKNOWN）。

---

## 1. 真实轮次的物理收据（本轮新增，可复现）

### 1.1 Tempo
```
service.name 标签值集合 = ["desktop-client"]          ← edge 按契约改写
tcount 'client.service.name="pony-agent"' = 1         ← 原服务名保留（归因正确）
```

### 1.2 Langfuse（semantic 轨，trace 详情）
| 项 | 实测值 |
| :--- | :--- |
| trace id | `acfb983f6ecef85c50ba442a82b87c3c` |
| trace name | `turn-728cf460-a54a-4ad3-ac46-e0b534753617`（来自 `langfuse.trace.name`） |
| sessionId | `cli-session-2a9dc017-4722-46aa-9f6b-cf1b9a1df12f`（= `langfuse.session.id`） |
| resource | `service.name=desktop-client`、`client.service.name=pony-agent`、`ingest.source=public`、`deployment.environment=production` |
| span 树 | `SPAN agent.turn`（`agent.turn.phase=failed`、`level=ERROR`） ← `GENERATION llm.chat` ×2（`model=gemini-3.8-flash`） |

⇒ **父子嵌套、会话归因、语义路由（`pony.trace.route=genai`）、状态映射（failed → ERROR）全部正确。**

### 1.3 日志（本地取证面，本轮新增能力）
```
<state_dir>/pony-agent/logs/pony-agent.log   首行即合法单行 JSON
真实失败轮次后文件增长 230 → 2750 字节
```

### 1.4 Issue（本地 ingest sink 逐字节验证，本轮新增能力）
真实 connect 失败轮次 → 1 条上报：`tags.project=pony-agent`、`event_id=62feffb0…`、
`trace_id=8aacaa3c…`（等于该轮 `exported_trace_id`）、`error_type=TurnExecutionFailed`、
`culprit=pony_agent_core::agent::turn_flow::emit_stream_failed`、19 帧栈；同 turn 去重后仅 1 条。

---

## 2. 本轮新发现并已修复的断链

| # | 断链 | 证据 | 修复 |
| :--- | :--- | :--- | :--- |
| N1 | **CLI/TUI 无任何可观测性引导**：`grep otel/ponysentry` 在 `bin/cli.rs`、`bin/tui.rs` 均 0 命中 ⇒ 两种交付形态 tracing 与 issue 全断 | 首测：配好 token+公网端点跑真实轮次 → Tempo 零 span、PonySentry 零新增 | 两 bin 增 `ObservabilityBootstrap`（panic hook + `otel::init(from_settings)` + 门禁/预算同步）+ 退出有界 shutdown；TUI 两处 `process::exit` 显式 shutdown |
| N2 | **CLI/TUI 日志无落盘**：`set_log_file_path` 仅 Tauri 调用 | 无日志文件生成 | bins 注入 `<state_dir>/pony-agent/logs/pony-agent.log`（复用既有轮转/脱敏） |
| N3 | **core 侧 turn 失败无 Issue 上报**：`capture_error` 在 ponysentry 之外 0 命中 | CLI 失败轮后 PonySentry issue 数不变（9） | `enable_turn_failure_capture`（缺省关，仅 bins 开启，避免与桌面端前端重复）+ `capture_turn_failure`（按 turn 去重）+ `flush()` FIFO 屏障 |
| N4 | **`--prompt` 形态进程在 worker POST 前退出** ⇒ issue 丢失 | 无屏障时 0 条送达 | 有界 `flush()`（1500ms）接入两 bin 退出路径 |

---

## 3. 仍存缺口与优化建议（按优先级）

### ~~P1-1 Tempo 跨 2 小时数据不可查~~ → **已撤回：不是数据丢失，是查询工具的窗口缺陷（已修）**
- 原判断（错误）：03:00Z 写入的探针 trace 在 04:57Z 查不到，疑似重启丢数据。
- 复核（正确）：用**显式时间窗**查询同一 window（`start/end` 覆盖 02:40–03:20Z）→ **trace 仍在**
  （`traceID=5b8efff798038103d269b633813fc60c`）。Tempo Pod 实测 `restarts=0`、`terminationGracePeriodSeconds=90`、S3 后端 + 168h 保留 ⇒ **无数据丢失**。
- 根因：`observability-onboarding/scripts/lib.sh::tcount()` 的 tag 搜索**不带 `start/end`**，Tempo 对 tag 搜索使用很窄的默认窗口（≈15min）⇒ 一小时前的 trace 被判成不存在。
- 已修（skill 侧，见 `.dev-team/evidence/wave-6/skill-lib.sh.patch`）：`tcount()` 与 `tcount_q()` 对齐，缺省回看 24h。
- 连带修复（skill 侧，`skill-audit-project.sh.patch`）：`g()/gh()` 未排除文档/治理树与 `target*` 构建产物、且 `/metrics` 用裸正则命中 `metrics-diagnostics` 模块名 ⇒ M1/M2 假 PASS。修后 `M1=MISSING / M2=PARTIAL`，与 `docs/observability.yaml` 的 N/A 声明一致。

### P1-2 `llm.chat` 子 span 缺 `langfuse.session.id`（客户端，低成本）
- 现状：`langfuse.session.id` 只在 `agent.turn` root 上；Langfuse 会话归集靠 trace 级字段，因此**当前不影响检索**，但子 observation 单独导出/聚合时会丢会话。
- 建议：把 session id 下沉到 `llm.chat` / `tool.call`（重复属性成本极低）。

### P2-1 未计价模型无成本属性（设计取舍，已文档化）
- `gemini-3.8-flash` 等无法公开核价的模型不入 `PRICING_TABLE` ⇒ `gen_ai.cost.usd` / `agent.turn.cost.usd` 缺席、UI 显示「—」。
- 这是「宁缺勿错」的显式取舍；若要覆盖，需在模型注册处补可核价来源，而不是猜价。

### P2-2 Metrics 维度结构性 NA（保持）
- 桌面端/CLI 无常驻监听地址，Prometheus pull 不成立；`docs/observability.yaml` 已声明 `applicable: false` 并给结构性理由。
- `audit-diff.sh` 目前对该仓输出 `WARN 规则集为空（门禁空转）`——因为尚未声明 `enforcement_points`（reqwest 仍 4 处直连，无法用正则可靠拦截）。建议进 OBS-GOV backlog 建 `http_client` 收口点。

### P3-1 UNKNOWN：CLI 侧 issue 到**公网** PonySentry 未验证
- 本机没有发布流水线烧录的 `PONYSENTRY_CLIENT_TOKEN`，公网 ingest 返回 401（fail-closed 正确）。
- 已用本地 ingest sink 逐字节验证 payload 契约；公网侧验证建议在发布产物或注入 token 的环境执行。

### P3-2 失败轮的 usage 全 0
- 失败轮无 provider usage ⇒ Langfuse 显示 0 token、无成本。属真实语义，非缺陷；如需区分「未知」与「0」，可在 usage 缺席时不写 `gen_ai.usage.*`（现状已不写键，Langfuse 侧展示为 0）。

---

## 4. 复审方法可复现

```bash
# 1) 真实轮次（需设备 token）
PONY_OTEL_TOKEN=$(kubectl -n observability get secret otel-public-ingest-tokens \
  -o jsonpath='{.data.tokens\.txt}' | base64 -d) \
OTEL_EXPORTER_OTLP_ENDPOINT=https://otel.ponyjob.top \
./target/debug/ponycli --prompt "只回复：pong"

# 2) 查 Tempo（经集群探针）
in_probe "wget -qO- 'http://tempo.observability.svc.cluster.local:3200/api/search?limit=5&tags=client.service.name%3D%22pony-agent%22'"

# 3) 查 Langfuse（port-forward + Basic Auth）
kubectl -n observability port-forward svc/langfuse-web 13000:3000 &
curl -sS -u "$PK:$SK" "http://127.0.0.1:13000/api/public/traces?limit=3"

# 4) 契约与巡检门禁
bash ~/.agents/skills/observability-onboarding/scripts/check-declaration.sh .
bash ~/.agents/skills/observability-onboarding/scripts/audit-diff.sh --repo .
```
