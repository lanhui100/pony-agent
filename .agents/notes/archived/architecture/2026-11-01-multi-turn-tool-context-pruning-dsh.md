# Agent Note: 借鉴 DeepSeek Harness 跨轮次工具上下文装载与确定性修剪

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0123-multi-turn-tool-context-pruning-dsh.md` 之归档快照，仅供溯源。

> 本文确立 Pony Agent 在多轮对话中跨轮次装载工具调用历史（Tool Calls 与 Tool Results）的组装规范，
> 并引入确定性工具输出修剪（Tool Result Pruning），解决跨轮次工具失忆（Context Amnesia）与上下文爆炸的冲突。

## 背景

在对真实 TUI 会话 `pa_01a1ca68-82e8-7b41-9c39-7ea82be75d2b` 的诊断中发现：
1. **多轮对话工具失忆**：会话在第 0 至 4 轮执行了大量的工具交互（如 `ls`、`git status`、`backlog lint` 等），但在新轮次（如第 5、6 轮）组装发给模型的 Prompt 上下文时，历史轮次的所有 `tool_calls` 与 `tool_result` 均被剥离，仅保留最终精简的 Assistant 回复纯文本（`ConversationCarryMode::FullReplay`）。
2. **模型连续工程能力受限**：当用户在后续轮次询问前序工具的具体输出、错误详情或前序命令执行结果时，模型因上下文缺失只能依靠上一轮的自述文本进行二次猜测，极易引发幻觉或强迫重复调用工具。
3. **直接全量保留会引发上下文爆炸**：如果未经处理直接把每轮所有工具的完整 stdout/stderr 堆叠在后续轮次中，长命令输出（如数百行日志或编译输出）会在 2~3 轮内耗尽模型上下文窗口（导致 HTTP 400 Context Length Exceeded）并引发巨额 Token 开销。
4. **既有清洗与持久化陷阱（审核揭示）**：旧逻辑中 `has_legacy_reasoning_gap` 将非 reasoning 模型的空 `reasoning_content` 误判为历史缺口，且 `has_incomplete_tool_roundtrip` 对未闭合调用直接执行破坏性全量清空（`clear()`），导致非 reasoning 模型原生工具转录 100% 被清空，跨轮工具装载完全失效。

参考 `deepseek-harness` (DSH) 源码（`packages/core/session/src/surface.ts` 与 `packages/compaction/compaction-tool-result-pruner`）的成熟实践：
- 工具结果（`tool/result`）是会话表层的第一公民，默认在跨轮次中持续保留；
- 设立确定性修剪器（Tool Result Pruner），对超过阈值（如 8KB）的工具结果进行语法级安全切片（保留前 4KB + 标记 + 后 1KB），在零模型调用的前提下将单工具 Token 消耗牢牢封顶。

## 决策

1. **引入确定性工具输出修剪器 (`ToolResultPruner`)**：
   - 实现无模型调用的轻量纯函数修剪，默认规则：当工具结果文本超过 8,192 字符时，保留前 4,096 字符与后 1,024 字符，中间插入 `\n\n[... tool result middle pruned ...]\n\n`。
   - 包含字节长度快路径（`len() <= threshold` 短路），且严格保证 UTF-8 字符边界安全性（按字符边界而非字节粗暴切片），不截断多字节汉字与 emoji。
2. **跨轮次上下文支持工具序列原生装载与读写对称**：
   - 协议守卫：仅对 OpenAI 家族协议（`protocol.is_openai_family()`）生效，Anthropic 协议保持 FullReplay。
   - 写侧与读侧对称：在 `turn_sync.rs` 中，纯文本轮次在支持原生工具流的会话中同样同构写入 `provider_native_transcript`；读侧在发现原生工具交互历史时激活回放（`User` -> `Assistant with tool_calls` -> `Tool result (pruned)` -> `Assistant final text`）。
   - 保留前缀缓存优化的物理层序（System Prefix -> Monotonic History Stream -> Volatile Environment -> Current Input）。
3. **持久化层安全自愈（替换破坏性整段清空）**：
   - 改造 `session/store.rs` 的清洗逻辑，将 `has_incomplete_tool_roundtrip` 后的破坏性 `clear()` 降解为就地自愈（`recover_incomplete_tool_roundtrips`）。
   - 悬空未决的 `tool_call` 自动补齐为合规的取消占位工具回复，并注入模型可见的固定注记：`[中断轮次检查点: 前序工具调用在返回前被中断]`，杜绝 OpenAI HTTP 400 报错。
   - 修复 `has_legacy_reasoning_gap`：仅对真正声明具备 reasoning 能力的模型校验 reasoning 缺口，豁免通用非 reasoning 工具模型的正常调用。
4. **平滑后向兼容与纯文本历史融合**：
   - 对于缺少 `provider_native_transcript` 的旧历史或纯文本对话，继续回退至 `history_to_native_messages`；若 native transcript 因历史切换缺失尾部纯文本轮，与 `recent_history` 智能融合，确保历史对话不丢失。

## 候选方案与取舍

- **A. 维持现状（仅保留 Assistant 最终纯文本）**：落选。虽然 Token 成本极低，但导致 Agent 在多轮对话中丧失工程连续性，无法回答前序工具细节，体验缺陷明显。
- **B. 全量保留历史工具输出，不做任何修剪**：落选。单个工具执行输出（如 `git log` 或编译器完整报错）可能高达数十万字符，极易直接打爆上下文或导致单轮请求成本畸高。
- **C. 摘要模型实时压缩（LLM Summarization）**：落选。每轮调用小模型进行工具输出摘要会显著增加端到端延迟和费用，且容易丢失精确的路径、行号与命令返回码信息。
- **D. 借鉴 DSH 实践：保留工具序列 + 确定性首尾修剪 + 持久化就地自愈（中选）**：兼顾工程上下文完整性与 Token 预算可控性。修剪过程为纯函数、零网络开销、确定性收敛，且完整原始日志仍保留在底层物理日志中供追溯。

## Consequences

- **正面效果**：
  - 多轮交互下模型能完整查阅前序工具的调用参数与修剪后输出，具备真正连续的工程排障和代码修改能力；
  - 极端超长工具输出（如数十万字节的日志）被确定性封顶在 ~5KB 以内，不会撑爆上下文；
  - 异常中断或取消后，未闭合的工具调用被平滑自愈，避免上游 API 报 400 错，且不会丢失已有历史。
- **负面与权衡**：
  - 相比纯文本回放，发往大模型的 prompt token 数会有适度上升（受 8KB 阈值约束）；
  - 仅支持 OpenAI 家族协议，Anthropic 等非 chat completions 协议暂不适用，需维持 FullReplay。

