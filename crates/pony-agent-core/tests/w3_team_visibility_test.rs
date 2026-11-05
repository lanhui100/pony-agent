//! W3 验证测试（task-10，green 阶段）：Team 可见性三件套。
//!
//! 覆盖：
//! 1. `/agents` 斜杠命令（`match_command_token` / `parse_command_input`，大小写不敏感）；
//! 2. Team 任务状态 → TUI 状态映射（pending/completed/blocked/in_progress/未知 ⇒ Pending）；
//! 3. `TeamTaskItem` → `TuiTask` 字段透传（id/subject/owner/blocked_by/write_scopes）；
//! 4. `AgentRosterState` 选择行为（0 / 1 / n 条目）；
//! 5. `AVAILABLE_COMMANDS` 中 `/tasks` 之后紧跟 `/agents`。
//!
//! 只读验证：不联网、不碰全局运行时/引擎、不修改生产代码。`tui.rs` 以既有
//! `#[path = "../src/bin/tui.rs"] mod tui;` 模式作为测试二进制内的模块编译。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use pony_agent_core::agent::orchestration::TeamTaskItem;

// ---------------------------------------------------------------------------
// 夹具
// ---------------------------------------------------------------------------

fn team_task(
    task_id: &str,
    subject: &str,
    status: &str,
    owner: Option<&str>,
    write_scopes: &[&str],
    blocked_by: &[&str],
) -> TeamTaskItem {
    TeamTaskItem {
        task_id: task_id.to_string(),
        subject: subject.to_string(),
        description: format!("{subject} 的详细描述"),
        status: status.to_string(),
        owner: owner.map(str::to_string),
        revision: 1,
        write_scopes: write_scopes.iter().map(|s| s.to_string()).collect(),
        blocked_by: blocked_by.iter().map(|s| s.to_string()).collect(),
        created_at_ms: 1_700_000_000_000,
    }
}

fn status_of(status_wire: &str) -> tui::TuiTaskStatus {
    let mapped = tui::tui_tasks_from_team_tasks(vec![team_task(
        "task-status-probe",
        "状态映射探针",
        status_wire,
        None,
        &[],
        &[],
    )]);
    assert_eq!(
        mapped.len(),
        1,
        "状态映射探针：输入 1 条任务必须产出 1 条 TuiTask"
    );
    mapped[0].status
}

fn roster_entry(target: &str) -> tui::AgentRosterEntry {
    tui::AgentRosterEntry {
        target: target.to_string(),
        name: target.to_string(),
        role: "worker".to_string(),
        status: "active".to_string(),
        pending_inbox: 0,
        driving: false,
        session_id: Some(format!("{target}-session")),
    }
}

// ---------------------------------------------------------------------------
// 1. `/agents` 命令解析
// ---------------------------------------------------------------------------

#[test]
fn w3_agents_command_is_registered_and_case_insensitive() {
    assert_eq!(
        tui::match_command_token("/agents"),
        Some("/agents"),
        "`/agents` 必须被首 token 口径识别"
    );
    assert_eq!(
        tui::parse_command_input("/agents"),
        Some("/agents"),
        "`/agents` 整串必须可分发"
    );

    for raw in ["/AGENTS", "  /Agents  ", "\t/AgEnTs\n"] {
        assert_eq!(
            tui::match_command_token(raw),
            Some("/agents"),
            "首 token 口径必须大小写/首尾空白不敏感：{raw:?}"
        );
        assert_eq!(
            tui::parse_command_input(raw),
            Some("/agents"),
            "分发口径必须大小写/首尾空白不敏感：{raw:?}"
        );
    }
}

#[test]
fn w3_agents_command_follows_first_token_vs_whole_string_rules() {
    // 首 token 口径：后面跟正文仍命中（供菜单/高亮使用）。
    assert_eq!(
        tui::match_command_token("/agents worker-1"),
        Some("/agents"),
        "首 token 命中即返回规范命令"
    );
    // 分发口径：整串不匹配 ⇒ None（正文不得被吞掉）。
    assert_eq!(
        tui::parse_command_input("/agents worker-1"),
        None,
        "存在额外 token 时分发口径必须返回 None"
    );
    // 非命令文本不得被识别为 `/agents`。
    assert_eq!(tui::match_command_token("agents"), None);
    assert_eq!(tui::match_command_token("/agent"), None);
}

// ---------------------------------------------------------------------------
// 2. Team 状态 → TUI 状态映射
// ---------------------------------------------------------------------------

#[test]
fn w3_team_status_maps_to_tui_task_status() {
    let cases = [
        ("pending", tui::TuiTaskStatus::Pending),
        ("in_progress", tui::TuiTaskStatus::InProgress),
        ("completed", tui::TuiTaskStatus::Completed),
        ("blocked", tui::TuiTaskStatus::Blocked),
        // 大小写不敏感（wire 字符串可能被上游大写化）。
        ("COMPLETED", tui::TuiTaskStatus::Completed),
        ("Blocked", tui::TuiTaskStatus::Blocked),
    ];

    for (wire, expected) in cases {
        assert_eq!(
            status_of(wire),
            expected,
            "Team 状态 {wire:?} 必须映射为 {expected:?}"
        );
    }
}

#[test]
fn w3_unknown_team_status_degrades_to_pending() {
    for wire in ["definitely-unknown-status", "", "  ", "weird_status_xyz"] {
        assert_eq!(
            status_of(wire),
            tui::TuiTaskStatus::Pending,
            "未知状态 {wire:?} 必须降级为 Pending（绝不吞成 Completed）"
        );
    }
}

// ---------------------------------------------------------------------------
// 3. TeamTaskItem → TuiTask 字段透传
// ---------------------------------------------------------------------------

#[test]
fn w3_team_tasks_map_fields_verbatim() {
    let mapped = tui::tui_tasks_from_team_tasks(vec![
        team_task(
            "task-10",
            "W3 验证测试",
            "in_progress",
            Some("test-agent"),
            &[
                "crates/pony-agent-core/tests/w3_team_visibility_test.rs",
                ".dev-team/contracts/w3.md",
            ],
            &["task-9", "task-6"],
        ),
        team_task("task-11", "无主任务", "pending", None, &[], &[]),
    ]);

    assert_eq!(mapped.len(), 2, "2 条输入必须产出 2 条 TuiTask");

    let first = &mapped[0];
    assert_eq!(first.id, "task-10", "id 必须原样透传");
    assert_eq!(first.subject, "W3 验证测试", "subject 必须原样透传");
    assert_eq!(first.status, tui::TuiTaskStatus::InProgress);
    assert_eq!(
        first.owner.as_deref(),
        Some("test-agent"),
        "owner 必须原样透传"
    );
    assert_eq!(
        first.blocked_by,
        vec!["task-9".to_string(), "task-6".to_string()],
        "blocked_by 必须原样透传（顺序保持）"
    );
    assert_eq!(
        first.write_scopes,
        vec![
            "crates/pony-agent-core/tests/w3_team_visibility_test.rs".to_string(),
            ".dev-team/contracts/w3.md".to_string()
        ],
        "write_scopes 必须原样透传（顺序保持）"
    );

    let second = &mapped[1];
    assert_eq!(second.id, "task-11", "顺序必须与输入一致");
    assert_eq!(second.subject, "无主任务");
    assert_eq!(second.status, tui::TuiTaskStatus::Pending);
    assert_eq!(second.owner, None, "无 owner 的任务必须保持 None");
    assert!(second.blocked_by.is_empty());
    assert!(second.write_scopes.is_empty());
}

#[test]
fn w3_empty_team_task_list_maps_to_empty_board() {
    let mapped = tui::tui_tasks_from_team_tasks(Vec::new());
    assert!(
        mapped.is_empty(),
        "空 Team 任务列表必须产出空看板（面板据此走空状态）"
    );
}

// ---------------------------------------------------------------------------
// 4. AgentRosterState 选择行为（0 / 1 / n）
// ---------------------------------------------------------------------------

#[test]
fn w3_roster_with_zero_entries_has_no_selection() {
    let mut state = tui::AgentRosterState::new(Vec::new());

    assert_eq!(state.selected, 0, "空表时 selected 必须为 0");
    assert!(
        state.selected_entry().is_none(),
        "空表时 selected_entry 必须为 None"
    );

    state.next();
    state.next();
    assert_eq!(state.selected, 0, "空表 next 不得越界/panic");
    assert!(state.selected_entry().is_none());

    state.prev();
    assert_eq!(state.selected, 0, "空表 prev 不得下溢/panic");
    assert!(state.selected_entry().is_none());
}

#[test]
fn w3_roster_with_single_entry_stays_at_zero() {
    let mut state = tui::AgentRosterState::new(vec![roster_entry("solo")]);

    assert_eq!(state.selected, 0);
    assert_eq!(
        state.selected_entry().map(|entry| entry.target.as_str()),
        Some("solo")
    );

    state.next();
    assert_eq!(state.selected, 0, "单条时 next 不得移动（不 wrap）");
    state.prev();
    assert_eq!(state.selected, 0, "单条时 prev 不得下溢");
    assert_eq!(
        state.selected_entry().map(|entry| entry.target.as_str()),
        Some("solo")
    );
}

#[test]
fn w3_roster_with_many_entries_clamps_without_wrapping() {
    let mut state = tui::AgentRosterState::new(vec![
        roster_entry("lead"),
        roster_entry("tester"),
        roster_entry("reviewer"),
    ]);

    assert_eq!(state.selected, 0);
    assert_eq!(
        state.selected_entry().map(|entry| entry.target.as_str()),
        Some("lead")
    );

    state.next();
    assert_eq!(state.selected, 1);
    assert_eq!(
        state.selected_entry().map(|entry| entry.target.as_str()),
        Some("tester")
    );

    state.next();
    assert_eq!(state.selected, 2);
    assert_eq!(
        state.selected_entry().map(|entry| entry.target.as_str()),
        Some("reviewer")
    );

    // 末条继续 next：夹紧在 len-1，不 wrap 回头部。
    state.next();
    assert_eq!(state.selected, 2, "末条 next 必须夹紧，不得 wrap");
    assert_eq!(
        state.selected_entry().map(|entry| entry.target.as_str()),
        Some("reviewer")
    );

    state.prev();
    assert_eq!(state.selected, 1);
    assert_eq!(
        state.selected_entry().map(|entry| entry.target.as_str()),
        Some("tester")
    );

    state.prev();
    assert_eq!(state.selected, 0);

    // 首条继续 prev：夹紧在 0，不下溢。
    state.prev();
    assert_eq!(state.selected, 0, "首条 prev 必须夹紧，不得下溢");
    assert_eq!(
        state.selected_entry().map(|entry| entry.target.as_str()),
        Some("lead")
    );
}

// ---------------------------------------------------------------------------
// 5. 命令表：/tasks 之后插入 /agents
// ---------------------------------------------------------------------------

#[test]
fn w3_available_commands_place_agents_right_after_tasks() {
    let commands: Vec<&str> = tui::AVAILABLE_COMMANDS.iter().map(|(cmd, _)| *cmd).collect();

    let tasks_index = commands
        .iter()
        .position(|cmd| *cmd == "/tasks")
        .unwrap_or_else(|| panic!("AVAILABLE_COMMANDS 必须仍包含 `/tasks`，实际 = {commands:?}"));
    let agents_index = commands
        .iter()
        .position(|cmd| *cmd == "/agents")
        .unwrap_or_else(|| panic!("AVAILABLE_COMMANDS 必须包含 `/agents`，实际 = {commands:?}"));

    assert_eq!(
        agents_index,
        tasks_index + 1,
        "`/agents` 必须紧随 `/tasks`（实际 /tasks=#{tasks_index}, /agents=#{agents_index}）"
    );
    assert_eq!(
        commands.iter().filter(|cmd| **cmd == "/agents").count(),
        1,
        "`/agents` 在命令表中必须只出现一次"
    );
    assert_eq!(
        commands.iter().filter(|cmd| **cmd == "/tasks").count(),
        1,
        "`/tasks` 在命令表中必须只出现一次（不得被新命令挤掉）"
    );
}
