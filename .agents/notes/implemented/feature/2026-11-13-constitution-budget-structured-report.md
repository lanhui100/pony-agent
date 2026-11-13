# Agent Note: 宪法层预算裁决的结构化账（AGENTS.md 治理·item-5）

Status: implemented

## Problem

工作区指令（AGENTS.md / CLAUDE.md 层级链）的注入预算只产出一段**模型可见文本**：被省略的层写成 `omitted <path>`、被截断的层写成 `truncated <path>`，此外再无痕迹。

这带来三个看不见的问题：

1. **去重完全静默**：trim 后同文的多层只保留最高优先级一份，被丢弃的层既不进 `omitted` 标记、也没有任何记录——「少注入了一整份规则」在观测面上不可见，且这是跨目录/跨 scope 常见情形（项目根与用户级放同一份规则）。
2. **省略无法归因**：`omitted <path>` 不区分「单文件超源文件上限」（`max_source_bytes`）与「渲染后超总预算」（`max_bytes`），排障时无法判断该调哪个参数。
3. **截断不可量化**：只知道某层被截断，不知道原始多少字节、实际保留多少，无法评估预算是否过紧。

对照 DSH 的实现：其 AGENTS.md 渲染产出 `RenderedAgentInstructions { text, omitted, truncated }` 结构化三元组（`omitted: InstructionFile[]`、`truncated: { displayPath, originalBytes, includedBytes }[]`），文本与账分开。pony 此前只有文本。

## Decision

1. **预算渲染器返回结构化报告**：`render_constitution_with_budget` 改为返回 `(String, ConstitutionBudgetReport)`，文本逐字节不变：
   - `omitted: Vec<OmittedConstitutionLayer { path, reason }>`，`reason ∈ { DuplicateContent, OversizedSource, BudgetExceeded }`（稳定 snake_case 串由 `as_str()` 给出）；
   - `truncated: Option<TruncatedConstitutionLayer { path, original_bytes, included_bytes }>`；
   - `clamped: bool`（是否走到「连标题与标记都放不下」的最后一步硬裁剪）；
   - `is_empty()` 作为「无任何裁决发生」的单一判定。
2. **去重也记账**：`dedupe_keep_flags` 丢弃的层以 `DuplicateContent` 进报告（历史行为**不产生** `omitted` 文本标记，故文本不变），让静默去重首次可观测。
3. **生产路径留痕**：`resolve_constitution_with_budget` 在报告非空时输出一条 `tracing::warn!(target: "hotloader", …)`，字段含各原因计数、截断原始/保留字节、clamped 标记。纯本地日志，不上报（见 `docs/observability.yaml` 归属说明）。
4. **测试锁死语义**：`agent::workspace_hotloader::tests` 新增四条——默认预算报告为空、同文去重记 `DuplicateContent` 且文本只保留一份、超预算记 `BudgetExceeded` 且文本严格 ≤ 预算、最高优先级层截断记精确 `original_bytes/included_bytes`。

## Alternatives considered

- **把结构带上报到 `ProviderRequestObservation`（serde 契约面）**：需要改 TS 契约快照与前端类型（`docs/contracts/ts-contract.snapshot.json` + `src/types/runtime.ts`），并用一次跨端改动换取「界面能看到省略账」；本轮目标是治理与可观测，先落在本地 tracing 与测试，避免线上契约扩散。
- **用内容哈希（SHA-1 / 既有 `fnv1a_hash`）替代整串文本做跨层去重身份**：DSH 用 trimmed SHA-1。pony 现有去重是**精确文本相等**（`HashSet<String>` + 归一化），在本仓文件规模（≤64KiB 预算）下内存与比较成本可忽略；改用 64 位 FNV 反而引入「哈希碰撞导致静默丢规则」的风险。保留精确文本判等，报告已补上可观测性。
- **把报告塞进 `SessionBaselineSnapshot`（会话基线）**：基线是「前缀字节冻结」用的，混入一次性的裁决账会让基线语义变脏（不同预算重渲染时账会变、前缀却必须不变）。报告只由当次渲染产生、当次消费。
- **目录级增量发现（DSH 触碰法）重新实现**：pony 已有 `agent/tools/instructions.rs` 的「触碰即载」机制（read/write/edit 成功后把子目录 AGENTS.md 前置进结果，按结果预算裁剪），无需重造；本轮只补账，不动发现链路。
- **把 `omitted`/`truncated` 文本标记换成结构化 JSON 注入模型**：会让模型可见文本变长且更脆，且破坏既有回归断言；模型要的是「有规则被省略」这个事实，账是给人的。

## Consequences

- 「为什么这轮没注入某份 AGENTS.md」「截掉了多少」「是否硬裁剪」三类问题现在可从一条本地 warn 与结构化报告回答；去重不再是静默行为。
- 模型可见文本零变化：既有 wave42 / wave58 / system-prompt / prompt-tool 套件全绿（17 + 25 + 12 + 6 条相关断言）。
- 边界：报告只在 `resolve_constitution_with_budget` 路径产出；技能清单的 `SkillOmission` 账此前已存在（`SkillScanReport`），本次不动。
- 残余：触碰即载的前置段截断（`cap_prelude_to_budget`）仍只有字符串标记，未结构化；如需要可在同一模型下扩展。
