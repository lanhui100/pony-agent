//! 底部**任务抽屉**：`Goal / Plan / Tasks / Agents / Jobs` 五个页签的统一入口。
//!
//! 为什么是抽屉而不是居中弹窗：TUI 的对话正文按 `Paragraph::scroll(offset)` 做窗口切片，
//! 「保留最新消息可见」若靠顶部下拉就得额外做滚动锚点补偿；而抽屉插在输入框基座之上、
//! 参与底部几何，正文区高度自动缩短 ⇒ 最新消息天然在屏（ADR
//! `2026-11-17-tui-task-visibility-bottom-drawer`）。
//!
//! 渲染纪律（2026-11-25 重构，ADR `2026-11-25-tui-drawer-ux-and-subagent-visibility`）：
//! - **单一折行口径**：[`build_lines`] 按内容宽度折行，[`height_for_width`] 与 [`render`]
//!   都从它派生 —— 布局预留高度与真实渲染行数必须逐行一致（否则底部呼吸行被截断）；
//! - **面向用户的极简分层**：语义图标 > 英文状态词；主文案（目标正文/任务标题/作业描述）
//!   占据视觉中心，内部实现字段（pid / owned / promoted / task_id）一律不上屏；
//! - **会话归属**：团队名册 / 团队任务 / 子代理按宿主会话过滤（见 `projection`）。

use super::*;

/// 抽屉页签（顺序即展示顺序：从「远处目标」到「近处执行」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskCenterTab {
    Goal,
    Plan,
    Tasks,
    Agents,
    Jobs,
}

impl TaskCenterTab {
    pub const ALL: [TaskCenterTab; 5] = [
        TaskCenterTab::Goal,
        TaskCenterTab::Plan,
        TaskCenterTab::Tasks,
        TaskCenterTab::Agents,
        TaskCenterTab::Jobs,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TaskCenterTab::Goal => "Goal",
            TaskCenterTab::Plan => "Plan",
            TaskCenterTab::Tasks => "Tasks",
            TaskCenterTab::Agents => "Agents",
            TaskCenterTab::Jobs => "Jobs",
        }
    }

    pub fn next(self) -> Self {
        match self {
            TaskCenterTab::Goal => TaskCenterTab::Plan,
            TaskCenterTab::Plan => TaskCenterTab::Tasks,
            TaskCenterTab::Tasks => TaskCenterTab::Agents,
            TaskCenterTab::Agents => TaskCenterTab::Jobs,
            TaskCenterTab::Jobs => TaskCenterTab::Goal,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            TaskCenterTab::Goal => TaskCenterTab::Jobs,
            TaskCenterTab::Plan => TaskCenterTab::Goal,
            TaskCenterTab::Tasks => TaskCenterTab::Plan,
            TaskCenterTab::Agents => TaskCenterTab::Tasks,
            TaskCenterTab::Jobs => TaskCenterTab::Agents,
        }
    }

    /// 该页签是否有「可选行」（决策 1A：只有 Agents / Jobs 支持行焦点）。
    pub fn has_selectable_rows(self) -> bool {
        matches!(self, TaskCenterTab::Agents | TaskCenterTab::Jobs)
    }
}

/// 抽屉状态：当前页签 + 内容滚动偏移 + 行焦点（`None` 在 `TuiState` 里表示抽屉关闭）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCenterState {
    pub tab: TaskCenterTab,
    pub scroll: usize,
    /// 行焦点（仅 Agents / Jobs 页签有意义；0 起，越界由渲染与操作层钳制）。
    pub selected_index: usize,
}

impl TaskCenterState {
    pub fn new() -> Self {
        Self {
            tab: TaskCenterTab::Goal,
            scroll: 0,
            selected_index: 0,
        }
    }

    /// 直接打开指定页签（`/tasks` / `/agents` 的单一真值源入口，决策 2A）。
    pub fn open_tab(tab: TaskCenterTab) -> Self {
        Self {
            tab,
            scroll: 0,
            selected_index: 0,
        }
    }

    /// 切页签：滚动与行焦点归零（否则新页签会继承上一页的偏移与选中，看起来像「内容丢了」）。
    pub fn next_tab(&mut self) {
        self.tab = self.tab.next();
        self.scroll = 0;
        self.selected_index = 0;
    }

    pub fn prev_tab(&mut self) {
        self.tab = self.tab.prev();
        self.scroll = 0;
        self.selected_index = 0;
    }
}

impl Default for TaskCenterState {
    fn default() -> Self {
        Self::new()
    }
}

// -----------------------------------------------------------------------------
// 样式与行原语
// -----------------------------------------------------------------------------

/// 一条**逻辑行**：若干带样式的片段。折行在渲染层按宽度统一处理。
pub(crate) type Row = Vec<Span<'static>>;

/// 行是否处于「行焦点选中」态。
///
/// 用枚举而不是 `bool`：本仓 `clippy::fn_params_excessive_bools` 阈值为 0（新增 bool 入参即判红），
/// 且调用点 `selected: true` 不可读。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RowSelection {
    Selected,
    Unselected,
}

impl RowSelection {
    /// 由「当前选中索引 == 该行索引」推出选中态。
    ///
    /// 入参是 `Option<usize>` 而非 `bool`：本仓 `fn_params_excessive_bools` 阈值为 0，
    /// 且调用点 `from_match(selected == Some(i))` 把比较留在调用方更易读。
    pub(crate) fn from_match(selected: Option<usize>, index: usize) -> Self {
        if selected == Some(index) {
            Self::Selected
        } else {
            Self::Unselected
        }
    }

    /// 选中态判定（跨模块 `jobs_panel` 也用它，避免各自复制 `matches!`）。
    pub(crate) fn is_selected(self) -> bool {
        matches!(self, Self::Selected)
    }
}

fn style_ink() -> Style {
    Style::default().fg(theme::INK)
}

fn style_ink_soft() -> Style {
    Style::default().fg(theme::INK_SOFT)
}

fn style_muted() -> Style {
    Style::default().fg(theme::MUTED)
}

fn style_faint() -> Style {
    Style::default().fg(theme::GHOST)
}

fn style_accent() -> Style {
    Style::default().fg(theme::ACCENT)
}

fn style_danger() -> Style {
    Style::default().fg(theme::DANGER)
}

fn style_warn() -> Style {
    Style::default().fg(theme::WARN)
}

fn style_success() -> Style {
    Style::default().fg(theme::SUCCESS)
}

fn style_disabled() -> Style {
    Style::default().fg(theme::GHOST)
}

/// 空逻辑行（段落间距）。
fn gap_row() -> Row {
    Vec::new()
}

/// 单片段逻辑行。
fn simple_row(text: impl Into<String>, style: Style) -> Row {
    vec![Span::styled(text.into(), style)]
}

// -----------------------------------------------------------------------------
// 行焦点：可选条目抽象（Agents / Jobs 共用）
// -----------------------------------------------------------------------------

/// Agents 页签里一个**可进入**的条目（决策 1A 的 `Enter` 目标）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPick {
    /// 队友子会话（`target` 用于 `enter_teammate_session`）。
    Teammate { target: String, session_id: String },
    /// 子智能体会话（`session_id` 直接进入）。
    Subagent { session_id: String },
}

/// 当前 Agents 页签的可选条目序列（顺序与渲染顺序一致，只含**能进入**的行）。
pub fn agent_picks(state: &TuiState) -> Vec<AgentPick> {
    let mut picks = Vec::new();
    if let Some(roster) = state.agent_roster_state.as_ref() {
        for entry in &roster.entries {
            if entry.target == "lead" {
                continue;
            }
            if let Some(session_id) = entry.session_id.clone() {
                picks.push(AgentPick::Teammate {
                    target: entry.target.clone(),
                    session_id,
                });
            }
        }
    }
    for run in &state.subagent_runs {
        picks.push(AgentPick::Subagent {
            session_id: run.session_id.clone(),
        });
    }
    picks
}

/// 当前页签的可选行数（决策 1A：决定 `↑↓` 是否移动行焦点而不是滚动抽屉）。
pub fn selectable_count(state: &TuiState) -> usize {
    let Some(center) = state.task_center.as_ref() else {
        return 0;
    };
    match center.tab {
        TaskCenterTab::Agents => agent_picks(state).len(),
        TaskCenterTab::Jobs => state
            .jobs_state
            .as_ref()
            .map_or(0, |view| view.jobs.len()),
        _ => 0,
    }
}

/// 移动行焦点并夹到可选范围内（`↑` = `-1`、`↓` = `+1`）；无可选行时不动。
pub fn move_selection(state: &mut TuiState, delta: isize) {
    let count = selectable_count(state);
    if count == 0 {
        return;
    }
    let Some(center) = state.task_center.as_mut() else {
        return;
    };
    let next = if delta >= 0 {
        center
            .selected_index
            .saturating_add(delta.unsigned_abs())
            .min(count - 1)
    } else {
        center.selected_index.saturating_sub(delta.unsigned_abs())
    };
    center.selected_index = next.min(count - 1);
}

// -----------------------------------------------------------------------------
// Goal 页签
// -----------------------------------------------------------------------------

/// `Goal` 页签的行：目标正文是主体（自动换行铺在最上方），阶段/轮次收拢成一行弱化状态，
/// 命令提示用最弱色放在末尾。
fn goal_rows(state: &TuiState) -> Vec<Row> {
    let Some(goal) = state.goal_state.as_ref() else {
        return vec![simple_row(
            "暂无目标 · 输入 /goal <目标描述> 设定",
            style_faint(),
        )];
    };
    let mut rows: Vec<Row> = Vec::new();
    let objective = goal.objective.trim();
    if objective.is_empty() {
        rows.push(simple_row("（目标描述为空）", style_muted()));
    } else {
        // 目标正文按源换行拆段（每段再由渲染层按宽度折行），保留用户原始断句。
        for part in objective.lines() {
            rows.push(simple_row(part.trim().to_string(), style_ink()));
        }
    }
    rows.push(gap_row());
    rows.push(goal_status_row(goal, state.goal_rounds_stopped.as_ref()));
    if let Some(reason) = state.goal_rounds_stopped.as_ref() {
        // 自动续轮已停：措辞由枚举唯一提供（B193 的「未启动」语义不得被硬编码前缀抹掉）。
        let severity = severity_for_goal_stop(reason.severity());
        rows.push(simple_row(
            reason.visible_line(),
            severity_style(severity, style_muted()),
        ));
    }
    rows.push(gap_row());
    rows.push(simple_row(goal_panel::GOAL_PANEL_HINT, style_faint()));
    rows
}

/// 目标状态行：`{图标} {阶段} · {round}/{max} 轮`（只有图标带语义色）。
fn goal_status_row(
    goal: &TuiGoal,
    stopped: Option<&pony_agent_core::agent::goal::GoalRoundStopReason>,
) -> Row {
    let (glyph, glyph_style) = match goal.phase {
        GoalPhase::Active => {
            if stopped.is_some() {
                ("◎", style_warn())
            } else {
                ("◎", style_accent())
            }
        }
        GoalPhase::Paused => ("⏸", style_muted()),
        GoalPhase::Blocked => ("⛔", style_danger()),
        GoalPhase::Completed => ("✔", style_success()),
    };
    let rounds = match goal.max_rounds {
        Some(max) => format!("{}/{max} 轮", goal.round_count),
        None => format!("{} 轮", goal.round_count),
    };
    vec![
        Span::styled(glyph, glyph_style),
        Span::raw(" "),
        Span::styled(phase_text(goal.phase), style_muted()),
        Span::styled(format!(" · {rounds}"), style_faint()),
    ]
}

fn phase_text(phase: GoalPhase) -> &'static str {
    match phase {
        GoalPhase::Active => "进行中",
        GoalPhase::Completed => "已完成",
        GoalPhase::Blocked => "已阻塞",
        GoalPhase::Paused => "已暂停",
    }
}

// -----------------------------------------------------------------------------
// Plan 页签
// -----------------------------------------------------------------------------

fn plan_rows(state: &TuiState) -> Vec<Row> {
    let Some(plan) = state.plan_state.as_ref() else {
        return vec![simple_row("当前会话还没有计划", style_faint())];
    };
    let mut rows: Vec<Row> = Vec::new();
    rows.push(plan_header_row(plan));
    for (index, step) in plan.steps.iter().enumerate() {
        rows.push(plan_step_row(index, step));
    }
    if plan.awaiting_approval {
        rows.push(gap_row());
        rows.push(simple_row(
            "/approve 批准执行 · /plan 查看详情",
            style_faint(),
        ));
    }
    rows
}

fn plan_header_row(plan: &PlanState) -> Row {
    let total = plan.steps.len();
    let done = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::Completed)
        .count();
    let summary = plan.summary.trim();
    let mut row = vec![Span::styled(
        if summary.is_empty() {
            "实施计划".to_string()
        } else {
            summary.to_string()
        },
        style_ink().add_modifier(Modifier::BOLD),
    )];
    row.push(Span::styled(format!(" {done}/{total}"), style_faint()));
    if plan.awaiting_approval {
        row.push(Span::styled(" · 待批准", style_warn()));
    }
    if plan.is_terminal {
        row.push(Span::styled(" · 已结束", style_faint()));
    }
    row
}

fn plan_step_row(index: usize, step: &PlanStep) -> Row {
    let (glyph, glyph_style) = match step.status {
        PlanStepStatus::Completed => ("✔", style_success()),
        PlanStepStatus::InProgress => ("●", style_accent()),
        PlanStepStatus::Pending => ("○", style_faint()),
        PlanStepStatus::Failed => ("✘", style_danger()),
    };
    let mut row = vec![
        Span::styled(glyph, glyph_style),
        Span::raw(" "),
        Span::styled(format!("{}. ", index + 1), style_faint()),
        Span::styled(step.name.clone(), style_ink_soft()),
    ];
    let detail = step.summary.trim();
    if !detail.is_empty() {
        row.push(Span::styled(format!(" — {detail}"), style_faint()));
    }
    row
}

// -----------------------------------------------------------------------------
// Tasks 页签（会话 todo + 团队任务）
// -----------------------------------------------------------------------------

fn tasks_rows(state: &TuiState) -> Vec<Row> {
    let todos = state.todos_state.as_deref().unwrap_or(&[]);
    let team = state
        .task_board_state
        .as_ref()
        .map_or(&[][..], |board| board.tasks.as_slice());
    if todos.is_empty() && team.is_empty() {
        return vec![simple_row("当前没有任务", style_faint())];
    }
    let mut rows: Vec<Row> = Vec::new();
    let both = !todos.is_empty() && !team.is_empty();
    if !todos.is_empty() {
        let header = if both {
            format!("本会话待办{}", task_progress(todos))
        } else {
            format!("任务{}", task_progress(todos))
        };
        rows.push(simple_row(header, style_muted()));
        rows.extend(todos.iter().map(task_row));
    }
    if !team.is_empty() {
        if both {
            rows.push(gap_row());
        }
        let header = if both {
            format!("团队任务{}", task_progress(team))
        } else {
            format!("任务{}", task_progress(team))
        };
        rows.push(simple_row(header, style_muted()));
        rows.extend(team.iter().map(task_row));
    }
    rows
}

/// 进度后缀：` 2/4`（完成数/总数）。
fn task_progress(tasks: &[TuiTask]) -> String {
    let completed = tasks
        .iter()
        .filter(|task| task.status == TuiTaskStatus::Completed)
        .count();
    format!(" {}/{}", completed, tasks.len())
}

fn task_row(task: &TuiTask) -> Row {
    let (glyph, glyph_style) = match task.status {
        TuiTaskStatus::Completed => ("✔", style_success()),
        TuiTaskStatus::InProgress => ("●", style_accent()),
        TuiTaskStatus::Pending => ("○", style_faint()),
        TuiTaskStatus::Blocked => ("✕", style_danger()),
    };
    let text_style = if task.status == TuiTaskStatus::Completed {
        style_muted()
    } else {
        style_ink_soft()
    };
    let mut row = vec![
        Span::styled(glyph, glyph_style),
        Span::raw(" "),
        Span::styled(task.subject.clone(), text_style),
    ];
    if let Some(owner) = task.owner.as_deref().filter(|owner| !owner.trim().is_empty()) {
        row.push(Span::styled(format!(" @{owner}"), style_faint()));
    }
    if !task.blocked_by.is_empty() {
        row.push(Span::styled(
            format!(" ⛔{}", task.blocked_by.len()),
            style_warn(),
        ));
    }
    row
}

// -----------------------------------------------------------------------------
// Agents 页签（队友 + 子智能体 + ACP 委派）
// -----------------------------------------------------------------------------

/// 队友可进入提示（逐字冻结，供渲染与测试共用）。
pub(crate) const AGENTS_HINT: &str = "↑↓ 选择 · Enter 进入会话 · Ctrl+↑ 返回主会话";

fn agents_rows(state: &TuiState) -> Vec<Row> {
    let selected = state
        .task_center
        .as_ref()
        .map(|center| center.selected_index);
    let mut rows: Vec<Row> = Vec::new();
    let mut pick_index = 0usize;
    if let Some(roster) = state.agent_roster_state.as_ref() {
        let teammates: Vec<&AgentRosterEntry> = roster
            .entries
            .iter()
            .filter(|entry| entry.target != "lead")
            .collect();
        if !teammates.is_empty() {
            rows.push(simple_row("队友", style_muted()));
            for entry in teammates {
                // 只有「能进入」的行才占用一个可选槽位（顺序与 `agent_picks` 一致）。
                let selection = if entry.session_id.is_some() {
                    let selection = RowSelection::from_match(selected, pick_index);
                    pick_index += 1;
                    selection
                } else {
                    RowSelection::Unselected
                };
                rows.push(teammate_row(entry, selection));
            }
        }
    }
    if !state.subagent_runs.is_empty() {
        rows.push(simple_row("子智能体", style_muted()));
        for run in &state.subagent_runs {
            let selection = RowSelection::from_match(selected, pick_index);
            pick_index += 1;
            rows.push(subagent_row(run, selection));
        }
    }
    if !state.acp_delegates.is_empty() {
        rows.push(simple_row("ACP 委派 · 外部进程", style_muted()));
        for delegate in &state.acp_delegates {
            rows.push(acp_row(delegate));
        }
    }
    if rows.is_empty() {
        return vec![simple_row("当前没有子智能体", style_faint())];
    }
    if pick_index > 0 {
        rows.push(gap_row());
        rows.push(simple_row(AGENTS_HINT, style_faint()));
    }
    rows
}

fn teammate_row(entry: &AgentRosterEntry, selection: RowSelection) -> Row {
    let failed = entry.status.eq_ignore_ascii_case("failed");
    let interrupted = entry.status.eq_ignore_ascii_case("interrupted");
    let (glyph, glyph_style) = if failed {
        ("✘", style_danger())
    } else if interrupted {
        ("✕", style_muted())
    } else if entry.driving {
        ("●", style_accent())
    } else {
        ("○", style_muted())
    };
    let mut row = select_prefix(selection);
    row.push(Span::styled(glyph, glyph_style));
    row.push(Span::raw(" "));
    row.push(Span::styled(entry.name.clone(), select_body_style(selection)));
    row.push(Span::styled(format!("（{}）", entry.role), style_muted()));
    if entry.pending_inbox > 0 {
        row.push(Span::styled(
            format!(" ✉{}", entry.pending_inbox),
            style_warn(),
        ));
    }
    row
}

fn subagent_row(run: &SubagentViewRow, selection: RowSelection) -> Row {
    let mut row = select_prefix(selection);
    let glyph_style = if run.needs_attention() {
        style_danger()
    } else if run.is_running() {
        style_accent()
    } else {
        style_success()
    };
    row.push(Span::styled(run.glyph(), glyph_style));
    row.push(Span::raw(" "));
    let text = if run.description.trim().is_empty() {
        "子智能体".to_string()
    } else {
        run.description.trim().to_string()
    };
    row.push(Span::styled(text, select_body_style(selection)));
    if let Some(error) = run
        .error
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        row.push(Span::styled(format!(" · {error}"), style_danger()));
    } else if !run.is_running() {
        let elapsed = jobs_panel::format_elapsed_ms(run.started_at_ms);
        if !elapsed.is_empty() {
            row.push(Span::styled(format!(" · {elapsed}"), style_faint()));
        }
    }
    row
}

fn acp_row(delegate: &AcpDelegateRow) -> Row {
    let label = delegate
        .description
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map_or_else(|| last_path_segment(&delegate.cwd), str::to_string);
    vec![
        Span::styled("●", style_accent()),
        Span::raw(" "),
        Span::styled(delegate.agent_id.clone(), style_muted()),
        Span::raw(" "),
        Span::styled(label, style_ink_soft()),
        Span::styled(" [外部进程 · 不可切入]", style_disabled()),
    ]
}

/// `cwd` 末段目录名（无分隔符时回落原文）。
fn last_path_segment(path: &str) -> String {
    path.rsplit(['/', '\\'])
        .find(|segment| !segment.is_empty())
        .unwrap_or(path)
        .to_string()
}

/// 选中行前缀：`▏` 选中条（未选中为空；`RAIL` 缩进由渲染层统一补）。
fn select_prefix(selection: RowSelection) -> Row {
    if selection.is_selected() {
        vec![Span::styled("▏", style_accent())]
    } else {
        Vec::new()
    }
}

fn select_body_style(selection: RowSelection) -> Style {
    if selection.is_selected() {
        style_ink().bg(theme::SURFACE_SEL)
    } else {
        style_ink()
    }
}

// -----------------------------------------------------------------------------
// Jobs 页签
// -----------------------------------------------------------------------------

fn jobs_rows(state: &TuiState) -> Vec<Row> {
    let selected = state
        .task_center
        .as_ref()
        .map(|center| center.selected_index);
    jobs_panel::styled_lines(state, selected)
}

// -----------------------------------------------------------------------------
// 行装配（唯一真值）
// -----------------------------------------------------------------------------

/// 当前页签的逻辑行（未折行）：渲染与 `content_lines` 投影共用同一装配点。
pub(crate) fn rows(state: &TuiState) -> Vec<Row> {
    let Some(center) = state.task_center.as_ref() else {
        return Vec::new();
    };
    match center.tab {
        TaskCenterTab::Goal => goal_rows(state),
        TaskCenterTab::Plan => plan_rows(state),
        TaskCenterTab::Tasks => tasks_rows(state),
        TaskCenterTab::Agents => agents_rows(state),
        TaskCenterTab::Jobs => jobs_rows(state),
    }
}

/// 逻辑行 → `(纯文本, 首片段样式)`：保持既有测试与 `task_center_lines` 导出可用。
pub fn content_lines(state: &TuiState) -> Vec<(String, Style)> {
    rows(state)
        .into_iter()
        .map(|row| {
            let text: String = row.iter().map(|span| span.content.as_ref()).collect();
            let style = row.first().map_or_else(style_ink, |span| span.style);
            (text, style)
        })
        .collect()
}

// -----------------------------------------------------------------------------
// 折行与几何
// -----------------------------------------------------------------------------

/// 抽屉页签与内容之间的呼吸行数（没有它页签栏会贴住内容，读起来像内容的标题）。
const TAB_CONTENT_GAP: u16 = 1;

/// 固定开销（发丝线 1 + 页签 1 + 呼吸行）：布局与渲染共用的**唯一**口径。
const CHROME_ROWS: u16 = 2 + TAB_CONTENT_GAP;

/// 兼容口径下的内容宽度（既有单测用 `height(state, available)`，不传宽度）。
pub(crate) const DEFAULT_CONTENT_WIDTH: usize = 100;

/// 抽屉高度上限：正文区的 2/3（留 1/3 给最新消息），至少 4 行。
fn height_cap(available_height: u16) -> u16 {
    let two_thirds = (available_height as u32 * 2 / 3) as u16;
    two_thirds.max(4)
}

/// 把逻辑行按 `width`（含 `RAIL` 前缀列）折行 → 渲染行。
///
/// 渲染（[`render`]）与高度（[`height_for_width`]）都从这里派生，
/// 保证「布局预留 N 行 == 实际画 N 行」。
pub(crate) fn build_lines(state: &TuiState, width: usize) -> Vec<Line<'static>> {
    let body_width = width.saturating_sub(RAIL).max(1);
    let mut lines: Vec<Line<'static>> = Vec::new();
    for row in rows(state) {
        if row.is_empty() {
            lines.push(blank());
            continue;
        }
        let line = Line::from(row);
        for wrapped in wrap_line(&line, body_width) {
            lines.push(prefix_rail(wrapped));
        }
    }
    lines
}

/// 给内容行加 `RAIL` 前缀（左缩进与页签栏对齐）。
fn prefix_rail(line: Line<'static>) -> Line<'static> {
    let mut spans = vec![Span::raw(" ".repeat(RAIL))];
    spans.extend(line.spans);
    Line::from(spans)
}

/// 抽屉高度（渲染与布局共用的**唯一**口径）。
pub(crate) fn height_for_width(state: &TuiState, available_height: u16, width: usize) -> u16 {
    if state.task_center.is_none() {
        return 0;
    }
    let cap = height_cap(available_height);
    let content = build_lines(state, width).len() as u16;
    (CHROME_ROWS + content.min(cap.saturating_sub(CHROME_ROWS).max(1))).min(cap)
}

/// 兼容包装：默认内容宽度下的抽屉高度（既有单测调用）。
pub fn height(state: &TuiState, available_height: u16) -> u16 {
    height_for_width(state, available_height, DEFAULT_CONTENT_WIDTH)
}

/// 选中条目在**折行后**内容序列中的起始行号（供 `↑↓` 自动滚动到可见）。
pub fn selection_line_offset(state: &TuiState, width: usize) -> Option<usize> {
    let center = state.task_center.as_ref()?;
    if !center.tab.has_selectable_rows() {
        return None;
    }
    let body_width = width.saturating_sub(RAIL).max(1);
    let target_row = selected_row_index(state)?;
    let mut offset = 0usize;
    for (index, row) in rows(state).into_iter().enumerate() {
        if index == target_row {
            return Some(offset);
        }
        let wrapped = if row.is_empty() {
            1
        } else {
            wrap_line(&Line::from(row), body_width).len()
        };
        offset += wrapped;
    }
    None
}

/// 选中条目对应的**逻辑行下标**（无选中/无行 ⇒ `None`）。
fn selected_row_index(state: &TuiState) -> Option<usize> {
    let center = state.task_center.as_ref()?;
    match center.tab {
        TaskCenterTab::Agents => agents_selected_row(state, center.selected_index),
        TaskCenterTab::Jobs => jobs_selected_row(state, center.selected_index),
        _ => None,
    }
}

/// Agents 页签：第 N 个可选条目对应的逻辑行下标。
fn agents_selected_row(state: &TuiState, selected: usize) -> Option<usize> {
    let mut pick_index = 0usize;
    let mut row_index = 0usize;
    if let Some(roster) = state.agent_roster_state.as_ref() {
        let teammates: Vec<&AgentRosterEntry> = roster
            .entries
            .iter()
            .filter(|entry| entry.target != "lead")
            .collect();
        if !teammates.is_empty() {
            row_index += 1; // 分区标题
            for entry in teammates {
                if entry.session_id.is_some() {
                    if pick_index == selected {
                        return Some(row_index);
                    }
                    pick_index += 1;
                }
                row_index += 1;
            }
        }
    }
    if !state.subagent_runs.is_empty() {
        row_index += 1; // 分区标题
        for _run in &state.subagent_runs {
            if pick_index == selected {
                return Some(row_index);
            }
            pick_index += 1;
            row_index += 1;
        }
    }
    None
}

/// Jobs 页签：第 N 条作业对应的逻辑行下标（首行是统计行）。
fn jobs_selected_row(state: &TuiState, selected: usize) -> Option<usize> {
    let view = state.jobs_state.as_ref()?;
    if view.jobs.is_empty() {
        return None;
    }
    let index = 1 + selected; // 0 = 统计行
    (index <= view.jobs.len()).then_some(index)
}

// -----------------------------------------------------------------------------
// 渲染
// -----------------------------------------------------------------------------

/// 渲染抽屉：无边框、CANVAS 底色（与 Ask/Queue 内联面板同一视觉语法）。
pub fn render(f: &mut Frame, rect: Rect, state: &TuiState) {
    let Some(center) = state.task_center.as_ref() else {
        return;
    };
    if rect.height == 0 {
        return;
    }
    f.render_widget(ratatui::widgets::Clear, rect);
    let width = rect.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(hairline(width));
    lines.push(tab_bar(state, width));
    for _ in 0..usize::from(TAB_CONTENT_GAP) {
        lines.push(blank());
    }

    let content = build_lines(state, width);
    let visible = usize::from(rect.height.saturating_sub(CHROME_ROWS));
    let max_scroll = content.len().saturating_sub(visible);
    let scroll = center.scroll.min(max_scroll);
    lines.extend(content.into_iter().skip(scroll).take(visible));
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(theme::CANVAS)),
        rect,
    );
}

/// 滚动步长：一次 3 行（与正文滚轮一致），并夹到内容末尾。
pub fn scroll_by(state: &mut TuiState, delta: isize) {
    if state.task_center.is_none() {
        return;
    }
    let total = build_lines(state, DEFAULT_CONTENT_WIDTH)
        .len()
        .saturating_sub(1);
    let Some(center) = state.task_center.as_mut() else {
        return;
    };
    let next = if delta >= 0 {
        center.scroll.saturating_add(delta.unsigned_abs()).min(total)
    } else {
        center.scroll.saturating_sub(delta.unsigned_abs())
    };
    center.scroll = next;
}

/// 页签栏：`▸ Goal  Plan  Tasks  Agents  Jobs` + 右侧键位提示。
fn tab_bar(state: &TuiState, width: usize) -> Line<'static> {
    let Some(center) = state.task_center.as_ref() else {
        return blank();
    };
    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ".repeat(RAIL))];
    for (index, tab) in TaskCenterTab::ALL.iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  "));
        }
        let selected = *tab == center.tab;
        let style = if selected {
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme::MUTED)
        };
        let text = if selected {
            format!("▸ {}", tab.label())
        } else {
            format!("  {}", tab.label())
        };
        spans.push(Span::styled(text, style));
    }
    let hint = "←→ 切页签 · ↑↓ 滚动 · Esc 关闭";
    let used = RAIL
        + TaskCenterTab::ALL
            .iter()
            .map(|tab| str_width(tab.label()) + 2)
            .sum::<usize>();
    let hint_w = str_width(hint);
    if used + hint_w + 2 <= width {
        spans.push(Span::raw(" ".repeat(width.saturating_sub(used + hint_w))));
        spans.push(Span::styled(hint, Style::default().fg(theme::GHOST)));
    }
    Line::from(spans)
}
