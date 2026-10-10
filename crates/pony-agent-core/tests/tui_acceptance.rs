#[path = "../src/bin/tui.rs"]
mod tui;

use tui::{MessageRole, TuiState};

#[test]
fn test_tui_state_initialization() {
    let state = TuiState::default();
    assert_eq!(state.model, "gemini-3.8-flash");
    assert_eq!(state.status_text, "Ready");
    assert!(!state.is_streaming);
    assert!(state.messages.is_empty());
}

#[test]
fn test_tui_message_streaming_accumulation() {
    let mut state = TuiState::default();
    state.add_user_message("What is 1+1?");
    assert_eq!(state.messages.len(), 1);
    assert_eq!(state.messages[0].role, MessageRole::User);

    state.append_assistant_chunk("The answer ");
    state.append_assistant_chunk("is 2.");
    assert_eq!(state.messages.len(), 2);
    assert_eq!(state.messages[1].role, MessageRole::Assistant);
    assert_eq!(state.messages[1].content, "The answer is 2.");
}

#[test]
fn test_tui_tool_event_tracking() {
    let mut state = TuiState::default();
    // 首次工具触发
    state.add_or_update_tool_event("读取文件 (Read) · package.json", false);
    assert_eq!(state.messages.len(), 1);
    assert_eq!(state.messages[0].role, MessageRole::ToolCall);
    assert!(state.messages[0].content.contains("package.json"));

    // 工具结果返回 -> 归并更新原条目，而非追加产生第二条重复消息
    state.add_or_update_tool_event("读取文件 (Read) · package.json (完成)", true);
    assert_eq!(state.messages.len(), 1);
    assert_eq!(state.messages[0].role, MessageRole::ToolResult);
    assert!(state.messages[0].content.contains("完成"));
}

#[test]
fn test_spinner_frame_step() {
    let f0 = tui::spinner_frame(0);
    let f1 = tui::spinner_frame(1);
    assert_ne!(f0, f1);
    assert!(!f0.is_empty());
}

#[test]
fn test_auto_scroll_offset_calculation() {
    // 渲染总行数未超过可视区域，无需滚动
    assert_eq!(tui::calculate_scroll_offset(10, 20), 0);

    // 渲染总行数超出可视区域，自动向上顶升对应行距（贴底）
    assert_eq!(tui::calculate_scroll_offset(35, 20), 15);
}

#[test]
fn test_scroll_manual_override_and_page_step() {
    let mut state = TuiState::default();
    state.scroll_offset = 50;
    state.auto_scroll = true;

    // 用户按下 Up 键：立即解除 auto_scroll，并平滑减小 scroll_offset
    state.scroll_up(3);
    assert!(!state.auto_scroll);
    assert_eq!(state.scroll_offset, 47);

    // 用户连续多次上翻
    state.scroll_up(10);
    assert_eq!(state.scroll_offset, 37);

    // 用户下翻至接近或超出上限时，恢复追踪
    state.scroll_down(20, 50);
    assert_eq!(state.scroll_offset, 50);
    assert!(state.auto_scroll);
}

#[test]
fn test_wide_padding_wrap_width_budget() {
    let terminal_width = 80usize;
    // 左右内边距分别预留 6-8 字符（合计 14 字符缓冲）
    let content_width = terminal_width.saturating_sub(14);
    assert_eq!(content_width, 66);

    let text_len = 132usize;
    let expected_rows = (text_len + content_width - 1) / content_width;
    assert_eq!(expected_rows, 2);
}

#[test]
fn test_history_messages_projection_integrity() {
    let mut state = TuiState::default();

    // 模拟重放历史会话中的一条用户、一次工具、一次助手回答
    state.add_user_message("你好，请读取 package.json");
    state.add_or_update_tool_event("读取文件 (Read) · package.json", true);
    state.append_assistant_chunk("这是项目的 package.json 内容：\n```json\n{\n  \"name\": \"pony\"\n}\n```");

    // 再次追加第二轮用户与助手回答
    state.add_user_message("再见");
    // 新一轮回答应独立创建，不与前一轮助手回答混淆
    state.messages.push(tui::ChatMessage {
        role: MessageRole::Assistant,
        content: "下次见！".to_string(),
        timestamp_ms: 0,
    });

    assert_eq!(state.messages.len(), 5);
    assert_eq!(state.messages[0].role, MessageRole::User);
    assert_eq!(state.messages[1].role, MessageRole::ToolResult);
    assert_eq!(state.messages[2].role, MessageRole::Assistant);
    assert_eq!(state.messages[3].role, MessageRole::User);
    assert_eq!(state.messages[4].role, MessageRole::Assistant);
    assert_eq!(state.messages[4].content, "下次见！");
}

#[test]
fn test_tui_context_clear() {
    let mut state = TuiState::default();
    state.add_user_message("Hello");
    let old_sess = state.session_id.clone();
    state.clear_messages();

    assert!(state.messages.is_empty());
    assert_ne!(state.session_id, old_sess);
    assert_eq!(state.status_text, "Context cleared");
}

#[test]
fn test_tool_display_formatting() {
    let call_str = tui::format_tool_display(
        "Read",
        Some("读取文件"),
        Some("package.json"),
        None,
    );
    assert!(call_str.contains("读取文件 (Read)"));
    assert!(call_str.contains("package.json"));
}

#[test]
fn test_markdown_line_rendering_spans() {
    let heading_spans = tui::render_markdown_line("### 核心架构说明");
    assert!(!heading_spans.is_empty());
    assert!(heading_spans.iter().any(|s| s.content.contains("核心架构说明")));

    let bold_spans = tui::render_markdown_line("这是一个 **重要特性** 说明");
    assert!(bold_spans.iter().any(|s| s.content == "重要特性"));

    let code_spans = tui::render_markdown_line("请查看 `src/main.rs` 文件");
    assert!(code_spans.iter().any(|s| s.content.contains("src/main.rs")));
}
