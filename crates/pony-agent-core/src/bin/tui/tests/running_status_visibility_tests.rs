//! 2026-11-16：正文末尾「进行中」状态行的可见性契约（backlog:B197）。
//!
//! 现场证据：会话 `pa_01a1df2f-8315` 第一轮 16.9 min、35 次工具调用，末段一次 provider
//! follow-up 连续失败（SSE 解码失败 75.5s → 同步兜底 connect 31s → 4×21.2s 超时 → 预算耗尽），
//! 期间界面底部**没有任何进行中反馈**，用户据此判定「会话无故停止」。
//!
//! 三条不变量：
//! 1. 门是「轮次进行中」，不是 `is_streaming`（派发点标志）：goal 续轮等入口在轮次线程内
//!    直接开轮，`is_streaming` 未必为真 ⇒ 以它作门会让整段没有反馈；
//! 2. 工具执行期同样渲染：非折叠态下工具行只随正文滚动可见（长输出会顶出视口），
//!    折叠态（`hide_process`）下完全没有工具行 ⇒ 该行是唯一的过程信号；
//! 3. 终态（就绪 / 完成 / 失败 / 待确认）不渲染：终态由 `status_notice`、消息区与
//!    Ask 面板承载，避免残留一行假「进行中」。
//!
//! 对齐面：deepseek harness 的 `RunningStatus` 在整个运行期恒显并带 1s 实时耗时。

use super::*;

fn state_in(mode: RuntimeMode) -> TuiState {
    let mut state = TuiState::default();
    state.transition_runtime_mode(mode);
    state.turn_started_at = Some(std::time::Instant::now());
    state
}

/// 现场回归：工具执行期且过程可见（默认 `hide_process=false`）必须有进行中状态行。
#[test]
fn tool_execution_keeps_running_status_visible() {
    let mut state = state_in(RuntimeMode::ExecutingTool("Run".to_string()));
    state.is_streaming = true;
    assert!(
        !state.hide_process,
        "前提：默认过程可见，工具行随正文滚动 —— 正是「工具行被顶出视口」场景"
    );

    assert!(
        should_render_running_status(&state),
        "工具执行期不得整行抑制：长工具调用期间这是底部唯一的进行中信号"
    );
    let (text, _tone) = status_word_base(&state);
    assert!(
        text.contains("执行") && text.contains("Run"),
        "工具执行期状态词必须点名在跑什么，实际 = {text:?}"
    );
}

/// 折叠态（过程隐藏）不得因工具执行而丢失进行中反馈。
#[test]
fn collapsed_process_keeps_running_status_visible() {
    let mut state = state_in(RuntimeMode::ExecutingTool("Run".to_string()));
    state.hide_process = true;

    assert!(
        should_render_running_status(&state),
        "折叠态下完全没有工具行，状态行是唯一过程信号"
    );
}

/// 思考 / 生成 / 连接三态在运行期恒显（长静默 = 仍在跑，不是停止）。
#[test]
fn in_flight_modes_keep_running_status_visible() {
    for mode in [
        RuntimeMode::Connecting,
        RuntimeMode::Thinking,
        RuntimeMode::Responding,
    ] {
        let state = state_in(mode.clone());
        assert!(
            should_render_running_status(&state),
            "进行中态 {mode:?} 必须渲染状态行"
        );
    }
}

/// 门不依赖 `is_streaming`：goal 自动续轮在轮次线程内直接开轮。
#[test]
fn gate_does_not_depend_on_is_streaming() {
    let mut state = state_in(RuntimeMode::Thinking);
    state.is_streaming = false;

    assert!(
        should_render_running_status(&state),
        "`is_streaming` 只是派发点标志，不得当作渲染门"
    );
}

/// 终态不渲染：避免「完成/失败/待确认」时残留一行进行中。
#[test]
fn terminal_modes_hide_running_status() {
    for mode in [
        RuntimeMode::Ready,
        RuntimeMode::Completed,
        RuntimeMode::Failed("失败摘要".to_string()),
        RuntimeMode::WaitingApproval,
    ] {
        let state = state_in(mode.clone());
        assert!(
            !should_render_running_status(&state),
            "终态 {mode:?} 不得渲染进行中状态行"
        );
    }
}

/// 重试进度与停止请求是「用户此刻唯一关心的事实」：独立于运行态强制可见。
#[test]
fn retry_and_stopping_force_visible() {
    let mut retrying = state_in(RuntimeMode::Ready);
    retrying.note_provider_retry(2, 5);
    assert!(
        should_render_running_status(&retrying),
        "重试进度必须可见（此时模型并未在产出）"
    );

    let mut stopping = state_in(RuntimeMode::Ready);
    stopping.stopping_turn_id = Some("turn-x".to_string());
    assert!(
        should_render_running_status(&stopping),
        "停止请求受理后必须可见「正在停止…」"
    );
}

/// 时钟驱动门必须与渲染门同源：`is_streaming=false` 的续轮入口也要走节拍。
///
/// 只修渲染门的反例：行画出来了，但 `spinner_tick`/`elapsed_label` 不再刷新 ——
/// 用户看到的是「17 分钟前开始、秒数不动」的一行，与没有这一行同样被读成卡死。
#[test]
fn running_clock_drives_without_is_streaming() {
    let mut state = state_in(RuntimeMode::Thinking);
    state.is_streaming = false;
    assert!(
        should_drive_running_clock(&state, false),
        "续轮入口未置 is_streaming 时，计时/动效仍必须走帧节拍"
    );
}

/// 工具执行期同样要驱动计时：非折叠态且无流式时，秒数不得停在进入工具的那一刻。
#[test]
fn running_clock_drives_during_tool_execution_without_streaming() {
    let mut state = state_in(RuntimeMode::ExecutingTool("Run".to_string()));
    state.is_streaming = false;
    assert!(!state.hide_process, "前提：默认过程可见");
    assert!(
        should_drive_running_clock(&state, false),
        "长工具调用期间即使没有流式增量，已耗时也必须继续走"
    );
}

/// 终态不再走节拍：不制造假进行中，也不为空转做全屏重绘。
#[test]
fn running_clock_idles_in_terminal_modes() {
    for mode in [
        RuntimeMode::Ready,
        RuntimeMode::Completed,
        RuntimeMode::Failed("失败摘要".to_string()),
        RuntimeMode::WaitingApproval,
    ] {
        let state = state_in(mode.clone());
        assert!(
            !should_drive_running_clock(&state, false),
            "终态 {mode:?} 不得继续驱动运行期节拍"
        );
    }
}

/// Ask 等待期独立驱动（与运行态无关的既有低频呼吸）。
#[test]
fn running_clock_drives_while_awaiting_answer() {
    let state = state_in(RuntimeMode::Ready);
    assert!(
        should_drive_running_clock(&state, true),
        "问答等待期的呼吸提醒不得被运行态门吞掉"
    );
}
