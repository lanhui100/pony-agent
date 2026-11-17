# Agent Note: 审批续轮必须带回被挂起回合的用户指令（挂起即捕获，续轮注入）

Status: implemented

## Problem

pony TUI 里「工作区外路径 Read → 审批挂起 → 用户批准 → 续轮」这条链路会把**被挂起回合的用户指令整段丢掉**，模型退回上一轮任务。

现场取证（TUI 会话 `pa_01a1e905`，只读 `~/.local/share/PonyAgent/sessions.db` 与 `~/.pony-agent/tui-logs/tui-runtime.log`）：

- 18:54:27 第二轮用户指令「本轮重点处理ACP相关事项包括以入账事项，dev team实施p0-p2以及入账的B164的修复。」进入 `turn-45f905c5`；第 24 hop 读 `~/.agents/skills/dev-team/SKILL.md`（工作区外）触发 `Approval`：
  `turn:ask-suspend-stream hop=24 request=control-548804653985 kind=Approval`，随后 `turn.suspended` + `decision:WaitUser` + `goal_rounds_stopped reason=turn_suspended`。
- 该轮只有 `turn/start`、`context/observation`、`user/message`、13 个 step、24 次 tool call/result、`turn/speed` —— **没有 `assistant/message`、没有 `turn/end`**，因此 `append_turn` 从未执行，历史节点里也没有这一轮（`normalized_history_nodes` 4 个 commit 节点全不含该指令）。
- 19:19:08 用户批准，`ApprovalResumeTurn` 走 `start_ask_aware_turn_stream`（**新起一轮**，无 run 绑定），上下文只由已提交历史重建：续轮请求的消息序列是
  `System ×2 + Developer(domain) + Developer(session label) + User(旧轮审核请求)×2 + Developer(上轮答复) + Developer(environment) + User(（已批准该工具调用，请重试刚才的操作并继续任务）)` —— **第二轮指令完全缺席**。
- 结果：模型把「继续任务」理解成上一轮的审核任务，重跑 54 步审计并输出审核报告；本会话 `Write/Edit` 调用数为 0（零修复）。用户随后两次追问「本轮会话id是什么？」「本轮会话还没有落修改是吗？」。

根因有两层，互相独立：

1. **写路径边界在轮末**：`SessionStore::append_turn_fallible_with_kind` 才把 user+assistant 成对写入历史（`agent/session/store.rs:655`）。挂起轮不收口 ⇒ 指令不落历史（与 B209「稀疏会话轮内不落盘」同源）。
2. **审批续轮是新轮**：`UiEvent::ApprovalResumeTurn` 的注释明确「没有 run 绑定 ⇒ 走普通宿主流式入口」，于是只剩宿主固定文案「（已批准…）」作为续轮意图，挂起轮的 run 内状态（已执行步骤、待执行工具）全部丢弃。

该失败模式已被登记但未修：`.dev-team/backlog-approval-resume-followups.md` F-12（"挂起轮的会话上下文未落盘……批准后的续轮里模型看不到上一轮用户原始请求"，tmux 复现已记录）；本卡把 F-12 的「在 `turn:suspended` 载荷/宿主侧重放」一支落地为最小实现。

## Decision

在**宿主侧（TUI 事件循环）**补一条"挂起即捕获、续轮注入"的最小闭包，不动运行时、不动历史 schema、不改续轮入口：

1. `TuiState.suspended_turn_context: Option<SuspendedTurnContext { turn_id, user_message }>`：`UiEvent::TurnSuspended` 分支首行 `capture_suspended_turn_context()`。
   捕获来源是 **composer 提交处登记的 `pending_user_turn`**（`turn_id_str` + 原始输入），且仅当 `pending.turn_id == streaming_turn_id` 时才写入；**不**从 `state.messages` 反查用户气泡 —— goal 轮（`tui.rs:9257`）、Ask 续轮、审批续轮都不产生用户气泡，反查会把更早某轮指令张冠李戴成「挂起前指令」（审查 B2 现场复核）。
2. 纯函数 `compose_approval_resume_message(context, prompt)`：输出「【挂起前用户指令（宿主补录）】\n<trim 后的指令>\n\n【宿主续轮】\n<prompt>」；无上下文或指令全空白 ⇒ 原样返回 `prompt`（fail-open，旧行为逐字不变）。
3. `UiEvent::ApprovalResumeTurn` 分支改用生产接缝 `build_approval_resume_turn_input(state, prompt, provider_id, model_id)`：`message = take_approval_resume_message(&prompt)`（一次性消费并清空），**`display_message = None`**。
   为什么不是 `Some(prompt)`：运行时落库的 user 行取 `display_message`（`turn_prep.rs:125-131` 的回落 + `turn_persist.rs:203` 的落库实参），置 `Some` 会让历史只剩宿主文案、组合正文只进本轮请求 ⇒ F-12 只修一轮（审查 B1 现场复核）。置 `None` 后历史 user 行 = 组合正文，模型面与可审计历史一致（INV-7）。UI 仍不产生用户气泡（TUI 本来就不 `add_user_message`），批准/拒绝两条路径共用同一注入逻辑。
4. 一次性 + 不污染：新提交消息（composer 主提交）、API-key 失败分支、会话作用域复位（`reset_session_scope`，覆盖 `/clear`/切会话）、轮次终态（`StreamCompleted`/`StreamFailed`）都清空捕获；续轮取用时落一条 `[pony-tui] approval_resume_context: injected=… turn=… user_chars=…` 诊断，现场可判"是否真的注入"。
5. **链式挂起**（实施审查提出、同一病的真实复现）：续轮自身再次挂起时（应答后又要读第二个工作区外文件）没有 composer 登记，会第二次丢指令。用 `resume_instruction` 记住本次续轮携带的原指令，续轮在生成自身 `turn_id` 后 `seed_pending_after_resume` 登记成本轮 pending ⇒ 第二次挂起可被正常认领；Ask 续轮亦从挂起捕获转写该字段。

因为注入内容随续轮 `append_turn` 落盘（`display_message=None` ⇒ 落库取 `message`），**后续轮次也能看到原指令**（修复了"历史永远缺这一轮"的表象）。

## Alternatives considered

- **A. 在 `turn:suspended` 时把 user 消息单独 append 进会话历史**：落选。历史是 user+assistant 成对的轮次结构（`history_contains_turn` 幂等键是 turn_id），单独写一条 user 会让同一 `turn_id` 后续的 `append_turn` 命中 B222 幂等判据而被整轮跳过（assistant 回复反而丢失）；要安全就得改存储契约与迁移，爆炸半径远超本 bug。
- **B. 把审批续轮改成 run-bound 续跑（与 Ask 续轮同构）**：落选（本轮）。语义上最干净，但需要 `continue_ask_resume_turn_stream` 的等价能力 + dispatcher 级一次性豁免（非路径审批 fail-fast，见 F-1），还要迁移已冻结的 `approval_resume_e2e.rs` T1~T4。已登记 B233，留作专项。
- **C. 让模型自己从 `turn_events` 读回原指令**：落选。模型侧无"读本轮事件"的稳定工具面，且这是宿主职责；让模型补取证等于把确定性缺陷变成概率性补救。
- **D. 在 `TurnInput.history` 里塞一条 user 消息**：落选。`build_turn_input` 的 `history` 为空是既有契约（历史统一由会话库装配），宿主自造 history 会与 store 历史的去重/排序/压缩规则打架。
- **E. 用 system/developer 角色承载宿主文案与指令**：落选（本轮）。`context.rs` 的角色白名单与历史节点 schema 需要 additive 变更，收益（语义更纯）与风险（契约面）不成比例；先把"不丢指令"这条止血，角色改造登记 B235。
- **G. 从 `state.messages` 反查最近一条用户气泡**：落选（审查 B2）。goal 轮 / Ask 续轮 / 审批续轮都不产生用户气泡，反查会命中更早某轮的用户发言，把它贴成「挂起前指令」注入——比丢指令更坏（错误归因）。
- **H. 保持 `display_message = Some(prompt)`、只改请求正文**：落选（审查 B1）。落库 user 行取 `display_message`，历史仍缺原指令，且模型面与历史不一致；下一轮/`/resume` 后问题复现。
- **F. 顺带修 `skill_load` 全局技能 `unknown_skill`**：落选（本轮）。它是本次挂起的触发器而非丢失指令的原因，且原因未定性（扫描省略/缓存/workspace 解析三选一待查），登记 B234 独立处理。

## Consequences

- 批准/拒绝续轮的模型面正文包含被挂起回合的原用户指令；续轮落盘后历史补齐该意图（不再是"凭空多一轮宿主文案"）。
- UI 行为不变：仍不新增用户气泡、仍显示宿主固定文案；`display_message` 与 `message` 的分工保持不变。
- 旧行为在无捕获时逐字不变（fail-open），不影响 Ask 续轮、桌面端、CLI（本改动只在 `bin/tui.rs`）。
- 边界：进程重启后无法恢复（审批请求本身只在 dispatcher 内存、未持久化）；跨实例同样不可恢复 —— 与既有审批恢复能力边界一致，不假装解决。
- 验收：`crates/pony-agent-core/src/bin/tui/tests/approval_resume_context_tests.rs`（v3 共 12 例：逐字回落、顺序/段标/单次注入、空白边界、capture 三态（含轮次不一致/无提交登记）、一次性消费、拒绝路径、重复挂起不复用、会话复位清空、生产接缝落库口径与无气泡断言、fail-open、链式挂起 re-seed、终态/复位清空）。契约矩阵 `.dev-team/contract-matrix-approval-resume-context.md` C6/C7/C8（v2）。
- 遗留：B233（run-bound 续轮）、B234（`skill_load` 全局解析）、B235（宿主文案角色改造）。

## 契约变更登记

无删除/重编号、无 schema 变更、无对外接口签名变化：新增字段仅存在于 TUI 进程内 `TuiState`，新增函数均为 `bin/tui.rs` 内 `pub`（bin 目标，非库公开面）。历史 schema、`TurnInput`、`ApprovalPlan`、续轮入口签名均不变。
