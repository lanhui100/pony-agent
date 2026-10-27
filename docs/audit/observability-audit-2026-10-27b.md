# 可观测性四维上报审计（2026-10-27 第二轮，真实会话 pa_01a17c85 驱动）

> 口径：声明（`docs/observability.yaml`）× 实测（Tempo / Langfuse / PonySentry / 本仓代码）交叉判定。
> 真实会话：`pa_01a17c85-b27b-729d-bcc3-94efa28f7e7c`（桌面端），trace `eed9ba479418dca23cc0a272d70bbabf`，
> 起于 2026-10-27 16:48:09 CST，根 span 98.47s，41 个 span。
> 端点事实来源：`~/.agents/skills/observability-onboarding/references/contract.json`（contract_version 1.2.0）。
> 采集时间 2026-10-27 17:10–17:40 CST；全部只读（除 3 发合成 OTLP 探针，打 `pony.probe=true`）。

## 1. 机器门禁收据

| 命令 | 结果 |
| :--- | :--- |
| `ponygo observe audit --level ob1 --json` | EXIT 0（`current=ob2` / `target=ob1`） |
| `bash ~/.agents/skills/observability-onboarding/scripts/check-declaration.sh /home/dm/pony-agent` | **EXIT 0**：tracing `T1~T5` PASS、issues `I1~I5` PASS、metrics/logs 声明与现状一致 |
| `bash ~/.agents/skills/observability-onboarding/scripts/audit-diff.sh --base origin/main` | EXIT 0（**收口提交后**复跑；该门禁只覆盖已提交内容，故不计入改动前基线） |

结论：**四维归属声明无造假、无漏维**；`metrics`/`logs` 的 N-A 仍是结构性理由。

## 2. 会话事实（Tempo × Langfuse 对账）

- 结构：`agent.turn` ×1 + `llm.chat` ×20 + `tool.call` ×19 + `governance.decide` ×1 = **41 span**。
- 归因 resource：`service.name=desktop-client`、`client.service.name=pony-agent`、`service.version=0.1.163`
  （== `package.json`）、`deployment.environment=production`、`ingest.client.kind=device`、`ingest.source=public`。
- 用量：根 span 358,633 in / 4,542 out / 2,075 reasoning / 327,808 cache_read，
  **与 20 个 `llm.chat` 逐项求和完全相等**（输入、输出、思考、缓存四条线均一致）。
- Langfuse：41 个 observation（SPAN ×2 = root+governance、GENERATION ×20、TOOL ×19），
  `version=0.1.163`；GENERATION `usageDetails` = `input 1416`（已扣缓存）、`output 1359`、
  `input_cached_tokens 30336`、`reasoning.output_tokens 719`、`total 33830`。
- 错误面：19 个 `tool.call` 中 2 个 `status=failed`（`Run`，退出码 255 / 9009），
  根 span `phase=completed` + `outcome=error` + `STATUS_CODE_ERROR(turn_error: tool_failed)`；
  Langfuse root SPAN `level=ERROR`，两个失败 TOOL 亦为 `ERROR`。

## 3. 上轮（wave-51）修复的真实数据验证

| 上轮项 | 实测证据 | 判定 |
| :--- | :--- | :--- |
| P0-1 版本单一来源 | `service.version=0.1.163` == 发布线；Langfuse trace `version=0.1.163` | **FIXED** |
| P0-2 `governance.decide` 会话归属 | 该 span 带 `langfuse.session.id` 且 `pony.trace.route=genai`；Langfuse observation 41/41 命中会话过滤 | **FIXED** |
| P0-3 思考/缓存 token 出口 | 20/20 `llm.chat` 均带两键；根 span 聚合 == Σ；Langfuse `usageDetails` 已含 `input_cached_tokens` / `reasoning.output_tokens` | **FIXED** |
| Langfuse `release` 同步 | trace `release=null`（`service.version` 已正确但 release 属性未发） | **PARTIAL** |

> 修正上一轮报告结论：`docs/audit/observability-audit-2026-10-27.md` P0-3 所说「Langfuse usage 不含思考/缓存」
> **不成立**——`usageDetails` 已含；本轮真正的缺口在**成本**与**trace 级聚合**（§4 P0-2 / P0-1）。

## 4. 错漏清单（按优先级，均带实测证据）

### P0-1 trace 级 `usage` 恒空（infra 面；`totalCost` 已随 wave-53 修复可聚合）
- 证据：本 trace `usage=null`，而 41 个 observation 的 usage 均有值；合成探针（`usage-probe-a/b`、
  `cost-probe`）复现同一现象。**`totalCost` 另论**：wave-53 探针（携带 `cost_details`）落地后
  trace `totalCost=0.003`（§6.5），即 trace 成本聚合本身可用——此前恒 0 是因客户端从未发送任何成本量。
- 影响：按 trace/会话汇总 **token** 的面板仍恒空，只能下钻 observation；成本聚合已在 wave-53 打通
  （前提：有价模型）。
- 归属：usage 聚合属 Langfuse 侧（v3 镜像 + OTel 直入路径），非客户端可修 → 需 infra 单独立项验证/升级。

### P0-2 成本恒 0，且「未计价」被显示成「0 元」
- 证据：GENERATION `costDetails={}`、`calculatedTotalCost=0`、trace `totalCost=0`；
  本仓 `cost.rs` 对网关型号 `workbuddy/deepseek-v4.1-flash` 不收录 ⇒ 客户端 `record_cost(None)` ⇒ 不发任何成本属性。
- 反证（可修）：合成探针带 `langfuse.observation.cost_details={"input":0.001,"output":0.002,"total":0.003}`
  → Langfuse `costDetails` 与 `calculatedTotalCost=0.003` **正确落库**。
- 影响：ADR 0047 承诺的"多轮会话成本"不可用；且 0 与"无价"不可区分，会误导容量/预算决策。

### P1-3 `gen_ai.system` 取的是展示名，不是协议枚举
- 证据：20/20 `llm.chat` 的 `gen_ai.system=PonyLlm`（`provider.name()`，桌面端品牌名）；
  同一协议在 CLI 为 `ponyllm`。可用枚举源已存在：`provider.protocol_label()` =
  `openai-completions|openai-responses|anthropic-messages`。
- 影响：跨设备/跨宿主不可聚合，按 provider 分组的看板会碎成 N 份；与 ADR 0047 决策 4 措辞不一致。

### P1-4 失败语义自相矛盾：`phase=completed` 与 `outcome=error` 同时成立
- 证据：`agent.turn` 同时有 `phase=completed`、`outcome=error`、`STATUS_CODE_ERROR(turn_error: tool_failed)`；
  根因是两个 `Run` 工具失败被 PA-154 聚合为根错误，而 turn 本身正常收尾。
- 影响：按 `phase` 算成功率会把失败算成功，按 root status 算错误率会把"模型已自愈的工具失败"算成错误；
  两个口径互相打架且无第三态可表达。

### P2-5 `langfuse.trace.name` 为纯数字 turn id
- 证据：`langfuse.trace.name=1793090889676`。影响 Langfuse 列表不可读、不可按名检索。

### P2-6 无用户归因
- 证据：`langfuse.user.id` 缺省（仅 `PONY_OTEL_USER_ID` 注入），trace `userId=null`。

### P3-7 语义属性补全项（非阻断）
- `tool.call` 19/19 缺 `gen_ai.tool.call.id`（无法与模型返回的 tool_call 对账）；
- `llm.chat` 缺 `gen_ai.response.model`、TTFT、请求参数（temperature / max_tokens）；
- Langfuse 把思考 token 存成自定义键 `usageDetails["reasoning.output_tokens"]`（非 `output_reasoning_tokens`），
  其托管计价不认这一键——随 P0-2 的 `cost_details` 自算成本即可绕开。

### P3-8 `langfuse.release` 未发
- 证据：trace `release=null`；`service.version` 正确但 release 属语义轨的独立字段。

## 5. 结论矩阵

| 维度 | 声明 | 实测 | 判定 |
| :--- | :--- | :--- | :--- |
| tracing | applicable=true | 41 span、四类用量一致、归因齐全 | **PASS**（成本/trace 聚合有缺口，见 P0-1/P0-2） |
| issues | applicable=true | 管道可达（公网 401 鉴权正常）、无伪造；本会话仅工具失败非崩溃 | **PASS**（release 待补，P3-8） |
| metrics | N-A | 无伪造 `/metrics` | **一致** |
| logs | N-A | 仅本地取证面 | **一致** |

## 6. wave-53 实施项（dev team，冻结于 `.dev-team/contract-matrix-wave53.md`）

| # | 动作 | 验收判据 | 关联 |
| :--- | :--- | :--- | :--- |
| C1 | `LlmSpanGuard` 增 `record_cost_components`，按 `cost.rs` 的 input/output/total 发 `langfuse.observation.cost_details`（JSON） | 有价模型 GENERATION `costDetails` 三分量齐全，`calculatedTotalCost>0`；无价不发 | P0-2 |
| C2 | 无价显式标记：记录了 usage 却无成本 ⇒ `pony.cost.priced=false` | 无价会话可区分"0 元"与"未计价" | P0-2 |
| C3 | `gen_ai.system` 收敛为协议枚举（`openai`/`anthropic`/`google`/`generic`），展示名迁 `gen_ai.provider.name` | 同一协议跨宿主取值一致；展示名仍可查 | P1-3 |
| C4 | 新增 `agent.turn.degraded`（bool）+ `agent.turn.tool_failures`（int）：`phase=completed` 且有工具失败 ⇒ degraded=true | 失败工具已收尾的 turn 不再只靠 root status 表达 | P1-4 |
| C5 | `langfuse.trace.name` 改为可读格式 `pony-agent#turn-<sequence>` | Langfuse 列表可读可检索 | P2-5 |
| C6 | `tool.call` 补 `gen_ai.tool.call.id`（模型返回的 call id） | tool 观测可与模型请求对账 | P3-7 |

### 6.5 服务端映射探针收据（wave-53 属性形态在真实 Langfuse 的落库验证）

> **证据等级说明（重要）**：本节是**合成 OTLP 探针**（`pony.probe=true`）经真实 collector → Langfuse 的
> **服务端映射**证明——回答「这些键到达服务端后是否被正确解析」，**不回答**「客户端真实运行是否发出这些键」；
> 后者由 wave-53 冻结测试 T1~T6 的 wire 级解码覆盖，真实桌面会话复跑留待下一轮上传后核对。
> 探针不含任何客户端二进制路径，故本表**不得**当作端到端交付收据。
合成探针 `dddddddd…`（`pony.probe=true`，只打标不污染设备维度）经集群内 collector → Langfuse：

| 断言 | 实测 |
| :--- | :--- |
| 可读 trace 名 | `name = pony-agent#turn-7` |
| 成本三分量 | GENERATION `costDetails={input:0.001,output:0.002,total:0.003}`、`calculatedTotalCost=0.003` |
| trace 级成本聚合 | `trace.totalCost=0.003`（此前无成本量时为 0） |
| 枚举与展示名分家 | `gen_ai.system=openai` + `gen_ai.provider.name=PonyLlm` |
| 未计价标记 | `pony.cost.priced=true`（无价分支由 T2 分子断言覆盖） |
| 工具对账 | TOOL observation `gen_ai.tool.call.id=call_probe_1` |
| 降级标记 | root `agent.turn.degraded=true`、`agent.turn.tool_failures=1` |
| 遗留 | `trace.usage` 仍为 `null`（P0-1，infra 面） |

**不在本波范围**：P0-1（infra 侧 trace 聚合）、P2-6（用户归因需产品决策）、TTFT/请求参数采集（隐私口径待定）、
内容级 input/output（隐私默认不采）。

## 7. 复跑命令

```bash
ponygo observe audit --level ob1 --json
bash ~/.agents/skills/observability-onboarding/scripts/check-declaration.sh /home/dm/pony-agent
bash ~/.agents/skills/observability-onboarding/scripts/audit-diff.sh --base origin/main
# 本轮会话（Tempo）：{ resource.client.service.name = "pony-agent" && span.langfuse.session.id = "pa_01a17c85-b27b-729d-bcc3-94efa28f7e7c" }
```
