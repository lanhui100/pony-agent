# Agent Note: 中断轮次的已产出正文进入下一轮上下文

Status: implemented

## Problem

用户在中止一轮后继续对话，被中断那轮的内容（模型已经渲染到界面上的探索结果、结论）
在下一次请求里**完全缺席**：会话历史里只剩 `用户终止，发送消息可继续。` 这一条占位文案，
或（有工具活动时）一条只有工具步骤清单的检查点。于是新一轮必须从零重新定位代码。

两条证据链：

- `AgentRuntime::cancel_stream_turn`（`runtime/turn_control.rs`）调用
  `persist_cancelled_turn_outcome(..., prior_assistant_text, ...)` 时把正文**硬编码为空串**，
  而同一份正文在失败路径是透传的（`fail_stream_turn_gated_dispatch` 收到 `accumulated_turn_text`）。
  纯函数 `interrupted_turn_checkpoint_text` 在「正文空 + 无工具步骤」时就退回占位文案。
- 会话历史是下一轮上下文的真值来源（`prepare_turn` → `SessionStore::snapshot_at`），
  取消轮的事件侧不会物化 `TurnEvent::AssistantMessage`（只有 `turn:completed` 会），
  因此持久化的 assistant 文本**只**来自 `persist_cancelled_turn_outcome` 的入参。

## Decision

1. `cancel_stream_turn` 增加最后一个参数 `partial_assistant_text: &str`，直接作为
   `prior_assistant_text` 交给 `persist_cancelled_turn_outcome`；不再有硬编码 `""`。
2. 正文来源按取消发生时点分档，**只取已经真实存在的内容**：
   - 工具 hop 循环内（预/后阶段、hop 结束回读）：`accumulated_turn_text`（每 hop 正文累积器，
     与失败路径 B017 同一个变量）；
   - 首轮决策真流式期间（provider 读通道被停止位打断）：新增
     `streamed_assistant_text: Rc<RefCell<String>>`，在决策 delta 回调的
     `ProviderStreamChunk::Text` 分支累积**已渲染给用户**的正文；
   - 决策完成后的两处取消点：`first_decision.output_text`（与失败路径落 `completed_text` 同口径）；
   - 决策发起前（`turn:started` 之后）：`""` —— 此时确实没有任何正文，保留占位文案是对的。
3. 取消事件（`turn:cancelled` 的 `text`）保持原样，仍是 `用户终止，发送消息可继续。`：
   本次只修**落库的 assistant 正文**（进上下文的真值），不动呈现层哨兵与 stopReason 语义。

## Alternatives considered

- **保持现状，靠 TUI 侧回放补上下文**：`bin/tui/errors.rs::interrupted_turn_checkpoint` 只服务
  本地回放，宿主侧无法把 UI 状态塞进 provider 请求；且桌面端同样缺失，等于按界面分别修两遍。
- **在取消路径重新读 checkpoint/trace 拼正文**：checkpoint 只存状态与步骤，没有正文；
  trace 的 `output_text` 是逐调用指标快照，语义不同，且会在取消路径引入一次额外 IO。
- **把正文放进 `turn:cancelled` 事件、由事件物化回填历史**：需要给 `TurnEvent` 增加
  cancelled 物化分支（当前仅 `AssistantMessage` 物化），改动跨事件格式与投影，成本远大于收益。
- **在 `AgentRuntime` 上挂一个「本轮正文」字段供取消路径读取**：隐式共享状态，
  与既有的显式参数透传风格冲突，且并发/多轮场景下容易读到上一轮残值。
- **顺带把 TUI 表头 `首token` 改成 `首token延时`**：本次不做（用户明确要求保持不变）；
  字段语义问题另行处理。

## Consequences

- 机器可查的承诺：`agent::runtime::tests::start_turn_stream_cancels_with_canonical_finalize_boundary_when_stop_is_requested_during_tool_execution`
  现在断言持久化检查点里含被中断 hop 的正文（修复前该断言必然失败）；
  `start_turn_stream_can_emit_cancelled_when_stop_requested_before_plan` 守住「无正文时仍是占位文案」。
- 代价：取消轮检查点文本变长（正文 + 工具步骤清单）；会话历史按既有 `DEFAULT_HISTORY_LIMIT`
  裁剪，未引入新的截断策略。
- 未覆盖：reasoning_content 仍不持久化（既有行为），因此中断轮的思考过程依旧不进上下文。
