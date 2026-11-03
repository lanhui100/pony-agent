# 桌面端 Trace 诊断评估（pa_01a1a134 · 4 turn）

> 数据源：Langfuse（ClickHouse `observability/clickhouse-0`）+ Tempo 交叉核对。
> 评估方式：dev team 三路并行取证（trace 取证 / Rust 核心 / 桌面端 UI+遥测），Lead 自检 12 项断言 + L3 独立 verifier 复核 10 项（8 TRUE / 2 PARTIAL；两处 PARTIAL 均为**论据夸大**，已按其纠正改写，见 `verify.json` 的 `verdicts` 与 `prior_selfcheck`）。
> 证据实体：`.dev-team/evidence/trace-20261103/{forensics,core,desktop,verify}.json`。

## 1. Trace 定位与基础元数据

| 项 | 值 |
| :--- | :--- |
| Langfuse session | `pa_01a1a134-0bfa-791d-82fa-0b5eb3808905` |
| Turn / trace_id | turn-0 `0831f979…` / turn-1 `feb84198…` / turn-2 `164e2438…` / turn-3 `d571e252…` |
| 上传时间 | 2026-11-03 11:38:53 – 11:51:03（UTC） |
| 模型 / 版本 | gemini-3.8-flash / release 0.1.176 |
| 规模 | 4 turn、27 次 llm.chat、24 次工具、wall 334.8s、input 579,237、output 2,759、cache_read 396,788（68.5%） |

逐 turn 结局：

| Turn | 用户消息 | wall | 关键结局 |
| :--- | :--- | ---: | :--- |
| 0 | 评估当前项目有哪些需要优化的地方？ | 164.7s | 14 次 LLM+13 工具；首次 `Run` **60s 超时**；最终又产出 699 token 回复，仍判 `ERROR / turn_error: tool_failed`、`degraded=true` |
| 1 | 根据上述优化点…制定下一步实施计划 | 41.1s | 3 次 LLM（4.3s/5.8s/**30.7s**）+2 工具，全成功 |
| 2 | 按照建议开始实施。 | 6.8s | 单次 LLM 后 `phase=cancelled`（会话唯一被中断的轮次），无工具 |
| 3 | dev team实施 | 122.3s | 9 次 LLM+9 工具；`Run` 4 连败（3×exit 101 + 1×exit 1）；末次失败后**无收尾回复**即结束，`ERROR / failed` |

真实耗时结构（turn-0）：`Σttft 57.0% + Σ工具 36.8% + 纯生成 4.9%` —— 桌面端慢在「等首字」和「等工具」，不在生成。

## 2. 诊断结论：七组优化点（按杠杆排序）

### P0-1 上传的 trace 只有指标、没有内容 —— 本轮诊断本身就是它的受害者

- **现象**：4 条 trace 的 `traces.input/output` 全为 NULL；59/59 observation 的 `input/output` 全空（含 llm.chat 27、工具 24）；工具名之外的入参/出参、`tool_calls`、`tool_definitions` 全空。Tempo 原始 OTLP 同样 0 events，排除 Langfuse 侧丢失。
- **代码事实**：`turn_span_attributes` 固定 7 个属性、`llm_request_shape_attributes` 只记「条数 + 是否有 system」、`ToolSpanInput` 只带 name/status/duration/error；全仓无 `langfuse.observation.input/output`、无 `gen_ai.prompt/completion`、无 span event（`crates/pony-agent-core/src/agent/otel/mod.rs:1250`、`:1393`、`:396`）。计划文档里唯一那条 payload 项从未实现（`.dev-team/plan-otel-langfuse-migration.md:61`）。
- **两路径都要断**：实时路径在收尾时会再补 `usage/cost/degraded` 等**数值**属性（`otel/mod.rs:1975-1995`），故「只有 7 属性」仅适用于**重放**路径；但两条路径都无任何 payload（内容）属性——这才是根因。
- **排除误判**：脱敏器只替换路径/令牌，不清空内容（`crates/pony-agent-core/src/agent/ponysentry/sanitizer.rs:59`）；无体积上限（`:2699` 的 512 截断仅用于日志）。
- **影响**：上传后无法离线复盘提示词、模型输出、工具参数与 stderr；本次 4 条用户消息只能由用户口述回填。这让「一键上传 trace 供诊断」在能力上不成立。

### P0-2 Run 的 60s 超时是结构性的，且没有任何「继续跑」的出口

- **现象**：turn-0 首个 `cargo`/`npm` 类命令在 60,048ms 被杀，整轮判 `tool_failed`。
- **代码事实**：60s 不是硬上限，而是**大小写敏感子串匹配**（`build|test|cargo|check|install`，故 `git checkout` 也会被判 60s）得出的自适应默认值，真硬上限 120s（`crates/pony-agent-core/src/agent/tools.rs:210`、`:2859-2877`）；超时提示只叫模型「缩短命令或传更大 timeoutMs」（`:3156`、`:7961`），**从不提示**已挂在模型可见面上的 `job_start` 后台通道（`tools.rs:7554` 将 JobStart/JobOutput/JobKill/JobList 标为 `ModelVisible`）。更关键：timeout 类失败指纹会**删掉 timeoutMs 再比对**（`crates/pony-agent-core/src/agent/runtime/stream_support.rs:591`），于是「延长超时重试」和「原样重试」被算成同一个失败信号 → 3 次即熔断。
- **两道闸且都只终结不换策略**：指纹闸 3 次（`runtime/limits.rs:18`）+ 同工具连续失败 4 次（`:38`）；触发即 `fail_stream_turn_with_hook_dispatch` 收尾（`runtime/turn_stream.rs:512`）或 `Err`（`runtime/turn_sync.rs:204`），无降级继续。
- **影响**：任何 >120s 的构建/测试在单轮内数学上不可解，模型只能反复重试或改命令。桌面端最核心的使用场景（编译本仓）被判死刑。

### P0-3 失败分类与归因缺失，且失败轮丢弃已产出的正文

- **现象**：turn-3 四次 `Run` 失败只拿到 `non_zero_exit 101`；turn 级 `status_message` 只有 `failed`。
- **代码事实**：非零退出统一映射 `non_zero_exit`，唯一特判是 Windows 9009（`tools.rs:3194`、`:7948`）；但 **stderr 是结构化顶层字段**，模型侧能看到 `stderr` + `stderrTruncated`（`:3230`）——所以「模型读不到编译错误」不成立，真实缺口是**没有分类**（101 与普通失败同码）、没有「已试 N 次同命令」的自适应策略，以及产品面/遥测面看不到 stderr。turn 级：`failed` 分支只写 `Status::error(phase)` 不写归因，`tool_failed` 走另一条聚合分支（`otel/mod.rs:1870`、`:1884`；`degraded` 仅在 `phase=completed` 时输出，`:1986`，故 turn-0「降级答完」有、turn-3「真崩」没有——是设计分流，不是口径不一致）；失败轮只下发 `text:"This turn failed."` + error 串，不含已产出的 assistant 正文（`crates/pony-agent-core/src/agent/turn_flow.rs:520`，唯一装配点无正文通道；已下发的流式 delta 不撤回）。
- **影响**：模型拿到的是「未分类退出码 + 可能被截断的 stderr」，于是 turn-3 只能连续 4 次换命令硬试；用户跑了 2 分钟既没拿到内容、也看不出哪一步坏；遥测上工具 span 无 exitCode/error kind/stderr 摘要/重试计数（`otel/mod.rs:1428`、`:2531`），事后只能靠 error 文本 grep。

### P1-1 并行工具被 provider 截断（Anthropic 报错 / OpenAI 静默丢弃），轮内不压缩 —— 14 次往返是代码强制的

- **代码事实**：**并行能力被 provider 层截断两种不同形态**——Anthropic 多 `tool_use` 直接 `Err`（`crates/pony-agent-core/src/agent/provider/mod.rs:2638`、`:3444`），OpenAI 则是 `tool_calls.first()` **静默丢弃**多余调用（`provider/mod.rs:3222`，不报错也不执行）；并行引擎 `parallel_tools.rs` 零生产接线（全仓唯一生产引用是 `agent/mod.rs:31` 的 `pub mod`，`execute_calls` 无调用者，`parallel_tools.rs:13` 记 W28b 待接）；`workspace_batch(parallel)` 白名单只允许只读子调用，`Run/Write` 不可批量（`tools.rs:187`）；自动压缩只在 turn 起点发生，轮内 `accumulated_messages` 单调增长且每 hop 全量 clone 两次（`crates/pony-agent-core/src/agent/runtime/turn_prep.rs:40`、`provider/mod.rs:3469`）；工具结果回灌无摘要，OpenAI 路径只做 12k head4500+tail2500（中部丢弃），Anthropic 路径**完全无上限**（`provider/mod.rs:3802`、`:3809`）；`read_file` 只回前 4000 字符、不给总行数（`tools.rs:3708`）→ 诱导同文件反复 `Read`（turn-0 出现 11 次、其中 5 次 1ms）。
- **影响**：turn-0 的 14 次 LLM 往返（≈84s 纯 LLM 时间）与 308k input tokens 是结构产物；缓存命中 74% 只是「重复发送同一前缀」的结果，不是效率。

### P1-2 桌面 UI 把「降级/失败/等待」全都藏了起来

- `agent.turn.degraded`、`agent.turn.tool_failures` 后端有、**trace 面板不渲染**（`HomeTracePanel` 内两者零命中；后端见 `crates/pony-agent-core/src/agent/otel/mod.rs:227`、`:1984-1995`）→ turn-0 看起来「completed 且健康」。**注意同名不同义**：`src/` 里确有 `degraded` 的既有消费（`src/components/chat/WorkspaceComposer.vue:267`、`src/stores/runtime.ts:2059`、`src/lib/runtime/history.ts:208/240`），那是 history/checkpoint 降级，与 agent.turn 口径无关，不能当反证。
- 工具详情无结构化 `exitCode`/stderr 字段，结果整体当文本块渲染（`HomeTracePanel.vue:1142`、`src/types/runtime.ts:111`）。
- 等待期只有「思考中…用时 N 秒」+240s 静默看门狗，无「已发请求未收首字」阶段、无 per-call 进度（`src/lib/runtime/status.ts:143`、`src/components/chat/AgentRuntimeStatusBar.vue:84`）→ ttft 7.2s / 单次 30.7s 时无法区分「慢」与「卡死」；turn-2 在 6.8s 被中断可能与此有关（**inferred**，原因未取证）。
- 失败轮有呈现（`src/lib/runtime/messages.ts:471`）但**没有恢复动作**：全前端无「重试本轮/重新生成/继续」（`WorkspaceComposer.vue:570` 的「重试」只是发给模型的文本）。
- 双击 ESC 停止白名单不含 `executing_tool`（恰是长轮主阶段），主停止按钮可用（`src/components/HomeWorkspace.vue:1602`、`WorkspaceComposer.vue:1368`）；`ErrorBoundary.vue:53` 只兜组件渲染异常，与 `tool_failed/failed/degraded` 零交集。

### P1-3 一键上传是「空壳重放 + 语义误导」

- `force_export_session_trace` 只读 record 的 session_id/turn_id/sequence/phase，建 span 即 end：**不带 error、ttft、duration、token、tool_activities、trace_steps，也不带子 span、不写 status/时间**（`otel/mod.rs:1218`、`:1229-1233`）；连 `record.trace_id` 都被无视，故重放产物与原始 trace 无任何可对齐的 id；而本地记录里这些真值本来就有（`crates/pony-agent-core/src/agent/session/types.rs:706`、`:734`）。
- 前端把「flush 完成的 turn 数」说成「已成功上传至 Langfuse」，无 trace_id/链接/回执，4s 自动清空（`src/components/telemetry/TelemetryPage.vue:73`、`:84`、`:92`）。
- 部分成功被吞：chunk 中途失败直接 `Err`，已 flush 的 `delivered` 丢弃（`otel/mod.rs:1236`、`crates/pony-agent-core/src/agent/control_plane/query_commands.rs:56`）→ 云端已有数据却报失败，易诱发重复上传；无去重、仅当前活跃会话、上传后不回填 trace_id（`TelemetryPage.vue:65`、`otel/mod.rs:1176`）。
- 上传入口还藏在 coding 模式的 Trace tab，配置页开关无隐私说明、也不说明「不含对话内容」（`TelemetryPage.vue:111`、`:226`、`ConfigGeneralSection.vue:458`）——用户预期与实际相反。
- **交叉事实**：本次 4 条 trace 携带 ttft/degraded 等运行期属性且有完整子 span，属**实时上报路径**的产物；Tempo 在 2 天窗口内查不到任何 `pony.otel.forced=true` 的重放 span，说明手工上传路径要么未被点击、要么静默失败（**UNVERIFIED**，需 e2e 验证）。若被点击，产生的是「只有 7 个属性、零指标」的条目，诊断价值为零。

### P2 指标口径与日志基建

- `agent.turn.speed.tokens_per_second=152.75` 的分母是 `Σoutput ÷ Σgeneration_ms`，剔除了 ttft/工具/排队（27/27 复算吻合）→ 体感 164.7s 的轮次被报成 152 tok/s。
- `usage_details.input` 存的是**未命中缓存**部分（Σ79,032 = 308,019 − 228,987），直接求和会少算 74%。
- Langfuse 原生延迟/成本字段全空（`completion_start_time` 59/59 NULL、`total_cost` 59/59 NULL、`priced=false`）→ TTFT 只能读自定义属性，成本侧无法闭环。
- 结构化日志缺口：Run 超时分支零结构化日志（`tools.rs:3094`），压缩与流式降级只走 `eprintln`（全仓 267 处）。

## 3. 下一步优先级（建议）

| 优先级 | 动作 | 验收锚点 |
| :--- | :--- | :--- |
| P0 | 工具 span / turn span 补 payload 与 `exitCode`/`error_kind`/`stderr` 摘要（默认脱敏+可配置上限） | 上传后 Langfuse 可见用户消息、模型输出、工具入参/结果；重放 span 属性数 > 7 |
| P0 | 超时可解：超时提示指向 `JobStart` 后台续跑；timeout 指纹纳入 `timeoutMs` 语义（延长=新策略而非同信号） | 同一 >120s 构建在一次会话内可完成，且不被 3 次熔断终止 |
| P0 | 失败归因：退出码分类（101=编译/panic）+ 失败轮保留已产出正文 + `failed` 写归因 | turn-3 类场景用户可见「哪一步失败、已产出什么」 |
| P1 | 并行工具接线与轮内压缩（复用 `parallel_tools.rs`） | turn-0 类轮次往返次数与 input token 可测下降 |
| P1 | UI：渲染 degraded/tool_failures、工具 exitCode、首字阶段、失败轮「重试/继续」 | 视觉回归基线 + Console Error=0 |
| P1 | 上传链路：真实回执（trace_id+链接）、部分成功语义、去重、payload 说明 | 上传 e2e：云端条数与本地 turn 数一致且可点击跳转 |
| P2 | 指标口径修正（speed 分母、usage 字段说明）+ 结构化日志收敛 | 报表口径文档 + lint 门禁 |

## 4. 待验证（不得直接作为结论使用）

1. 工具入参重复性无法证据化（无入参/出参，仅代理指标：11 次 Read、5 次 1ms）。
2. 手工上传路径是否被点击、是否静默失败（Tempo 无 `pony.otel.forced=true` 证据）。
3. turn-2 `cancelled` 的触发动机（用户中断 / 系统中断未知）。
4. Langfuse UI 的实际呈现（未开浏览器核对，属推断）。
5. turn-1 单次 30.7s 生成与 turn-2 中断是否同源（provider 侧时序未取证）。

## 5. 团队分工与证据

| 角色 | 产出 |
| :--- | :--- |
| trace-analyst | `forensics.json`：36 条事实（verified 32 / inferred 4）、16 条 SQL 原文、59 条 observation 全字段时间线 |
| core-optimizer | `core.json`：15 条核心侧发现（12 verified / 3 inferred），全部带 path:line |
| desktop-optimizer | `desktop.json`：15 条桌面侧发现（全部 verified），全部带 path:line |
| L3 独立 verifier（subagent） | `verify.json`：其自述为 `verified_by=独立复核`；`meta.independence=full` 属其自我声明，实际与三路队友同基座模型，仅上下文隔离（证据不对称） |
| Lead | 12 项自检（如实保留在 `verify.json.prior_selfcheck`）；被 verifier 判 PARTIAL 的 2 项已按其纠正改写：① OpenAI 多 tool_calls 是 `first()` 静默丢弃而非报错；② `src/` 内确有 `degraded` 消费（history/checkpoint 语义，与 agent.turn 无关） |

## 6. 修订记录

- v1：三路取证 + Lead 12 项自检，初版结论。
- v2（本轮）：L3 verifier 复核 10 项（8 TRUE / 2 PARTIAL）后修订两处**论据**（结论不变）：
  ① P1-1 由「多 tool_use 一律报错」改为「Anthropic 报错 / OpenAI `first()` 静默丢弃」（`provider/mod.rs:3222`）；
  ② P1-2 的 `degraded` 论据改为「trace 面板不渲染 + `src/` 内同名不同义消费」；同时按 verifier 补正 P0-3（stderr 已是结构化字段，缺口在分类与产品面）、P1-3（重放无视 `record.trace_id`）。
