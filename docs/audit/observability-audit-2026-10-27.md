# 可观测性四维上报审计（2026-10-27，真实会话 pa_01a17b23 驱动）

> 口径：声明（`docs/observability.yaml`）× 实测（Tempo / Langfuse / PonySentry / 本仓代码）交叉判定。
> 真实会话：`pa_01a17b23-1e98-7a53-a2bb-15d116495edb`（桌面端；trace 报 core 计数 0.1.135，
> 当前发布线为 0.1.162，二者不是同一计数线，见 P0-1）。
> 端点事实来源：`~/.agents/skills/observability-onboarding/references/contract.json`（contract_version 1.2.0）。
> 采集时间 2026-10-27 10:49–11:10 CST；全部只读。

## 1. 机器门禁收据（先证「没接错」）

| 命令 | 结果 | 说明 |
| :--- | :--- | :--- |
| `ponygo observe audit --level ob1 --json` | **EXIT 0** | `current=ob2`、`target=ob1`；declaration/no_placeholder/consistent/enforcement 四项 pass |
| `bash scripts/check-declaration.sh .` | **EXIT 0（PASS）** | metrics/logs「声明与现状一致」；tracing `T1~T5` 全 PASS；issues `I1~I5` 全 PASS |
| `bash scripts/audit-diff.sh --base origin/main` | **EXIT 0** | 无越界实现、收口点完整（含本报告文档增量复跑） |

结论：**四维归属声明没有造假、没有漏维**；`metrics`/`logs` 的 N-A 是结构性理由（桌面端无常驻监听、
不进容器日志白名单），不是为了变绿改 `applicable`。

## 2. 真实会话事实（Tempo × Langfuse 对账）

- Trace：`cd37e079ff7012f85b3dba913441a923`，`agent.turn` 根 span **21.96s**，共 **9 个 span**。
- 结构：`agent.turn` ×1 + `llm.chat` ×4 + `tool.call` ×3（List/Read/Search）+ `governance.decide` ×1。
- 用量：根 span `gen_ai.usage` = 32,120 in / 461 out，**与 4 个 `llm.chat` 之和完全一致**（wave-49 聚合有效）。
- 工具耗时：`tool.call` span 10 / 15 / 68 ms，与属性 `agent.tool.duration_ms` 一致（不再恒 0）。
- 归因 resource：`service.name=desktop-client`、`client.service.name=pony-agent`、`pony.app.name=pony-agent`、
  `ingest.source=public`、`ingest.client.kind=device`、`deployment.environment=production` —— 齐全。
- Langfuse：1 条 trace / **8 个 observation** = SPAN ×1 + GENERATION ×4 + TOOL ×3；工具与模型名正确映射。
- 24h 窗口同服务 trace 3 条：1 条生产桌面（本会话）+ 2 条开发态 CLI（`deployment.environment=development`）。

## 3. 结论矩阵

| 维度 | 声明 | 实测 | 判定 |
| :--- | :--- | :--- | :--- |
| tracing | applicable=true | Tempo 9 span / Langfuse 8 obs；归因、用量、耗时可用 | **PASS**（有 3 处值错/缺项，见 §4 P0-1/2/3） |
| issues | applicable=true | 本会话无错误 ⇒ 0 事件（预期）；项目级管道有数据（last_seen 2026-10-25） | **PASS**（release 标注错，见 P0-1） |
| metrics | N-A（桌面端无常驻 /metrics） | 无伪造 `/metrics`，结构性理由成立 | **一致** |
| logs | N-A（不进容器日志白名单） | 仅本地落盘取证面，不上送 Loki | **一致** |

## 4. 错漏清单（按优先级）

### P0-1 `service.version` / PonySentry `release` 与安装包版本脱钩
- 证据：本会话 trace `service.version=0.1.135`（core crate 计数），当前发布线为 **0.1.162**；
  Langfuse trace `version=0.1.135`、`release=null`。PonySentry issue `last_release=0.1.128`，
  而 2026-10-25 当时 app 版本为 **0.1.155**（core/app 是两条独立计数线，无法互相反查）。
- 根因：两处都取 `env!("CARGO_PKG_VERSION")`（core crate 版本，与 app 版本是两条独立计数线）：
  `crates/pony-agent-core/src/agent/otel/mod.rs:554`、`crates/pony-agent-core/src/agent/ponysentry/config.rs:42`；
  `release.yml` 只烧 `PONY_RELEASE_BUILD` / token，未注入 `PONY_OTEL_SERVICE_VERSION` / `APP_RELEASE`。
- 影响：按版本归因的排查、"这个回归从哪个版本开始"、发布后对照全部指向错误版本；跨 trace/issue 两条轨
  也无法与 GitHub Release 对齐。

### P0-2 `governance.decide` 缺 `langfuse.session.id`
- 证据：9 个 span 只有 **8** 个命中会话过滤（Tempo `matched=8`）；Langfuse 只有 8 个 observation，
  缺的正是 `governance.decide`（`pony.trace.route=system`）。
- 根因：`start_governance_decide_span()` 只写 `gen_ai.operation.name`，未调用已有的
  `push_active_session_attribute`（`otel/mod.rs:1667` vs 工具 span 的 `otel/mod.rs:1739` + `:2264`）。
- 影响：配额/治理决策在 Langfuse 完全不可见；一旦发生 `decision=block`，语义轨只能看到 turn 变红，
  看不到"为什么被拦"；会话级过滤也漏 1/9 的 span。

### P0-3 思考 token / 缓存命中 token 没有上报出口
- 证据：`TokenUsage` 有 `reasoning_tokens` / `cache_hit_input_tokens`（`provider/mod.rs:307-315`），
  但 `LlmSpanGuard::record_usage` 只写 input/output（`otel/mod.rs:1582`），`llm.chat` 属性常量表中
  没有任何 reasoning/cache 键。
- 影响：本会话模型为 high 思考档，`llm.chat` 输出仅 26–349 tokens；思考 token 与缓存命中既不进 Langfuse
  usage，也不进根 span 聚合 ⇒ 用量/成本口径系统性偏小，"思考档位 → 成本"的因果关系不可观测。

### P1-4 `gen_ai.system` 取了用户可配置的 provider 显示名
- 证据：同一产品同一协议，桌面 `PonyLlm`、CLI `ponyllm`（`provider.name()`，`turn_flow.rs:2390`）。
- 影响：不是 OTel/Langfuse 语义枚举，跨设备/跨用户不可聚合，按 provider 分组的看板会碎成 N 份；
  与 ADR 0047 决策 4「`gen_ai.system`（provider kind）」的措辞也不一致。

### P1-5 Langfuse 成本恒 0
- 证据：本 trace `totalCost=0`，无 `gen_ai.cost.usd`；Langfuse managed pricing 无 `gemini-3.8-flash-high`
  （带 `-high` 推理档后缀，即使有 base 价也匹配不上）；本仓 `cost.rs` 对虚构型号按「宁缺勿错」故意不计价。
- 影响：ADR 0047 承诺的"多轮会话成本"在当前网关型号上不可用（不是 0 元，是"无价"被显示成 0）。

### P1-6 语义轨只有"时序树"，没有"思考树"内容
- 证据：Langfuse GENERATION `input=null` / `output=null`，TOOL 无入参/结果；客户端只报结构属性。
- 影响：与 ADR 0047 背景段"思考树、工具调用透视"的表述有落差。属隐私取舍（默认不采内容），
  但文档需要把口径改成"结构树"，或提供 opt-in 内容采集（脱敏 + 显式开关）。

### P2-7 `phase=completed` 与 `outcome=error` / `status=error` 可同时成立
- 证据：24h 内两条开发态 trace（`504fef21…`、`415411dd…`）根 span `phase=completed` + `outcome=error` +
  `STATUS_CODE_ERROR(turn_error: tool_failed)`：工具失败被 PA-154 聚合为根错误，但 turn 本身正常收尾
  （`turn_stream.rs:2770` 写 completed；`otel/mod.rs:1467` 写 error）。
- 影响：按 `agent.turn.phase` 算成功率会把失败算成功；按 root status 算错误率会把"模型已自愈的工具失败"
  算成错误。两个口径互相矛盾，需要新增 `agent.turn.degraded` 之类的显式降级标记。

### P2-8 trace 名在不同宿主形态不一
- 证据：桌面 `langfuse.trace.name=1793067274939`（数字 turn id）；CLI `turn-b0a7f931-…`（uuid）。
- 影响：Langfuse 列表不可读、不可按名检索/对比。

### P2-9 无用户归因
- 证据：`langfuse.user.id` 仅由 `PONY_OTEL_USER_ID` 注入，本 trace `userId=null`。
- 影响：无法回答"某个用户/某台设备的失败率与成本"。

### P3-10 语义属性补全项（非阻断）
- `llm.chat` 缺请求参数（temperature / max_tokens）、首 token 时延（TTFT）、`gen_ai.response.model`；
- `tool.call` 缺 `gen_ai.tool.call.id`（无法与模型返回的 tool_call 对账）；
- `governance.decide` 只进 Tempo，不进语义轨（见 P0-2）。

### P3-11 声明 note 未登记 ADR 0050 的配置入口收敛
- 证据：`docs/observability.yaml` 的 `reviewed_at=2026-10-26`，note 最后一条为 wave-46；2026-10-27 的
  ADR 0050（删除 OTel 端点/Token/采样率与成本预算的桌面 UI 入口）未在 note 留归属登记。
- 影响：四维归属不变（不阻断），但"上报配置只剩开关、其余只能靠 env/构建期烧录"这一事实在声明里不可见，
  下一位审计者会重新发现一次。建议补一行归属登记并更新 `reviewed_at`。

### P3-12 开发态流量进入生产 collector
- 证据：24h 内 3 条 trace 中 2 条为 `deployment.environment=development`、core 0.1.129 的 CLI 开发运行。
- 影响：`desktop-client` 维度的错误率会被开发态工具失败稀释（本窗口错误率 2/3）；看板/告警应默认
  `deployment.environment="production"` 过滤，或 dev 走独立端点。

## 5. 优化建议（可直接立项）

| # | 动作 | 验收判据 | 关联 |
| :--- | :--- | :--- | :--- |
| O1 | 版本单一来源：宿主把 `app.package_info().version` 传进 `OtelConfig`（或 release.yml 注入 `PONY_OTEL_SERVICE_VERSION`）；`APP_RELEASE` 同步注入 | 新 trace 的 `service.version` == `package.json` 版本；PonySentry `last_release` 同步；Langfuse `version` 正确 | P0-1 |
| O2 | `start_governance_decide_span` 追加 `push_active_session_attribute` | Tempo 会话过滤 9/9；Langfuse 出现 governance observation；block 场景可归因 | P0-2 |
| O3 | `LlmSpanGuard` 增 `reasoning_tokens` / `cached_input_tokens`（有值才写），根 span 同步聚合 | Langfuse GENERATION usage 明细含思考/缓存；high 档会话输出与思考分离 | P0-3 |
| O4 | `gen_ai.system` 改协议/厂商枚举；provider 显示名移到 `gen_ai.provider.name` | 跨设备同一 provider 值一致；Langfuse 按 system 分组收敛 | P1-4 |
| O5 | 成本：Langfuse 侧注册网关模型价（含 `-high` 匹配规则），或客户端写 `langfuse.observation.cost_details` | `totalCost > 0`；缺价时显式标注"未计价"而不是 0 | P1-5 |
| O6 | 文档口径：ADR 0047 / `agent-trace-pipeline.md` 把"思考树"改为"结构树"；如需内容则加 opt-in 采集 | 文档与实际一致；采集开关默认关且脱敏 | P1-6 |
| O7 | 失败语义拆两键：`phase` 只述生命周期，新增 `agent.turn.degraded=true` 表达"有子失败但已收尾" | 成功率/错误率两个看板不再互相打架 | P2-7 |
| O8 | `langfuse.trace.name` 统一为 `pony-agent#turn-<seq>`（可选拼会话标题）；`langfuse.user.id` 默认用哈希设备 id | Langfuse 列表可读、可按用户钻取 | P2-8/9 |

## 6. 无法判定（UNKNOWN，不判 PASS）

- **本会话完整性**：只能看到 1 个 turn 的 trace；设备侧若在导出前关闭上报/进程被强杀，缺失的 turn
  在服务端不可见，需以本地 session DB 对账才能判定"零丢失"。
- **桌面端本地日志**：本机无该设备 app_data，无法核验 `<app_data_dir>/logs/pony-agent.log` 的实际轮转与内容。
- **故障场景**：本会话无错误，issues 维度只验证了"管道有数据"，未验证"本次会话的错误必达"。

## 7. 复跑命令

```bash
ponygo observe audit --level ob1 --json
bash ~/.agents/skills/observability-onboarding/scripts/check-declaration.sh /home/dm/pony-agent
bash ~/.agents/skills/observability-onboarding/scripts/audit-diff.sh --base origin/main
# 本会话 trace（Tempo）
# { resource.client.service.name = "pony-agent" && span.langfuse.session.id = "pa_01a17b23-1e98-7a53-a2bb-15d116495edb" }
```

## 8. 修复闭环收据（wave-51，ADR 0051）

| 编号 | 修复内容 | 落点 | 机器收据 |
| :--- | :--- | :--- | :--- |
| P0-1 | 版本单一来源：`init_with_service_version` 三级优先 + 注入合法化；`PonySentryConfig::with_release`；Tauri 注入 `package_info().version` | `otel/mod.rs`、`ponysentry/config.rs`、`src-tauri/src/lib.rs` | `crates/pony-agent-core/tests/observability_reporting_fixes_test.rs` C1 四例（含脏值对抗）PASS；`cargo check -p pony-agent` EXIT 0 |
| P0-2 | `governance.decide` 补 `langfuse.session.id`（复用 `push_active_session_attribute`） | `otel/mod.rs::start_governance_decide_span` | C2 两例（活跃 turn 带键 / 无 turn 缺键，均解码 span 自身属性）PASS |
| P0-3 | 思考/缓存 token 出口：semconv 键 `gen_ai.usage.reasoning.output_tokens` / `cache_read.input_tokens`，子 span + 根整轮聚合，`u64→i64` 饱和，cache 仅 OpenAI 协议族 | `otel/mod.rs`、`turn_flow.rs` | C3 五例（值级/缺键/0 值/饱和/根聚合 Σ=10,7）PASS；`--lib` 1175 passed 零新增失败 |
| — | 测试缺陷 TD-1：`l2at_concurrent_init` Barrier 参与方不匹配死锁（绿相发现，Test Agent 修复） | 冻结测试 `:1019-1024` | 修复后 `11 passed`；修复缘由与红相掩盖原因登记在 `.dev-team/l2at-done-wave51.json` |

**门禁**：`audit-diff.sh --base 090df55` EXIT 0；`check-declaration.sh` PASS exit 0（四维不变）；
`npm run version:check` EXIT 0；`cargo test --test observability_reporting_fixes_test` 11 passed。

**遗留（不在本波范围）**：P1-4（`gen_ai.system` 显示名）、P1-5（Langfuse 成本恒 0）、P1-6（思考树内容）、
P2-7~P3-12 仍按 §5 建议表排期；另登记一条测试残留风险：`l2at_concurrent_init` 若实现回退为 panic，
主线程可能 hang 而非快速失败（根治方案已在 `l2at-done-wave51.json.residual_risk_note`）。

