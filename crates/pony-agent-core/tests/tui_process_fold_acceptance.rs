//! 独立验收测试：TUI 右侧滚动条 + 思考折叠 + `/settings` 面板 + Agent 过程隐藏。
//!
//! 契约真值源：`.dev-team/contract-matrix-tui-process-fold-scroll-settings.md`（C1–C12）。
//! 本文件是**唯一**验收载体；契约冻结后只读，不得为让测试变绿而放宽断言。
//!
//! 范式来源：`crates/pony-agent-core/tests/tui_thinking_bottom_gap.rs`
//! （`#[path = "../src/bin/tui.rs"] mod tui;` + ratatui `TestBackend` + `row_text`/`all_text`）。
//!
//! 断言分层（全部确定性、无 sleep、无真实终端）：
//! - 纯函数层：`scrollbar_thumb` / `should_hide_message` / `thinking_fold_label`；
//! - 渲染层：`ui()` 画进 `TestBackend` 后逐格断言（滚动条列 / 折叠行 / 隐藏后不可见）；
//! - 状态机层：`/settings` 的打开/选择/切换/关闭与按键吸收（`handle_settings_key`）；
//! - 持久化层：`TuiPreferencesStore` 的默认值、缺失/损坏/字段缺失回落。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use crossterm::event::KeyCode;
use pony_agent_core::agent::tui_preferences::{TuiPreferences, TuiPreferencesStore};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::Terminal;
use std::path::PathBuf;

// ── 渲染夹具 ────────────────────────────────────────────────────────────────

/// 单帧渲染：把 `ui()` 画进 TestBackend 并取回缓冲。
fn screen(state: &mut tui::TuiState, width: u16, height: u16) -> Buffer {
    let mut terminal =
        Terminal::new(TestBackend::new(width, height)).expect("A-harness：TestBackend 必须可创建");
    terminal
        .draw(|f| tui::ui(f, state))
        .expect("A-harness：ui() 渲染不得失败");
    terminal.backend().buffer().clone()
}

/// 多帧渲染直到自动贴底收敛（`auto_scroll` 是阻尼跟随，单帧最多推进 3 行）。
///
/// 显式条件轮询（无 sleep、无盲猜）：每帧后检查 `scroll_offset >= max_scroll_offset`，
/// 达到即停，上限 400 帧防死循环。
fn screen_until_settled(state: &mut tui::TuiState, width: u16, height: u16) -> Buffer {
    let mut terminal =
        Terminal::new(TestBackend::new(width, height)).expect("A-harness：TestBackend 必须可创建");
    let mut last = terminal.backend().buffer().clone();
    for _ in 0..400 {
        terminal
            .draw(|f| tui::ui(f, state))
            .expect("A-harness：ui() 渲染不得失败");
        last = terminal.backend().buffer().clone();
        if state.scroll_offset >= state.max_scroll_offset {
            break;
        }
    }
    last
}

fn row_text(buf: &Buffer, y: u16) -> String {
    let mut out = String::new();
    for x in 0..buf.area.width {
        out.push_str(buf[(x, y)].symbol());
    }
    out
}

fn all_text(buf: &Buffer) -> String {
    (0..buf.area.height)
        .map(|y| row_text(buf, y))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 宽字符右侧占位 cell 是空白，CJK 断言前先去空白再比。
fn squeeze(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    squeeze(haystack).matches(&squeeze(needle)).count()
}

fn msg(role: tui::MessageRole, content: &str) -> tui::ChatMessage {
    tui::ChatMessage {
        role,
        content: content.to_string(),
        timestamp_ms: 0,
    }
}

/// 造 40 条必然溢出 30 行视口的长用户消息（不靠「内容不够长」碰巧成立）。
fn overflow_state() -> tui::TuiState {
    let mut state = tui::TuiState::default();
    for i in 0..40 {
        state.add_user_message(&format!(
            "第{i:02}问：这是一段足够长的用户消息，用来把消息区撑到溢出视口，保证贴底时最后一条内容仍然完整可见。"
        ));
    }
    state
}

/// 单条短消息 ⇒ 内容远小于视口（不溢出）。
fn non_overflow_state() -> tui::TuiState {
    let mut state = tui::TuiState::default();
    state.add_user_message("短消息");
    state
}

/// 过程三件套 + 一轮正文的会话（隐藏/可见对照共用同一夹具）。
fn process_state(hide_process: bool) -> tui::TuiState {
    let mut state = tui::TuiState::default();
    state.set_hide_process(hide_process);
    state.add_user_message("帮我看下仓库状态");
    state.messages.push(msg(
        tui::MessageRole::Thinking,
        "先核对工作区改动\n再看分支差异\n最后给出结论",
    ));
    state.add_tool_event(true, "shell · git status --short");
    state.messages.push(msg(tui::MessageRole::Assistant, "工作区干净。"));
    state
}

fn last_col_cells(buf: &Buffer, y_from: u16, rows: u16) -> Vec<char> {
    let x = buf.area.width - 1;
    (y_from..y_from.saturating_add(rows))
        .filter(|y| *y < buf.area.height)
        .map(|y| buf[(x, y)].symbol().chars().next().unwrap_or(' '))
        .collect()
}

fn temp_prefs_path(name: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!(
            "pony-tui-fold-{}-{name}",
            std::process::id()
        ))
        .join("tui-preferences.json")
}

fn cleanup(path: &PathBuf) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
}

// ── A1：C1 滚动条几何纯函数（表驱动）────────────────────────────────────────

#[test]
fn a1_scrollbar_thumb_geometry() {
    // ① 未溢出 / 空视口 ⇒ None（不绘制）。
    assert_eq!(tui::scrollbar_thumb(10, 10, 0), None, "C1：total == visible 必须不绘制");
    assert_eq!(tui::scrollbar_thumb(3, 10, 0), None, "C1：total < visible 必须不绘制");
    assert_eq!(tui::scrollbar_thumb(50, 0, 0), None, "C1：visible == 0 必须不绘制");

    // ② 溢出：滑块长度按可见比例取整且至少 1 行，轨道内右侧留出剩余轨道。
    let (start, len) = tui::scrollbar_thumb(200, 20, 0).expect("C1：200 > 20 必须绘制");
    assert_eq!((start, len), (0, 2), "C1：顶端 thumb_len = round(20*20/200) = 2，起点 0");

    // ③ 末帧恰好落底（不得出现「永远差一行」）。
    let (start, len) = tui::scrollbar_thumb(200, 20, 180).expect("C1：溢出必须绘制");
    assert_eq!(
        (start as usize) + (len as usize),
        20,
        "C1：offset = max_offset 时滑块必须贴底，实际 start={start} len={len}"
    );

    // ④ offset 越界饱和钳制：start + len <= visible。
    let (start, len) = tui::scrollbar_thumb(200, 20, 99_999).expect("C1：溢出必须绘制");
    assert!(
        (start as usize) + (len as usize) <= 20,
        "C1：越界 offset 必须饱和，实际 start={start} len={len}"
    );

    // ⑤ 极端比例：total 略大于 visible ⇒ 至少 1 行滑块，且不越界。
    let (start, len) = tui::scrollbar_thumb(21, 20, 1).expect("C1：21 > 20 必须绘制");
    assert!(len >= 1, "C1：滑块长度下限为 1 行，实际 len={len}");
    assert!(
        (start as usize) + (len as usize) <= 20,
        "C1：滑块必须留在轨道内，实际 start={start} len={len}"
    );
}

// ── A2：C2 渲染位置（最右列 / 只覆盖消息区 / 不侵占正文列）──────────────────

#[test]
fn a2_scrollbar_drawn_on_rightmost_column() {
    let mut state = overflow_state();
    let buf = screen_until_settled(&mut state, 80, 24);
    let chat = state
        .last_chat_rect
        .expect("A2 harness：ui() 必须回写 last_chat_rect");
    assert!(
        state.max_scroll_offset > 0,
        "A2 harness：40 条长消息在 80x24 下必须溢出（否则样本不成立）"
    );

    // 不侵占正文列：正文区右边界不得压到最右列。
    assert!(
        chat.x + chat.width <= buf.area.width - 1,
        "C2：正文区必须为最右列让位，chat.x={} chat.width={} 终端宽={}",
        chat.x,
        chat.width,
        buf.area.width
    );

    let cells = last_col_cells(&buf, chat.y, chat.height);
    assert_eq!(
        cells.len(),
        chat.height as usize,
        "C2：滚动条纵向必须恰好覆盖消息区高度"
    );
    assert!(
        cells.iter().all(|c| *c == '│' || *c == '█'),
        "C2：溢出时最右列每个消息区行必须是轨道 `│` 或滑块 `█`，实际 = {cells:?}"
    );
    assert!(
        cells.contains(&'█'),
        "C2：溢出时最右列必须出现滑块 `█`，实际 = {cells:?}"
    );

    // 纵向不外溢：消息区之外的相邻行不得被滚动条污染。
    let above = last_col_cells(&buf, chat.y.saturating_sub(1), 1);
    if chat.y > 0 {
        assert_ne!(
            above[0], '█',
            "C2：滚动条不得绘制到消息区上方（chat.y-1 出现滑块）"
        );
    }
}

// ── A3：C3 不溢出 / 空态不绘制 ──────────────────────────────────────────────

#[test]
fn a3_no_scrollbar_when_content_fits() {
    // ① 有内容但不溢出。
    let mut state = non_overflow_state();
    let buf = screen_until_settled(&mut state, 80, 24);
    assert_eq!(state.max_scroll_offset, 0, "A3 harness：短消息不得产生可滚动量");
    let chat = state
        .last_chat_rect
        .expect("A3 harness：ui() 必须回写 last_chat_rect");
    let cells = last_col_cells(&buf, chat.y, chat.height);
    assert!(
        cells.iter().all(|c| *c == ' '),
        "C3：未溢出时最右列必须全为背景（无 `│`/`█`），实际 = {cells:?}"
    );

    // ② 空态。
    let mut empty = tui::TuiState::default();
    let buf = screen(&mut empty, 80, 24);
    let chat = empty
        .last_chat_rect
        .expect("A3 harness：空态 ui() 也必须回写 last_chat_rect");
    let cells = last_col_cells(&buf, chat.y, chat.height);
    assert!(
        cells.iter().all(|c| *c == ' '),
        "C3：空态最右列必须全为背景（无滚动条），实际 = {cells:?}"
    );
}

// ── A4：C4 思考折叠 = 单行摘要 + 行数 N ─────────────────────────────────────

#[test]
fn a4_thinking_fold_label_and_single_line() {
    let content = "第一行\n第二行\n第三行";
    assert_eq!(
        tui::thinking_fold_label(content),
        "✻ 思考 · 3 行 · 已折叠",
        "C4：摘要文案必须是 `✻ 思考 · N 行 · 已折叠`，N = lines().count()"
    );
    assert_eq!(
        tui::thinking_fold_label(""),
        "✻ 思考 · 0 行 · 已折叠",
        "C4：空内容 ⇒ N = 0（不得 panic、不得写成 1）"
    );
    assert_eq!(
        tui::thinking_fold_label("只有一行"),
        "✻ 思考 · 1 行 · 已折叠",
        "C4：单行内容 ⇒ N = 1"
    );

    // 折叠态块：1 行摘要 + 1 行尾部呼吸（C6/C7 轮间距不变），正文任何一行都不得出现。
    let lines = tui::message_lines_for_with_flags(
        &msg(tui::MessageRole::Thinking, content),
        60,
        false,
        true,
    );
    let rendered: Vec<String> = lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect();
    let non_empty: Vec<&String> = rendered.iter().filter(|line| !line.trim().is_empty()).collect();
    assert_eq!(
        non_empty.len(),
        1,
        "C4：折叠态思考块只允许 1 行非空摘要，实际 = {rendered:?}"
    );
    assert!(
        squeeze(non_empty[0]).contains(&squeeze("✻ 思考 · 3 行 · 已折叠")),
        "C4：折叠行必须是摘要文案，实际 = {:?}",
        non_empty[0]
    );
    let joined = squeeze(&rendered.join("\n"));
    for body in ["第一行", "第二行", "第三行"] {
        assert!(
            !joined.contains(&squeeze(body)),
            "C4：折叠态不得泄漏思考正文 `{body}`，实际 = {joined}"
        );
    }
}

// ── A5：C5 展开态逐行渲染（回归守卫）────────────────────────────────────────

#[test]
fn a5_thinking_expanded_keeps_legacy_behavior() {
    let message = msg(
        tui::MessageRole::Thinking,
        "第一行\n第二行\n第三行",
    );
    let expanded = tui::message_lines_for_with_flags(&message, 60, false, false);
    let legacy = tui::message_lines_for(&message, 60);
    let text: String = expanded
        .iter()
        .flat_map(|line| line.spans.iter())
        .map(|span| span.content.as_ref())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        expanded.len() > 2,
        "C5：展开态必须逐行渲染（> 折叠态的 2 行），实际 = {} 行",
        expanded.len()
    );
    for body in ["第一行", "第二行", "第三行"] {
        assert!(
            squeeze(&text).contains(&squeeze(body)),
            "C5：展开态必须逐行可见 `{body}`，实际 = {text}"
        );
    }
    assert_eq!(
        expanded.len(),
        legacy.len(),
        "C5/契约 §2.2：公共入口 `message_lines_for` 必须保持旧行为（展开 + 可见）"
    );
}

// ── A6：C6 命令注册与模式迁移 ───────────────────────────────────────────────

#[test]
fn a6_settings_command_registered() {
    assert!(
        tui::is_panel_command("/settings"),
        "C6：`/settings` 必须是面板类命令（流式期可打开、不排队）"
    );
    assert!(
        tui::is_panel_command("  /settings  "),
        "C6：命令判定必须 trim 前后空白"
    );
    assert!(
        tui::AVAILABLE_COMMANDS
            .iter()
            .any(|(cmd, desc)| *cmd == "/settings" && *desc == "设置 TUI 显示与交互偏好"),
        "C6：命令表必须含 (\"/settings\", \"设置 TUI 显示与交互偏好\")，实际 = {:?}",
        tui::AVAILABLE_COMMANDS
            .iter()
            .filter(|(cmd, _)| cmd.starts_with("/s"))
            .collect::<Vec<_>>()
    );

    let mut state = tui::TuiState::default();
    assert_eq!(state.mode, tui::InteractionMode::Normal, "C6 harness：初始态必须是 Normal");
    state.open_settings();
    assert_eq!(
        state.mode,
        tui::InteractionMode::Settings,
        "C6：open_settings 必须进入 InteractionMode::Settings"
    );
    state.close_settings();
    assert_eq!(
        state.mode,
        tui::InteractionMode::Normal,
        "C7：close_settings 必须回到 Normal"
    );
}

// ── A7：C7 面板行内容与渲染 ─────────────────────────────────────────────────

#[test]
fn a7_settings_panel_rows_and_render() {
    let mut state = tui::TuiState::default();
    let rows = state.settings_rows();
    let keys: Vec<&str> = rows.iter().map(|row| row.0).collect();
    assert_eq!(
        keys,
        vec!["hide_process", "thinking_collapsed"],
        "C7：设置项顺序必须固定为 hide_process → thinking_collapsed，实际 = {keys:?}"
    );
    assert_eq!(
        rows[0].2, state.hide_process,
        "C7：行内开关状态必须直读同一字段（hide_process）"
    );
    assert_eq!(
        rows[1].2, state.thinking_collapsed,
        "C7：行内开关状态必须直读同一字段（thinking_collapsed）"
    );

    state.open_settings();
    let buf = screen(&mut state, 100, 30);
    let text = all_text(&buf);
    for needle in [
        "设置",
        "↑↓ 选择",
        "隐藏 Agent 过程",
        "折叠思考",
        "轮次间只保留状态消息",
        "思考块折叠为一行摘要",
    ] {
        assert!(
            squeeze(&text).contains(&squeeze(needle)),
            "C7：面板必须渲染 `{needle}`，实际屏幕 = {text}"
        );
    }
    // 默认（TuiState::default 基线）两项均为关闭 ⇒ 两个 `[关]`。
    assert_eq!(
        count_occurrences(&text, "[关]"),
        2,
        "C7：基线下两项均为关闭，必须渲染 2 个 `[关]`，实际屏幕 = {text}"
    );

    state.set_hide_process(true);
    state.set_thinking_collapsed(true);
    let buf = screen(&mut state, 100, 30);
    let text = all_text(&buf);
    assert_eq!(
        count_occurrences(&text, "[开]"),
        2,
        "C7：两项开启后必须渲染 2 个 `[开]`，实际屏幕 = {text}"
    );
}

// ── A8：C7/C8 选择、切换、关闭（含面板按键吸收）─────────────────────────────

#[test]
fn a8_settings_key_state_machine() {
    let mut state = tui::TuiState::default();
    state.open_settings();
    state.input_buffer = "草稿".to_string();
    state.input_cursor = 2;
    let (buf_before, cursor_before) = (state.input_buffer.clone(), state.input_cursor);

    // ① ↑↓ 选择首尾不环绕。
    state.move_settings_selection(-1);
    assert_eq!(state.settings_selected_index, 0, "C7：首项再上移必须夹紧在 0");
    state.move_settings_selection(1);
    assert_eq!(state.settings_selected_index, 1, "C7：下移必须前进一项");
    state.move_settings_selection(1);
    assert_eq!(state.settings_selected_index, 1, "C7：末项再下移必须夹紧");

    // ② Space/Enter 切换选中项，返回 (key, 新值)，且两项互不串扰。
    assert_eq!(
        state.handle_settings_key(KeyCode::Char(' ')),
        Some(("thinking_collapsed", true)),
        "C7/C8：Space 必须切换选中项并回报 (key, 新值)"
    );
    assert!(state.thinking_collapsed, "C8：切换必须即时写入同一字段");
    assert!(!state.hide_process, "C8：切换 thinking_collapsed 不得串扰 hide_process");
    assert!(
        state
            .status_notice
            .as_ref()
            .map(|(text, _)| text.contains("折叠思考：已开启"))
            .unwrap_or(false),
        "C7：切换后必须给出一次性状态提示，实际 = {:?}",
        state.status_notice
    );

    state.settings_selected_index = 0;
    assert_eq!(
        state.handle_settings_key(KeyCode::Enter),
        Some(("hide_process", true)),
        "C7/C8：Enter 必须切换选中项并回报 (key, 新值)"
    );
    assert!(state.hide_process, "C8：hide_process 必须即时生效");

    // ③ 面板态吸收所有按键：字符键绝不漏进 Composer（含 j/k 这类选择键）。
    for code in [
        KeyCode::Char('x'),
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Char(' '),
        KeyCode::Enter,
        KeyCode::Left,
        KeyCode::Backspace,
    ] {
        state.handle_settings_key(code);
        assert_eq!(
            (state.input_buffer.as_str(), state.input_cursor),
            (buf_before.as_str(), cursor_before),
            "C7：面板态按键 {code:?} 不得漏进输入框（当前缓冲 = {:?}）",
            state.input_buffer
        );
        assert_eq!(
            state.mode,
            tui::InteractionMode::Settings,
            "C7：普通按键不得关闭面板（{code:?}）"
        );
    }

    // ④ Esc 关闭。
    state.handle_settings_key(KeyCode::Esc);
    assert_eq!(
        state.mode,
        tui::InteractionMode::Normal,
        "C7：Esc 必须关闭面板并回到 Normal"
    );

    // ⑤ 状态机层不写盘（C8：落盘只由事件循环负责）：临时偏好文件必须原封不动。
    let path = temp_prefs_path("state-machine");
    let store = TuiPreferencesStore::with_path(&path);
    let saved = TuiPreferences {
        hide_process: false,
        thinking_collapsed: false,
    };
    store.save(&saved).expect("A8 harness：临时偏好必须可写");
    state.open_settings();
    state.settings_selected_index = 0;
    state.handle_settings_key(KeyCode::Char(' '));
    assert_eq!(
        TuiPreferencesStore::with_path(&path).load(),
        saved,
        "C8：状态机层切换不得落盘（写盘必须只发生在事件循环）"
    );
    cleanup(&path);
}

// ── A9：C8/C11 偏好持久化（默认值 / 缺失 / 损坏 / 字段缺失）────────────────

#[test]
fn a9_preferences_store_defaults_and_fallbacks() {
    let default = TuiPreferences::default();
    assert!(default.hide_process, "C11：产品默认必须隐藏 Agent 过程");
    assert!(default.thinking_collapsed, "C11：产品默认必须折叠思考");

    // ① 缺失 ⇒ 默认。
    let path = temp_prefs_path("missing");
    assert_eq!(
        TuiPreferencesStore::with_path(&path).load(),
        TuiPreferences::default(),
        "C11：偏好文件缺失必须回落默认值（绝不阻塞启动）"
    );
    cleanup(&path);

    // ② 损坏 ⇒ 默认。
    let path = temp_prefs_path("corrupted");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{not json").unwrap();
    assert_eq!(
        TuiPreferencesStore::with_path(&path).load(),
        TuiPreferences::default(),
        "C11：偏好文件损坏必须回落默认值"
    );
    cleanup(&path);

    // ③ 字段缺失 ⇒ 逐字段回落（向前兼容旧文件）。
    let path = temp_prefs_path("partial");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, r#"{"hideProcess": false}"#).unwrap();
    let loaded = TuiPreferencesStore::with_path(&path).load();
    assert!(!loaded.hide_process, "C11：显式字段必须生效");
    assert!(
        loaded.thinking_collapsed,
        "C11：缺失字段必须回落默认值 true（不得因缺字段而变成 false）"
    );
    cleanup(&path);

    // ④ 往返一致（原子写：save → load）。
    let path = temp_prefs_path("roundtrip");
    let store = TuiPreferencesStore::with_path(&path);
    let prefs = TuiPreferences {
        hide_process: false,
        thinking_collapsed: true,
    };
    store.save(&prefs).expect("A9 harness：偏好必须可落盘");
    assert_eq!(
        TuiPreferencesStore::with_path(&path).load(),
        prefs,
        "C8：save → load 必须往返一致"
    );
    cleanup(&path);
}

// ── A10：C9 隐藏判据（纯函数 + 渲染块行数为 0）──────────────────────────────

#[test]
fn a10_hide_process_covers_thinking_and_tools() {
    for role in [
        tui::MessageRole::Thinking,
        tui::MessageRole::ToolCall,
        tui::MessageRole::ToolResult,
    ] {
        assert!(
            tui::should_hide_message(&role, true),
            "C9：hide_process = true 时 {role:?} 必须判定为隐藏"
        );
        let lines = tui::message_lines_for_with_flags(&msg(role.clone(), "过程内容"), 60, true, true);
        assert!(
            lines.is_empty(),
            "C9：{role:?} 隐藏态渲染块行数必须为 0，实际 = {} 行",
            lines.len()
        );
    }
}

// ── A11：C9 非过程消息不受影响（隐藏开 / 关双向）────────────────────────────

#[test]
fn a11_hide_process_leaves_dialogue_visible() {
    for role in [
        tui::MessageRole::User,
        tui::MessageRole::Assistant,
        tui::MessageRole::System,
    ] {
        let hidden = tui::message_lines_for_with_flags(&msg(role.clone(), "正文内容"), 60, true, true);
        assert!(
            !hidden.is_empty(),
            "C9：hide_process = true 时 {role:?} 必须仍然可见"
        );
        assert!(
            !tui::should_hide_message(&role, true),
            "C9：{role:?} 不属于过程消息，判据必须为 false"
        );
        assert!(
            !tui::should_hide_message(&role, false),
            "C9：hide_process = false 时 {role:?} 判据必须为 false"
        );
    }

    // 关闭隐藏 ⇒ 过程三件套逐条可见（不得「关也关不掉」）。
    for role in [
        tui::MessageRole::Thinking,
        tui::MessageRole::ToolCall,
        tui::MessageRole::ToolResult,
    ] {
        assert!(
            !tui::should_hide_message(&role, false),
            "C9：hide_process = false 时 {role:?} 不得被隐藏"
        );
        let lines = tui::message_lines_for_with_flags(&msg(role.clone(), "过程内容"), 60, false, true);
        assert!(
            !lines.is_empty(),
            "C9：hide_process = false 时 {role:?} 必须逐条可见"
        );
    }
}

// ── A12：C10 隐藏态状态行显示当前工具信息 ───────────────────────────────────

#[test]
fn a12_hidden_status_line_shows_current_tool() {
    let mut state = process_state(true);
    state.is_streaming = true;
    state.transition_runtime_mode(tui::RuntimeMode::ExecutingTool("shell".to_string()));
    state.add_tool_event(true, "shell · git status --short");

    let status = tui::hidden_tool_status_text(&state, 60)
        .expect("C10：工具执行期必须给出当前工具信息");
    assert!(
        status.starts_with("执行 · "),
        "C10：文案必须以 `执行 · ` 开头，实际 = {status}"
    );
    assert!(
        status.contains("shell") && status.contains("git status --short"),
        "C10：文案必须含工具名与参数摘要，实际 = {status}"
    );
    assert!(
        tui::hidden_tool_status_text(&process_state(true), 60).is_none(),
        "C10：非工具执行期必须回落既有状态行（None），不得伪造工具信息"
    );

    let buf = screen(&mut state, 100, 30);
    let text = all_text(&buf);
    assert!(
        squeeze(&text).contains(&squeeze("执行 · shell · git status --short")),
        "C10：隐藏态屏幕上必须看到状态行里的当前工具信息，实际屏幕 = {text}"
    );
    assert_eq!(
        count_occurrences(&text, "git status --short"),
        1,
        "C10：隐藏态工具信息只允许出现在状态行（工具行本身必须隐藏），实际屏幕 = {text}"
    );
    for glyph in ["◦", "✓", "✕"] {
        assert!(
            !text.contains(glyph),
            "C10：隐藏态不得渲染工具行字形 `{glyph}`，实际屏幕 = {text}"
        );
    }
    // 正文与用户消息不受影响。
    assert!(
        squeeze(&text).contains(&squeeze("工作区干净")),
        "C10：隐藏过程不得连带隐藏助手正文，实际屏幕 = {text}"
    );
    assert!(
        squeeze(&text).contains(&squeeze("帮我看下仓库状态")),
        "C10：隐藏过程不得连带隐藏用户消息，实际屏幕 = {text}"
    );
}

// ── A13：C11 默认隐藏（启动加载点）+ Default 基线不回退 ─────────────────────

#[test]
fn a13_product_default_hides_process_and_folds_thinking() {
    // ① Default 保持既有基线（契约修订 R1）：不做 IO、过程可见、思考展开。
    let baseline = tui::TuiState::default();
    assert!(!baseline.hide_process, "C11-R1：TuiState::default 必须保持过程可见基线");
    assert!(
        !baseline.thinking_collapsed,
        "C11-R1：TuiState::default 必须保持思考展开基线"
    );

    // ② 启动加载点（run_tui 调用 apply_tui_preferences）⇒ 产品默认：隐藏 + 折叠。
    let mut state = tui::TuiState::default();
    state.add_user_message("问题");
    state.messages.push(msg(tui::MessageRole::Thinking, "思考一行\n思考二行"));
    state.add_tool_event(true, "shell · ls");
    state.apply_tui_preferences(&TuiPreferences::default());
    assert!(state.hide_process, "C11：启动后必须默认隐藏过程");
    assert!(state.thinking_collapsed, "C11：启动后必须默认折叠思考");

    let buf = screen(&mut state, 100, 30);
    let text = all_text(&buf);
    assert!(
        !squeeze(&text).contains(&squeeze("shell · ls")),
        "C11：默认隐藏后工具行不得出现在屏幕上，实际屏幕 = {text}"
    );
    assert!(
        !squeeze(&text).contains(&squeeze("思考一行")),
        "C11：默认隐藏后思考正文不得出现在屏幕上，实际屏幕 = {text}"
    );
    assert!(
        squeeze(&text).contains("问题"),
        "C11：默认隐藏不得连带隐藏用户消息，实际屏幕 = {text}"
    );

    // ③ 偏好文件显式关闭 ⇒ 启动后过程可见（可开关，不是硬编码隐藏）。
    let mut visible = tui::TuiState::default();
    visible.add_user_message("问题");
    visible.messages.push(msg(tui::MessageRole::Thinking, "思考一行\n思考二行"));
    visible.add_tool_event(true, "shell · ls");
    visible.apply_tui_preferences(&TuiPreferences {
        hide_process: false,
        thinking_collapsed: false,
    });
    let buf = screen(&mut visible, 100, 30);
    let text = all_text(&buf);
    assert!(
        squeeze(&text).contains(&squeeze("shell · ls")),
        "C11：偏好关闭隐藏后工具行必须重新可见，实际屏幕 = {text}"
    );
    assert!(
        squeeze(&text).contains(&squeeze("思考一行")),
        "C11：偏好关闭折叠后思考正文必须重新可见，实际屏幕 = {text}"
    );
}

// ── A14：C12 快捷开关与面板共用同一字段 ─────────────────────────────────────

#[test]
fn a14_toggle_shortcut_shares_same_field() {
    let mut state = tui::TuiState::default();
    // 面板行如实反映字段（同一真值源）。
    assert_eq!(
        state.settings_rows()[0].2, state.hide_process,
        "C12：面板行状态与字段必须同源（初始）"
    );

    // ① 快捷开关（Ctrl+O 的状态机入口）翻转字段。
    let after = state.toggle_hide_process();
    assert_eq!(after, state.hide_process, "C12：toggle_hide_process 必须返回切换后的值");
    assert!(state.hide_process, "C12：初始 false ⇒ 切换后必须为 true");
    assert_eq!(
        state.settings_rows()[0].2, state.hide_process,
        "C12：快捷开关后面板行状态必须同步（同一字段）"
    );

    // ② 面板切换反写同一字段（两条入口不产生影子状态）。
    state.open_settings();
    state.settings_selected_index = 0;
    assert_eq!(
        state.handle_settings_key(KeyCode::Enter),
        Some(("hide_process", false)),
        "C12：面板切换必须复用同一字段并回报新值"
    );
    assert!(!state.hide_process, "C12：面板切换后字段必须为 false");
    assert_eq!(
        state.settings_rows()[0].2, state.hide_process,
        "C12：面板切换后行状态必须同步"
    );

    // ③ 快捷开关立即影响渲染口径（历史整体重排：缓存已失效）。
    let mut state = process_state(false);
    state.set_thinking_collapsed(true);
    let visible = all_text(&screen(&mut state, 100, 30));
    assert!(
        squeeze(&visible).contains(&squeeze("shell · git status --short")),
        "C14 harness：关闭隐藏时工具行必须可见，实际屏幕 = {visible}"
    );
    state.toggle_hide_process();
    let hidden = all_text(&screen(&mut state, 100, 30));
    assert!(
        !squeeze(&hidden).contains(&squeeze("shell · git status --short")),
        "C12：快捷开关后必须立即生效（缓存不得残留旧布局），实际屏幕 = {hidden}"
    );
    assert!(
        !squeeze(&hidden).contains(&squeeze("先核对工作区改动")),
        "C12：快捷开关后思考块必须同样隐藏，实际屏幕 = {hidden}"
    );
}
