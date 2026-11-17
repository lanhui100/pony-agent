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

// ─────────────────────────────────────────────────────────────────────────────
// B218：状态行生命周期门 + 合成纯函数（本段为新增验收，不得改动上方 B197 已冻结用例）
//
// 红线：`should_render_running_status` 与「生命周期」必须同源（唯一门），渲染文案由
// `compose_running_status_text` 单点合成 —— 杜绝「门开了但文案为空」「终态残留进行中」
// 「折叠态丢工具名」三类复发。时间不取自真实时钟：已耗时一律由调用方注入字符串。
// ─────────────────────────────────────────────────────────────────────────────

/// 只经由公开的状态迁移入口进入运行态；不写 `turn_started_at`，避免用例依赖真实时钟。
fn b218_state_in(mode: RuntimeMode) -> TuiState {
    let mut state = TuiState::default();
    state.transition_runtime_mode(mode);
    state
}

/// 运行态模式下「生产侧同一状态词真源」产出的状态词（与渲染点同源，不自造文案）。
fn b218_word(state: &TuiState) -> String {
    status_word_base(state).0
}

/// 现场回归（B218 主症）：工具执行期、非折叠、`is_streaming=false` 且**无任何事件流入**时，
/// 状态行必须是可渲染的非空文本，且逐字点名工具与已耗时。
#[test]
fn b218_tool_execution_line_non_empty_with_elapsed() {
    // Arrange：只进入工具执行态，不投递任何 UI 事件、不置 is_streaming。
    let mut state = b218_state_in(RuntimeMode::ExecutingTool("Bash".to_string()));
    state.is_streaming = false;
    assert!(
        !state.hide_process,
        "前提：默认过程可见（工具行随正文滚动，这正是长输出顶出视口的现场）"
    );
    let word = b218_word(&state);

    // Act
    let text = compose_running_status_text(&state, &word, "1m20s").expect(
        "轮次进行中（ExecutingTool）必须合成状态行：返回 None 就等于整段工具执行期零反馈",
    );

    // Assert
    assert!(!text.trim().is_empty(), "状态行不得为空白，实际 = {text:?}");
    assert!(
        text.contains("Bash"),
        "状态行必须点名正在执行的工具，实际 = {text:?}"
    );
    assert!(
        text.contains("1m20s"),
        "elapsed 非空时必须逐字出现在状态行，实际 = {text:?}"
    );
    assert!(
        should_render_running_status(&state),
        "工具执行期渲染门必须为开，否则纯函数算得出、渲染点却不画"
    );
}

/// 反向门禁：`is_streaming` 不是生命周期门。非流式续轮入口（goal 自动续轮）同样要有
/// 状态行与逐字耗时，防止「以 is_streaming 当门」的缺陷换个入口复发。
#[test]
fn b218_non_streaming_continuation_keeps_line() {
    // Arrange
    let mut state = b218_state_in(RuntimeMode::Thinking);
    state.is_streaming = false;
    let word = b218_word(&state);

    // Act
    let text = compose_running_status_text(&state, &word, "5s")
        .expect("思考期即便没有流式增量，轮次仍在进行中，状态行不得缺席");

    // Assert
    assert!(!text.trim().is_empty(), "状态行不得为空白，实际 = {text:?}");
    assert!(
        text.contains("5s"),
        "非流式续轮也必须带上已耗时（秒数定格会被读成卡死），实际 = {text:?}"
    );
    assert!(
        should_render_running_status(&state),
        "非流式续轮的渲染门必须为开"
    );
}

/// 折叠态（`hide_process=true`）没有任何工具行，状态行是唯一过程信号：不得因折叠而丢工具名。
#[test]
fn b218_collapsed_process_keeps_tool_line() {
    // Arrange
    let mut state = b218_state_in(RuntimeMode::ExecutingTool("Bash".to_string()));
    state.hide_process = true;
    state.is_streaming = false;
    assert!(state.hide_process, "前提：折叠态下工具行整块不渲染");
    let word = b218_word(&state);

    // Act
    let text = compose_running_status_text(&state, &word, "2s")
        .expect("折叠态必须合成状态行：此时界面没有其他过程信号");

    // Assert
    assert!(!text.trim().is_empty(), "状态行不得为空白，实际 = {text:?}");
    assert!(
        text.contains("Bash"),
        "折叠态状态行必须点名工具（否则用户只看到一句无内容的副词），实际 = {text:?}"
    );
}

/// 终态四态一律不合成、不渲染：终态由 status_notice / 消息区 / Ask 面板承载。
#[test]
fn b218_terminal_lifecycle_hides_line() {
    for mode in [
        RuntimeMode::Ready,
        RuntimeMode::Completed,
        RuntimeMode::Failed("x".to_string()),
        RuntimeMode::WaitingApproval,
    ] {
        // Arrange
        let state = b218_state_in(mode.clone());
        let word = "处理中";

        // Act
        let text = compose_running_status_text(&state, word, "3s");

        // Assert
        assert_eq!(
            text,
            None,
            "终态 {mode:?} 不得合成进行中状态行（否则残留一行假「进行中」）"
        );
        assert!(
            !should_render_running_status(&state),
            "终态 {mode:?} 渲染门必须为关"
        );
    }
}

/// 门与生命周期同源：运行态 ⇒ Running + 门开；终态 ⇒ Terminal + 门关。
///
/// 两套口径分家时会出现「门按 A 判、文案按 B 判」——本用例逐态钉住两边同源。
#[test]
fn b218_turn_lifecycle_matches_gate() {
    for mode in [
        RuntimeMode::Connecting,
        RuntimeMode::Thinking,
        RuntimeMode::Responding,
        RuntimeMode::ExecutingTool("Bash".to_string()),
    ] {
        let state = b218_state_in(mode.clone());
        assert!(
            matches!(turn_lifecycle(&state), TurnLifecycle::Running),
            "运行态 {mode:?} 的生命周期必须是 Running"
        );
        assert!(
            should_render_running_status(&state),
            "运行态 {mode:?} 的渲染门必须与 Running 同源为开"
        );
    }

    for mode in [
        RuntimeMode::Ready,
        RuntimeMode::Completed,
        RuntimeMode::Failed("x".to_string()),
        RuntimeMode::WaitingApproval,
    ] {
        let state = b218_state_in(mode.clone());
        assert!(
            matches!(turn_lifecycle(&state), TurnLifecycle::Terminal),
            "终态 {mode:?} 的生命周期必须是 Terminal"
        );
        assert!(
            !should_render_running_status(&state),
            "终态 {mode:?} 的渲染门必须与 Terminal 同源为关"
        );
    }
}

/// `word` 为空白时必须回落非空占位：空词不得把整行变成一串空白（视觉等价于没有状态行）。
#[test]
fn b218_blank_word_falls_back_to_placeholder() {
    // Arrange：轮次进行中（门开），但调用方这次拿到的状态词是空白。
    let state = b218_state_in(RuntimeMode::Thinking);
    assert!(
        should_render_running_status(&state),
        "前提：思考期渲染门为开，空白状态词必须由合成纯函数兜住"
    );

    // Act
    let text = compose_running_status_text(&state, "   ", "")
        .expect("运行态即便入参全空也必须给出占位状态行");

    // Assert
    assert!(
        !text.trim().is_empty(),
        "word 为空白时不得产出空白行，实际 = {text:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// B231：等待首字节的用户可见文案（`status_dwell` 的 `UpstreamIdle` + 状态词）
//
// 现场：请求已发出、上游尚无任何增量时，状态行显示「思考中 45s · 仍在等待上游首字节…
// （已静默 40s）」——「上游 / 首字节 / 静默秒数」是工程细节，用户只想知道「在等」。
// 本段钉住三件事：等待窗口内状态词 = 「等待响应中…」、状态行不含任何工程细节、
// 窗口只由真实事件（请求已发出 / 增量 / 工具执行 / 收口）推进，不伪造等待态。
// ─────────────────────────────────────────────────────────────────────────────

/// 等待窗口内状态词必须是等待态（而不是「思考中」或任何上游/首字节措辞）。
#[test]
fn b231_waiting_word_when_awaiting_first_byte() {
    // Arrange：本轮进行中 + 真实「请求已发出」事件。
    let mut state = state_in(RuntimeMode::Thinking);
    state.upstream_idle.note_request_sent();
    assert!(
        state.upstream_idle.awaiting_first_byte(),
        "前提：请求已发出且尚无增量 ⇒ 必须处于等待窗口"
    );

    // Act
    let (word, tone) = status_word_base(&state);

    // Assert
    assert_eq!(
        word, "等待响应中…",
        "等待首字节只呈现「在等」这一事实，实际 = {word:?}"
    );
    assert_eq!(tone, StatusTone::Muted, "等待态是中性信息色，不是告警");
}

/// 状态行（状态词 + 耗时）不得出现任何工程细节：上游 / 首字节 / 静默 / token。
#[test]
fn b231_status_line_has_no_engineering_details() {
    // Arrange
    let mut state = state_in(RuntimeMode::Thinking);
    state.upstream_idle.note_request_sent();

    // Act
    let (word, _) = status_word_base(&state);
    let text = compose_running_status_text(&state, &word, "12s").expect("运行态必须给出状态行");

    // Assert：等待词 + 耗时，且无工程附注。
    assert!(text.contains("等待响应中"), "实际 = {text:?}");
    assert!(text.contains("12s"), "等待时长仍由既有耗时承担，实际 = {text:?}");
    for banned in ["上游", "首字节", "静默", "token", "TTFT"] {
        assert!(
            !text.contains(banned),
            "状态行不得含工程细节 {banned:?}，实际 = {text:?}"
        );
    }
}

/// 首个增量即退出等待窗口：状态词回到本轮真实进度，不残留等待词。
#[test]
fn b231_first_delta_leaves_waiting_window() {
    // Arrange
    let mut state = state_in(RuntimeMode::Thinking);
    state.upstream_idle.note_request_sent();

    // Act
    state.upstream_idle.note_delta();

    // Assert
    assert!(
        !state.upstream_idle.awaiting_first_byte(),
        "已有增量 ⇒ 不得仍判为等待首字节"
    );
    let (word, _) = status_word_base(&state);
    assert_ne!(
        word, "等待响应中…",
        "首增量后必须回到真实进度词，实际 = {word:?}"
    );
}

/// 窗口只由真实事件推进：无事件不伪造等待态，工具执行期与收口必须退出窗口。
#[test]
fn b231_waiting_window_is_event_driven() {
    // Assert 1：无 `calling_model` 事件 ⇒ 不在等待窗口，状态词保持既有进度词。
    let mut state = state_in(RuntimeMode::Thinking);
    assert!(
        !state.upstream_idle.awaiting_first_byte(),
        "无真实事件时不得伪造等待态"
    );
    let (word, _) = status_word_base(&state);
    assert_ne!(word, "等待响应中…", "无事件时实际 = {word:?}");

    // Assert 2：工具执行期不等待上游 ⇒ 窗口退出，状态词点名工具。
    state.upstream_idle.note_request_sent();
    state.transition_runtime_mode(RuntimeMode::ExecutingTool("Run".to_string()));
    state.upstream_idle.reset();
    assert!(
        !state.upstream_idle.awaiting_first_byte(),
        "工具执行期不得残留等待态"
    );

    // Assert 3：复位（轮次边界 / 终态 / 换会话共用同一入口）后窗口必须关闭。
    state.upstream_idle.note_request_sent();
    state.upstream_idle.reset();
    assert!(
        !state.upstream_idle.awaiting_first_byte(),
        "reset 后不得残留等待态"
    );
}

// ── B198：followup 阶段位（断流→首次同步→本地兜底）──────────────────────────

/// 阶段位必须让生命周期保持 Running：即使 `runtime_mode` 已回终态（Ready/Completed），
/// 只要兜底链路还在跑，底部就必须有进行中反馈——这是 B198 现场的直接回归。
#[test]
fn b198_phase_keeps_running_status_visible() {
    use pony_agent_core::agent::retry::ProviderPhase;

    let mut state = state_in(RuntimeMode::Ready);
    assert!(
        !should_render_running_status(&state),
        "前提：无任何在途信号时终态不渲染进行中行"
    );

    state.note_provider_phase(ProviderPhase::SyncAttempt { attempt: 1 });

    assert!(
        should_render_running_status(&state),
        "首次同步兜底尝试没有重试计数，阶段位是唯一在途信号"
    );
    let (text, _tone) = status_word_base(&state);
    assert!(
        text.contains("同步兜底") && text.contains('1'),
        "状态词必须点名阶段与真实尝试序号，实际 = {text:?}"
    );
}

/// 三阶段文案互不混淆，且「本地兜底」是可识别的降级收尾（不是正常生成）。
#[test]
fn b198_phase_texts_are_distinguishable() {
    use pony_agent_core::agent::retry::ProviderPhase;

    let mut state = state_in(RuntimeMode::Responding);
    state.note_provider_phase(ProviderPhase::StreamFallback);
    let (stream, _) = status_word_base(&state);

    state.note_provider_phase(ProviderPhase::LocalFallback);
    let (local, tone) = status_word_base(&state);

    assert!(stream.contains("流式中断"), "实际 = {stream:?}");
    assert!(local.contains("本地兜底"), "实际 = {local:?}");
    assert_ne!(stream, local, "过程中断与降级收尾必须可区分");
    assert!(
        matches!(tone, StatusTone::Warn),
        "降级收尾必须用警示色，不得读成正常输出"
    );
}

/// 重试计数比阶段更精确 ⇒ 同时存在时以计数为准；二者生命周期同源，一起清空。
#[test]
fn b198_retry_outranks_phase_and_both_clear_together() {
    use pony_agent_core::agent::retry::ProviderPhase;

    let mut state = state_in(RuntimeMode::Responding);
    state.note_provider_retry(2, 5);
    state.note_provider_phase(ProviderPhase::SyncAttempt { attempt: 2 });

    let (text, _tone) = status_word_base(&state);
    assert!(
        text.contains("正在重试"),
        "重试计数在场时必须优先显示真实进度，实际 = {text:?}"
    );

    state.clear_provider_retry();
    assert!(state.provider_phase.is_none(), "阶段位必须与重试位一并摘除");
    let (after, _tone) = status_word_base(&state);
    assert!(
        !after.contains("正在重试") && !after.contains("同步兜底"),
        "两处都清空后不得残留进度文案，实际 = {after:?}"
    );
}
