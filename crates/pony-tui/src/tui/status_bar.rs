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
    /// 团队看板已完成数。
    pub team_done: usize,
    /// 团队看板任务总数（已完成 + 未完成）。
    pub team_total: usize,
    /// 当前会话里未完成的 todo 数（pending + in_progress）。
    pub todos_open: usize,
    /// 当前会话 todo 已完成数。
    pub todos_done: usize,
    /// 当前会话 todo 总数。
    pub todos_total: usize,
    /// 正在跑（`driving`）的队友数。
    pub agents_running: usize,
    /// 需要人看一眼的队友数（`failed`）。
    pub agents_attention: usize,
    /// 正在跑的子智能体数（`subagent_runs` 中 `Running` 的条目）。
    pub subagents_running: usize,
    /// 失败的子智能体数。
    pub subagents_attention: usize,
    /// 正在跑的外部 ACP 委派数。
    pub acp_running: usize,
}

/// 摘要刷新周期（毫秒）。
///
/// 事件循环每轮都调用 [`sync_task_glance`]（空闲 ~120ms / 流式 ~16ms 一次）。其中
/// `jobs::job_list()` **不是纯读**：`owned` 作业走 `child.try_wait()`，`promoted` 作业走
/// `ProcessManager::poll()` 并把增量输出追加进 ring buffer。按 16ms 刷新等于每秒 60 次系统调用
/// 与 60 次缓冲推进，既浪费 CPU 又扰动 `job_output` 的增量语义。故按固定周期节流，
/// `TuiState::last_glance_sync` 是唯一时间戳。
pub(crate) const GLANCE_REFRESH_MS: u64 = 500;

/// 刷新常驻任务摘要与抽屉 live 数据源：全部是**进程内**注册表（无网络、无 LLM、无磁盘）。
///
/// 与 `sync_tasks_from_store` / `sync_jobs_from_registry` 同一投影纪律：失败只写状态文案，
/// 不让既有快照瞬间归零。节流口径见 [`GLANCE_REFRESH_MS`]：作业起止最多延迟一个周期可见。
pub(crate) fn sync_task_glance(state: &mut TuiState, control_plane: &HostControlPlane) {
    if let Some(last) = state.last_glance_sync {
        if last.elapsed().as_millis() < GLANCE_REFRESH_MS as u128 {
            return;
        }
    }
    state.last_glance_sync = Some(std::time::Instant::now());

    // 团队视图归属**宿主会话**：进入队友/子代理子会话后仍看主会话的团队，不突然变空。
    let host = effective_host_session_id(state).to_string();

    match team_task_list_for_session(&host) {
        Ok(list) => {
            let mut open = 0usize;
            let mut blocked = 0usize;
            let mut done = 0usize;
            for task in &list.tasks {
                match tui_task_status_from_team_status(&task.status) {
                    TuiTaskStatus::Completed => done += 1,
                    TuiTaskStatus::Blocked => {
                        open += 1;
                        blocked += 1;
                    }
                    TuiTaskStatus::Pending | TuiTaskStatus::InProgress => open += 1,
                }
            }
            state.task_glance.team_open = open;
            state.task_glance.team_blocked = blocked;
            state.task_glance.team_done = done;
            state.task_glance.team_total = list.tasks.len();
        }
        Err(error) => state.status_text = format!("任务同步失败: {error}"),
    }

    // 本会话 todo：与团队任务合并计入「☑ 任务」（不重复计数：两者是不同数据源）。
    projection::sync_todos_from_store(state);
    // 本会话 todo：与团队任务合并计入「☑ 任务 x/x」。
    // 归属口径与抽屉一致——todo 按**当前会话**取（`sync_todos_from_store` 已按 session 过滤），
    // 团队看板按**宿主会话**取（子会话里仍看主会话的团队，不突然变空）。
    let (todos_open, todos_done) = state.todos_state.as_ref().map_or((0, 0), |todos| {
        let done = todos
            .iter()
            .filter(|task| task.status == TuiTaskStatus::Completed)
            .count();
        (todos.len() - done, done)
    });
    state.task_glance.todos_open = todos_open;
    state.task_glance.todos_done = todos_done;
    state.task_glance.todos_total = state.todos_state.as_ref().map_or(0, Vec::len);

    let roster = agent_roster_state_for_session(&host);
    state.task_glance.agents_running = roster.entries.iter().filter(|entry| entry.driving).count();
    state.task_glance.agents_attention = roster
        .entries
        .iter()
        .filter(|entry| entry.status.eq_ignore_ascii_case("failed"))
        .count();

    jobs_panel::sync_jobs_from_registry(state);
    // Agents 页签的「ACP 委派」「子智能体」分区：与名册同周期刷新（进程内、无网络）。
    state.acp_delegates = acp_delegates_from_runtime(&host);
    state.subagent_runs = subagent_runs_from_runtime(&host);
    state.task_glance.subagents_running =
        state.subagent_runs.iter().filter(|run| run.is_running()).count();
    state.task_glance.subagents_attention = state
        .subagent_runs
        .iter()
        .filter(|run| run.needs_attention())
        .count();
    state.task_glance.acp_running = state.acp_delegates.len();

    // 抽屉 Plan / Tasks 页签的 live 数据源（plan 存储 + 会话 todo + 团队任务）：
    // 与摘要同周期刷新，让抽屉开着时也能看到轮内计划/待办推进。
    projection::sync_drawer_live_sources(state, control_plane);
    // goal 的安全刷新：只认 `Ok`（瞬时错误不得清横幅），与「打开抽屉」的全量同步区分开。
    projection::sync_goal_state_lossless(state);
}

/// Goal 摘要段（决策 3A，2026-11-27 修订）：`{状态图标} 目标`。
///
/// 只报**状态**，不报轮次：轮次属于抽屉 Goal 页签（那里有空间写 `轮次 3 / 5`），
/// 常驻底栏多一个数字只会挤占宽度，且在目标卡住时被读成「进度」。
/// 状态一律由图标承担（`◎` 续轮在跑 / `◌` 续轮不在跑 / `⏸` 阶段暂停 / `⛔` 阻塞），
/// 与抽屉的阶段词（`phase_text`）不重复占词。
fn goal_segment(state: &TuiState) -> Option<String> {
    let goal = state.goal_state.as_ref()?;
    let glyph = match goal.phase {
        // 完成态不出段 —— 抽屉 Goal 页签仍可回看；常驻左槽只报活跃/需关注态。
        GoalPhase::Completed => return None,
        // 「续轮不在跑」必须与「进行中」区分：否则横幅会谎称还在自动续轮。
        // 图标保持中性（「未在跑」），既不说「停」也不说「未续」——B193 的语义区分
        // （从未启动 ≠ 跑起来后停下）由抽屉的 `visible_line()` 逐字承担，底栏不复制。
        GoalPhase::Active => {
            if state.goal_rounds_stopped.is_some() {
                "◌"
            } else {
                "◎"
            }
        }
        GoalPhase::Paused => "⏸",
        GoalPhase::Blocked => "⛔",
    };
    Some(format!("{glyph} 目标"))
}

/// Plan 模式标识段：**只在模式激活时**出现，且不带任何数字。
///
/// Plan 是会话级协作模式（写/执行锁定），不是任务清单；步骤进度归 `☑ 任务 x/x`。
/// 旧实现把计划步骤计数画在这里、与 Todo 进度并排出现，直接制造了「Plan 还是 Todo」的混淆
/// （ADR 2026-11-27-plan-mode-dsh-alignment-and-task-count）。
fn plan_segment(state: &TuiState) -> Option<String> {
    state.plan_state.as_ref()?.marker().map(str::to_string)
}

/// 任务摘要段（方案 A）：`☑ 任务 {done}/{total}`（有阻塞补 `⛔N`），全部完成时隐藏。
///
/// 合并本会话 todo 与团队任务的**已完成/总数**：两者是不同数据源，相加不重复计数
/// （单人场景只有 todo，团队场景才有 team task）。语法与抽屉 Tasks 页签的 `x/x` 一致。
fn tasks_segment(glance: &TaskGlance) -> Option<String> {
    let done = glance.todos_done + glance.team_done;
    let total = glance.todos_total + glance.team_total;
    if total == 0 || done >= total {
        return None;
    }
    if glance.team_blocked > 0 {
        Some(format!("☑ 任务 {done}/{total}(⛔{})", glance.team_blocked))
    } else {
        Some(format!("☑ 任务 {done}/{total}"))
    }
}

/// 智能体摘要段（决策 3A）：`⚡ 智能体 {running}`（需关注补 `(⚠N)`）。
///
/// 合并三类运行态：Teammate driving + 运行中子代理 + 活跃 ACP 委派；失败数只取
/// Teammate 与子代理（ACP 委派失败即返回，不在注册表留痕）。
///
/// 2026-11-27 修订：需关注数不再以 `✘N` 直接粘在运行数后面（`2·✘1` 会被读成一个数字串，
/// 且 `✘` 与「失败」的语义权重过高）。改与任务段同语法：括号里挂图标计数 `(⚠1)`。
fn agents_segment(glance: &TaskGlance) -> Option<String> {
    let running = glance.agents_running + glance.subagents_running + glance.acp_running;
    let attention = glance.agents_attention + glance.subagents_attention;
    match (running, attention) {
        (0, 0) => None,
        (running, 0) => Some(format!("⚡ 智能体 {running}")),
        (0, attention) => Some(format!("⚡ 智能体 (⚠{attention})")),
        (running, attention) => Some(format!("⚡ 智能体 {running}(⚠{attention})")),
    }
}

/// 后台任务摘要段（决策 3A）：`⚙ 后台 {running}` / `⚙ 后台 ✘{attention}`。
///
/// 只统计当前会话归属的作业（契约 B012，`jobs_state` 已按 `state.session_id` 过滤）。
fn jobs_segment(state: &TuiState) -> Option<String> {
    let view = state.jobs_state.as_ref()?;
    let (running, attention) = view.counts();
    match (running, attention) {
        (0, 0) => None,
        (running, 0) => Some(format!("⚙ 后台 {running}")),
        (0, attention) => Some(format!("⚙ 后台 ✘{attention}")),
        (running, attention) => Some(format!("⚙ 后台 {running}·✘{attention}")),
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
    if let Some(segment) = tasks_segment(&state.task_glance) {
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
