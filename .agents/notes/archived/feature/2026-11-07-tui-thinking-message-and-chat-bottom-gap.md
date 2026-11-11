# Agent Note: TUI 思考（reasoning）消息可见与消息区底部呼吸间距

Status: implemented

> **归档冻结说明**：本文档为原历史决策 `docs/archive/decisions/0076-tui-thinking-message-and-chat-bottom-gap.md` 之归档快照，仅供溯源。

## 背景

用户实测 `pony` TUI 反馈两处体验缺陷：

1. **消息列表里没有思考消息**：上游 `turn:delta` 明明携带 `reasoningContent`
   （`crates/pony-agent-core/src/agent/runtime/turn_stream.rs` 的
   `emit_stream_event_with_step(..., Some(reasoning_content), ...)`），但
   `TuiTurnSink::emit` 的 `"turn:delta"` 分支只读 `payload.text`；`TurnStreamEvent.reasoning_content`
   在 TUI 全链路被丢弃 —— 没有 `UiEvent` 变体、没有 `MessageRole`、没有渲染分支。
   桌面端（Vue）有完整的思考呈现（ADR 0056 / 0059），两个交付形态出现可见能力差。
2. **消息区底部零间距**：`ui()` 里 `chat_h = bottom_top - chat_top`，`chat_rect` 下缘恰好等于
   Composer 色带上缘 ⇒ 滚屏贴底时最后一行正文（或流式状态行「思考中 · 12秒」）紧贴输入带，
   阅读拥挤、状态行与输入框视觉粘连。

## 候选方案

### 思考呈现

- **A. 只显示「思考中…」状态词**（不新增角色/事件，改动最小）：落选——用户诉求正是「消息列表里要能看到思考内容」，
  状态词只表达"正在思考"，不承载思考内容。
- **B. 思考内容并入正文 Assistant 消息**（前缀标记区分）：落选——破坏「正文气泡只由文本增量续写」的既有不变量
  （`append_assistant_chunk` 仅在末条为 `Assistant` 时续写），且思考/正文/工具会混在同一条消息里，
  历史语义不可区分，后续折叠与检索都无从下手。
- **C. 新增独立角色 `MessageRole::Thinking` + `UiEvent::StreamReasoning`，视觉从属于正文
  （`✻` 前缀 + `theme::FAINT` + `Modifier::ITALIC` + RAIL 续行缩进）**：采用。
- **D. 思考默认折叠**（按键展开）：落选（本轮）——折叠态需要额外状态机、按键契约与持久化语义，
  首版先满足「可见且不干扰阅读」，折叠作为后续增强，不在本轮闭包内。

### 底部间距

- **E. 在 Composer 色带内加内边距**：落选——会改 ADR 0042 的 composer 几何（提示符行位置、指标行），
  直接冲击既有布局契约测试，且间距属于消息区语义而非输入区语义。
- **F. 只把滚动数学的 `visible` 减 1（矩形不变）**：落选——`last_chat_rect` 仍包含间距行，
  「消息区下缘」的语义与覆盖层/鼠标选区几何分裂，两个真值并存必然漂移。
- **G. 在 `chat_h` 上扣除 `CHAT_BOTTOM_GAP`（取值见决策 6）**：采用——间距行不属于消息区几何，
  `last_chat_rect`、鼠标选区、Ask 覆盖层、splash 居中全部自动保持一致。

### 尾部空白行与滚动锚定

用户消息块以 1 行 SURFACE 内衬收尾（`push_user_block` 的下内边距）。若照旧把该行计入滚动总量，
贴底时最后一个可见行落在这条空白上，「间距行之上必须是内容行」不成立 ⇒
`scroll_total = total - trailing_blanks`（只剔除**尾部连续**空白行；流式状态行非空时该值为 0）。

## 决策

1. **取载唯一面**：`stream_delta_ui_payload(&TurnStreamEvent) -> DeltaUiPayload{ text, reasoning }`。
   `text` 继续走 `sanitize_model_protocol_text`（DSML 协议原文不得进消息区），
   `reasoning` 原样透传（思考不是工具协议通道，仅 `trim()` 为空时丢弃）；
   `TuiTurnSink` 只允许经该符号取载，思考事件先于正文事件投递。
2. **思考入库**：`TuiState::apply_stream_reasoning` —— 末条为 `Thinking` 时原地续写，否则新开一条；
   当前不是 `RuntimeMode::ExecutingTool(_)` 时把 `runtime_mode` 推进为 `Thinking`（工具执行中不抢状态）；
   不改写 `is_streaming` / `scroll_offset` / `auto_scroll`。
3. **思考渲染**：首行 `✻`（`theme::ACCENT_DIM`），正文 `theme::FAINT` + `ITALIC`，续行 RAIL(2 列) 缩进，
   按 `width - RAIL` 折行。1 行呼吸落在**思考块尾部**（`message_lines_with_ctx` 每条消息返回独立
   Block，块首插空行会被 `push_blank` 的 `lines.last() == None` no-op 吞掉；两端都插则变 2 行），
   随后正文块不再额外叠加轮间距。
4. **底部呼吸**：`CHAT_BOTTOM_GAP`，消息区高度扣除该常量 ⇒ 消息区下缘恒停在距 Composer 色带固定行数的位置，
   自动贴底与手动滚到底一致。Ask 内联覆盖层下缘仍贴色带（覆盖层允许占用该行，保既有 O2 断言）。
5. **滚动锚定**：`scroll_total` 剔除尾部连续空白行，保证「内容行 → 空白行 → 色带」的几何可观测。
6. **间距取值（R5 修订，用户实测）**：首发取 1 行，用户反馈偏挤 ⇒ 定为 **2 行**（`CHAT_BOTTOM_GAP = 2`）。
   间距行数只由该常量表达，`chat_h` 推导、Ask 覆盖层补偿、`show_banner` 阈值补偿一律引用常量，禁止字面量散落。

## 影响

- **代码**：`crates/pony-agent-core/src/bin/tui.rs`。新增契约面：`DeltaUiPayload`、
  `stream_delta_ui_payload`、`UiEvent::StreamReasoning`、`MessageRole::Thinking`、
  `TuiState::apply_stream_reasoning`、`CHAT_BOTTOM_GAP`。
- **验收**：`crates/pony-agent-core/tests/tui_thinking_bottom_gap.rs`（A1–A11，红相冻结）；
  契约矩阵 `.dev-team/contract-matrix-thinking-bottom-gap.md`；收据
  `.dev-team/red-phase-thinking-bottom-gap.md`、`.dev-team/green-phase-thinking-bottom-gap.md`。
- **既有契约不变**：ADR 0042 的 composer 几何（提示符行 / 呼吸行 / 指标行）、ADR 0038 的发丝线与弱化层级、
  ADR 0043 的消息层级均未改动；`tui_render_layout` / `tui_batch6_virtualization` /
  `tui_overlay_cancel_acceptance`（除环境性 PTY 用例外）保持绿。
- **审查修正（两轮 L3 对抗，`review-l3-thinking-gap-a/b.json`）**：① 思考块前驱侧呼吸一度是死代码
  （块内 `push_blank` 在空 Vec 上 no-op，Assistant/ToolResult→Thinking 实际 0 行），改为条件式
  `lines.push(blank())`（排除 User 前驱，避免与 User 块内边距叠加成 2 行），并以 A12 锁死；
  ② 正文判空一度误用 `trim()`，会吞掉纯空白增量（`\n\n`）导致段落丢失，回退为 HEAD 口径
  `!cleaned.is_empty()`，以 A13 锁死；③ 宿主续轮（Ask 恢复 / 审批恢复 / goal 续轮）不经过
  `add_user_message`，思考块缺轮边界会续写进上一轮块，改为在 `begin_turn_usage()` 置
  `reasoning_turn_closed`，以 A14 锁死；④ `show_banner` 阈值补偿 `CHAT_BOTTOM_GAP`，恢复
  height=31/35 的空态品牌画。
- **已知残余（本轮不处理）**：① 思考块不做折叠/截断，超长思考会占满视口；② reasoning 增量与同一 hop
  的工具调用交错时，末条消息不再是运行中的 `ToolCall`，`add_or_update_tool_event` 会新起一条工具行
  （L3-A 取证：`turn:delta` 全部在 provider 流回调内、`turn:tool` 之间无模型调用 ⇒ 当前不可达，
  登记为待观测风险）；③ `/resume` 不回放思考（历史 `message_state` 无 reasoning 字段）；
  ④ 流式状态行上方的 `need_blank` 呼吸是既有死代码（`trailing_status_lines` 此时恒为空），
  与本轮无关，未夹带修复；⑤ reasoning 通道零净化（C3 冻结口径）：协议原文若出现在
  `reasoning_content` 会原样作为思考文本渲染（仅展示，不参与解析）。
- **可逆性**：纯前端渲染 + 事件分派改动，无持久化格式、无 wire 契约、无外部接口变化（可逆面为空）。

## 关联

- 承接 ADR 0038（TUI 设计系统的发丝线与弱化层级）、0042（composer 几何与指标行）、0043（消息区层级）；
- 用户消息块自身两侧各 1 行页面底色间距由 [ADR 0088](0088-tui-user-message-vertical-gap.md) 承接
  （2026-11-07）：思考块呼吸行数不变，但 A12 的 User 侧行差随之上调
  （User→Thinking 2→3、Thinking→User 3→4）；「排除 User 前驱另加呼吸」的结论仍然成立。
- 与 ADR 0056 / 0059（桌面端流式状态与视觉层）为不同交付形态的同一语义，互不替代；
- 事件来源：`turn:delta` 的 `reasoningContent`（`agent/runtime/turn_stream.rs`、`StreamReasoningBatcher`）。

