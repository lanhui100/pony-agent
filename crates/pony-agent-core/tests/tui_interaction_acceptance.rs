// L2-T: TUI 交互模块验收测试 (PA-120)
// 覆盖：权限审批(Permission)、AskUser问答、Plan模式与交互、Task看板、Goal展示

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

fn disp_width(ch: char) -> usize {
    let cp = ch as u32;
    let wide = matches!(cp,
        0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xA960..=0xA97F
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x3FFFD);
    if wide { 2 } else { 1 }
}

fn row_text(buf: &Buffer, y: u16) -> String {
    let mut out = String::new();
    let mut skip_placeholder = false;
    for x in 0..buf.area.width {
        if skip_placeholder {
            skip_placeholder = false;
            continue;
        }
        let sym = buf[(x, y)].symbol();
        out.push_str(sym);
        let mut chars = sym.chars();
        if let (Some(ch), None) = (chars.next(), chars.next()) {
            if disp_width(ch) == 2 {
                skip_placeholder = true;
            }
        }
    }
    out
}

fn buffer_to_string(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

// -----------------------------------------------------------------------------
// 1. 权限审批测试 (Permission Dialog & Decision Flow)
// -----------------------------------------------------------------------------

#[test]
fn test_permission_state_machine_and_choices() {
    let req = PermissionRequest {
        request_id: "perm-001".to_string(),
        session_id: "sess-1".to_string(),
        tool_name: "bash".to_string(),
        description: "Execute cargo test".to_string(),
        command_or_path: Some("cargo test --workspace".to_string()),
        timestamp_ms: 1000,
    };

    let mut perm_state = PermissionState::new(req.clone());
    assert_eq!(perm_state.selected_choice(), PermissionChoice::AllowOnce);

    perm_state.next_choice();
    assert_eq!(perm_state.selected_choice(), PermissionChoice::AllowSession);

    perm_state.next_choice();
    assert_eq!(perm_state.selected_choice(), PermissionChoice::Deny);

    // 循环切换
    perm_state.next_choice();
    assert_eq!(perm_state.selected_choice(), PermissionChoice::AllowOnce);

    perm_state.prev_choice();
    assert_eq!(perm_state.selected_choice(), PermissionChoice::Deny);
}

#[test]
fn test_permission_dialog_rendering_contract() {
    let mut state = TuiState::default();
    let req = PermissionRequest {
        request_id: "perm-002".to_string(),
        session_id: "sess-1".to_string(),
        tool_name: "danger_tool".to_string(),
        description: "Delete obsolete temp files".to_string(),
        command_or_path: Some("rm -rf /tmp/test-trash".to_string()),
        timestamp_ms: 1000,
    };
    state.start_permission_approval(req);
    assert_eq!(state.mode, InteractionMode::PermissionApproval);

    let buf = screen(&mut state, 100, 30);
    let rendered = buffer_to_string(&buf);

    // 契约断言：弹窗必须渲染出具体工具名、命令或路径、以及审批选项
    assert!(
        rendered.contains("danger_tool"),
        "渲染中必须包含请求的 tool_name, 实际渲染内容:\n{}",
        rendered
    );
    assert!(
        rendered.contains("rm -rf /tmp/test-trash"),
        "渲染中必须包含请求的具体命令/路径, 实际渲染内容:\n{}",
        rendered
    );
    assert!(
        rendered.contains("AllowOnce") || rendered.contains("仅本次允许") || rendered.contains("允许一次"),
        "渲染中必须包含允许选项, 实际渲染内容:\n{}",
        rendered
    );
}

#[test]
fn test_permission_keys_and_transitions() {
    let mut state = TuiState::default();
    let req = PermissionRequest {
        request_id: "perm-003".to_string(),
        session_id: "sess-1".to_string(),
        tool_name: "write_file".to_string(),
        description: "Overwrite critical config".to_string(),
        command_or_path: Some("src/main.rs".to_string()),
        timestamp_ms: 2000,
    };
    state.start_permission_approval(req);

    // 状态初始化校验
    assert_eq!(state.mode, InteractionMode::PermissionApproval);
    assert!(state.permission_state.is_some());

    // 取消审批流转
    state.cancel_permission_approval();
    assert_eq!(state.mode, InteractionMode::Normal);
    assert!(state.permission_state.is_none());
}

// -----------------------------------------------------------------------------
// 2. AskUser 问答交互测试 (AskUser Question & Options)
// -----------------------------------------------------------------------------

#[test]
fn test_ask_user_state_navigation_and_selection() {
    let q1 = AskUserQuestion {
        id: "q-1".to_string(),
        header: Some("Framework Selection".to_string()),
        question: "Select target frontend frameworks:".to_string(),
        multi_select: true,
        options: vec![
            QuestionOption {
                label: "React".to_string(),
                description: Some("Meta's UI library".to_string()),
            },
            QuestionOption {
                label: "Vue".to_string(),
                description: Some("Progressive framework".to_string()),
            },
            QuestionOption {
                label: "Svelte".to_string(),
                description: Some("Cybernetically enhanced".to_string()),
            },
        ],
    };

    let mut state = AskUserState::new("req-ask-1".to_string(), vec![q1]);
    assert_eq!(state.selected_option_index, 0);
    assert!(!state.is_custom_input_active);

    // 选中 React
    state.toggle_current_option();
    assert_eq!(state.multi_selected_indices, vec![0]);

    // 下移到 Vue
    state.next_option();
    assert_eq!(state.selected_option_index, 1);
    assert!(!state.is_custom_input_active);

    // 选中 Vue
    state.toggle_current_option();
    assert_eq!(state.multi_selected_indices, vec![0, 1]);

    // 反选 React: 回到 0 并再次 toggle
    state.prev_option();
    assert_eq!(state.selected_option_index, 0);
    state.toggle_current_option();
    assert_eq!(state.multi_selected_indices, vec![1]);

    // 移动到自定义输入项（options.len() == 3）
    state.next_option(); // index 1
    state.next_option(); // index 2
    state.next_option(); // index 3 = 自定义输入
    assert_eq!(state.selected_option_index, 3);
    assert!(state.is_custom_input_active);
}

#[test]
fn test_ask_user_dialog_rendering_contract() {
    let mut state = TuiState::default();
    let q = AskUserQuestion {
        id: "target_arch".to_string(),
        header: Some("架构确认".to_string()),
        question: "请选择本次交付的执行架构：".to_string(),
        multi_select: false,
        options: vec![
            QuestionOption {
                label: "单体架构".to_string(),
                description: Some("快速轻量部署".to_string()),
            },
            QuestionOption {
                label: "微服务架构".to_string(),
                description: Some("高并发可扩展".to_string()),
            },
        ],
    };
    state.start_ask_user("req-ask-2".to_string(), vec![q]);

    let buf = screen(&mut state, 100, 30);
    let rendered = buffer_to_string(&buf);

    assert!(
        rendered.contains("请选择本次交付的执行架构："),
        "渲染内容必须包含问题正文, 实际渲染内容:\n{}",
        rendered
    );
    assert!(
        rendered.contains("单体架构") && rendered.contains("微服务架构"),
        "渲染内容必须包含所有候选项, 实际渲染内容:\n{}",
        rendered
    );
}

#[test]
fn test_ask_user_resume_version_resolution_logic() {
    // 模拟控制面 graph_list_ask_waits 返回的值，验证 expectedVersion 获取逻辑
    let waits_json = serde_json::json!([
        {
            "requestId": "ask-req-1",
            "expectedVersion": 2,
            "runId": "run-101"
        },
        {
            "requestId": "ask-req-2",
            "expectedVersion": 5,
            "runId": "run-101"
        }
    ]);

    let handle_req_id = "ask-req-1";
    let fallback_version = 1u64;

    let resume_version = waits_json
        .as_array()
        .and_then(|waits| {
            waits.iter().find(|w| {
                w.get("requestId").and_then(|r| r.as_str()) == Some(handle_req_id)
            })
        })
        .and_then(|w| w.get("expectedVersion").and_then(|v| v.as_u64()))
        .unwrap_or(fallback_version);

    assert_eq!(resume_version, 2, "必须成功从 ask_waits 匹配得到 expectedVersion");
}

// -----------------------------------------------------------------------------
// 3. Plan 交互与展示测试 (Plan Steps & /plan Mode)
// -----------------------------------------------------------------------------

#[test]
fn test_plan_state_step_transitions() {
    let mut plan = PlanState::new("plan-101".to_string(), "重构核心通信总线".to_string());
    plan.steps = vec![
        PlanStep {
            step_id: "s1".to_string(),
            name: "设计契约".to_string(),
            summary: "定义 protobuf 与状态接口".to_string(),
            status: PlanStepStatus::Pending,
        },
        PlanStep {
            step_id: "s2".to_string(),
            name: "编写红相测试".to_string(),
            summary: "编写端到端验收用例".to_string(),
            status: PlanStepStatus::Pending,
        },
    ];

    assert_eq!(plan.steps[0].status, PlanStepStatus::Pending);

    // 状态推进
    plan.steps[0].status = PlanStepStatus::InProgress;
    assert_eq!(plan.steps[0].status, PlanStepStatus::InProgress);

    plan.steps[0].status = PlanStepStatus::Completed;
    assert_eq!(plan.steps[0].status, PlanStepStatus::Completed);
}

#[test]
fn test_plan_mode_toggle_and_rendering_contract() {
    let mut state = TuiState::default();
    let mut plan = PlanState::new("plan-102".to_string(), "实施 TUI 全屏增强".to_string());
    plan.steps.push(PlanStep {
        step_id: "step-1".to_string(),
        name: "实现权限审批面板".to_string(),
        summary: "包含快捷键和状态机".to_string(),
        status: PlanStepStatus::InProgress,
    });
    state.update_plan(plan);

    // 触发 /plan 模式切换
    state.toggle_plan_mode();
    assert_eq!(state.mode, InteractionMode::PlanMode);
    assert!(state.plan_state.as_ref().unwrap().is_active_plan_mode);

    let buf = screen(&mut state, 100, 30);
    let rendered = buffer_to_string(&buf);

    assert!(
        rendered.contains("实施 TUI 全屏增强"),
        "Plan 视图必须渲染计划总纲, 实际渲染内容:\n{}",
        rendered
    );
    assert!(
        rendered.contains("实现权限审批面板"),
        "Plan 视图必须渲染各步骤名称, 实际渲染内容:\n{}",
        rendered
    );

    // 再次切换退出
    state.toggle_plan_mode();
    assert_eq!(state.mode, InteractionMode::Normal);
    assert!(!state.plan_state.as_ref().unwrap().is_active_plan_mode);
}

// -----------------------------------------------------------------------------
// 4. Task 看板展示测试 (Task Board & Counts)
// -----------------------------------------------------------------------------

#[test]
fn test_task_board_summary_counts_and_rendering_contract() {
    let mut state = TuiState::default();
    let tasks = vec![
        TuiTask {
            id: "task-1".to_string(),
            subject: "制定数据结构契约".to_string(),
            status: TuiTaskStatus::Completed,
            owner: Some("protocol-agent".to_string()),
            blocked_by: vec![],
            write_scopes: vec!["crates/pony-agent-core".to_string()],
        },
        TuiTask {
            id: "task-2".to_string(),
            subject: "编写端到端验收测试".to_string(),
            status: TuiTaskStatus::InProgress,
            owner: Some("test-agent".to_string()),
            blocked_by: vec!["task-1".to_string()],
            write_scopes: vec!["crates/pony-agent-core/tests".to_string()],
        },
        TuiTask {
            id: "task-3".to_string(),
            subject: "实现组件代码".to_string(),
            status: TuiTaskStatus::Pending,
            owner: Some("executor".to_string()),
            blocked_by: vec!["task-2".to_string()],
            write_scopes: vec![],
        },
    ];

    state.update_task_board(tasks);
    state.toggle_task_board();
    assert_eq!(state.mode, InteractionMode::TaskBoard);

    let buf = screen(&mut state, 100, 30);
    let rendered = buffer_to_string(&buf);

    assert!(
        rendered.contains("制定数据结构契约"),
        "看板渲染必须包含具体任务标题, 实际内容:\n{}",
        rendered
    );
    assert!(
        rendered.contains("test-agent") || rendered.contains("executor"),
        "看板渲染必须包含任务负责人信息, 实际内容:\n{}",
        rendered
    );
    // 汇总计数断言（待实现的看板头部应展示 1 完成 / 1 进行中 / 1 待处理）
    assert!(
        rendered.contains("已完成: 1") || rendered.contains("完成 1") || rendered.contains("1 completed") || rendered.contains("1/3"),
        "看板必须渲染任务汇总计数或进度, 实际内容:\n{}",
        rendered
    );
}

// -----------------------------------------------------------------------------
// 5. Goal 目标交互与展示测试 (Goal Metrics & Activation)
// -----------------------------------------------------------------------------

#[test]
fn test_goal_activation_and_state_transitions() {
    let mut goal = TuiGoal::new(
        "goal-001".to_string(),
        "完成 L3 全防护工程交付".to_string(),
        Some(10),
    );
    assert_eq!(goal.phase, GoalPhase::Active);
    assert_eq!(goal.activation, GoalActivation::Armed);
    assert_eq!(goal.round_count, 0);

    // 推进轮次
    goal.round_count += 1;
    assert_eq!(goal.round_count, 1);

    // 完成目标
    goal.phase = GoalPhase::Completed;
    assert_eq!(goal.phase, GoalPhase::Completed);
}

#[test]
fn test_goal_metrics_line_rendering_contract() {
    let mut state = TuiState::default();
    let goal = TuiGoal {
        id: "goal-002".to_string(),
        objective: "自动化重构验证".to_string(),
        phase: GoalPhase::Active,
        activation: GoalActivation::Armed,
        round_count: 3,
        max_rounds: Some(8),
        revision: 2,
    };
    state.set_goal(goal);

    let buf = screen(&mut state, 100, 30);
    let rendered = buffer_to_string(&buf);

    // 契约：当存在活跃 Goal 时，底部状态行或指标行必须展示 Goal 进度 (例如 [3/8] 或 目标: 3/8轮)
    assert!(
        rendered.contains("3/8") || rendered.contains("自动化重构验证") || rendered.contains("Goal"),
        "界面必须展示当前 Goal 的轮次指标或目标信息, 实际内容:\n{}",
        rendered
    );
}
