//! 输入框下方**状态条**的唯一渲染真源：`任务摘要 | 提示·队列 | 模型·档位·权限`。
//!
//! 取代原有的两处时分轮播（`goal_banner_phase` 的 Goal/指标 6s/8s 交替、
//! `status_carousel_phase` 的模型/权限 3s/3s 交替）——轮播把「信息不足」伪装成
//! 「信息齐全」：用户看不到目标时会以为没有目标（见 ADR
//! `2026-11-17-tui-task-visibility-bottom-drawer`）。
//!
//! 纪律：
//! - **每类信息全屏恰好一处**：数值指标在 header 右槽，本行只承载任务摘要、提示/队列、模型与权限；
//! - **不做分时**：任何一段要么在场，要么因宽度让位（让位顺序固定，写在 [`status_line`] 里）；
//! - 纯函数、无 `Frame`：本模块可无 TTY 单测。

use super::*;

/// 常驻任务摘要的计数快照（由 [`sync_task_glance`] 刷新）。
///
/// 为什么不直接读 `task_board_state` / `agent_roster_state`：那两个字段是**面板生命周期**
/// 的状态（关闭面板时按既有契约清空），而摘要必须常驻；混用会让「关闭面板 ⇒ 摘要段消失」。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TaskGlance {
    /// 团队共享看板里未完成的任务数（pending + in_progress + blocked）。
    pub team_open: usize,
    /// 其中被阻塞的任务数。
    pub team_blocked: usize,
    /// 正在跑（`driving`）的队友数。
    pub agents_running: usize,
    /// 需要人看一眼的队友数（`failed`）。
    pub agents_attention: usize,
}

/// 摘要刷新周期（毫秒）。
///
/// 事件循环每轮都调用 [`sync_task_glance`]（空闲 ~120ms / 流式 ~16ms 一次）。其中
/// `jobs::job_list()` **不是纯读**：`owned` 作业走 `child.try_wait()`，`promoted` 作业走
/// `ProcessManager::poll()` 并把增量输出追加进 ring buffer。按 16ms 刷新等于每秒 60 次系统调用
/// 与 60 次缓冲推进，既浪费 CPU 又扰动 `job_output` 的增量语义。故按固定周期节流，
/// `TuiState::last_glance_sync` 是唯一时间戳。
pub const GLANCE_REFRESH_MS: u64 = 500;

/// 刷新常驻任务摘要：三条数据源都是**进程内**注册表（无网络、无 LLM、无磁盘）。
///
/// 与 `sync_tasks_from_store` / `sync_jobs_from_registry` 同一投影纪律：失败只写状态文案，
/// 不让既有快照瞬间归零。节流口径见 [`GLANCE_REFRESH_MS`]：作业起止最多延迟一个周期可见。
pub fn sync_task_glance(state: &mut TuiState) {
    if let Some(last) = state.last_glance_sync {
        if last.elapsed().as_millis() < GLANCE_REFRESH_MS as u128 {
            return;
        }
    }
    state.last_glance_sync = Some(std::time::Instant::now());

    match team_task_list() {
        Ok(list) => {
            let mut open = 0usize;
            let mut blocked = 0usize;
            for task in &list.tasks {
                match tui_task_status_from_team_status(&task.status) {
                    TuiTaskStatus::Completed => {}
                    TuiTaskStatus::Blocked => {
                        open += 1;
                        blocked += 1;
                    }
                    TuiTaskStatus::Pending | TuiTaskStatus::InProgress => open += 1,
                }
            }
            state.task_glance.team_open = open;
            state.task_glance.team_blocked = blocked;
        }
        Err(error) => state.status_text = format!("任务同步失败: {error}"),
    }

    let roster = agent_roster_state_from_runtime();
    state.task_glance.agents_running = roster.entries.iter().filter(|entry| entry.driving).count();
    state.task_glance.agents_attention = roster
        .entries
        .iter()
        .filter(|entry| entry.status.eq_ignore_ascii_case("failed"))
        .count();

    jobs_panel::sync_jobs_from_registry(state);
}

/// Goal 摘要段（`None` = 没有目标，不出段）。
fn goal_segment(state: &TuiState) -> Option<String> {
    let goal = state.goal_state.as_ref()?;
    let rounds = match goal.max_rounds {
        Some(max) => format!("轮{}/{}", goal.round_count, max),
        None => format!("轮{}", goal.round_count),
    };
    let suffix = match goal.phase {
        GoalPhase::Active => {
            // 「已停止」与「进行中」必须共存可见：否则横幅会谎称还在自动续轮。
            if state.goal_rounds_stopped.is_some() {
                "·停"
            } else {
                ""
            }
        }
        GoalPhase::Paused => "·暂停",
        GoalPhase::Blocked => "·阻塞",
        GoalPhase::Completed => "·完成",
    };
    Some(format!("goal {rounds}{suffix}"))
}

/// Plan 摘要段：`plan 已完成/总数`，待批准时补 `待批`。
fn plan_segment(state: &TuiState) -> Option<String> {
    let plan = state.plan_state.as_ref()?;
    if plan.steps.is_empty() {
        return None;
    }
    let total = plan.steps.len();
    let done = plan
        .steps
        .iter()
        .filter(|step| step.status == PlanStepStatus::Completed)
        .count();
    let suffix = if plan.awaiting_approval { "·待批" } else { "" };
    Some(format!("plan {done}/{total}{suffix}"))
}

/// 团队看板摘要段：未完成数为 0 时不出段；有阻塞补 `(⛔n)`。
fn team_segment(glance: &TaskGlance) -> Option<String> {
    if glance.team_open == 0 {
        return None;
    }
    if glance.team_blocked > 0 {
        Some(format!(
            "团队 {}(⛔{})",
            glance.team_open, glance.team_blocked
        ))
    } else {
        Some(format!("团队 {}", glance.team_open))
    }
}

/// 队友摘要段：只报「在跑」与「失败」，都为空时不出段。
fn agents_segment(glance: &TaskGlance) -> Option<String> {
    match (glance.agents_running, glance.agents_attention) {
        (0, 0) => None,
        (running, 0) => Some(format!("队友 {running}")),
        (0, attention) => Some(format!("队友 ⚠{attention}")),
        (running, attention) => Some(format!("队友 {running}⚠{attention}")),
    }
}

/// 作业摘要段：只报「在跑」与「失败」，都为空时不出段。
fn jobs_segment(state: &TuiState) -> Option<String> {
    let view = state.jobs_state.as_ref()?;
    let (running, attention) = view.counts();
    match (running, attention) {
        (0, 0) => None,
        (running, 0) => Some(format!("作业 {running}")),
        (0, attention) => Some(format!("作业 ⚠{attention}")),
        (running, attention) => Some(format!("作业 {running}⚠{attention}")),
    }
}

/// 任务摘要段（左槽）：按固定顺序拼装，任何一段为空即整段省略。
pub fn task_summary(state: &TuiState) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    if let Some(segment) = goal_segment(state) {
        parts.push(segment);
    }
    if let Some(segment) = plan_segment(state) {
        parts.push(segment);
    }
    if let Some(segment) = team_segment(&state.task_glance) {
        parts.push(segment);
    }
    if let Some(segment) = agents_segment(&state.task_glance) {
        parts.push(segment);
    }
    if let Some(segment) = jobs_segment(state) {
        parts.push(segment);
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

/// 中槽：一次性提示（TTL 内、非流式）优先，其次排队反馈。
fn middle_segment(state: &TuiState) -> Option<(String, Style)> {
    let notice = state
        .status_notice
        .as_ref()
        .filter(|(_, at)| {
            should_render_status_notice(
                state.is_streaming,
                at.elapsed().as_millis() as u64,
                STATUS_NOTICE_TTL_MS,
            )
        })
        .map(|(notice, _)| {
            let style = severity_style(system_message_severity(notice), muted());
            (truncate_width(notice, 34), style)
        });
    if notice.is_some() {
        return notice;
    }
    queue_indicator_line(
        state.queued_count(),
        state
            .queue_notice
            .as_ref()
            .filter(|(_, at)| at.elapsed().as_millis() as u64 <= QUEUE_NOTICE_TTL_MS)
            .map(|(text, _)| text.as_str()),
        state.is_streaming,
        matches!(state.runtime_mode, RuntimeMode::ExecutingTool(_)),
    )
    .map(|indicator| {
        (
            truncate_width(&indicator, 34),
            Style::default().fg(theme::GHOST),
        )
    })
}

/// 右槽候选段（顺序 = 保留优先级）：模型 · 档位 → 权限 · 级别。
///
/// 与旧实现不同：两者**同时**在场，不再 3s/3s 交替；宽度不足时先丢权限。
fn identity_segments(state: &TuiState) -> Vec<(String, Style)> {
    let model = match state.reasoning_effort {
        Some(_) => format!("{} · {}", state.model, state.reasoning_effort_label()),
        None => state.model.clone(),
    };
    let (perm_text, perm_style) = match state.permission_mode {
        PermissionMode::ReadOnly => ("权限 · 只读".to_string(), Style::default().fg(theme::WARN)),
        PermissionMode::WorkspaceWrite => {
            ("权限 · 工作区".to_string(), Style::default().fg(theme::MUTED))
        }
        PermissionMode::FullAccess => (
            "权限 · 完全访问".to_string(),
            Style::default().fg(theme::ACCENT),
        ),
    };
    vec![
        (model, Style::default().fg(theme::GHOST)),
        (perm_text, perm_style),
    ]
}

/// 状态条整行：左 = 任务摘要，中 = 提示/队列，右 = 模型·档位 + 权限。
///
/// 让位顺序（宽度从紧到宽）：**右槽权限 → 中槽 → 左槽摘要 → 压缩模型**。
/// 即模型身份最后退出（它是「我在用什么」的唯一家），任务摘要最先整段让位。
pub fn status_line(state: &TuiState, width: usize) -> Line<'static> {
    let width = width.max(1);
    let content = width.saturating_sub(RAIL);
    let sep = " · ";
    let sep_w = str_width(sep);

    let mut summary = task_summary(state);
    let mut middle = middle_segment(state);
    let mut identity = identity_segments(state);

    let identity_width = |segments: &[(String, Style)]| -> usize {
        segments.iter().map(|(text, _)| str_width(text)).sum::<usize>()
            + sep_w * segments.len().saturating_sub(1)
    };

    // 让位循环：只做「整段丢弃」，省略号留给最后一步。
    loop {
        let left_w = summary.as_ref().map_or(0, |text| str_width(text));
        let mid_w = middle.as_ref().map_or(0, |(text, _)| str_width(text));
        let right_w = identity_width(&identity);
        let gaps = if left_w > 0 && mid_w > 0 { sep_w } else { 0 };
        if RAIL + left_w + mid_w + right_w + gaps <= width {
            break;
        }
        if identity.len() > 1 {
            identity.pop();
        } else if middle.take().is_some() {
            continue;
        } else if summary.take().is_some() {
            continue;
        } else {
            break;
        }
    }

    let mut spans: Vec<Span<'static>> = vec![Span::raw(" ".repeat(RAIL))];
    let mut used = RAIL;

    let mid_w = middle.as_ref().map_or(0, |(text, _)| str_width(text));
    let right_w = identity_width(&identity);
    // 左槽预算：给中槽与右槽留位（右槽右对齐，其左侧至少留 1 列空格）。
    let left_budget = content.saturating_sub(
        mid_w + right_w + sep_w * usize::from(mid_w > 0) + usize::from(right_w > 0),
    );
    if let Some(text) = summary.as_ref() {
        let trimmed = ellipsize_width(text, left_budget);
        if str_width(&trimmed) >= 4 {
            used += str_width(&trimmed);
            spans.push(Span::styled(trimmed, Style::default().fg(theme::MUTED)));
        }
    }

    if let Some((text, style)) = middle.as_ref() {
        if used > RAIL {
            spans.push(Span::raw(sep));
            used += sep_w;
        }
        used += str_width(text);
        spans.push(Span::styled(text.clone(), *style));
    }

    if right_w > 0 {
        let pad = width.saturating_sub(used + right_w);
        if pad == 0 && used > RAIL {
            spans.push(Span::raw(sep));
        } else {
            spans.push(Span::raw(" ".repeat(pad)));
        }
        for (index, (text, style)) in identity.iter().enumerate() {
            if index > 0 {
                spans.push(Span::raw(sep));
            }
            spans.push(Span::styled(text.clone(), *style));
        }
    }

    Line::from(spans)
}
