# TUI 思考（reasoning）消息可见与消息区底部呼吸间距

Status: implemented

## Context

`pony` TUI 的消息列表看不到思考内容：上游 `turn:delta` 携带 `reasoningContent`，但
`TuiTurnSink::emit` 的 `"turn:delta"` 分支只取 `payload.text`，思考内容在 TUI 侧全链路丢弃
（无 `UiEvent` 变体、无 `MessageRole`、无渲染分支）。另有一条用户反馈：消息区底部（含流式状态行）
与 Composer 输入带零间距，滚屏贴底时最后一行紧贴输入框，阅读拥挤。

本决策的完整正文（背景、候选方案比较、影响与残余风险）见 ADR 0076：
[`docs/decisions/0076-tui-thinking-message-and-chat-bottom-gap.md`](../../../docs/decisions/0076-tui-thinking-message-and-chat-bottom-gap.md)。

## Decision

1. 新增取载唯一面 `stream_delta_ui_payload(&TurnStreamEvent) -> DeltaUiPayload { text, reasoning }`：
   `text` 继续走 DSML 净化，`reasoning` 原样透传（仅空白丢弃），`TuiTurnSink` 只经它取载。
2. 新增 `UiEvent::StreamReasoning` + `MessageRole::Thinking` + `TuiState::apply_stream_reasoning`：
   末条 `Thinking` 续写、否则新开；非工具执行时把运行态推进为 `Thinking`。
3. 思考块渲染从属正文：`✻`（ACCENT_DIM）+ `FAINT` + `ITALIC` + RAIL 续行，前后各 1 行呼吸。
4. 新增 `CHAT_BOTTOM_GAP = 1`：消息区高度扣除该行，底部恒留 1 行空白；Ask 内联覆盖层仍贴色带。
5. `scroll_total` 剔除尾部连续空白行，使「内容行 → 1 行空白 → 色带」几何可观测。

## Alternatives considered

- **只显示「思考中…」状态词**：改动最小，但不承载思考内容，不满足「消息列表里要能看到思考」的诉求。
- **思考并入正文 Assistant 消息**：破坏「正文气泡只由文本增量续写」的既有不变量，思考/正文/工具语义不可区分。
- **思考默认折叠（按键展开）**：需要额外状态机与按键契约，首版取舍为「可见且不干扰阅读」，折叠留作后续增强。
- **在 Composer 色带内加内边距**：会改 ADR 0042 的 composer 几何（提示符行 / 指标行），冲击既有布局契约测试。
- **只把滚动 `visible` 减 1（矩形不变）**：`last_chat_rect` 仍含间距行，「消息区下缘」语义与覆盖层/选区几何分裂。

## Consequences

- 改动集中在 `crates/pony-agent-core/src/bin/tui.rs`；验收测试
  `crates/pony-agent-core/tests/tui_thinking_bottom_gap.rs`（A1–A11）。
- 契约矩阵与红/绿收据：`.dev-team/contract-matrix-thinking-bottom-gap.md`、
  `.dev-team/red-phase-thinking-bottom-gap.md`、`.dev-team/green-phase-thinking-bottom-gap.md`。
- 已知残余：思考块不折叠不截断；`/resume` 不回放思考；reasoning 与同一 hop 工具调用交错时
  末条消息不再是运行中的 `ToolCall`（当前 runtime 顺序不可达，登记为待观测风险）。
