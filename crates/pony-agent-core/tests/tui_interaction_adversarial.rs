// L2-AT: 对抗破坏与极端边界注入测试 (PA-120)
// 针对 TUI 进阶交互模块执行超长文本注入、越界索引容错、空候选项穿透、高频按键与恶意输入断言

#[path = "../src/bin/tui.rs"]
mod tui;

use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;
use tui::*;

fn screen(state: &mut TuiState, width: u16, height: u16) -> Buffer {
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend).unwrap();
    terminal.draw(|f| ui(f, state)).unwrap();
    terminal.backend().buffer().clone()
}

// -----------------------------------------------------------------------------
// 1. 对抗测试：超长文本截断与缓冲区防溢出
// -----------------------------------------------------------------------------

#[test]
fn test_adversarial_super_long_payload_rendering() {
    let mut state = TuiState::default();

    // 构造超长文本 (10,000 字符)
    let huge_desc = "A".repeat(10_000);
    let huge_cmd = "B".repeat(10_000);

    let req = PermissionRequest {
        request_id: "adv-perm-long".to_string(),
        session_id: "sess-adv".to_string(),
        tool_name: "bash".to_string(),
        description: huge_desc,
        command_or_path: Some(huge_cmd),
        timestamp_ms: 12345678,
    };

    state.start_permission_approval(req);

    // 在小屏幕（30x10）与标准屏幕（80x24）下分别渲染，绝不触发 panic，且不越界写入
    let buf_small = screen(&mut state, 30, 10);
    assert_eq!(buf_small.area.width, 30);
    assert_eq!(buf_small.area.height, 10);

    let buf_std = screen(&mut state, 80, 24);
    assert_eq!(buf_std.area.width, 80);
    assert_eq!(buf_std.area.height, 24);
}

// -----------------------------------------------------------------------------
// 2. 对抗测试：AskUser 空候选项与零问题鲁棒性
// -----------------------------------------------------------------------------

#[test]
fn test_adversarial_ask_user_empty_options_and_zero_questions() {
    // 零问题场景
    let mut zero_state = AskUserState::new("adv-zero".to_string(), vec![]);
    assert!(zero_state.current_question().is_none());
    zero_state.next_option();
    zero_state.prev_option();
    zero_state.toggle_current_option();
    // 确保在零问题时按键操作静默安全，不触发 panic

    // 空选项问题场景（仅有自定义输入）
    let empty_opt_q = AskUserQuestion {
        id: "q-empty".to_string(),
        header: None,
        question: "没有任何选项的问题".to_string(),
        multi_select: false,
        options: vec![],
    };
    let mut state = AskUserState::new("adv-empty-opt".to_string(), vec![empty_opt_q]);
    assert_eq!(state.selected_option_index, 0);

    // options 为空时，全量索引为 0（即自定义输入），next/prev 必须安全停留或循环
    state.next_option();
    assert_eq!(state.selected_option_index, 0);
    assert!(state.is_custom_input_active);

    state.prev_option();
    assert_eq!(state.selected_option_index, 0);
    assert!(state.is_custom_input_active);
}

// -----------------------------------------------------------------------------
// 3. 对抗测试：大量任务列表与极限截断
// -----------------------------------------------------------------------------

#[test]
fn test_adversarial_massive_task_board_truncation() {
    let mut state = TuiState::default();

    // 注入 1,000 个任务
    let massive_tasks: Vec<TuiTask> = (0..1_000)
        .map(|i| TuiTask {
            id: format!("task-{}", i),
            subject: format!("高频并发任务项编号 #{}", i),
            status: if i % 2 == 0 {
                TuiTaskStatus::Pending
            } else {
                TuiTaskStatus::InProgress
            },
            owner: Some("subagent-stress".to_string()),
            blocked_by: vec![],
            write_scopes: vec![],
        })
        .collect();

    state.update_task_board(massive_tasks);
    state.toggle_task_board();
    assert_eq!(state.mode, InteractionMode::TaskBoard);

    // 渲染在有限高度 (80x24) 下不崩溃，且 TaskBoard 能够稳定承受
    let buf = screen(&mut state, 80, 24);
    assert_eq!(buf.area.width, 80);

    // 模拟连续按键下翻 5,000 次
    if let Some(ref mut board) = state.task_board_state {
        for _ in 0..5_000 {
            board.next_task();
        }
        assert!(board.selected_index < 1_000);

        for _ in 0..5_000 {
            board.prev_task();
        }
        assert!(board.selected_index < 1_000);
    }
}

// -----------------------------------------------------------------------------
// 4. 对抗测试：Goal 异常数值溢出与极端轮次
// -----------------------------------------------------------------------------

#[test]
fn test_adversarial_goal_overflow_and_extremes() {
    let mut state = TuiState::default();

    // 轮次超过上限（round_count 远大于 max_rounds）
    let overflow_goal = TuiGoal {
        id: "goal-overflow".to_string(),
        objective: "极限压力对抗".to_string(),
        phase: GoalPhase::Blocked,
        activation: GoalActivation::Disarmed,
        round_count: u32::MAX,
        max_rounds: Some(10),
        revision: u64::MAX,
    };

    state.set_goal(overflow_goal);

    // 渲染指标行与界面不应因数值溢出崩溃
    let buf = screen(&mut state, 100, 30);
    assert_eq!(buf.area.width, 100);

    // 在小屏幕极窄宽度 (20x10) 下全屏渲染，断言不 panic
    let buf_narrow = screen(&mut state, 20, 10);
    assert_eq!(buf_narrow.area.width, 20);
}
