# 0123 借鉴 DeepSeek Harness 跨轮次工具上下文装载与确定性修剪

Status: implemented

> 本文确立 Pony Agent 在多轮对话中跨轮次装载工具调用历史（Tool Calls 与 Tool Results）的组装规范，
> 并引入确定性工具输出修剪（Tool Result Pruning），解决跨轮次工具失忆（Context Amnesia）与上下文爆炸的冲突。

## 背景

在对真实 TUI 会话 `pa_01a1ca68-82e8-7b41-9c39-7ea82be75d2b` 的诊断中发现：
1. **多轮对话工具失忆**：会话在第 0 至 4 轮执行了大量的工具交互（如 `ls`、`git status`、`backlog lint` 等），但在新轮次（如第 5、6 轮）组装发给模型的 Prompt 上下文时，历史轮次的所有 `tool_calls` 与 `tool_result` 均被剥离，仅保留最终精简的 Assistant 回复纯文本（`ConversationCarryMode::FullReplay`）。
2. **模型连续工程能力受限**：当用户在后续轮次询问前序工具的具体输出、错误详情或前序命令执行结果时，模型因上下文缺失只能依靠上一轮的自述文本进行二次猜测，极易引发幻觉或强迫重复调用工具。
3. **直接全量保留会引发上下文爆炸**：如果未经处理直接把每轮所有工具的完整 stdout/stderr 堆叠在后续轮次中，长命令输出（如数百行日志或编译输出）会在 2~3 轮内耗尽模型上下文窗口（导致 HTTP 400 Context Length Exceeded）并引发巨额 Token 开销。

参考 `deepseek-harness` (DSH) 源码（`packages/core/session/src/surface.ts` 与 `packages/compaction/compaction-tool-result-pruner`）的成熟实践：
- 工具结果（`tool/result`）是会话表层的第一公民，默认在跨轮次中持续保留；
- 设立确定性修剪器（Tool Result Pruner），对超过阈值（如 8KB）的工具结果进行语法级安全切片（保留前 4KB + 标记 + 后 1KB），在零模型调用的前提下将单工具 Token 消耗牢牢封顶。

## 决策

1. **引入确定性工具输出修剪器 (`ToolResultPruner`)**：
   - 实现无模型调用的轻量纯函数修剪，默认规则：当工具结果合并文本超过 8,192 字符时，保留前 4,096 字符与后 1,024 字符，中间插入 `\n\n[... tool result middle pruned ...]\n\n`。
   - 严格保证 UTF-8 字符边界安全性（按字符而非字节粗暴切片），不截断多字节字符与 emoji。
2. **跨轮次上下文支持工具序列原生装载**：
   - 在 `context.rs` 组装面向模型的输入消息时，具备工具支持能力（`supports_tools` 为 true）的模型，在跨轮次重放时携带历史的完整交互序列（`User` -> `Assistant with tool_calls` -> `Tool result (pruned)` -> `Assistant final text`）。
   - 保留前缀缓存优化的物理层序（System Prefix -> Monotonic History Stream -> Volatile Environment -> Current Input）。
3. **OpenAI 协议工具配对合法性守卫**：
   - 严格保证发给模型的序列中每个 `tool_call` 都有对应的 `tool_call_id` 结果；若历史中存在因取消或异常导致的未决工具调用，自动填充 synthetic canceled 占位，杜绝上游 API 拒绝。
4. **平滑后向兼容**：
   - 对于缺少 `provider_native_transcript` 的旧历史或纯文本对话，继续无缝回退至 `history_to_native_messages`，不破坏历史会话加载。

## 候选方案与取舍

- **A. 维持现状（仅保留 Assistant 最终纯文本）**：落选。虽然 Token 成本极低，但导致 Agent 在多轮对话中丧失工程连续性，无法回答前序工具细节，体验缺陷明显。
- **B. 全量保留历史工具输出，不做任何修剪**：落选。单个工具执行输出（如 `git log` 或编译器完整报错）可能高达数十万字符，极易直接打爆上下文或导致单轮请求成本畸高。
- **C. 借鉴 DSH 实践：保留工具序列 + 确定性首尾修剪（中选）**：兼顾工程上下文完整性与 Token 预算可控性。修剪过程为纯函数、零网络开销、确定性收敛，且完整原始日志仍保留在底层物理日志中供追溯。
