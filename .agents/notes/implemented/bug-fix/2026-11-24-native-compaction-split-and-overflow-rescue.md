# Agent Note: 压缩切分计入原生工具转录并接线 RequiresRequestMutation 溢出自救

Status: implemented

## Problem

在 1M 上下文窗口模型与重工具调用（`native-tool-flow`）会话中，原有实现存在四处叠加缺口，并曾导致工作区尝试「客户端 512 KiB 请求体硬拦截」时把正常长会话直接堵死：

1. **原生消息 token 漏算**：`compression::estimate_native_message_tokens`（`crates/pony-agent-core/src/agent/compression.rs`）只估算 `content`，漏算 `reasoning_content`、`tool_calls` 与 `tool_call_id`，使系统性估算偏低。
2. **压缩触发与切分口径分裂**：`turn_prep.rs` 与 `compaction_ops.rs` 虽用 `ContextPressure`（含 `provider_native_transcript` 与实测 `inputTokens`）判定是否到达 80% 压缩线，但切分阶段仅调用 `split_history_by_tokens(&base_history, retain_tokens)` 度量纯文本 `history` 体积。实测重工具会话 7 轮下来 `history` 仅 ~7.5k tokens、而 `provider_native_transcript` 已达数十万 tokens；因 `7.5k << retain_tokens`（1M 模型下为 `149,760`），自动压缩与手动 `/compact` 会直接返回 `history_too_short` 而不压缩任何内容。
3. **400/413 上下文溢出错误被 BadRequest 掩盖**：`retry.rs` 的 `upstream_code_for_hints` 先匹配 `UPSTREAM_BAD_REQUEST_HINTS`（含 `400`/`413`）再匹配 `UPSTREAM_CONTEXT_OVERFLOW_HINTS`，导致上游真实的 `400 ... context_length_exceeded` / `maximum context length` 或 `413 Payload Too Large` 被判为不可变的 `UpstreamBadRequest`。
4. **缺少 `RequiresRequestMutation` 上层消费者**：`.agents/notes/implemented/architecture/2026-11-10-compaction-token-budget-and-log-command.md` 遗留的溢出兜底未接线，一旦上游返回 `UpstreamContextOverflow` 或单轮工具往返暴涨超窗，当前轮只能直接失败且 `resume` 也无法自动脱困。

## Decision

1. **不在客户端对 HTTP 请求体设 512 KiB 类小体积硬拦截**：1M 上下文在 80% 压缩线时请求体天然达到数 MB（`ponyllm` 网关也为此提供 128 MB 上限）；客户端保持按 token 窗口与原生转录体积控制上下文，不再因传输层误判把正常会话拦死在本地。
2. **补齐原生消息 token 估算**：`compression::estimate_native_message_tokens` 计入 `reasoning_content`、`reasoning`、`tool_calls` 与 `tool_call_id`（仅字段存在时计入，对纯文本历史零影响）。
3. **切分逻辑对 `provider_native_transcript` 保持同一口径**（`compression.rs`、`compaction_ops.rs`）：
   - 新增 `split_history_with_transcript` 与 `apply_compression_with_transcript`，当 `transcript` 非空时，尾部保留 `K` 轮的代价取 `max(history 尾部 token, transcript 尾部 token)`，使重工具会话能准确切出可压缩前缀、真正裁剪 `provider_native_transcript`；`transcript` 为空时与 `split_history_by_tokens` 逐字一致。
   - 新增 `trim_native_transcript_to_token_budget`：当单轮（仅 1 个 `user` 轮次）自身已溢出而无可摘要前缀时，从 `user` 之后、按已闭合的 `[assistant(tool_calls), tool...]` 往返边界丢弃最旧的工具步并就地修剪剩余工具输出，确保 `user` 与最近工具调用闭合不破坏协议（DeepSeek/Anthropic/OpenAI）。
4. **修正 400/413 溢出分类优先级**（`retry.rs`）：在 `UPSTREAM_BAD_REQUEST_HINTS` 分支内（排除 `401`/`403` 鉴权），若报文含明确溢出标识（`context too large`、`context_length`、`maximum context length`、`payload too large`、`request entity too large`、`prompt is too long`、`too many tokens`），返回 `ErrorCode::UpstreamContextOverflow` → `FailureKind::RequiresRequestMutation`；裸 `400`/`413` 与普通参数校验错误（如无效 `max_tokens`）仍保持 `UpstreamBadRequest`。
5. **接线 `RequiresRequestMutation` 单次受控溢出自救**（`crates/pony-agent-core/src/agent/runtime/overflow_rescue.rs`，接入 `turn_stream.rs` 与 `turn_sync.rs`）：
   - 首跳决策返回匹配 `RequiresRequestMutation` 的错误且未取消时，执行至多 **1 次** 受控自救：优先按收紧保留预算运行压缩；若仍无可摘要轮次或摘要失败（如单轮超长），直接走安全的原生转录缩窗兜底写回会话（记录审计事件）；
   - 自救成功后刷新 `PreparedTurn` 并重试首跳请求 1 次；若仍失败则不再循环，正常落盘终态错误。

## Alternatives considered

- **在客户端继续保留 512 KiB / 1 MiB 硬限制**：拒绝。1M 模型 80% 自动压缩阈值前即可达 2–4 MB 以上，小上限会提前阉割上下文窗口并阻断正常压缩路径；防重试体积膨胀已由 `c61a1430` 的 pristine 重置解决。
- **在 HTTP 出口（`provider/mod.rs`）静默裁剪消息字节**：拒绝。HTTP 层缺乏会话轮次与 `CompactHistory` 持久化语义，硬截会破坏 prompt cache 前缀、造成 `tool_call`/`tool_result` 断裂或误删 `reasoning_content` 引发 400。
- **把 `max_tokens` 也纳入 400 优先溢出词表**：拒绝。`"invalid max_tokens"` 类参数错同样是 HTTP 400，纳入会把不可恢复的请求参数错误误判为可压缩自救。

## Consequences

- 涉及源码：`crates/pony-agent-core/src/agent/compression.rs`、`crates/pony-agent-core/src/agent/retry.rs`、`crates/pony-agent-core/src/agent/runtime/compaction_ops.rs`、`crates/pony-agent-core/src/agent/runtime/overflow_rescue.rs`、`crates/pony-agent-core/src/agent/runtime/turn_stream.rs`、`crates/pony-agent-core/src/agent/runtime/turn_sync.rs`。
- 1M 上下文重工具会话在到达 80% 线或执行 `/compact` 时不再卡死于 `history_too_short`，且上游返回 `400 context_length_exceeded` / `413 Payload Too Large` 时可自动触发一轮受控压缩/裁剪并恢复。
- 残余边界：单轮中途已执行多个 tool hop 后的 follow-up 溢出此前按既有失败检查点收口；该缺口已由 `.agents/notes/implemented/bug-fix/2026-11-26-tool-followup-overflow-rescue.md` 补齐单次受控自救接线（首轮单工具结果自身过大且无历史可压时仍按失败终态落盘）。
