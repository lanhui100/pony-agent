# Agent Note: 流式断流与网络瞬态异常自愈重试——解除增量阻断 + TUI 脏增量回滚 + 实时重试状态

Status: implemented

影响文件：`crates/pony-agent-core/src/agent/provider/mod.rs`、`crates/pony-agent-core/src/agent/provider/tests.rs`、`crates/pony-agent-core/src/bin/tui.rs`、`crates/pony-agent-core/src/bin/tui/tests/provider_retry_status_tests.rs`。

## Problem

在真实长文本会话中（如会话 `pa_01a1f31d-89a2-7542-858e-230f0b5fe3d6`），7 轮交互有 5 轮遭遇了 `interrupted` 状态中断，需用户反复手动发送「继续」：
1. **流式断流后人为阻断重试**：
   旧实现（PA-100 遗留）在 `decision_stream` 与 `continue_with_tool_result_stream_bounded` 中使用了 `streamed_any_delta` 标记，一旦首个 delta 吐出，后续只要发生网络抖动（例如 `pproxy` 中继 30s 空闲切断、上游网络波动导致 `error decoding response body` / `unexpected eof`），闭包立即返回 `"stream already emitted delta; auto-retry forbidden"`，导致本可自愈的传输瞬态错误直接退出重试循环，造成整轮崩溃。
2. **文本拼接与状态展示顾虑**：
   旧实现设置 `streamed_any_delta` 禁止重试的初衷是避免多次 attempt 在界面上输出重复或拼接混乱的文本；但由于未在 TUI 消费端实现未完成消息的弹出机制，导致这一顾虑直接牺牲了会话的连通性与自愈能力；且重试期间缺乏清晰的状态通知与回退指示。

## Decision

对齐 `deepseek-harness` (dsh) 的断流重发与增量重置设计：

1. **解除增量发出后的重试门禁**：
   在 `decision_stream` 与 `continue_with_tool_result_stream_bounded` 中移除 `streamed_any_delta` 人为阻断。传输瞬态错误（`TransientRetryable`）正常进入退避重试循环，仅在用户显式取消（`guard.cancel_requested()`）时提前放弃。
2. **TUI 端未闭合流式消息回滚**：
   在 `TuiState` 中实现 `rollback_streaming_message()`。当收到 `UiEvent::ProviderRetry` 重试事件时，自动弹出并丢弃当前正在生成的未闭合 `Assistant` 或 `Thinking` 消息残片。新 attempt 吐出首个 chunk 时将重新开辟干净的消息块，彻底杜绝多 attempt 碎片拼接。
3. **状态栏实时警示与收口**：
   重试期间 TUI 状态行始终展示 `正在重试（attempt/max_attempts）…`（保持 `Warn` 色调），重试预算耗尽后再展示结构化的终态错误提示。

## Alternatives considered

1. **保留增量阻断，回退同步 buffered 请求**：
   已在 B254 契约中被彻底废除。同步请求在长文本生成时无法收到 SSE 活动心跳，极易触发总时限超时，且破坏了流式是唯一交互通道的不变量。
2. **在 Provider 内部缓存全部 delta，直到整流结束再一次性发射**：
   破坏实时流式响应（TTFT）与交互体验，退化为伪流式，违背客户端响应性原则。
3. **在 TUI 端不清空半截消息，而是追加分界线标记**：
   会导致会话上下文中残留废弃的错误片段，影响人类阅读与后续多轮上下文构建，不可取。
