# Agent Note: TUI 会话「自己中断」的两个可见性缺口——goal 首轮门静默与排队消息只有计数

Status: implemented

## Problem

用户在 2026-11-15 的连续几个 TUI 会话里都遇到「会话自己中断，发一句『继续』才继续」。取证（`~/.local/share/PonyAgent/session-logs/--home-dm-pony-agent--/pa_01a1df0a…/session.jsonl` 与 `~/.pony-agent/tui-logs/tui-runtime.log`）指向两个**可见性**缺口，而不是调度器真的停了：

1. **goal 首轮门静默**：PA-171 C1-T1 的首轮门规定「只有首轮真正成功收口才进入自动续轮」，但实现是
   `if !first_completed { return; }`（`crates/pony-agent-core/src/bin/tui.rs` 的轮次线程体）。首轮终态为
   `Failed`/`Cancelled`/`Suspended`/`Unknown` 时既不产生 `UiEvent::GoalRoundsStopped`，也不写状态行与
   消息区 —— 用户只看到「这一轮结束了」，不知道**自动续轮从未启动**。该会话首个 turn 即
   `turn_end reason=error`（provider `SendRequest` 42s），其后无任何停止原因可见。
2. **排队消息只有计数**：`TuiState::queued_user_messages` 只存正文快照，渲染仅经
   `queue_indicator_line` 出「N 条排队 · 本轮结束后自动发送」。用户看不到排队的是哪几条、不能删、
   也不能插队；流式期打入的「继续」在界面上只剩一个数字，容易被读成「没发出去」。桌面端早已有
   `QueuedMessagesBubble.vue` + `promoteToSteer` + Alt+S，TUI 未对齐。

## Decision

1. 新增 additive 变体 `GoalRoundStopReason::FirstRoundNotSucceeded`（`crates/pony-agent-core/src/agent/goal.rs`），
   逐字文案 `goal 续轮未启动（首轮未成功收口，未进入自动续轮）`——用「未启动」而不是「已停止」，
   因为续轮从未进入。日志 `reason` tag 为 `first_round_not_succeeded`。
   > **⚠ 本条逐字文案已被取代（2026-11-20）**：现行为
   > `目标已暂停：本轮未成功｜首轮失败，自动续轮未启动｜发「继续」重试`。
   > 变体、触发路径、日志 tag 全部不变；「未启动 vs 已停止」的语义区分**保留并加强**（`never_started()`）。
   > 改动理由与全表见 [2026-11-20-goal-stop-reason-structured-split](2026-11-20-goal-stop-reason-structured-split.md)。
2. 「首轮终态 → 停止原因」收成纯函数 `first_round_stop_reason`（新模块
   `crates/pony-agent-core/src/bin/tui/goal_round_stop.rs`），映射：`Completed ⇒ None` /
   `Cancelled ⇒ Cancelled` / `Suspended ⇒ TurnSuspended` / `Unknown ⇒ TurnUnclassified` /
   `Failed ⇒ FirstRoundNotSucceeded`。轮次线程只负责把结果交给既有 `report_goal_rounds_stopped`
   （状态行 + 消息区 + 结构化日志三通道一次到位），不复制可见性逻辑。
3. 排队消息正文进**内联面板**（新模块 `crates/pony-agent-core/src/bin/tui/queue_panel.rs`）：
   与 Ask 覆盖层同构（下缘贴 Composer 色带、不参与消息区几何，契约 C8 不变），最多 3 条 + 「还有 N 条」，
   Ask 面板在场时让位（计数仍在指标行）。
4. 新增 `Alt+S` = 立即发送队首，裁决是纯函数 `queue_immediate_action`：
   空队列 ⇒ 提示；有在飞轮次 ⇒ 走与 Esc 同一停止通道（先置 `goal_round_cancel`，再取 turn id 调
   `stop_turn`，绝不持锁回调），收口后由既有单飞闸门自动派发；空闲 ⇒ 立刻经 `QueuedAutoSubmit`
   回灌提交（不注入 `InputKey(Enter)`，保留 C21 的空闲校验）。输入框有草稿时拒绝覆盖（C15 前哨同口径）。
5. 契约矩阵 `.dev-team/contract-matrix-pa171.md` 同步登记第 10 变体与其触发路径；快捷键帮助
   （`tui/shortcuts.rs`）登记 `Alt+S`。

## Alternatives considered

- **首轮失败也进续轮循环（去掉首轮门）**：会让一次上游 429 被重放成多轮，正是 PA-171 要消灭的放大面；
  被否决，首轮门语义不变，只补可见性。
- **复用 `ConsecutiveFailures { failures: 1 }` 表达首轮失败**：文案会读作「连续 1 轮失败」，暗示已经连跑
  多轮；与「续轮从未启动」语义冲突，被否决（additive 变体的成本低于一次语义漂移）。
- **排队正文用消息区 System 消息逐条落历史**：会在每帧/TTL 内反复写历史，污染会话且与「排队消息尚未
  发送」的事实矛盾；改为覆盖层面板（可随队列增删实时反映），被否决。
- **Alt+S 直接并发起第二个 turn（真插队）**：会与在飞轮次交叠写同一会话历史（C10/C17 的不变量）；
  改为「先停止当前轮，再自动派发」，语义上仍是用户可感知的「立即发送」。
- **只加文案不提键位**：用户仍需等当前轮结束；「能看见」之外还要求「能催」，故补 Alt+S 而非仅补面板。

## Consequences

- 首轮失败/取消/挂起现在都会留下一条可见结论 + 一条 `goal_rounds_stopped` 结构化日志（`reason` 可按
  `first_round_not_succeeded` 单独统计）。
- 排队消息首次在 TUI 可见与可插队；面板是覆盖层，消息区几何与 C8 契约不变，Ask 优先。
- 新增两个模块文件（`goal_round_stop.rs` / `queue_panel.rs`），`tui.rs` 只增加接线与一个键位分支，
  规模棘轮（`npm run size:check`）不受影响（`tui.rs` 已有 file-size 豁免围栏）。
- 与目标面板、Goal 横幅的既有语义无冲突：`goal_rounds_stopped` 的复位点仍在新轮起点，
  「未启动」状态同被复位。
