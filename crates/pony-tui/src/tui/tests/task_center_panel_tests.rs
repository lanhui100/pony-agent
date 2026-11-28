//! Ctrl+T 任务抽屉的**表现层不变量**（2026-11-22 批次 A）。
//!
//! 只锁三件事，均可在无 TTY 环境断言：
//! 1. 五个页签在「没有数据」时都是**恰好一行**空态（旧实现 2~3 行解释性长句）；
//! 2. 页签切换支持 `←/→`（`prev_tab/next_tab` 环绕，且切页签滚动归零）；
//! 3. 底部状态栏的后台任务段用「后台任务」而非旧词「作业」，且计数旁不再带 `⚠`。
//!
//! 数据接线（Plan/Tasks/Goal 真实数据源）与归属（Jobs 会话过滤、ACP 名册）由后续批次
//! 的测试覆盖，本文件不重复。

use super::*;

/// 打开抽屉（默认页签 Goal）的空状态机。
fn drawer_state() -> TuiState {
    let mut state = TuiState::default();
    state.task_center = Some(task_center::TaskCenterState::new());
    state
}

/// 构造一条后台任务视图行（命令/描述/起始时刻都用固定值，断言只看措辞与计数）。
fn job_row(id: &str, status: jobs_panel::JobStatus) -> jobs_panel::JobViewRow {
    jobs_panel::JobViewRow {
        id: id.to_string(),
        command: "sleep 1".to_string(),
        description: None,
        status,
        started_at: 0,
    }
}

#[test]
fn every_tab_without_items_is_exactly_one_line() {
    for tab in task_center::TaskCenterTab::ALL {
        let mut state = drawer_state();
        state.task_center.as_mut().expect("抽屉已打开").tab = tab;
        let lines = task_center::content_lines(&state);
        assert_eq!(
            lines.len(),
            1,
            "{tab:?} 空态必须恰一行（无数据时不得再输出解释性第二行）：{lines:?}"
        );
        assert!(
            !lines[0].0.trim().is_empty(),
            "{tab:?} 空态行不得是空白（否则抽屉看起来像渲染失败）"
        );
    }
}

#[test]
fn jobs_tab_synced_but_empty_is_one_line() {
    let mut state = drawer_state();
    state.task_center.as_mut().expect("抽屉已打开").tab = task_center::TaskCenterTab::Jobs;
    // 「同步了但为空」与「从未同步」是两个状态，但空态都只给一行。
    state.jobs_state = Some(jobs_panel::JobsViewState::default());
    let lines = task_center::content_lines(&state);
    assert_eq!(lines.len(), 1, "jobs 空态必须一行：{lines:?}");
    assert!(
        lines[0].0.contains("没有后台任务"),
        "空态文案应指向「没有后台任务」：{lines:?}"
    );
}

#[test]
fn arrows_cycle_tabs_with_wraparound_and_reset_scroll() {
    let mut center = task_center::TaskCenterState::new();
    assert_eq!(center.tab, task_center::TaskCenterTab::Goal);

    // ← 从首页签回绕到末页签。
    center.prev_tab();
    assert_eq!(center.tab, task_center::TaskCenterTab::Jobs);

    // → 从末页签回绕到首页签。
    center.next_tab();
    assert_eq!(center.tab, task_center::TaskCenterTab::Goal);

    // 切页签必须把滚动偏移归零（否则新页签继承旧偏移，像内容丢失）。
    center.scroll = 9;
    center.next_tab();
    assert_eq!(center.tab, task_center::TaskCenterTab::Plan);
    assert_eq!(center.scroll, 0);
}

#[test]
fn status_bar_uses_icon_and_short_wording_for_jobs() {
    // 决策 3A：底栏左槽是「语义图标 + 极简中文短词」，失败用 `✘N` 而非长句。
    let mut state = TuiState::default();
    state.jobs_state = Some(jobs_panel::JobsViewState {
        jobs: vec![
            job_row("job-running", jobs_panel::JobStatus::Running),
            job_row("job-failed", jobs_panel::JobStatus::Failed),
        ],
    });
    let summary = task_summary(&state).expect("有在跑/失败作业时后台任务段必须出现");
    assert!(
        summary.contains("⚙ 后台 1·✘1"),
        "状态栏应报「⚙ 后台 运行中数·✘失败数」：{summary}"
    );
    assert!(
        !summary.contains('⚠'),
        "计数旁不得再出现警告图标：{summary}"
    );
    assert!(
        !summary.contains("作业") && !summary.contains("后台任务"),
        "旧长措辞不得再出现在状态栏：{summary}"
    );
}

#[test]
fn status_bar_background_task_segment_hides_when_nothing_to_report() {
    let mut state = TuiState::default();
    // 无快照 / 空快照 / 只有已完成作业 ⇒ 都不出段。
    assert!(task_summary(&state).is_none(), "无任务快照时不应有摘要段");
    state.jobs_state = Some(jobs_panel::JobsViewState::default());
    assert!(task_summary(&state).is_none(), "空快照不应有摘要段");
    state.jobs_state = Some(jobs_panel::JobsViewState {
        jobs: vec![job_row("job-done", jobs_panel::JobStatus::Completed)],
    });
    assert!(task_summary(&state).is_none(), "只有完成态不应有摘要段");
}

// ---------------------------------------------------------------------------
// 2026-11-25 重构：统一折行、Goal 层级、底栏 3A
// ---------------------------------------------------------------------------

/// 长目标在窄/标准宽度下必须自动换行（旧实现从不折行 ⇒ 右侧硬截断）。
#[test]
fn goal_objective_wraps_instead_of_being_truncated() {
    let mut state = drawer_state();
    state.task_center.as_mut().expect("抽屉已打开").tab = task_center::TaskCenterTab::Goal;
    let objective = "把底部任务抽屉的目标正文完整显示出来，不能因为宽度不足而在右侧被硬截断掉后半段内容";
    state.set_goal(TuiGoal {
        id: "goal-wrap".to_string(),
        objective: objective.to_string(),
        phase: GoalPhase::Active,
        activation: GoalActivation::Armed,
        round_count: 1,
        max_rounds: Some(5),
        revision: 1,
    });

    for width in [30usize, 60, 100] {
        let lines = task_center::build_lines(&state, width);
        let joined: String = lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("");
        let joined: String = joined.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            joined.contains(&objective.chars().filter(|c| !c.is_whitespace()).collect::<String>()),
            "宽度 {width} 下目标正文必须完整可读（折行而非截断）：{joined}"
        );
        // 折行口径：任何渲染行都不得超过抽屉宽度。
        for line in &lines {
            let w: usize = line.spans.iter().map(|span| str_width(&span.content)).sum();
            assert!(w <= width, "宽度 {width} 下行宽 {w} 越界：{line:?}");
        }
    }
}

/// Goal 页签：阶段与轮次收拢在一行弱化状态里，命令提示在最末行（不与正文抢中心）。
#[test]
fn goal_tab_status_and_hint_are_demoted() {
    let mut state = drawer_state();
    state.task_center.as_mut().expect("抽屉已打开").tab = task_center::TaskCenterTab::Goal;
    state.set_goal(TuiGoal {
        id: "goal-hierarchy".to_string(),
        objective: "主体目标".to_string(),
        phase: GoalPhase::Active,
        activation: GoalActivation::Armed,
        round_count: 2,
        max_rounds: Some(8),
        revision: 1,
    });
    let lines = task_center::content_lines(&state);
    assert_eq!(lines[0].0.trim(), "主体目标", "目标正文是首行主体：{lines:?}");
    let status_index = lines
        .iter()
        .position(|(text, _)| text.contains("进行中") && text.contains("2/8 轮"))
        .expect("阶段与轮次必须在同一行");
    let hint_index = lines
        .iter()
        .position(|(text, _)| text.contains("/goal pause"))
        .expect("命令提示必须存在");
    assert!(hint_index > status_index, "命令提示必须弱化地排在状态之后：{lines:?}");
    assert_eq!(hint_index, lines.len() - 1, "命令提示必须是最后一行：{lines:?}");
}

/// Plan 页签：展示**模式状态与方案正文**（Plan 不再是步骤清单）。
#[test]
fn plan_tab_shows_mode_state_and_plan_body() {
    let mut state = drawer_state();
    state.task_center.as_mut().expect("抽屉已打开").tab = task_center::TaskCenterTab::Plan;
    state.update_plan(PlanModeState {
        active: true,
        plan: Some("# 方案\n- 第一步".to_string()),
        plan_approved: false,
        revision: 3,
    });

    let joined = task_center::content_lines(&state)
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("规划中"),
        "激活且未批准时必须显示「规划中」: {joined}"
    );
    assert!(joined.contains("# 方案"), "抽屉必须展示方案正文: {joined}");
    assert!(
        joined.contains("/approve"),
        "待批状态必须给出 /approve 入口: {joined}"
    );
}

/// 底栏 3A：完成态 Goal 从常驻左槽隐藏（抽屉内仍可查看）。
#[test]
fn completed_goal_is_hidden_from_status_bar() {
    let mut state = TuiState::default();
    state.set_goal(TuiGoal {
        id: "goal-done".to_string(),
        objective: "已完成的目标".to_string(),
        phase: GoalPhase::Completed,
        activation: GoalActivation::Armed,
        round_count: 5,
        max_rounds: Some(5),
        revision: 2,
    });
    assert!(
        task_summary(&state).is_none(),
        "完成态目标必须从底栏消失：{:?}",
        task_summary(&state)
    );

    // 抽屉 Goal 页签仍能看到它。
    let mut drawer = state;
    drawer.task_center = Some(task_center::TaskCenterState::new());
    let joined = task_center::content_lines(&drawer)
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("已完成的目标"), "抽屉内仍可回看目标：{joined}");
}

/// 底栏 3A：Subagent 运行态计入 `⚡ 智能体`。
#[test]
fn running_subagent_counts_toward_agents_segment() {
    let mut state = TuiState::default();
    state.subagent_runs = vec![SubagentViewRow {
        session_id: "subagent-1".to_string(),
        description: "跑测试".to_string(),
        status: pony_agent_core::agent::orchestration::SubagentRunStatus::Running,
        started_at_ms: 0,
        error: None,
    }];
    state.task_glance.subagents_running = 1;
    let summary = task_summary(&state).expect("有运行中子代理时智能体段必须出现");
    assert!(summary.contains("⚡ 智能体 1"), "子代理必须计入智能体段：{summary}");
}

/// 2026-11-28 回归：Ctrl+T → Goal 页签的挂起行必须是**一句自解释的话**。
///
/// 现场：用户报「`续轮暂停：等你作答｜本轮在等你作答/审批｜作答后自动继续` 完全不知道
/// 这句的设计意图」。三处判据：
/// 1. 同一事实只说一次（「等你作答」只出现一次，旧实现 headline + detail 各一遍）；
/// 2. 不外泄实现词「续轮」（用户侧无此概念）；
/// 3. 仍须表达「暂停 ≠ 终止」+ 解锁动作（`等你作答` + `作答后自动继续`），否则用户不会去作答。
#[test]
fn goal_tab_suspended_row_is_self_explanatory_once() {
    use pony_agent_core::agent::goal::GoalRoundStopReason;

    let mut state = drawer_state();
    state.task_center.as_mut().expect("抽屉已打开").tab = task_center::TaskCenterTab::Goal;
    state.set_goal(TuiGoal {
        id: "goal-suspended".to_string(),
        objective: "把抽屉里的每一句都写成用户读得懂的话".to_string(),
        phase: GoalPhase::Active,
        activation: GoalActivation::Armed,
        round_count: 2,
        max_rounds: Some(5),
        revision: 1,
    });
    let reason = GoalRoundStopReason::TurnSuspended;
    state.apply_goal_rounds_stopped(reason.clone(), reason.visible_line());

    let joined = task_center::content_lines(&state)
        .into_iter()
        .map(|(text, _)| text)
        .collect::<Vec<_>>()
        .join("\n");

    assert!(
        joined.contains("等你作答/审批"),
        "挂起行必须说清在等什么（Ask 与审批共用本信号）：{joined}"
    );
    assert!(
        joined.contains("作答后自动继续"),
        "必须承诺「作答后自动继续」（否则用户以为目标已死）：{joined}"
    );
    assert_eq!(
        joined.matches("等你作答").count(),
        1,
        "「等你作答」只允许出现一次（旧实现 headline + detail 各说一遍）：{joined}"
    );
    assert!(
        !joined.contains("续轮"),
        "Goal 页签不得对用户说「续轮」这类实现词：{joined}"
    );

    // 用户作答 ⇒ 新一轮起点（`begin_turn_usage`）⇒ 该行立刻消失，不残留过期结论。
    state.begin_turn_usage();
    let after = task_center::content_lines(&state)
        .into_iter()
        .map(|(text, _)| text)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !after.contains("等你作答"),
        "作答后挂起行必须失效（否则面板在谎称还在等人）：{after}"
    );
}
