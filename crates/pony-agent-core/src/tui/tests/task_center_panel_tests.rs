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

/// 构造一条后台任务视图行（命令/pid/来源都用固定值，断言只看措辞与计数）。
fn job_row(id: &str, status: jobs_panel::JobStatus) -> jobs_panel::JobViewRow {
    jobs_panel::JobViewRow {
        id: id.to_string(),
        command: "sleep 1".to_string(),
        status,
        pid: 4242,
        kind: "owned".to_string(),
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
fn status_bar_uses_background_task_wording_without_warning_glyph() {
    let mut state = TuiState::default();
    state.jobs_state = Some(jobs_panel::JobsViewState {
        jobs: vec![
            job_row("job-running", jobs_panel::JobStatus::Running),
            job_row("job-failed", jobs_panel::JobStatus::Failed),
        ],
    });
    let summary = task_summary(&state).expect("有在跑/失败作业时后台任务段必须出现");
    assert!(
        summary.contains("后台任务 1·失败 1"),
        "状态栏应报「后台任务 运行中数·失败数」：{summary}"
    );
    assert!(
        !summary.contains('⚠'),
        "计数旁不得再出现警告图标：{summary}"
    );
    assert!(
        !summary.contains("作业"),
        "旧措辞「作业」不得再出现在状态栏：{summary}"
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
