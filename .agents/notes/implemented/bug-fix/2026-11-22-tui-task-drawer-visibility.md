# Agent Note: Ctrl+T 任务抽屉「六个页签没内容 / 措辞错位」——表现层、数据接线与归属的整批修复

Status: implemented

## Problem

用户在 TUI 里通过 Ctrl+T 打开底部任务抽屉（`crates/pony-agent-core/src/bin/tui/task_center.rs`，
`Goal / Plan / Tasks / Agents / Jobs` 五页签）后，观察到六个独立症状。逐项定位（全部有代码证据）：

1. **ACP 委派子智能体完全不可见**：`acp_delegate` 是同步工具调用（`agent/tools/orchestration.rs`
   的 `execute_acp_delegate`），子进程与 ACP 会话只活在该次调用的栈上；TUI 全文件无任何 ACP 状态投影
   （`bin/tui.rs` 对 `acp` 零引用），Agents 页签只读 teammate 名册（`list_agents()`）。
2. **Plan 页签从没显示过**：`state.plan_state` 只在 `/plan`、`/approve` 两条命令里被写入；
   `UiEvent::PlanUpdated` **没有任何生产者**（定义于 `bin/tui.rs`，仅 event loop 消费），
   而 core 的 `plan/update`（`agent/turn_event.rs`）也没被 `TuiTurnSink` 消费。
   同时 `tui_plan_from_value` 固定取 `plan_list().first()`——会话产生第二份计划后永远显示最早那份。
3. **Tasks 页签从没显示过**：抽屉读 `task_board_state`（`team_task_list()` 的**进程全局团队看板**），
   单会话 TUI 下几乎恒空；会话 todo 的投影 `tui_tasks_from_todos` 早已 `#[allow(dead_code)]`。
4. **Goal 页签实际是「打开时快照过期」**：`goal_state` 只在 `/goal` 命令、轮末 `StreamCompleted`、
   `GoalUpdated` 事件与 resume hydrate 时写入；Ctrl+T 打开抽屉时**不同步**，于是打开瞬间看到的是
   上一次写入的状态（多为空态），用户侧读作「goal 从没显示过」。
5. **Jobs 页签显示全进程作业**：`jobs_panel::sync_jobs_from_registry` 调 `job_list(JobListArgs {})`
   （`caller = None` ⇒ 不过滤），把别的会话（含 teammate 子会话）的后台任务混进本会话视图，
   违反 B012 契约已有的归属语义（`job_list_scoped` 早就支持按 owner 过滤）。
6. **措辞与交互错位**：切页签只支持 `Tab/BackTab`（缺 `←/→`）；五个页签空态是 2~3 行解释性长句；
   状态栏摘要把后台任务写成「作业」并在计数旁带 `⚠`（`status_bar.rs::jobs_segment`）。

## Decision

1. **切页签**：新增裸 `←/→`（`prev_tab/next_tab`，环绕且滚动归零）；`Tab/BackTab` 保留为别名，
   但**带 Ctrl 的方向键不拦**（`Ctrl+←/→` 仍是子会话切换，W3b）——判据在 key 分支里用
   `bare_arrow` 一次表达。页签提示改为「←→ 切页签 · ↑↓ 滚动 · Esc 关闭」。
2. **空态一行**：五页签在无数据时统一「恰好一行、只说结论不给教程」；Goal 的一行空态在抽屉层实现，
   Ctrl+G 目标详情覆盖层的引导文案不受影响（同一状态在两种容器里的信息预算不同）。
3. **状态栏**：后台任务段改为「后台任务 {运行中}」/「后台任务 失败 {n}」/「后台任务 {n}·失败 {m}」，
   **不再用 `⚠`**（`⚠` 在本条状态栏与「队友」段同形，混读成整句告警）。任务抽屉内的 Jobs 表头
   同步改用「后台任务」。
4. **Plan 数据接线（拉取式）**：新增 `projection::sync_plan_from_store(state, control_plane)`，
   读取 `control_plane.plan_list(session)` 并投影；`tui_plan_from_value` 改为**优先最新未终态**
   （draft/executing），全终态时退回最新一份。同步点三处：打开抽屉（Ctrl+T / `/panel`）、
   轮末 `StreamCompleted`、500ms 常驻刷新（见第 6 条）。
5. **Tasks 数据接线（换语义）**：抽屉 Tasks 页签改读**本会话 todo**（新增
   `TuiState::todos_state`，`projection::sync_todos_from_store` 读 `todo_list(session_id)`）；
   `task_board_state` 与 `sync_tasks_from_store` 保留给 `/tasks` 覆盖层（团队共享看板），
   两者不互相冒充。空列表保留 `None`（一行空态，不出 0/0 进度）。
6. **常驻刷新**：`status_bar::sync_task_glance` 扩参 `&HostControlPlane`，在既有 500ms 节流块内
   追加 `projection::sync_drawer_live_sources`（todo + plan，均为进程内注册表）。goal 不进常驻刷新
   （它有自己的命令/事件通道，且读取失败会 fail-closed 清横幅），改在**打开抽屉时**
   `sync_drawer_sources` 里 hydrate。
7. **Jobs 归属**：`sync_jobs_from_registry` 改用 `job_list_scoped(args, Some(session_id))`——
   非 owner 作业连存在性都不暴露（B012 原口径）。
8. **ACP 可见性**：新增 `crates/pony-agent-core/src/acp/registry.rs`（进程内活跃注册表 +
   `ActiveDelegateGuard` RAII 注销），`execute_acp_delegate` 在 spawn 成功后登记
   （agent / ACP session id / 宿主会话 / cwd / 起始时刻），委派返回即注销；
   TUI 侧 `TuiState::acp_delegates` 按**当前宿主会话**过滤，Agents 页签以
   「ACP 委派 N · 运行中」分区展示 `● agent · 会话尾号 · cwd`。注册表不参与 ACP 生命周期、
   预算与权限裁决，也**不**把委派正文写进任何可见面。
9. 测试：`bin/tui/tests/task_center_panel_tests.rs`（5 例）、`task_center_data_sources_tests.rs`（4 例）、
   `task_center_acp_and_jobs_tests.rs`（3 例）+ `acp/registry.rs` 内联 2 例；账本 B265–B270。
10. **契约修订与冻结用例同批改写**（不是放宽断言，是换真值）：
    - `tests/tui_task_visibility.rs`：摘要第五段「作业」→「后台任务」（顺序/让位规则不变），
      Jobs 空态断言改为一行「当前没有后台任务」；
    - `tests/slash_commands_acceptance.rs::t1b`：`/panel` 分发点的同步函数由
      `sync_tasks_from_store` 换成 `projection::sync_drawer_sources`，「先同步再开抽屉」的
      顺序不变量逐字保留。

## Related

- 抽屉形态与「为什么是抽屉不是弹窗」：`implemented/architecture/2026-11-17-tui-task-visibility-bottom-drawer.md`
- `/panel` 斜杠入口与页签呼吸行（本次只改同步点，入口不变量不变）：
  `implemented/feature/2026-11-19-tui-task-drawer-panel-entry.md`
- 作业归属契约：B012（`agent/jobs.rs::ensure_owned` / `job_list_scoped`）
- ACP 委派实现：`agent/tools/orchestration.rs::execute_acp_delegate`、`acp/session.rs`

## Alternatives considered

- **`plan/update` 走 turn 事件到 TUI**：`TurnStreamEvent` 没有 `plan_state` 字段（`runtime/types.rs`），
  要接线就得改 wire 形状 + sink + 反序列化，成本明显高于「打开抽屉/轮末从 PlanStore 拉一次」，
  且拉取式与 jobs/tasks 的既有投影纪律同构（同构 > 新增事件通道），故不选。若将来需要逐帧刷新
  计划步骤，再补事件面。
- **Tasks 页签同时展示团队看板与会话 todo（分区）**：单会话 TUI 下团队看板几乎恒空，双分区会让
  「一片空 + 一小段实」读起来像加载失败；且 `/tasks` 已能直达团队看板，抽屉保持单一语义更可预期。
- **把 ACP 委派伪装成 teammate 名册条目**：`list_agents()` 的语义是「可寻址的常驻成员」
  （可 `send_message`、可进入其会话），ACP 委派是一次性同步子会话，混入会造成「点了进不去」的
  假能力；故用独立注册表 + 独立分区，不给名册造假成员。
- **ACP 注册表做成全局可见（不自会话过滤）**：与 B012 作业归属同一条纪律——跨会话可见性既是噪声
  也是泄漏面；`None` 宿主的委派无归属，宁可不上屏。
- **Jobs 仅在 TUI 侧过滤（拉全量再 filter）**：`job_list_scoped` 已是注册表层的单一判据，
  TUI 侧再写一份比对等于把归属规则复制成两处（必然漂移），故直接换 API。
- **保留「作业」措辞不改**：与用户心智（background jobs = 后台任务）及 `job_start` 文档不一致，
  且同一状态栏里「作业」在本仓其他语境指 POSIX 作业控制（`Ctrl+Z`），双语义易误读。
