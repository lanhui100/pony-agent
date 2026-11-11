# Agent Note: 中断轮——上下文载体与展示载体分层

Status: implemented

## Problem

用户在继续一个被中断的会话时，信息流里出现整块 `[中断轮次检查点]` 正文与「已执行的工具步骤」清单。
这不是文案问题，而是**一份字符串同时承担两个相反诉求**：

- **模型上下文**：中断/失败轮没有 `assistant/message`，历史里必须补一条 assistant 行，
  否则下一轮模型不知道做过什么、停在哪（`pa_01a1c4d0` 事故：重启后模型从头再问一遍）。
- **信息流**（人读）：用户只需要「上一轮中断了，可以继续」，过程清单属于过程明细。

两者被同一段文本承载，于是模型得到的是「过程事实伪装成 assistant 自述」，用户得到的是噪音。
同一事实还散布在四处实现：`session/store.rs` 的真身、`bin/tui/errors.rs` 的复制品、
`agent/projection.rs` 自带的 20 条截断副本，以及 `bin/tui.rs` 对 `[中断轮次检查点]` 的标记嗅探。

对照 dsh（deepseek harness）：它把中断当作**事件日志里的缺口**——在打开会话处补齐
`tool_call` / `tool_result` 配对与 `turn/end{interrupted}`，用户可见提示由 `turn/end.reason`
派生为一行状态节点，**从不伪造 assistant 正文**。

## Decision

按 dsh 的两条原则（补齐配对、派生展示）分层，不做事件日志迁移：

1. **取消轮写入 native transcript**（`native_transcript_for_cancelled_turn`）。与失败轮同构：
   首条用户原话、已闭合的工具往返原样保留、末条 assistant 以「中断事实 + reasoning」收口；
   措辞区分「用户终止」与「上游中断」（模型需要判断能否重试）。**只在存在已闭合 hop 时写**，
   无 hop ⇒ `None`，仍由文本检查点承载。
2. **历史行新增结构化 `kind`**（`HistoryMessageKind::{Cancelled, Interrupted}`，`serde(default)`
   向后兼容）。写入路径是 `append_turn_with_kind` / `append_turn_fallible_with_kind`；
   历史图节点种类与 `kind` 同判（`TurnCancelled`），不再依赖文本嗅探。
3. **展示层按 `kind` 渲染一行状态**：TUI（`interrupted_record_status_line`）与桌面
   （`interruptedRecordStatusLine`）都只看结构化字段；旧会话无 `kind` 时回退标记嗅探，
   两条路径给出同一句文案。检查点原文不再作为正文直出。
4. **步骤上限与省略标记收口**到 `session::CHECKPOINT_TOOL_LINE_LIMIT`（唯一真源），
   事件投影不再自留阈值副本；`bin/tui/errors.rs` 的检查点函数改为委派真身。
5. **取消哨兵句唯一真源**：`session::CANCELLED_TURN_SENTINEL`，运行时别名、TUI 与历史分类
   均引用它，不再各写一份字面量。

机械证据（可复跑）：

- `cargo test -p pony-agent-core --lib cancelled_turn_transcript_keeps_failed_shape_with_its_own_note`
  —— 取消轮 transcript 闭合、末条带 reasoning、措辞与失败轮不同。
- `cargo test -p pony-agent-core --lib append_turn_with_kind_marks_synthetic_interruption_row`
  —— `kind` 落库且历史图分类同判；成功轮 `kind` 恒为 `None`。
- `cargo test -p pony-agent-core --lib checkpoint_text_collapses_steps_beyond_limit`
  —— 上限与省略标记由纯函数唯一决定。
- `npx vitest run tests/interrupted-record-status.spec.ts` —— 信息流只出一行状态，
  步骤清单不进入消息正文，普通回答逐字保真。

## Alternatives considered

- **把检查点文本彻底删掉（只留结构化 transcript）**：否决。非 native provider（非 openai 家族
  或无 reasoning 的会话）与事件投影重折叠仍以文本为唯一过程记忆；删了等于让这些会话在中断后失忆。
- **迁移成 dsh 式事件日志 + surface 折叠（读取处修补）**：否决。这是架构级改造（sqlite
  `normalized_sessions`、trace schema、前端/TUI/mobile 全部消费点），收益与成本不成比例；
  本次只取「补齐配对」与「派生展示」两条原则，日志级修补留作独立演进。
- **只在展示层过滤，上下文保持原样**：否决。模型侧仍把过程清单当成自己的发言（语义污染），
  且四份实现继续并存，改一处漏一处的风险不减。
- **给 `normalized_messages` 加 `kind` 列并做 DB 迁移**：否决。读取方已能按标记回退判定，
  迁移成本换不来任何行为差异；`kind` 在内存/日志路径生效即可。
- **在 `create_goal`/计划层做指示性文案**：否决。指示性文案不改变载体混装，用户看到的噪音依旧。

## Consequences

- 承接并闭环前序决策：
  [`2026-11-10-failed-turn-context-capture.md`](2026-11-10-failed-turn-context-capture.md)
  （失败轮双写，其「取消路径仍未写 transcript」遗留项本条闭环）与
  [`2026-11-08-cancelled-turn-assistant-text-in-context.md`](2026-11-08-cancelled-turn-assistant-text-in-context.md)
  （取消轮正文进上下文；本条不改其正文采集口径，只把「过程清单」从正文里分离出来）。
- native 家族会话的取消轮：上下文由 transcript 承载（检查点文本不再进入 prompt），
  信息流只出现一行状态；失败轮行为不变（此前已是同构实现）。
- 旧会话、sqlite 归一化读取路径（表无 `kind` 列）与 TUI 冻结契约测试均走回退判定，
  行为与文案一致，无需数据迁移。
- 非 native provider 仍会在上下文里看到检查点文本（模型可见），但界面不再直出清单——
  这是本轮有意保留的兼容面，不是遗留缺陷。
