# Agent Note: 轮末状态噪声治理——resume 误报复用旧失败 + 停止原因重复占两行

Status: implemented

## Problem

用户在同一会话（`pa_01a1f82e`）报两处「不该出现的状态行」，实测取证后确认是**两条独立特性在同一个收口点各写一行**，根因互不相关：

### 缺陷 1：resume 后误报「上一轮因错误终止」

resume 恢复会话后，信息流末尾出现 `✕ 上一轮因错误终止，已保留已执行的步骤记录。`，但上一轮实际正常完成。

`crates/pony-agent-core/src/bin/tui.rs` 用 `history.iter().rev().find_map(..)` 找「能匹配状态行的记录」。正常完成的 assistant 行**没有信号**（`kind = None` 且正文不以 `[中断轮次检查点]` 开头 ⇒ `interrupted_record_status_line` 返回 `None`），于是 `find_map` 越过它继续往前翻，直到撞上历史上任意一条 `kind = Error` 的旧检查点，把它当成「上一轮」贴到末尾。

实测该会话 `session.jsonl`：末条 assistant 为 `turn-8148d88b`（`kind=None`，正常完成），而历史里有 **8 条** `kind=error` 检查点（索引 13/15/17/19/23/25/27/33，均为早期 provider SSE 失败），最近一条之后已跑了 6 轮正常对话。即：**只要会话历史上出现过一次失败轮，之后每次 resume 都会在底部报「因错误终止」**。

紧邻注释写的是「只取最后一条——更早的中断轮之后既然还有对话，就说明会话已经继续过」，而代码从未实现这个守卫——典型的「注释描述意图、代码做另一件事」。

两路独立子智能体（opencode / antigravity）分别诊断，结论一致：**读取（展示）侧判定错，非落盘写错**。写入侧 `kind = Error` 只在真实失败时产生（`turn_persist.rs` 的 `persist_failed_turn_context` 与 `projection.rs::checkpoint_kind_for` 的 `TurnEndReason::Error` 分支），`TurnEndReason` 未知 wire 值 fail-soft 回退 `Completed`，不会伪造 Error。

### 缺陷 2：轮末连续两行，第二行是状态事实而非对话内容

成功轮：`本轮完成 · 用时 35分7秒 · 工具 13 次`（B216 轮末摘要，`push_settled_turn_summary`）紧跟 `目标已暂停：续轮已停止｜本轮在等你作答/审批｜先回答上面的提问。`（`GoalRoundStopReason::TurnSuspended` 的 `visible_line()`）。

失败轮：`本轮失败 · 工具 5 次` 紧跟 `目标已暂停：本轮未成功｜首轮失败，自动续轮未启动｜发「继续」重试。`（`FirstRoundNotSucceeded`）。

`GoalRoundsStopped` 事件处理里 `if reason.enters_history() { state.messages.push(..) }` 是第二行的来源。问题有三：①与第一行信息重复（「本轮在等你作答」正是屏幕上方的提问）；②把「目标已暂停」这个**状态**塞进**对话流**，而对话流是回看历史用的；③首轮门场景下 `本轮失败` 与 `本轮未成功` 同义反复。

## Decision

1. **resume 状态行判据收成纯函数 `errors::resume_interrupted_status_line(history)`**：只看 `history.last()`，且要求该行 `role == "assistant"`。
   - 末条正常完成 ⇒ `None`（不再回溯旧失败）；
   - 末条不是 assistant（末轮尚未产出回复 / 宿主补录指令）⇒ `None`（此时不存在「上一轮」这个可断言对象）；
   - `kind` 优先级与旧会话标记嗅探逻辑原样保留（`interrupted_record_status_line` 不动）。
   独立成函数是为了让「历史形状 → 是否补行」成为可机械断言的唯一判定点，而不是埋在 500 行事件处理里的一行 `find_map`。

2. **停止原因不再写消息区**：`GoalRoundsStopped` 处理改为调用新增的 `TuiState::apply_goal_rounds_stopped(reason, status_line)`，只写 `status_text` 与 `goal_rounds_stopped`。
   - 可见性**不降级**，三条通道全保留：状态栏 goal 段（`·停` / `·未续`，由 `never_started()` 决定措辞）、`/goal` 面板的 `visible_line()`、`goal_rounds_stopped` 结构化日志（字段集冻结）。
   - `GoalRoundStopReason::enters_history()` 与 `tests/b193_2_goal_stop_visibility_contract.rs` 的 9 条契约判据**一律不动**——它们锁的是「哪些原因值得进历史」这个 core 语义，TUI 侧选择「本轮一条都不进」是展示策略，不构成语义回退。

3. **新增回归测试 `bin/tui/tests/turn_end_status_noise_tests.rs`**（7 条）：历史含旧 error 但末轮成功不补行、末轮真失败仍补行、取消/中断文案仍分流、旧会话标记回退、末条非 assistant 静默、停止原因不落消息区、三条可见性通道逐条断言。

## Alternatives considered

- **修 `visible_line()` 文案（去掉「目标已暂停」前缀 / 精简三段式）**：治不了根。用户抱怨的是「这一行根本不该出现在消息区」，而非措辞。且 `visible_line()` 的逐字文案已由 `tests/b193_2_*` 与 `tests/pa172_legacy_closure_contract.rs` 双重冻结，改文案要连带改两处契约测试，收益却只是让重复的那行变短。
- **把停止原因并进 B216 摘要行**（`本轮失败 · 工具 5 次 · 发「继续」重试`）：信息密度更高，但需要把 `GoalRoundStopReason` 传进 `settled_turn_summary` 纯函数，破坏其「只依赖 outcome/elapsed/tool_calls 三个标量」的可测性；且两个事件（终态摘要 / 续轮停止）到达顺序不保证（`stop` → `completed` 竞态在 B216 里已有记载），合并要引入状态机等待。留作后续独立议题。
- **只对 `never_started()` 与 `TurnSuspended` 两类收窄**（保留「连续 N 轮失败」这类真有信息量的行）：仍会在成功轮出现「连续失败」之外的冗余行，且引入「哪些原因进消息区」的第二套白名单，与 `enters_history()` 语义重叠。用户明确选择「一条都不进」。
- **改 `enters_history()` 让它全部返回 false**：这是把 TUI 的展示策略写进 core 语义，会让 `b193_2` 契约测试（第 4 条：`LoopAlreadyOwned` 之外都应留痕）与 ADR 2026-11-20 的决策同时失效。放弃。
- **resume 时改判「最后一条 assistant」而非「最后一行」**（跳过末条 user）：仍会把「末轮已产出失败回复、之后用户又追问但没等到回复」的场景判成「上一轮失败」，而用户看到的最后一条 assistant 确实是失败记录——但那已经不是「上一轮」。严格取 `last()` 语义最清晰。
- **让 resume 完全不显示任何中断状态行**：会丢掉 B217 / interrupted-turn-resume 波次建立的核心价值（末轮真失败时用户必须知道「可以发继续」）。保留判定，只收紧触发条件。

## Consequences

- 用户侧：resume 不再把若干轮之前的旧失败贴到末尾；轮末只剩一行 `本轮完成 · 用时 … · 工具 N 次` / `本轮失败 · 工具 N 次`。停止原因改由状态栏 `goal 轮N/M·停`（或 `·未续`）与 Ctrl+G 面板承载。
- `tui.rs` 净减约 10 行（删除 push 块），`errors.rs` 净增约 20 行（纯函数 + doc）；`tui_event_loop` 规模未增长（棘轮只许降）。
- 回归面：`--bin pony` 全量 251 项测试通过（含 7 条新增）。受影响契约测试 5 个二进制共 269 项：仅 `interrupted_turn_resume_acceptance` 的 `at2_concurrent_repeated_recovery` 失败，且**已在干净基线上用 `git stash` 复现同一失败**（`at2_repeated_recovery_race.rs:137` 的 Assert 0：进程级 `agent_team_policy_enabled()` 门控被同进程其他用例懒初始化污染，用例注释已自述该防抖限制），与本次改动无关；该用例单独运行通过。
- 未覆盖：桌面端（Vue）`src/lib/runtime/cancelled-turn.ts` 的 `interruptedRecordStatusLine` 的 kind 联合类型缺 `"error"`（诊断副产物），会让桌面端把「错误终止」显示成「已中断」。不在本轮最小闭包内，另行登记。
