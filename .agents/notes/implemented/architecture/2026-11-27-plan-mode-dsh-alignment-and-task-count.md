# Agent Note: Plan 语义对齐 DeepSeek Harness 协作模式，任务计数统一为 x/x

Status: implemented

## Problem

当前系统在「Plan（计划）」与「Task / Todo（任务）」之间存在多处语义交叉与计数失真，同时困扰用户和模型：

1. **底栏 `☑ 任务` 计数语义不清且来源混杂**：TUI 输入框下方状态条 `crates/pony-tui/src/tui/status_bar.rs`（`tasks_segment`）仅展示未完成数量 `☑ 任务 {open}`（`todos_open + team_open`），与相邻的 `▤ 计划 {done}/{total}` 及抽屉 `Tasks` 页签的 `{done}/{total}` 语法不一致；同时把单会话 `TodoWrite` 与团队看板 `team_task_*` 的未完成数直接相加，且在子会话下两者分别按当前子会话和宿主会话过滤，导致数字容易误读或重复累计。
2. **桌面端侧边栏混加不同量纲**：`src/components/PlanPanel.vue` 折叠头角标直接渲染 `plans.length + todos.length`（把 Plan 份数与 Todo 条数相加），且把 Plan 步骤和 Todo 待办揉在同一「计划与任务」区块中。
3. **`Plan` 既充当第二套任务清单又承载草稿写锁，与 `Todo` 严重串名**：
   - `crates/pony-tools/src/plan_state.rs` 与 `plan_control`（模型可见名 `Plan`）被设计成带 `steps[]`、`complete_step`、`revision` CAS 与 `Draft/Executing/Completed/Aborted` 状态机的步骤清单；`governed_executor.rs` 通过 `plan_draft_gate` 在 `Draft` 态阻止写/执行工具。
   - 但 `crates/pony-tools/src/system_prompt_template.rs` 与 `prompt_contract.rs` 把 `TodoWrite` 称为 `Implementation planning`，而 `crates/pony-agent-core/src/agent/context.rs` 每轮回灌给模型的动态上下文更是将 `todo_list` 直接标成 `Plan: {completed}/{total} completed; in_progress: ...`。
   - 对照 DeepSeek Harness（`packages/plan/plan-mode/src/index.ts` 与 `packages/todo/README.md`）：`dsh` 中的 **Plan Mode** 是一种纯粹的**会话级协作模式（`active: boolean`）**——用 `/plan` / `/plan off` 切换、激活时注入 `plan:policy` 提示词、完成后通过 `exit_plan_mode(plan: markdown)` 经交互通道请用户批准退出；所有步骤进度追踪均归口 `todo_write`，`Plan` 不再参与任何任务清单与 `x/x` 进度语义。

## Decision

「目标 / 计划模式 / 待办 / 团队任务」四套概念彻底正交化，并同步对齐 TUI、桌面端与 Agent 提示词面（按 `1A + 2A` 方案落地）：

### 1. 概念边界定义（四者正交）

| 概念 | 工具 / 入口 | 唯一职责 | 是否包含步骤清单 |
|---|---|---|---|
| **Plan（计划模式）** | `/plan`、`/plan off`、`ExitPlanMode` | 会话级协作模式（先调研出方案，审批通过再动手执行） | **否**（只有模式开关与 Markdown 方案审查闭环） |
| **Todo（个人待办）** | `TodoWrite` / `TodoList` | 单智能体会话内步骤拆解与进度打勾 | **是**（无写锁、无需审批） |
| **Team Task（团队任务）** | `team_task_create` / `update` / `list` | 多智能体团队看板分工（认领人、写域、依赖阻塞） | **是**（团队协同专用） |
| **Goal（持久目标）** | `create_goal` / `get_goal` / `update_goal`、`/goal` | 跨多轮自动续行的顶层验收目标 | **否**（只管目标阶段与续轮上限） |

### 2. Core：Plan 收敛为协作模式（对齐 DSH + 硬禁写门禁）

- **会话级 Plan 模式状态**：会话级 `plan_mode` 布尔状态（`active` + 轮内切换 `pending`，跨恢复/重启可重建；进程级注册表 + JSON sidecar 原子写）取代原有的 `PlanStore` 多步骤清单状态机（`steps[]`、`complete_step`、`plan_execution_approve` 等已一并下线）。
- **进入与退出机制**：
  - 用户通过 `/plan`、`/plan <指令>` 或前端开关进入 Plan 模式，通过 `/plan off` 直接退出；
  - 仅在 Plan 模式激活时，于动态上下文层（`crates/pony-agent-core/src/agent/context.rs::volatile_context_notes`）注入 `plan:policy` 策略段（保持稳定系统提示词 `BASE_PROMPT_TEMPLATE` 前缀缓存不失效）；
  - 模型完成调研与方案编写后，调用规范化的退出审查工具（如 `ExitPlanMode` / 沿用受控工具槽位）提交完整 Markdown 方案，通过宿主 Ask 交互通道呈现给用户审批：**批准**则退出 Plan 模式并解锁执行；**继续规划 / 打回**则保留在 Plan 模式并把反馈带回模型。
- **激活期硬禁写护栏（确认 1A）**：复用 `crates/pony-tools/src/governed_executor.rs` 权限闸门，Plan 模式激活期间硬拦截写文件与命令执行工具（只允许只读检索、提问与提交退出方案），待用户批准退出或 `/plan off` 后自动解锁。
- **提示词与上下文回灌去混淆**：
  - 将 `crates/pony-agent-core/src/agent/context.rs` 的 `Plan: {completed}/{total} completed...` 修正为 `Todos: {completed}/{total} completed...`（同步更新 `docs/architecture/plan-goal-contract.md`）；
  - 更新 `crates/pony-tools/src/tools/prompt_contract.rs`、`system_prompt_template.rs` 与 `crates/pony-protocol/src/tool_contract/schema.rs`，明确单智能体步骤跟踪只用 `TodoWrite`，团队协作只用 `team_task_*`，长程自动续行只用 `create_goal`，Plan 仅用于模式内方案审查。

### 3. TUI 底栏与任务抽屉改造

- **`☑ 任务` 改为合并 `x/x`（确认 1A）**：扩展 `TaskGlance` 同时记录 `todos_done` / `todos_total` 与 `team_done` / `team_total`，底栏 `tasks_segment` 渲染为 `☑ 任务 {todos_done + team_done}/{todos_total + team_total}`（有阻塞保留 `(⛔N)`），全部完成（`done == total`）时从常驻底栏隐藏；并统一子会话下 `todos` 与 `team_tasks` 的查询归属口径。
- **`▤ Plan` 剥离计数**：底栏 `plan_segment` 不再展示 `2/5` 进度，仅在 Plan 模式激活时显示模式标识（如 `▤ Plan模式`）；`Ctrl+T` 抽屉 `Plan` 页签改为展示当前 Plan 模式状态与最近方案正文，不再呈现步骤列表。
- **命令对齐**：`/plan` 支持进入（含带指令）/ `/plan off` 退出，取代原 `render_plan_panel_stub` 步骤展示弹窗。

### 4. 桌面端一并修复

- **修正任务面板与计数**：修改 `src/components/PlanPanel.vue`、`HomeSidebar.vue` 与 `src/stores/plan.ts`，移除 `plans.length + todos.length` 混加角标，将任务列表独立展示清晰的 `completed/total`（`x/x`）进度。
- **接入 Plan 模式开关与审查流**：修改 `src/components/HomeWorkspace.vue`、`src/lib/chat/slash-commands.ts`、`src/components/chat/WorkspaceComposer.vue` 及 `src-tauri/src/lib.rs`，使 `/plan` 与 UI 入口成为真正的 Plan 模式切换器，并在 Ask 交互区域支持 Markdown 方案审查与「批准执行 / 继续规划」流转。

## Alternatives considered

- **仅改底栏文案与 `x/x` 格式，保留 `PlanStore` 作为重型任务清单**：否决。只要 `Plan` 还保留 `steps[]` 和 `complete_step`，同时又存在 `TodoWrite`（且上下文回灌自称 `Plan:`），模型与用户就仍会持续在“到底用 Plan 还是用 Todo”上犯错。
- **Plan 模式采用 DSH 原版纯软引导（不加写工具拦截）**：否决。Pony 已有 `governed_executor` 写门禁基础设施，在 Plan 模式激活期硬禁写能防止模型在未批准前意外修改工作区，比纯文本约束更可靠。
- **先切界面与提示词、底层旧 `PlanStore` 暂留兼容**：否决。双套 Plan 状态机并存会拉长维护债并迫使前后端长期维护两套 IPC 与契约，一步替换清理更干净。
- **底栏任务计数采用“仅显示团队或拆成两段”**：否决。单人场景只用 `TodoWrite`、团队场景才用 `team_task_*` 语义理清后，合算 `done/total` 既紧凑又不会重复计数。

## Verification

1. **Core 语义与提示词**：`context.rs` 动态上下文不再将 `todo_list` 标注为 `Plan:`；`Plan` 工具/模式与 `TodoWrite`、`team_task_*`、`create_goal` 边界在系统提示词、工具 schema 与 `docs/architecture/plan-goal-contract.md` 中完全对齐，`prompt_tool_consistency` / `tool_surface_order_contract` / `tool_surface_singularity` 与新 `PlanMode` 门禁测试全绿。
2. **Plan 模式闭环**：TUI 与桌面端均可通过 `/plan` 进入 Plan 模式（写入/执行工具被门禁拦截）、提交 Markdown 方案走用户审查批准退出并自动恢复写权限，或通过 `/plan off` 直接退出；状态跨恢复持久化。
3. **TUI 底栏与抽屉**：`tasks_segment` 以 `☑ 任务 {done}/{total}` 显示合并进度并在全完成时隐藏；`plan_segment` 无 `x/x` 数字且仅在 Plan 模式激活时显示；抽屉 `Plan` 页签与 `Tasks` 页签职责清晰。
4. **桌面端**：`PlanPanel` 不再存在 `plans.length + todos.length` 混加；任务进度准确显示 `x/x`；Plan 模式开关与方案审查 UI 正常可用，`npm run verify` 全绿。

## Consequences and risks

- **工具面缓存与冻结契约**：改动 `builtin_tools()` 名称或 schema 会影响 `crates/pony-agent-core/tests/tools_stop_suite/tool_surface_singularity.rs`（golden hash）与 `tool_surface_order_contract.rs`（工具顺序），实施时需在原槽位替换或显式同步冻结表与契约说明。
- **存量测试与 IPC 迁移面**：`wave36_draft_gate_session.rs`、`wave36_plan_execution_approval.rs`、`wave23_governance_and_plan_sync_contract.rs` 以及 `src-tauri/src/lib.rs` 的 `plan_*` 命令需按新 `PlanMode` 契约同步重构，避免遗留死引用。
- **多方并行写冲突**：本改造横跨 `crates/pony-protocol`、`crates/pony-tools`、`crates/pony-agent-core`、`crates/pony-tui`、`src-tauri` 与 `src/`，新会话实施时应以独立分支/worktree 按 Core → TUI → Desktop 顺序推进。

## Related

- 前置决策（TUI 抽屉与底栏布局）：`.agents/notes/implemented/architecture/2026-11-17-tui-task-visibility-bottom-drawer.md`
- 前置决策（TUI 抽屉面向用户重构）：`.agents/notes/implemented/feature/2026-11-25-tui-drawer-ux-and-subagent-visibility.md`
- 前置决策（底栏 Goal 图标化收敛）：`.agents/notes/implemented/bug-fix/2026-11-27-goal-stop-copy-owner-and-status-bar-icon.md`
- 提示词工具一致性与工具自注册条款：`.agents/notes/implemented/process/2026-11-13-prompt-tool-consistency-gate.md`、`.agents/notes/implemented/architecture/2026-11-13-tool-owned-prompt-guidelines-and-templates.md`
- DeepSeek Harness 参考实现：`/home/dm/deepseek-harness/packages/plan/plan-mode/src/index.ts`、`/home/dm/deepseek-harness/packages/todo/README.md`
- 差距分析矩阵：`docs/audit/agent-core-gap-matrix.md`（A12 / C10）

## Implementation notes（2026-11-27 落地）

按 1A + 2A 方案全部落地，四条 backlog 任务同批闭环：

| 任务 | 落地要点 |
|---|---|
| B1795774426483 | 新增 `crates/pony-tools/src/plan_mode.rs`（会话级 `active`/`pending` + 方案正文 + sidecar 原子写）与 `exit_plan_mode.rs`；`governed_executor` 的 Draft 步骤机门禁改为 `plan_mode_gate`（按权限事实拦写/执行，`ExitPlanMode` 自豁免）；`PlanStore`/`PlanControlHandler`/`plan_execution_approve`/`plan_*` 控制面与 IPC 一步下线，改为 `plan_mode_get`/`set`/`commit`/`review`；工具面 `plan_control` 槽位模型名 `Plan` → `ExitPlanMode`，schema 收敛为单 `plan` 入参（`[decl-change-approved]`，golden 同步）。 |
| B1795774426544 | `context.rs` 回灌标签 `Plan:` → `Todos:`（新增单列回归测试 `context/tests/todo_label_tests.rs`）；`prompt_contract.rs` 新增 `plan_mode_scope` 条款，`schema.rs` 补齐 Plan/Todo/Goal/TeamTask 描述边界；提示词契约升级 v2.4.0（§2 全文 + §6 字节数 3886B）。 |
| B1795774426604 | `TaskGlance` 增 `todos_done/total` 与 `team_done/total`，底栏 `☑ 任务 {done}/{total}(⛔N)` 全完成隐藏；`plan_segment` 去掉数字、仅激活时显示 `▤ Plan 模式`；抽屉 Plan 页签改为模式状态 + 方案正文；`/plan`、`/plan off` 接入模式开关，`/approve` 批准已提交方案。 |
| B1795774426666 | `PlanPanel.vue` 移除 `plans.length + todos.length` 混加，折叠头只显示任务 `x/x`，卡片拆为「Plan 模式（状态 + 方案正文 + 批准执行/继续规划）」与「任务（独立 x/x）」；`src/stores/plan.ts` 改为模式 store；`/plan` 与 UI 入口走 `plan_mode_set`，审批复用 `plan_mode_review`。 |

**与 Proposal 的偏差（均已在代码注释中说明理由）**：

1. `ExitPlanMode` 走的是**宿主 Ask 通道的既有 `WaitingHost` 控制请求面**，没有新增独立工具槽位——沿用原 `plan_control` 槽位保持内建工具数 46 与工具面顺序不变（KV-cache 前缀稳定）。
2. `plan_execution_approval` 形参保留在 `decide_with_context` 签名上但**不再参与判定**（惰性数据），以避免 `DispatchContext` 线缆与 26 个调用点的大面积改动；语义已由 `plan_mode_review` 取代。
3. 底栏任务段在「全部完成」时整段隐藏（方案 A 原文），而不是显示 `n/n`。

**收口补录（2026-11-28，rebase 到 main `2db72111` 后）**：

1. `crates/pony-tools/src/plan_state.rs`（1308 行、17 个内部测试）在首轮改造里只下线了调用面，文件本体与其 `pub mod` 导出仍在仓内但零消费者（`grep -rn "plan_state::"` 除自身外为空）。本次整文件删除并清掉 `pony-tools/src/lib.rs` 与 `agent/mod.rs` 的两处导出，兑现本文「一步下线 `PlanStore`」的承诺。
2. `contract_view_tests` 的两张模型可见名单仍冻结为旧名 `Plan`（与 `names.rs` 的 `ExitPlanMode` 冲突），本次同步为 `ExitPlanMode`——属本文已批准的改名收尾，非新契约。
3. `PlanExecutionApproval` 线缆字段（166 触点 / 35 文件）确认为**有意留手**并登记账本 `B1795821777375`（P2 Debt）：删除需一次收敛 `DispatchContext`/`TurnInput`/前端 `runtime.ts`/契约快照，不宜与本次主改造同 PR 放大评审面。
