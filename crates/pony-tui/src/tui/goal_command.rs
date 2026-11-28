//! 契约 C-6：`/goal` 斜杠命令的 TUI 接线。
//!
//! 分界线（与桌面端同口径）：**语法、状态机与回执文案的唯一真源在 Rust core**
//! （`pony_agent_core::agent::goal_command`）；本模块负责：
//! ① 从输入框原文提取参数；② 把执行结果投递到 TUI（System 消息 + 状态行 + goal 横幅）；
//! ③ 按结果驱动宿主生命周期：`create`/`resume` 自动拉起 `run_goal_rounds`，`pause`/`clear`/`complete`
//! 在运行中置位取消令牌并中止当前在飞轮次（对齐 DSH `command-goal` + `goal-round-driver`）。

use super::*;
use pony_agent_core::agent::goal::{
    goal_round_gate, GoalData, GoalRoundGateInput,
};
use pony_agent_core::agent::goal_command::{execute_goal_command, GoalCommandOutcome};

/// `/goal` 之后的参数文本；未命中 `/goal` ⇒ `None`。
///
/// 口径（C-6.2）：首 token 大小写不敏感等于 `/goal`（复用 [`match_command_token`]，
/// 与命令表同源，禁止另写一份字面量表）；返回其余文本（可为空串，代表 `show`）。
/// `/goa`、`/goalx`、`/goalx 目标` 一律 `None`——它们仍按普通消息发送。
pub fn goal_command_args(input: &str) -> Option<String> {
    if match_command_token(input) != Some("/goal") {
        return None;
    }

    let trimmed = input.trim();
    Some(
        trimmed
            .split_once(char::is_whitespace)
            .map(|(_, rest)| rest.trim().to_string())
            .unwrap_or_default(),
    )
}

/// TUI 宿主侧 `/goal` 自动驱动的前哨判定结论（纯函数可单测）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiAutoDriveDecision {
    Start,
    DeferAlreadyRunning,
    DeferPendingHostRequest,
    MissingApiKey {
        provider_name: String,
        env_var: String,
    },
    GateStopped {
        visible_line: String,
    },
}

/// 判定当前会话是否可由 `/goal create|resume` 立即自动启动续轮循环。
pub fn evaluate_tui_auto_drive(
    goal: &GoalData,
    is_streaming: bool,
    turn_in_flight: bool,
    pending_asks: usize,
    pending_approvals: usize,
    provider_missing_key: Option<(String, String)>,
) -> TuiAutoDriveDecision {
    if is_streaming || turn_in_flight {
        return TuiAutoDriveDecision::DeferAlreadyRunning;
    }
    if pending_asks > 0 || pending_approvals > 0 {
        return TuiAutoDriveDecision::DeferPendingHostRequest;
    }
    if let Some((provider_name, env_var)) = provider_missing_key {
        return TuiAutoDriveDecision::MissingApiKey {
            provider_name,
            env_var,
        };
    }
    let decision = goal_round_gate(GoalRoundGateInput {
        has_active_goal: true,
        cancelled: false,
        pending_asks: 0,
        pending_approvals: 0,
        consecutive_failures: 0,
        rounds_started: goal.rounds_started,
        max_rounds: goal.max_goal_rounds,
        consecutive_auto_continues: 0,
        max_auto_continues: u32::try_from(
            pony_agent_core::agent::planner::MAX_CONSECUTIVE_AUTO_CONTINUES,
        )
        .unwrap_or(u32::MAX),
    });
    if let Some(reason) = decision.stop_reason() {
        return TuiAutoDriveDecision::GateStopped {
            visible_line: reason.visible_line(),
        };
    }
    TuiAutoDriveDecision::Start
}

fn apply_goal_command_outcome(
    state: &mut TuiState,
    input: &str,
) -> Option<Result<GoalCommandOutcome, String>> {
    let args = goal_command_args(input)?;
    let result = execute_goal_command(&state.session_id, &args);
    match &result {
        Ok(outcome) => {
            match outcome.goal.as_ref() {
                Some(goal) => state.set_goal(tui_goal_from_goal(goal)),
                None => state.clear_goal_banner(),
            }
            state.set_status_notice(single_line_notice(&outcome.message));
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: outcome.message.clone(),
                timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
            });
        }
        Err(error) => {
            let receipt = format!("目标命令执行失败：{error}");
            state.set_status_notice(receipt.clone());
            state.messages.push(ChatMessage {
                role: MessageRole::System,
                content: receipt,
                timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
            });
        }
    }
    Some(result)
}

fn append_receipt_note(state: &mut TuiState, note: &str) {
    let notice = if let Some(last) = state.messages.last_mut() {
        if last.role == MessageRole::System {
            last.content.push('\n');
            last.content.push_str(note);
            Some(single_line_notice(&last.content))
        } else {
            None
        }
    } else {
        None
    };
    state.set_status_notice(notice.unwrap_or_else(|| single_line_notice(note)));
}

fn check_provider_api_key(state: &TuiState) -> Result<String, (String, String)> {
    let prov_id = resolve_active_provider_id(state)
        .unwrap_or_else(|| state.current_provider_id.clone());
    let registry = state.registry_store();
    let key_opt = registry.provider_api_key(Some(&prov_id));
    if !key_opt.as_deref().unwrap_or("").trim().is_empty() {
        return Ok(prov_id);
    }
    let view = registry.load_view();
    let prov_cfg = view.providers.iter().find(|p| p.id == prov_id);
    let env_name = prov_cfg
        .map(|p| p.api_key_env_var.as_str())
        .unwrap_or("API_KEY")
        .to_string();
    let prov_name = prov_cfg
        .map(|p| p.name.as_str())
        .unwrap_or(&prov_id)
        .to_string();
    Err((prov_name, env_name))
}

#[allow(clippy::too_many_arguments)]
fn spawn_goal_auto_rounds(
    state: &mut TuiState,
    prov_id: String,
    objective: &str,
    control_plane: &Arc<HostControlPlane>,
    tx: &std::sync::mpsc::Sender<UiEvent>,
    goal_round_cancel: &GoalRoundCancelToken,
    current_turn_id: &Arc<std::sync::Mutex<Option<String>>>,
    goal_round_loop_lease: &GoalRoundLoopLeaseRegistry,
    inflight_turns: &Arc<std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>>,
    turn_loop_in_flight: &TurnLoopInFlightRegistry,
) {
    state.begin_turn_usage();
    state.context_window_tokens = resolve_context_window_tokens(state);
    state.suspended_turn_context = None;
    state.pending_user_turn = None;
    state.resume_instruction = None;
    state.is_streaming = true;
    state.auto_scroll = true;
    state.status_text = format!("目标自动推进中：{}", objective.trim());
    state.transition_runtime_mode(RuntimeMode::Connecting);
    state.turn_started_at = Some(std::time::Instant::now());
    state.stopping_turn_id = None;
    state.streaming_turn_id = None;
    goal_round_cancel.reset_for(&state.session_id);
    // 上一轮的「续轮停止」留痕已由 `begin_turn_usage()`（唯一复位点）清空，此处不再重复赋值。

    let mdl_id = state.model.clone();
    let turn_input = build_turn_input(state, String::new(), prov_id, mdl_id);
    let sink = Arc::new(TuiTurnSink::new(tx.clone()));
    let turn_control_plane = Arc::clone(control_plane);
    let cancel_for_rounds = goal_round_cancel.clone();
    let current_turn_for_rounds = Arc::clone(current_turn_id);
    let lease_for_rounds = Arc::clone(goal_round_loop_lease);
    let turn_loop_in_flight_for_thread = Arc::clone(turn_loop_in_flight);
    let tx_turn = tx.clone();
    let session_id = state.session_id.clone();

    let turn_thread = std::thread::spawn(move || {
        let _retry_observer = install_tui_retry_observer(tx_turn.clone());
        let _phase_observer = install_tui_phase_observer(tx_turn.clone());
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let _guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
            let _in_flight_guard =
                TurnLoopInFlightGuard::enter(turn_loop_in_flight_for_thread, &session_id);
            let needs_streaming_reset = run_goal_rounds(
                &turn_control_plane,
                &sink,
                &turn_input,
                &session_id,
                &cancel_for_rounds,
                &current_turn_for_rounds,
                &lease_for_rounds,
            );
            if needs_streaming_reset {
                let _ = sink.tx.send(UiEvent::GoalAutoStartAborted {
                    session_id: session_id.clone(),
                });
            }
        }));
        if let Err(payload) = outcome {
            let _ = tx_turn.send(UiEvent::StreamFailed(format!(
                "目标续轮线程 panic: {}",
                panic_payload_message(payload.as_ref())
            )));
        }
    });

    inflight_turns
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .push(turn_thread);
}

/// 执行 `/goal` 并联动宿主驱动（`create`/`resume` 自动起轮，`pause`/`clear`/`complete` 中止在飞轮次）。
#[allow(clippy::too_many_arguments)]
pub fn handle_goal_command_with_driver(
    state: &mut TuiState,
    input: &str,
    control_plane: &Arc<HostControlPlane>,
    tx: &std::sync::mpsc::Sender<UiEvent>,
    goal_round_cancel: &GoalRoundCancelToken,
    current_turn_id: &Arc<std::sync::Mutex<Option<String>>>,
    goal_round_loop_lease: &GoalRoundLoopLeaseRegistry,
    inflight_turns: &Arc<std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>>,
    turn_loop_in_flight: &TurnLoopInFlightRegistry,
) -> bool {
    let Some(result) = apply_goal_command_outcome(state, input) else {
        return false;
    };
    let Ok(outcome) = result else {
        return true;
    };

    if outcome.should_cancel_in_flight() {
        goal_round_cancel.cancel_for(&state.session_id);
        let stopping_turn = {
            current_turn_id
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take()
        };
        if let Some(turn_id) = stopping_turn {
            control_plane.stop_turn(StopTurnCommand {
                turn_id: turn_id.clone(),
                stop_reason: Some(StopReason::User),
            });
            state.apply_stop_requested(&turn_id);
        }
        return true;
    }

    if outcome.should_auto_drive() {
        let Some(goal) = outcome.goal.as_ref() else {
            return true;
        };
        let turn_in_flight = turn_loop_in_flight
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains(&state.session_id);
        let (pending_asks, pending_approvals) =
            session_pending_host_request_count(control_plane, &state.session_id);
        let key_check = check_provider_api_key(state);
        let missing_key = key_check.as_ref().err().cloned();

        match evaluate_tui_auto_drive(
            goal,
            state.is_streaming,
            turn_in_flight,
            pending_asks,
            pending_approvals,
            missing_key,
        ) {
            TuiAutoDriveDecision::Start => {
                if let Ok(prov_id) = key_check {
                    spawn_goal_auto_rounds(
                        state,
                        prov_id,
                        &goal.objective,
                        control_plane,
                        tx,
                        goal_round_cancel,
                        current_turn_id,
                        goal_round_loop_lease,
                        inflight_turns,
                        turn_loop_in_flight,
                    );
                    append_receipt_note(state, "自动续轮已启动。");
                }
            }
            TuiAutoDriveDecision::DeferAlreadyRunning => {
                append_receipt_note(state, "目标已就绪，将在当前轮次结束后自动推进。");
            }
            TuiAutoDriveDecision::DeferPendingHostRequest => {
                append_receipt_note(
                    state,
                    "当前存在待作答的提问或审批，处理后将自动继续推进目标。",
                );
            }
            TuiAutoDriveDecision::MissingApiKey {
                provider_name,
                env_var,
            } => {
                append_receipt_note(
                    state,
                    &format!(
                        "目标已保存，但当前提供商 [{provider_name}] 未配置 API Key，无法启动自动续轮；请在 /provider 中配置或设置环境变量 [{env_var}]。"
                    ),
                );
            }
            TuiAutoDriveDecision::GateStopped { visible_line } => {
                append_receipt_note(state, &visible_line);
            }
        }
    }

    true
}

/// 状态行是**单行**通道（C-6.3）：多行回执折叠为 ` · `，既不丢内容也不撑破布局。
fn single_line_notice(message: &str) -> String {
    message
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_command_args_matches_only_exact_token() {
        assert_eq!(goal_command_args("/goal"), Some(String::new()));
        assert_eq!(goal_command_args("  /goal  "), Some(String::new()));
        assert_eq!(goal_command_args("/GOAL"), Some(String::new()));
        assert_eq!(goal_command_args("/goal pause"), Some("pause".to_string()));
        assert_eq!(
            goal_command_args("/goal edit 更小的目标  "),
            Some("edit 更小的目标".to_string())
        );
        // 正文不得被吞：首 token 命中即可带走全部剩余文本。
        assert_eq!(
            goal_command_args("/goal 完成迁移并跑通门禁"),
            Some("完成迁移并跑通门禁".to_string())
        );
    }

    #[test]
    fn goal_command_args_rejects_near_misses() {
        assert_eq!(goal_command_args("/goa"), None);
        assert_eq!(goal_command_args("/goalx"), None);
        assert_eq!(goal_command_args("/goalx 目标"), None);
        assert_eq!(goal_command_args("goal 目标"), None);
        assert_eq!(goal_command_args("/plan"), None);
        assert_eq!(goal_command_args("/clear 这个需求怎么拆"), None);
        assert_eq!(goal_command_args(""), None);
    }

    #[test]
    fn single_line_notice_folds_multiline_receipt() {
        let folded = single_line_notice("目标状态: 进行中\n目标: 迁移\n\n轮次: 0/30");
        assert_eq!(folded, "目标状态: 进行中 · 目标: 迁移 · 轮次: 0/30");
        assert!(!folded.contains('\n'));
    }

    fn sample_goal(rounds_started: u32, max_goal_rounds: u32) -> GoalData {
        GoalData {
            id: "goal-tui-test".to_string(),
            session_id: "sess-tui-test".to_string(),
            objective: "完成修复".to_string(),
            phase: "active".to_string(),
            activation: pony_agent_core::agent::goal::GoalActivation::Armed,
            revision: 1,
            rounds_started,
            max_goal_rounds,
            blocked_reason: None,
        }
    }

    #[test]
    fn evaluate_tui_auto_drive_covers_all_gates() {
        let goal = sample_goal(0, 10);
        assert_eq!(
            evaluate_tui_auto_drive(&goal, false, false, 0, 0, None),
            TuiAutoDriveDecision::Start
        );
        assert_eq!(
            evaluate_tui_auto_drive(&goal, true, false, 0, 0, None),
            TuiAutoDriveDecision::DeferAlreadyRunning
        );
        assert_eq!(
            evaluate_tui_auto_drive(&goal, false, true, 0, 0, None),
            TuiAutoDriveDecision::DeferAlreadyRunning
        );
        assert_eq!(
            evaluate_tui_auto_drive(&goal, false, false, 1, 0, None),
            TuiAutoDriveDecision::DeferPendingHostRequest
        );
        assert_eq!(
            evaluate_tui_auto_drive(&goal, false, false, 0, 1, None),
            TuiAutoDriveDecision::DeferPendingHostRequest
        );
        assert_eq!(
            evaluate_tui_auto_drive(
                &goal,
                false,
                false,
                0,
                0,
                Some(("Pony".to_string(), "PONY_KEY".to_string())),
            ),
            TuiAutoDriveDecision::MissingApiKey {
                provider_name: "Pony".to_string(),
                env_var: "PONY_KEY".to_string(),
            }
        );
        let exhausted = sample_goal(10, 10);
        assert!(matches!(
            evaluate_tui_auto_drive(&exhausted, false, false, 0, 0, None),
            TuiAutoDriveDecision::GateStopped { .. }
        ));
    }

    #[test]
    fn handle_goal_command_with_driver_defers_when_streaming_and_cancels_on_pause() {
        let _lock = pony_agent_core::agent::goal::test_support::lock();
        let _store = pony_agent_core::agent::goal::test_support::TempGoalStore::install("tui-driver");
        let control_plane = Arc::new(HostControlPlane::new());
        let (tx, _rx) = std::sync::mpsc::channel();
        let cancel = GoalRoundCancelToken::new();
        let current_turn_id = Arc::new(std::sync::Mutex::new(None));
        let loop_lease = new_goal_round_loop_lease_registry();
        let inflight_turns = Arc::new(std::sync::Mutex::new(Vec::new()));
        let turn_loop_in_flight: TurnLoopInFlightRegistry =
            Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));

        let mut state = TuiState::default();
        state.session_id = "sess-tui-driver".to_string();
        state.is_streaming = true;

        assert!(handle_goal_command_with_driver(
            &mut state,
            "/goal 运行中创建目标",
            &control_plane,
            &tx,
            &cancel,
            &current_turn_id,
            &loop_lease,
            &inflight_turns,
            &turn_loop_in_flight,
        ));
        let last_msg = state.messages.last().expect("receipt message");
        assert!(last_msg.content.contains("目标已就绪，将在当前轮次结束后自动推进。"));
        assert!(!last_msg.content.contains("自动续轮已启动"));
        assert!(!cancel.is_cancelled_for(&state.session_id));

        assert!(handle_goal_command_with_driver(
            &mut state,
            "/goal pause",
            &control_plane,
            &tx,
            &cancel,
            &current_turn_id,
            &loop_lease,
            &inflight_turns,
            &turn_loop_in_flight,
        ));
        assert!(cancel.is_cancelled_for(&state.session_id));
        assert!(state.goal_state.is_some());
    }
}
