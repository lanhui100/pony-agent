# Agent Note: goal 会话归属收口——删除 default 兜底与 TUI 跨会话串话治理（B152）

Status: implemented

## Problem

TUI 的 goal 在存储层本已按会话绑定（`SESSION_GOALS: HashMap<session_id, GoalData>`，
`active_goal_for_session` 精确匹配且要求 `phase=active && Armed`），但**运行层**留了多处
「没有会话 → 落到 `default` 键」的兜底，以及三条缺失的会话隔离判据。实测症状：会话中断后
继续（`/resume`、Ctrl+↑ 返回、或另一会话发消息）时，**别的会话的 goal 被读出来继续跑**。

逐条证据（修复前）：

1. **`default` 共用桶**（违反 `docs/architecture/plan-goal-contract.md` §6 的
   「禁止 `"default"` 兜底冒充」）：
   - `agent/goal.rs::resolve_session` 把 `None` 折叠成 `"default"`，`create/goal/get/update`
     三个入口都用它 ⇒ A 会话的 goal 可能被写成/读成 B 的共同键；
   - `agent/control_plane/mod.rs::execute_graph_run_stream` 与 `advance_graph_run` 两处续轮
     `response.run.session_id.unwrap_or("default")` ⇒ 无会话事实的 run 直接续 `default` 桶的目标；
   - `bin/tui.rs` 三处轮次线程 `turn_input.session_id.unwrap_or_else(|| "default")` ⇒ 轮次与
     `run_goal_rounds` 都挂在 `default` 分区上。
   本机 `goal_store.json` 里确实存在 `session_id="default"`、`activation=armed` 的条目，
   证明这条兜底有真实写入点。
2. **取消令牌是进程级**：`GoalRoundCancelToken` 原为 `Arc<AtomicBool>`，任何会话的新消息都
   `reset()` ⇒ 用户切到 B 会话说一句话，A 会话仍在跑的续轮循环被**重新武装**并继续开轮。
3. **`GoalUpdated` 事件无会话标签**：续轮线程收口后直接 `state.set_goal(goal)`，把 A 会话的
   横幅画到 B 会话头上（`GoalRoundsStopped` 早已补了 `session_id` 判据，`GoalUpdated` 没有）。
4. **切会话不终止续轮、也不装载 goal**：`/clear`、`/resume` 只 `reset_session_scope`，
   不 cancel；且切换守卫只看 `is_streaming`，两轮之间存在「`StreamCompleted` 已到、
   下一轮 `turn:started` 未到」的假阴性窗口。`hydrate_state_from_session` 不读 goal ⇒
   「中断后继续」时目标不显示，等下一轮 `StreamCompleted` 回读才**凭空冒出并自动续跑**。

## Decision

把「会话归属」在**读取面、取消面、事件面、切换面**四个位置一次性收口，全部 fail-closed：

1. **store 层**：`goal.rs::resolve_session(Option<String>, entry) -> Result<String, String>`。
   缺 `session_id` / 空白 ⇒ 显式 `Err("<entry> requires a non-empty session_id …")`。
   `create_goal` / `get_goal` / `update_goal` 一律 `?` 传播；`default` 兜底被删除，
   「借用别的会话的目标」不再是一种可能。工具入口仍由 `fill_session_from_context` 注入
   当前会话，因此正常调用面不受影响。
2. **宿主续轮**：`control_plane` 两处续轮先用 `session_id.as_deref().filter(non-empty)`
   取真实会话，缺失即 `return Ok(response)`（不续轮、不猜会话）。
3. **取消面按会话分区**：`GoalRoundCancelToken` 改为
   `Arc<Mutex<HashSet<String>>>`，API 变成 `cancel_for(session)` /
   `reset_for(session)` / `is_cancelled_for(session)`；空会话 id 在 `is_cancelled_for`
   上 fail-closed 返回 `true`。Esc/Ctrl+C 只取消当前会话，新用户消息只重新武装它自己。
4. **事件面**：`UiEvent::GoalUpdated` 变为结构体变体并携带 `session_id`；事件循环在
   `session_id != state.session_id` 时 `continue`（与 `GoalRoundsStopped` 同源判据）。
5. **切换面**：新增唯一收口 `tui::switch_session_scope(state, control_plane, target, goal_cancel)`：
   停**旧会话**续轮 → `reset_session_scope` → `hydrate_state_from_session` →
   `hydrate_goal_state`（读 `get_goal` 并投影横幅）。`/resume`、队友会话进入/左右切换、
   Ctrl+↑ 返回主会话全部改走它；`/clear` 只停旧续轮 + 换新 id（刻意不 hydrate，
   保留当前 workspace 绑定）；启动期 `pony resume <id>` 在 hydrate 后补 `hydrate_goal_state`。
   切换守卫判据从 `is_streaming` 改为「**在飞轮次线程**」`is_turn_loop_in_flight`，
   并把该守卫接到 `/clear` 与 `/resume` 上（原先只保护子会话切换）。
6. **线程入口**：`require_turn_session_id(&TurnInput)` 取代三处 `default` 兜底；缺失时补发
   `UiEvent::StreamFailed`（可见终态，不会让 `is_streaming` 永真）。会话缺失的 sender 走
   `tx_session_fail` 克隆，避免内层 `move` 闭包抢走 outer 的 `tx_turn`。
7. **存量迁移**：`load_goal_store` 解析成功后调用 `purge_legacy_goal_entries`，剔除
   「key 去空白为空 / key 是 `default` / 内嵌 `session_id` 是 `default`」三类旧兜底产物，
   并把被删 key 作为 `explicit_removals` 交给 `persist_goal_store` 立即回写——
   `persist_goal_store` 是「先合入磁盘快照再写」的，只清内存 map 会让旧条目在下一次
   任意写盘时原样复活（与 `clear_goal` 被复活同源）。回写失败仅打日志，不影响读面，
   下次加载重试。key 与内嵌 `session_id` 不一致但两者都非 `default` 的记录一律保留：
   那不属于本缺陷定义域，误删会丢真实会话的目标。

## Alternatives considered

- **只加 UI 侧过滤，保留 `default` 桶**：过滤能挡住横幅错画，但挡不住**续轮本身**——
  宿主循环仍会按 `default` 桶推进别会话的目标（烧 token、写历史），属于治标。
- **给 `GoalData` 增加 `owner_process` / 运行期租约字段**：能进一步防止跨进程误读，
  但 `goal_store.json` 是磁盘契约（`docs/architecture/plan-goal-contract.md` §8 冻结路径），
  本次病根是「无会话事实时的兜底」而非「存在多写者」，先删兜底即闭合，schema 扩展留待
  真有跨进程并发写需求时再立法。
- **保留进程级取消令牌，改用「epoch + 归属会话」**：需要续轮循环持有 epoch 并在每轮比较，
  语义等价但状态更复杂；按会话分区直接消除了「B 的重置影响 A」这一类，代价只是每个会话
  一条 `HashSet` 项。
- **切会话时直接 `stop_turn` 掉旧会话在飞轮次**：跨会话中断别人的轮次越权（旧会话可能
  正是用户想看的结果）；改为「守卫拦下切换 + 只取消续轮」，把决定权留在用户手里。
- **`/clear` 也走完整 hydrate**：会让「全新会话」继承启动目录的 workspace 绑定，
  悄悄改写用户当前 workspace，属于顺手引入的回归，故 `/clear` 只换 id + 复位作用域。

## Consequences

- 任何缺少 `session_id` 的 goal 读写都变成**显式错误**，不再有静默的 `default` 桶；
  存量 `goal_store.json` 里的 `default` 条目不再被任何读取方命中，并由决策 7 的加载期迁移
  自动删除 + 回写（无需人工清理；迁移前已存在的进程若仍持有旧 map，其下一次写盘只可能
  写回自己会话的 key，不会重建 `default` 桶，因为写入点已被 fail-closed 关掉）。
- 会话切换后 goal 横幅立即与 core 对齐：属于目标会话就显示，不属于就不显示；
  不再出现「目标凭空出现并自动续跑」。
- 切换会话的判据变严：在飞轮次（含两轮之间的窗口）期间 `/clear`、`/resume`、队友切换、
  Ctrl+↑ 都会被拦下并给出提示 —— 用户必须先停止或等本轮收口。
- `pony resume` 不带 id 时仍按「最近主会话」挑选（`session/startup_meta.rs`），
  这是**选择**语义而非归属 bug；但因切换后立即 hydrate goal，「续到了哪个会话」现在
  在界面上可判读（横幅即证据）。
- 新增/改写回归门：`agent::goal::wave1_tests::missing_session_id_is_rejected_instead_of_falling_back_to_default`、
  `tests/tui_batch2_regression.rs::n8b_cancel_token_is_partitioned_per_session`（跨会话取消分区）、
  `n8b_cancel_token_drives_the_round_abort_decision`（按会话 API 改写）、
  `tests/pa109_goal_acceptance.rs`（改用显式会话 id，不再依赖 `default` 槽位）、
  `agent::goal::wave1_tests::legacy_default_entries_are_purged_on_load`（三类旧兜底产物
  加载即清、磁盘同步删除、下一次普通写盘不复活）。
