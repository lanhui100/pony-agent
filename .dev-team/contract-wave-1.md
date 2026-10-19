# Wave-1 冻结契约：计划/目标链路修复（v1）

主线基线：commit 4e4fedc（main）。所有验收测试以本契约为唯一真值；波内不得就地修改契约。

## C1 目标真值源（goal-core）
- 单一 API（新）：`crate::agent::goal::active_goal_for_session(session_id: &str) -> Option<GoalData>`；
  过滤条件：`phase == "active" && activation == Armed`，按 session 精确匹配。
- `DefaultGraphPlanner::decide_after_turn` 目标解析顺序：
  1. `context.session_id` → `active_goal_for_session` 命中 → 用其 `objective`（**且必须 armed 才允许 Continue**）；
  2. 未命中 → 回退 `run.goal` + 关键词启发（保留既有行为，供 run 级续行）。
- `GraphPlanningContext` 新增字段 `session_id: Option<&str>`；`from_run` 用 `run.session_id`。
- 续行预算：`MAX_GRAPH_AUTO_CONTINUE_STEPS` 语义改为「**同一用户消息系列内的连续自动续行次数**」，
  新增常量 `MAX_CONSECUTIVE_AUTO_CONTINUES: usize = 8`；用户手动轮次不消耗该额度。
- 问号启发式收紧：仅当助手消息**末尾**（trim 后最后一个字符）为 `?`/`？`，或命中既有显式短语表时判 `WaitUser`；
  正文中间出现问号不再拦截（回归用例必须覆盖“代码示例含 ? 但未提问”）。

## C2 持久化（goal-core）
- 存储介质：JSON sidecar（不新增 sqlite 表，避免迁移风险）。
  默认路径 = `default_sqlite_path().parent()/goal_store.json`；测试/宿主可用 env `PONY_AGENT_GOAL_STORE_PATH` 覆盖。
- 写入时机：`create_goal` / `update_goal` 成功后原子写（`*.tmp` + rename）；读取为首次访问时懒加载，进程内仍缓存。
- 对外测试接缝：`goal::reset_goal_store_for_test()`（清空内存+重新加载）与 `goal::goal_store_path()`。
- 重启语义：同一路径下重启后 `get_goal` 必须仍返回目标（phase/rounds/revision 完整）。

## C3 turn 结束续行驱动（autocontinue）
- 后端（桌面 graph 路径）：`HostControlPlane::advance_graph_run` 外包一层有界循环：
  每轮结束后依据 decision 判定；当 `decision.kind == Continue` 且存在 armed 目标且连续续行 < `MAX_CONSECUTIVE_AUTO_CONTINUES`
  时，构造 goal-round `TurnInput`（`user_message = goal::build_goal_round_message(objective, round)`，`display_message` 标注 `[goal round N]`）
  并继续下一轮；达到上限或 decision != Continue 立即停止，返回最后一次响应。
- 每成功发起一轮：`GoalData.rounds_started += 1` 并持久化；发 `goal://round` 宿主事件（payload: goalId, round, maxRounds, status）。
- goal-round 文案（唯一实现于 `goal::build_goal_round_message`）：
  `<goal_round round="N" max="M">继续推进目标：{objective}。以工作区与工具结果为准；仅当目标真正达成才 update_goal complete。若本轮仍有未完成步骤，直接继续执行，不要询问用户。</goal_round>`
- TUI：同一文案 + 同一预算，在 turn 流结束后由 TUI 循环发起下一条 goal-round 消息（不得改变用户手动输入路径）。

## C4 每轮回灌（prompt/context）
- `context.rs::volatile_context_note` 追加（在 `Run goal` 之前）：
  - 若 `active_goal_for_session(conversation_id)` 命中：`Active goal: {objective} (phase={phase}, round={rounds_started}/{max_goal_rounds}).`
  - 若 `todo::todo_list(session)` 非空：`Plan: {completed}/{total} completed; in_progress: {content, …}.`
- 注入位置不变（developer 动态层，位于稳定层之后），不得破坏 `prompt_layers_are_ordered`（ADR 0007）。
- session 键统一为 `session_context.conversation_id`（= session id）；工具缺省 session 时由 runtime 注入当前 session id。

## C5 工具与提示词（prompt）
- `BASE_SYSTEM_PROMPT` 新增 `Goal & planning policy` 段（放在工程契约之后、输出规范之前），必须包含：
  - 何时主动 `create_goal`（长时/多步/跨轮目标，即使用户未说“目标”）；
  - `update_goal` 仅在真正达成时 complete、pause/resume 需人类请求、blocked 需同一阻断连续 3 轮；
  - 多步任务开始前用 `TodoWrite` 建计划并保持状态；
  - 允许一行计划状态更新（把“No restating: never restate the plan”改为“不复述已展示的工具输出；计划状态变更可一行播报”）。
- 全文工具名统一为模型可见名 `TodoWrite`（禁止残留 `todo_write` 文案）；`system_prompt_contract` 测试同步改为断言 `TodoWrite`。
- `ToolExecutionContext` 新增 `session_id: Option<String>`；`TodoWrite/TodoList/create_goal/get_goal/update_goal` 在参数未显式给 session 时使用它。

## C6 前端可见性（frontend）
- `planStore` 与 `goalStore` 以真实 sessionId 为键；turn 结束（`turn:completed` / graph 流结束）后自动 `refresh(sessionId)`。
- `HomeSidebar` 的 PlanPanel 不再依赖“已有数据”才渲染：面板常驻（空态显示“暂无计划”），挂载即拉取。
- `GoalBar` 在 turn 结束/goal 事件后刷新；后端 `goal://changed` 事件至少覆盖 create/update/round。
- 桌面端 Plan 可见性：`HostControlPlane` 与 runtime 的 `governed_executor` 共享同一 `PlanStore` 实例（生产构造必须注入；测试注入路径保持）。

## 验收（Test Agent 冻结）
1. 提示词契约：含 goal policy、`TodoWrite`、允许计划状态播报；无 `todo_write` 残留。
2. planner：armed 目标驱动 Continue（与关键词无关）；仅末尾问号 WaitUser；连续预算按系列计。
3. 持久化：跨 reset 仍可读回；损坏文件不 panic（降级为空）。
4. 注入：含 Active goal / Plan 摘要；层序合法。
5. 续行：有界循环（≤8）、complete 后停止、round 计数递增并持久化；TUI 路径同文案（可用纯函数断言）。
6. 前端：空计划时面板仍渲染；turn 结束后 store 触发 refresh（vitest，mock invoke）。
