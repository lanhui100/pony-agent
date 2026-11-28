# Agent Note: 挂起文案去重与停止留痕的轮次作用域复位

Status: implemented

## Problem

用户在 Ctrl+T 任务抽屉的 Goal 页签里读到这一屏：

```
◎ 进行中 · 2/5 轮
续轮暂停：等你作答｜本轮在等你作答/审批｜作答后自动继续
```

反馈是「完全不知道这句的设计意图，或者根本就是多余的」。查证后有两处独立缺陷：

1. **同一行自我复述**。`GoalRoundStopReason::TurnSuspended` 的 `visible_line()` 由
   `headline｜detail｜action_hint` 拼成，其中 `headline`「续轮暂停：等你作答」与 `detail`
   「本轮在等你作答/审批」说的是同一件事——「等什么」在一行里出现了两次。同时「续轮」是
   实现侧机制词（自动续轮循环），用户侧没有这个概念可挂靠，于是这句话只剩噪声。
   该变体同时服务 Ask 与审批（`TurnSuspended` 是两者共用的挂起信号），旧 headline 只写
   「等你作答」，审批场景下也不准确。

2. **留痕不随状态失效**。`TuiState.goal_rounds_stopped`（Goal 页签停止行与底栏 `◌`/`◎` 图标的
   唯一数据源）原先只在两处置空：composer 派发点（`tui.rs` 用户提交消息）与
   `spawn_goal_auto_rounds`（`/goal` 自动起轮）。而 `UiEvent::AskResumeTurn` 与
   `UiEvent::ApprovalResumeTurn` 这两条「作答后续轮」入口都不经那两处 —— 于是用户已经作答、
   目标已经重新开跑，面板仍宣称「等你作答」，底栏也仍停在「续轮未在跑」。这与上一批
   `2026-11-27-goal-stop-copy-owner-and-status-bar-icon` 修掉的「同一状态两个结论」同族，
   只是这次矛盾的来源是**时间**而不是**措辞**。

## Decision

1. `TurnSuspended` 的文案收敛为 `暂停中：等你作答/审批｜作答后自动继续`：
   - 「等什么」只由 `headline()` 说一次，`detail()` 归 `None`（该变体没有第二层原因可讲）；
   - 去掉实现词「续轮」，改用用户已在别处见过的「暂停」+「作答/审批」+「作答后自动继续」；
   - Ask 与审批共用同一信号 ⇒ 「作答/审批」必须写全，不得只说作答；
   - 仍**不**使用 `GoalPhase::Paused` 的既有译词「已暂停」（`goal_panel` 的
     `stopped_reason_never_contradicts_active_phase` 仍锁这条）。
2. `action_hint()` 保持 `作答后自动继续` 不变：它与 `headline()` 一起构成 O1-B 的可见面契约
   ——「暂停 ≠ 终止、作答即解锁」必须能被用户读到，否则用户不会去作答。
3. **复位点收敛到轮次起点**：把 `goal_rounds_stopped = None` 的唯一写点移进
   `TuiState::begin_turn_usage()`（`turn:started` 事件与所有开轮入口的公共锚），删除 composer
   派发点与 `spawn_goal_auto_rounds` 里的两行重复赋值。语义是「任一轮次起点 ⇒ 上一轮那句
   停止原因作废」，因此 Ask 作答 / 审批恢复 / goal 自动续轮三条入口自动被覆盖。
4. 回归钉子：`goal::b130_event_tests::suspended_line_states_each_fact_once`（逐字冻结 +
   「等你作答」只出现一次 + 不含「续轮」+ 含「审批」）、
   `tui::turn_end_status_noise_tests::new_turn_start_clears_stale_goal_stop_reason`、
   `tui::turn_end_status_noise_tests::stop_reason_survives_until_next_turn`。

## Alternatives considered

- **只改文案、不动复位点**：用户已作答后仍会看到「等你作答」——那才是「多余」的主因；只改字
  等于把一句仍会过期的文案润色一遍。
- **在 Ask/审批两个续轮分支各写一行 `goal_rounds_stopped = None`**：能治当前症状，但复位点变成
  三处（外加 goal 自动起轮与 composer），下一个开轮入口必然漏写——这正是本次缺陷的成因。
  收在 `begin_turn_usage()` 才是「一个事实一个家」。
- **把 `goal_rounds_stopped` 改成按 `pending_request` 派生（有待决请求才显示）**：显示层不再过期，
  但底栏 `◌`/`◎` 也读同一字段，派生值无法表达「循环停了但没人等」这一类停止原因；且派生逻辑会
  与 `apply_goal_rounds_stopped` 形成两个真值源。
- **保留 `detail` 只把它改写成别的说明（如「本轮未推进」）**：该变体的语义就是「等人」，
  任何补充说明都会与 `headline` 同义；`detail` 归 `None` 是唯一不产生复述的写法。
- **在挂起时直接说「目标已暂停」**：目标此刻通常仍是 `GoalPhase::Active`，会与同屏阶段行打脸
  （`2026-11-27-goal-stop-copy-owner-and-status-bar-icon` 已否决过一次）。

## Consequences

- 用户可见影响：Goal 页签/Ctrl+G 面板/状态行里，挂起原因从三句复述变成一句可读的话；
  作答之后该行与底栏 `◌` 会立刻消失（状态行文案由下一轮进度短语覆盖）。
- `.dev-team/contract-matrix-pa171.md` 追加 `Amendment: suspended-copy-dedup-and-reset`，
  supersedes 上一条 Amendment 的 `TurnSuspended` 逐字行与 `detail` 描述；变体集合、
  `code()`/`severity()`/`enters_history()`/`never_started()` 不变。
- `begin_turn_usage()` 的职责从「usage 回执复位」扩为「轮次作用域复位」，方法文档已同步；
  该方法同时被 composer 派发点、Ask 续轮、审批续轮、`/goal` 自动起轮与 `UiEvent::TurnStarted`
  调用，是轮次作用域状态的唯一公共锚。
- 未覆盖的相邻事实（不做，登记在案）：`/goal pause` 收口时循环上报的 `Cancelled`
  仍带 `发消息可继续` 的 hint，而暂停态其实需要 `/goal resume`；这与本次「挂起文案 / 复位点」
  不是同一根因，另开卡处理。
