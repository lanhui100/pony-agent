# 0123 借鉴 DeepSeek Harness 跨轮次工具上下文装载与确定性修剪

Status: implemented

> 本文确立 Pony Agent 在多轮对话中跨轮次装载工具调用历史（Tool Calls 与 Tool Results）的组装规范，
> 并引入确定性工具输出修剪（Tool Result Pruning），解决跨轮次工具失忆（Context Amnesia）与上下文爆炸的冲突。

## Context

在对真实 TUI 会话 `pa_01a1ca68-82e8-7b41-9c39-7ea82be75d2b` 的诊断与团队审核中发现：
1. **多轮对话工具失忆**：会话在第 0 至 4 轮执行了大量的工具交互（如 `ls`、`git status`、`backlog lint` 等），但在新轮次（如第 5、6 轮）组装发给模型的 Prompt 上下文时，历史轮次的所有 `tool_calls` 与 `tool_result` 均被剥离，仅保留最终精简的 Assistant 回复纯文本（`ConversationCarryMode::FullReplay`）。
2. **模型连续工程能力受限**：当用户在后续轮次询问前序工具的具体输出、错误详情或前序命令执行结果时，模型因上下文缺失只能依靠上一轮的自述文本进行二次猜测，极易引发幻觉或强迫重复调用工具。
3. **直接全量保留会引发上下文爆炸**：如果未经处理直接把每轮所有工具的完整 stdout/stderr 堆叠在后续轮次中，长命令输出（如数百行日志或编译输出）会在 2~3 轮内耗尽模型上下文窗口（导致 HTTP 400 Context Length Exceeded）并引发巨额 Token 开销。
4. **既有清洗与持久化陷阱**：旧逻辑中 `has_legacy_reasoning_gap` 将非 reasoning 模型的空 `reasoning_content` 误判为历史缺口，且 `has_incomplete_tool_roundtrip` 对未闭合调用直接执行破坏性全量清空（`clear()`），导致非 reasoning 模型原生工具转录 100% 被清空；且当 reasoning 模型被中断时，自愈注入的注记若缺少思维链占位，会再次被后一步 gap 判定误杀清空。

参考 `deepseek-harness` (DSH) 源码（`packages/core/session/src/surface.ts` 与 `packages/compaction/compaction-tool-result-pruner`）的成熟实践：
- 工具结果（`tool/result`）是会话表层的第一公民，默认在跨轮次中持续保留；
- 设立确定性修剪器（Tool Result Pruner），对超过阈值（如 8KB 字符）的工具结果进行语法级安全切片（保留前 4KB + 标记 + 后 1KB），在零模型调用的前提下将单工具 Token 消耗牢牢封顶。

## Decision

1. **引入确定性工具输出修剪器 (`ToolResultPruner`)**：
   - 实现无模型调用的轻量纯函数修剪，默认规则：当工具结果文本超过 8,192 字符时，保留前 4,096 字符与后 1,024 字符，中间插入 `\n\n[... tool result middle pruned ...]\n\n`。
   - 包含字节长度快路径（`len() <= threshold` 零开销借用短路），且严格保证 UTF-8 字符边界安全性（按字符而非粗暴字节切片），不截断多字节汉字与 emoji。
2. **跨轮次上下文支持工具序列原生装载与读写对称**：
   - 协议守卫：仅对 OpenAI 家族协议（`protocol.is_openai_family()`）生效，Anthropic 协议保持 FullReplay。
   - 写侧与读侧对称：在 `turn_sync.rs` 中，纯文本轮次在支持原生工具流的会话中同样同构写入 `provider_native_transcript`；读侧在发现原生工具交互历史时激活回放（`User` -> `Assistant with tool_calls` -> `Tool result (pruned)` -> `Assistant final text`）。
   - 保留前缀缓存优化的物理层序（System Prefix -> Monotonic History Stream -> Volatile Environment -> Current Input）。
3. **单遍严格双向配对状态机与持久化就地自愈（单一真源）**：
   - 改造 `session/store.rs` 的清洗与检测逻辑，直接基于 `repair_and_filter_tool_roundtrips` 单一真源状态机执行就地自愈与过滤：
     1. 遇到无对应 `tool_calls` 的孤儿 `tool` 结果直接安全剔除，绝不透传给大模型（防御 HTTP 400）；
     2. 遇到连续 `assistant` 出现未闭合调用时立即在其之前就地补齐 synthetic tool 占位，恢复严格紧邻性；
     3. 遇到 `user` 边界或序列末尾时，补齐 synthetic tool 结果与 `[中断轮次检查点: 前序工具调用在返回前被中断]` assistant 消息（并附带合规思维链 `"（本轮在工具调用完成前被中断，无有效推理链）"`）；
   - 修复 `has_legacy_reasoning_gap`：仅在会话中包含非中断检查点的有效 `reasoning_content` 时才进行推理空洞校验，且显式豁免检查点注记本身，彻底消除 reasoning 会话中断后的破坏性误杀；仅在不可伪造的推理链缺口发生时才降级清空。
4. **尾部 User 内容身份锚定算法（防重复注入）**：
   - 读侧 `align_native_transcript_with_history` 采用基于 native 尾部 user 内容在 history 中倒序锚定的策略，仅追加锚点后真正未落盘的尾部新轮次，天然免疫 400 条滑动截断与切模型场景下的重复注入。
5. **生产构建零污染门控**：
   - `MemorySessionBackend` 与 `SessionStore::memory_only()` 严格限定在 `#[cfg(any(test, feature = "test-support"))]` 门控下，生产二进制完全不包含测试桩。
6. **代码落点（同提交同步）**：
   - 窗口装配与尾部锚定对齐的纯函数集中在 `crates/pony-agent-core/src/agent/context_window.rs`，`context.rs` 只保留分层装配与提示词渲染；
   - 会话清洗契约单测在 `crates/pony-agent-core/src/agent/session/tests/native_context.rs`，多轮端到端集成测试在 `crates/pony-agent-core/tests/multi_turn_tool_context_test.rs`。

## Alternatives considered

- **A. 维持现状（仅保留 Assistant 最终纯文本）**：落选。虽然 Token 成本极低，但导致 Agent 在多轮对话中丧失工程连续性，无法回答前序工具细节，体验缺陷明显。
- **B. 全量保留历史工具输出，不做任何修剪**：落选。单个工具执行输出（如 `git log` 或编译器完整报错）可能高达数十万字符，极易直接打爆上下文或导致单轮请求成本畸高。
- **C. 摘要模型实时压缩（LLM Summarization）**：落选。每轮调用小模型进行工具输出摘要会显著增加端到端延迟和费用，且容易丢失精确的路径、行号与命令返回码信息。
- **D. 借鉴 DSH 实践：保留工具序列 + 确定性首尾修剪 + 持久化就地自愈（中选）**：兼顾工程上下文完整性与 Token 预算可控性。修剪过程为纯函数、零网络开销、确定性收敛，且完整原始日志仍保留在底层物理日志中供追溯。

## Consequences

- **正面效果**：
  - 多轮交互下模型能完整查阅前序工具的调用参数与修剪后输出，具备真正连续的工程排障和代码修改能力；
  - 极端超长工具输出（如数十万字节的日志）被确定性封顶在 ~5K 字符以内（head 4096 + tail 1024，按字符而非字节计），不会撑爆上下文；
  - 异常中断或取消后，未闭合的工具调用被平滑自愈，孤儿 tool 结果被安全剔除，完全满足 OpenAI 契约，避免上游 API 报 400 错；
  - 400 条滑动截断与模型切换场景下，历史问答不发生重复注入且文本不丢失。
- **负面与权衡**：
  - 相比纯文本回放，发往大模型的 prompt token 数会有适度上升（受 8KB 字符阈值约束）；
  - 仅支持 OpenAI 家族协议，Anthropic 等非 chat completions 协议暂不适用，需维持 FullReplay。

## Supplement（2026-11-12）：中断轮上下文闭合与失败原因留痕

> 触发：真实 TUI 会话 `pa_01a1cb08-f340-7020-83a2-3a9b2d62df29` 的 trace 诊断。
> 取证点：`turn-e3bdbfa6`（同工具连败闸中断）、`turn-c603e22a`（0 步取消）、`turn-e58723fc`（`openai tool follow-up missing text or tool call` 降级）。
> 本补充与前文不是同一个会话（前文 Context 引用的 `pa_01a1ca68…` 为另一会话），条款以本节为准。

1. **截断后仍须配对闭合**（前文 Decision 2/3 的边界补齐）：`context_window::truncate_native_messages` 的「预算击穿骨架兜底」只保留 `user`/`assistant`，会留下「assistant 带 `tool_calls` 而无对应 `tool` 结果」的断裂往返。落点：skeleton 构造后复用单一状态机 `tool_pruner::repair_and_filter_tool_roundtrips` 闭合（丢弃孤儿 tool、为未闭合调用补 synthetic tool 结果），并按**修复后**的列表重新做预算判定。
2. **失败轮检查点必须携带失败原因**：`session::failed_turn_checkpoint_text(prior_text, tool_activities, failure_reason)` 是失败轮检查点文本的唯一投影；`failure_reason` 非空时在既有检查点文本末尾追加「失败原因：<reason>」，为空时与 `interrupted_turn_checkpoint_text` 逐字一致。落点：`runtime/turn_persist.rs::persist_failed_turn_context`。**禁止**因「本轮存在工具步骤」而丢弃 error（否则中断轮在下一轮上下文里只剩步骤清单，模型无法自诊断——这正是本次中断后盲试的根因）。
3. **中断注记判据唯一化**：`session::is_interrupt_placeholder_text` 统一识别 `[中断轮次检查点`、`（本轮上游调用中断`、`（本轮由用户终止` 三类注记；`has_legacy_reasoning_gap` 的豁免判定全部改走该函数，避免 reasoning 会话的中断轮 transcript 被整段 `clear()` 误杀。
4. **0 步取消轮的历史补齐**：先 claim 终态的一方是 C5 宽限监督者（`stop_escalation::escalate_now` → `turn_flow::emit_supervisor_cancelled`），它**只写 checkpoint 与派发事件、不写历史**。因此 `runtime/turn_control.rs::cancel_stream_turn` 在 `claim_terminal_status` 失败时，先判该 turn 是否已有持久化痕迹（`turn_outcome_persisted`：`turn_trace_history` 或 `history` 中存在同 `turn_id`）；无痕迹 ⇒ 只补历史与 trace、**不派发第二条终态**；有痕迹 ⇒ 维持静默退出（terminal-once 不变量不变）。
5. **验收（真实退出码）**：
   - `cargo check -p pony-agent-core --features test-support --quiet` → exit 0；
   - `cargo test -p pony-agent-core --features test-support --test adr0123_supplement_test` → 6 passed / 0 failed（T-A×3、T-D×2、T-C×1）；
   - `cargo test -p pony-agent-core --features test-support --test multi_turn_tool_context_test` → 回归见本波基线 `.dev-team/regression-baseline-wave-adr0123-supp.json`。
6. **已知限制**：Anthropic 家族协议仍不适用（沿用前文）；`tool_pruner` 的孤儿判定仍以前缀 `[中断轮次检查点` 为准，与第 3 条的三类注记判据保持「保守不删」语义，未做合并。

## Supplement 2（2026-11-12 第二轮）：审查缺陷收口

> 触发：L3-A（并发/幂等）与 L3-B（契约/格式）对 `a5634316` 的审查，合计 1 项 BLOCKING + 3 项风险。

1. **B2（失败原因行格式）**：`failed_turn_checkpoint_text` 的原因行改为 ①置于「已执行的工具步骤：」段**之前**（在 `"\n已执行的工具步骤："` 处插入，无该段则追加末尾）；②`trim` + 换行/连续空白折叠为单空格；③按 `char` 截断，**含 `…` 总长 ≤200**。`reason` 为空仍逐字委托 `interrupted_turn_checkpoint_text`。理由：上游 `provider_status_error` 正文可能内嵌 HTML/多段 body，不折叠会让检查点退化成多段正文，并与工具行 `- ` 同形。
2. **D1（中断注记判据唯一化）**：`UPSTREAM_INTERRUPTED_NOTE` / `USER_CANCELLED_NOTE` 两个常量从 `runtime/tool_recovery.rs` 上移到 `session/store.rs`（`pub const`，字符串逐字不变），`is_interrupt_placeholder_text` 改为「前缀 `[中断轮次检查点`」**或**「与两常量整句相等」。裸子串匹配会让「正文引用注记」被误判为中断注记，从而跳过真实 reasoning 空洞检查。
3. **A1（持久化痕迹单值真源）**：历史消息现携带 `turn_id`（`append_turn*` 增加 `turn_id: Option<&str>` 并透传；`persist_turn_outcome` / `persist_failed_turn_context` / `persist_cancelled_turn_outcome` 同步透传）。`turn_outcome_persisted` **只认历史腿**，删除 trace 腿——先 claim 方可能是 C5 宽限监督者或 watchdog 兜底，二者都只写 trace/checkpoint、不写历史，trace 腿会使闸门恒真，0 步取消轮的用户消息便永远补不回来。
4. **B1（重投影与写路径同源）**：`TurnEvent::TurnEnd` 增加 `error`、`TurnEvent::ToolCall` 增加 `description`（additive + `#[serde(default)]`，不 bump `EVENT_SCHEMA_VERSION`）；`turn_flow` 落盘真实值；`projection` 的检查点文本改走 `failed_turn_checkpoint_text`，工具行 `name + description` 与写路径同形。旧事件/旧快照缺字段 ⇒ 退化为改动前文本（有专测逐字断言）。
5. **验收（真实退出码，2026-11-12 09:05）**：
   - `cargo check -p pony-agent-core --features test-support` → exit 0
   - `--test adr0123_supplement_test` → 6 passed；`--test adr0123_supplement_r2_test` → 9 passed（含 T-A1a/T-A1b/T-B1a）；`--test multi_turn_tool_context_test` → 3 passed；`--lib -- session` → 273 passed；`--lib -- projection` → 31 passed
6. **已知边界**：①store 层不按 turn_id 去重（幂等由运行时闸门保证），同 turn 双写仍可能并列，属 TOCTOU 残留；②若某轮事件流无 `user/message` 导致投影 `pending` 未建立，失败轮合成检查点仍不产生（与改动前同形）。
