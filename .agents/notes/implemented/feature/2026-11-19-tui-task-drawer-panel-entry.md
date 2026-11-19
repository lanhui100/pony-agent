# Agent Note: TUI 任务抽屉补斜杠入口 `/panel` 与页签-内容呼吸行

Status: implemented

## Problem

1. **面板存在，但用户找不到。** 子智能体 / 后台作业 / Agent Team / Team 任务 / Goal 已在
   ADR `2026-11-17-tui-task-visibility-bottom-drawer` 收敛为底部任务抽屉（五页签），但唯一入口是
   快捷键 `Ctrl+T`：斜杠命令表、斜杠候选菜单里都没有它。用户的实际反馈是「整合重构是没构建，
   还是没修好？为什么当前 TUI 没有该面板」——事实是**既构建了也在屏，只是没有可发现的入口**。
   `/tasks` 不能兼职：它是「完整 Team 共享看板」，与五页签抽屉不是同一件事。
2. **页签栏与内容首行紧贴。** 抽屉是密集行结构（发丝线 + 页签 + 内容），页签行直接压住内容首行时，
   视觉上会被读成「内容的小标题」，扫读时要多花一次停顿去分辨层次。

## Decision

1. **新增 `/panel` 斜杠命令**（`AVAILABLE_COMMANDS` 中紧随 `/approve`），与 `Ctrl+T` **完全同路径**：
   `sync_tasks_from_store` → `toggle_task_center`（顺序固定：先同步真实 todo 存储再开抽屉，否则抽屉
   展示上一轮快照，用户看到的仍是「空的」）。纳入 `is_panel_command`（流式期可打开、不排队、不拒绝）。
   `Ctrl+H` 帮助条目 keys 改为 `Ctrl+T / /panel`，让快捷键与斜杠两条入口互相指认。
2. **契约 C-4 补齐为 20 条真值**：`/panel` 入表的同时，把历史欠账 `/permission`、`/goal`、`/title`
   （早已在 `AVAILABLE_COMMANDS` 与冻结测试里，但契约矩阵与 C-4 表未同步）一次补齐；契约矩阵
   `.dev-team/contract-matrix-slash-commands.md` 记修订16，冻结测试 `CONTRACT_COMMANDS` 同步 20 条。
   **断言未放宽**：顺序固定、逐条相等。
3. **页签与内容之间加 1 行呼吸行**：`TAB_CONTENT_GAP = 1`；抽屉高度 `< GAP_MIN_HEIGHT(5)` 时呼吸行
   让位给内容（小屏优先保信息）。固定开销由 `chrome_rows(total_height)` 单点给出，`height()`（布局）与
   `render()`（绘制）共用同一口径，滚动可视行数同步扣除，避免「高度算少 / 内容被截」两套真值。
4. **回归钉子**：
   - `tests/tui_task_visibility.rs::drawer_puts_a_breathing_row_between_tabs_and_content`：页签行下一行必须
     全空白、再下一行必须是内容首行；
   - `tests/slash_commands_acceptance.rs::t1b_panel_command_opens_the_task_drawer_like_ctrl_t`：`/panel`
     属面板类命令（含大小写/空白容忍与近似拼写反例），且分发块内 `sync_tasks_from_store` 先于
     `toggle_task_center`。

## Alternatives considered

- **A. 维持只走 `Ctrl+T`，靠 `/help` 宣传**：快捷键是「记不住的入口」，斜杠命令才是可发现面；用户已经
  因此认为功能缺失；否决。
- **B. 让 `/tasks` 兼职打开抽屉**：`/tasks` 的语义是完整 Team 共享看板（`team_task_list`），抽屉是五类
  关注点的只读汇总；一条命令两种结果会让「我按了 /tasks 为什么看不到依赖树」重新变成困惑；否决。
- **C. 加裸 `t` 单键快捷键**：会吞掉以 `t` 开头的正文输入（与既有「不占用裸字母」纪律冲突）；否决。
- **D. 呼吸行改两行或改用空发丝线**：抽屉内容区上限是正文 2/3，两行挤压真实信息；再画一条发丝线会与
  顶部真发丝线抢语义（哪个是边界）；否决。
- **E. 采用：`/panel` 斜杠入口 + 单行呼吸行（本 ADR）。**

## Consequences

- 正面：面板可发现性从「记住快捷键」变成斜杠菜单里一条可见命令；层次上页签与内容分离，扫读更快。
- 负面/已知限制：命令表从 19 条增至 20 条（契约面多一条需长期同步的条目）；抽屉高度 < 5 行时无呼吸行
  （小屏取舍）。
- 契约面：C-4 冻结表与契约矩阵同时改写（修订16），`is_panel_command` 增加 `/panel`；未改动任何写路径，
  抽屉仍是只读展示。
- 观测性：结构性 N-A（纯 TUI 呈现层与命令路由：无新增对外接口 / 网络 IO / 后台任务 / LLM 调用，
  不新增 metrics 归属与收口点）。

## Related

- 抽屉本体与「为什么是抽屉不是弹窗」：`implemented/architecture/2026-11-17-tui-task-visibility-bottom-drawer.md`
- 契约真值源：`.dev-team/contract-matrix-slash-commands.md` §C-4（修订16）
- 冻结测试：`crates/pony-agent-core/tests/slash_commands_acceptance.rs`、
  `crates/pony-agent-core/tests/tui_task_visibility.rs`
