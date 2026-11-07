//! 独立验收测试（L2-T）：TUI 思考消息可见 + 消息区底部与 Composer 间距。
//!
//! 契约真值源：`.dev-team/contract-matrix-thinking-bottom-gap.md`（C1–C12）。
//! 本文件是**唯一**验收载体，红相冻结后由 Executor 只读，禁止改断言放宽。
//!
//! 缺陷 1：`turn:delta` 的 `reasoning_content` 在 TUI 全链路被丢弃 ⇒ 消息列表看不到思考消息。
//! 缺陷 2：消息区（含流式状态行）下缘与 Composer 色带之间零间距，滚屏阅读拥挤。
//!
//! 范式来源：`crates/pony-agent-core/tests/tui_render_layout.rs`
//! （`#[path = "../src/bin/tui.rs"] mod tui;` + ratatui `TestBackend` + `row_text`/`all_text` 辅助）。
//!
//! 说明（红相归属）：本轮 `tui.rs` 的 SUT 空桩为
//! `stream_delta_ui_payload` / `TuiState::apply_stream_reasoning` / `Thinking` 渲染分支，
//! 三处均为 `todo!()`；故 A1–A7 的红相是 **SUT panic**，A8–A10 的红相是**真实断言失败**
//! （底部间距尚未实现），A11 是回归守卫（应保持绿）。

#[path = "../src/bin/tui.rs"]
#[allow(dead_code, unused_imports, unused_variables, unused_mut)]
mod tui;

use pony_agent_core::agent::runtime::TurnStreamEvent;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;

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
/// 这是**显式条件轮询**（无 sleep、无盲猜）：每帧渲染后检查
/// `scroll_offset >= max_scroll_offset`，达到即停，上限 400 帧防死循环。
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

/// 宽字符右侧的占位 cell 是空白，CJK 断言前先去空白再比。
fn squeeze(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 找到包含指定文本（去空白后匹配）的首行；找不到即 `None`（交由调用方给出契约化失败信息）。
fn find_row(buf: &Buffer, needle: &str) -> Option<u16> {
    let needle = squeeze(needle);
    (0..buf.area.height).find(|y| squeeze(&row_text(buf, *y)).contains(&needle))
}

/// 输入提示符行（Normal 态屏幕上必须唯一包含 `›`）。
fn prompt_row(buf: &Buffer) -> u16 {
    let rows: Vec<u16> = (0..buf.area.height)
        .filter(|y| row_text(buf, *y).contains('›'))
        .collect();
    assert_eq!(
        rows.len(),
        1,
        "A8/A9/A10 harness：Normal 态屏幕上必须恰好 1 行含输入提示符 '›'（定位 Composer 色带上缘），实际行 = {rows:?}"
    );
    rows[0]
}

/// 构造最小合法 `TurnStreamEvent`（与 `tests/headless_contract.rs` 的 `stream_event` 同范式：
// 反序列化构造，避免逐字段手写 30+ 个 Option 字段）。
fn stream_event(text: Option<&str>, reasoning: Option<&str>) -> TurnStreamEvent {
    let mut value = serde_json::json!({
        "turnId": "turn-thinking-gap",
        "kind": "delta",
        "sessionId": "session-thinking-gap",
    });
    if let Some(t) = text {
        value["text"] = serde_json::Value::String(t.to_string());
    }
    if let Some(r) = reasoning {
        value["reasoningContent"] = serde_json::Value::String(r.to_string());
    }
    serde_json::from_value(value).expect("A-harness：TurnStreamEvent 夹具必须可反序列化")
}

fn msg(role: tui::MessageRole, content: &str) -> tui::ChatMessage {
    tui::ChatMessage {
        role,
        content: content.to_string(),
        timestamp_ms: 0,
    }
}

/// 造 40 条必然溢出 30 行视口的长用户消息（契约 C9：间距不得靠「内容不够长」碰巧成立）。
fn overflow_state() -> tui::TuiState {
    let mut state = tui::TuiState::default();
    for i in 0..40 {
        state.add_user_message(&format!(
            "第{i:02}问：这是一段足够长的用户消息，用来把消息区撑到溢出视口，保证贴底时最后一条内容仍然完整可见。"
        ));
    }
    state
}

/// `theme::FAINT` 的字面量（来源：`crates/pony-agent-core/src/bin/tui.rs` 的
/// `pub const FAINT: Color = Color::Rgb(124, 118, 110);`，契约 C6 指定思考正文用它）。
/// 测试内写字面量而**不**引用内部模块常量，是为了让断言独立于实现侧的常量搬迁。
const THEME_FAINT: Color = Color::Rgb(124, 118, 110);

// ── A1 / A2：turn:delta 双通道取载（契约 C1/C2/C3）───────────────────────────

/// A1：仅 text 通道 ⇒ `DeltaUiPayload { text: Some("hello"), reasoning: None }`。
#[test]
fn a1_delta_payload_text_only() {
    let event = stream_event(Some("hello"), None);
    let got = tui::stream_delta_ui_payload(&event);
    assert_eq!(
        got,
        tui::DeltaUiPayload {
            text: Some("hello".to_string()),
            reasoning: None,
        },
        "契约 C1/C2：仅带正文的 turn:delta 必须只投影到 text 通道且 reasoning=None，\
         实际 = {got:?}（若 panic 在 todo!() 则说明 turn:delta 双通道取载未实现）"
    );
}

/// A2：双通道互相独立 + 空载 + reasoning 空白 + text 经 DSML 净化后为空（契约 C2/C3）。
#[test]
fn a2_delta_payload_channel_isolation() {
    // ① 仅 reasoning：思考内容必须独立成通道，不能被丢进 text。
    let got = tui::stream_delta_ui_payload(&stream_event(None, Some("先核对事件通道")));
    assert_eq!(
        got,
        tui::DeltaUiPayload {
            text: None,
            reasoning: Some("先核对事件通道".to_string()),
        },
        "契约 C2：仅带 reasoning_content 的 delta 必须只投影到 reasoning 通道且 text=None，实际 = {got:?}"
    );

    // ② 两者都非空：两条通道可同时有值（不得二选一）。
    let got = tui::stream_delta_ui_payload(&stream_event(Some("正文片段"), Some("思考片段")));
    assert_eq!(
        (got.text.clone(), got.reasoning.clone()),
        (Some("正文片段".to_string()), Some("思考片段".to_string())),
        "契约 C2：text 与 reasoning 可同时非空，实际 = {got:?}"
    );

    // ③ 两者都为空 ⇒ 全 None（空载不得产生任何 UI 事件）。
    let got = tui::stream_delta_ui_payload(&stream_event(None, None));
    assert_eq!(
        got,
        tui::DeltaUiPayload::default(),
        "契约 C2：双通道皆空的 delta 必须等于 DeltaUiPayload::default()（None/None），实际 = {got:?}"
    );

    // ④ reasoning 为纯空白 ⇒ reasoning=None（原样透传但 trim 后为空即丢弃）。
    let got = tui::stream_delta_ui_payload(&stream_event(None, Some("   \n\t ")));
    assert_eq!(
        got.reasoning, None,
        "契约 C3：reasoning 仅 trim().is_empty() 时置 None，纯空白 chunk 不得投递思考事件，实际 = {got:?}"
    );

    // ⑤ text 为 DSML 协议分片（真实 SSE 分片原语，取自
    //    `tests/tui_session_resilience_chunk_split.rs` 的 `FULL_DSML` 首个分片）：净化后为空 ⇒ text=None。
    let dsml_fragment = "<｜｜DSML｜｜ inv";
    let (cleaned, incomplete) = tui::sanitize_model_protocol_text(dsml_fragment);
    assert!(
        cleaned.is_empty() && incomplete,
        "A2 harness：DSML 分片样本必须被净化清空且标记 incomplete=true（否则样本本身不成立），\
         实际 cleaned = {cleaned:?}, incomplete = {incomplete}"
    );
    let got = tui::stream_delta_ui_payload(&stream_event(Some(dsml_fragment), None));
    assert_eq!(
        got.text, None,
        "契约 C3：text 通道经 sanitize_model_protocol_text 净化后为空 ⇒ text=None（不得投递空正文事件），\
         实际 = {got:?}"
    );
    assert_eq!(
        got.reasoning, None,
        "契约 C3：text 通道被净化不得连带伪造 reasoning 通道，实际 = {got:?}"
    );
}

// ── A3 / A4：思考增量入库（契约 C4/C7）───────────────────────────────────────

/// A3：连续两次思考增量合并为单条 Thinking 消息；空白 chunk 不新增/不改变消息。
#[test]
fn a3_reasoning_chunks_merge_into_single_thinking_message() {
    let mut state = tui::TuiState::default();

    state.apply_stream_reasoning("甲");
    state.apply_stream_reasoning("乙");

    assert_eq!(
        state.messages.len(),
        1,
        "契约 C4：连续思考增量在末条为 Thinking 时必须原地续写为 1 条消息，实际 {} 条",
        state.messages.len()
    );
    assert_eq!(
        state.messages[0].role,
        tui::MessageRole::Thinking,
        "契约 C4：思考增量的消息角色必须是 MessageRole::Thinking，实际 = {:?}",
        state.messages[0].role
    );
    assert_eq!(
        state.messages[0].content, "甲乙",
        "契约 C4：思考增量必须按到达顺序原地续写，实际 = {:?}",
        state.messages[0].content
    );

    // 空白 chunk：不新增消息、不改变内容。
    state.apply_stream_reasoning("   ");
    assert_eq!(
        state.messages.len(),
        1,
        "契约 C4：纯空白思考 chunk 不得新开消息，实际 {} 条",
        state.messages.len()
    );
    assert_eq!(
        state.messages[0].content, "甲乙",
        "契约 C4：纯空白思考 chunk 不得改变已有思考内容，实际 = {:?}",
        state.messages[0].content
    );
}

/// A4：思考不得被并入正文，正文也不得并入思考（角色顺序 = [User, Thinking, Assistant]）。
#[test]
fn a4_thinking_and_assistant_stay_separate() {
    let mut state = tui::TuiState::default();
    state.add_user_message("问");
    state.apply_stream_reasoning("想");
    state.append_assistant_chunk("答");

    let roles: Vec<tui::MessageRole> = state.messages.iter().map(|m| m.role.clone()).collect();
    assert_eq!(
        roles,
        vec![
            tui::MessageRole::User,
            tui::MessageRole::Thinking,
            tui::MessageRole::Assistant
        ],
        "契约 C4/C7：思考必须是独立的 Thinking 消息，随后正文必须新起 Assistant（不得追加进思考块），\
         实际角色序列 = {roles:?}"
    );
    assert_eq!(
        state.messages[1].content, "想",
        "契约 C4：Thinking 消息正文只能是思考内容，实际 = {:?}",
        state.messages[1].content
    );
    assert_eq!(
        state.messages[2].content, "答",
        "契约 C7：Assistant 消息正文只能是正文内容（正文不得并入思考），实际 = {:?}",
        state.messages[2].content
    );
}

// ── A5：运行态推进（契约 C5）─────────────────────────────────────────────────

/// A5：收到思考增量 ⇒ runtime_mode=Thinking；is_streaming/scroll/auto_scroll 不变；工具执行中不抢状态。
#[test]
fn a5_reasoning_updates_runtime_mode_without_stealing_tool_state() {
    // Arrange：非流式、手动滚动（auto_scroll=false）且已有滚动偏移。
    let mut state = tui::TuiState::default();
    state.is_streaming = false;
    state.auto_scroll = false;
    state.scroll_offset = 7;

    state.apply_stream_reasoning("推理中");

    assert_eq!(
        state.runtime_mode,
        tui::RuntimeMode::Thinking,
        "契约 C5：收到思考增量且当前不在工具执行中 ⇒ runtime_mode 必须为 Thinking，实际 = {:?}",
        state.runtime_mode
    );
    assert!(
        !state.is_streaming,
        "契约 C5：思考增量不得改写 is_streaming（必须保持调用前的 false），实际 = {}",
        state.is_streaming
    );
    assert!(
        !state.auto_scroll,
        "契约 C5：思考增量不得改写 auto_scroll（必须保持调用前的 false），实际 = {}",
        state.auto_scroll
    );
    assert_eq!(
        state.scroll_offset, 7,
        "契约 C5：思考增量不得改写 scroll_offset（必须保持调用前的 7），实际 = {}",
        state.scroll_offset
    );

    // 工具执行中：思考增量不得抢工具执行状态。
    let mut tool_state = tui::TuiState::default();
    tool_state.runtime_mode = tui::RuntimeMode::ExecutingTool("bash".to_string());
    tool_state.apply_stream_reasoning("推理中");
    assert_eq!(
        tool_state.runtime_mode,
        tui::RuntimeMode::ExecutingTool("bash".to_string()),
        "契约 C5：工具执行中（ExecutingTool）到达的思考增量不得抢状态，实际 = {:?}",
        tool_state.runtime_mode
    );
}

// ── A6 / A7：思考块渲染（契约 C6/C7）─────────────────────────────────────────

/// A6：Thinking 消息在 110×30 真实渲染可见；正文 ITALIC + FAINT；首行带 ✻ 字形。
#[test]
fn a6_thinking_message_renders_with_faint_italic_and_glyph() {
    const THINKING_TEXT: &str = "推理阶段：先核对事件通道。";

    let mut state = tui::TuiState::default();
    state
        .messages
        .push(msg(tui::MessageRole::Thinking, THINKING_TEXT));

    let buf = screen(&mut state, 110, 30);

    let row = find_row(&buf, THINKING_TEXT).unwrap_or_else(|| {
        panic!(
            "契约 C6：MessageRole::Thinking 必须在消息列表可见（屏幕上找不到 {THINKING_TEXT:?}）。\
             实际屏幕 = \n{}",
            all_text(&buf)
        )
    });

    // 逐字形校验：该行所有属于思考正文的 cell 必须 ITALIC + FAINT。
    let text_chars: std::collections::HashSet<char> = THINKING_TEXT.chars().collect();
    let mut checked = 0usize;
    for x in 0..buf.area.width {
        let cell = &buf[(x, row)];
        let mut chars = cell.symbol().chars();
        let (Some(ch), None) = (chars.next(), chars.next()) else {
            continue;
        };
        if !text_chars.contains(&ch) {
            continue;
        }
        checked += 1;
        assert!(
            cell.modifier.contains(Modifier::ITALIC),
            "契约 C6：思考正文必须带 Modifier::ITALIC（视觉从属于正文），\
             第 {row} 行第 {x} 列字符 {ch:?} 的 modifier = {:?}",
            cell.modifier
        );
        assert_eq!(
            cell.fg, THEME_FAINT,
            "契约 C6：思考正文前景色必须是 theme::FAINT = {THEME_FAINT:?}（tui.rs 常量字面量），\
             第 {row} 行第 {x} 列字符 {ch:?} 实际前景色 = {:?}",
            cell.fg
        );
    }
    assert!(
        checked >= 5,
        "A6 harness：必须在屏幕上命中至少 5 个思考正文字形（否则无法证明字样样式落地），实际命中 {checked} 个"
    );

    let text = all_text(&buf);
    assert!(
        text.contains('✻'),
        "契约 C6：思考块首行必须以 '✻' 作为前缀字形（theme::ACCENT_DIM），屏幕实际 = \n{text}"
    );
}

/// A7：思考块最后一行与随后正文第一行之间恰好 1 行呼吸（不是 2 行、不是 0 行）。
#[test]
fn a7_single_breathing_line_between_thinking_and_answer() {
    let mut state = tui::TuiState::default();
    state.messages.push(msg(tui::MessageRole::User, "第一问"));
    state.messages.push(msg(tui::MessageRole::Thinking, "思考一行"));
    state.messages.push(msg(tui::MessageRole::Assistant, "最终回答"));

    let buf = screen(&mut state, 110, 30);

    let thinking_row = find_row(&buf, "思考一行").unwrap_or_else(|| {
        panic!(
            "契约 C6：思考块必须在屏幕上可见（找不到 \"思考一行\"），实际屏幕 = \n{}",
            all_text(&buf)
        )
    });
    let answer_row = find_row(&buf, "最终回答").unwrap_or_else(|| {
        panic!(
            "契约 C7：思考后的正文必须在屏幕上可见（找不到 \"最终回答\"），实际屏幕 = \n{}",
            all_text(&buf)
        )
    });

    assert_eq!(
        answer_row,
        thinking_row + 2,
        "契约 C6/C7：思考块最后一行（y={thinking_row}）与最终回答第一行（y={answer_row}）之间\
         必须**恰好 1 行**呼吸留白，实际间隔 {} 行；屏幕 = \n{}",
        answer_row as i32 - thinking_row as i32,
        all_text(&buf)
    );
    assert!(
        row_text(&buf, thinking_row + 1).trim().is_empty(),
        "契约 C6：思考块与正文之间的呼吸行必须全空白，实际 = {:?}",
        row_text(&buf, thinking_row + 1)
    );
}

// ── A8 / A9 / A10：底部间距（契约 C8/C9/C10/C11）─────────────────────────────

/// A8：自动贴底 + 溢出视口 ⇒ 消息区下缘正好停在 CHAT_BOTTOM_GAP=2 行空白间距之上，且间距之上是内容。
#[test]
fn a8_bottom_gap_holds_when_auto_scrolled_to_bottom() {
    let mut state = overflow_state();
    assert!(
        state.auto_scroll,
        "A8 harness：TuiState::default() 必须默认自动贴底（auto_scroll=true）"
    );

    let buf = screen_until_settled(&mut state, 110, 30);

    assert!(
        state.max_scroll_offset > 0,
        "A8 harness：40 条长用户消息必须溢出 110×30 视口（max_scroll_offset 必须 > 0），实际 = {}",
        state.max_scroll_offset
    );
    assert_eq!(
        state.scroll_offset, state.max_scroll_offset,
        "契约 C9：auto_scroll 收敛后必须贴底（scroll_offset == max_scroll_offset），\
         实际 offset = {} / max = {}",
        state.scroll_offset, state.max_scroll_offset
    );

    let prompt = prompt_row(&buf);
    // 契约 C8 修订 R5（2026-11-06 用户实测：1 行偏挤）⇒ CHAT_BOTTOM_GAP = 2。
    // 几何：输入提示符行 prompt；其上一行 = Composer 色带上缘（band_top = prompt - 1）；
    // 再往上 CHAT_BOTTOM_GAP 行 = 间距行，即 [prompt-3, prompt-2]；间距之上 prompt-4 必须是内容。
    let band_top = prompt - 1;
    let gap_rows = [prompt.saturating_sub(3), prompt.saturating_sub(2)];

    assert_eq!(
        tui::CHAT_BOTTOM_GAP, 2,
        "契约 C8(修订 R5)：消息区与 Composer 带的固定呼吸行数常量必须为 2（用户实测 1 行偏挤），\
         实际 = {}",
        tui::CHAT_BOTTOM_GAP
    );
    for gap_row in gap_rows {
        assert!(
            row_text(&buf, gap_row).trim().is_empty(),
            "契约 C8(修订 R5)：间距行 y={gap_row}（共 {} 行：{gap_rows:?}）必须全空白\
             （CANVAS 底色、无任何字形），实际 = {:?}；屏幕 = \n{}",
            tui::CHAT_BOTTOM_GAP,
            row_text(&buf, gap_row),
            all_text(&buf)
        );
    }

    // 底色证据：间距行必须落在 CANVAS 画布上，而色带上缘 band_top 是 SURFACE 表面
    // ⇒ 这 2 行确实在 Composer 带之外，而不是被色带吞掉（字面量来源同 THEME_FAINT 注释）。
    const THEME_CANVAS: Color = Color::Rgb(18, 16, 14); // tui.rs `theme::CANVAS`
    const THEME_SURFACE: Color = Color::Rgb(27, 25, 22); // tui.rs `theme::SURFACE`
    let probe_x = buf.area.width / 2;
    for gap_row in gap_rows {
        assert_eq!(
            buf[(probe_x, gap_row)].bg,
            THEME_CANVAS,
            "契约 C8(修订 R5)：间距行 y={gap_row} 必须落在 CANVAS 画布底（不在 Composer 色带内），\
             实际底色 = {:?}",
            buf[(probe_x, gap_row)].bg
        );
    }
    assert_eq!(
        buf[(probe_x, band_top)].bg,
        THEME_SURFACE,
        "A8 harness：Composer 色带上缘 y={band_top} 必须是 SURFACE 底色\
         （用于判定 2 行间距确实位于色带之外），实际 = {:?}",
        buf[(probe_x, band_top)].bg
    );

    let chat = state
        .last_chat_rect
        .expect("A8 harness：ui() 渲染后必须写入 last_chat_rect");
    assert_eq!(
        chat.y + chat.height,
        gap_rows[0],
        "契约 C8(修订 R5)：消息区下缘（last_chat_rect.y={} + height={} = {}）必须正好停在\
         第一行间距上沿 y={}，否则最后一行正文会贴住 Composer 色带",
        chat.y,
        chat.height,
        chat.y + chat.height,
        gap_rows[0]
    );

    assert!(
        !row_text(&buf, prompt.saturating_sub(4)).trim().is_empty(),
        "契约 C9：间距行之上（y={}）必须就是内容行（防止「内容没铺满」碰巧留白），实际 = {:?}",
        prompt.saturating_sub(4),
        row_text(&buf, prompt.saturating_sub(4))
    );
}

/// A9：流式状态行是内容区最后一行，其下同样保留 CHAT_BOTTOM_GAP=2 行空白间距。
#[test]
fn a9_streaming_status_line_keeps_bottom_gap() {
    let mut state = overflow_state();
    state.is_streaming = true;
    state.runtime_mode = tui::RuntimeMode::Thinking;

    let buf = screen_until_settled(&mut state, 110, 30);
    let text = all_text(&buf);

    // 注意：宽字符右侧的占位 cell 是空白，CJK 断言必须先 squeeze 再去空白匹配。
    assert!(
        squeeze(&text).contains("思考中"),
        "契约 C10：is_streaming + RuntimeMode::Thinking 时内容区必须渲染「思考中」状态行，\
         屏幕实际 = \n{text}"
    );
    assert_eq!(
        state.scroll_offset, state.max_scroll_offset,
        "契约 C9：auto_scroll 收敛后必须贴底，实际 offset = {} / max = {}",
        state.scroll_offset, state.max_scroll_offset
    );

    let prompt = prompt_row(&buf);
    // 契约 C8 修订 R5 / C10：prompt-1 = 色带上缘 ⇒ 间距行 = [prompt-3, prompt-2]（2 行）。
    let gap_rows = [prompt.saturating_sub(3), prompt.saturating_sub(2)];

    for gap_row in gap_rows {
        assert!(
            row_text(&buf, gap_row).trim().is_empty(),
            "契约 C8(修订 R5)/C10：流式状态行之下的间距行 y={gap_row}（共 {} 行：{gap_rows:?}）\
             必须全空白，实际 = {:?}；屏幕 = \n{}",
            tui::CHAT_BOTTOM_GAP,
            row_text(&buf, gap_row),
            text
        );
    }

    let chat = state
        .last_chat_rect
        .expect("A9 harness：ui() 渲染后必须写入 last_chat_rect");
    assert_eq!(
        chat.y + chat.height,
        gap_rows[0],
        "契约 C10(修订 R5)：流式状态下消息区下缘（{} + {} = {}）同样必须正好停在\
         第一行间距上沿 y={}",
        chat.y,
        chat.height,
        chat.y + chat.height,
        gap_rows[0]
    );
    assert!(
        squeeze(&row_text(&buf, prompt.saturating_sub(4))).contains("思考中"),
        "契约 C10(修订 R5)：「思考中」状态行必须是内容区最后一行（紧贴 2 行间距之上，y={}），\
         实际 = {:?}",
        prompt.saturating_sub(4),
        row_text(&buf, prompt.saturating_sub(4))
    );
}

/// A10：极小终端（24×10）不 panic，且消息区下缘不得越过 `prompt_row - (1 + CHAT_BOTTOM_GAP)`。
#[test]
fn a10_tiny_terminal_keeps_gap_without_panic() {
    let mut state = overflow_state();
    let buf = screen_until_settled(&mut state, 24, 10);

    let prompt = prompt_row(&buf);
    let chat = state
        .last_chat_rect
        .expect("A10 harness：ui() 渲染后必须写入 last_chat_rect");

    // 契约 C11 修订：24×10 下消息区下缘 ≤ prompt_row - (1 + CHAT_BOTTOM_GAP) = prompt_row - 3。
    let limit = prompt.saturating_sub(3);
    assert!(
        chat.y + chat.height <= limit,
        "契约 C11(修订 R5)：24×10 极小终端下消息区下缘（{} + {} = {}）仍必须 ≤ prompt_row - 3 = {}，\
         不得把间距行挤进 Composer；屏幕 = \n{}",
        chat.y,
        chat.height,
        chat.y + chat.height,
        limit,
        all_text(&buf)
    );
    assert!(
        !all_text(&buf).contains("终端窗口过小"),
        "A10：24×10 是契约允许的最小尺寸（width≥24 且 height≥10），不得降级为「终端窗口过小」提示"
    );
}

// ── A11：回归不变（契约 C12）─────────────────────────────────────────────────

/// A11：User/Assistant/ToolCall/System 各一条渲染不 panic，且思考字形不得泄漏到既有角色。
#[test]
fn a11_existing_roles_render_unchanged_without_thinking_glyph() {
    let mut state = tui::TuiState::default();
    state.messages.push(msg(tui::MessageRole::User, "用户问题"));
    state.messages.push(msg(tui::MessageRole::Assistant, "助手回答"));
    state
        .messages
        .push(msg(tui::MessageRole::ToolCall, "bash · cargo test"));
    state.messages.push(msg(tui::MessageRole::System, "系统提示"));

    let buf = screen(&mut state, 110, 30);
    let text = all_text(&buf);

    assert!(
        !text.contains('✻'),
        "契约 C12：✻ 是思考块专属字形，不得泄漏到 User/Assistant/ToolCall/System 既有角色，\
         屏幕实际 = \n{text}"
    );
    for expected in ["用户问题", "助手回答", "系统提示"] {
        assert!(
            squeeze(&text).contains(&squeeze(expected)),
            "契约 C12：既有角色渲染不得回归（屏幕上找不到 {expected:?}），屏幕实际 = \n{text}"
        );
    }
    assert!(
        squeeze(&text).contains(&squeeze("cargo test")),
        "契约 C12：ToolCall 行渲染不得回归（屏幕上找不到 \"cargo test\"），屏幕实际 = \n{text}"
    );
}

// ── A12：思考块前驱侧间距（契约 C6 修订 R1，2026-11-06；Review F1/BLOCKING）──────
//
// 追加说明：本用例为 L3-B 审查 F1（BLOCKING）的机械载体，A1–A11 断言一字未改。
// 契约 C6 修订真值：思考块与**轮内**相邻消息（Assistant / ToolCall / ToolResult / System）
// 之间恰好 1 行呼吸（前驱侧不得为 0 粘连）；与 `User` 的边界沿用既有轮间距
// （User 块自带 1 行内边距 ⇒ 前驱侧视觉 2 行；后驱侧 = 思考块尾 1 行 + User 块首内边距 1 行
// ⇒ 行差 3），不得由思考块额外叠加。

/// 按给定角色序列渲染一屏（110×30，Normal 态），返回缓冲。
fn screen_sequence(seq: &[(tui::MessageRole, &str)]) -> Buffer {
    let mut state = tui::TuiState::default();
    for (role, content) in seq {
        state.messages.push(msg(role.clone(), content));
    }
    screen(&mut state, 110, 30)
}

/// 断言两个哨兵行之间**恰好** `expected_rows` 行间距，且这些中间行全空白。
fn assert_sentinel_gap(buf: &Buffer, before: &str, after: &str, expected_rows: u16, why: &str) {
    let before_row = find_row(buf, before).unwrap_or_else(|| {
        panic!(
            "{why}：屏幕上找不到前驱哨兵 {before:?}，实际屏幕 = \n{}",
            all_text(buf)
        )
    });
    let after_row = find_row(buf, after).unwrap_or_else(|| {
        panic!(
            "{why}：屏幕上找不到后继哨兵 {after:?}，实际屏幕 = \n{}",
            all_text(buf)
        )
    });
    assert!(
        after_row > before_row,
        "{why}：后继哨兵必须位于前驱哨兵之下（before={before_row}, after={after_row}），\
         实际屏幕 = \n{}",
        all_text(buf)
    );
    assert_eq!(
        after_row - before_row,
        expected_rows,
        "{why}：哨兵行差必须为 {expected_rows}（= 中间 {} 行空白），\
         实际 before={before_row} → after={after_row} 行差 {}；屏幕 = \n{}",
        expected_rows - 1,
        after_row - before_row,
        all_text(buf)
    );
    for y in (before_row + 1)..after_row {
        assert!(
            row_text(buf, y).trim().is_empty(),
            "{why}：前驱哨兵与后继哨兵之间的第 {y} 行必须全空白（间距行不得夹带字形），\
             实际 = {:?}",
            row_text(buf, y)
        );
    }
}

/// A12：思考块**前驱侧**间距——轮内 1 行呼吸、跨轮沿用既有节奏（契约 C6 修订 R1）。
#[test]
fn a12_thinking_keeps_one_breath_against_previous_non_user_blocks() {
    // 序列 1：Assistant → Thinking。
    // Review F1 实测：思考块块首的 `push_blank` 作用在**空块**上是 no-op ⇒ 前驱侧 0 行（粘连）。
    let buf = screen_sequence(&[
        (tui::MessageRole::Assistant, "ASSISTA"),
        (tui::MessageRole::Thinking, "THINKT"),
    ]);
    assert_sentinel_gap(
        &buf,
        "ASSISTA",
        "THINKT",
        2,
        "契约 C6(R1)：Assistant → Thinking 前驱侧必须恰好 1 行呼吸（行差 2），不得 0 行粘连",
    );
    assert!(
        all_text(&buf).contains('✻'),
        "契约 C6：思考块首行必须仍以 '✻' 为前缀，屏幕实际 = \n{}",
        all_text(&buf)
    );

    // 序列 2：ToolResult → Thinking（工具结果行之后同样不得粘连）。
    let buf = screen_sequence(&[
        (tui::MessageRole::ToolResult, "bash · cargo test"),
        (tui::MessageRole::Thinking, "THINKT"),
    ]);
    assert_sentinel_gap(
        &buf,
        "cargo test",
        "THINKT",
        2,
        "契约 C6(R1)：ToolResult → Thinking 前驱侧必须恰好 1 行呼吸（行差 2），不得 0 行粘连",
    );

    // 序列 3：User → Thinking。ADR 0088（2026-11-07）给用户块下方新增 1 行页面底色空白
    // ⇒ 行差 = 内边距 1 + 新增空白 1 + 1 = 3；思考块仍不得额外叠加。
    let buf = screen_sequence(&[
        (tui::MessageRole::User, "USERQ"),
        (tui::MessageRole::Thinking, "THINKT"),
    ]);
    assert_sentinel_gap(
        &buf,
        "USERQ",
        "THINKT",
        3,
        "ADR 0088：User → Thinking 行差 3（内边距 1 + 用户块下间距 1），思考块不得额外叠加成 4",
    );

    // 序列 4：Thinking → User。思考块尾 1 行 + User 块上间距 1 行（ADR 0088）+ 内边距 1 行 ⇒ 行差 4。
    let buf = screen_sequence(&[
        (tui::MessageRole::Thinking, "THINKT"),
        (tui::MessageRole::User, "USERQ"),
    ]);
    assert_sentinel_gap(
        &buf,
        "THINKT",
        "USERQ",
        4,
        "ADR 0088：Thinking → User 行差 4（思考尾 1 + 用户块上间距 1 + 内边距 1），\
         思考块不得把轮间距撑成 5",
    );
}

// ── A13：正文通道纯空白增量（契约 C3 修订 R3，2026-11-06）─────────────────────
//
// 真值：HEAD 版判空口径是 `!visible.is_empty()`——`"\n\n"` 是**段落分隔符**，属正文增量，
// 必须原样保留；若误用 `trim().is_empty()` 判空会吞掉空行 ⇒ 渲染侧段落丢失。

/// A13：`text` 通道纯空白增量不得被 trim 判空丢弃；真「空载」仍为 None。
#[test]
fn a13_delta_payload_preserves_whitespace_only_text() {
    // ① 段落分隔符增量必须原样透传。
    let got = tui::stream_delta_ui_payload(&stream_event(Some("\n\n"), None));
    assert_eq!(
        got.text.as_deref(),
        Some("\n\n"),
        "契约 C3(修订 R3)：正文通道纯空白增量 \"\\n\\n\"（段落分隔）必须原样保留，\
         用 trim() 判空会丢弃空行导致段落丢失，实际 = {got:?}"
    );
    assert_eq!(
        got.reasoning, None,
        "契约 C3：正文纯空白增量不得连带伪造 reasoning 通道，实际 = {got:?}"
    );

    // ② 单空格同样是正文增量（`!is_empty()` 口径），不得被 trim 丢弃。
    let got = tui::stream_delta_ui_payload(&stream_event(Some(" "), None));
    assert_eq!(
        got.text.as_deref(),
        Some(" "),
        "契约 C3(修订 R3)：单空格正文增量在 `!is_empty()` 口径下必须保留，实际 = {got:?}"
    );

    // ③ 对照：真正的空载（空字符串）仍必须等于 default（否则会投递空正文事件）。
    let empty = tui::stream_delta_ui_payload(&stream_event(Some(""), None));
    assert_eq!(
        empty,
        tui::DeltaUiPayload::default(),
        "契约 C2/C3：text 为空字符串（真空载）仍必须全 None（不得投递空正文事件），实际 = {empty:?}"
    );
}

// ── A14：思考按轮分块（契约 C4a 修订，2026-11-06）───────────────────────────
//
// 真值：宿主续轮（Ask 恢复 / 审批恢复 / goal 续轮）**不经过** `add_user_message`，
// 因此不能依赖 User 消息切块；唯一公共复位点是 `turn:started` → `TuiState::begin_turn_usage()`。
// 若不在此处置轮边界，上一轮思考块会被下一轮思考增量续写（跨轮混块）。

/// A14：`begin_turn_usage()`（等价 `UiEvent::TurnStarted`）必须切断思考块续写。
#[test]
fn a14_thinking_block_is_bounded_by_turn_started_reset() {
    let mut s = tui::TuiState::default();
    s.add_user_message("问");
    s.apply_stream_reasoning("第一轮思考");
    s.begin_turn_usage(); // 宿主续轮的唯一公共复位点（UiEvent::TurnStarted 的落点）
    s.apply_stream_reasoning("第二轮思考");

    let roles: Vec<tui::MessageRole> = s.messages.iter().map(|m| m.role.clone()).collect();
    assert_eq!(
        s.messages.len(),
        3,
        "契约 C4a：turn:started 之后到达的思考增量必须**新开** Thinking 块（User + 2×Thinking = 3 条），\
         实际 {} 条，角色序列 = {roles:?}",
        s.messages.len()
    );
    assert_eq!(
        roles,
        vec![
            tui::MessageRole::User,
            tui::MessageRole::Thinking,
            tui::MessageRole::Thinking
        ],
        "契约 C4a：跨轮思考必须是两个独立 Thinking 块（不得续写到上一轮块里），实际角色序列 = {roles:?}"
    );
    assert_eq!(
        s.messages[1].content, "第一轮思考",
        "契约 C4a：第一轮思考块内容不得被下一轮增量污染，实际 = {:?}",
        s.messages[1].content
    );
    assert_eq!(
        s.messages[2].content, "第二轮思考",
        "契约 C4a：第二轮思考增量必须落在新块，实际 = {:?}",
        s.messages[2].content
    );
}
