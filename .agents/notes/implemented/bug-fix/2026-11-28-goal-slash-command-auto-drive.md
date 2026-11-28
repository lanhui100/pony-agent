# Agent Note: `/goal` 斜杠命令自动起轮与运行中控制对齐 DSH

Status: implemented

- 日期：2026-11-28
- 关联旧记录：[2026-11-12-goal-slash-command-and-clear.md](../feature/2026-11-12-goal-slash-command-and-clear.md)（本条局部取代其中「`/goal` 永不发起 turn」的条款：`create` / `resume` 由宿主自动起轮，`show` / `pause` / `complete` / `clear` 仍不发起模型轮次）
- 对照参考：`/home/dm/deepseek-harness/packages/goal/`（`command-goal`、`goal-round-driver`）
- 交付面：`crates/pony-storage/src/goal_command.rs`、`crates/pony-tui/src/tui/goal_command.rs`、`crates/pony-tui/src/tui/goal_panel.rs`、`crates/pony-tui/src/tui.rs`、`src-tauri/src/lib.rs`、`src/lib/chat/slash-commands.ts`、`src/stores/runtime/orchestration.ts`、`src/components/HomeWorkspace.vue`

## Problem

1. **只落盘不启动**：直接输入 `/goal <目标>`（含 completed 态 `/goal edit <目标>`）或 `/goal resume` 时，两端只调用 `execute_goal_command` 写入 `goal_store.json` 并提示「自动续轮已启动」，但既不发起首轮也不触发 `run_goal_rounds` / `start_graph_run_stream`，目标停在 `active + armed, roundsStarted = 0` 空转（实证：`sessions.db` 与运行时日志中 0 条消息、0 次 turn）。
2. **运行中控制断裂**：TUI 流式期输入 `/goal pause` / `clear` / `complete` 只写盘不置位 `goal_round_cancel` 也不调 `stop_turn` 中止当前在飞轮次；桌面端 `executeSlashCommand` 在 `isSubmitting` 期间直接一刀切拦截所有斜杠命令（包含 `/goal` 查看与暂停）。
3. **提示文案漂移**：`GOAL_PANEL_HINT` 仍宣传不存在的 `/goal blocked <理由>` 语法（输入会被当作新建目标报错）；`phase_commands` 对 `active + Disarmed` 仍只提示 `/goal pause` 而非 `/goal resume`。

## Decision

1. **Core 命令输出结构化驱动信号（`crates/pony-storage/src/goal_command.rs`）**：
   - `GoalCommandOutcome` 新增可选字段 `next_round_prompt` / `next_round_display` 与方法 `should_auto_drive()` / `should_cancel_in_flight()`；
   - 仅当 `action ∈ {"create", "resume"}` 且目标满足 `active + Armed + rounds_started < max_goal_rounds` 时填充 `next_round_prompt`（复用 `build_goal_round_message`）；`show` / `pause` / `complete` / `clear` / 非 completed `edit` 保持不产出起轮提示；
   - Core 不再预先拼接「自动续轮已启动」，由宿主层按实际驱动结果（成功起轮 / 已有运行中轮次延后衔接 / 待作答提问阻挡 / 缺 API Key）如实反馈；
   - `phase_commands` 按 `(phase, activation)` 分支，对 `active + Disarmed` 提示 `/goal resume`。
2. **TUI 宿主联动（`crates/pony-tui/src/tui/goal_command.rs` + `tui.rs`）**：
   - 新增 `handle_goal_command_with_driver`（主体逻辑留在子模块以守住 `tui.rs` 规模）：`pause`/`clear`/`complete` 置位 `goal_round_cancel` 并释放锁后调 `control_plane.stop_turn`；`create`/`resume` 经 `evaluate_tui_auto_drive`（检测在飞态、待作答 Ask/审批、Provider API Key、`goal_round_gate`）放行后直接拉起 `run_goal_rounds` 线程；
   - `run_goal_rounds` 返回 `needs_streaming_reset`，若未产生终态事件即退出则发送 `UiEvent::GoalAutoStartAborted` 复位 `is_streaming`，杜绝界面假流式卡死；
   - `goal_panel::GOAL_PANEL_HINT` 移除不存在的 `blocked <理由>`。
3. **Tauri 与桌面端联动（`src-tauri/src/lib.rs`、`slash-commands.ts`、`orchestration.ts`、`HomeWorkspace.vue`）**：
   - `start_graph_run_stream` 补齐 `authorization_modes.record(...)`，确保 `/goal` 新建会话首轮挂起作答后重入续轮能继承授权模式；
   - `orchestration.ts` 新增模块级 helper `startGoalRound(store, prompt, displayMessage)`（复用 `resolveSubmissionForStore` / `applyStreamStartOutcome`，保持 Pinia store 83/18/97 冻结表面不变；不推入伪造用户气泡，通过 `is_goal_round_prompt` 在后端判为 `TurnKind::GoalRound`，首轮结束后由 `goal_round_loop` 自动续跑）；
   - `HomeWorkspace.vue` 的 `isSubmitting` 一刀切拦截豁免 `command === "goal"`（`/plan`/`/tasks`/`/compact` 冻结行为不变）：运行中 `pause`/`clear`/`complete` 调 `stopTurn()`，运行中或有待作答 Ask 时 `create`/`resume` 不并发起新轮并如实提示，空闲时调用 `startGoalRound`。

## Alternatives considered

1. **保留「`/goal` 只写存储、等用户再发一条普通消息才启动」**
   否决：既违背「输入 `/goal <目标>` 即由系统自主推进」的心智与「自动续轮已启动」既定文案，也与 DSH `command-goal` + `goal-round-driver` 行为不一致；用户实测表现为命令无效。
2. **桌面端前端直接把 `/goal <目标>` 透传成普通聊天消息 `submitTurn()`**
   否决：`/goal` 命令语法包含 `edit`/`pause`/`resume`/`complete`/`clear`/`show` 与冲突拒绝规则，透传给模型不仅会在聊天区产生冗余对话记录，还无法在运行中控制已有目标；必须在前端只执行 `goal_command` 状态机后，由 `startGoalRound` 走规范的 `<goal_round>` 启动入口。
3. **在前端重复实现 `build_goal_round_message` 模板**
   否决：跨端模板漂移会破坏 `is_goal_round_prompt` 识别与契约单源原则，因此由 Rust `GoalCommandOutcome` 直接下发 `nextRoundPrompt` / `nextRoundDisplay`。

## Consequences

- 正向：TUI 与桌面端输入 `/goal <目标>` 或 `/goal resume` 后立即启动首轮/续轮执行；运行中 `/goal pause|clear|complete` 立即中止当前轮次并取消后续续轮；提示文案与实际能力一致。
- 边界：`create`/`resume` 启动失败或被前哨门（运行中、待作答 Ask、缺少 API Key）延后时，目标状态仍保留为 `active + armed`，回执明确告知下一步条件而不谎报已启动。
