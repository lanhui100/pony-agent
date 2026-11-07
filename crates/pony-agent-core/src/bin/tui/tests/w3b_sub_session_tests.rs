//! 原 `tui.rs` 内联测试模块 `mod w3b_sub_session_tests`（Step1 机械外移，断言零改动）。

use super::*;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn roster_entry(target: &str, session: Option<&str>) -> AgentRosterEntry {
    AgentRosterEntry {
        target: target.to_string(),
        name: format!("{target}-name"),
        role: "executor".to_string(),
        status: "inactive".to_string(),
        pending_inbox: 0,
        driving: false,
        session_id: session.map(str::to_string),
    }
}

fn context(position: Option<(usize, usize)>) -> SubSessionContext {
    SubSessionContext {
        target: "worker-1".to_string(),
        name: "Ada".to_string(),
        role: "executor".to_string(),
        position,
    }
}

#[test]
fn shortcut_only_ctrl_arrows() {
    let key = |code, modifiers| KeyEvent::new(code, modifiers);
    assert_eq!(
        sub_session_shortcut(&key(KeyCode::Up, KeyModifiers::CONTROL)),
        Some(SubSessionShortcut::ReturnToMain)
    );
    assert_eq!(
        sub_session_shortcut(&key(KeyCode::Right, KeyModifiers::CONTROL)),
        Some(SubSessionShortcut::Switch(SubSessionStep::Next))
    );
    assert_eq!(
        sub_session_shortcut(&key(KeyCode::Left, KeyModifiers::CONTROL)),
        Some(SubSessionShortcut::Switch(SubSessionStep::Prev))
    );
    // 裸方向键 / Ctrl+Char / Alt+Up 一律不消费（保持原有光标与输入语义）。
    assert_eq!(sub_session_shortcut(&key(KeyCode::Left, KeyModifiers::NONE)), None);
    assert_eq!(sub_session_shortcut(&key(KeyCode::Up, KeyModifiers::NONE)), None);
    assert_eq!(
        sub_session_shortcut(&key(KeyCode::Char('a'), KeyModifiers::CONTROL)),
        None
    );
    assert_eq!(sub_session_shortcut(&key(KeyCode::Up, KeyModifiers::ALT)), None);
}

#[test]
fn candidates_drop_lead_sessionless_and_unloadable() {
    let entries = vec![
        roster_entry("lead", Some("sess-lead")),
        roster_entry("worker-1", Some("sess-1")),
        roster_entry("worker-2", None),
        roster_entry("worker-3", Some("sess-3")),
    ];
    let candidates =
        sub_session_candidates(&entries, |session_id| session_id != "sess-3");
    let targets: Vec<&str> = candidates
        .iter()
        .map(|candidate| candidate.target.as_str())
        .collect();
    assert_eq!(targets, vec!["worker-1"], "只保留真实可加载的非 lead 会话");
    assert_eq!(candidates[0].session_id, "sess-1");
    assert_eq!(candidates[0].name, "worker-1-name");
}

#[test]
fn step_wraps_and_handles_miss() {
    let entries = vec![
        roster_entry("worker-1", Some("sess-1")),
        roster_entry("worker-2", Some("sess-2")),
        roster_entry("worker-3", Some("sess-3")),
    ];
    let candidates = sub_session_candidates(&entries, |_| true);
    let target = |step, current: Option<&str>| {
        step_sub_session_target(&candidates, current, step)
            .map(|candidate| candidate.target.clone())
    };
    assert_eq!(target(SubSessionStep::Next, Some("worker-3")), Some("worker-1".into()));
    assert_eq!(target(SubSessionStep::Prev, Some("worker-1")), Some("worker-3".into()));
    assert_eq!(target(SubSessionStep::Next, Some("worker-1")), Some("worker-2".into()));
    assert_eq!(target(SubSessionStep::Next, None), Some("worker-1".into()));
    assert_eq!(target(SubSessionStep::Prev, None), Some("worker-3".into()));
    // 当前目标不在候选序列里 ⇒ Next 取首个、Prev 取末个。
    assert_eq!(target(SubSessionStep::Next, Some("gone")), Some("worker-1".into()));
    assert_eq!(target(SubSessionStep::Prev, Some("gone")), Some("worker-3".into()));
    assert!(step_sub_session_target(&[], None, SubSessionStep::Next).is_none());
}

#[test]
fn badge_text_formats_position() {
    assert_eq!(
        sub_session_badge_text(&context(Some((2, 5)))),
        "子智能体 · Ada（executor） 2/5 · Ctrl+↑ 返回主会话 · Ctrl+←/→ 切换"
    );
    assert_eq!(
        sub_session_badge_text(&context(None)),
        "子智能体 · Ada（executor） · Ctrl+↑ 返回主会话 · Ctrl+←/→ 切换"
    );
}

#[test]
fn roster_command_accepts_agents_and_teams() {
    assert!(is_agent_roster_command("/agents"));
    assert!(is_agent_roster_command("  /TEAMS "));
    assert!(!is_agent_roster_command("/tasks"));
    assert!(!is_agent_roster_command("/agents worker-1"));
}

#[test]
fn badge_render_survives_narrow_terminal() {
    for width in [1u16, 2, 3, 8, 40, 120] {
        let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, 1))
            .expect("TestBackend 应可创建");
        let ctx = context(Some((1, 2)));
        terminal
            .draw(|f| {
                let area = f.area();
                render_sub_session_badge(f, area, &ctx);
            })
            .expect("窄终端徽标渲染不得失败");
    }
}

fn render_ui(state: &mut TuiState, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height))
        .expect("TestBackend 应可创建");
    terminal.draw(|f| ui(f, state)).expect("ui 渲染不得失败");
    terminal.backend().buffer().clone()
}

/// 行内文本匹配：宽字符的右侧占位 cell 是空白，故与断言串比较前先去空白。
fn find_row(buf: &ratatui::buffer::Buffer, needle: &str) -> Option<u16> {
    (0..buf.area.height).find(|y| {
        let text: String = (0..buf.area.width)
            .flat_map(|x| buf[(x, *y)].symbol().chars())
            .filter(|c| !c.is_whitespace())
            .collect();
        text.contains(needle)
    })
}

#[test]
fn badge_row_appears_only_in_sub_session() {
    let mut main_state = TuiState::default();
    let main = render_ui(&mut main_state, 80, 24);
    assert!(
        find_row(&main, "子智能体").is_none(),
        "主会话不得出现子会话徽标（也不得留出空行）"
    );

    let mut sub_state = TuiState::default();
    sub_state.sub_session = Some(context(Some((1, 2))));
    let sub = render_ui(&mut sub_state, 80, 24);
    let row = find_row(&sub, "子智能体").expect("子会话必须渲染身份徽标");
    assert_eq!(
        row,
        sub.area.height - 2,
        "徽标必须紧贴状态行上方（输入区正下方 1 行）"
    );
    assert!(find_row(&sub, "◆").is_some(), "徽标必须有 `◆ ` 标记");
}

/// 任务看板只展示状态符号与标题：内部 uuid 不得上屏，状态不用 `[x]` 方括号形态。
#[test]
fn task_board_hides_uuid_and_uses_status_marks() {
    let mut state = TuiState::default();
    state.mode = InteractionMode::TaskBoard;
    state.task_board_state = Some(TaskBoardState::new(vec![
        TuiTask {
            id: "task-0000-1111-2222".into(),
            subject: "已完成项".into(),
            status: TuiTaskStatus::Completed,
            owner: Some("worker-1".into()),
            blocked_by: vec![],
            write_scopes: vec![],
        },
        TuiTask {
            id: "task-3333-4444-5555".into(),
            subject: "待办项".into(),
            status: TuiTaskStatus::Pending,
            owner: Some("worker-2".into()),
            blocked_by: vec!["task-0000-1111-2222".into()],
            write_scopes: vec![],
        },
    ]));

    let buf = render_ui(&mut state, 100, 30);
    assert!(
        find_row(&buf, "task-").is_none(),
        "任务看板不得渲染内部 task_id/uuid"
    );
    assert!(find_row(&buf, "✔").is_some(), "Completed 必须用 `✔` 标记");
    assert!(find_row(&buf, "○").is_some(), "Pending 必须用 `○` 标记");
    assert!(
        find_row(&buf, "[✔]").is_none(),
        "状态标记不得保留 `[ ]` 方括号形态"
    );
    assert!(
        find_row(&buf, "worker-1").is_some(),
        "@owner 右列必须保留"
    );
    assert!(find_row(&buf, "⛔1").is_some(), "blocked_by 计数后缀必须保留");
    assert!(find_row(&buf, "已完成项").is_some(), "标题必须保留");
}
