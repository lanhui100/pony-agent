//! 底部**任务抽屉**：`Goal / Plan / Tasks / Agents / Jobs` 五个页签的统一入口。
//!
//! 为什么是抽屉而不是居中弹窗：TUI 的对话正文按 `Paragraph::scroll(offset)` 做窗口切片，
//! 「保留最新消息可见」若靠顶部下拉就得额外做滚动锚点补偿；而抽屉插在输入框基座之上、
//! 参与底部几何，正文区高度自动缩短 ⇒ 最新消息天然在屏（ADR
//! `2026-11-17-tui-task-visibility-bottom-drawer`）。
//!
//! 本模块只做**只读展示**：各关注点的交互仍留在既有面板/命令（`/plan` 的批准、`/agents` 的
//! 进入队友会话等），抽屉不复制任何写路径。

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
}

/// 抽屉状态：当前页签 + 内容滚动偏移（`None` 在 `TuiState` 里表示抽屉关闭）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskCenterState {
    pub tab: TaskCenterTab,
    pub scroll: usize,
}

impl TaskCenterState {
    pub fn new() -> Self {
        Self {
            tab: TaskCenterTab::Goal,
            scroll: 0,
        }
    }

    /// 切页签：滚动归零（否则新页签会继承上一页的偏移，看起来像"内容丢了"）。
    pub fn next_tab(&mut self) {
        self.tab = self.tab.next();
        self.scroll = 0;
    }

    pub fn prev_tab(&mut self) {
        self.tab = self.tab.prev();
        self.scroll = 0;
    }
}

impl Default for TaskCenterState {
    fn default() -> Self {
        Self::new()
    }
}

/// 抽屉高度上限：正文区的 2/3（留 1/3 给最新消息）。
fn height_cap(available_height: u16) -> u16 {
    // 至少 4 行（发丝线 + 页签 + 1 行内容 + 1 行呼吸），上限按 2/3 取。
    let two_thirds = (available_height as u32 * 2 / 3) as u16;
    two_thirds.max(4)
}

/// 当前页签的内容行（纯行 + 样式，无 `Frame` 依赖）。
pub fn content_lines(state: &TuiState) -> Vec<(String, Style)> {
    let Some(center) = state.task_center.as_ref() else {
        return Vec::new();
    };
    let ink = Style::default().fg(theme::INK);
    let faint = Style::default().fg(theme::GHOST);
    let warn = Style::default().fg(theme::DANGER);

    match center.tab {
        TaskCenterTab::Goal => goal_panel::render_lines(state)
            .into_iter()
            .map(|line| {
                let style = if line.starts_with("已停止") {
                    warn
                } else if line.is_empty() {
                    faint
                } else {
                    ink
                };
                (line, style)
            })
            .collect(),
        TaskCenterTab::Plan => {
            let Some(plan) = state.plan_state.as_ref() else {
                return vec![
                    ("当前会话还没有计划".to_string(), faint),
                    (
                        "模型调用 Plan 后会实时显示步骤进度；/plan 可打开完整面板".to_string(),
                        faint,
                    ),
                ];
            };
            let mut rows: Vec<(String, Style)> = Vec::new();
            rows.push((
                format!(
                    "实施计划：{}（{}/{} 已完成{})",
                    plan.summary.trim().replace('\n', " "),
                    plan.steps
                        .iter()
                        .filter(|step| step.status == PlanStepStatus::Completed)
                        .count(),
                    plan.steps.len(),
                    if plan.awaiting_approval {
                        " · 待批准"
                    } else {
                        ""
                    }
                ),
                ink,
            ));
            for (index, step) in plan.steps.iter().enumerate() {
                let (mark, style) = match step.status {
                    PlanStepStatus::Completed => ("✔", Style::default().fg(theme::ACCENT)),
                    PlanStepStatus::InProgress => ("●", Style::default().fg(theme::ACCENT)),
                    PlanStepStatus::Pending => ("○", faint),
                    PlanStepStatus::Failed => ("✘", warn),
                };
                let detail = step.summary.trim();
                let text = if detail.is_empty() {
                    format!("  {mark} {}. {}", index + 1, step.name)
                } else {
                    format!("  {mark} {}. {} — {}", index + 1, step.name, detail)
                };
                rows.push((text, style));
            }
            rows
        }
        TaskCenterTab::Tasks => {
            let Some(board) = state.task_board_state.as_ref() else {
                return vec![
                    ("当前没有共享任务".to_string(), faint),
                    (
                        "Agent 调用 team_task_create 后会实时汇总到这里；/tasks 可打开完整看板"
                            .to_string(),
                        faint,
                    ),
                ];
            };
            let mut rows: Vec<(String, Style)> = Vec::new();
            let total = board.tasks.len();
            let completed = board
                .tasks
                .iter()
                .filter(|task| task.status == TuiTaskStatus::Completed)
                .count();
            rows.push((format!("任务进度：{completed}/{total} 已完成"), ink));
            for task in &board.tasks {
                let (mark, style) = match task.status {
                    TuiTaskStatus::Completed => ("✔", Style::default().fg(theme::ACCENT)),
                    TuiTaskStatus::InProgress => ("●", Style::default().fg(theme::ACCENT)),
                    TuiTaskStatus::Pending => ("○", faint),
                    TuiTaskStatus::Blocked => ("✕", warn),
                };
                let suffix = if task.blocked_by.is_empty() {
                    String::new()
                } else {
                    format!(" · ⛔{}", task.blocked_by.len())
                };
                let owner = task
                    .owner
                    .as_deref()
                    .map(|owner| format!(" · @{owner}"))
                    .unwrap_or_default();
                rows.push((format!("  {mark} {}{suffix}{owner}", task.subject), style));
            }
            rows
        }
        TaskCenterTab::Agents => {
            let Some(roster) = state.agent_roster_state.as_ref() else {
                return vec![
                    ("当前没有 teammate".to_string(), faint),
                    (
                        "spawn_teammate 派生队友后此处显示真实名册；/agents 可进入队友会话"
                            .to_string(),
                        faint,
                    ),
                ];
            };
            let mut rows: Vec<(String, Style)> = Vec::new();
            let driving = roster.entries.iter().filter(|entry| entry.driving).count();
            rows.push((
                format!(
                    "队友 {} · 驱动中 {driving}",
                    roster.entries.len()
                ),
                ink,
            ));
            for entry in &roster.entries {
                let style = if entry.status.eq_ignore_ascii_case("failed") {
                    warn
                } else if entry.driving {
                    Style::default().fg(theme::ACCENT)
                } else {
                    faint
                };
                let mark = if entry.driving { "●" } else { "○" };
                let inbox = if entry.pending_inbox > 0 {
                    format!(" · 待处理 {}", entry.pending_inbox)
                } else {
                    String::new()
                };
                rows.push((
                    format!(
                        "  {mark} {} · {} · {}{inbox}",
                        entry.name, entry.role, entry.status
                    ),
                    style,
                ));
            }
            rows
        }
        TaskCenterTab::Jobs => jobs_panel::render_lines(state)
            .into_iter()
            .map(|line| {
                let style = if line.contains("failed") {
                    warn
                } else if line.is_empty() {
                    faint
                } else {
                    ink
                };
                (line, style)
            })
            .collect(),
    }
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
    let hint = "Tab 切页签 · ↑↓ 滚动 · Esc 关闭";
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

/// 页签与内容之间的呼吸行数（没有它页签栏会贴住内容，读起来像内容的标题）。
const TAB_CONTENT_GAP: u16 = 1;

/// 呼吸行生效的最小抽屉高度：低于此高度时优先保内容可见，呼吸行让位。
const GAP_MIN_HEIGHT: u16 = 5;

/// 固定开销（发丝线 1 + 页签 1 [+ 呼吸行]）：布局与渲染共用的同一口径。
fn chrome_rows(total_height: u16) -> u16 {
    if total_height >= GAP_MIN_HEIGHT {
        2 + TAB_CONTENT_GAP
    } else {
        2
    }
}

/// 抽屉高度（渲染与布局共用的**唯一**口径）：发丝线 1 + 页签 1 + 呼吸行 + 内容行（最多 [height_cap]）。
pub fn height(state: &TuiState, available_height: u16) -> u16 {
    if state.task_center.is_none() {
        return 0;
    }
    let cap = height_cap(available_height);
    let chrome = chrome_rows(cap);
    let content = content_lines(state).len() as u16;
    (chrome + content.min(cap.saturating_sub(chrome).max(1))).min(cap)
}

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
    // 页签与内容之间留一行呼吸区：没有它页签栏会贴住首行内容（高度不足时让位给内容）。
    let chrome = chrome_rows(rect.height);
    let gap = usize::from(chrome.saturating_sub(2));
    for _ in 0..gap {
        lines.push(blank());
    }

    let content = content_lines(state);
    let visible = usize::from(rect.height.saturating_sub(chrome));
    let max_scroll = content.len().saturating_sub(visible);
    let scroll = center.scroll.min(max_scroll);
    for (text, style) in content.iter().skip(scroll).take(visible) {
        lines.push(Line::from(vec![
            Span::raw(" ".repeat(RAIL)),
            Span::styled(text.clone(), *style),
        ]));
    }
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
    // 先取内容行数（不可变借用），再取可变借用：避免同帧双借用。
    let max = content_lines(state).len().saturating_sub(1);
    let Some(center) = state.task_center.as_mut() else {
        return;
    };
    let next = if delta >= 0 {
        (center.scroll + delta as usize).min(max)
    } else {
        center.scroll.saturating_sub(delta.unsigned_abs())
    };
    center.scroll = next;
}
