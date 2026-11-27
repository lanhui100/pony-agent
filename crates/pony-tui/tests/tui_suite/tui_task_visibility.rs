//! 任务可见性重构的验收测试（ADR `2026-11-17-tui-task-visibility-bottom-drawer`）：
//!
//! 1. 状态条左槽的**常驻任务摘要**（goal / plan / 团队 / 队友 / 后台任务）与让位顺序；
//! 2. 底部**任务抽屉**：高度上限、页签轮转、渲染时不遮最新消息；
//! 3. 后台**作业投影**：wire 状态解析与计数；
//! 4. 权限审批**底部内联**：不再整块清正文。
//!
//! 2026-11-22 契约修订（ADR `2026-11-22-tui-task-drawer-visibility`）：
//! - 摘要第五段措辞由「作业」改为「后台任务」（用户心智 = background jobs），且**不再带 `⚠`**；
//! - 抽屉 Jobs 空态由 3 行改成 1 行「当前没有后台任务」（空态一律一行）。
//!
//! 渲染接缝沿用 `tests/tui_ask_panel_layout.rs`：`#[path] mod tui` + TestBackend。

use pony_tui as tui;

use pony_agent_core::agent::jobs::JobListItem;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;
use tui::{
    GoalActivation, GoalPhase, JobStatus, JobsViewState, PermissionChoice, PermissionRequest,
    PermissionState, PlanState, PlanStep, PlanStepStatus, TaskCenterState, TaskCenterTab, TuiGoal,
    TuiState,
};

const LATEST_SENTINEL: &str = "LATEST_SENTINEL_最新一条正文";

fn render(state: &mut TuiState, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("create test backend");
    terminal
        .draw(|frame| tui::ui(frame, state))
        .expect("render frame");
    terminal.backend().buffer().clone()
}

fn all_text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

fn row_text(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol().to_string())
        .collect()
}

fn goal(round: u32, max: Option<u32>, phase: GoalPhase) -> TuiGoal {
    TuiGoal {
        id: "g1".to_string(),
        objective: "把任务可见性收敛到一处".to_string(),
        phase,
        activation: GoalActivation::Armed,
        round_count: round,
        max_rounds: max,
        revision: 1,
    }
}

fn plan_with(done: usize, total: usize) -> PlanState {
    let mut plan = PlanState::new("p1".to_string(), "步骤".to_string());
    for index in 0..total {
        plan.steps.push(PlanStep {
            step_id: format!("s{index}"),
            name: format!("步骤 {index}"),
            summary: String::new(),
            status: if index < done {
                PlanStepStatus::Completed
            } else {
                PlanStepStatus::Pending
            },
        });
    }
    plan
}

fn job(status: &str) -> JobListItem {
    JobListItem {
        job_id: "job-1".to_string(),
        command: "npm run dev".to_string(),
        status: status.to_string(),
        pid: 4242,
        started_at: 0,
        kind: "owned".to_string(),
        description: None,
    }
}

// ---------------------------------------------------------------------------
// 1. 状态条摘要
// ---------------------------------------------------------------------------

#[test]
fn summary_is_absent_when_nothing_is_running() {
    let state = TuiState::default();
    assert!(
        tui::task_summary(&state).is_none(),
        "没有任何任务信息时摘要段必须整段消失（不留空分隔符）"
    );
}

#[test]
fn summary_lists_goal_plan_tasks_agents_jobs_in_order() {
    // 决策 3A：左槽采用「语义图标 + 极简中文短词」，顺序
    // 目标 → 计划 → 任务 → 智能体 → 后台。
    let mut state = TuiState::default();
    state.set_goal(goal(3, Some(5), GoalPhase::Active));
    state.plan_state = Some(plan_with(2, 5));
    state.task_glance.team_open = 4;
    state.task_glance.team_blocked = 1;
    state.task_glance.agents_running = 2;
    state.jobs_state = Some(JobsViewState::from_items(vec![job("running")]));

    let summary = tui::task_summary(&state).expect("摘要必须存在");
    let positions = ["◎ 目标", "▤ 计划", "☑ 任务", "⚡ 智能体", "⚙ 后台"]
        .map(|needle| summary.find(needle).unwrap_or_else(|| panic!("缺少 {needle} 段：{summary:?}")));
    assert!(
        positions.windows(2).all(|pair| pair[0] < pair[1]),
        "摘要段顺序必须是 目标 → 计划 → 任务 → 智能体 → 后台，实际 {summary:?}"
    );
    assert!(
        summary.contains("◎ 目标"),
        "目标段必须出现且带状态图标：{summary:?}"
    );
    assert!(
        !summary.contains("3/5"),
        "底栏目标段不再显示轮次（轮次只属于抽屉 Goal 页签）：{summary:?}"
    );
    assert!(summary.contains("2/5"), "计划段必须带完成比：{summary:?}");
    assert!(summary.contains("⛔1"), "任务段必须标阻塞数：{summary:?}");
}

#[test]
fn summary_marks_goal_without_auto_rounds_by_icon_only() {
    let mut state = TuiState::default();
    state.set_goal(goal(2, Some(5), GoalPhase::Active));
    state.goal_rounds_stopped = Some(pony_agent_core::agent::goal::GoalRoundStopReason::Cancelled);
    let summary = tui::task_summary(&state).expect("摘要必须存在");
    assert!(
        summary.contains("◌ 目标"),
        "续轮未在跑时目标段必须换状态图标，实际 {summary:?}"
    );
    assert!(
        !summary.contains("轮") && !summary.contains("停"),
        "底栏不再用文字/轮次表达续轮状态，只用图标：{summary:?}"
    );
}

#[test]
fn summary_marks_paused_and_blocked_goal_by_icon() {
    for (phase, glyph) in [
        (GoalPhase::Paused, "⏸ 目标"),
        (GoalPhase::Blocked, "⛔ 目标"),
    ] {
        let mut state = TuiState::default();
        state.set_goal(goal(1, Some(5), phase));
        let summary = tui::task_summary(&state).expect("摘要必须存在");
        assert!(
            summary.contains(glyph),
            "{phase:?} 阶段必须用 {glyph} 图标表达，实际 {summary:?}"
        );
    }
}

#[test]
fn status_line_keeps_model_identity_when_width_is_tight() {
    let mut state = TuiState::default();
    state.model = "fixture-model".to_string();
    state.set_goal(goal(9, Some(9), GoalPhase::Active));
    state.task_glance.team_open = 7;
    state.task_glance.agents_running = 3;

    let narrow = tui::status_bar_line(&state, 30)
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect::<String>();
    assert!(
        narrow.contains("fixture-model"),
        "窄屏下模型身份最后退出，实际 {narrow:?}"
    );
    assert!(
        !narrow.contains("队友"),
        "窄屏下摘要段必须先让位，实际 {narrow:?}"
    );
}

// ---------------------------------------------------------------------------
// 2. 任务抽屉
// ---------------------------------------------------------------------------

#[test]
fn drawer_height_is_zero_when_closed() {
    let state = TuiState::default();
    assert_eq!(tui::task_center_height(&state, 30), 0);
}

#[test]
fn drawer_height_never_exceeds_two_thirds() {
    let mut state = TuiState::default();
    state.task_center = Some(TaskCenterState::new());
    state.plan_state = Some(plan_with(1, 40));
    for available in [6u16, 10, 30, 60] {
        let height = tui::task_center_height(&state, available);
        let cap = (available as u32 * 2 / 3) as u16;
        assert!(
            height <= cap.max(4),
            "抽屉高度 {height} 不得超过可用区 {available} 的 2/3（上限 {cap}）"
        );
        assert!(height >= 4, "抽屉至少保留发丝线 + 页签 + 1 行内容：{height}");
    }
}

#[test]
fn drawer_tabs_cycle_forward_and_backward() {
    let mut center = TaskCenterState::new();
    assert_eq!(center.tab, TaskCenterTab::Goal);
    center.next_tab();
    assert_eq!(center.tab, TaskCenterTab::Plan);
    center.prev_tab();
    assert_eq!(center.tab, TaskCenterTab::Goal);
    center.prev_tab();
    assert_eq!(center.tab, TaskCenterTab::Jobs, "反向轮转必须回绕到末尾页签");
}

#[test]
fn drawer_puts_a_breathing_row_between_tabs_and_content() {
    let mut state = TuiState::default();
    let mut center = TaskCenterState::new();
    // 空态首行文案稳定，便于断言「呼吸行之后就是内容第一行」。
    center.tab = TaskCenterTab::Plan;
    state.task_center = Some(center);
    let buf = render(&mut state, 110, 30);
    let rows: Vec<String> = (0..buf.area.height).map(|y| row_text(&buf, y)).collect();
    let tab_row = rows
        .iter()
        .position(|row| row.contains("Goal") && row.contains("Plan") && row.contains("Jobs"))
        .expect("页签栏必须渲染");
    assert!(
        rows[tab_row + 1].trim().is_empty(),
        "页签与内容之间必须留一行呼吸区，实际：{:?}",
        rows[tab_row + 1]
    );
    assert!(
        rows[tab_row + 2].replace(' ', "").contains("当前会话还没有计划"),
        "呼吸行之后应是内容首行，实际：{:?}",
        rows[tab_row + 2]
    );
}

#[test]
fn drawer_render_keeps_latest_message_visible() {
    let mut state = TuiState::default();
    state.add_user_message(LATEST_SENTINEL);
    state.task_center = Some(TaskCenterState::new());
    let buf = render(&mut state, 110, 30);
    let text = all_text(&buf);
    assert!(
        text.contains("LATEST_SENTINEL"),
        "抽屉打开时最新消息必须仍在屏（正文区让位而非被遮），实际屏幕：{text:?}"
    );
    assert!(
        text.contains("Goal") && text.contains("Jobs"),
        "抽屉必须渲染页签栏，实际屏幕：{text:?}"
    );
}

// ---------------------------------------------------------------------------
// 3. 作业投影
// ---------------------------------------------------------------------------

#[test]
fn job_status_unknown_value_degrades_to_failed() {
    assert_eq!(JobStatus::from_wire("running"), JobStatus::Running);
    assert_eq!(JobStatus::from_wire("completed"), JobStatus::Completed);
    assert_eq!(JobStatus::from_wire("killed"), JobStatus::Killed);
    assert_eq!(
        JobStatus::from_wire("something-new"),
        JobStatus::Failed,
        "未知状态不得被吞成 running（会让已结束作业长期算作在跑）"
    );
}

#[test]
fn jobs_view_counts_running_and_attention_separately() {
    let view = JobsViewState::from_items(vec![
        job("running"),
        job("failed"),
        job("killed"),
        job("completed"),
    ]);
    assert_eq!(view.counts(), (1, 1), "killed/completed 不计入运行中或需关注");
}

#[test]
fn jobs_tab_renders_an_actionable_empty_state() {
    let mut state = TuiState::default();
    state.jobs_state = Some(JobsViewState::default());
    let center = TaskCenterState {
        tab: TaskCenterTab::Jobs,
        scroll: 0,
        selected_index: 0,
    };
    state.task_center = Some(center);
    let lines = tui::task_center_lines(&state);
    let text = lines
        .iter()
        .map(|(line, _)| line.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        text.contains("当前没有后台任务"),
        "空态必须给出可读结论（且只一行），实际 {text:?}"
    );
}

// ---------------------------------------------------------------------------
// 4. 权限审批底部内联
// ---------------------------------------------------------------------------

fn permission_state() -> PermissionState {
    let request = PermissionRequest {
        request_id: "req-1".to_string(),
        session_id: "s1".to_string(),
        tool_name: "Bash".to_string(),
        description: "执行构建".to_string(),
        command_or_path: Some("npm run build".to_string()),
        approval_path: None,
        timestamp_ms: 0,
    };
    let mut state = PermissionState::new(request);
    state.selected_choice_index = 0;
    assert_eq!(state.selected_choice(), PermissionChoice::AllowOnce);
    state
}

#[test]
fn permission_approval_renders_inline_without_hiding_latest_message() {
    let mut state = TuiState::default();
    state.add_user_message(LATEST_SENTINEL);
    state.permission_state = Some(permission_state());
    state.mode = tui::InteractionMode::PermissionApproval;
    let buf = render(&mut state, 110, 30);
    let text = all_text(&buf);
    // CJK 宽字符在 buffer 里占两格（右格是填充空白），断言必须走「去空白字形序列」口径。
    let flat: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(flat.contains("审批"), "底部必须出现内联审批面板：{text:?}");
    assert!(
        text.contains("AllowOnce"),
        "审批选项必须可见，实际屏幕：{text:?}"
    );
    assert!(
        text.contains("LATEST_SENTINEL"),
        "审批不再整块清正文，最新消息必须仍在屏：{text:?}"
    );
    assert!(
        !flat.contains("权限审批请求"),
        "居中弹窗标题必须消失（改为底部内联）：{text:?}"
    );
}

// ---------------------------------------------------------------------------
// 5. 2026-11-25 重构：抽屉与输入框间距、真实渲染下的长文本折行
// ---------------------------------------------------------------------------

/// 抽屉下缘与输入框色带之间必须恒定留 `DRAWER_INPUT_GAP` 行空白（CANVAS 呼吸）。
#[test]
fn drawer_keeps_breathing_row_above_composer() {
    let mut state = TuiState::default();
    state.task_center = Some(TaskCenterState::new());
    let buf = render(&mut state, 110, 30);

    // 找到抽屉页签行与输入框提示符行，断言两者之间至少隔 2 行（抽屉内容末行 + 1 行空白）。
    let rows: Vec<String> = (0..buf.area.height).map(|y| row_text(&buf, y)).collect();
    let tab_row = rows
        .iter()
        .position(|row| row.contains("Goal") && row.contains("Jobs"))
        .expect("页签栏必须渲染");
    let composer_row = rows
        .iter()
        .rposition(|row| row.contains('›') || row.contains('>'))
        .expect("输入框提示符必须渲染");
    assert!(
        composer_row > tab_row,
        "输入框必须在抽屉之下：tab={tab_row} composer={composer_row}"
    );
    assert!(
        composer_row.saturating_sub(tab_row) >= 3,
        "抽屉（发丝线+页签+内容）与输入框之间必须留有间距：tab={tab_row} composer={composer_row}"
    );
}

/// 真实渲染下长目标必须折行显示（旧实现整行渲染 ⇒ 右侧被终端裁掉）。
#[test]
fn long_goal_objective_is_wrapped_in_real_render() {
    let mut state = TuiState::default();
    state.task_center = Some(TaskCenterState::new());
    let tail_sentinel = "结尾哨兵Z";
    let objective = format!(
        "把底部任务抽屉里的目标正文完整显示出来，不能因为终端宽度不足而在右侧被硬截断掉后半段内容{tail_sentinel}"
    );
    state.set_goal(TuiGoal {
        id: "goal-long".to_string(),
        objective,
        phase: GoalPhase::Active,
        activation: GoalActivation::Armed,
        round_count: 1,
        max_rounds: Some(5),
        revision: 1,
    });

    for width in [60u16, 80, 110] {
        let buf = render(&mut state, width, 30);
        let flat: String = all_text(&buf).chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            flat.contains(tail_sentinel),
            "宽度 {width} 下目标正文结尾必须可见（折行而非截断）：{flat}"
        );
    }
}
