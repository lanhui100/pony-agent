# ADR 0052：可观测性语义轨补全（成本明细、未计价标记、枚举收敛、降级标记、可读 trace 名、工具调用 id）

Status: implemented

## 背景

第二轮真实会话审计（[observability-audit-2026-10-27b](../audit/observability-audit-2026-10-27b.md)，
会话 `pa_01a17c85`、trace `eed9ba47`，41 span）在 wave-51 三项修正已生效的前提下，暴露六处口径问题：

1. **成本恒 0**：Langfuse GENERATION `costDetails={}`、`calculatedTotalCost=0`；本仓 `cost.rs` 不收录
   网关型号 `workbuddy/deepseek-v4.1-flash` ⇒ 客户端不发任何成本属性。且「0 元」与「未计价」不可区分。
2. **`gen_ai.system` 取了展示名**：20/20 `llm.chat` 为 `PonyLlm`（桌面品牌名），CLI 为 `ponyllm`，
   同一协议跨宿主不可聚合。
3. **失败语义自相矛盾**：`phase=completed` 与 `outcome=error` / root `STATUS_CODE_ERROR` 同时成立
   （工具失败被聚合为根错误），成功率与错误率两个口径互相打架。
4. **trace 名不可读**：`langfuse.trace.name=1793090889676`（数字 turn id）。
5. **工具调用无法对账**：`tool.call` 19/19 缺 `gen_ai.tool.call.id`。

## 实测证据（决定方案的关键实验，全部只读/探针）

在集群内 collector 直投合成 OTLP span（打 `pony.probe=true`）后比对 Langfuse：

| 探针 | 观测结果 | 结论 |
| :--- | :--- | :--- |
| `usage-probe-a`（仅 semconv 键） | `usageDetails={input:200, output:100, input_cached_tokens:800, reasoning.output_tokens:50, total:1150}` | wave-51 的思考/缓存出口 **在语义轨已被正确归并**；上一轮报告「Langfuse 不含思考/缓存」的结论作废 |
| `usage-probe-b`（自带 `langfuse.observation.usage_details` JSON） | `usageDetails` 原样落库但 `usage.total=1950`（缓存被重复计入） | **禁止**客户端自造 usage_details，收益为负 |
| `cost-probe`（`langfuse.observation.cost_details` JSON） | `costDetails={input:0.001,output:0.002,total:0.003}`、`calculatedTotalCost=0.003` | 客户端发 cost_details 是**可行且唯一**的客户端侧成本通道 |
| 三条探针的 trace 级聚合 | `trace.usage=null`、`trace.totalCost=0`（observation 级均有值） | trace 级聚合属 Langfuse 侧，**不在客户端闭环内**（登记为 infra 待办） |

## 候选方案与取舍

- **成本：注册 Langfuse 托管模型价 / 客户端自算并发 `cost_details`**：托管价依赖 Langfuse 侧配置且对
  网关别名（`workbuddy/*`、`-high` 后缀）匹配脆弱；客户端已有 `cost.rs` 单价表与协议语义。
  **采纳客户端 `cost_details`**（三分量 + 6 位小数），托管价作为可选叠加。
- **未计价标记：静默不发 / 显式 `pony.cost.priced`**：静默会把「无价」显示成 0 元，误导成本决策。
  **采纳显式布尔标记**（有 usage 无成本 ⇒ `false`）。
- **`gen_ai.system`：保留展示名 / 新增 `gen_ai.provider.name` 并在 system 放协议枚举**：
  **采纳枚举 + 展示名分家**，枚举源自既有 `provider.protocol_label()`
  （`openai-completions|openai-responses|anthropic-messages`），映射函数幂等，未知标签 fail-closed 到 `generic`。
- **降级标记：改 `phase` 三态 / 新增 `agent.turn.degraded`**：改 `phase` 会打破既有看板与语义
  （phase 是生命周期，不是质量）。**采纳新增键**：`completed` + 有工具失败 ⇒ `degraded=true`；
  非 `completed` 不出 `degraded`（错误仍由 outcome/status 表达）。
- **API 形态：改 `start_llm_span` 签名 + 给 `LlmSpanInput/ToolSpanInput` 加字段 / 附加式函数**：
  前者使 6 个历史冻结测试 E0061/E0063 编译红，等于解冻验收工件。**采纳附加式**
  （`*_with_provider` / `*_with_call_id` 包装，旧入口等价传 `None`），冻结工件零改动。
- **trace 名：`pony-agent#turn-<seq>` / 拼会话标题**：标题属内容面（默认不采）。**采纳前者**。

## 决策

1. `LlmSpanGuard::record_cost_components(input, output, total)`：有 total 有效值时发
   `langfuse.observation.cost_details`（单行 JSON，键仅 `input/output/total`），并计入 turn 成本聚合；
   `gen_ai.cost.usd` 继续输出（Tempo 面）。
2. `pony.cost.priced`：有 usage 无成本 ⇒ `false`，有成本 ⇒ `true`，两者皆无 ⇒ 不输出。
3. `gen_ai.system` 收敛为协议枚举；展示名迁移到 `gen_ai.provider.name`。
4. `agent.turn.degraded`（bool）+ `agent.turn.tool_failures`（int）：`completed` + 工具失败 ⇒ degraded；
   计数只要 >0 恒输出。
5. `langfuse.trace.name = pony-agent#turn-<sequence>`。
6. `tool.call` 增 `gen_ai.tool.call.id`（模型侧 call id），经 `record_tool_call_span_with_call_id` 接线。

## 后果

- 语义轨首次具备**可区分**的成本口径（有价/无价显式化），成本看板不再把「无价」当「免费」。
  但**网关型号缺价 ⇒ 仍无成本数值**：需产品侧给 `cost.rs` 补网关型号单价，属独立待办。
- 由于本波不采内容，Langfuse 仍是「结构树」而非「思考树」（ADR 0047 的措辞已按此理解）。
- **服务端映射复核（合成探针，非端到端）**：探针经真实 collector→Langfuse 后，trace `name=pony-agent#turn-7`、
  `totalCost=0.003`（随 `cost_details` 打通）、`gen_ai.system=openai` + `provider.name=PonyLlm`、
  root `degraded=true`/`tool_failures=1`、TOOL `call.id` 全部落库（见审计 §6.5）；客户端出口由 T1~T6 覆盖。
  仍遗留：trace 级 **usage** 恒 `null`（Langfuse 侧聚合行为，infra 待办）。
- 冻结测试矩阵：wave-53 新增 T1~T6；wave-51 的 14 例保持绿（本波未改其语义）。

## 关联

- 上游审计：[observability-audit-2026-10-27b](../audit/observability-audit-2026-10-27b.md)
- 前序 ADR：[0051](0051-observability-version-session-and-thinking-token-fixes.md)、[0047](0047-agent-trace-otel-langfuse-migration.md)
- 契约矩阵：`.dev-team/contract-matrix-wave53.md`
