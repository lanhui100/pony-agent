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
fn test_matching_commands_filter() {
    let mut state = TuiState::default();
    assert!(state.matching_commands().is_empty());

    state.input_buffer = "/".to_string();
    let all = state.matching_commands();
    assert_eq!(all.len(), 5);

    state.input_buffer = "/re".to_string();
    let re = state.matching_commands();
    assert_eq!(re.len(), 1);
    assert_eq!(re[0].0, "/resume");
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
    assert!(!heading_spans[0].content.contains('#'));
    assert!(heading_spans.iter().any(|s| s.content.contains("核心架构说明")));

    let h4_spans = tui::render_markdown_line("#### 1. 根级别字段");
    assert!(!h4_spans.is_empty());
    assert!(!h4_spans[0].content.contains('#'));
    assert!(h4_spans.iter().any(|s| s.content.contains("根级别字段")));

    let bold_spans = tui::render_markdown_line("这是一个 **重要特性** 说明");
    assert!(bold_spans.iter().any(|s| s.content == "重要特性"));

    let code_spans = tui::render_markdown_line("请查看 `src/main.rs` 文件");
    assert!(code_spans.iter().any(|s| s.content.contains("src/main.rs")));
}

#[test]
fn test_provider_and_models_commands_matching() {
    let mut state = TuiState::default();

    // 1. 验证 AVAILABLE_COMMANDS 包含 /provider 和 /models
    assert!(tui::AVAILABLE_COMMANDS.iter().any(|(cmd, _)| *cmd == "/provider"));
    assert!(tui::AVAILABLE_COMMANDS.iter().any(|(cmd, _)| *cmd == "/models"));

    // 2. 验证 /p 匹配到 /provider
    state.input_buffer = "/p".to_string();
    let p_cmds = state.matching_commands();
    assert!(p_cmds.iter().any(|(cmd, _)| *cmd == "/provider"));

    // 3. 验证 /m 匹配到 /models
    state.input_buffer = "/m".to_string();
    let m_cmds = state.matching_commands();
    assert!(m_cmds.iter().any(|(cmd, _)| *cmd == "/models"));

    // 4. 验证完整命令匹配
    state.input_buffer = "/provider".to_string();
    let full_p = state.matching_commands();
    assert_eq!(full_p.len(), 1);
    assert_eq!(full_p[0].0, "/provider");

    state.input_buffer = "/models".to_string();
    let full_m = state.matching_commands();
    assert_eq!(full_m.len(), 1);
    assert_eq!(full_m[0].0, "/models");
}

#[test]
fn test_provider_selector_state_logic() {
    let mut state = TuiState::default();
    assert_eq!(state.mode, tui::InteractionMode::Normal);

    let providers = vec![
        tui::ProviderItem {
            id: "openai".to_string(),
            name: "OpenAI".to_string(),
            protocol: "openai".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
        },
        tui::ProviderItem {
            id: "anthropic".to_string(),
            name: "Anthropic".to_string(),
            protocol: "anthropic".to_string(),
            base_url: "https://api.anthropic.com".to_string(),
        },
        tui::ProviderItem {
            id: "deepseek".to_string(),
            name: "DeepSeek".to_string(),
            protocol: "openai".to_string(),
            base_url: "https://api.deepseek.com".to_string(),
        },
    ];

    // 打开 provider 选择器
    state.open_provider_selector(providers);
    assert_eq!(state.mode, tui::InteractionMode::SelectingProvider);
    assert_eq!(state.provider_list.len(), 3);
    assert_eq!(state.selected_provider_index, 0);

    // 下移与边界检查
    state.next_provider();
    assert_eq!(state.selected_provider_index, 1);
    state.next_provider();
    assert_eq!(state.selected_provider_index, 2);
    state.next_provider(); // 到底不越界
    assert_eq!(state.selected_provider_index, 2);

    // 上移与边界检查
    state.prev_provider();
    assert_eq!(state.selected_provider_index, 1);
    state.prev_provider();
    assert_eq!(state.selected_provider_index, 0);
    state.prev_provider(); // 到顶不越界
    assert_eq!(state.selected_provider_index, 0);

    // 选择 provider
    state.select_current_provider();
    assert_eq!(state.mode, tui::InteractionMode::Normal);
    assert_eq!(state.current_provider_id, "openai");

    // 取消选择器
    state.open_provider_selector(vec![tui::ProviderItem {
        id: "temp".to_string(),
        name: "Temp".to_string(),
        protocol: "openai".to_string(),
        base_url: "https://temp.com".to_string(),
    }]);
    assert_eq!(state.mode, tui::InteractionMode::SelectingProvider);
    state.cancel_provider_selector();
    assert_eq!(state.mode, tui::InteractionMode::Normal);
    assert!(state.provider_list.is_empty());
}

#[test]
fn test_model_selector_state_logic() {
    let mut state = TuiState::default();
    assert_eq!(state.mode, tui::InteractionMode::Normal);

    let models = vec![
        tui::ModelItem {
            id: "gpt-4o".to_string(),
            name: "GPT-4o".to_string(),
        },
        tui::ModelItem {
            id: "claude-3-7-sonnet".to_string(),
            name: "Claude 3.7 Sonnet".to_string(),
        },
        tui::ModelItem {
            id: "deepseek-reasoner".to_string(),
            name: "DeepSeek R1".to_string(),
        },
    ];

    // 打开 model 选择器
    state.open_model_selector(models);
    assert_eq!(state.mode, tui::InteractionMode::SelectingModel);
    assert_eq!(state.model_list.len(), 3);
    assert_eq!(state.selected_model_index, 0);

    // 下移与边界检查
    state.next_model();
    assert_eq!(state.selected_model_index, 1);
    state.next_model();
    assert_eq!(state.selected_model_index, 2);
    state.next_model(); // 到底不越界
    assert_eq!(state.selected_model_index, 2);

    // 上移与边界检查
    state.prev_model();
    assert_eq!(state.selected_model_index, 1);
    state.prev_model();
    assert_eq!(state.selected_model_index, 0);
    state.prev_model(); // 到顶不越界
    assert_eq!(state.selected_model_index, 0);

    // 选择 model 切换当前模型
    state.select_current_model();
    assert_eq!(state.mode, tui::InteractionMode::Normal);
    assert_eq!(state.model, "gpt-4o");

    // 取消选择器
    state.open_model_selector(vec![tui::ModelItem {
        id: "temp-model".to_string(),
        name: "Temp Model".to_string(),
    }]);
    assert_eq!(state.mode, tui::InteractionMode::SelectingModel);
    state.cancel_model_selector();
    assert_eq!(state.mode, tui::InteractionMode::Normal);
    assert!(state.model_list.is_empty());
}

#[test]
fn test_add_provider_data_flow_and_validation() {
    // 1. 合法 payload 校验通过
    let valid_payload = tui::AddProviderPayload {
        id: "custom-ollama".to_string(),
        name: "Custom Ollama".to_string(),
        protocol: "openai".to_string(),
        base_url: "http://localhost:11434/v1".to_string(),
        api_key: None,
    };
    assert!(valid_payload.validate().is_ok());

    // 2. 空 id 校验失败
    let empty_id = tui::AddProviderPayload {
        id: "   ".to_string(),
        name: "Ollama".to_string(),
        protocol: "openai".to_string(),
        base_url: "http://localhost:11434/v1".to_string(),
        api_key: None,
    };
    assert!(empty_id.validate().is_err());
    assert_eq!(empty_id.validate().unwrap_err(), "Provider ID 不能为空");

    // 3. 空 name 校验失败
    let empty_name = tui::AddProviderPayload {
        id: "ollama".to_string(),
        name: "".to_string(),
        protocol: "openai".to_string(),
        base_url: "http://localhost:11434/v1".to_string(),
        api_key: None,
    };
    assert!(empty_name.validate().is_err());
    assert_eq!(empty_name.validate().unwrap_err(), "Provider 名称不能为空");

    // 4. 非法 URL 校验失败
    let invalid_url = tui::AddProviderPayload {
        id: "ollama".to_string(),
        name: "Ollama".to_string(),
        protocol: "openai".to_string(),
        base_url: "not-a-valid-url".to_string(),
        api_key: None,
    };
    assert!(invalid_url.validate().is_err());
    assert_eq!(invalid_url.validate().unwrap_err(), "Base URL 必须是以 http:// 或 https:// 开头的合法地址");

    // 5. 不支持的协议校验失败
    let invalid_protocol = tui::AddProviderPayload {
        id: "ollama".to_string(),
        name: "Ollama".to_string(),
        protocol: "unsupported-protocol".to_string(),
        base_url: "http://localhost:11434/v1".to_string(),
        api_key: None,
    };
    assert!(invalid_protocol.validate().is_err());
    assert_eq!(invalid_protocol.validate().unwrap_err(), "不支持的协议类型");
}

