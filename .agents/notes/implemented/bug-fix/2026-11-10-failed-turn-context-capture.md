# Agent Note: 失败轮次同样进上下文（历史 + native transcript 双写）

Status: implemented

- 日期：2026-11-10
- 事故会话：`pa_01a1c4d0`（上游 LLM 报错后用户续聊，新一轮拿不到上一轮任何痕迹）
- 交付面：`crates/pony-agent-core`（runtime / session）

## Problem

上游 LLM 在流式工具轮中途报错后，用户发「继续」时模型**从零开始**（本会话 `pending_asks.control-476836940480`
的 prompt 里写着「本会话的原始任务描述在上下文刷新中丢失了」），复现会话 `pa_01a1c553` 第二轮请求
`messageCount=6`、历史 carry 为 0。

根因不是「记不住」，而是**没抄**：四条轮次收口路径口径不一致——

| 路径 | 历史 | native transcript |
|---|---|---|
| 成功（`turn_stream.rs` 尾部 → `persist_turn_outcome`） | ✅ | ✅ |
| 取消（`persist_cancelled_turn_outcome`） | ✅（拼检查点） | ❌ |
| **流式失败（`fail_stream_turn_with_hook_dispatch_impl`）** | **❌** | **❌** |
| 同步失败（`append_failed_turn`） | ✅（仅错误文案） | ❌ |

失败轮其实**事件层记得很全**（`session.jsonl` 的 `flush_events` 里有 `user/message` 原文与 6 组
`tool_call/tool_result`），丢的只是「把事件折叠成两层上下文载体」这一步。而 native 会话的 prompt
只取 `provider_native_transcript`（`context.rs` 里 transcript 非空即以它为准），所以**只补历史等于没补**。

## Decision

1. **失败轮走与成功轮同一条落盘通道**：新增 `AgentRuntime::persist_failed_turn_context`，内部调用
   既有 `persist_turn_outcome`（⇒ `SessionStore::append_turn_fallible` 的两阶段暂存提交 + 日志命令）。
   不新增第二条写路径、不复制状态机。
2. **历史层**：写「user 原话 + 中断检查点」；检查点正文复用取消轮同一函数
   `interrupted_turn_checkpoint_text`（已产出正文 + 已执行工具步骤），无内容时回落错误文本。
3. **native 层**：新增 `native_transcript_for_failed_turn`，产出
   `user` → `assistant(tool_calls)`/`tool`（逐 hop 成对）→ `assistant`（中断标记）的片段。
   三条硬约束（缺一条都会让 `sanitize_provider_native_transcript` **清空整段 transcript**，
   比不写更糟）：
   - 闭合：只从 `hop_records` 取已成对记录，绝不写孤儿 `tool_calls`；
   - 收尾：末条必须是 assistant（`tool` 结尾会让下一请求出现「tool 后直接 user」的非法序列）；
   - reasoning 非空：`has_legacy_reasoning_gap` 把「`tool` 之后的空 reasoning」判为 legacy 缺口，
     故末条 assistant 的 `reasoning_content` 写**中断事实本身**（不是伪造的模型思考）。
4. **参数传递**：失败派发（`fail_stream_turn_gated_dispatch[_tagged]` /
   `fail_stream_turn_with_hook_dispatch[_tagged]`）新增末位参数 `Option<Vec<Value>>`；工具阶段之前的
   调用点传 `None`（本轮没有可恢复的往返），工具循环内的 14 个调用点传实时构造的片段。
5. **不写 trace**：trace 仍由 `persist_turn_trace_with_provider_calls_and_hooks` 负责，避免双写。

## Alternatives considered

- **A. 只补一条 user 消息**：最小改动，但 native 会话的上下文不读历史层 ⇒ 等于没修；
  且「已执行工具 + 已得结果」仍不可见，模型会重复执行有副作用的工具。
- **B. 只写历史、不动 transcript**：对遗留（非 native）provider 有效，对本事故的 native 会话无效。
- **C. 从日志事件重建 transcript（dsh 式单一真源）**：更彻底，但 `TurnEvent::ToolCall` 目前不携带
  assistant 的 reasoning，重建结果会触发上面第 3 条缺口而**清空整段 transcript**；要做得先扩事件字段
  并处理存量日志，属独立变更（下文「下一步」）。
- **D. 在 `context.rs` 组装时补一段「恢复提示」而不写 transcript**：避开 sanitize，但会与 transcript
  重复表达同一轮，且给 prefix 增加一个随轮次变化的动态段——缓存命中受损。
- **E. 复用 `append_failed_turn`（把 trace 一起写）**：会把 trace 写两遍（本路径已由 trace 通道负责），
  并要求把整份 trace 也透传进来，耦合更大。

## Consequences

- 机器回执（本机）：
  - `cargo check -p pony-agent-core` 通过；
  - `cargo test -p pony-agent-core --lib failed_turn_transcript`：2 passed（新增门禁）；
  - `cargo test -p pony-agent-core --lib fail_stream_turn`：1 passed（新增「失败轮必须进历史」断言）；
  - `cargo test -p pony-agent-core --lib fail_turn_policy`：10 passed。
- 新增门禁：
  - `tool_recovery.rs::failed_turn_transcript_tests`：闭合性（声明 id 必被回应，OpenAI/Anthropic 双协议）
    + 末条 assistant 带非空 reasoning + 首条是用户原话；
  - `runtime/tests.rs::fail_stream_turn_with_hook_dispatch_forwards_partial_text`：失败后
    `snapshot.history` 必须含该轮 user 原话与已产出正文。
- 残余风险（靠 review / 下一步）：
  1. ~~**取消路径仍未写 transcript**（`persist_cancelled_turn_outcome` 传 `None`），native 会话的取消轮
     依旧是「上下文隐身」；同一套 `native_transcript_for_failed_turn` 可直接复用，属下一步。~~
     **已闭环**（2026-11-11）：取消轮复用同一构造（措辞区分「用户终止」），见
     [`2026-11-11-interrupted-turn-context-vs-display.md`](2026-11-11-interrupted-turn-context-vs-display.md)。
  2. **压缩对 native 会话基本失效**：`should_compress` 只看文本历史、压缩后也只
     `replace_session_history`，transcript 完全不动 ⇒ 92 hop 的轮次增长不触发压缩。已在方案中列为 P1。
  3. **双 24 条滑窗**（`store.rs` 的 `DEFAULT_HISTORY_LIMIT` 与 `context.rs` 的
     `TRANSCRIPT_CONTEXT_MESSAGE_LIMIT`）仍在：每轮整体前移一条，前缀逐轮变化，KV 缓存命中被打穿。
  4. 失败轮不写输入附件（失败入口没有 `input.images`），附件只在成功/取消路径落盘。
