# Agent Note: TUI 任务可见性按「唯一家 + 底部交互」重构（状态条摘要 + 底部任务抽屉）

Status: implemented

## Problem

TUI 里与「任务」有关的信息原本分散在六个互不相干的入口：`/plan`（PlanMode）、`Ctrl+T`/`/tasks`
（Team 任务看板）、`/agents`（花名册）、`Ctrl+G`（目标详情）、`Ctrl+H`（帮助），以及**完全没有入口**的
后台作业（`crates/pony-agent-core/src/agent/jobs.rs` 的注册表在 `src/bin/` 下引用数为 0）。

由此产生四个可复现的问题：

1. **记忆成本线性增长**：每类信息占一个平级快捷键，用户要知道「哪个键看哪个」；键位已挤到
   `Ctrl+G/H/T/R/M/P/O` 全占，再加关注点只能继续抢键。
2. **没有「一次看清」的入口**：用户 90% 时间只想知道「有没有东西在等我」，但必须逐个打开面板才能确认。
3. **后台作业不可见**：`job_start` 起的进程只在聊天流里留一行回执；还在跑、跑完了、失败了，界面上都
   没有家，状态条也无法回答「现在有什么在跑」。
4. **同类信息重复且互相挤占**：`header` 右槽与输入框下方状态行都在承载数值/身份类信息；Goal 与指标
   共用一行做 6s/8s 时分轮播（`goal_banner_phase`），模型身份与权限模式再做 3s/3s 轮播
   （`status_carousel_phase`）。轮播把「信息不足」伪装成「信息齐全」——用户看不到目标时会以为没有目标。

## Decision

关注点分离落成两条硬规则：**每类信息全屏恰好一处**、**需要人交互的东西都在底部**。

1. **状态条（常驻 1 行）三段固定布局**，不再有任何时分轮播：
   - 左槽：任务摘要 `goal 轮3/5 · plan 2/5 · team 4(⛔1) · agents 2run · jobs 1run`，无数据即留空；
   - 中槽：一次性提示 / 排队反馈（原有 TTL 语义不变）；
   - 右槽：`<model> · <档位>` + `权限 · <级别>` 同现，宽度不足时按「提示 → 权限 → 模型」顺序让位。
   实现收敛到新模块 `crates/pony-agent-core/src/bin/tui/status_bar.rs`（纯函数，无 `Frame`，可单测）。
2. **删除两个时分轮播**：`goal_banner_phase` / `GOAL_BANNER_HOLD_MS` / `METRICS_HOLD_MS` 与主循环里
   的相位重绘调度；`status_carousel_phase` 与 `STATUS_CAROUSEL_*` 及其重绘调度。Goal 的可见性由状态条
   摘要承担，模型与权限同时可见。
3. **底部任务抽屉**：新增非模态覆盖层（**不是** `InteractionMode`，打开时 `mode` 保持 `Normal`），
   从输入框基座上沿向上生长，内容按页签分区 `Goal / Plan / Tasks / Agents / Jobs`。
   - 高度按内容自适应，上限为正文区 2/3；正文区高度随之缩短，故最新消息天然可见（**不做**滚动锚点补偿）。
   - 输入框保持可输入；抽屉打开时 `↑↓/PgUp/PgDn` 归属抽屉，正文滚动改由鼠标滚轮与 `Ctrl+↑↓`。
4. **后台作业接入投影层**：`crates/pony-agent-core/src/bin/tui/jobs_panel.rs` 提供
   `sync_jobs_from_registry`（读 `jobs::job_list()` → `TuiJobsState`），与 `sync_tasks_from_store`
   同构；状态条摘要与抽屉的 Jobs 页签共用这份投影。
5. **打断层统一在底部**：Ask 与排队面板已是「覆盖层贴 Composer 上缘」的内联形态（`inline_ask_height`
   / `inline_queue_height`），权限审批从居中弹窗改为同一内联形态，保持「阻塞性请求紧贴输入框」的规则。

## Alternatives considered

- **继续「一个关注点一个面板 + 一个新快捷键」**：边际成本是「又多一个要记的键」，而收益只是多一条信息；
  键位空间已耗尽（`Ctrl+字母` 大面积被占），新增只能往 `Alt+` 或功能键溢出，记忆负担更差。
- **常驻大 dashboard（所有分区都展开）**：永久吃掉正文阅读空间，而多数区块常年为空；与「阅读正文」
  这一主诉求直接冲突。
- **停靠右侧栏（固定列宽）**：TUI 的内容列本就限制在 120 列并居中，再切一列会把正文压到不可读宽度；
  且窄终端（80 列）无位置可停。
- **顶部下拉面板（占屏 1/2~2/3，保留底部最新消息）**：只改面板 `Rect` 看似最便宜，但正文渲染是
  `Paragraph::scroll(offset)` 的窗口切片而非布局贴底，面板开合会让阅读位置跳动；要「保留最新消息」
  必须额外做滚动锚点补偿（记录并恢复 `scroll_offset`/`auto_scroll`）、遮蔽边界提示、滚动归属裁决三件事。
  底部抽屉复用已有的内联几何（`inline_ask_height` 同构），正文区自动缩短即天然贴底，成本更低且不破坏
  「交互都在底部」这一条规则。
- **沿用 ADR 0119 的模型/权限 3s 轮播**（该条当时把「状态行同时并排模型与权限」列为落选方案 C，
  理由是「窄终端容易挤爆」）：落选理由本身成立，但结论过强 —— 并排可以配**显式让位梯子**
  （宽度不足先丢权限、再丢中槽、最后压缩模型），行为由 `tests/tui_task_visibility.rs` 的
  宽度用例在 30 列（摘要整段让位、模型仍在）与 100 列（模型 + 权限同帧）上锁住。反过来，轮播的代价是**确定性信息缺失**：目标与身份各有 3~6 秒
  不可见，用户会把「没看到」读成「没有」。本 ADR 取代该条的状态行轮播部分
  （`.agents/notes/archived/feature/2026-11-10-tui-permission-mode-governance-and-carousel.md`），
  权限治理本体（`Ctrl+P`/`/permission`/偏好持久化）不变。
- **把抽屉做成 `InteractionMode`（模态）**：实现最省（复用 `begin_overlay`/`end_overlay`），但模态会在
  `ui()` 里触发 `Clear(chat_rect)` 整块清场，正是本次要消灭的「开面板就看不见正文」。
- **jobs 只做状态条计数、不做页签**：计数能回答「有没有在跑」，回答不了「跑的是哪条命令、跑了多久、
  谁起的」；排障场景仍要回到聊天流里翻历史行。

## Consequences

- 三类信息获得唯一家：数值指标在 `header` 右槽、任务摘要与模型/权限在状态条、任务细目在抽屉。
  重复渲染面减少，`tui.rs` 因迁出状态条与抽屉两段逻辑而缩短（文件级 `file-size-exempt` 不变）。
- 轮播删除后，`tests/tui_metrics_snapshot.rs`、`tests/tui_dialog_wave39.rs`、`tests/tui_render_layout.rs`
  里 `pin_carousel_phase` 夹具失去对象，改为「模型与权限同时可见」的相位无关断言；取代
  `.agents/notes/implemented/testing/2026-11-12-carousel-phase-must-be-pinned.md`（该条移入 `archived/`）。
- 后台作业首次在 TUI 可见（摘要 + 页签）；数据源是拉取式 `job_list()`，抽屉打开期间按既有
  `team-panel-live-polling` 的同构做法刷新，不新增事件通道。
- 抽屉是覆盖层而非布局项：`CHAT_BOTTOM_GAP` 与消息区下缘契约（C8）保持成立，既有 Ask/Queue 用例的
  几何断言不受影响。
- 已知取舍：抽屉打开时正文可视行数减少（内容不丢，只是屏内行数变少），这是「不遮最新消息」的实现代价。
