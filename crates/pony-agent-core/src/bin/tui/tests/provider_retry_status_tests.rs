//! 2026-11-14：上游重试进度的状态行呈现 + 终态错误的单一可见面（去重）契约。
//!
//! 三条不变量：
//! 1. 重试期状态行给出**真实进度**（`正在重试（n/N）…`，Warn 色），不再含糊显示「思考中」；
//! 2. 进度位不依赖 `is_streaming`（goal 续轮在轮次线程内直接开轮，不经派发点），
//!    由「首个增量 / 任一终态 / 新轮起点」清空（不残留假状态）；
//! 3. 终态错误只在状态行出现**一次**（消息区不再推同一句 System 消息），且只用警示色、
//!    不再叠加 BOLD 加重。

use super::*;

fn streaming_state_with_retry(attempt: u32, max_attempts: u32) -> TuiState {
    let mut state = TuiState::default();
    state.is_streaming = true;
    state.transition_runtime_mode(RuntimeMode::Thinking);
    state.note_provider_retry(attempt, max_attempts);
    state
}

#[test]
fn retry_progress_replaces_progress_phrase_while_streaming() {
    let state = streaming_state_with_retry(2, 10);

    let (text, tone) = status_word_base(&state);

    assert_eq!(
        text, "正在重试（2/10）…",
        "重试期状态行必须给出后端真实进度（分子=已失败次数、分母=尝试总数），实际 = {text:?}"
    );
    assert_eq!(
        tone,
        StatusTone::Warn,
        "重试属「进行中的异常」，必须用警示色（Warn），不得混同普通进度"
    );
}

#[test]
fn retry_progress_shows_without_relying_on_is_streaming() {
    // goal 自动续轮在轮次线程内直接开轮，`is_streaming` 未必为真 ⇒ 进度位不得被它门掉。
    let mut state = TuiState::default();
    assert!(!state.is_streaming, "夹具前提：非流式态");
    state.note_provider_retry(1, 10);

    let (text, tone) = status_word_base(&state);

    assert_eq!(
        text, "正在重试（1/10）…",
        "重试进度不得依赖派生标志 `is_streaming`，实际 = {text:?}"
    );
    assert_eq!(tone, StatusTone::Warn);
}

#[test]
fn retry_progress_is_cleared_by_clear_call() {
    let mut state = streaming_state_with_retry(1, 10);
    state.clear_provider_retry();

    let (text, _tone) = status_word_base(&state);

    assert!(
        !text.contains("正在重试"),
        "清空后不得残留重试文案（恢复/终态/新轮三处都走 clear_provider_retry），实际 = {text:?}"
    );
    assert_eq!(state.provider_retry, None);
}

#[test]
fn terminal_failure_renders_controlled_summary_in_status_line() {
    let mut state = TuiState::default();
    state.transition_runtime_mode(RuntimeMode::Failed(ERROR_SUMMARY_TRANSIENT.to_string()));

    let (text, tone) = status_word_base(&state);

    assert_eq!(
        text, ERROR_SUMMARY_TRANSIENT,
        "终态错误是状态行**唯一**可见载体（消息区不再重复），必须原样呈现受控摘要，\
         不能退化成「失败」占位词，实际 = {text:?}"
    );
    assert_eq!(tone, StatusTone::Danger, "终态错误必须用警示色 DANGER");
}

#[test]
fn retry_notice_reaches_installed_observer() {
    let (tx, rx) = channel();
    let _observer = install_tui_retry_observer(tx);

    pony_agent_core::agent::retry::notify_provider_retry(
        &pony_agent_core::agent::retry::RetryNotice {
            label: "decision".to_string(),
            scope: "decision",
            attempt: 3,
            max_attempts: 10,
            delay_ms: 2_000,
            error_class: "transient_retryable".to_string(),
        },
    );

    match rx.try_recv() {
        Ok(UiEvent::ProviderRetry {
            attempt,
            max_attempts,
        }) => {
            assert_eq!(
                (attempt, max_attempts),
                (3, 10),
                "观察者必须逐字转发真实进度，不得改写分子/分母"
            );
        }
        other => panic!("必须收到 UiEvent::ProviderRetry，实际 = {other:?}"),
    }
}

#[test]
fn retry_observer_guard_detaches_on_drop() {
    let (tx, rx) = channel();
    {
        let _observer = install_tui_retry_observer(tx);
    }

    pony_agent_core::agent::retry::notify_provider_retry(
        &pony_agent_core::agent::retry::RetryNotice {
            label: "decision".to_string(),
            scope: "decision",
            attempt: 1,
            max_attempts: 10,
            delay_ms: 500,
            error_class: "transient_retryable".to_string(),
        },
    );

    assert!(
        rx.try_recv().is_err(),
        "守卫 drop 后观察者必须已摘除（否则后续轮次的线程会收到悬挂通道的错误投递）"
    );
}
