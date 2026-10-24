# 可观测性巡检适配性判定（wave-43 收口证据）

> 结论：**tracing / issues 两维已过硬件门禁；metrics / logs 两维对本项目「结构不适用」，且巡检器的 M1/M2 PASS 是文档误命中（非真实能力）**。
> 用户裁决（2026-10-24）：按适配性收口，不为了让巡检变绿而伪造 /metrics 端点或部署。
> 巡检命令：`bash ~/.agents/skills/observability-onboarding/scripts/audit-project.sh /home/dm/pony-agent --project pony-agent`

## 1. 为什么 metrics / logs 不适用

本项目是**桌面端（Tauri）+ CLI/TUI + headless harness**，交付形态是安装在用户机器上的二进制，不是集群内常驻服务：

```console
$ kubectl get deploy -A | grep -i pony
ponyllm   ponyllm-gateway-dev ...        # 只有网关系，无 pony-agent
$ grep -rn "/metrics" --include=*.rs --include=*.ts crates/ src-tauri/src src/
（无输出 —— 源码里不存在 /metrics 端点）
$ ls deploy/ k8s/ manifests/
（仓库内没有 k8s 清单）
```

- **metrics 维度**：拉取式抓取（Prometheus pull）前提是有一个稳定可达的 `/metrics`。桌面端无常驻地址、无 Service、不进集群，**结构上不成立**。
- **logs 维度（L3/L4）**：日志采集是节点侧 fluentd 按**容器日志路径白名单** tail。桌面端进程不产生 `/var/log/containers/*.log`，同样结构上不成立。
- **不是「还没接」，是「这套接法不适用于该交付形态」**。若将来真有集群内常驻形态（如 headless harness 作为服务部署），应单独立项，而不是给桌面端伪造一个端点。

## 2. 巡检器的两处误判（已记录，不作为交付依据）

### 2.1 M1/M2 是文档误命中

| 断言 | 巡检器判定 | 真实情况 | 误命中来源 |
| :--- | :--- | :--- | :--- |
| M1「代码暴露 /metrics」 | PASS | **不存在** | `g()` 搜索全仓非 vendor 文件，命中的是文档：`management/task-system/01_TASK_BOARD.md`、`openspec/changes/archive/**/design.md`、`.dev-team/*.md` 里的文字提及 |
| M2「清单声明 metrics 端口」 | PASS | **仓库无 k8s 清单** | 同样来自文档/归档文本 |

→ **metrics 维度的 PASS 不可采信**；真实状态是「不适用（无 pullable 服务）」。

### 2.2 T3 假阴性（已修）

| 断言 | 修复前 | 修复后 | 说明 |
| :--- | :--- | :--- | :--- |
| T3「存在显式 span 打点」 | PARTIAL | **PASS** | 巡检器正则只认 `start_as_current_span / start_span / tracer.start / with_span / #[instrument`，不认识 OTel 0.30 的标准写法 `Tracer::build_with_context(SpanBuilder, &Context)`（本项目用它是因为**采样器需要读到创建期属性**才能识别 `pony.otel.forced` 旁路标记） |

**已在 skill 侧修复**：`scripts/audit-project.sh` 的 T3 正则追加 `start_with_context|build_with_context`；
`scripts/selftest.sh` 仍 Exit 0（三档 fixture 判定未被污染），`scripts/check-drift.sh` 仍 Exit 0（契约未变）。

## 3. 公网入口上线后的复测（2026-10-24 晚）

infra 侧已完成 `https://otel.ponyjob.top/v1/traces`（Traefik Exact + TLS + ratelimit/buffering）
→ `otel-collector-edge`（bearertokenauth 多 token）→ 内部 collector → Tempo + Langfuse。
本仓用真实公网路径验证（探针经 `agent/otel` 发 `agent.turn` + `llm.chat` + `tool.call`）：

- 无令牌 401 / 裸令牌 401 / `Bearer <token>` 200 —— 由此发现并修复本仓鉴权 scheme 缺陷；
- Tempo 侧同一条 trace 的 resource：`{service.name=desktop-client, client.service.name=pony-agent, ingest.source=public}`；
- Langfuse 侧：`agent.turn`=SPAN(root)、`llm.chat`=**GENERATION**(model `gpt-4o-mini`, usage 123/45)、`tool.call`=**TOOL**(`read_file`)，父子关系正确。
- 由此发现并修复更严重的 **Resource 丢失**：包装导出器未转发 `SpanExporter::set_resource`，
  导致导出的 `ResourceSpans` 完全没有 resource（Tempo 无 `service.name`）。

复测后巡检：**tracing 5/5 PASS**（T4 由真实 trace 支撑，公网 edge 项目按 `client.service.name` 回退命中）。

## 4. 当前真实判定

| 维度 | 静态（写没写） | 动态（数据到没到） | 本次结论 |
| :--- | :--- | :--- | :--- |
| tracing | **T1/T2/T3/T5 全 PASS** | **T4 PASS**（公网路径实测 trace 已进 Tempo；按 `client.service.name` 归因） | **PASS 5/5** |
| issues | **I1~I4 全 PASS** | **I5 PASS**（PonySentry 有 `project=pony-agent` 的 issue 记录） | **PASS** |
| metrics | M1/M2 为文档误命中（真实：无端点） | M3 无抓取 | **结构不适用** |
| logs | **L1/L2 PASS**（单行 JSON + `trace_id`，新增日志面 `otel/log.rs::log_json`） | L3/L4 无集群采集面 | **静态达标，动态结构不适用** |

## 5. 剩余结构不适用项

见 `.dev-team/handoff-otel-public-ingress.md`。补充两条：

1. **metrics/logs**：若未来新增集群内常驻形态，再按 `contract.json` 的 `scrape_job_required_fields`
   与 log-collector `<source>/<filter>` 接入，属独立立项，不在本波。

## 6. 命令收据

```console
$ bash scripts/selftest.sh            # skill 自检
OK   selftest 全过                    → EXIT 0
$ bash scripts/check-drift.sh
✅ 契约无漂移：skill 与集群现状一致     → EXIT 0
$ audit-project.sh /home/dm/pony-agent --project pony-agent
metrics 🟡 PARTIAL 3/4 (M3)   ← M1/M2 为文档误命中，不计入
tracing 🟡 PARTIAL 4/5 (T4)   ← T3 已由 PASS 修复，T4 待上线
logs    🟡 PARTIAL 2/4 (L3,L4)
issues  ✅ PASS    5/5
```
