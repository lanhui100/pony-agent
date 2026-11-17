# 契约矩阵：审批续轮不丢被挂起回合的用户指令（F-12 / B232）

> 缺陷现场：TUI 会话 `pa_01a1e905`，`turn-45f905c5`（第二轮指令「本轮重点处理ACP相关事项…dev team实施p0-p2以及入账的B164的修复」）
> 在第 24 hop 读工作区外 `~/.agents/skills/dev-team/SKILL.md` 触发 `Approval` 挂起；该轮无 `assistant/message`、无 `turn/end`、
> 无 history node；19:19 批准后 `ApprovalResumeTurn` 新起一轮，续轮请求消息序列里**没有任何**第二轮指令 ⇒ 模型退回上一轮审核任务。
> 归属：`.dev-team/backlog-approval-resume-followups.md` F-12（已登记、未修）。
> 状态：**frozen**（变更须 Lead 原地改写 + 文末流水）｜基线 `c5761b27`｜分级 **A**（单模块 `bin/tui.rs`，无并发原语/无外部 IO/无 schema 变更）

## 0. 根因（已复核）

1. 写路径边界在轮末：`SessionStore::append_turn_fallible_with_kind` 只在轮收口时把 user+assistant 成对写入历史
   （`crates/pony-agent-core/src/agent/session/store.rs:655`）。挂起轮没有收口 ⇒ 指令不落历史（B209 同源）。
2. 审批续轮是新轮：`UiEvent::ApprovalResumeTurn` 走 `start_ask_aware_turn_stream`（无 run 绑定），
   上下文只由已提交历史重建（`bin/tui.rs:12107` 附近）；宿主固定文案「（已批准该工具调用，请重试刚才的操作并继续任务）」
   成为唯一的"续轮意图"来源。
3. 结论：修法必须在**挂起时**把本轮用户指令捕获到宿主内存，并在**续轮时**把它作为模型面上下文注入，且随续轮历史持久化。

## 1. 语义不变量（INV）

- **INV-1 不丢意图**：批准/拒绝续轮的模型面正文必须包含被挂起回合的原用户指令（非空白时），顺序为「原指令 → 宿主续轮文案」。
- **INV-2 界面不伪造发言**：续轮仍**不得** `add_user_message`（不产生用户气泡）；UI 只显示宿主固定文案（批准/拒绝两条常量）。
- **INV-3 一次性消费**：捕获的上下文只被一次续轮消费；消费后立即清空，第二次续轮回到旧行为（正文 = 宿主文案）。
- **INV-4 fail-open**：无捕获（无 `streaming_turn_id` / 无用户消息 / 指令全空白）⇒ 正文与旧实现逐字一致（`prompt`）。
- **INV-5 陈旧不污染**：用户在挂起后**新提交**消息 ⇒ 丢弃旧捕获，下一轮不得注入上一挂起回合的指令。
- **INV-6 零行为夹带**：不改 `ApprovalRequired` 判定、不改 CAS/授权顺序、不改 Ask 续轮路径、不改 proto/历史 schema。
- **INV-7 历史一致**：续轮落库的 user 行必须等于模型面正文（组合正文），不得只落宿主文案（审查 B1）。

## 2. 冻结契约

**C6 — 挂起捕获（TUI 状态面）**
- `TuiState.suspended_turn_context: Option<SuspendedTurnContext>`，`SuspendedTurnContext { turn_id, user_message }`。
- **v2 捕获来源（审查 B2 裁决）**：新增 `TuiState.pending_user_turn: Option<SuspendedTurnContext>`，仅在
  **composer 用户提交**处登记（`turn_id_str` + 原始输入）。`capture_suspended_turn_context(&mut self)`
  **只消费该登记**，且仅当 `pending.turn_id == streaming_turn_id` 时写入 `suspended_turn_context`；
  **禁止**从 `state.messages` 反查用户气泡——goal 轮（`tui.rs:9257`）、Ask 续轮、审批续轮都不产生用户气泡，
  反查会把更早某轮指令张冠李戴成「挂起前指令」。
- 触发点：`UiEvent::TurnSuspended` 分支首行（挂起即捕获，不依赖批准事件）；同一提交只捕获一次（pending 被 take）。

**C7 — 续轮注入（模型面正文）**
- 纯函数 `compose_approval_resume_message(Option<&SuspendedTurnContext>, prompt) -> String`，输出严格为：
  ```
  【挂起前用户指令（宿主补录）】
  <user_message.trim()>

  【宿主续轮】
  <prompt>
  ```
  上下文缺失或 `user_message.trim()` 为空 ⇒ 返回 `prompt`（逐字）。
- `TuiState::take_approval_resume_message(&mut self, prompt) -> String`：取走并清空捕获，返回上述组合。
- **v2 落库口径（审查 B1 裁决）**：`display_message = None`。运行时 `PreparedTurn.display_message` 会回落
  `user_message`（`agent/runtime/turn_prep.rs:125-131`），而落库 user 行取的是 `display_message`
  （`agent/runtime/turn_persist.rs:203`、`turn_stream.rs:1330-1350`）——置 `Some(prompt)` 会让历史只写宿主文案、
  组合正文只进本轮请求（F-12 只修一轮）。置 `None` ⇒ 历史 user 行 = 组合正文，模型面与可审计历史一致。
- 生产接缝 `build_approval_resume_turn_input(state, prompt, provider_id, model_id) -> TurnInput`
  （审查 B3 裁决）：事件循环与单测**共用**这一组装口径（take → build_turn_input → display_message=None），
  杜绝「纯函数自证、接线零断言」的假绿。

**C8 — 观测与收口**
- 续轮取用时落一条 `[pony-tui] approval_resume_context: injected=<bool> turn=<id|-> user_chars=<n>` 诊断（现场可判"是否真的注入"）。
- 新提交路径（composer 主提交，`state.add_user_message(&input)` 之后）清空捕获（INV-5）。
- 会话作用域复位 `TuiState::reset_session_scope`（`/clear`、切会话）清空捕获（INV-5：跨会话不得注入旧指令）。
- `UiEvent::AskResumeTurn` 分支**同样清空捕获**：Ask 续轮是 run-bound 续跑（自带上下文），
  问答应答后不得把 Ask 挂起期的捕获留给后续审批续轮（防张冠李戴注入）。
- API-key 缺失的提交失败分支（`tui.rs:11488` 一带的 `state.add_user_message(&input)` 后）同样清空捕获与 pending
  （审查 A5：该分支已产生用户气泡但不发起轮次，INV-5 必须成立）。
- `reset_session_scope` 同时清空 `pending_user_turn`（未消费的提交登记同属会话作用域）。
- 本波**不做**：run-bound 续轮（B233）、`skill_load` 全局解析（B234）、宿主文案角色改造（B235）——只登记，不改。

## 3. 验收（红→绿，Test 层冻结）

`crates/pony-agent-core/src/bin/tui/tests/approval_resume_context_tests.rs`（Lead 编写并冻结，Executor 只读，v2 共 10 例）：

- **T1**：无上下文 ⇒ 组合结果与 `prompt` **逐字相等**（INV-4）。
- **T2**：有上下文 ⇒ 含原指令与宿主文案，原指令位置在前，两个段标存在，原指令只出现一次（INV-1，防重复注入）。
- **T3**：空白指令上下文 ⇒ 回落为 `prompt`（INV-4 边界）。
- **T4**：捕获三态 —— 无提交登记（即便消息区有旧 User 气泡）不捕获 / 登记轮次与流式轮不一致不捕获 / 一致则捕获并消费 pending（C6，审查 B2）。
- **T5**：`take` 一次性 —— 首次注入原指令，第二次只剩宿主文案，且捕获已清空（INV-3）。
- **T6**：拒绝路径同样带回原指令，且与批准路径组合正文不同（C7/INV-1）。
- **T7**：同一提交只捕获一次；重复挂起信号不得让 pending 复活（C6 一次性边界）。
- **T8**：`reset_session_scope` 清空捕获与 pending（INV-5）。
- **T9**：生产接缝 `build_approval_resume_turn_input` —— `message` 含原指令与宿主文案、`display_message == None`（INV-7）、不新增用户气泡（INV-2）、provider/model 原样透传、捕获被消费（审查 B1/B3）。
- **T10**：无捕获时接缝 fail-open —— `message` 逐字等于 `prompt`、`display_message == None`（INV-4）。

红相判据：实现前 `cargo test --bin pony approval_resume_context` 必须因 `cannot find function/struct`（NotImplemented 等价）失败，原始 stderr 入档。

## 4. 门禁

- 机器：`cargo check -p pony-agent-core --bin pony` exit 0；`cargo test -p pony-agent-core --bin pony approval_resume_context` exit 0；
  存量回归：`cargo test -p pony-agent-core --bin pony`（bin 全量，至少 `wave65*`/`running_status_visibility` 不得回归）。
- 审查：A 级 1 路靶向审查（视角：状态流转 + 假注入/重复注入 + INV-5 陈旧清除）。
- 文档：ADR `.agents/notes/implemented/bug-fix/2026-11-17-approval-resume-suspended-context.md`（`npm run adr:check` exit 0）；
  账本 `B232` record→start→resolve。

## 5. 变更流水

- 2026-11-17 Lead：初版冻结（A 级，B232）。范围仅 C6/C7/C8；B233/B234/B235 明确出界。
- 2026-11-17 Lead（独立审查后裁决，v2）：采纳 3 条 BLOCKING —— B1 落库取 `display_message` ⇒ C7 改为 `display_message=None` 并新增 INV-7；
  B2 捕获不得反查 UI 气泡 ⇒ C6 改为 `pending_user_turn`（提交处登记 + 轮次身份校验）；B3 接线零断言 ⇒ 新增生产接缝
  `build_approval_resume_turn_input` 与 T9/T10。同步采纳 A5（API-key 失败分支清空）；冻结测试由 8 例扩到 10 例并重锚红相。
