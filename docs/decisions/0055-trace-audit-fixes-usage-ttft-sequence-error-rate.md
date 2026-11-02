# ADR 0055：真实 trace 审计驱动的四项语义缺陷与三项可观测性优化

Status: implemented

## 背景

对桌面端最新真实上传 trace 的审计（trace `f47fd028ca876ed437fcf9616bd7d653`，
session `pa_01a1974d-1e7a-7d15-8512-f9f281f07215`，release 0.1.167，15 次 LLM 调用 /
14 次工具调用 / 3 次工具失败 / 总耗时 207s），并以另两条生产 trace
（`a1751e1b2e0b71f2b494d086861c2b1d`、`a66af1c45e03544a33a0ef5fb6d991cd`）做横向对照，
确认四处**口径错误**与三处可优化点。全部结论均有 trace 侧数据或集群内探针为物理凭证。

### 实测证据

| 编号 | 现象 | 证据 |
| :--- | :--- | :--- |
| P0-1 | `usage.total` 被抬高约 100 万 | 45 个 `llm.chat` 全部满足 `total = input + output + reasoning + 1,000,000`（例：`input 7455 + output 181 + reasoning 39 + context_window 1,000,000 = 1,007,675`） |
| P0-2 | 纯工具调用步骤无 `llm.call.ttft_ms` | 15 次调用中 6 次缺 ttft，且 `generation_ms == duration_ms`；对应步骤 reasoning≈0（只有 tool_calls 增量）⇒ 这些步骤速度被算成 10~20 tok/s，整轮 31.8 tok/s |
| P0-3 | 所有 trace 同名 `pony-agent#turn-0@…` | 两个 turn 入口（`turn_stream.rs` / `turn_sync.rs`）均硬编码 `sequence = 0`；三日内 3 条生产 trace 的 `agent.turn.sequence` 全为 0 |
| P0-4 | 错误率虚高 | 3/3 生产 trace 均 `phase=completed` + `outcome=error`（`turn_error: tool_failed`），而失败是 `grep` 无匹配的 exit 1 与 Windows 9009 |
| P1-5 | 同一轮第 7 次调用前缀缓存全失效 | 两条不同会话均在 index 6 出现 `cache_read=0`，ttft 从 ~7s 跳到 17.5s（`a1751e1b` 为 4.8s→33s 同类断崖） |
| P1-6 | Windows 首个命令即 9009 | `f47fd028` 与 `a1751e1b` 的首个 `Run` 均为 `non_zero_exit: 9009`（cmd「命令不存在」） |
| P2-7 | 成本恒 0 / release 空 | `pony.cost.priced=false`、`totalCost=0`（`sense/deepseek-flash` 无单价）；`version=0.1.167` 而 `release=null` |

### Langfuse 摄入口径探针（合成 OTLP，12 例，只读）

在集群内以 `pony.probe` 之外的独立探针 trace 直投 Langfuse OTLP 端点，读回
`GET /api/public/traces/{id}`、`/observations/{id}` 比对：

| 探针 | 输入 | 落库结果 |
| :--- | :--- | :--- |
| 对照组 | `gen_ai.usage.context_window=1000000` + input 1000 + output 200 | `usageDetails={input,output,context_window}`、`total=1,001,200` |
| 改键名 | `llm.context.window_tokens=1000000` + `llm.context.ratio=0.01` | `usageDetails={input,output}`、`total=1,200` |
| A 组 | input 1000 + output 200 + `cache_read=800` + `reasoning=50` | `{input:200, output:200, input_cached_tokens:800, reasoning:50}`、`total=1250` |
| C 组 | 仅 `cache_read=800` | `total=1200`（cache **不**重复计，input 被扣减） |
| D 组 | 仅 `reasoning=50` | `total=1250`（reasoning **重复计**，与 semconv「已含于 output」矛盾） |
| release | span `langfuse.release=9.9.9` | `trace.release=9.9.9`；而 resource `service.version` 只填 `trace.version` |

结论：**只有 `gen_ai.usage.` 前缀会被求和进 `usage.total`**；`reasoning` 键在 semconv 键名下
与 Langfuse 聚合口径冲突；`release` 必须由 `langfuse.release` 显式提供。

## 决策

1. **上下文容量/比例迁出用量前缀**（P0-1）：`gen_ai.usage.context_window` →
   `llm.context.window_tokens`；`gen_ai.usage.context_ratio` → `llm.context.ratio`。
2. **思考 token 迁出用量前缀**（P0-1 续）：`gen_ai.usage.reasoning.output_tokens` →
   `llm.reasoning.output_tokens`（语义仍为「已含于 output」，只是不再被 Langfuse 二次求和）。
   `gen_ai.usage.input_tokens|output_tokens|cache_read.input_tokens` **保持 semconv 名**——
   它们是 Langfuse 扣减与计费的唯一依据，探针证明 cache 不会被重复计。
3. **首包时点信号**（P0-2）：新增 `ProviderStreamChunk::StreamStart`（无内容）。
   触发规则：**仅当首个上游事件不携带 text/reasoning 增量时**才发（纯工具调用轮次）；
   首包自带内容时既有 Text/Reasoning 增量已完成时点采集 ⇒ 内容增量顺序契约零变更。
   `openai_sse` 按该规则发出；anthropic 流在首个 SSE 事件发出（其内容分片始终晚于事件）。
   runtime 三个消费者在闭包入口对该变体提前返回 ⇒ **不参与** UI `first_token_latency` 口径
   （UI 语义仍是「首个内容增量」），仅 `turn_flow` 的独立 `on_delta` 用它采集
   `llm.call.ttft_ms`。
4. **真实轮次序号**（P0-3）：新增 `SessionStore::turn_trace_count` +
   `AgentRuntime::session_turn_sequence`；两个 turn 入口改为传会话已登记轮次数（0-based）。
5. **错误语义分层**（P0-4）：新增 `tool_error_kind_is_turn_fatal`——
   `non_zero_exit` 只计 `agent.turn.tool_failures` / `agent.turn.degraded`，
   **不**把根 span 的 `outcome` 与状态码改为 error；未知/缺失种类 fail-closed 仍提升为错误。
   工具子 span 的 `status=ERROR` 与 `agent.tool.error` 不变（失败事实不隐藏）。
6. **缓存命中率派生指标**（P1-5）：`llm.chat` 增 `llm.context.cache_hit_ratio`
   （= `cache_read / input`），把「前缀缓存断崖」变成可查询/可告警的指标。
7. **Windows 9009 纠错提示**（P1-6）：`Run` 的非零退出错误消息追加 Windows 等价写法建议
   （`dir`/`where`/`findstr`/`type`/`set VAR=… &&`/PowerShell），不改 `code`/`kind`/`exitCode`。
8. **release 归因**（P2-7）：根 `agent.turn` span 增 `langfuse.release` = 已解析的
   `service.version`（init 阶段固化，禁止另起口径）。

## 备选方案与取舍

- **自造 `langfuse.observation.usage_details` 修正 total**：ADR 0052 已实测为负收益
  （缓存被重复计入、total 错得更远），本轮探针结论一致，故不采纳。
- **把 `output_tokens` 改为「已扣思考」的值**：能让 `total` 精确等于 prompt+completion，
  但会让 `output_tokens` 与上游 `completion_tokens` 语义分叉，破坏与 provider 的对账口径。
  改为「键名搬出用量前缀」，保留原始真值（探针推荐方案）。
- **纯工具调用的 ttft 用「首个非空 content」近似**：无法取得时点，只能退化为整段耗时，
  即当前 bug 本身，故不采纳。
- **把 `non_zero_exit` 一律视为成功**：会隐藏真实失败（`cargo build` 失败也是非零退出），
  故只做「不提升为整轮错误」，保留工具级失败与降级标记。
- **为 `sense/deepseek-flash` 补单价**：网关别名无公开价，凭空写数会污染成本看板
  （ADR 0052 已登记为产品侧待办），本波不伪造。

## 后果

- `usage.total` 恢复为「真实 token 之和」量级（探针 1,001,200 → 1,200 同构关系）；
  思考 token 仍在 span 属性里（`llm.reasoning.output_tokens`），只是不再污染合计。
- 纯工具调用步骤获得真实 ttft ⇒ per-call 速度与整轮 `turn_weighted` 速度不再系统性偏低
  （同一 trace 按首包口径重算约为 52 tok/s，此前显示 31.8）。
- 多轮会话的 trace 名与 `agent.turn.sequence` 恢复区分度，`pony-agent#turn-<n>` 可用于按名检索。
- 错误率指标恢复单义：「轮次失败」与「命令返回非零」不再混同；
  `agent.turn.degraded=true` 仍是「完成但有工具失败」的唯一表达。
- 每条 `llm.chat` 多一个浮点键 `llm.context.cache_hit_ratio`（有缓存真值且有输入时才输出）。
- 冻结工件变更（**已在 ADR 内显式声明**）：
  - `crates/pony-agent-core/tests/wave54_metric_truth.rs` 的 `KEY_CONTEXT_WINDOW` /
    `KEY_CONTEXT_RATIO` 常量与两条断言文案随键名迁移（断言**语义**不变：仍要求
    「有窗口能力时存在窗口键且 ratio ≈ in/win」）。
  - `crates/pony-agent-core/tests/observability_reporting_fixes_test.rs` 的 `KEY_REASONING`
    随思考键迁移（断言语义不变：值级/缺键/0 值/饱和/根聚合）。
- 未闭环（诚实登记）：
  - `P1-5` 缓存断崖的**上游侧**根因未能证实（客户端侧同一轮内 system/history 前缀字节稳定：
    volatile 段固定在历史之后、`current_date` 只到日；且 `cache_read=0` 同时 ttft 翻倍，
    属整段前缀重算 ⇒ 指向网关上游账号轮换或 provider 缓存过期）。本波只提供
    `llm.context.cache_hit_ratio` 作为可查询证据，根因判定需网关侧日志。
  - 成本仍为 0：`sense/deepseek-flash` 缺单价（产品侧待办，ADR 0052 已登记）。
  - Langfuse trace 级 `usage`/`totalCost` 恒空属集侧聚合行为（infra 待办）。

## 验收（物理收据）

- `cargo check --lib`：**EXIT 0**。
- `cargo test --lib`（全量单元）：**1195 passed / 0 failed**。
- `cargo test --lib adr0055`：**7 passed**（键名前缀约束、错误种类判定 fail-closed、
  `StreamStart` 三种时序契约（纯工具调用必发 / 带内容不发 / 全流至多一次）、release 键契约）。
- `cargo test --lib windows_command_not_found_hint`：**PASS**（仅 Windows+9009 才追加提示）。
- 冻结/既有可观测性矩阵（9 个目标，**74 passed / 0 failed**）：
  `observability_reporting_fixes_test`(14) / `observability_reporting_wave53_test`(7) /
  `otel_trace_migration_test`(23) / `otel_resource_turn_status_test`(6) /
  `otel_user_id_acceptance_test`(2) / `wave49_observability_gaps`(5) /
  `wave4_observability_chain_test`(6) / `wave54_metric_truth`(8) / `ponysentry_manual_upload_test`(3)。
- 可观测性门禁：`check-declaration.sh --declaration docs/observability.yaml` → **PASS (exit=0)**；
  `audit-diff.sh --decl-approved` → **OK，无越界实现、收口点完整**。
- 基线缺陷（**非本波因果**，工作树既有脏改）：`tests/cli_acceptance.rs` 断言
  `SlashCommand::Cost`，而工作树 `src/bin/cli.rs` 已删除该变体 ⇒ `cargo check --tests` 在该目标失败，
  与本波改动无关，登记待处理。

## 关联

- 上游审计：本轮 trace `f47fd028ca876ed437fcf9616bd7d653`（session `pa_01a1974d`）
- 前序 ADR：[0047](0047-agent-trace-otel-langfuse-migration.md)、[0051](0051-observability-version-session-and-thinking-token-fixes.md)、
  [0052](0052-observability-semantic-completeness-fixes.md)、[0053](0053-token-speed-truth-contract.md)
- 探针证据：`/tmp/lf_probe_evidence.json`（12 例原始输入/落库值，仅本机临时）
