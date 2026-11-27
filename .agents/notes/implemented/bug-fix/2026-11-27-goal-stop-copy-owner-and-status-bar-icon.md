# Agent Note: 停止原因不再冒认阶段，底栏 goal 段收敛为状态图标

Status: implemented

## Problem

用户在 Ctrl+T 抽屉的 Goal 页签里同时读到两条互相否认的结论：

```
◎ 目标 · 进行中 · 2/5 轮
自动续轮已停止｜…        ← 旧实现渲染为「目标已暂停：续轮已停止｜…」
```

旧 `GoalRoundStopReason::headline()` 对 9 个变体一律返回 `目标已暂停：…`（`FirstRoundNotSucceeded`
返回 `目标已暂停：本轮未成功`）。但 `GoalPhase` 在续轮停下时通常仍是 `Active`——**续轮循环停了，
目标没有被暂停**。于是同一个状态被两处分别声明，用户只能读到矛盾。

同一屏的第二处噪声来自底栏左槽 `◎ 目标 {round}/{max}{·停|·未续|·暂停|·阻塞}`：抽屉本来就逐字
显示 `轮次 2 / 5`，底栏再放一个 `2/5` 只是挤占宽度，且在目标卡住时被读成「进度」。第三处是
`⚡ 智能体 2·✘1`——`✘` 直接贴在数字前，读起来像「2 个里 1 个报错」，实际是「2 个在跑 + 1 个需关注」，
两种计数被一个 `✘` 糊成一个数字串。

## Decision

1. **停止原因的措辞只描述它自己的事实**：`headline()` 改为 `自动续轮已停止` /
   `自动续轮未启动` / `本会话已有续轮循环在运行`，不再出现「目标已暂停」。
   **唯一例外是 `TurnSuspended`**：它表达的「续轮让位等你作答、作答后自动继续」，说成「已停止」
   会让用户以为目标已死从而不去作答（PR #106 的 O1-B 可见面契约），故取
   `续轮暂停：等你作答`——主语仍是「续轮」而非「目标」，且用「续轮暂停」避开
   `GoalPhase::Paused` 的既有译词「已暂停」，不冒认阶段。
   阶段措辞的唯一家仍是 `goal_panel::phase_label` / `task_center::phase_text`；
   `visible_line()` 仍是三段式（`headline｜detail｜action_hint`），因此
   `errors.rs::goal_stop_severity_for_line` 的身份比对、`b193_2` 的
   「`FirstRoundNotSucceeded` 必含『未启动』且不含『已停止』」判据全部原样成立。
   `FirstRoundNotSucceeded.detail()` 同步去掉与 headline 重复的「自动续轮未启动」，只留「首轮失败」。
2. **底栏 goal 段只报状态，不报轮次**：`◎ 目标`（续轮在跑）/ `◌ 目标`（续轮未在跑）/
   `⏸ 目标`（阶段暂停）/ `⛔ 目标`（阻塞）；轮次只在抽屉 Goal 页签出现。图标保持中性
   （「未在跑」），不派生 `·停` / `·未续` 文字——B193 的语义区分由面板的 `visible_line()` 逐字承担，
   底栏的常驻窄槽不再复制。
3. **底栏智能体段改用括号图标计数**：`⚡ 智能体 {running}(⚠{attention})`，与任务段
   `☑ 任务 4(⛔1)` 同语法；`✘` 不再出现在数字前。
4. 回归测试：`tui_task_visibility.rs` 增加「底栏 goal 段无轮次、只有图标」与
   「Paused/Blocked 用图标表达」两组断言；`goal_panel.rs` 增加
   `stopped_reason_never_contradicts_active_phase`（阶段进行中时全量行不得出现「已暂停」）；
   `tui_interaction_acceptance.rs` 的 goal 指标契约改为「底栏不含 `3/8`、抽屉仍逐字含 `3/8 轮`」。

## Alternatives considered

- **同时把 GoalPhase 改成 `Paused`**（让「已暂停」变成真话）：续轮停下不等于目标被暂停，
  改阶段会让 `/goal resume` 之外的读方（`active_goal_for_session` 只认 `Armed + Active`）
  把目标判成不可续，等于把展示层的一句话变成领域状态变更；且用户随后发一条消息就期望它继续跑，
  阶段一旦落成 `Paused` 就得先恢复再续轮。展示层矛盾用展示层修。
- **保留 `目标已暂停` 前缀、只在面板里隐藏**：面板与消息区/状态行共用 `visible_line()`，
  隐藏就要再分叉一条「面板专用文案」，等于把「一个事实一个家」拆成两个家，且下一次改文案必漂移。
- **底栏继续显示轮次、只删 `·停`/`·未续`**：宽度没省下来，用户抱怨的两件事里这一件没解决；
  轮次在抽屉里是逐字可见的，删掉不构成信息降级。
- **把 `attention` 并入运行数显示 `⚡ 智能体 2/3`**：读者无法区分分子分母的语义（谁在跑、谁需关注），
  与任务段既有的括号语法也不一致。

## Consequences

- `.dev-team/contract-matrix-pa171.md` 的 `Amendment: b193-2` 逐字文案表由本次追加的
  `Amendment: goal-stop-copy-owner` 取代（表内 9 行 headline、1 行 detail、2 行状态条说明）；
  变体集合、`code()`、`severity()`、`enters_history()`、`never_started()` 一律不变。
- 底栏 goal 段从此不携带任何数字：想读轮次去 Ctrl+T → Goal 页签。
- `never_started()` 仍被 `headline()` 与契约测试消费，保持为唯一判据，未降级为死代码。

## Commit boundary（同工作区并发写入面归属）

本卡提交面（10 个已改文件 + 2 个新文件，全部已由上方测试取证覆盖）：

| 类别 | 路径 |
|---|---|
| 文案真源 | `crates/pony-storage/src/goal.rs`（`headline()` / 一处 `detail()`） |
| 渲染层 | `crates/pony-tui/src/tui/status_bar.rs`、`crates/pony-tui/src/tui/goal_panel.rs` |
| 注释同步 | `crates/pony-tui/src/tui.rs`（一条 doc 注释里的展示通道描述） |
| 测试 | `crates/pony-tui/tests/tui_suite/{tui_task_visibility,tui_interaction_acceptance}.rs`、`crates/pony-agent-core/tests/tools_stop_suite/b193_2_goal_stop_visibility_contract.rs`（仅注释） |
| 契约文档 | `.dev-team/contract-matrix-pa171.md`、`.dev-team/contract-matrix-pa172.md`（只追加 Amendment 与指针，不改写冻结表） |
| 可观测归属 | `docs/observability.yaml`（邻接变更说明，提交信息须带 `[decl-change-approved]`） |
| 记录 | 本文件 + `backlog/log/20261127T090044Z-tui-48a5.jsonl` |

**不属于本卡、提交时必须排除**（同工作区另一并发会话的未跟踪件）：

- `.agents/notes/proposed/bug-fix/2026-11-27-goal-round-gate-and-ask-resume.md`（goal gate 收敛提案，proposed）
- `backlog/log/20261127T082001Z-general-c97f.jsonl`（该会话批次 B1795767639536）
- `docs/analysis/goal-ask-blocking-audit-2026-11-26.md`（该会话的审计输入）

拆分判据：本卡不引用上述三件中的任何内容（`grep` 无命中），上述三件也不引用本卡的改动面
⇒ 两方可独立提交，无先后依赖。该会话的 `proposed` ADR 冻结了「`TurnSuspended` 走 `headline()`
`_` 兜底臂」这一现状（`goal.rs:656-662`），本卡只换字面量未拆臂 ⇒ 其前提仍成立，不需要它先落。
