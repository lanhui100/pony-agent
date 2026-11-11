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

## Alternatives considered

- **A. 维持现状（仅保留 Assistant 最终纯文本）**：落选。虽然 Token 成本极低，但导致 Agent 在多轮对话中丧失工程连续性，无法回答前序工具细节，体验缺陷明显。
- **B. 全量保留历史工具输出，不做任何修剪**：落选。单个工具执行输出（如 `git log` 或编译器完整报错）可能高达数十万字符，极易直接打爆上下文或导致单轮请求成本畸高。
- **C. 摘要模型实时压缩（LLM Summarization）**：落选。每轮调用小模型进行工具输出摘要会显著增加端到端延迟和费用，且容易丢失精确的路径、行号与命令返回码信息。
- **D. 借鉴 DSH 实践：保留工具序列 + 确定性首尾修剪 + 持久化就地自愈（中选）**：兼顾工程上下文完整性与 Token 预算可控性。修剪过程为纯函数、零网络开销、确定性收敛，且完整原始日志仍保留在底层物理日志中供追溯。

## Consequences

- **正面效果**：
  - 多轮交互下模型能完整查阅前序工具的调用参数与修剪后输出，具备真正连续的工程排障和代码修改能力；
  - 极端超长工具输出（如数十万字节的日志）被确定性封顶在 ~5KB 以内，不会撑爆上下文；
  - 异常中断或取消后，未闭合的工具调用被平滑自愈，孤儿 tool 结果被安全剔除，完全满足 OpenAI 契约，避免上游 API 报 400 错；
  - 400 条滑动截断与模型切换场景下，历史问答不发生重复注入且文本不丢失。
- **负面与权衡**：
  - 相比纯文本回放，发往大模型的 prompt token 数会有适度上升（受 8KB 字符阈值约束）；
  - 仅支持 OpenAI 家族协议，Anthropic 等非 chat completions 协议暂不适用，需维持 FullReplay。
