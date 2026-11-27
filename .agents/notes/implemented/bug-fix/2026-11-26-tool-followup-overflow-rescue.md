# Agent Note: ToolFollowup 阶段接入单次受控上下文溢出自救

Status: implemented

## Problem

`.agents/notes/implemented/bug-fix/2026-11-24-native-compaction-split-and-overflow-rescue.md` 已在首跳决策路径接入了 `overflow_rescue.rs` 的受控自救，并将「工具执行后 followup 阶段的溢出」列为残余边界。当长会话恰好在工具执行返回大结果或经过首跳后，在 ToolFollowup 阶段撞到上游 `413 Payload Too Large` / `400 context_length_exceeded` 或触发 `.agents/notes/implemented/bug-fix/2026-11-26-request-body-gate-align-gateway-limit.md` 的 128 MiB 请求体闸门时，同步路径（`turn_sync.rs::handle_sync_tool_turn`）与流式路径（`turn_stream.rs::handle_stream_tool_turn`）此前会直接按失败收口——既包括直接返回 `Err` 的路径，也包括 OpenAI 1:1 同步 followup 经 `local_tool_followup_fallback_response` 包装成 `provider_failure_message` 的路径。

## Decision

1. **在 `overflow_rescue.rs` 新增 `rescue_followup_after_overflow`**：
   - 先调用 `rescue_session_after_context_overflow` 对已落盘会话执行受控紧缩（`compact_session_with_retain_override(..., Some(0))` 或 `trim_native_transcript_to_token_budget`）；
   - 紧缩成功后重跑 `prepare_turn` 刷新 `planning_request` 与 `context_observation`，**绝不就地篡改进行中的 `hop_records` 或脏 `accumulated_messages`**；
   - 若本轮在当前失败 hop 之前已有前置闭合工具批次（`prior_batches`），通过 `ProviderManager::rebuild_followup_messages_for_batches`（`crates/pony-provider/src/provider/mod.rs`）基于新 `planning_request` 与前置批次按原协议重建干净 `accumulated_messages`；若无前置批次则清空 `accumulated_messages`，交由下次 followup 从新 `planning_request` 重新装配。
2. **在 `turn_sync.rs` 与 `turn_stream.rs` 的 ToolFollowup 循环双出口接线**：
   - 同时覆盖 `Err(error)` 与 `Ok(response)` 携带 `provider_failure_message` 两种失败出口；
   - 严格受控边界：**每轮工具循环至多自救 1 次**（`followup_overflow_rescued` 布尔锁，严禁循环），且仅当 `is_context_overflow_error(&error)` 为真时触发；非溢出类 400（如 `invalid max_tokens`）绝不自救；
   - 流式路径额外检查 `!self.should_cancel_turn(control, turn_id)` 且 `!followup_emitted_visible.get()`（参照既有流式重试「已向 sink 发出可见正文增量即禁止重试」判据，且在 `flush_tail_delta_frames` 之前拦截 `provider_failure_message` 兜底包），取消态或已出可见文本时直接按原失败边界收口。

## Alternatives considered

- **允许 ToolFollowup 阶段多次循环自救直到成功或 `Ok(false)`**：拒绝。多轮循环自救可能在高失败场景放大摘要调用与重复请求风险；单轮至多 1 次受控自救失败即落盘终态，边界可预测。
- **在 `accumulated_messages` 内存向量上就地裁剪旧消息**：拒绝。`accumulated_messages` 在失败尝试后可能已处于局部写脏状态，且直接截断消息向量会破坏协议闭合性、prompt cache 与工具去重/计费语义；必须重跑 `prepare_turn` 并从 `prior_batches` 干净重建。
- **在 `provider/mod.rs` HTTP 出口裁剪超限消息**：拒绝（沿用 2026-11-24 与 2026-11-26 ADR 结论）。HTTP 出口无会话轮次语义，静默裁剪会破坏 `tool_call`/`tool_result` 配对与 `reasoning_content`。
- **流式 followup 即使已经向 UI 吐出可见 delta 也强制重试**：拒绝。会造成客户端同一 hop 出现重复/拼接正文，违反流式无重复不变量。

## Consequences

- 涉及源码：`crates/pony-provider/src/provider/mod.rs`、`crates/pony-agent-core/src/agent/runtime/overflow_rescue.rs`、`crates/pony-agent-core/src/agent/runtime/turn_sync.rs`、`crates/pony-agent-core/src/agent/runtime/turn_stream.rs`。
- 原 ADR `.agents/notes/implemented/bug-fix/2026-11-24-native-compaction-split-and-overflow-rescue.md` 中「ToolFollowup 阶段溢出暂不自救」的残余边界已由本条补齐。
- 残余边界：若全新会话（无历史轮次可压缩）在首轮单个工具调用内因工具输出本身极大而溢出，因不篡改当前在飞工具结果，自救会返回未收缩并按既定失败终态落盘。
