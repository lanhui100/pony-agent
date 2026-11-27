# Agent Note: 批量工具结果回填（N > 1）接入真流式 SSE 通道

Status: implemented

影响文件：`crates/pony-provider/src/provider/mod.rs`、`crates/pony-agent-core/src/agent/turn_flow.rs`、`crates/pony-agent-core/src/agent/runtime/tests.rs`、`crates/pony-agent-core/tests/session_storage_suite/b254_stream_only_followup.rs`、`docs/observability.yaml`。

前序决策：`.agents/notes/implemented/bug-fix/2026-11-19-tool-followup-stream-only-and-plain-copy.md`（B254 已将 1:1 `tool_followup` 收口为流式唯一通道）、`.agents/notes/implemented/bug-fix/2026-11-09-pa173-upstream-policy-timeout-guard.md`（PA-173 D3 为同步批量 followup 补充了重试）。

## Problem

流式会话 `pa_01a21aa3-533e-7da3-b4af-22774f04699d`（`deepseek-free`，`ponyllm`）在 step 47 因模型并发发起 2 个工具调用而出错中断：

1. `ProviderManager::continue_with_tool_results_stream`（`crates/pony-provider/src/provider/mod.rs`）在 `calls.len() == 1` 时走真流式 SSE（15s 建连 + 180s 单次读空闲，不设总 deadline），但在 `calls.len() > 1` 时无条件委派同步非流式 `continue_with_tool_results`（`"stream": false`、`read_timeout = 45s`、跨 attempt 180s 聚合 deadline）。
2. 当上下文增长至 82 条消息（约 410KB+）且推理模型单次生成耗时超过 45s 时，非流式通道在首个响应字节到达前即连续触发 4 次 `type=timeout+request`（前 3 次 45s、第 4 次被 180s 聚合 deadline 收紧至 41.377s），第 5 次短路后耗尽重试升级为 `turn.failed`。同一时段同一会话的 1:1 流式调用（`followup-stream`）与并行会话的 270k–386k token 流式请求均正常完成。
3. 伴随隐患：`stream_openai_request` 与 `send_openai_tool_decision_stream_request` 虽在 `ProviderResponse.tool_calls` / `ProviderDecision.tool_calls` 返回整批调用，构造 `assistant_message` 时却仅写入了首个 `tool_call`；且 `turn_flow::batch_followup_transport()` 硬编码返回 `buffered`，未按真实支持流式能力区分通道。

## Decision

1. **`continue_with_tool_results_stream` 对流式 provider 走真流式 SSE**：
   - `calls.len() != results.len()` 与空集合检查仍先早退（零 HTTP）；`calls.len() == 1` 仍原样委派 `continue_with_tool_result_stream`。
   - `!self.config.capabilities.supports_streaming` 的模型继续委派同步 `continue_with_tool_results` 并整段回放 `on_delta`。
   - `supports_streaming == true` 且 `N > 1` 时，与 1:1 流式通道对齐：OpenAI Completions / Responses 复用 `openai_followup_messages_for_tool_results` 构造 `1 assistant + N tool` 转录并通过 `build_streaming_http_client` 发送 `"stream": true` 请求，外层走 `retry_provider_followup_stream_timeout_with_policy`（每次重试前从快照重置 `accumulated_messages`，并在发出可见 `Text` 增量后禁止自动重试）；Anthropic 复用 `anthropic_followup_messages_for_tool_results` 并走 `stream_anthropic_request`（`"stream": true`）。
   - 同时保留同步入口 `continue_with_tool_results`（供 `!supports_streaming` 及非流式调用方使用，PA-173 D3 语义不变）。
2. **修补 OpenAI 流式响应的多调用 `assistant_message` 构造**：
   - 新增 `provider_native_assistant_tool_calls_message_with_reasoning_value`（基于 `openai_tool_calls_payload` 生成完整 `tool_calls` 数组；单数版委托它保持逐字节等价），并在 `stream_openai_request` 与 `send_openai_tool_decision_stream_request` 中对非空 `tool_calls` 写入整批调用。
3. **对齐遥测与 `llm.request.transport` 口径**：
   - `turn_flow::batch_followup_transport_for(provider)` 根据 `provider.supports_streaming()` 返回 `stream` 或 `buffered`；`provider_followup_stream_with_calls` 仅在 `supports_streaming == true` 时采集并上报 `ttft_ms`（保持 `buffered` span 无 `ttft_ms` 的 RT-T2 不变量）。

## Alternatives considered

- **继续走同步通道并把 `request_timeout_ms` 上调至 180s~300s**：同步请求受 `UPSTREAM_AGGREGATE_DEADLINE_MS_MAX = 180_000ms` 封顶，且无字节增量进度、无 TTFT，长推理仍会整包超时并在等待期间界面无输出（与 B254 否决同步兜底同因）。
- **在运行时把 N>1 工具结果拆成多次单工具请求**：OpenAI / Anthropic 线协议要求同一条 assistant 消息中的 N 个工具调用必须配齐 N 个结果一次性回填，拆开会导致上游 400 或上下文状态撕裂。
- **同步放开 Anthropic 流式接收多 `tool_use` 与去掉 `disable_parallel_tool_use`**：属于 Anthropic 协议能力扩张（需改造 `collect_anthropic_sse_message_from_response` 报错语义及既有单调用测试），与本次修复「已知并发回填走错同步通道」解耦，留作独立任务。

## Consequences

- `supports_streaming == true` 的模型在多工具并发执行后不再回退 45s 非流式通道，从根本上消除长上下文/长思考场景下的 `followup_sync_batch` 超时中断。
- `crates/pony-agent-core/src/agent/runtime/tests.rs` 中原锁定旧临时偏差的 `batch_followup_transport_is_buffered_not_stream` 迁移为同时校验流式与非流式口径；`b254_stream_only_followup.rs` 补充 `N > 1` 批量流式断流重试且零 `"stream":false` 验收用例。
- `crates/pony-provider/src/provider/mod.rs` 中的 `continue_with_tool_results_stream` 复用共享流式发送器与重试逻辑，无重复 HTTP 协议层。
