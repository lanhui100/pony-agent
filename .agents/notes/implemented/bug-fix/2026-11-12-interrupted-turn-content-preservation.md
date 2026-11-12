# Agent Note: 中断轮次的内容保真——在飞工具往返必须落成闭合真值，续跑从它投影

Status: implemented

## Problem

真实事故会话 `pa_01a1ce16-cf4e-72d0-a92b-ae9857aac737`（TUI）：一轮任务在中途被中断，用户继续后，
**该轮已完成的工作从模型上下文里整体消失**——续跑请求只有 12 条消息 / 16165 tokens，模型对那一轮
的全部记忆就是一条 492 字符的 `[中断轮次检查点] 已执行的工具步骤：…`；随后 36 跳从 16k 重新长回 60k，
即模型在"没有前情"的状态下把活重做了一遍。用户侧同一现象表现为"中断轮次的信息全部消失 / 原任务没了"。

这不是提示词问题，是三处载体链路同时失真的结果：

1. **在飞工作不是真值，只在内存**。中断键（Ctrl+C 单按 / Esc 双击）只发取消令牌
   （`crates/pony-agent-core/src/bin/tui.rs`、`agent/control_plane/mod.rs` 的 `stop_turn`），
   真正的落库发生在轮收口：`agent/runtime/turn_persist.rs::persist_turn_outcome` 先 flush 事件，
   再把整轮压成一条检查点文本写入历史——**已交付正文与工具往返不构成正文**。
2. **续跑上下文只读历史 carry**。`agent/runtime/turn_prep.rs` → `agent/context.rs` 的请求组装
   只取 `session.history`（native transcript 仅在"能对齐"时启用）。事故会话的 native transcript
   在库里非空（598 KB），续跑却走 `normalized-input` carry ⇒ 在飞工具往返永久缺席。
   事件折叠（`agent/projection.rs`）也救不了：它只对 `event_seq_range` 非空的引用化节点生效，
   且 `ToolCall` 只留 name+description、`ToolResult` 直接丢弃。
3. **收口 append 污染历史图**。`agent/session/store.rs::prepare_session_for_new_turn` 在
   `visible_node_id != branch_head_node_id` 时建隐式 fork 分支，且新节点 `turn_id` 取
   `turn_trace_history.last()` ⇒ 事故库出现 `…-branch-2` 与"同一轮两条记录（并被错标成上一轮）"。

第四条是展示层：DSML 协议原文在 **SSE 解析层**被逐行折叠成 `［工具调用］` 并**写回** assistant 正文
（事件载荷、`normalized_messages`），用户看到占位符连排 + 参数残片，上下文里还留着一句垃圾正文。

对照 deepseek-harness（DSH）的实际设计，本仓缺的正是它的三条不变量：日志即真源、上下文是纯投影、
**补齐优先于截断**。DSH 的 `openTurnClosers` 对开放尾部合成 `error tool/result → step/end →
turn/end{interrupted}` 并写进日志（`packages/core/session/src/repair.ts`），模型历史由
`deriveMessages()` 从 surface 节点投影（`packages/core/session/src/index.ts`），核心没有任何代码按
`turn/end.reason` 过滤模型消息；它甚至把"加载时截断被中断轮"写成**被否决**的提案
（`.agents/notes/rejected/simplification/2026-06-20-truncate-interrupted-turns.md`）。

## Decision

按"补齐优先于截断 + 上下文即投影 + 展示与载体分层"三条原则改造中断轮次，**不迁移成 DSH 式事件日志**：

1. **中断/失败轮落成闭合结构**（唯一真值）：`agent/session/interrupted_turn.rs` 提供纯函数出口，
   把已交付正文 + 工具往返构造成**配对闭合**的 native transcript 片段；悬空 `tool_call` 补一条
   error 结果，措辞显式表达"结果未知/未开始"并给可操作下一步（对齐 DSH 的
   `TOOL_OUTCOME_UNKNOWN` / `TOOL_NOT_STARTED`，不伪装成功）。中断轮与失败轮走同一出口（同构）。
2. **续跑请求从该结构投影**：请求组装在 native 通道可用时使用闭合结构；不可用时退化为
   "步骤清单 + 结果摘要"的可见步骤文本，而不是单条检查点句。检查点文本降级为兼容旧会话的回退。
3. **历史图不再因中断而漂移**：中断收口不建隐式 fork；历史节点 `turn_id` 取本轮 id（不再借
   `turn_trace_history.last()`）；同一轮在历史里恰好一条。
4. **折叠不写回**：DSML 协议块的折叠只发生在展示侧；持久化正文只保留已交付正文，协议块整体丢弃。
   旧库既有脏行只读兼容（不再回放、不再放大、不回写）。
5. **展示一行状态 + 步骤可展开**：中断轮在信息流只出一行状态（复用既有 `kind` 结构化判定），
   过程明细不直出正文。

实施落点（同提交，供复核）：

- 纯函数出口：`crates/pony-agent-core/src/agent/session/interrupted_turn.rs`
  （`interrupted_turn_assistant_text*` / `interrupted_turn_transcript_closure` /
  `pending_tool_result_note` / `history_node_turn_id` / `persisted_assistant_text`）。
- 生产接线：`agent/runtime/turn_persist.rs` 的唯一收口出口 `build_interrupted_turn_close_out`
  （中断/失败两路同构）；`turn_control.rs`、`turn_stream.rs` 透传全量活动与"已宣告未执行"调用。
- 历史图一致性：`agent/session/store.rs`（`TurnCancelled` 不推进分支头、收口后 visible 与 head 一致、
  中断收口不走隐式 fork、节点 `turn_id` 显式取本轮）。
- 折叠分离：`agent/provider/mod.rs` 的 `strip_dsml_protocol_text`（持久化，整块丢弃、无占位符）与
  `display_dsml_protocol_text`（展示，折叠 + 碎片丢弃 + 占位符收敛）；`bin/tui/errors.rs` 改走展示侧。
- 请求体确定性：`agent/workspace_hotloader.rs` 的"基线未命中"通知只进结构化日志，不再注入模型可见
  请求体（进程本地缓存状态不得决定请求体，否则同一落盘会话两次恢复不幂等）。
- 接线守卫：验收套件 C9 用结构性断言挡住"纯函数实现但生产未接线"的假绿，并有运行时单测
  真实调用生产入口（`agent/runtime/tests.rs`）。

机械证据（可复跑，收据见 `.dev-team/green-receipt-interrupted-turn-resume.md`）：

- `cargo test -p pony-agent-core --test interrupted_turn_resume_acceptance`（C1–C8 验收，含 C1
  "工具往返必须出现在下一请求里"、C3 "正常续跑不 fork"、C5 "折叠不写回"）。
- `cargo test -p pony-agent-core --lib`（本 worktree 口径 1510 passed / 0 failed；主工作树 1518 为另一 HEAD 口径，无删测/跳过）。
- `.dev-team/red-phase-interrupted-turn-resume.json`（红相冻结：断言失败而非编译失败）。

## Alternatives considered

- **改成 DSH 式 append-only 事件日志 + surface 纯投影**：否决。这是架构级重写（sqlite 归一化层、
  trace schema、TUI/桌面/CLI 全部消费点），与本缺陷的因果面不成比例；本次只取它的三条不变量。
- **只把检查点文本写得更细（把工具结果全文塞进那条文本）**：否决。仍是"过程事实伪装成 assistant
  自述"，继续污染语义，且历史图漂移与展示污染两条缺陷照旧。
- **让续跑请求直接回放 `turn_events`（事件即上下文）**：否决。事件里的 tool/result 判据分散、
  与 native 对齐口径冲突，会把"投影"变成"第二套请求组装"，违背单一真源；改为把闭合结构写进
  native transcript（已有载体）。
- **中断时直接删除该轮（回到上一轮结束处）**：否决。DSH 已有被否决提案（"a single turn can contain
  substantial real work"）；本仓 `pa_01a1c4d0` 事故也证明删轮会让模型从头再问一遍。
- **在 TUI 展示层过滤中断轮**：否决。展示过滤不解决上下文缺席，用户看到"没发生"，模型同样看不到。

## Consequences

- 承接并收口 [`2026-11-11-interrupted-turn-context-vs-display.md`](2026-11-11-interrupted-turn-context-vs-display.md)
  的未覆盖面：该条只保证"检查点文本可读/可续"，本条保证"工具往返成为上下文真值"；其对
  native 家族会话的假设（取消轮由 transcript 承载上下文）在本事故中不成立，本条补上投影入口。
- 与 [`2026-11-10-history-fold-replay-and-default-256k-window.md`](../architecture/2026-11-10-history-fold-replay-and-default-256k-window.md)
  的折叠口径不冲突：折叠仍是"引用化节点"的读路径，本条不改变折叠判定。
- 旧会话（无闭合结构）行为不变：走检查点回退；`normalized_messages` schema 未动，无数据迁移。
- 已知边界：非 native provider 家族仍以可见步骤文本承载（沿用既有兼容面），不是遗留缺陷而是
  本轮有意的降级面。
