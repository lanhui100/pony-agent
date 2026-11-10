//! l2at-tui-overlay-cancel：TUI Ask 覆盖层与取消反馈的**独立黑盒验收**（Test Agent 冻结，Executor 只读）。
//!
//! 契约真值源：`.dev-team/contract-matrix-tui-overlay-cancel.md` §2「黑盒验收矩阵」
//! （O1–O8 / J1 / C9）与 `docs/decisions/0063-tui-ask-user-overlay-and-cancel-feedback.md`。
//!
//! 测试面分三层，全部落在**已存在的**公共接缝上（不新增业务接口）：
//!
//! 1. 渲染层（O1/O2/O3）：沿用既有 `#[path = "../src/bin/tui.rs"] mod tui` +
//!    ratatui `TestBackend`（与 `tests/tui_render_layout.rs` 同法），把 `ui()` 真实渲染进
//!    字符缓冲后断言「历史与问题同帧」「切换 Ask 不改变消息几何」。
//! 2. 状态迁移层（O4/O5/O6/O7/O8）：`TuiState::apply_stop_requested` /
//!    `note_stopping_key` / `apply_stop_terminal` / `start_ask_user` / `cancel_ask_user` 与
//!    `stop_sequence::StopSequenceState` 的既有公共语义。
//! 3. 真实可达行为层（C9/J1）：`CARGO_BIN_EXE_pony` 是**本次构建**的真实二进制，经伪终端
//!    （python `pty`）走完整键鼠/事件链路。这两条用例的判据只能由真实进程给出：
//!    - C9：双击 Esc 期间控制面 `stop_turn` **必须**有已注册 sink（否则 core 会打印
//!      `stop accepted but no turn event sink registered`），且终态「已停止（用户中断）」可见；
//!    - J1：`pony resume` 恢复的 trace 同时含 `description`/`arguments_text`/`result_text` 时，
//!      用户可见摘要是 `description`，结构化 JSON 不得成为消息摘要。
//!
//! 红相预期（当前实现）：O1、O2、J1、C9 以**断言失败**呈现；O3–O8 是兼容回归，不得伪造失败。
//! 物理凭证：`tests/acceptance/receipts/l2at-tui-overlay-cancel/`（red.log 由验收脚本产出）。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use pony_agent_core::agent::stop_sequence::{StopIdentity, StopSequenceState, STOP_SEQUENCE_MS};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;
use std::path::PathBuf;
use std::process::Command;
use tui::{AskSubmitVerdict, AskUserQuestion, InteractionMode, MessageRole, QuestionOption, TuiState};

// ─────────────────────────────────────────────────────────────────────────────
// 夹具与渲染工具
// ─────────────────────────────────────────────────────────────────────────────

/// 唯一的「历史消息」哨兵：只出现在历史正文，不出现在 Ask 面板与提示里。
const HISTORY_SENTINEL: &str = "HISTORY_SENTINEL_o1_overlay";
/// 唯一的「问题」哨兵：只出现在 Ask 面板标题行。
const QUESTION_SENTINEL: &str = "QUESTION_SENTINEL_o1_overlay";

fn render(state: &mut TuiState, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("create test backend");
    terminal
        .draw(|frame| tui::ui(frame, state))
        .expect("render frame");
    terminal.backend().buffer().clone()
}

fn row_text(buf: &Buffer, y: u16) -> String {
    let mut out = String::new();
    for x in 0..buf.area.width {
        out.push_str(buf[(x, y)].symbol());
    }
    out
}

/// CJK 等宽字符在 `Buffer` 里占 2 个 cell（第二格是空白占位），逐字取符号会得到
/// 「上 下 文」；对**内容**做包含判断前先去掉空白。逐格相等断言仍用原始字节（不做归一）。
fn squeeze_text(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn all_text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

fn ask_question(question: &str) -> AskUserQuestion {
    AskUserQuestion {
        id: "q-overlay-1".to_string(),
        header: Some("验收分支".to_string()),
        question: question.to_string(),
        multi_select: false,
        options: vec![
            QuestionOption {
                label: "分支 A".to_string(),
                description: Some("先做冒烟".to_string()),
            },
            QuestionOption {
                label: "分支 B".to_string(),
                description: Some("直接全量".to_string()),
            },
        ],
    }
}

/// 至少一条短历史（用户 + 助手），保证消息区非空且正文可滚动。
///
/// 同时给出**已知上下文窗口 + 已知用量**：指标行（`metrics_line`）只有在
/// `context_window_tokens > 0` 时才渲染「上下文 N%」段，否则该行只剩工作区标签。
/// 夹具缺这两项会让「Ask 遮挡指标行」的判据退化成对空行的断言（测试缺陷，不是实现回归）。
fn history_state() -> TuiState {
    let mut state = TuiState::default();
    state.add_user_message(&format!("上一条问题 {HISTORY_SENTINEL}"));
    state.append_assistant_chunk("历史回答正文：这里是提问前已经被看到的上下文。");
    state.context_window_tokens = 256_000;
    state.last_context_tokens = 1_024;
    state.total_input_tokens = 1_024;
    state.total_output_tokens = 512;
    state
}

/// 已挂起在 Ask 面板上的状态（问题哨兵只出现在面板标题）。
fn ask_state(question: &str) -> TuiState {
    let mut state = history_state();
    state.start_ask_user("req-l2at-1".to_string(), vec![ask_question(question)]);
    state
}

fn streaming_state() -> TuiState {
    let mut state = TuiState::default();
    state.is_streaming = true;
    state.add_user_message("stop me");
    state.append_assistant_chunk("partial answer");
    state
}

/// Composer 顶部行号：`input_h = 2`（空输入 = 1 视觉行 + 1 行呼吸，clamp 下限 2），
/// `bottom_h = input_h + 2`（色带 + 指标行）⇒ `bottom_top = height - 4`。
fn composer_top(height: u16) -> u16 {
    height - 4
}

fn last_assistant(state: &TuiState) -> &str {
    state
        .messages
        .iter()
        .rev()
        .find(|message| message.role == MessageRole::Assistant)
        .map(|message| message.content.as_str())
        .expect("必须存在助手消息")
}

// ─────────────────────────────────────────────────────────────────────────────
// O1：历史与问题同帧可见，Ask 不覆盖 header / Composer / 指标行
// ─────────────────────────────────────────────────────────────────────────────

/// O1 夹具前置条件（缺陷证据入口）：**没有 Ask** 的正常帧，header 右槽也必须已经渲染出
/// 数值指标（`上下文` 段）。若这里失败，说明是夹具没有设置 `context_window_tokens`，
/// 而不是「Ask 遮挡指标」——两种失败必须可区分，否则 O1 的判据会把夹具缺陷误报成实现回归。
///
/// ADR 0078：数值指标簇位于 header（row0）右槽，底部最后一行改为模型身份，
/// 故本前置条件与遮挡判据都锚定 row0。
#[test]
fn o1_metrics_fixture_precondition_closed_frame_has_token_metrics() {
    // Arrange
    let mut state = history_state();

    // Act：与 O1 同尺寸、同状态，只是不打开 Ask
    let buf = render(&mut state, 120, 34);
    let header_row = row_text(&buf, 0);

    // Assert
    assert!(
        squeeze_text(&header_row).contains("上下文"),
        "O1 夹具前置条件：正常（无 Ask）帧的 header 右槽必须含「上下文」数值段，\
         实际 row0 = {header_row:?}。若此处失败 = 夹具未设置 context_window_tokens（测试缺陷），\
         不是被测实现的遮挡回归。"
    );
}

#[test]
fn o1_history_and_question_share_one_frame_without_wiping_scrollback() {
    // Arrange：同一夹具的「关闭 Ask」基线帧，作为未被遮挡的逐格真值（不是放宽断言，
    // 而是把判据从『指标行含某个词』提升为『固定基座逐字不变』）。
    let mut baseline_state = history_state();
    let baseline = render(&mut baseline_state, 120, 34);
    assert!(
        squeeze_text(&row_text(&baseline, 0)).contains("上下文"),
        "O1 夹具前置条件：基线帧 header 右槽必须含数值指标（ADR 0078），实际 row0 = {:?}",
        row_text(&baseline, 0)
    );

    let mut state = ask_state(QUESTION_SENTINEL);

    // Act
    let buf = render(&mut state, 120, 34);
    let screen = all_text(&buf);

    // Assert 1：同一帧同时可见历史哨兵与问题哨兵
    assert!(
        screen.contains(HISTORY_SENTINEL),
        "O1：Ask 打开后历史消息必须仍在同一帧可见（不得整区 Clear）。首帧屏幕 =\n{screen}"
    );
    assert!(
        screen.contains(QUESTION_SENTINEL),
        "O1：Ask 的问题必须在同一帧可见。首帧屏幕 =\n{screen}"
    );

    // Assert 2：header 行必须与基线逐字一致（Ask 不得遮挡）
    assert_eq!(
        row_text(&buf, 0),
        row_text(&baseline, 0),
        "O1：Ask 打开后 header 行与基线帧必须逐字一致"
    );

    // Assert 3：Composer 基座仍在（Ask 态色带按既有约定显示「等待你的选择…」替身，
    // 但色带本身不得被面板覆盖成空白/被清除）。
    let band_rows: Vec<String> = (composer_top(34)..33)
        .map(|y| (y, squeeze_text(&row_text(&buf, y))))
        .filter(|(_, text)| text.contains("等待你的选择"))
        .map(|(y, _)| y.to_string())
        .collect();
    assert!(
        !band_rows.is_empty(),
        "O1：Ask 态 Composer 色带必须仍在渲染（既有「等待你的选择…」替身），\
         实际色带区 rows {}..33 = {:?}",
        composer_top(34),
        (composer_top(34)..33).map(|y| row_text(&buf, y)).collect::<Vec<_>>()
    );

    // Assert 4：底面行（右对齐模型身份，ADR 0078）必须与基线逐字一致（Ask 不得遮挡、不得改写）
    assert_eq!(
        row_text(&buf, 33),
        row_text(&baseline, 33),
        "O1：Ask 打开后底面行与基线帧必须逐字一致（实际 = {:?}，基线 = {:?}）",
        row_text(&buf, 33),
        row_text(&baseline, 33)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// O2：同一状态/宽度/滚动位置下，仅切换 Ask 显示不得改变消息几何
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn o2_toggling_ask_keeps_message_geometry_and_uncovered_rows_stable() {
    // Arrange：两份内容完全一致的状态，唯一差别是 Ask 是否显示。
    let mut closed = history_state();
    let buf_closed = render(&mut closed, 120, 34);
    let rect_closed = closed.last_chat_rect.expect("首帧必须记录 chat_rect");

    let mut opened = ask_state(QUESTION_SENTINEL);
    let ask_height = tui::inline_ask_height(&opened, 120);
    assert!(ask_height > 0, "O2 前置条件：Ask 面板必须有高度");
    let buf_opened = render(&mut opened, 120, 34);
    let rect_opened = opened.last_chat_rect.expect("首帧必须记录 chat_rect");

    // Assert 1：消息区几何与 Ask 显示解耦（覆盖层而不是布局项）
    assert_eq!(
        rect_closed, rect_opened,
        "O2：仅切换 Ask 显示时 chat_rect 必须完全一致（Ask 不得从消息区扣除高度）；\
         关闭时 = {rect_closed:?}，打开时 = {rect_opened:?}"
    );

    // Assert 2：未被面板覆盖的历史行逐格不变
    let overlay_top = composer_top(34) - ask_height;
    for y in 0..overlay_top {
        assert_eq!(
            row_text(&buf_closed, y),
            row_text(&buf_opened, y),
            "O2：row{y} 未被 Ask 覆盖，必须逐格不变"
        );
    }

    // Assert 3：面板矩形下缘仍贴 Composer 上缘，但**末行是下内边距**（1 行空白）——
    // 版式要求「面板有上下内边距」，最后一个内容行（键位提示）因此落在 `composer_top - 2`。
    let panel_bottom = (0..composer_top(34))
        .rev()
        .find(|&y| {
            let text = row_text(&buf_opened, y);
            text.contains(QUESTION_SENTINEL) || text.contains("esc")
        })
        .expect("O2：必须能在 Composer 之上找到 Ask 面板最后一条内容行");
    assert_eq!(
        panel_bottom,
        composer_top(34) - 2,
        "O2：Ask 面板最后一条内容行必须在 Composer 之上第 2 行（下内边距占 1 行；\
         期望 row{}，实际 row{panel_bottom}）",
        composer_top(34) - 2
    );
    let padding_row = row_text(&buf_opened, composer_top(34) - 1);
    assert!(
        padding_row.trim().is_empty(),
        "O2：面板下内边距必须是整行空白（不得贴字），实际 = {padding_row:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// O3：宽/窄/极小终端与长问题、长选项、CJK —— 不 panic、不越界、保留既有行上限
// ─────────────────────────────────────────────────────────────────────────────

fn long_ask_state() -> TuiState {
    let mut question = AskUserQuestion {
        id: "q-long".to_string(),
        header: Some("超长问题与选项".to_string()),
        question: "LONGQ_SENTINEL_这是一个很长很长的问题，用来验证折行与省略不会把面板撑破终端边界".to_string(),
        multi_select: false,
        options: (1..=12)
            .map(|index| QuestionOption {
                label: format!("选项{index}：OPTION_SENTINEL_很长的选项标签用于挤压列预算"),
                description: Some("说明文字同样很长，用于验证省略与窄终端降级".to_string()),
            })
            .collect(),
    };
    if question.question.is_empty() {
        question.question = "LONGQ_SENTINEL".to_string();
    }
    let mut state = history_state();
    state.start_ask_user("req-long".to_string(), vec![question]);
    state
}

#[test]
fn o3_ask_panel_survives_wide_narrow_and_tiny_terminals() {
    // Arrange：宽终端 120x34（≥60 列 ⇒ 硬上限 40 行，ADR 0081）
    let mut wide = long_ask_state();
    let wide_height = tui::inline_ask_height(&wide, 120);
    assert!(
        (4..=40).contains(&wide_height),
        "O3：宽终端面板高度必须落在 4..=40 行硬上限内（ADR 0081），实际 {wide_height}"
    );
    assert!(
        wide_height > 12,
        "O3：长问题换行后必须突破旧的 12 行上限，否则完整问题会被面板裁掉（ADR 0081），\
         实际 {wide_height}"
    );
    let wide_buf = render(&mut wide, 120, 34); // 不 panic 即通过
    assert!(
        all_text(&wide_buf).contains("LONGQ_SENTINEL"),
        "O3：宽终端长问题必须仍可见（允许省略尾部）"
    );

    // Arrange：窄终端 56x20（<60 列 ⇒ 硬上限 24 行，ADR 0081）
    let mut narrow = ask_state(QUESTION_SENTINEL);
    let narrow_height = tui::inline_ask_height(&narrow, 56);
    assert!(
        (4..=24).contains(&narrow_height),
        "O3：窄终端面板高度必须落在 4..=24 行硬上限内（ADR 0081），实际 {narrow_height}"
    );
    let narrow_buf = render(&mut narrow, 56, 20);
    let narrow_text = all_text(&narrow_buf);
    assert!(
        narrow_text.contains(QUESTION_SENTINEL),
        "O3：窄终端空间不足时必须优先保留问题，实际屏幕 =\n{narrow_text}"
    );
    assert!(
        narrow_text.contains("esc"),
        "O3：窄终端空间不足时必须优先保留提交/取消提示，实际屏幕 =\n{narrow_text}"
    );

    // Arrange：极小高度 40x10 —— 只要求不 panic、不越出终端（允许降级丢行）
    let mut tiny = long_ask_state();
    let tiny_buf = render(&mut tiny, 40, 10);
    assert_eq!(
        (tiny_buf.area.width, tiny_buf.area.height),
        (40, 10),
        "O3：极小终端渲染后缓冲区尺寸必须保持终端尺寸（不越出终端）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// O4：流式中 500ms 内双击 Esc ⇒ 第一击只上膛、第二击恰好一次；受理后只显示「正在停止…」
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn o4_double_escape_arms_then_fires_exactly_once_and_only_shows_stopping() {
    // Arrange
    let identity = StopIdentity {
        session_id: Some("session-o4".to_string()),
        turn_id: "turn-o4".to_string(),
        generation: 1,
    };
    let mut sequence = StopSequenceState::new(STOP_SEQUENCE_MS);

    // Act + Assert 1：第一击只上膛
    assert!(
        !sequence.press(identity.clone(), 1_000),
        "O4：第一击 Esc 只能上膛，不得派发停止请求"
    );
    // Assert 2：窗口内第二击恰好命中一次
    assert!(
        sequence.press(identity.clone(), 1_050),
        "O4：500ms 窗口内同身份第二击必须命中（发恰好一次停止请求）"
    );
    // Assert 3：命中即复位，第三击不得重复派发
    assert!(
        !sequence.press(identity.clone(), 1_060),
        "O4：命中后即复位，同一 turn 的第三击不得重复派发停止请求"
    );

    // Arrange：超窗（> 500ms）重新上膛而非命中
    let mut late = StopSequenceState::new(STOP_SEQUENCE_MS);
    assert!(!late.press(identity.clone(), 0));
    assert!(
        !late.press(identity.clone(), STOP_SEQUENCE_MS + 1),
        "O4：超过 500ms 窗口不得命中双击"
    );

    // Arrange：受理后只显示「正在停止…」，未收到终态不得显示已停止/已终止
    let mut state = streaming_state();
    state.apply_stop_requested("turn-o4");
    assert!(state.is_streaming, "O4：停止请求受理不是终态");
    assert!(
        state.status_text.contains("正在停止"),
        "O4：受理后状态行必须是「正在停止…」，实际 = {:?}",
        state.status_text
    );
    assert!(
        !state.status_text.contains("已停止") && !state.status_text.contains("已终止"),
        "O4：未收到终态事件前不得显示已停止/已终止，实际 = {:?}",
        state.status_text
    );
    assert!(
        state
            .messages
            .iter()
            .all(|message| !message.content.contains("已停止") && !message.content.contains("已终止")),
        "O4：未收到终态事件前不得向消息区写入终态文案"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// O5：turn:cancelled(stopReason=user) ⇒ 「已停止（用户中断）」、退出流式、正文保留、不新增消息
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn o5_user_cancel_terminal_shows_user_readable_stop_without_new_message() {
    // Arrange
    let mut state = streaming_state();
    state.apply_stop_requested("turn-o5");
    let messages_before = state.messages.len();

    // Act
    state.apply_stop_terminal("turn-o5", Some("user"));

    // Assert
    assert!(!state.is_streaming, "O5：终态事件后必须退出流式态");
    assert!(
        state.status_text.contains("已停止（用户中断）"),
        "O5：用户原因终态必须投影为「已停止（用户中断）」，实际 = {:?}",
        state.status_text
    );
    assert!(
        state.stopping_turn_id.is_none(),
        "O5：终态后 stopping 必须清理"
    );
    assert_eq!(
        last_assistant(&state),
        "partial answer",
        "O5：终态不得吞掉已流式正文"
    );
    assert_eq!(
        state.messages.len(),
        messages_before,
        "O5：取消终态不得新增 ChatMessage（终态只走既有状态通道）"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// O6：stopping 中重复 Esc / 重复终态 / 陈旧 turn 终态 —— 幂等且不得拆当前轮
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn o6_repeated_escape_and_duplicate_or_stale_terminal_are_idempotent() {
    // Arrange
    let mut state = streaming_state();
    state.apply_stop_requested("turn-o6");
    let messages_before = state.messages.len();

    // Act 1：stopping 中再按 Esc
    let accepted = state.note_stopping_key();
    assert!(
        !accepted,
        "O6：stopping 中重复 Esc 不得再派发停止请求（note_stopping_key 必须返回 false）"
    );
    assert_eq!(
        state.messages.len(),
        messages_before,
        "O6：stopping 中重复 Esc 不得追加用户/状态消息"
    );
    assert_eq!(
        state.stopping_turn_id.as_deref(),
        Some("turn-o6"),
        "O6：stopping 中重复 Esc 不得改写被停止的 turn"
    );

    // Act 2：同一终态事件重复投递
    state.apply_stop_terminal("turn-o6", Some("user"));
    let status_after_first = state.status_text.clone();
    let count_after_first = state.messages.len();
    state.apply_stop_terminal("turn-o6", Some("user"));
    assert_eq!(
        state.status_text, status_after_first,
        "O6：重复终态事件不得重复追加状态文案"
    );
    assert_eq!(
        state.messages.len(),
        count_after_first,
        "O6：重复终态事件不得重复追加消息"
    );

    // Act 3：陈旧 turn 的迟到终态不得终止当前流式轮
    let mut current = streaming_state();
    current.streaming_turn_id = Some("turn-current".to_string());
    current.apply_stop_terminal("turn-stale", Some("user"));
    assert!(
        current.is_streaming,
        "O6：陈旧 turn 的终态不得终止当前 turn"
    );
    assert!(
        current.stopping_turn_id.is_none(),
        "O6：陈旧终态不得伪造 stopping 态"
    );
    assert!(
        !current.status_text.contains("已停止"),
        "O6：陈旧终态不得改写当前状态文案，实际 = {:?}",
        current.status_text
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// O7：Ask 内 Esc 走既有取消语义 —— 关闭面板、不提交答案、不继承流式双击停止
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn o7_ask_escape_closes_panel_without_fabricating_answers_or_stop() {
    // Arrange
    let mut state = ask_state(QUESTION_SENTINEL);
    let messages_before = state.messages.len();
    assert_eq!(state.mode, InteractionMode::AskUserQuestion);
    assert!(
        tui::inline_ask_height(&state, 120) > 0,
        "O7 前置条件：Ask 面板必须展开"
    );

    // Act：等价于 Ask 面板内的 Esc 分支（既有 `cancel_ask_user` 收口）
    state.cancel_ask_user();

    // Assert
    assert_eq!(
        state.mode,
        InteractionMode::Normal,
        "O7：Ask 取消后必须关闭面板回到普通态"
    );
    assert!(
        state.ask_user_state.is_none(),
        "O7：Ask 取消后面板状态必须清空"
    );
    assert_eq!(
        tui::inline_ask_height(&state, 120),
        0,
        "O7：Ask 取消后内联面板高度必须为 0"
    );
    assert_eq!(
        state.messages.len(),
        messages_before,
        "O7：Ask 取消不得新增用户/系统消息（不注入取消答案 JSON）"
    );
    assert!(
        !state.status_text.contains("正在停止") && state.stopping_turn_id.is_none(),
        "O7：Ask 内的 Esc 不得继承普通流式双击停止语义，实际 = {:?}",
        state.status_text
    );
    // 空白提交必须被拒绝：面板关闭不是「伪造答案」的替代路径。
    assert!(
        matches!(
            tui::ask_submit_verdict(true, None, "   "),
            AskSubmitVerdict::Reject(_)
        ),
        "O7：空白回答必须被拒绝，不得靠伪造答案蒙混提交"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// O8：取消状态只来自受控停止原因投影，内部 text/error/JSON/Debug 不得进入用户可见面
// ─────────────────────────────────────────────────────────────────────────────

const PAYLOAD_SENTINEL: &str = "INTERNAL_PAYLOAD_SENTINEL_o8";

#[test]
fn o8_cancel_status_never_leaks_internal_payload() {
    // Arrange：非 "user" 的停止原因携带内部 payload 形态哨兵
    let mut state = streaming_state();
    state.apply_stop_requested("turn-o8");

    // Act
    state.apply_stop_terminal("turn-o8", Some(PAYLOAD_SENTINEL));

    // Assert 1：中间/未知原因只能是中性「已停止」，不得回显 payload
    assert!(
        state.status_text.contains("已停止"),
        "O8：取消终态必须给出受控的「已停止」语义，实际 = {:?}",
        state.status_text
    );
    assert!(
        !state.status_text.contains(PAYLOAD_SENTINEL),
        "O8：原始 stop_reason 不得进入用户状态提示，实际 = {:?}",
        state.status_text
    );
    assert!(
        !state.status_text.contains('{') && !state.status_text.contains('}'),
        "O8：取消状态提示不得包含协议 JSON 片段，实际 = {:?}",
        state.status_text
    );
    if let Some((notice, _)) = state.status_notice.as_ref() {
        assert!(
            !notice.contains(PAYLOAD_SENTINEL) && !notice.contains('{'),
            "O8：一次性提示同样不得泄漏内部 payload，实际 = {notice:?}"
        );
    }
    assert!(
        state
            .messages
            .iter()
            .all(|message| !message.content.contains(PAYLOAD_SENTINEL)),
        "O8：内部 payload 不得进入任何用户消息"
    );

    // Assert 2：用户原因投影使用固定文案（受控词表），且已有正文保留
    let mut user_stop = streaming_state();
    user_stop.apply_stop_requested("turn-o8-user");
    user_stop.apply_stop_terminal("turn-o8-user", Some("user"));
    assert_eq!(
        user_stop.status_text, "已停止（用户中断）",
        "O8：用户原因必须投影为固定文案，而不是事件 payload 原文"
    );
    assert_eq!(
        last_assistant(&user_stop),
        "partial answer",
        "O8：状态投影不得改写已渲染正文"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// J1 细化（Protocol Revision 2）：`history_tool_detail` 的确定性单测
//
// 契约（rev2）：非空且非「工具名/显示名占位」的 `description` 优先；占位时只提取结构化
// arguments 中已知的人类可读 `description` 字段；空/占位且无该字段时**省略 detail**，
// 绝不反显整段 JSON；交互 Ask 的结构化 arguments/result 任何时候不得成为摘要；
// 普通工具仅保留**非结构化纯文本** detail 的兼容回退。
// ─────────────────────────────────────────────────────────────────────────────

/// ① description = 工具名占位 + arguments 内含人类可读 `description` ⇒ 取人类文案。
#[test]
fn j1_detail_placeholder_description_takes_human_field_from_arguments() {
    // Arrange：工具名占位（description == name），arguments 是结构化 JSON 且带人类字段
    let arguments = r#"{"path":"package.json","description":"读取项目配置确认版本号"}"#;

    // Act
    let detail = tui::history_tool_detail(
        "Read",
        None,
        None,
        "Read", // 占位：等于工具名，不是用户可读摘要
        Some(arguments),
        Some("{\"name\":\"pony-agent\"}"),
    );

    // Assert
    assert_eq!(
        detail.as_deref(),
        Some("读取项目配置确认版本号"),
        "J1①：占位 description 必须只提取 arguments 中已知的人类可读 description 字段，\
         不得反显整段 JSON 或空手而归，实际 = {detail:?}"
    );
    assert!(
        !detail.as_deref().unwrap_or_default().contains('{'),
        "J1①：提取结果不得是 JSON 片段，实际 = {detail:?}"
    );
}

/// ② description 空 + arguments 为**无 description 字段的 JSON** ⇒ 省略 detail（None）。
#[test]
fn j1_detail_empty_description_with_structured_arguments_yields_none() {
    // Arrange
    let arguments = r#"{"path":"package.json","limit":40}"#;
    let result = r#"{"status":"ok","lines":12}"#;

    // Act
    let detail = tui::history_tool_detail("Read", None, None, "   ", Some(arguments), Some(result));

    // Assert
    assert_eq!(
        detail, None,
        "J1②：description 为空且 arguments 是结构化 JSON（无人类字段）时必须省略 detail，\
         不得把整段 JSON 反显为摘要，实际 = {detail:?}"
    );
}

/// ③ description 空 + arguments 为**纯文本** ⇒ 原样保留（非结构化兼容回退）。
#[test]
fn j1_detail_empty_description_with_plain_text_keeps_text_fallback() {
    // Arrange
    let plain = "命令已执行完成：3 个文件被格式化";

    // Act：arguments 纯文本
    let from_arguments = tui::history_tool_detail("Bash", None, None, "", Some(plain), None);
    // Act：arguments 缺省、result 纯文本（同一回退的另一入口）
    let from_result = tui::history_tool_detail("Bash", None, None, "", None, Some(plain));

    // Assert
    assert_eq!(
        from_arguments.as_deref(),
        Some(plain),
        "J1③：纯文本 arguments 必须原样作为 detail，实际 = {from_arguments:?}"
    );
    assert_eq!(
        from_result.as_deref(),
        Some(plain),
        "J1③：arguments 缺省时纯文本 result 必须原样作为 detail，实际 = {from_result:?}"
    );
}

/// ④ 交互 Ask（复用既有名称分类，大小写不敏感）任意结构化 JSON ⇒ 一律 None。
#[test]
fn j1_detail_interactive_ask_never_summarizes_structured_payload() {
    // Arrange：结构化参数**带**人类 description 字段（最容易被误提取的形态）
    let arguments = r#"{"question":"请选择分支","options":["A","B"],"description":"这里的人类字段也不得显示"}"#;

    // Act + Assert：四个既有类别 + 大小写不敏感 + 结构化 result
    for (name, canonical) in [
        ("Ask", None),
        ("ask_user", None),
        ("builtin:ask", None),
        ("ask", Some("builtin:ask_user")),
        ("Ask", Some("ASK_USER")),
    ] {
        let detail = tui::history_tool_detail(name, None, canonical, "", Some(arguments), Some(arguments));
        assert_eq!(
            detail, None,
            "J1④：交互 Ask（name={name:?}, canonical={canonical:?}）的结构化 arguments/result \
             不得成为消息摘要，实际 = {detail:?}"
        );
    }
    // 占位形态（description == 工具名）+ 结构化 result 同样必须 None
    let placeholder = tui::history_tool_detail("Ask", None, None, "Ask", None, Some(arguments));
    assert_eq!(
        placeholder, None,
        "J1④：占位 description 的交互 Ask 也不得回退到结构化 result，实际 = {placeholder:?}"
    );
    // 契约边界：rev2 的「非结构化纯文本回退」只授予**普通工具**；Ask 在占位/空 description 时
    // 一律省略 detail（更保守），此处不额外放宽为「Ask 纯文本必须可见」。
}

/// ⑤ 非占位 description ⇒ 原样返回（优先于 arguments/result）。
#[test]
fn j1_detail_non_placeholder_description_is_returned_verbatim() {
    // Arrange
    let human = "读取工作区配置并汇总版本信息";
    let arguments = r#"{"path":"package.json","description":"arguments 里的人类字段应被忽略"}"#;

    // Act
    let detail = tui::history_tool_detail(
        "Read",
        Some("读取文件"),
        Some("read_file"),
        human,
        Some(arguments),
        Some("结果文本"),
    );

    // Assert
    assert_eq!(
        detail.as_deref(),
        Some(human),
        "J1⑤：非占位 description 必须原样优先，实际 = {detail:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// C9：TUI 启动轮必须用既有 register/unregister_turn_event_sink 成对接通停止链路
//     （真实可达行为：当前构建的二进制 + 真实伪终端）
// ─────────────────────────────────────────────────────────────────────────────

/// 真实 TUI 的伪终端驱动（python `pty`），只做「按键 + 抓屏 + 取日志」，不做业务判定。
///
/// 输出最后一行 JSON 供 Rust 侧断言；所有等待都是**条件轮询**（无盲 sleep 决策）。
const PTY_DRIVER_PY: &str = r#"
import json, os, pty, re, select, signal, socket, subprocess, sys, tempfile, time

def norm(raw):
    text = re.sub(rb"\x1b\[[0-9;?]*[a-zA-Z]", b" ", bytes(raw))
    text = re.sub(rb"\x1b\][^\x07]*\x07", b" ", text)
    text = re.sub(rb"\x1b[()][A-Za-z0-9]", b" ", text)
    # 全屏 diff 渲染会在字素之间插入光标定位序列；断言前统一去掉空白与盲文点阵占位。
    return re.sub(r"[\s\u2800-\u28ff]", "", text.decode("utf-8", "replace"))

class Pty:
    def __init__(self, bin_path, args, env, cwd):
        master, slave = pty.openpty()
        import fcntl, struct, termios
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 34, 120, 0, 0))
        self.proc = subprocess.Popen([bin_path] + args, stdin=slave, stdout=slave,
                                     stderr=slave, env=env, cwd=cwd, close_fds=True)
        os.close(slave)
        self.master = master
        self.buf = bytearray()

    def pump(self, duration):
        end = time.time() + duration
        while time.time() < end:
            ready, _, _ = select.select([self.master], [], [], 0.05)
            if ready:
                try:
                    data = os.read(self.master, 65536)
                except OSError:
                    return
                if not data:
                    return
                self.buf.extend(data)

    def text(self):
        return norm(self.buf)

    def wait_for(self, needle, timeout):
        target = norm(needle.encode())
        end = time.time() + timeout
        while time.time() < end:
            self.pump(0.1)
            if target in self.text():
                return True
        return False

    def send(self, text):
        os.write(self.master, text.encode())

    def close(self):
        try:
            self.proc.send_signal(signal.SIGKILL)
        except Exception:
            pass
        try:
            self.proc.wait(timeout=5)
        except Exception:
            pass
        try:
            os.close(self.master)
        except Exception:
            pass

def main():
    import argparse
    ap = argparse.ArgumentParser()
    ap.add_argument("--flow", required=True)
    ap.add_argument("--bin", required=True)
    ap.add_argument("--root", required=True)
    ap.add_argument("--port-file", required=True)
    args = ap.parse_args()

    work = tempfile.mkdtemp(prefix="l2at-overlay-")
    iso = os.path.join(work, "iso")
    for sub in ("xdg-config/pony-agent", "xdg-data", "xdg-cache", "home"):
        os.makedirs(os.path.join(iso, sub), exist_ok=True)

    # 隔离：独立 mock 端口 + 独立 HOME/XDG + 关闭真实 OTel/PonySentry 外发。
    sock = socket.socket(); sock.bind(("127.0.0.1", 0)); port = sock.getsockname()[1]; sock.close()
    with open(args.port_file, "w") as fh:
        fh.write(str(port))
    model = {"id": "mock-alpha", "name": "mock-alpha", "model": "mock-alpha", "temperature": 0.2,
             "maxOutputTokens": 64000, "capabilityPreset": "open-ai-chat", "reasoningEffort": None,
             "reasoningBudgetTokens": None, "protocol": "openai-completions",
             "capabilities": {"contextWindowTokens": 256000, "supportsTools": True, "supportsStreaming": True,
                              "supportsImageInput": True, "supportsReasoning": False, "supportsVideoInput": False,
                              "supportsAudioInput": False, "supportsImageOutput": False, "supportsAudioOutput": False}}
    provider = {"id": "fakep", "name": "FakeProvider", "protocol": "openai-completions",
                "base_url": "http://127.0.0.1:%d/v1" % port, "auth_type": "bearer",
                "api_key_env_var": "MOCK_API_KEY", "secret_ref": "", "api_key_value": "mock-key",
                "supported_protocols": ["openai-completions"], "endpoints": [],
                "models": [model], "selected_model_id": "mock-alpha"}
    with open(os.path.join(iso, "xdg-config/pony-agent/providers.json"), "w", encoding="utf-8") as fh:
        json.dump({"providers": [provider], "selected_provider_id": "fakep"}, fh)

    mock_log = os.path.join(work, "mock-requests.jsonl")
    mock = subprocess.Popen([sys.executable, os.path.join(args.root, "tools/tui-audit/mock_llm.py"),
                             "--port", str(port), "--log", mock_log, "--first-token-delay-ms", "1500"],
                            stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    hello = mock.stdout.readline()
    if "listening" not in hello:
        print(json.dumps({"harness_error": "mock_llm.py did not start: %r" % hello}))
        return

    env = {k: v for k, v in os.environ.items()
           if k.lower() not in ("http_proxy", "https_proxy", "all_proxy", "no_proxy", "no_color")}
    runtime_log = os.path.join(work, "tui-runtime.log")
    env.update({
        "XDG_CONFIG_HOME": os.path.join(iso, "xdg-config"),
        "XDG_DATA_HOME": os.path.join(iso, "xdg-data"),
        "XDG_CACHE_HOME": os.path.join(iso, "xdg-cache"),
        "HOME": os.path.join(iso, "home"),
        "PONY_TUI_RUNTIME_LOG": runtime_log,
        "TERM": "xterm-256color",
        "LANG": "C.UTF-8",
        "PONYSENTRY_INGEST_URL": "",
        "PONYSENTRY_CLIENT_TOKEN": "",
        "OTEL_EXPORTER_OTLP_ENDPOINT": "http://127.0.0.1:9/v1/traces",
        "OTEL_TRACES_SAMPLER": "always_off",
        "OTEL_TRACES_SAMPLER_ARG": "0",
        "NO_COLOR": "",
    })

    result = {"work": work, "mock_log": mock_log, "runtime_log": runtime_log}
    if args.flow == "stop":
        tui = Pty(args.bin, [], env, args.root)
        result["ready"] = tui.wait_for("›", 40)
        tui.send("慢速 请给我一个流式回答\r")
        result["streaming"] = tui.wait_for("正在分析", 40)
        tui.pump(0.3)
        mark = len(tui.buf)
        tui.send("\x1b")
        time.sleep(0.12)
        tui.send("\x1b")
        tui.pump(0.4)
        result["stopping_visible"] = "正在停止" in norm(tui.buf[mark:])
        result["stopped_visible"] = tui.wait_for("已停止", 25)
        tui.pump(0.4)
        stream_text = norm(tui.buf)
        result["screen"] = stream_text[-4000:]
        result["sink_unregistered_warning"] = "noturneventsinkregistered" in stream_text
        result["stop_requested_dispatched_warning"] = "turn:stop_requestednotdispatched" in stream_text
        result["stopped_with_reason"] = "已停止（用户中断）" in stream_text
        result["panicked"] = "panickedat" in stream_text or tui.proc.poll() == 101
        tui.close()
        # C9 的第二观测面：run_tui 把结构化 JSON 日志改道 stderr（已 dup2 到本文件），
        # 因此 core 的「未注册 sink」告警只在运行日志里可见，必须回传原文供断言。
        result["runtime_log_exists"] = os.path.exists(runtime_log)
        try:
            with open(runtime_log, "r", encoding="utf-8", errors="replace") as fh:
                result["runtime_log_text"] = fh.read()[-40000:]
        except OSError as error:
            result["runtime_log_text"] = ""
            result["runtime_log_error"] = str(error)
    elif args.flow == "resume":
        seed = Pty(args.bin, [], env, args.root)
        result["seed_ready"] = seed.wait_for("›", 40)
        seed.send("读取 package.json 并总结\r")
        result["seed_answered"] = seed.wait_for("结论", 45)
        seed.pump(1.5)
        seed.send("/quit\r")
        seed.pump(2.0)
        try:
            seed.proc.wait(timeout=6)
        except Exception:
            seed.close()
        resumed = Pty(args.bin, ["resume"], env, args.root)
        result["resume_ready"] = resumed.wait_for("›", 40)
        resumed.pump(1.5)
        # 产品默认隐藏 Agent 过程（ADR 0079 显示偏好：hide_process=true）。
        # J1 校验的是「恢复视图的摘要选择优先级」（description 优先、结构化参数不得反显），
        # 不是折叠默认值，故先按 Ctrl+O 展开过程，再对展开态抓屏断言。
        resumed.send("\x0f")
        resumed.pump(1.5)
        # `norm()` 已剥掉全部空白，故按无空白形态匹配状态提示。
        result["resume_process_expanded"] = "已显示Agent过程" in norm(resumed.buf)
        screen = norm(resumed.buf)
        result["resume_screen"] = screen[-4000:]
        result["resume_history_visible"] = "结论" in screen
        result["resume_panicked"] = "panickedat" in screen
        resumed.close()
    else:
        print(json.dumps({"harness_error": "unknown flow %r" % args.flow}))
        return

    mock.terminate()
    print(json.dumps(result, ensure_ascii=False))

main()
"#;

/// C9 的核心判据（唯一真值文案，来自 `HostControlPlane::stop_turn` 的告警分支）：
/// 未注册 sink 时 core 会写下这条 warn；它是「TUI 本轮没接 sink」的唯一原生证据。
const SINK_UNREGISTERED_WARNING: &str = "no turn event sink registered";

/// 判定「给定的日志文本里是否出现未注册 sink 告警」。
///
/// 抽成函数是为了让**判别力自检**与**实际断言**共用同一实现：任何一侧改宽/改窄都会同时反映。
fn contains_sink_unregistered_warning(text: &str) -> bool {
    text.contains(SINK_UNREGISTERED_WARNING)
}

/// 已知告警样本（与 core 实际产出的 JSON 行同形；用于证明判据不是空断言）。
const KNOWN_SINK_WARNING_SAMPLE: &str = r#"{"level":"warn","message":"stop accepted but no turn event sink registered; turn:stop_requested not dispatched","session_id":"session-sample","stop_reason":"user","trace_id":null}"#;

struct PtyFlow {
    json: serde_json::Value,
    raw_stdout: String,
    raw_stderr: String,
}

impl PtyFlow {
    fn flag(&self, name: &str) -> bool {
        self.json
            .get(name)
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    fn text(&self, name: &str) -> String {
        self.json
            .get(name)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_string()
    }

    /// 驱动自身失败（缺 python3 / mock 起不来 / 二进制无法启动）必须显式报错，
    /// 绝不能被当成「被测行为失败」而伪造红相。
    fn assert_harness_ok(&self, ticket: &str) {
        if let Some(error) = self.json.get("harness_error").and_then(serde_json::Value::as_str) {
            panic!("{ticket} HARNESS ERROR：真实链路驱动失败，不得当作红相断言：{error}");
        }
        assert!(
            !self.json.is_null(),
            "{ticket} HARNESS ERROR：驱动未输出 JSON；stdout={:?} stderr={:?}",
            self.raw_stdout,
            self.raw_stderr
        );
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("repo root")
        .to_path_buf()
}

fn run_pty_flow(flow: &str) -> PtyFlow {
    let driver = std::env::temp_dir().join(format!("l2at-overlay-driver-{flow}.py"));
    std::fs::write(&driver, PTY_DRIVER_PY).expect("write pty driver");
    let port_file = std::env::temp_dir().join(format!("l2at-overlay-port-{flow}.txt"));
    let output = Command::new("python3")
        .arg(&driver)
        .arg("--flow")
        .arg(flow)
        .arg("--bin")
        .arg(env!("CARGO_BIN_EXE_pony"))
        .arg("--root")
        .arg(repo_root())
        .arg("--port-file")
        .arg(&port_file)
        .output()
        .unwrap_or_else(|error| panic!("C9/J1 HARNESS ERROR：无法启动 python3 伪终端驱动：{error}"));
    let raw_stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let raw_stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let json = raw_stdout
        .lines()
        .rev()
        .find_map(|line| serde_json::from_str::<serde_json::Value>(line.trim()).ok())
        .unwrap_or(serde_json::Value::Null);
    PtyFlow {
        json,
        raw_stdout,
        raw_stderr,
    }
}

#[test]
fn c9_each_turn_registers_turn_event_sink_so_stop_lifecycle_is_reachable() {
    // Arrange / Act：真实二进制 + 真实伪终端；流式中双击 Esc。
    let flow = run_pty_flow("stop");
    flow.assert_harness_ok("C9");

    // Assert 0：TUI 必须真的跑起来并进入流式（否则后续判据无意义）
    assert!(
        flow.flag("ready"),
        "C9：真实 TUI 未在 40s 内出现 Composer 提示，屏幕 = {:?}",
        flow.text("screen")
    );
    assert!(
        flow.flag("streaming"),
        "C9：mock 慢速流未开始（屏幕 = {:?}）",
        flow.text("screen")
    );

    // Assert 1：核心 —— 控制面 stop_turn 必须命中**本轮已注册**的 sink。
    // core 在「stop accepted but no turn event sink registered」时会打印
    // `turn:stop_requested not dispatched`；出现即证明 C9 未接线（红相真值）。
    assert!(
        !flow.flag("sink_unregistered_warning") && !flow.flag("stop_requested_dispatched_warning"),
        "C9：TUI 未在每轮 start_turn_stream 前注册 turn event sink，\
         控制面 stop_turn 因此无法派发 turn:stop_requested（core 已打印告警）。屏幕 = {:?}",
        flow.text("screen")
    );

    // Assert 2：受理后只显示「正在停止…」（不是本地伪造终态）
    assert!(
        flow.flag("stopping_visible"),
        "C9：双击 Esc 受理后必须可见「正在停止…」，屏幕 = {:?}",
        flow.text("screen")
    );

    // Assert 3：终态由事件驱动 —— 「已停止（用户中断）」必须可见
    assert!(
        flow.flag("stopped_visible") && flow.flag("stopped_with_reason"),
        "C9：取消终态必须经事件通道显示「已停止（用户中断）」，屏幕 = {:?}",
        flow.text("screen")
    );

    // Assert 4：真实链路不得 panic
    assert!(!flow.flag("panicked"), "C9：真实 TUI 进程不得 panic");

    // Assert 5（第二观测面）：`set_stderr_output(true)` 之后，core 的未注册 sink 告警只写
    //   PONY_TUI_RUNTIME_LOG，不再出现在屏幕/pane。因此必须回读运行日志并断言告警缺席，
    //   否则「删掉 register_turn_event_sink 也会绿」（假绿，已在评审突变探针中复现）。
    let runtime_log = flow.text("runtime_log_text");
    assert!(
        flow.flag("runtime_log_exists") && !runtime_log.is_empty(),
        "C9 判别力自检失败：运行日志未被捕获/为空（{:?}），第二观测面失效，C9 将无法判别 sink 注册回归",
        flow.json.get("runtime_log_error")
    );
    // 判别力自检：同一判据必须命中**已知告警样本**（防止函数被改宽/改空后恒真/恒假）。
    assert!(
        contains_sink_unregistered_warning(KNOWN_SINK_WARNING_SAMPLE),
        "C9 判别力自检失败：已知告警样本未被判据命中，断言形同虚设"
    );
    let offending_log_lines: Vec<&str> = runtime_log
        .lines()
        .filter(|line| contains_sink_unregistered_warning(line))
        .collect();
    assert!(
        offending_log_lines.is_empty(),
        "C9：运行日志出现 core 告警「stop accepted but no turn event sink registered」\
         （TUI 本轮未注册既有 turn event sink；屏幕/pane 已看不到该告警，只能在此观测）。\
         命中行 = {offending_log_lines:?}"
    );
}

#[test]
fn j1_resume_summary_prefers_description_and_hides_structured_json() {
    // Arrange / Act：真实二进制造一轮带工具调用的会话，再 `pony resume` 恢复视图。
    let flow = run_pty_flow("resume");
    flow.assert_harness_ok("J1");

    assert!(
        flow.flag("seed_ready") && flow.flag("seed_answered"),
        "J1 HARNESS：种子会话未完成（ready={} answered={}），屏幕 = {:?}",
        flow.flag("seed_ready"),
        flow.flag("seed_answered"),
        flow.text("resume_screen")
    );
    assert!(
        flow.flag("resume_ready") && flow.flag("resume_history_visible"),
        "J1 HARNESS：`pony resume` 未恢复出历史（ready={} history={}），屏幕 = {:?}",
        flow.flag("resume_ready"),
        flow.flag("resume_history_visible"),
        flow.text("resume_screen")
    );
    assert!(
        flow.flag("resume_process_expanded"),
        "J1 HARNESS：Ctrl+O 未能展开 Agent 过程（ADR 0079 默认折叠），屏幕 = {:?}",
        flow.text("resume_screen")
    );
    assert!(
        !flow.flag("resume_panicked"),
        "J1：恢复视图不得 panic"
    );

    let screen = flow.text("resume_screen");

    // Assert 1：结构化 arguments_text（内部 JSON）不得成为用户可见摘要
    assert!(
        !screen.contains("\"path\"") && !screen.contains("\"lineCount\""),
        "J1：恢复视图把工具的结构化 arguments JSON 反显成了消息摘要，\
         用户可见面不得出现内部协议字段。屏幕 = {screen:?}"
    );

    // Assert 2：非空 description 必须优先显示（用户可读摘要）
    assert!(
        screen.contains("收集路径package.json的上下文"),
        "J1：恢复视图必须优先显示 trace 的面向用户 description，屏幕 = {screen:?}"
    );
}
