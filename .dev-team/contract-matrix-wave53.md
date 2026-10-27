# Wave-53 契约矩阵（可观测性四维 · 第二轮修复）

> 驱动事实：`docs/audit/observability-audit-2026-10-27b.md`（真实会话 pa_01a17c85 / trace eed9ba47）。
> 冻结即只读：本文件由 Lead 冻结，实施期间任何变更走契约演进门禁（新波次）。
> 唯一实现文件：`crates/pony-agent-core/src/agent/otel/mod.rs` + 3 个运行时调用点。

## 0. 分级与门禁

- 分级：**B 级**（跨文件、触及对外语义属性与 LLM 调用链；`otel/mod.rs` 命中关键词黑名单 `net/io`）。
- 可观测性动线：**命中**（改的就是上报面）⇒ 验收追加 `audit-diff.sh` + `check-declaration.sh`。
- 回归基线：`.dev-team/regression-baseline-wave53.json`（改动前 `cargo test -p pony-agent-core --lib` 与
  `--test observability_reporting_fixes_test` 的机器收据）。

## 1. 冻结契约（实施不得改语义，只许实现）

### C1 `cost_details`（Langfuse 语义轨成本）
- 新常量 `ATTR_LANGFUSE_OBSERVATION_COST_DETAILS = "langfuse.observation.cost_details"`。
- 新方法 `LlmSpanGuard::record_cost_components(&mut self, input_cost_usd: Option<f64>, output_cost_usd: Option<f64>, total_cost_usd: Option<f64>)`：
  - 仅当 `total_cost_usd` 为 `Some(v)` 且 `v.is_finite() && v >= 0.0` 才输出属性；值为**单行 JSON 字符串**，
    键只允许 `input` / `output` / `total`，各自 `Some` 才出现；数字保留 6 位小数（`{:.6}`）。
  - 与既有 `record_cost(Some(total))` **等价地**计入该 turn 的成本聚合（根 span `gen_ai.cost.usd` 不得因此漏算或重复计）。
  - 既有 `gen_ai.cost.usd` 属性照旧输出（Tempo 面）。
- 调用点：`turn_flow.rs` 中 `estimate_cost_with_protocol` 成功处，改为同时给出三分量。

### C2 未计价显式标记
- 新常量 `ATTR_PONY_COST_PRICED = "pony.cost.priced"`（Bool）。
- `llm.chat` span 结束时：记录过 usage（input 或 output 至少一个 `Some`）且**未**记录成本 ⇒ `false`；
  记录过成本 ⇒ `true`；两条都没记录 ⇒ 不输出该键。

### C3 `gen_ai.system` 枚举化
- 新常量 `ATTR_GEN_AI_PROVIDER_NAME = "gen_ai.provider.name"`。
- 新纯函数 `pub fn gen_ai_system_enum(protocol_label: &str) -> &'static str`：
  `openai*` → `openai`；`anthropic*` → `anthropic`；`gemini*` / `google*` → `google`；其余 → `generic`。
- **契约演进门禁（2026-10-27 17:35，Lead 裁决 = 方案 a：附加式 API，冻结工件零改动）**：
  既有 6 个历史冻结测试用 3 参 `start_llm_span` 与 `LlmSpanInput/ToolSpanInput` 结构体字面量构造，
  新增字段/改签名会使其 E0061/E0063 编译红 ⇒ **禁止改既有签名与结构体字段**。改为：
  - `start_llm_span(system, model, operation)` **保持不变**；新增
    `pub fn start_llm_span_with_provider(system: &str, provider_name: Option<&str>, model: Option<&str>, operation: &str) -> LlmSpanGuard`。
  - `pub fn llm_span_attributes(input: &LlmSpanInput)` **保持不变**；新增
    `pub fn llm_span_attributes_with_provider(input: &LlmSpanInput, provider_name: Option<&str>) -> Vec<(String, AttrValue)>`，
    旧函数等价于传 `None` 委托新函数。
- 属性语义：`gen_ai.system = gen_ai_system_enum(provider_kind)`（入参也过映射，保证幂等，`"openai"→"openai"`）；
  `provider_name` 非空 ⇒ 输出 `gen_ai.provider.name`（展示名，如 `PonyLlm`）。
- 调用点（`turn_flow.rs` 4 处）：改用 `start_llm_span_with_provider(provider.protocol_label(), Some(provider.name()), Some(model), "chat")`。

### C4 失败语义拆键（degraded）
- 新常量 `ATTR_AGENT_TURN_DEGRADED = "agent.turn.degraded"`（Bool）、
  `ATTR_AGENT_TURN_TOOL_FAILURES = "agent.turn.tool_failures"`（I64）。
- `record_tool_call_span`：`status != "success"` ⇒ 累加该活跃 turn 的工具失败计数。
- 根 span 结束（`TurnTraceGuard` Drop/end）：`agent.turn.tool_failures > 0` ⇒ 输出计数；
  且当 `phase == "completed"` ⇒ 追加 `agent.turn.degraded = true`。
  `phase != "completed"`（含 error/aborted）⇒ **不输出** `degraded`（错误语义仍由 outcome/status 表达）。
- 既有 `phase` / `outcome` / `set_status` 语义**零变更**。

### C5 可读 trace 名
- `turn_span_attributes` 的 `langfuse.trace.name` = `pony-agent#turn-<sequence>`（十进制，无零填充）。
- `langfuse.session.id` / `agent.turn.id` / `agent.turn.sequence` 属性不变。

### C6 tool call id（同样适用附加式 API 裁决）
- `ToolSpanInput` **不新增字段**；新增
  `pub fn tool_span_attributes_with_call_id(input: &ToolSpanInput, call_id: Option<&str>) -> Vec<(String, AttrValue)>`
  （旧 `tool_span_attributes` 委托 `None`）与
  `pub fn record_tool_call_span_with_call_id(input: ToolSpanInput, call_id: Option<String>)`
  （旧 `record_tool_call_span` 委托 `None`）。
- 非空 ⇒ 输出 `gen_ai.tool.call.id`（新常量同名）。
- 2 个调用点（`runtime/turn_sync.rs`、`runtime/turn_stream.rs`）改用 `_with_call_id` 版本，传当前工具调用的模型侧 id
  （`current_tool_call.call_id`）。

## 2. 验收测试（Test Agent 冻结，Executor 只读）

文件：`crates/pony-agent-core/tests/observability_reporting_wave53_test.rs`（不得并入既有冻结文件）。

| 用例 | 断言（黑盒/线级） |
| :--- | :--- |
| T1 有价成本三分量 | `cost_details` 为合法 JSON，`input/output/total` 数值与入参一致（±1e-9）；`gen_ai.cost.usd` 同时存在 |
| T2 无价 vs 有价 | 只记 usage ⇒ `pony.cost.priced=false` 且无 `cost_details`；记成本 ⇒ `true`；两者都无 ⇒ 键不存在 |
| T3 枚举收敛 | `openai-completions→openai`、`openai-responses→openai`、`anthropic-messages→anthropic`、`gemini-2.5→google`、`PonyLlm→generic`；`gen_ai.provider.name` 保留展示名 |
| T4 degraded | completed+1 失败 ⇒ `degraded=true` + `tool_failures=1`；全成功 ⇒ 两键均不存在；error ⇒ 有计数无 degraded |
| T5 trace 名 | `sequence=7` ⇒ `langfuse.trace.name == "pony-agent#turn-7"` |
| T6 tool call id | `call_id=Some("call_1")` ⇒ 属性存在且相等；`None` ⇒ 键不存在 |

## 3. 写域（正交，禁止越界）

| 角色 | write_scopes |
| :--- | :--- |
| protocol-agent | `crates/pony-agent-core/src/agent/otel/mod.rs`（仅常量 + 签名 + `unimplemented!()` 桩） |
| test-agent | `crates/pony-agent-core/tests/observability_reporting_wave53_test.rs`、`.dev-team/l2at-done-wave53.json` |
| executor | `crates/pony-agent-core/src/agent/otel/mod.rs`、`crates/pony-agent-core/src/agent/turn_flow.rs`、`crates/pony-agent-core/src/agent/runtime/turn_sync.rs`、`crates/pony-agent-core/src/agent/runtime/turn_stream.rs` |
| lead | `docs/`、`.dev-team/`、`docs/observability.yaml` |

## 4. 机器门禁（Exit Code 必须为 0）

1. `cargo test -p pony-agent-core --test observability_reporting_wave53_test`
2. `cargo test -p pony-agent-core --test observability_reporting_fixes_test`（wave-51 冻结红相不得回归）
3. `cargo test -p pony-agent-core --lib`（基线对照，零新增失败）
4. `npm run version:check`
5. `bash ~/.agents/skills/observability-onboarding/scripts/check-declaration.sh /home/dm/pony-agent`
6. `bash ~/.agents/skills/observability-onboarding/scripts/audit-diff.sh --base origin/main`
