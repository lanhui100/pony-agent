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

## 2. 冻结契约

**C6 — 挂起捕获（TUI 状态面）**
- `TuiState.suspended_turn_context: Option<SuspendedTurnContext>`，`SuspendedTurnContext { turn_id, user_message }`。
- `TuiState::capture_suspended_turn_context(&mut self)`：`streaming_turn_id` 为 `Some` 且历史消息中**反向第一条**非空 `User`
  存在时才写入；缺任一条件保持 `None`（不得用 System/Assistant/ToolResult 冒充）。
- 触发点：`UiEvent::TurnSuspended` 分支首行（挂起即捕获，不依赖批准事件）。

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
- `UiEvent::ApprovalResumeTurn` 分支：`turn_input.message = 组合正文`，`turn_input.display_message = Some(prompt)`；
  批准与拒绝两条文案共用同一路径（拒绝时同样带回原指令，模型得以换路而不是空转）。

**C8 — 观测与收口**
- 续轮取用时落一条 `[pony-tui] approval_resume_context: injected=<bool> turn=<id|-> user_chars=<n>` 诊断（现场可判"是否真的注入"）。
- 新提交路径（composer 主提交，`state.add_user_message(&input)` 之后）清空捕获（INV-5）。
- 会话作用域复位 `TuiState::reset_session_scope`（`/clear`、切会话）清空捕获（INV-5：跨会话不得注入旧指令）。
- 本波**不做**：run-bound 续轮（B233）、`skill_load` 全局解析（B234）、宿主文案角色改造（B235）——只登记，不改。

## 3. 验收（红→绿，Test 层冻结）

`crates/pony-agent-core/src/bin/tui/tests/approval_resume_context_tests.rs`（Lead 编写并冻结，Executor 只读）：

- **T1**：无上下文 ⇒ 组合结果与 `prompt` **逐字相等**（INV-4）。
- **T2**：有上下文 ⇒ 含原指令与宿主文案，原指令位置在前，两个段标存在，原指令只出现一次（INV-1，防重复注入）。
- **T3**：空白指令上下文 ⇒ 回落为 `prompt`（INV-4 边界）。
- **T4**：`capture` 三态：无 turn id / 有 turn id 无用户消息 / 两者齐备（断言捕获内容与 `turn_id`）（C6）。
- **T5**：`take` 一次性 —— 首次注入原指令，第二次只剩宿主文案，且捕获已清空（INV-3）。
- **T6**：拒绝文案同样带回原指令，且与批准文案不同（INV-1/C7）。
- **T7**：反向找的是**非空** User：中间夹 System/ToolResult/空白 User 时仍命中真实指令（C6）。

红相判据：实现前 `cargo test --bin pony approval_resume_context` 必须因 `cannot find function/struct`（NotImplemented 等价）失败，原始 stderr 入档。

## 4. 门禁

- 机器：`cargo check -p pony-agent-core --bin pony` exit 0；`cargo test -p pony-agent-core --bin pony approval_resume_context` exit 0；
  存量回归：`cargo test -p pony-agent-core --bin pony`（bin 全量，至少 `wave65*`/`running_status_visibility` 不得回归）。
- 审查：A 级 1 路靶向审查（视角：状态流转 + 假注入/重复注入 + INV-5 陈旧清除）。
- 文档：ADR `.agents/notes/implemented/bug-fix/2026-11-17-approval-resume-suspended-context.md`（`npm run adr:check` exit 0）；
  账本 `B232` record→start→resolve。

## 5. 变更流水

- 2026-11-17 Lead：初版冻结（A 级，B232）。范围仅 C6/C7/C8；B233/B234/B235 明确出界。
