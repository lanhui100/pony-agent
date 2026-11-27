# Goal 机制审核：`/goal` 为何仍被 `ask_user` 阻断（对照 DSH）

- 日期：2026-11-26
- 触发问题：用户 `/goal <目标>` 之后，模型一次 `ask_user` 就把自动续轮打断，且作答后目标不再自己推进
- 对照对象：`/home/dm/deepseek-harness/packages/goal`（`goal` / `goal-round-driver` / `tool-goal`）、`packages/interaction/tool-ask-user`
- 取证口径：本仓结论均给 `文件:行号`；DSH 结论均给 `包/文件:行号`。未取证项显式标注

---

## 0. 一句话结论

**「goal 已设」与「goal 会自己跑下去」是两件事。**本仓的续轮循环以「上一轮 turn 成功收口」为唯一推进前提，而 `ask_user` 是 host-mediated 工具 ⇒ turn 以 `suspended` 收口 ⇒ 循环按设计退出；作答之后**没有任何驱动者把 goal 循环重新拉起**（TUI 的 Ask 恢复只跑单步、桌面端连单步都没有）。所以现象不是"goal 没生效"，而是"goal 在一次挂起后永久失活，且状态栏仍显示 active"。

---

## 1. 阻断链路（逐步取证）

### 1.1 `ask_user` 会终结 turn

| 步骤 | 事实 | 证据 |
|---|---|---|
| ① `ask_user` 是 host-mediated | 声明层固定 `host_mediated = true` | `crates/pony-protocol/src/tool_contract/names.rs:565-572` |
| ② 治理判定为 `WaitingHost` | 工具不执行，返回控制请求 | `crates/pony-tools/src/governed_executor.rs:120-123` |
| ③ turn 以 `suspended` 收口 | `SUSPENDED_TURN_PHASE = "suspended"` | `crates/pony-agent-core/src/agent/runtime/types.rs:248`、`turn_control.rs:529-542` |
| ④ graph 决策固定 `WaitUser` | 有未决 ask wait 时禁止 planner 决策 | `crates/pony-agent-core/src/agent/control_plane/mod.rs:2222-2235` |
| ⑤ 终态归约为 `Suspended` | 决策真值优先 | `control_plane/mod.rs:705-730`（`turn_stream_outcome_from_terminal`） |

### 1.2 续轮循环把 `Suspended` 当终止

| 路径 | 行为 | 证据 |
|---|---|---|
| TUI 首轮门 | `Suspended ⇒ TurnSuspended`，根本不进循环 | `crates/pony-tui/src/tui/goal_round_stop.rs:23-30` |
| TUI 循环内 | `Suspended ⇒ TurnSuspended` 并 `break` | `crates/pony-tui/src/tui.rs:10578-10588`、`10494-10506` |
| TUI 前置让位 | 有待作答 Ask/审批就直接 `break` | `tui.rs:957-964`、`10308-10322` |
| core 循环 | `decision.kind != Continue ⇒ return` | `crates/pony-agent-core/src/agent/control_plane/mod.rs:2017-2020`、`2884-2887` |

TUI 让位是 ADR 0092 的正确修复（避免续轮把待作答 Ask 判为过期、模型自转 16 轮），**问题不在让位本身**。

### 1.3 作答之后没有人重启 goal 循环 ← 真正的断点

| 端 | 作答后发生了什么 | 证据 |
|---|---|---|
| TUI | `answer_ask` + `graph_resume_ask` → 派发 `AskResumeTurn` → 只跑**一次** run-bound 单步（`continue_ask_resume_turn_stream`），**不重入 `run_goal_rounds`** | `tui.rs:9153-9270`、`13481-13620`；`ask_resume_commands.rs:13-16` 明确「只做单步执行，不用 `execute_graph_run_stream`」 |
| 桌面 | `ask_answer` → `graph_resume_ask` 把 run 放回 `Ready`，**前端没有任何 auto-continue 驱动者**；`continue_graph_run_stream` 只在用户主动提交时调用 | `src/stores/ask.ts:335-362`；`src/stores/runtime/orchestration.ts:631`（唯一调用点）；`grep -rn "autoContinue" src` 零命中 |

结论：run 停在 `Ready`、goal 停在 `active + Armed`、`GoalBar` 仍显示"进行中"，但没有任何线程会再发下一轮。桌面端尤其明显——用户答完，界面什么都不动。

### 1.4 第二条更隐蔽的阻断：正文问号启发式

`planner.rs` 在 armed goal 判定**之前**先跑文本启发式：

```rust
// crates/pony-agent-core/src/agent/planner.rs:88-104
if assistant_requests_user_input(assistant_message) { return wait_for_user_decision(); }
let armed_goal = ...;  // 只有没命中上面才看 goal
```

`assistant_requests_user_input` 的判据是「trim 后以 `?`/`？` 结尾」或命中 `需要我/是否继续/请提供/告诉我/can you/should i/...`（`planner.rs:774-797`）。

后果：goal 轮里模型只要**正文**以问号收尾（"要不要我顺手把 B 也改掉？"），循环就静默退出，而且**不会弹 Ask 面板**（没走工具），用户看不到任何问题——只有"目标不动了"。这条比 `ask_user` 更难诊断。

### 1.5 `ask_user` 永久挂起，无超时

| 事实 | 证据 |
|---|---|
| 参数只有 `question/options/multiSelect`，**没有 timeout** | `crates/pony-protocol/src/tool_contract/schema.rs:83-107` |
| `expires_at_ms` 是 inert trace 字段；`expire_asks` 明确不按墙钟回收 pending | `crates/pony-tools/src/tool_runtime.rs:192-215`、`ask_control.rs:381-393` |
| 只有「真人消息进入会话」才 supersede 旧 Ask | `control_plane/ask_plan_commands.rs:566-590`（ADR 0074） |

所以挂起是**无限期**的：不回答不取消，就永远停在那里。

---

## 2. DSH 的结构差异（为什么 DSH 不会出现这个症状）

| 维度 | DSH | 本仓 |
|---|---|---|
| `ask_user` 语义 | **turn 内的一次工具调用**：legacy 形态 block 在一个 Promise 上，turn 不结束；timed 形态前台等 120s，超时返回 `{pending:true}` 让模型**继续做独立工作**，问题仍可答 | **turn 级挂起**：`WaitingHost` ⇒ `suspended` 相位 ⇒ 整个 turn 收口（`tool-ask-user/src/index.ts:100-130`、`timed.ts:74-93` vs `names.rs:565`、`types.rs:248`） |
| 续轮前提 | `agent.status === 'idle'` + 无竞争队列 + subagent 静默 + checkpoint 落盘（`goal-round-driver/src/index.ts:106-117`） | 「上一轮 turn `Completed`」（`tui/goal_round_stop.rs:23`、`control_plane/mod.rs:2017`） |
| 作答后恢复 | **不需要专门代码**：ask 只是工具调用，作答后同一 turn 继续跑完，agent 回到 idle，driver 自然续下一轮 | 需要专门驱动者，而两端的驱动者都没接上 goal 循环（§1.3） |
| 轮次身份 | 结构化 `MessageSource{kind:'goal',goalId,revision,round}`，`pre-step` 校验 revision/round 匹配 | 中文文案前缀嗅探（`goal.rs:452-495`）+ `is_direct_human = !is_goal_round`（`runtime/mod.rs:448-449`） |
| blocked 门槛 | 默认 3 轮硬拒：`GOAL_TOOL_BLOCK_THRESHOLD`（`tool-goal/src/index.ts:263,271,305-312`） | 仅提示词软约束（`tools/prompt_contract.rs:45-48`），代码无门槛（`grep blockedAfter` 零命中） |
| 预算耗尽 | durable `block(round-limit)`，相位可见（`goal-round-driver/src/index.ts:183-186`） | 静默 `return Ok(response)`，goal 永远停在 `active+Armed`（`control_plane/mod.rs:2031-2034`、`2898-2901`） |
| 激活态 | `activation` 进程本地，重启即 disarmed（`goal/src/types.ts:44-49`） | `activation` 持久化 Armed（`goal.rs:852` 写入、`28-40` 序列化） |

**关键结论**：DSH 把「人类输入」建模成 turn 内部事件，所以 goal 循环的推进前提（idle）天然免疫 ask；本仓把「人类输入」建模成 turn 终态，于是 ask 与 goal 续轮在同一维度上互斥，必须显式接线才不丢——而现在没接。

---

## 3. 错漏清单（按严重度）

### P0-1 作答后 goal 循环不恢复（功能断点）
- 证据：§1.3 全表。
- 影响：`/goal` 的实际能力 = 「最多跑到第一次 ask（或第一次正文问句）」，之后必须人工再发消息。
- 补充：TUI 至少还有 `AskResumeTurn` 可复用；桌面端**完全没有 Rust 侧驱动者**，属于结构性缺失。

### P0-2 续轮判据双实现、能力不对等
| 能力 | TUI `run_goal_rounds` | core `execute_graph_run_stream` / `advance_graph_run`（桌面走这条） |
|---|---|---|
| 待作答 Ask/审批让位 | ✅ `tui.rs:10308-10322` | ❌ 无（`grep list_pending_asks mod.rs` 仅测试命中） |
| 连续失败熔断（2 轮） | ✅ `tui.rs:10494-10506` + `goal.rs:539,757-778` | ❌ 无（`MAX_CONSECUTIVE_GOAL_ROUND_FAILURES` 全仓仅 TUI 使用） |
| 失败轮不烧预算 | ✅ `advance_goal_round_for_outcome`（`goal.rs:784`） | ❌ 用 `advance_goal_round`（先 +1 再跑，`mod.rs:2035`、`2902`） |
| 停止可见 | ✅ `GoalRoundsStopped` 事件 + 状态行 | ❌ 静默 `return` |
- 影响：桌面端一次 429 会连烧 8 轮预算，用户零感知；TUI 与桌面同目标不同行为。

### P0-3 正文问号启发式压过 armed goal
- 证据：`planner.rs:88-104`、`774-797`。
- 影响：goal 轮只要以问号收尾就静默停机，且不产生任何 UI 信号；同时它使「`ask_user` 是唯一人类交互通道」的建模失效（暗通道）。

### P1-1 权限权威靠文案嗅探，Ask/审批恢复轮继承「人类」权威
- `is_direct_human = !is_goal_round_prompt(input.message)`（`runtime/mod.rs:448-449`）。
- `ASK_RESUME_CONTINUATION_PROMPT`、`APPROVAL_*_CONTINUATION_PROMPT` 都不是 goal round ⇒ 恢复轮被判为**直接人类**（`crates/pony-storage/src/session/store.rs:5193-5210` 的清单本身就是证据）。
- 后果：模型在"恢复轮"里可以 `create_goal` / `pause` / `edit` / `resume`（`crates/pony-tools/src/tools/misc.rs:117-127,202-230`），这些动作本应人类专属。DSH 用 open-turn 内的 `source.kind === 'user'` 判定，恢复轮不继承。

### P1-2 blocked 无硬门槛、预算耗尽不动相位
- blocked：提示词说"至少 3 轮"，代码只校验 `blocked_reason` 非空（`goal.rs:935-950`）⇒ 模型第 1 轮就能 blocked。
- 预算耗尽：`rounds_started >= max` 只 `return`，不落 `blocked`（`mod.rs:2031-2034`）。UI 永久显示"进行中"，`resume` 也会被"预算耗尽"拒绝（`goal.rs:922-927`）⇒ 死局且无解释。

### P1-3 桌面端缺少「续轮停止」事件面
- 仅 `goal://changed` / `goal://round` / `goal://cleared`（`src-tauri/src/goal_events.rs:24-28`），`goal://round` 只有 `status:"started"`。
- TUI 有 `GoalRoundsStopped`，桌面没有对应物 ⇒ 所有停止原因在桌面不可见。

### P2-1 双预算语义不一致（承诺 50 轮，实际 8 轮）
- `max_goal_rounds` 默认 50（`goal.rs:852`），但每个循环内 `MAX_CONSECUTIVE_AUTO_CONTINUES = 8` 到顶即停（`planner.rs:23`、`mod.rs:2026-2029`、`tui.rs:10330-10338`）。用户理解的"50 轮"永远到不了，且 TUI 报 `AutoContinueBudget`、桌面静默。

### P2-2 文档腐化（同一事实两个家）
- `docs/architecture/plan-goal-dsh-gap.md:17` 仍写 `crates/pony-agent-core/src/agent/goal.rs`；真源已迁 `crates/pony-storage/src/goal.rs`（`crates/pony-agent-core/src/agent/mod.rs:24` re-export）。
- 该文档 P0 建议「goal 工具权威校验」已由 `tools/misc.rs` 落地，但文档未标 superseded ⇒ 后来者会按过期结论重复施工。

---

## 4. 优化点（按 ROI 排序）

### O1（P0）把 ask 从「turn 终态」改成「turn 内等待」
两条路线，二选一：
- **A. 对齐 DSH timed 形态**：`ask_user` 增 `timeoutSeconds`（默认 120，`-1` 表示硬等），超时返回 `{pending:true, callId, message}`，模型继续做独立工作，问题保留可答。改动集中在 schema + `WaitingHost` 判定 + 前端卡片"待答"态。**这是根治**，一次解决 ask 阻断 + 无限期挂起。
- **B. 最小接线**：保留现语义，但把 `Suspended` 从「终止」改为「暂停」——宿主记录 `paused_awaiting_human`，`graph_resume_ask` 成功后重入 goal 循环。TUI 在 `AskResumeTurn` 分支完成后追加 `run_goal_rounds`；桌面端必须在 Rust 侧新增驱动者（不能依赖前端，现无 auto-continue）。

建议：先做 **O1-B** 止血（改 3 处、可测），再排 **O1-A** 作为根治（涉及协议与双端 UI，需要 ADR）。

### O2（P0）续轮判据单点化
抽一个纯函数 gate（输入：`cancelled / consecutive_failures / rounds_started / max_rounds / pending_host_requests / outcome`，输出：`Continue | Stop(reason)`），TUI 与 core 共用。直接堵掉 §3 的 P0-2 四项不对等，并让"停止可见"成为 gate 的返回值而不是各端自造。

### O3（P0）正文问号启发式让位于 armed goal
armed goal 存在时不走 `assistant_requests_user_input`；把"我要问用户"收口成**工具信号**（只有 `ask_user` 能收口）。这样"停机"永远有对应 UI 面板，可诊断。

### O4（P1）权威判定结构化
把 `DispatchContext.is_goal_round: bool` 升级为 `TurnKind { DirectHuman, GoalRound, AskResume, ApprovalResume, SubAgent }`，由宿主在构造 `TurnInput` 时写入（不靠文案嗅探）。`GhostCreate/Pause/Edit/Resume` 仅 `DirectHuman` 通过。

### O5（P1）补 DSH 的两个硬约束
- `blocked` 最小轮次（默认 3）硬拒，错误码 `goal_block_threshold`。
- 预算耗尽 ⇒ 落 `blocked` + `blocked_reason = round_limit`，UI 可见、`resume` 有明确指引。

### O6（P1）桌面端事件面补齐
新增 `goal://rounds_stopped`（载荷同 TUI `GoalRoundsStopped`：`reason/roundsStarted/maxRounds/consecutiveFailures`），`GoalBar` 展示停止原因。可观测性归属需先更新 `docs/observability.yaml`。

### O7（P2）预算语义统一
要么把 8 轮并入 `max_goal_rounds` 单闸门，要么显式声明"每次用户输入最多 8 轮、目标预算 50 轮跨多次输入"，并在 `/goal show` 与 GoalBar 上分别展示，避免"承诺 50 实跑 8"。

### O8（P2）文档收口
更新 `docs/architecture/plan-goal-dsh-gap.md`：路径改 `crates/pony-storage/src/goal.rs`，已落地项（权威校验、goal 事件发射）标 superseded，未落地项（失败熔断未进 core）保留。

---

## 5. 附：本次未取证 / 需决策的点

- 桌面端是否**有意**不自动续跑（例如产品上希望每轮都经人类确认）：文档未见声明。若是，则 §3 P0-1 的性质从"缺陷"变为"未声明的产品约束"，但仍需在 UI 上说明"目标已暂停，发送消息继续"。
- `ask_user` timed 形态是否要改协议（`ToolDefinition` 入参）——涉及 `pony-protocol` 契约与两端 UI，需独立 ADR。
- TUI 与桌面是否应统一由 core 驱动 goal（现状 TUI 自己一套 + core 一套），属结构性重构，建议单独立项评估。
