# Agent Note: TUI Ctrl+T 抽屉与底栏左槽的面向用户重构（统一折行 + 会话隔离 + 子代理可见性）

Status: implemented

## Problem

`Ctrl+T` 底部任务抽屉与输入框下方状态条在 2026-11-22 那批修复（[ADR
`2026-11-22-tui-task-drawer-visibility`](../bug-fix/2026-11-22-tui-task-drawer-visibility.md)）之后
**数据已经接通**，但「能不能读」和「读得准不准」仍有两层系统性缺陷（逐项有源码证据）：

1. **长文本被硬截断，抽屉读不全**：`crates/pony-agent-core/src/bin/tui/task_center.rs` 的
   `content_lines()` 返回单色 `Vec<(String, Style)>`，`render()` 直接整行渲染、从不调用
   `wrap_line`（`bin/tui.rs:4339`）；`height()` / `scroll_by()` 也只按**未折行**的行数计数。
   于是长目标 / 长计划步骤 / 长任务标题一律在右边界被截断，且布局预留高度与真实渲染行数不一致
   （底部呼吸行被吃掉）。
2. **抽屉贴着输入框，视觉粘连**：`bin/tui.rs` 的 `drawer_rect` 下缘直接等于 `bottom_top`（或
   `blocker_h` 上沿），抽屉最后一行与输入框顶部 `SURFACE` 色带零间隔，读起来像输入框的标题。
3. **Goal 页签信息层级倒置**：抽屉直接复用 `Ctrl+G` 覆盖层的 `goal_panel::render_lines()`，把
   「阶段」「轮次」当成与目标正文平级的中心行，并把整条 `GOAL_PANEL_HINT`
   （`/goal pause · resume · edit <目标> · complete · blocked <理由> · clear`）以正文色 `INK` 输出。
4. **Agents 页签不准确且不可操作**：
   - **Subagent 完全不可见**：`crates/pony-agent-core/src/agent/orchestration.rs` 的
     `subagent_in_workspace_with_context` 预分配了 `subagent-<uuid>` 会话 id，却没有登记到任何花名册或
     运行注册表——抽屉与状态条都看不到子代理。
   - **伪成员 `lead`**：`list_agents()` 在 `TEAMMATES` 初始化后永久插入 `status: Running` 的伪成员
     `lead`，抽屉照单渲染成「一个永远在跑的队友」。
   - **跨会话串话**：`TEAMMATES` / `TEAM_TASKS` 是进程全局静态表，无宿主会话归属过滤——同一进程
     多个 TUI 会话会互相看到对方的队友与任务。
   - **状态用英文词**：行尾直接打印 `running` / `inactive` / `failed`，与全界面「语义图标」语法不一致。
   - **不可进入**：抽屉只读，Agents 页签没有行焦点与 `Enter`，用户必须回到 `/agents` 覆盖层才能进入
     队友会话（子代理则完全无法进入）。
5. **Tasks 页签数据割裂**：只读 `state.todos_state`（会话 todo），团队共享任务
   `state.task_board_state` 未出现在抽屉里——两个数据源各有价值，用户看不到团队侧进展。
6. **Jobs 页签裸露实现细节**：`crates/pony-agent-core/src/bin/tui/jobs_panel.rs` 直接渲染
   `● running · pid <n> · owned|promoted · <command>`。`with_description`
   （`agent/tools/schema.rs`）早已强制模型为 `Run` / `job_start` 提供中文 `description`，但
   `JobStartArgs` / `JobSession` / `JobListItem` 与 `register_promoted` 全链路都不接收该字段——中文描述
   在数据层就丢了，UI 只能摊原始 shell 命令；抽屉也没有任何终止后台任务的操作入口。
7. **底栏左槽不准确且措辞冗长**：
   - `tui_plan_from_value`（`projection.rs`）在全终态时回退最后一份终态计划 ⇒ 已结束/已中止的计划
     常驻底栏，谎称还在推进；
   - `agents_segment` 只计 `roster.entries` 的 `driving`，漏掉 ACP 委派与 subagent；
   - `team_segment` 读全局 `team_task_list()`，无会话过滤；
   - `goal_state` 未纳入 500ms `sync_task_glance` 刷新（只在命令/事件/切会话时写），切轮后横幅可能
     停在旧值；
   - 措辞是 `goal 轮3/5 · plan 2/5 · team 4(⛔1) · agents 2run · jobs 1run` 这类英文混排长串。

## Decision

按用户确认的 `1A / 2A / 3A` 三条决策实施，落成六组硬规则。

**1A（抽屉内交互，空输入优先）**：`Ctrl+T` 抽屉保持非模态。当**输入框为空**且当前页签有可选列表
（`Agents` / `Jobs`）时：`↑↓` 移动行焦点、`Enter`（Agents）进入选中的 Teammate / Subagent 会话、
`x` / `Delete`（Jobs）终止选中的运行中作业；`Ctrl+K` 在 Jobs 页签下**不受输入框内容限制**。输入框有
字符或页签不可选时，`↑↓` 仍滚动抽屉、`Enter` / `x` 照常落进 Composer（不抢占正常打字）。

**2A（合并旧独立弹窗入口）**：`/tasks`、`/agents`（含 `/teams`）直接打开 `Ctrl+T` 抽屉并切到
`Tasks` / `Agents` 页签（单一真值源）；`InteractionMode::TaskBoard` 与 `InteractionMode::AgentRoster`
的代码与事件分支**保留**作为向后兼容兜底（既有针对这两个枚举态的独立单测不受影响）。

**3A（底栏左槽极简风格）**：采用「语义图标 + 极简中文短词」，只报活跃 / 需关注态：
`◎ 目标 2/8 · ▤ 计划 2/5 · ☑ 任务 4 · ⚡ 智能体 2 · ⚙ 后台 1`（有阻塞/失败附 `⛔N` / `✘N`）。
终态 Plan（`PlanState::is_terminal`）与完成态 Goal 从常驻左槽隐藏。

具体落点：

- **单一折行口径**（`task_center.rs`）：新增 `build_lines(state, width)`，把「逻辑行（多 `Span`）→
  按内容宽度折行」收成一个函数；`height_for_width()` 与 `render()` 都从它派生 ⇒ 布局预留行数 ==
  真实渲染行数。`height()` 保留为默认宽度的兼容包装供既有单测调用。
- **抽屉与输入框间距**：`bin/tui.rs` 新增 `DRAWER_INPUT_GAP = 1`，`drawer_bottom` 在抽屉打开时
  再扣 1 行 CANVAS 空白（并不越过 `chat_top`）。
- **5 个页签面向用户分层**：Goal 以目标正文为主体（自动换行）、阶段/轮次收拢成一行弱化状态、命令提示
  用最弱色；Plan 首行摘要 + 进度，步骤用 `✔ ● ○ ✘`；Tasks 合并会话 todo 与团队任务（空分区隐身）；
  Agents 三类分区（队友 / 子智能体 / ACP 委派）纯图标状态 + 行焦点 + `Enter`；Jobs 优先
  `description`、无 `pid`/`owned`/`promoted`/英文状态词、运行中显示时长。
- **Core 数据层补齐**（这是 UI 准确性的前提，不是 UI 局部修补）：
  - `jobs.rs`：`JobStartArgs` / `JobListItem` / `JobSession` 增加 `description`；保留
    `register_promoted` 原 11 参签名（契约接缝测试冻结），新增 `PromotedJobSpec` +
    `register_promoted_with_description` 供 `tools/run.rs` 的 promote 路径透传 Run 的 `description`。
  - `orchestration.rs`：新增 `SubagentRunStatus` / `SubagentRunRecord` 与进程内 `SUBAGENT_RUNS`
    （上限 64 条，运行中不淘汰），在 `subagent_in_workspace_with_context` 入口登记 `Running`、前后台
    收尾更新终态；`list_subagent_runs_for_session(parent)` 按宿主会话过滤（无 `parent_session_id`
    的内部直调不串到任何 TUI 会话）。新增 `rebind_subagent_run` 处理引擎回传真实 id 的迁移。
  - `orchestration.rs`：新增 `TEAMMATE_HOST_SESSIONS` / `TEAM_TASK_HOST_SESSIONS` 与
    `list_agents_for_session` / `team_task_list_for_session`；`host_session_for_caller` 把队友
    `team-*` 会话反查回宿主会话。原 `list_agents()` / `team_task_list()` 行为不变（库层测试不受影响）。
  - `acp/registry.rs`：`ActiveDelegate.description` + `register_with_description`（原 `register`
    签名与 `DelegateRegistration` 字段集不变），`execute_acp_delegate` 透传 `description`。
- **TUI 投影层**（`projection.rs` / `protocol.rs` / `status_bar.rs`）：新增
  `effective_host_session_id(state)`（`return_session_id` 优先）作为**团队视图**的会话归属；
  `PlanState::is_terminal`；`SubagentViewRow`；`sync_goal_state_lossless`（只认 `Ok`，瞬时错误不清
  横幅）接入 500ms 节流块。`jobs` **不**走宿主归属，严格按 `state.session_id`（契约 B012）。
- **子会话候选扩容**：`sub_session_candidates_from_runtime` 在 teammate 之外追加可加载的
  `subagent-*` 会话 ⇒ `Enter` / `Ctrl+←/→` / `Ctrl+↑` 链路对子代理同样成立。

## Alternatives considered

- **只在 UI 层做截断省略号**（不改 Core）：长目标仍读不全，`height()` 与真实行数依旧双口径，
  且 Jobs 的中文描述在数据层已丢、无法在 UI 补回。否决。
- **把折行下推给 ratatui `Paragraph::wrap`**：`height()` 需要提前知道折行后行数才能预留布局，
  而 `Paragraph` 的 wrap 只在渲染期发生，无法反查 ⇒ 仍会出现「预留 N 行、画 M 行」的错位。
  自建 `build_lines` 是唯一能同时服务布局与渲染的口径。否决。
- **给 `register_promoted` 直接加第 12 个参数**：`crates/pony-agent-core/tests/promote_script_cleanup_test.rs`
  直接按 11 个位置参数调用该函数，加参会破坏契约接缝测试。改为保留原签名 + 新增结构体入口。否决。
- **让 `list_agents()` / `team_task_list()` 直接按会话过滤**：这两个函数是库层公共 API，被非 TUI
  调用方（桌面宿主、测试）依赖「看到全量」语义；直接改会静默改变既有契约。改为**新增**
  `*_for_session` 变体，TUI 侧切换调用点。否决。
- **抽屉内实现 ACP 委派「进入会话」**：ACP 是外部子进程同步调用，历史不在宿主 `SessionStore`
  且不追踪正文（`acp/registry.rs` / `acp/session.rs` 已证实），进入只会得到空会话。按用户确认的
  边界**不实现**，UI 上显式标注 `[外部进程 · 不可切入]`。
- **底栏保留英文混排**：与全界面「语义图标优先」语法冲突，且 `goal 轮3/5` 这类混排在窄屏让位时
  更难裁剪。改用 3A 的图标 + 中文短词。否决。

## Consequences

- 抽屉内所有长文本（目标、计划步骤、任务标题、子代理描述）按内容宽度自动折行，布局预留与真实渲染
  行数同源（`height_for_width` / `render` / `scroll_by` 共用 `build_lines`）。
- 抽屉与输入框之间恒定 1 行 CANVAS 空白，两层视觉分离。
- Agents 页签三类条目准确、纯图标状态、可 `Enter` 进入 Teammate / Subagent；ACP 委派明确标注不可切入。
- Tasks 页签同时承载会话 todo 与团队任务；Jobs 页签优先中文描述、无内部字段、可 `x` / `Ctrl+K` 终止。
- 底栏左槽缩短为「图标 + 中文短词」，终态 Plan 与完成态 Goal 不再常驻；Subagent 与 ACP 运行态计入
  `⚡ 智能体`。
- 进程全局表新增两张归属映射（`TEAMMATE_HOST_SESSIONS` / `TEAM_TASK_HOST_SESSIONS`）与一张子代理运行
  注册表（`SUBAGENT_RUNS`，上限 64）。它们只承载可见性，不参与任何权限/生命周期裁决。
- 回归面：`tui_task_visibility.rs` / `task_center_panel_tests.rs` / `task_center_data_sources_tests.rs`
  / `task_center_acp_and_jobs_tests.rs` 的措辞与结构断言同步更新；`w3b_sub_session_tests.rs` 的徽标
  文案锚点保持不变（本次未改 `sub_session_badge_text`）。
- 兼容性：`register_promoted` 11 参签名、`DelegateRegistration` 字段集、`list_agents()` /
  `team_task_list()` 行为、`InteractionMode::TaskBoard` / `AgentRoster` 分支、`tui_ctrl_o_global_shortcut.rs`
  的源码文本锚点均保持不变。
- **门禁收口（clippy 棘轮，零重基线）**：本卡新增代码产生的 15 条违例全部改代码消除，未上调
  `scripts/clippy-baseline.json`：`orchestration.rs` 的 7 处 `Mutex::lock().unwrap()` 改为既有惯例
  `unwrap_or_else(PoisonError::into_inner)`（毒锁恢复，与本仓 `acp/*` 同形）；`task_center.rs` /
  `tui.rs` / `status_bar.rs` 的 `map(..).unwrap_or*(..)` 改 `map_or` / `map_or_else`；`isize as usize`
  改 `saturating_add(delta.unsigned_abs())`；`jobs_panel.rs` / `task_center.rs` 的 `pub` 收窄为
  `pub(crate)`（这些项只在 `bin/tui.rs` 与其内联测试内可见）；`#[allow(dead_code)]` 改
  `#[expect(dead_code, reason = ..)]`；`PromotedJobSpec` 因 `ProcessManager` 未实现 `Debug` 而带
  `#[expect(missing_debug_implementations, reason = ..)]`；`register_promoted_spec` 去掉恒 `Ok` 的
  `Result` 外壳（两个公开入口仍返回 `Result`，兼容既有调用点）；行选中态由 `bool` 入参改为
  `RowSelection` 枚举（本仓 `fn_params_excessive_bools` 阈值 0）。实测 3833 / 基线 3870，新增 0。
- **测试隔离修补（跨进程）**：`tests/tui_overlay_cancel_acceptance.rs` 的 PTY 驱动原本只隔离
  `HOME`/`XDG_*`；在 `test-support` 兜底沙箱按 pid 分目录后，`resume` 流的 seed 与 `pony resume`
  是两个进程 ⇒ 各写各的沙箱、恢复不到历史（J1 红）。修法是在驱动 env 里显式给同一个
  `PONY_AGENT_DATA_DIR`（该变量本就是 ADR `2026-11-25-test-support-default-data-dir-isolation`
  认可的显式旁路），不动 core 的路径解析语义。
- **已确认为既有 flaky（非本卡引入）**：`interrupted_turn_resume_acceptance` 的
  `at2_concurrent_repeated_recovery_is_deterministic_and_never_duplicates` 在 **pristine HEAD**
  （`git worktree` 独立检出）上同样失败——它断言进程级 `agent_team_policy_enabled()` 在同进程
  并行用例期间不变，而同二进制的其他用例会装 `set_subagent_engine`；`--test-threads=1` 全绿。
  `c6_repeated_recovery_is_idempotent_and_does_not_duplicate_turns` 同因（单独跑绿）。两者归属
  测试并行隔离债，不在本卡范围。
- **本机门禁执行环境事实（复现用）**：本机存在两套 `rustc 1.91.1`——发行版 tarball 版
  （`/usr/lib/rust-1.91`，`rustc --version` 带 `(built from a source tarball)`）与 rustup 版
  （`~/.rustup/toolchains/1.91.1-*`，版本串不带后缀）。`cargo`/`rustc` 解析到发行版，而
  `cargo-clippy` / `clippy-driver` / `rustdoc` 解析到 rustup 版 ⇒ 同一 `target*` 目录里两套
  rmeta 混用，报 `error[E0514]: found crate ... compiled by an incompatible version of rustc`
  上千条。这不是代码问题：把三个变量都指向同一套工具链后全部门禁可跑通——
  `TC=~/.rustup/toolchains/1.91.1-x86_64-unknown-linux-gnu/bin; RUSTC=$TC/rustc RUSTDOC=$TC/rustdoc`。
  `npm run rust:lint` 只注入 `CLIPPY_CONF_DIR`、不接管 `RUSTC`，故 clippy 门在本机必须带 `RUSTC`；
  否则门以 `exit 2` fail-closed 拒绝判定（不会假绿，但会挡流程）。
