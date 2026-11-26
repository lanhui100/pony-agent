//! Ctrl+T 任务抽屉的**数据接线**（2026-11-22 批次 B）：
//! Plan / Tasks / Goal 三个页签的投影必须读到真实、会话作用域的数据源。
//!
//! 覆盖的根因（旧实现「从没显示过」）：
//! 1. Plan：只有 `/plan` / `/approve` 会投影计划，且投影固定取 `plan_list().first()`
//!    ⇒ 会话产生第二份计划后永远显示最早那份；
//! 2. Tasks：抽屉读的是进程全局 Team 看板，单会话 TUI 下几乎恒空（会话 todo 从未接线）；
//! 3. Goal：没有打开抽屉时的同步点，只有轮末事件回写。

use super::*;
use pony_agent_core::agent::todo::{todo_write, TodoWriteArgs};

/// 会话作用域的 todo 写入（真实存储，不做 mock；session 用测试专属名避免串扰）。
fn write_todos(session: &str, items: Vec<(&str, StoreTodoStatus)>) {
    let todos = items
        .into_iter()
        .map(|(content, status)| TodoItem::new(content, status))
        .collect();
    todo_write(TodoWriteArgs {
        todos,
        session_id: Some(session.to_string()),
    })
    .expect("todo_write 必须成功");
}

/// 计划 JSON（`plan_list` 的真实 wire 形状：camelCase 字段 + snake_case lifecycle）。
fn plan_json(plan_id: &str, summary: &str, lifecycle: &str) -> serde_json::Value {
    serde_json::json!({
        "planId": plan_id,
        "sessionId": "s",
        "revision": 1,
        "kind": "implementation",
        "summary": summary,
        "lifecycle": lifecycle,
        "steps": [
            { "stepId": "step-1", "name": "第一步", "summary": "", "status": "pending" }
        ]
    })
}

#[test]
fn drawer_tasks_tab_reads_session_todos_not_team_board() {
    let session = "b268-session-todos";
    write_todos(
        session,
        vec![
            ("实现抽屉数据接线", StoreTodoStatus::Completed),
            ("补测试", StoreTodoStatus::InProgress),
        ],
    );

    let mut state = TuiState::default();
    state.session_id = session.to_string();
    state.task_center = Some(task_center::TaskCenterState::new());
    state.task_center.as_mut().unwrap().tab = task_center::TaskCenterTab::Tasks;

    sync_todos_from_store(&mut state);
    let lines = task_center::content_lines(&state);
    assert_eq!(lines.len(), 3, "一行进度 + 两条待办：{lines:?}");
    assert!(lines[0].0.contains("1/2"), "进度必须是 1/2：{lines:?}");
    assert!(
        lines.iter().any(|(text, _)| text.contains("实现抽屉数据接线")),
        "必须显示本会话 todo 正文：{lines:?}"
    );

    // 反例：别的会话没有 todo ⇒ 抽屉回到一行空态（会话隔离，不串话）。
    let mut other = TuiState::default();
    other.session_id = "b268-session-todos-other".to_string();
    sync_todos_from_store(&mut other);
    assert!(other.todos_state.is_none(), "空会话不得有 todo 快照");
}

#[test]
fn tasks_tab_merges_session_todos_and_team_tasks() {
    // 需求：Tasks 页签同时承载主 Agent 会话 Todo 与 Agent Team 团队 Tasks，
    // 两者皆有值时分成两段（各自带进度），只有一方时直接列表（不加冗余分组头）。
    let mut state = TuiState::default();
    state.task_center = Some(task_center::TaskCenterState::new());
    state.task_center.as_mut().unwrap().tab = task_center::TaskCenterTab::Tasks;
    state.todos_state = Some(vec![TuiTask {
        id: "todo-1".to_string(),
        subject: "本会话待办".to_string(),
        status: TuiTaskStatus::InProgress,
        owner: None,
        blocked_by: Vec::new(),
        write_scopes: Vec::new(),
    }]);
    state.update_task_board(vec![TuiTask {
        id: "team-1".to_string(),
        subject: "团队任务".to_string(),
        status: TuiTaskStatus::Pending,
        owner: None,
        blocked_by: Vec::new(),
        write_scopes: Vec::new(),
    }]);
    let lines = task_center::content_lines(&state);
    let joined = lines
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("本会话待办 0/1"), "必须有会话待办分区：{joined}");
    assert!(joined.contains("团队任务 0/1"), "必须有团队任务分区：{joined}");
    assert!(joined.contains("本会话待办\n") || joined.contains("本会话待办"), "{joined}");
    assert!(joined.contains("团队任务"), "{joined}");

    // 只有团队任务时：单段直接列表，不出现「本会话待办」空标题。
    let mut team_only = TuiState::default();
    team_only.task_center = Some(task_center::TaskCenterState::new());
    team_only.task_center.as_mut().unwrap().tab = task_center::TaskCenterTab::Tasks;
    team_only.update_task_board(vec![TuiTask {
        id: "team-2".to_string(),
        subject: "仅团队".to_string(),
        status: TuiTaskStatus::Pending,
        owner: None,
        blocked_by: Vec::new(),
        write_scopes: Vec::new(),
    }]);
    let lines = task_center::content_lines(&team_only);
    let joined = lines
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!joined.contains("本会话待办"), "单段不得有空分组头：{joined}");
    assert!(joined.contains("仅团队"), "{joined}");
}

#[test]
fn plan_projection_prefers_newest_unfinished_plan() {
    // 旧实现固定取 first() ⇒ 第二份（未终态）计划永远不显示。
    let plans = serde_json::Value::Array(vec![
        plan_json("plan-old", "旧计划", "completed"),
        plan_json("plan-new", "新计划", "draft"),
    ]);
    let plan = tui_plan_from_value(&plans).expect("有未终态计划必须投影成功");
    assert_eq!(plan.plan_id, "plan-new", "必须选最新一份未终态计划");
    assert_eq!(plan.summary, "新计划");
    assert!(plan.awaiting_approval, "draft 计划必须标记待批准");

    // 全终态 ⇒ 退回最新一份（可回看最近完成/中止的计划）。
    let all_terminal = serde_json::Value::Array(vec![
        plan_json("plan-a", "A", "completed"),
        plan_json("plan-b", "B", "aborted"),
    ]);
    assert_eq!(
        tui_plan_from_value(&all_terminal).map(|plan| plan.plan_id),
        Some("plan-b".to_string()),
        "全终态时必须退回最后一份"
    );

    // 空数组 / 非数组 ⇒ None（空态，不得伪造计划）。
    assert!(tui_plan_from_value(&serde_json::json!([])).is_none());
    assert!(tui_plan_from_value(&serde_json::json!({"plans": []})).is_none());
}

#[test]
fn drawer_goal_tab_renders_current_goal() {
    let mut state = TuiState::default();
    state.task_center = Some(task_center::TaskCenterState::new());
    // 抽屉默认页签就是 Goal；设置真实目标快照后必须显示目标正文与轮次。
    state.set_goal(TuiGoal {
        id: "goal-b268".to_string(),
        objective: "把抽屉的三个页签接上真实数据".to_string(),
        phase: GoalPhase::Active,
        activation: GoalActivation::Armed,
        round_count: 2,
        max_rounds: Some(5),
        revision: 3,
    });
    let lines = task_center::content_lines(&state);
    assert!(
        lines.iter().any(|(text, _)| text.contains("把抽屉的三个页签接上真实数据")),
        "Goal 页签必须显示目标正文：{lines:?}"
    );
    assert!(
        lines.iter().any(|(text, _)| text.contains("2/5 轮")),
        "Goal 页签必须显示真实轮次：{lines:?}"
    );
    // 决策：阶段/轮次收拢成一行弱化状态，不再各占一行中心位置。
    assert!(
        lines
            .iter()
            .any(|(text, _)| text.contains("进行中") && text.contains("2/5 轮")),
        "阶段与轮次必须在同一行：{lines:?}"
    );
}
