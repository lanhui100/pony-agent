//! 交互修复六项验收：Shift+Enter 换行、Ctrl+U 清空、模型优选/默认持久化、
//! 用户块去竖线、输入框可鼠标选择、宽字符选区精确。
//!
//! 手法与 `tui_render_layout.rs` 一致：把 `src/bin/tui.rs` 以模块方式编进测试进程，
//! 直接驱动渲染与纯函数，断言**真实行为**而不是字符串常量。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use pony_agent_core::agent::model_preferences::ModelPreferencesStore;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;
use ratatui::Terminal;

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn screen(state: &mut tui::TuiState, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| tui::ui(f, state)).unwrap();
    terminal.backend().buffer().clone()
}

/// 还原一行真实的终端文本：宽字符右半格是渲染占位空格，复制/搜索时必须跳过，
/// 否则 CJK 会被读成「复 制 要 干 净」。
fn row_text(buf: &ratatui::buffer::Buffer, y: u16) -> String {
    let mut out = String::new();
    let mut skip = 0usize;
    for x in 0..buf.area.width {
        if skip > 0 {
            skip -= 1;
            continue;
        }
        let symbol = buf[(x, y)].symbol();
        let width = tui::str_width(symbol).max(1);
        if width > 1 {
            skip = width - 1;
        }
        out.push_str(symbol);
    }
    out
}

fn line_texts(buf: &ratatui::buffer::Buffer) -> Vec<String> {
    (0..buf.area.height).map(|y| row_text(buf, y)).collect()
}

fn model(id: &str, name: &str) -> tui::ModelItem {
    tui::ModelItem {
        id: id.to_string(),
        name: name.to_string(),
    }
}

// ── #1 Shift+Enter 换行（而非发送）────────────────────────────────────────────

#[test]
fn shift_enter_inserts_newline_and_plain_enter_submits() {
    assert_eq!(
        tui::composer_enter_action(&key(KeyCode::Enter, KeyModifiers::SHIFT)),
        tui::ComposerEnterAction::Newline,
        "Shift+Enter 必须是换行"
    );
    assert_eq!(
        tui::composer_enter_action(&key(KeyCode::Enter, KeyModifiers::NONE)),
        tui::ComposerEnterAction::Submit,
        "裸 Enter 必须仍然是发送"
    );
    assert_eq!(
        tui::composer_control_action(&key(KeyCode::Char('j'), KeyModifiers::CONTROL)),
        Some(tui::ComposerControlAction::InsertNewline),
        "Ctrl+J 与 Shift+Enter 同语义"
    );
}

#[test]
fn keyboard_enhancement_push_is_not_gated_by_terminal_probe() {
    // 回归：旧实现依赖 `supports_keyboard_enhancement()` 阻塞探测，tmux/VS Code 内终端被误判为
    // 不支持 ⇒ 标志从未 push ⇒ Shift+Enter 被降级成裸 Enter（＝发送）。
    for term in [
        "xterm-256color",
        "tmux-256color",
        "screen-256color",
        "alacritty",
        "wezterm",
    ] {
        assert!(
            tui::keyboard_enhancement_should_push_for(Some(term)),
            "{term} 必须 push 键盘增强标志"
        );
    }
    assert!(!tui::keyboard_enhancement_should_push_for(Some("dumb")));
    assert!(!tui::keyboard_enhancement_should_push_for(Some("DUMB")));
    assert!(!tui::keyboard_enhancement_should_push_for(None));
}

// ── #2 Ctrl+U 清空整个输入框（流式期也生效）───────────────────────────────────

#[test]
fn ctrl_u_clears_whole_composer_even_mid_stream() {
    let mut state = tui::TuiState::default();
    state.input_buffer = "草稿 hello world".to_string();
    state.input_cursor = 4;
    state.is_streaming = true;

    let ctrl_u = key(KeyCode::Char('u'), KeyModifiers::CONTROL);
    assert_eq!(
        tui::composer_control_action(&ctrl_u),
        Some(tui::ComposerControlAction::ClearInput)
    );
    // 回归护栏：流式守卫不得吞掉输入框编辑类快捷键。
    assert!(
        !tui::should_skip_while_streaming(&ctrl_u),
        "流式中 Ctrl+U 必须放行"
    );
    // 契约演进（`tui-streaming-input` C1，2026-11-06 Lead 裁决）：旧断言
    // 「流式中普通字符被 `should_skip_while_streaming` 丢弃（避免污染草稿）」作废；
    // 新真值 = `streaming_key_disposition == Forward`（事件循环改由该裁决面决定透传）。
    // 真值源：`.dev-team/contract-matrix-tui-streaming-input.md` §2 权威符号表 / §4 A1、A6。
    assert_eq!(
        tui::streaming_key_disposition(&key(KeyCode::Char('x'), KeyModifiers::NONE), false),
        tui::StreamingKeyDisposition::Forward,
        "C1：流式中可打印字符必须 Forward 到输入框（旧断言『流式中丢弃』已作废）"
    );

    // 复刻主循环顺序：流式守卫 → 应用动作。
    if !(state.is_streaming && tui::should_skip_while_streaming(&ctrl_u)) {
        let action = tui::composer_control_action(&ctrl_u).expect("Ctrl+U 必须有动作");
        state.apply_composer_control(action);
    }
    assert!(state.input_buffer.is_empty(), "Ctrl+U 必须清空整个输入框");
    assert_eq!(state.input_cursor, 0);
}

// ── #4 用户消息前无竖线（复制干净）────────────────────────────────────────────

#[test]
fn user_message_copy_text_is_free_of_track_bar() {
    let mut state = tui::TuiState::default();
    state.add_user_message("复制要干净");
    let buf = screen(&mut state, 100, 26);
    let user_row = (0..buf.area.height)
        .find(|y| row_text(&buf, *y).contains("复制要干净"))
        .expect("用户轮未渲染");
    let text = row_text(&buf, user_row);
    assert!(!text.contains('▌'), "用户轮不得再有行首竖线: {text:?}");

    // 复制路径取的是 rendered_lines_text（OSC52），这里直接断言它同样干净。
    state.ensure_full_text();
    let copy_text = state.rendered_lines_text.join("\n");
    assert!(
        !copy_text.contains('▌'),
        "复制文本不得带竖线: {copy_text:?}"
    );
    assert!(
        copy_text.contains("复制要干净"),
        "复制文本丢失正文: {copy_text:?}"
    );
}

// ── #5 输入框内文本可鼠标选择复制 ─────────────────────────────────────────────

#[test]
fn composer_text_is_mouse_selectable_and_extracts_exact_run() {
    let mut state = tui::TuiState::default();
    state.input_buffer = "hello world".to_string();
    state.input_cursor = 11;
    let _ = screen(&mut state, 100, 26);
    let rect = state.last_input_rect.expect("输入框文本区矩形必须记录");
    assert_eq!(
        state.last_input_lines.first().map(String::as_str),
        Some("hello world"),
        "输入框纯文本快照必须与缓冲一致"
    );

    // 点在 'w'（正文区第 6 个显示列）。
    let anchor = tui::selection_point_at(&state, rect.x + 6, rect.y).expect("输入框必须可命中");
    assert!(
        anchor.line >= tui::INPUT_LINE_BASE,
        "输入框选区必须落在 INPUT_LINE_BASE 坐标空间: {anchor:?}"
    );
    assert_eq!(anchor.col, 6, "列号必须按显示列计");

    let head = tui::TextCoord {
        line: anchor.line,
        col: anchor.col + 4,
    };
    let copied = tui::extract_selection_across(
        &state.rendered_lines_text,
        &state.last_input_lines,
        anchor,
        head,
    );
    assert_eq!(copied, "world", "输入框选区复制内容必须精确");
}

// ── #6 宽字符 / 端点 / 偏移的选区精度 ────────────────────────────────────────

#[test]
fn cjk_selection_maps_display_columns_not_char_indexes() {
    let lines = vec!["中文abc".to_string()];
    let coord = |col: usize| tui::TextCoord { line: 0, col };

    // 「文」占显示列 2..=3；点在右半格也整字选中（旧实现每 char 只 +1 列 ⇒ 左漂）。
    assert_eq!(
        tui::extract_selected_by_display_cols(&lines, coord(2), coord(3)),
        "文"
    );
    assert_eq!(
        tui::extract_selected_by_display_cols(&lines, coord(0), coord(3)),
        "中文"
    );
    // 'a' 在显示列 4：选区止于列 3 时不得把它带上。
    assert_eq!(
        tui::extract_selected_by_display_cols(&lines, coord(4), coord(4)),
        "a"
    );
    // 端点反序（从右往左拖）结果必须一致。
    assert_eq!(
        tui::extract_selected_by_display_cols(&lines, coord(3), coord(2)),
        "文"
    );
}

#[test]
fn mouse_hit_test_and_highlight_agree_on_cjk_columns() {
    let mut state = tui::TuiState::default();
    state.add_user_message("中文测试");
    let mut buf = screen(&mut state, 100, 26);
    let chat = state.last_chat_rect.expect("正文矩形必须记录");
    let row = (0..buf.area.height)
        .find(|y| row_text(&buf, *y).contains("中文测试"))
        .expect("用户轮未渲染");

    // 用户块左侧留白 2 列（RAIL）：正文首字「中」从显示列 2 起、占 2 列。
    let anchor = tui::selection_point_at(&state, chat.x + 2, row).expect("正文必须可命中");
    assert_eq!(anchor.col, 2, "命中列必须与渲染列一致（含留白）");

    // 精确性核心：点在「中」的右半格（chat.x+3）必须归约到该字起点列 2，
    // 而不是落到下一个字「文」的起点列 4 —— 否则选区边界会切在字中间。
    let right_half = tui::selection_point_at(&state, chat.x + 3, row).expect("右半格必须可命中");
    assert_eq!(right_half.col, 2, "宽字符右半格必须吸附到字形起点");
    let next_lead = tui::selection_point_at(&state, chat.x + 4, row).expect("下一字必须可命中");
    assert_eq!(next_lead.col, 4, "下一个字形起点列不得漂移");

    // 真实拖拽形态：从「中」内部拖到「文」首列 ⇒ 整字语义只含「中文」。
    state.mouse_selection.anchor = Some(right_half);
    state.mouse_selection.head = Some(next_lead);
    state.mouse_selection.active = true;
    buf = screen(&mut state, 100, 26);

    let reversed = |x: u16| buf[(x, row)].modifier.contains(Modifier::REVERSED);
    assert!(reversed(chat.x + 2), "选中字「中」必须高亮");
    assert!(reversed(chat.x + 4), "选中字「文」必须高亮");
    assert!(
        !reversed(chat.x + 6),
        "选区止于「文」，第三个字不得被带上: {:?}",
        row_text(&buf, row)
    );

    state.ensure_full_text();
    let copied = tui::extract_selection_across(
        &state.rendered_lines_text,
        &state.last_input_lines,
        right_half,
        next_lead,
    );
    assert_eq!(copied, "中文", "高亮范围与复制范围必须一致");
}

// ── #3 模型面板：Ctrl+D 默认 / Ctrl+F 优选 / 优选栏前置 ──────────────────────

#[test]
fn model_panel_shortcuts_persist_default_and_preferred_to_disk() {
    let dir = std::env::temp_dir().join(format!("pony-six-fixes-{}", std::process::id()));
    let path = dir.join("model-preferences.json");
    let store = ModelPreferencesStore::with_path(&path);

    let mut state = tui::TuiState::default();
    state.model_list = vec![model("alpha", "Model Alpha"), model("beta", "Model Beta")];
    state.selected_model_index = 1;

    // Ctrl+D：设默认 + 落盘 + 同步 state
    assert!(tui::apply_model_panel_shortcut(
        &mut state,
        &key(KeyCode::Char('d'), KeyModifiers::CONTROL),
        &store
    ));
    assert_eq!(store.load().default_model.as_deref(), Some("beta"));
    assert_eq!(state.model_preferences.default_model.as_deref(), Some("beta"));
    assert_eq!(state.model, "beta", "设为默认同时应切换当前模型");

    // Ctrl+F：切换优选 + 落盘；再按一次取消（幂等对）
    assert!(tui::apply_model_panel_shortcut(
        &mut state,
        &key(KeyCode::Char('f'), KeyModifiers::CONTROL),
        &store
    ));
    assert_eq!(store.load().preferred, vec!["beta".to_string()]);
    assert!(state.model_preferences.is_preferred("beta"));
    assert!(tui::apply_model_panel_shortcut(
        &mut state,
        &key(KeyCode::Char('f'), KeyModifiers::CONTROL),
        &store
    ));
    assert!(store.load().preferred.is_empty());

    // 裸 d / f 不是面板快捷键（不能被吞掉）。
    assert!(!tui::apply_model_panel_shortcut(
        &mut state,
        &key(KeyCode::Char('d'), KeyModifiers::NONE),
        &store
    ));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn model_panel_lists_preferred_section_before_full_list() {
    let mut state = tui::TuiState::default();
    state.add_user_message("x");
    state.mode = tui::InteractionMode::SelectingModel;
    state.model_list = vec![model("alpha", "Model Alpha"), model("beta", "Model Beta")];
    state.model_preferences.preferred = vec!["beta".to_string()];
    state.model_preferences.default_model = Some("alpha".to_string());

    let buf = screen(&mut state, 120, 40);
    let lines = line_texts(&buf);
    let pref_idx = lines
        .iter()
        .position(|line| line.contains("优选"))
        .expect("面板缺少「优选」栏");
    let all_idx = lines
        .iter()
        .position(|line| line.contains("全部模型"))
        .expect("面板缺少「全部模型」栏");
    let beta_idx = lines
        .iter()
        .position(|line| line.contains("Model Beta"))
        .expect("优选模型未出现在面板");

    assert!(pref_idx < all_idx, "「优选」栏必须排在模型列表之前");
    assert!(
        beta_idx > pref_idx && beta_idx < all_idx,
        "优选模型必须落在「优选」栏内（pref={pref_idx}, beta={beta_idx}, all={all_idx}）"
    );
    assert!(
        lines
            .iter()
            .any(|line| line.contains("Model Alpha") && line.contains("默认")),
        "Ctrl+D 设定的默认模型必须有「默认」徽标: {lines:?}"
    );
}


