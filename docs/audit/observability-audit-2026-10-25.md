# pony-agent 可观测性审计（2026-10-25，wave-44）

> 口径：声明（`docs/observability.yaml`）× 实测（Prometheus / Tempo / Loki / PonySentry / Langfuse）交叉判定。
> 数据窗：近 7 天（`since=7d`），采集时间 2026-10-25 17:1x–17:4x CST。
> 只读采集；端点事实来源 `~/.agents/skills/observability-onboarding/references/contract.json`。
> 本报告不替代「声明一致性」门禁：`check-declaration.sh` 另行 Exit 0。

## 1. 结论矩阵

| 维度 | 声明 | 实测 | 判定 | 证据 |
| :--- | :--- | :--- | :--- | :--- |
| metrics | `applicable=false`（桌面端无常驻监听） | Prometheus 48 target，`job=~".*(desktop\|pony-agent).*"` → **0** | **一致**（结构性 N-A） | `count(up)=48`；`count(up{job=~...})=空` |
| tracing | `applicable=true` | Tempo 7d `service.name=desktop-client` **77** 条（24h 76） | **PASS** | `/api/search?tags=service.name="desktop-client"` |
| logs | `applicable=false`（非容器，无节点侧路径） | Loki 7d 含 desktop-client/pony-agent 行数 **0**（全库 448453） | **一致**（结构性 N-A） | `label/namespace/values` 无本项目 |
| issues | `applicable=true` | PonySentry `project=pony-agent` **11 issue / 185 事件**（7d 上报 4 次） | **PASS** | `/api/v1/issues?project=pony-agent` |
| langfuse（语义轨） | 必需（AI Agent） | ClickHouse 7d **47 trace / 100 observation**，与 Tempo `route=genai` 48 条对账 | **PASS** | `SELECT count() FROM traces/observations` |

`audit-project.sh` 自动矩阵：metrics PARTIAL / tracing PASS / logs PARTIAL / issues PARTIAL（**exit 2**）。
差异来自工具对 desktop 项目的探测缺陷（见 P3-1），以本表人工校正后的口径为准。

## 2. 关键数据

- Tempo：77 条 trace = `acc.size.probe` 27 + `acc.public.probe` 24 + `agent.turn` 18 + `public-otel-smoke-span` 8。
  即 **59/77（76.6%）为验收探针**；真正的产品运行时 trace 仅 18 条。
- Tempo 错误面：`{resource.service.name="desktop-client" && status=error}` = **9** 条（11.7%）；
  `tool.call` 命中 10 条、其中 error 2 条；`governance.decide` 4 条。
- PonySentry：unresolved 2（provider 连接失败 ×3、探针 ×1）、ignored 7、resolved 2；
  ignored 中 6 条 `DIAG_STUB_UNIMPLEMENTED` 合计 **178 次事件（96%）**。
- Prometheus：1 个 down target `api-v2-dev-host`（`up=0` 且 `scrape_samples_scraped=0`）；`otelcol_*` 指标全缺。

## 3. 发现（按优先级）

### P1（影响“看得见/看得对”）

1. **根 span 不承接错误状态 → 按根状态告警会全漏**
   - 证据：trace `4de1a7c76c2211de2e87711caa54158e`（31.64s）根 `agent.turn` `status={}`，
     子 `tool.call` 为 `STATUS_CODE_ERROR`（`child_not_allowed` / `invalid_path` / `non_zero_exit`）。
   - 影响：任何以 root status / trace 级 error 判定的告警与看板都会漏掉这类失败。
2. **验收探针污染主维度（基数与口径双重污染）**
   - 证据：59/77 探针；`client.service.name` 7d 出现 36 个值，其中 33 个为一次性设备 ID
     （`desktop-acc-probe-*` / `macbook-air-*` / `size-probe`），真实生产标识 `pony-agent` 仅 16 条。
   - 影响：按 client 维度聚合的错误率/延迟/成本被探针稀释，无法用于 SLO。
3. **issue 噪声被“静音”而非消除**
   - 证据：ignored 7 条中 `DIAG_STUB_UNIMPLEMENTED`（IPC stub 历史产物）占 178/185 事件；
     真实 unresolved 仅 2 条，列表头部仍被噪声占据；无 TTL/自动归档。
   - 影响：真实问题发现时间被拉长；PonySentry 作为信号源的可信度下降。
4. **归因维度缺失：环境恒为 production、版本缺失**
   - 证据：抽查 50 条 trace 的 resource 全部 `deployment.environment=production`，且无 `service.version`。
   - 影响：无法区分 dev/preprod/prod 流量，也无法把 trace 与发布版本（PonySentry `last_release`）对齐。
5. **LLM 语义属性不完整 → 成本/Token 视图不可用**
   - 证据：`llm.chat` 属性仅 `gen_ai.system/request.model/operation.name` + `langfuse.observation.model.name`；
     无 `gen_ai.usage.input_tokens/output_tokens`、无 `gen_ai.response.model`、无 tool→turn 关联。
   - 影响：Langfuse 有 47 trace/100 observation 但 Token 与成本字段为空；跨会话成本归因只能回本地 DB。

### P2（管道自身与聚合口径）

6. **trace 管道自身不可观测**：Prometheus 无 `otelcol_*` 指标（无 scrape job 或未开 telemetry），
   export 失败/丢弃/队列水位无法告警（本次只能靠 collector 日志 + ClickHouse 行数人工对账）。
7. **Prometheus 聚合口径不可靠**：`sum(up==0)` 返回 0 而逐条 `up==0` 有 1 条（`api-v2-dev-host` 空抓取）。
   巡检脚本若只用聚合函数会漏掉空抓取 target。
8. **本地日志不上送（结构性选择，非缺口）**：桌面端日志落 `<app_data_dir>/logs/pony-agent.log`，
   远程排障只能依赖用户导出；建议提供可选“诊断包”上传（脱敏 + 显式同意）。

### P3（工具/接口能力缺口）

9. **`audit-project.sh` 对 desktop 项目探测失真**：service/project 为空，
   “T4=10 条命中”是 `limit=10` 上限而非真实计数（真实 77），I5 在空 project 上产生假 PARTIAL。
10. **PonySentry 分析接口缺口**：`/api/v1/events`、`/api/v1/stats` 均 404；`/api/v1/issues/:id/events`
    `limit` 无效（恒 20 行）→ 单 issue >20 事件无法精确统计。

## 4. 优化点（可直接立项）

| # | 动作 | 验收标准 | 关联发现 |
| :--- | :--- | :--- | :--- |
| O1 | `agent.turn` 在工具/治理失败时置 `ERROR` + `status.message`，并聚合子 span 错误 | TraceQL `{name="agent.turn" && status=error}` 计数与真实失败数一致 | P1-1 |
| O2 | resource 补 `service.version`；`deployment.environment` 取真实运行环境 | 抽查 10 条 trace 属性齐全且与发布版本一致 | P1-4 |
| O3 | `llm.chat` 补 `gen_ai.usage.*` 与 `gen_ai.response.model`，`tool.call` 关联 turn/step | Langfuse 成本/Token 非空；同 turn 可下钻工具链 | P1-5 |
| O4 | 探针隔离：resource 打 `pony.probe=true` 或独立 `service.name`，看板/告警排除 | `desktop-client` 维度探针占比 <5% | P1-2 |
| O5 | issue 治理：ignored TTL 归档 + suppressed 率指标；客户端硬抑制 DIAG_STUB | ignored 占比 <20%，unresolved 列表信噪比可读 | P1-3 |
| O6 | 把 `{status=error}` 全 span 扫描写成告警规则（不依赖 root status） | 注入一条 tool 失败可触发告警 | P1-1/P2-6 |
| O7 | Prometheus 为 otel-collector 增加 scrape job（`otelcol_*`） | `up{job="otel-collector"}=1` 且有 exporter 指标 | P2-6 |
| O8 | 巡检改逐 target 判定（禁用 `sum(up==0)` 单口径） | `api-v2-dev-host` 空抓取能被巡检报出 | P2-7 |
| O9 | 修正 `audit-project.sh` desktop 探测/计数（回写 skill） | 对 pony-agent 输出真实 service/project 与 77 条计数 | P3-9 |

## 5. 与 wave-44 变更的关系

- PA-117 新增 `[workspace-scroll]` 调试开关：属于**开发者诊断输出**（非结构化日志面），
  默认关闭后生产态不产生该 console 输出；已在 `docs/observability.yaml` 的 note 登记归属。
- 本波无新增对外接口、网络 IO、后台任务或 LLM 调用；`audit-diff.sh` 与 `check-declaration.sh`
  的收据见 `.dev-team/evidence/wave44/`。

## 6. 机器收据

| 命令 | Exit | 结果 |
| :--- | :--- | :--- |
| `audit-project.sh /home/dm/pony-agent --json` | 2 | metrics PARTIAL / tracing PASS / logs PARTIAL / issues PARTIAL（desktop 探测失真，见 P3-9） |
| `check-declaration.sh /home/dm/pony-agent` | 0 | 四维声明一致（metrics/logs 结构性 N-A 有理由） |
| Tempo `/api/search`（7d，limit 500） | — | 77 traces；error 9；probe 59；genai 48 |
| PonySentry `/api/v1/issues?project=pony-agent` | — | 11 issue / 185 事件 |
| ClickHouse `count(traces/observations)` | — | 47 / 100 |
| Prometheus `count(up)` / `up==0` | — | 48 / 1（api-v2-dev-host） |
| `agent-browser` dev :4176 / prod preview :4177 | — | Console Error=0；生产态 `[workspace-scroll]`=0（PA-117 门控生效） |
| `audit-diff.sh --base ab54b06 --decl-approved` | 1（仅 WARN） | 无违规；WARN=声明无收口点（结构性，见 §7） |

## 7. 盲区（UNKNOWN，不判 PASS）

- 桌面端进程级资源指标（CPU/内存/句柄）无落点：remote_write 未开，是否该开属 infra 决策。
- Langfuse `user_id/session_id` 覆盖率未统计（`langfuse.session.id` 仅见 CLI 前缀）。
- PonySentry 单 issue >20 事件精确计数受接口限制。
- 公网 edge 的 token 签发/限流/采样策略未审计；Tempo 保留期与磁盘水位未审计。
