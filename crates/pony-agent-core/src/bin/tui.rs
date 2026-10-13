//! pony-tui: Claude Code 风格的终端全屏 Agent 交互界面
//! 基于 ratatui + crossterm 构建，支持对话历史、流式输出、工具状态与多行输入。

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ProviderRegistryStore, ProviderSelectionResolver,
    ResolvedProviderSelection, ThinkingParamPattern,
};
use pony_agent_core::agent::control_plane::{
    HostControlPlane, HostControlPlaneBuilder, StartTurnStreamCommand, StopTurnCommand,
};
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};
use pony_agent_core::agent::runtime::{AgentRuntimeBuilder, TurnInput, TurnStreamEvent};
use pony_agent_core::agent::session::SessionStore;
use pony_agent_core::agent::turn_flow::TurnEventSink;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Wrap},
    Frame, Terminal,
};
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    ToolCall,
    ToolResult,
    System,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp_ms: u64,
}

pub enum UiEvent {
    InputKey(crossterm::event::KeyEvent),
    ScrollWheel(bool), // true = ScrollUp, false = ScrollDown
    StreamDelta(String),
    StreamToolCall(String),
    StreamToolResult(String),
    StreamCompleted,
    StreamFailed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeMode {
    Ready,
    Connecting,
    Thinking,
    Responding,
    ExecutingTool(String),
    WaitingApproval,
    Completed,
    Failed(String),
}

pub fn format_elapsed_seconds(ms: u64) -> String {
    let secs = ms / 1000;
    if secs < 60 {
        format!("{}秒", secs)
    } else {
        let mins = secs / 60;
        let rem_secs = secs % 60;
        if rem_secs > 0 {
            format!("{}分{}秒", mins, rem_secs)
        } else {
            format!("{}分钟", mins)
        }
    }
}

pub fn clean_tool_name_display(name: &str) -> String {
    let base = name.split('·').next().unwrap_or(name).trim();
    let cleaned = base
        .trim_start_matches('●')
        .trim_start_matches('✔')
        .trim();
    cleaned.to_string()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InteractionMode {
    Normal,
    SelectingSession,
    SelectingProvider,
    SelectingModel,
    AddingProvider,
}

pub const AVAILABLE_COMMANDS: &[(&str, &str)] = &[
    ("/resume", "选择并恢复历史会话"),
    ("/provider", "管理与切换模型提供商"),
    ("/models", "切换当前活跃模型"),
    ("/clear", "清空当前消息上下文"),
    ("/cost", "查看当前模型与网关配额信息"),
    ("/help", "查看所有支持的快捷指令"),
    ("/quit", "安全退出 TUI 界面"),
];

#[derive(Debug, Clone)]
pub struct SessionItem {
    pub id: String,
    pub title: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderItem {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelItem {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AddingProviderStep {
    Name,
    BaseUrl,
    ApiKey,
    Commit,
}

#[derive(Debug, Clone, Default)]
pub struct AddProviderForm {
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AddProviderPayload {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    pub api_key: Option<String>,
}

impl AddProviderPayload {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.id.trim().is_empty() {
            return Err("Provider ID 不能为空");
        }
        if self.name.trim().is_empty() {
            return Err("Provider 名称不能为空");
        }
        let url = self.base_url.trim();
        if !url.starts_with("http://") && !url.starts_with("https://") {
            return Err("Base URL 必须是以 http:// 或 https:// 开头的合法地址");
        }
        match self.protocol.trim().to_lowercase().as_str() {
            "openai" | "anthropic" | "gemini" | "ollama" => Ok(()),
            _ => Err("不支持的协议类型"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TuiState {
    pub messages: Vec<ChatMessage>,
    pub input_buffer: String,
    pub is_streaming: bool,
    pub spinner_tick: usize,
    pub session_id: String,
    pub model: String,
    pub current_provider_id: String,
    pub gateway_url: String,
    pub workspace: PathBuf,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub status_text: String,
    pub scroll_offset: u16,
    pub auto_scroll: bool,
    pub mode: InteractionMode,
    pub session_list: Vec<SessionItem>,
    pub selected_session_index: usize,
    pub provider_list: Vec<ProviderItem>,
    pub selected_provider_index: usize,
    pub model_list: Vec<ModelItem>,
    pub selected_model_index: usize,
    pub selected_command_index: usize,
    pub add_provider_step: AddingProviderStep,
    pub add_provider_form: AddProviderForm,
    pub runtime_mode: RuntimeMode,
    pub turn_started_at: Option<std::time::Instant>,
}

impl Default for TuiState {
    fn default() -> Self {
        let env_gateway = std::env::var("PONYLLM_GATEWAY_URL")
            .unwrap_or_else(|_| "http://10.42.0.67:8080".to_string());
        let current_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let canonical_workspace = std::fs::canonicalize(&current_dir).unwrap_or(current_dir);

        Self {
            messages: Vec::new(),
            input_buffer: String::new(),
            is_streaming: false,
            spinner_tick: 0,
            session_id: format!("session-{}", &uuid::Uuid::new_v4().to_string()[..8]),
            model: "gemini-3.8-flash".to_string(),
            current_provider_id: "ponyllm".to_string(),
            gateway_url: env_gateway,
            workspace: canonical_workspace,
            total_input_tokens: 0,
            total_output_tokens: 0,
            status_text: "Ready".to_string(),
            scroll_offset: 0,
            auto_scroll: true,
            mode: InteractionMode::Normal,
            session_list: Vec::new(),
            selected_session_index: 0,
            provider_list: Vec::new(),
            selected_provider_index: 0,
            model_list: Vec::new(),
            selected_model_index: 0,
            selected_command_index: 0,
            add_provider_step: AddingProviderStep::Name,
            add_provider_form: AddProviderForm::default(),
            runtime_mode: RuntimeMode::Ready,
            turn_started_at: None,
        }
    }
}

impl TuiState {
    pub fn transition_runtime_mode(&mut self, next: RuntimeMode) {
        self.runtime_mode = next;
    }

    pub fn runtime_status_display(&self) -> String {
        match &self.runtime_mode {
            RuntimeMode::Ready => "Ready".to_string(),
            RuntimeMode::Connecting => "准备就绪中...".to_string(),
            RuntimeMode::Thinking => "正在思考...".to_string(),
            RuntimeMode::Responding => "正在回复...".to_string(),
            RuntimeMode::ExecutingTool(name) => format!("执行工具: {}...", name),
            RuntimeMode::WaitingApproval => "等待确认中...".to_string(),
            RuntimeMode::Completed => "完成".to_string(),
            RuntimeMode::Failed(reason) => format!("失败: {}", reason),
        }
    }
    pub fn start_add_provider(&mut self) {
        self.mode = InteractionMode::AddingProvider;
        self.add_provider_step = AddingProviderStep::Name;
        self.add_provider_form = AddProviderForm::default();
        self.input_buffer.clear();
    }

    pub fn cancel_add_provider(&mut self) {
        self.mode = InteractionMode::Normal;
        self.add_provider_step = AddingProviderStep::Name;
        self.add_provider_form = AddProviderForm::default();
        self.input_buffer.clear();
    }

    pub fn add_provider_input(&mut self, text: &str) {
        match self.add_provider_step {
            AddingProviderStep::Name => {
                self.add_provider_form.name = text.to_string();
            }
            AddingProviderStep::BaseUrl => {
                self.add_provider_form.base_url = text.to_string();
            }
            AddingProviderStep::ApiKey => {
                let trimmed = text.trim();
                self.add_provider_form.api_key = if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string())
                };
            }
            AddingProviderStep::Commit => {}
        }
    }

    pub fn advance_add_provider_step(&mut self) -> Result<(), &'static str> {
        match self.add_provider_step {
            AddingProviderStep::Name => {
                if self.add_provider_form.name.trim().is_empty() {
                    return Err("Provider 名称不能为空");
                }
                self.add_provider_step = AddingProviderStep::BaseUrl;
                Ok(())
            }
            AddingProviderStep::BaseUrl => {
                let url = self.add_provider_form.base_url.trim();
                if url.is_empty() {
                    return Err("Base URL 不能为空");
                }
                if !url.starts_with("http://") && !url.starts_with("https://") {
                    return Err("Base URL 必须是以 http:// 或 https:// 开头的合法地址");
                }
                self.add_provider_step = AddingProviderStep::ApiKey;
                Ok(())
            }
            AddingProviderStep::ApiKey => {
                self.add_provider_step = AddingProviderStep::Commit;
                Ok(())
            }
            AddingProviderStep::Commit => Ok(()),
        }
    }

    pub fn finalize_add_provider(&mut self) -> Option<AddProviderPayload> {
        let name = self.add_provider_form.name.trim().to_string();
        let base_url = self.add_provider_form.base_url.trim().to_string();
        if name.is_empty() || base_url.is_empty() {
            return None;
        }
        let id = name
            .to_lowercase()
            .replace(|c: char| !c.is_alphanumeric(), "-")
            .trim_matches('-')
            .to_string();
        let final_id = if id.is_empty() {
            format!("provider-{}", &uuid::Uuid::new_v4().to_string()[..6])
        } else {
            id
        };

        let payload = AddProviderPayload {
            id: final_id,
            name,
            protocol: "openai".to_string(),
            base_url,
            api_key: self.add_provider_form.api_key.clone(),
        };

        self.mode = InteractionMode::Normal;
        self.add_provider_step = AddingProviderStep::Name;
        self.add_provider_form = AddProviderForm::default();
        Some(payload)
    }

    pub fn open_provider_selector(&mut self, list: Vec<ProviderItem>) {
        self.provider_list = list;
        self.selected_provider_index = 0;
        self.mode = InteractionMode::SelectingProvider;
    }

    pub fn cancel_provider_selector(&mut self) {
        self.mode = InteractionMode::Normal;
        self.provider_list.clear();
    }

    pub fn next_provider(&mut self) {
        if !self.provider_list.is_empty() && self.selected_provider_index + 1 < self.provider_list.len() {
            self.selected_provider_index += 1;
        }
    }

    pub fn prev_provider(&mut self) {
        if self.selected_provider_index > 0 {
            self.selected_provider_index -= 1;
        }
    }

    pub fn select_current_provider(&mut self) {
        if !self.provider_list.is_empty() && self.selected_provider_index < self.provider_list.len() {
            let p = &self.provider_list[self.selected_provider_index];
            self.current_provider_id = p.id.clone();
            self.status_text = format!("Switched provider to {}", p.name);
        }
        self.mode = InteractionMode::Normal;
    }

    pub fn open_model_selector(&mut self, list: Vec<ModelItem>) {
        self.model_list = list;
        self.selected_model_index = 0;
        self.mode = InteractionMode::SelectingModel;
    }

    pub fn cancel_model_selector(&mut self) {
        self.mode = InteractionMode::Normal;
        self.model_list.clear();
    }

    pub fn next_model(&mut self) {
        if !self.model_list.is_empty() && self.selected_model_index + 1 < self.model_list.len() {
            self.selected_model_index += 1;
        }
    }

    pub fn prev_model(&mut self) {
        if self.selected_model_index > 0 {
            self.selected_model_index -= 1;
        }
    }

    pub fn select_current_model(&mut self) {
        if !self.model_list.is_empty() && self.selected_model_index < self.model_list.len() {
            let m = &self.model_list[self.selected_model_index];
            self.model = m.id.clone();
            self.status_text = format!("Switched model to {}", m.name);
        }
        self.mode = InteractionMode::Normal;
    }

    pub fn open_session_selector(&mut self, list: Vec<SessionItem>) {
        self.session_list = list;
        self.selected_session_index = 0;
        self.mode = InteractionMode::SelectingSession;
    }

    pub fn cancel_session_selector(&mut self) {
        self.mode = InteractionMode::Normal;
        self.session_list.clear();
    }

    pub fn next_session(&mut self) {
        if !self.session_list.is_empty() && self.selected_session_index + 1 < self.session_list.len() {
            self.selected_session_index += 1;
        }
    }

    pub fn prev_session(&mut self) {
        if self.selected_session_index > 0 {
            self.selected_session_index -= 1;
        }
    }

    pub fn matching_commands(&self) -> Vec<(&'static str, &'static str)> {
        if self.input_buffer.starts_with('/') {
            let filter = self.input_buffer.to_lowercase();
            AVAILABLE_COMMANDS
                .iter()
                .copied()
                .filter(|(cmd, _)| cmd.to_lowercase().starts_with(&filter))
                .collect()
        } else {
            Vec::new()
        }
    }
    pub fn scroll_up(&mut self, delta: u16) {
        if self.auto_scroll {
            // 如果之前处于自动贴底，第一次上翻时从当前最大偏移量开始扣减
            self.auto_scroll = false;
        }
        self.scroll_offset = self.scroll_offset.saturating_sub(delta);
    }

    pub fn scroll_down(&mut self, delta: u16, max_offset: u16) {
        let next = self.scroll_offset.saturating_add(delta);
        if next >= max_offset {
            self.scroll_offset = max_offset;
            self.auto_scroll = true; // 滚到底部时恢复自动贴底跟踪
        } else {
            self.scroll_offset = next;
        }
    }
}

pub fn calculate_scroll_offset(total_lines: usize, visible_height: usize) -> u16 {
    if total_lines > visible_height {
        (total_lines - visible_height) as u16
    } else {
        0
    }
}

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn spinner_frame(tick: usize) -> &'static str {
    SPINNER_FRAMES[tick % SPINNER_FRAMES.len()]
}

impl TuiState {
    pub fn add_user_message(&mut self, text: &str) {
        self.messages.push(ChatMessage {
            role: MessageRole::User,
            content: text.to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
        });
    }

    pub fn append_assistant_chunk(&mut self, chunk: &str) {
        if let Some(last) = self.messages.last_mut() {
            if last.role == MessageRole::Assistant {
                last.content.push_str(chunk);
                return;
            }
        }
        self.messages.push(ChatMessage {
            role: MessageRole::Assistant,
            content: chunk.to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
        });
    }

    pub fn add_or_update_tool_event(&mut self, desc: &str, is_done: bool) {
        // 如果末尾就是当前工具条目，直接原位更新状态与文本，彻底解决上下产生两条重复消息的困扰
        if let Some(last) = self.messages.last_mut() {
            if last.role == MessageRole::ToolCall {
                last.content = desc.to_string();
                if is_done {
                    last.role = MessageRole::ToolResult;
                }
                return;
            }
        }
        self.messages.push(ChatMessage {
            role: if is_done { MessageRole::ToolResult } else { MessageRole::ToolCall },
            content: desc.to_string(),
            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
        });
    }

    pub fn add_tool_event(&mut self, is_call: bool, desc: &str) {
        self.add_or_update_tool_event(desc, !is_call);
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
        self.session_id = format!("session-{}", &uuid::Uuid::new_v4().to_string()[..8]);
        self.status_text = "Context cleared".to_string();
    }
}

pub fn format_tool_display(
    name: &str,
    display_zh: Option<&str>,
    summary_or_args: Option<&str>,
    _status: Option<&str>,
) -> String {
    let title = if let Some(zh) = display_zh {
        if !zh.trim().is_empty() && zh != name {
            format!("{} ({})", zh.trim(), name)
        } else {
            name.to_string()
        }
    } else {
        name.to_string()
    };

    if let Some(detail) = summary_or_args {
        let trimmed = detail.trim();
        if !trimmed.is_empty() {
            return format!("{} · {}", title, trimmed);
        }
    }
    title
}

pub fn render_markdown_line(line: &str) -> Vec<Span<'static>> {
    let trimmed = line.trim_start();
    let hash_count = trimmed.chars().take_while(|&c| c == '#').count();
    if hash_count > 0 && trimmed[hash_count..].starts_with(' ') {
        let clean_text = trimmed[hash_count..].trim_start();
        let (marker_symbol, marker_color, text_color) = match hash_count {
            1 => ("■ ", Color::Rgb(140, 195, 255), Color::Rgb(220, 235, 255)),
            2 => ("◆ ", Color::Rgb(120, 180, 255), Color::Rgb(200, 225, 255)),
            3 => ("▲ ", Color::Rgb(100, 160, 240), Color::Rgb(180, 215, 255)),
            _ => ("● ", Color::Rgb(90, 145, 225), Color::Rgb(165, 205, 245)),
        };
        let mut spans = vec![
            Span::styled(marker_symbol, Style::default().fg(marker_color).add_modifier(Modifier::BOLD)),
        ];
        let mut content_spans = parse_inline_formatting(clean_text);
        for s in &mut content_spans {
            s.style = s.style.fg(text_color).add_modifier(Modifier::BOLD);
        }
        spans.extend(content_spans);
        return spans;
    }

    if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
        let mut spans = vec![
            Span::styled("  • ", Style::default().fg(Color::Rgb(100, 200, 240))),
        ];
        spans.extend(parse_inline_formatting(&trimmed[2..]));
        return spans;
    }

    // 数字有序列表例如 "1. ", "2. "
    if let Some(dot_idx) = trimmed.find(". ") {
        let num_part = &trimmed[..dot_idx];
        if !num_part.is_empty() && num_part.chars().all(|c| c.is_ascii_digit()) {
            let mut spans = vec![
                Span::styled(format!("  {}. ", num_part), Style::default().fg(Color::Rgb(130, 190, 250))),
            ];
            spans.extend(parse_inline_formatting(&trimmed[dot_idx + 2..]));
            return spans;
        }
    }

    // 分割线 --- 或 ***
    if trimmed == "---" || trimmed == "***" || trimmed == "___" {
        return vec![Span::styled(
            "────────────────────────────────────────────────────────",
            Style::default().fg(Color::Rgb(60, 65, 80)),
        )];
    }

    // 代码块围栏 ``` 或 ```lang
    if trimmed.starts_with("```") {
        let lang = trimmed.trim_start_matches('`').trim();
        let label = if lang.is_empty() {
            "代码块 ──".to_string()
        } else {
            format!("[{}] ──", lang)
        };
        return vec![
            Span::styled("  ── ", Style::default().fg(Color::Rgb(70, 80, 100))),
            Span::styled(label, Style::default().fg(Color::Rgb(140, 160, 195))),
        ];
    }

    parse_inline_formatting(line)
}

fn parse_inline_formatting(text: &str) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    let mut rest = text;

    while !rest.is_empty() {
        if let Some(bold_start) = rest.find("**") {
            if let Some(bold_end) = rest[bold_start + 2..].find("**") {
                let actual_end = bold_start + 2 + bold_end;
                if bold_start > 0 {
                    spans.push(Span::styled(
                        rest[..bold_start].to_string(),
                        Style::default().fg(Color::Rgb(230, 235, 245)),
                    ));
                }
                spans.push(Span::styled(
                    rest[bold_start + 2..actual_end].to_string(),
                    Style::default()
                        .fg(Color::Rgb(255, 255, 255))
                        .add_modifier(Modifier::BOLD),
                ));
                rest = &rest[actual_end + 2..];
                continue;
            }
        }

        if let Some(code_start) = rest.find('`') {
            if let Some(code_end) = rest[code_start + 1..].find('`') {
                let actual_end = code_start + 1 + code_end;
                if code_start > 0 {
                    spans.push(Span::styled(
                        rest[..code_start].to_string(),
                        Style::default().fg(Color::Rgb(225, 230, 240)),
                    ));
                }
                spans.push(Span::styled(
                    format!(" {} ", &rest[code_start + 1..actual_end]),
                    Style::default()
                        .fg(Color::Rgb(255, 220, 130))
                        .bg(Color::Rgb(45, 50, 68))
                        .add_modifier(Modifier::BOLD),
                ));
                rest = &rest[actual_end + 1..];
                continue;
            }
        }

        spans.push(Span::styled(
            rest.to_string(),
            Style::default().fg(Color::Rgb(230, 235, 245)),
        ));
        break;
    }
    spans
}

pub fn ascii_logo() -> Vec<Line<'static>> {
    vec![
        Line::from(vec![Span::styled(r#"   ____  ____  _   ____  __"#, Style::default().fg(Color::Rgb(140, 180, 255)).add_modifier(Modifier::BOLD))]),
        Line::from(vec![Span::styled(r#"  / __ \/ __ \/ | / /\ \/ /"#, Style::default().fg(Color::Rgb(140, 195, 255)).add_modifier(Modifier::BOLD))]),
        Line::from(vec![Span::styled(r#" / /_/ / / / /  |/ /  \  / "#, Style::default().fg(Color::Rgb(120, 215, 245)).add_modifier(Modifier::BOLD))]),
        Line::from(vec![Span::styled(r#"/ ____/ /_/ / /|  /   / /  "#, Style::default().fg(Color::Rgb(100, 230, 230)).add_modifier(Modifier::BOLD))]),
        Line::from(vec![Span::styled(r#"/_/    \____/_/ |_/   /_/   "#, Style::default().fg(Color::Rgb(110, 240, 200)).add_modifier(Modifier::BOLD))]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("● Autonomous Agent Runtime ", Style::default().fg(Color::Rgb(200, 210, 230))),
            Span::styled("· Type your prompt to start", Style::default().fg(Color::Rgb(120, 130, 150))),
        ]),
        Line::raw(""),
    ]
}

struct DynamicProviderResolver {
    gateway_url: String,
    api_key: Option<String>,
    model: String,
    registry: ProviderRegistryStore,
}

impl ProviderSelectionResolver for DynamicProviderResolver {
    fn resolve_provider_selection(
        &self,
        provider_id: Option<&str>,
        model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
        if let Some(pid) = provider_id {
            if pid != "ponyllm" {
                return self.registry.resolve_selection(Some(pid), model_id);
            }
        }

        let mut base = self.gateway_url.trim_end_matches('/').to_string();
        if !base.ends_with("/v1") {
            base.push_str("/v1");
        }

        ResolvedProviderSelection {
            requested_name: "ponyllm".to_string(),
            provider_name: "ponyllm".to_string(),
            protocol: ProviderProtocol::OpenAiCompletions,
            base_url: base,
            auth_type: ProviderAuthType::Auto,
            api_key_env_var: "PONYLLM_API_KEY".to_string(),
            api_key: self.api_key.clone(),
            model: model_id.unwrap_or(&self.model).to_string(),
            temperature: 0.2,
            max_output_tokens: 4096,
            reasoning_effort: None,
            reasoning_budget_tokens: None,
            capabilities: ProviderModelCapabilities {
                supports_tools: true,
                supports_streaming: true,
                supports_image_input: true,
                supports_reasoning: false,
                context_window_tokens: Some(1_000_000),
                ..Default::default()
            },
            thinking_param_pattern: ThinkingParamPattern::None,
        }
    }
}

struct TuiTurnSink {
    tx: Sender<UiEvent>,
}

impl TurnEventSink for TuiTurnSink {
    fn emit(&self, name: &str, payload: TurnStreamEvent) {
        match name {
            "turn:delta" => {
                if let Some(text) = payload.text {
                    let _ = self.tx.send(UiEvent::StreamDelta(text));
                }
            }
            "turn:tool" => {
                if let Some(activities) = &payload.tool_activities {
                    if let Some(act) = activities.last() {
                        let is_running = act.status == "running" || act.status == "planned";
                        let desc_text = if !act.description.trim().is_empty() {
                            Some(act.description.trim())
                        } else {
                            act.arguments_text.as_deref().or(act.result_text.as_deref())
                        };
                        let short_desc = desc_text.and_then(|d| {
                            let t = d.trim();
                            if t.is_empty() {
                                None
                            } else if t.len() > 60 {
                                Some(format!("{}...", &t[..60]))
                            } else {
                                Some(t.to_string())
                            }
                        });
                        let formatted = format_tool_display(
                            &act.name,
                            act.display_name_zh.as_deref(),
                            short_desc.as_deref(),
                            Some(&act.status),
                        );
                        if is_running {
                            let _ = self.tx.send(UiEvent::StreamToolCall(formatted));
                        } else {
                            let _ = self.tx.send(UiEvent::StreamToolResult(formatted));
                        }
                        return;
                    }
                }
                if let Some(text) = payload.text {
                    let formatted = format_tool_display("Tool", None, Some(&text), None);
                    let _ = self.tx.send(UiEvent::StreamToolCall(formatted));
                }
            }
            "turn:tool_call" => {
                if let Some(activities) = &payload.tool_activities {
                    if let Some(act) = activities.last() {
                        let formatted = format_tool_display(
                            &act.name,
                            act.display_name_zh.as_deref(),
                            Some(&act.description),
                            Some(&act.status),
                        );
                        let _ = self.tx.send(UiEvent::StreamToolCall(formatted));
                        return;
                    }
                }
                if let Some(text) = payload.text {
                    let formatted = format_tool_display("Tool", None, Some(&text), None);
                    let _ = self.tx.send(UiEvent::StreamToolCall(formatted));
                }
            }
            "turn:tool_result" => {
                let (name, display_zh, status, detail) = payload
                    .tool_activities
                    .as_ref()
                    .and_then(|a| a.last())
                    .map(|act| {
                        let desc = if !act.description.trim().is_empty() {
                            act.description.clone()
                        } else {
                            act.result_text.clone().unwrap_or_default()
                        };
                        (
                            act.name.clone(),
                            act.display_name_zh.clone(),
                            act.status.clone(),
                            desc,
                        )
                    })
                    .unwrap_or_else(|| ("Tool".to_string(), None, "ok".to_string(), String::new()));
                let short_detail = if detail.len() > 60 {
                    format!("{}...", &detail[..60])
                } else {
                    detail
                };
                let formatted = format_tool_display(&name, display_zh.as_deref(), Some(&short_detail), Some(&status));
                let _ = self.tx.send(UiEvent::StreamToolResult(formatted));
            }
            "turn:completed" | "turn:output_end" => {
                let _ = self.tx.send(UiEvent::StreamCompleted);
            }
            "turn:failed" => {
                let err = payload.error.unwrap_or_else(|| "Unknown error".to_string());
                let _ = self.tx.send(UiEvent::StreamFailed(err));
            }
            _ => {}
        }
    }
}

fn build_control_plane(workspace: &std::path::Path, gateway_url: &str, model: &str) -> HostControlPlane {
    // 彻底复用桌面端体系：直接使用与桌面端相同的生产环境默认 SessionStore（默认加载与写入 ~/.pony-agent/sessions.sqlite 或 sessions.json）
    // 使得 TUI 创建和更新的会话在桌面端无缝可见与互通。
    let api_key = std::env::var("PONYLLM_API_KEY")
        .or_else(|_| std::env::var("PONY_API_KEY"))
        .ok()
        .or_else(|| {
            let cfg_path = dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config")
                .join("ponyllm")
                .join("ponyllm.toml");
            if let Ok(content) = std::fs::read_to_string(cfg_path) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if trimmed.starts_with("api_key") {
                        if let Some(val) = trimmed.split('=').nth(1) {
                            return Some(val.trim().trim_matches('"').to_string());
                        }
                    }
                }
            }
            None
        });

    let resolver = DynamicProviderResolver {
        gateway_url: gateway_url.to_string(),
        api_key,
        model: model.to_string(),
        registry: ProviderRegistryStore::new(),
    };

    let runtime_builder = AgentRuntimeBuilder::desktop()
        .session_store(SessionStore::new())
        .provider_resolver(Box::new(resolver))
        .workspace_root(workspace.to_path_buf());

    HostControlPlaneBuilder::desktop()
        .runtime_builder(runtime_builder)
        .build()
}

fn ui(f: &mut Frame, state: &mut TuiState) {
    // 背景 100% 铺满终端：直接使用 f.area() 进行垂直切分，色块贴满屏幕边缘，无黑边
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),      // 顶栏 Header
            Constraint::Min(6),         // 聊天视窗
            Constraint::Length(4),      // 输入容器：3行输入文本区域 + 1行底栏状态
        ])
        .split(f.area());

    // 1. Header: 铺满终端，暗调微蓝底色，标签式徽标
    let header_bg = Color::Rgb(28, 32, 42);
    let ws_name = state.workspace.file_name().and_then(|n| n.to_str()).unwrap_or(".");
    let total_header_w = chunks[0].width as usize;
    let right_meta = format!("session: {}  ", state.session_id);
    let left_meta_len = 2 + 8 + 2 + ws_name.len() + 4;
    let header_pad = if total_header_w > left_meta_len + right_meta.len() {
        total_header_w - left_meta_len - right_meta.len()
    } else {
        1
    };

    let header_text = Line::from(vec![
        Span::raw("  "),
        Span::styled(" PONY ", Style::default().fg(Color::Rgb(15, 20, 30)).bg(Color::Rgb(120, 185, 255)).add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(format!("project: {} ", ws_name), Style::default().fg(Color::Rgb(210, 220, 235)).add_modifier(Modifier::BOLD)),
        Span::raw(" ".repeat(header_pad)),
        Span::styled(right_meta, Style::default().fg(Color::Rgb(130, 140, 160))),
    ]);
    let header = Paragraph::new(vec![header_text]).style(Style::default().bg(header_bg));
    f.render_widget(header, chunks[0]);

    // 2. Chat history or Initial Logo: 背景使用深灰青黑底色
    let chat_bg = Color::Rgb(18, 20, 26);
    let mut message_lines: Vec<Line> = Vec::new();

    if state.messages.is_empty() {
        for l in ascii_logo() {
            let mut padded = vec![Span::raw("  ")];
            padded.extend(l.spans);
            message_lines.push(Line::from(padded));
        }
    } else {
        let mut prev_is_tool = false;
        for msg in &state.messages {
            match msg.role {
                MessageRole::User => {
                    message_lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled(" USER ", Style::default().fg(Color::Rgb(20, 15, 28)).bg(Color::Rgb(205, 160, 255)).add_modifier(Modifier::BOLD)),
                        Span::raw(" "),
                        Span::styled(&msg.content, Style::default().fg(Color::Rgb(250, 250, 255)).add_modifier(Modifier::BOLD)),
                    ]));
                    message_lines.push(Line::raw(""));
                    prev_is_tool = false;
                }
                MessageRole::Assistant => {
                    if prev_is_tool {
                        message_lines.push(Line::raw(""));
                    }
                    for line in msg.content.lines() {
                        let mut rendered_spans = vec![Span::raw("  ")];
                        rendered_spans.extend(render_markdown_line(line));
                        message_lines.push(Line::from(rendered_spans));
                    }
                    message_lines.push(Line::raw(""));
                    prev_is_tool = false;
                }
                MessageRole::ToolCall => {
                    message_lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled("● ", Style::default().fg(Color::Rgb(120, 190, 240))),
                        Span::styled(&msg.content, Style::default().fg(Color::Rgb(165, 175, 195))),
                    ]));
                    prev_is_tool = true;
                }
                MessageRole::ToolResult => {
                    message_lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled("✔ ", Style::default().fg(Color::Rgb(80, 210, 130))),
                        Span::styled(&msg.content, Style::default().fg(Color::Rgb(150, 165, 180))),
                    ]));
                    prev_is_tool = true;
                }
                MessageRole::System => {
                    message_lines.push(Line::from(vec![
                        Span::raw("  "),
                        Span::styled("ℹ ", Style::default().fg(Color::Rgb(120, 185, 255))),
                        Span::styled(&msg.content, Style::default().fg(Color::Rgb(170, 180, 200))),
                    ]));
                    message_lines.push(Line::raw(""));
                    prev_is_tool = false;
                }
            }
        }

        // 动态状态占位（在对话历史底部，替换原“正在思考”占位，伴随整数秒耗时）：
        if state.is_streaming {
            let has_elapsed = state.turn_started_at.is_some();
            let elapsed_str = if let Some(started) = state.turn_started_at {
                format!("，用时 {} ···", format_elapsed_seconds(started.elapsed().as_millis() as u64))
            } else {
                String::new()
            };
            let spinner = spinner_frame(state.spinner_tick);
            let (status_text, status_color) = match &state.runtime_mode {
                RuntimeMode::Ready => ("就绪".to_string(), Color::Rgb(140, 210, 160)),
                RuntimeMode::Connecting => {
                    let base = if has_elapsed { "准备就绪中" } else { "准备就绪中..." };
                    (format!("{base}{elapsed_str}"), Color::Rgb(120, 185, 255))
                }
                RuntimeMode::Thinking => {
                    let base = if has_elapsed { "正在思考" } else { "正在思考..." };
                    (format!("{base}{elapsed_str}"), Color::Rgb(170, 150, 240))
                }
                RuntimeMode::Responding => {
                    let base = if has_elapsed { "正在回复" } else { "正在回复..." };
                    (format!("{base}{elapsed_str}"), Color::Rgb(120, 205, 240))
                }
                RuntimeMode::ExecutingTool(name) => {
                    let base = if has_elapsed { format!("执行工具: {name}") } else { format!("执行工具: {name}...") };
                    (format!("{base}{elapsed_str}"), Color::Rgb(240, 180, 80))
                }
                RuntimeMode::WaitingApproval => {
                    let base = if has_elapsed { "等待确认中" } else { "等待确认中..." };
                    (format!("{base}{elapsed_str}"), Color::Rgb(240, 180, 80))
                }
                RuntimeMode::Completed => ("完成".to_string(), Color::Rgb(120, 210, 140)),
                RuntimeMode::Failed(reason) => (format!("失败: {reason}"), Color::Rgb(240, 100, 100)),
            };

            message_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{} {}", spinner, status_text), Style::default().fg(status_color).add_modifier(Modifier::BOLD)),
            ]));
            message_lines.push(Line::raw(""));
        }
    }

    // 估算多行折行后的实际行高（左右内边距分别预留 4 字符缓冲）
    let chat_width = chunks[1].width.saturating_sub(6).max(20) as usize;
    let mut total_wrapped_lines = 0usize;
    for line in &message_lines {
        let line_len: usize = line.spans.iter().map(|s| s.content.chars().count()).sum();
        let rows = if line_len == 0 {
            1
        } else {
            (line_len + chat_width - 1) / chat_width
        };
        total_wrapped_lines += rows.max(1);
    }

    let visible_history_height = chunks[1].height as usize;
    let max_scroll_offset = if total_wrapped_lines > visible_history_height {
        (total_wrapped_lines - visible_history_height) as u16
    } else {
        0
    };

    let effective_scroll_offset = if state.auto_scroll {
        state.scroll_offset = max_scroll_offset;
        max_scroll_offset
    } else {
        state.scroll_offset = state.scroll_offset.min(max_scroll_offset);
        state.scroll_offset
    };

    let chat_widget = Paragraph::new(message_lines)
        .style(Style::default().bg(chat_bg))
        .wrap(Wrap { trim: false })
        .scroll((effective_scroll_offset, 0));
    f.render_widget(chat_widget, chunks[1]);

    // 3. Input Box: 带有输入框边框与底栏状态
    let input_container_bg = Color::Rgb(24, 27, 36);

    let total_width = chunks[2].width as usize;
    let model_tag = format!("  {}  ", state.model);

    let (status_text_disp, status_bg, status_fg) = match &state.runtime_mode {
        RuntimeMode::Ready => (" ● 就绪 ", Color::Rgb(45, 95, 60), Color::Rgb(190, 240, 205)),
        RuntimeMode::Connecting | RuntimeMode::Thinking | RuntimeMode::Responding | RuntimeMode::ExecutingTool(_) => {
            (" ⠋ 执行中 ", Color::Rgb(60, 110, 190), Color::Rgb(255, 255, 255))
        }
        RuntimeMode::WaitingApproval => (" ⏸ 待确认 ", Color::Rgb(180, 110, 30), Color::Rgb(255, 245, 200)),
        RuntimeMode::Completed => (" ✔ 完成 ", Color::Rgb(40, 160, 90), Color::Rgb(255, 255, 255)),
        RuntimeMode::Failed(_) => (" ✕ 失败 ", Color::Rgb(200, 60, 60), Color::Rgb(255, 255, 255)),
    };

    let left_info_len = status_text_disp.chars().count() + 4;
    let right_info_len = model_tag.chars().count() + 4;
    let pad_spaces = if total_width > left_info_len + right_info_len {
        total_width - left_info_len - right_info_len
    } else {
        1
    };

    let cursor_span = if !state.is_streaming {
        Span::styled("█", Style::default().fg(Color::Rgb(100, 210, 240)))
    } else {
        Span::raw("")
    };

    // 支持3行高度输入展示，单行/多行自然排版
    let mut input_content_lines: Vec<Line> = Vec::new();
    let raw_input_lines: Vec<&str> = state.input_buffer.split('\n').collect();
    if raw_input_lines.is_empty() || (raw_input_lines.len() == 1 && raw_input_lines[0].is_empty()) {
        input_content_lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled("> ", Style::default().fg(Color::Rgb(100, 210, 240)).add_modifier(Modifier::BOLD)),
            cursor_span,
        ]));
        input_content_lines.push(Line::raw(""));
        input_content_lines.push(Line::raw(""));
    } else {
        for (i, line) in raw_input_lines.iter().take(3).enumerate() {
            let prefix = if i == 0 {
                Span::styled("> ", Style::default().fg(Color::Rgb(100, 210, 240)).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("  ", Style::default().fg(Color::Rgb(100, 210, 240)))
            };
            let is_last_displayed = i == raw_input_lines.len().min(3) - 1;
            input_content_lines.push(Line::from(vec![
                Span::raw("  "),
                prefix,
                Span::styled(*line, Style::default().fg(Color::Rgb(245, 248, 255)).add_modifier(Modifier::BOLD)),
                if is_last_displayed { cursor_span.clone() } else { Span::raw("") },
            ]));
        }
        while input_content_lines.len() < 3 {
            input_content_lines.push(Line::raw(""));
        }
    }

    let mut input_lines = input_content_lines;
    input_lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(status_text_disp, Style::default().fg(status_fg).bg(status_bg).add_modifier(Modifier::BOLD)),
        Span::raw(" ".repeat(pad_spaces)),
        Span::styled(model_tag, Style::default().fg(Color::Rgb(170, 195, 230)).bg(Color::Rgb(38, 44, 58))),
        Span::raw("  "),
    ]));

    let input_widget = Paragraph::new(input_lines).style(Style::default().bg(input_container_bg));
    f.render_widget(input_widget, chunks[2]);

    // 斜杠命令自动弹出菜单（当用户输入以 / 开头时实时浮动在输入框上方）
    let match_cmds = state.matching_commands();
    if !match_cmds.is_empty() && state.mode == InteractionMode::Normal {
        let menu_height = (match_cmds.len() as u16 + 2).min(8);
        let menu_width = 46u16;
        let menu_x = 4u16;
        let menu_y = chunks[2].y.saturating_sub(menu_height);
        let menu_rect = ratatui::layout::Rect::new(menu_x, menu_y, menu_width, menu_height);

        let mut menu_lines = Vec::new();
        menu_lines.push(Line::from(vec![
            Span::styled(" 命令提示 ", Style::default().fg(Color::Rgb(15, 20, 30)).bg(Color::Rgb(100, 210, 240)).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled("↑↓选择 · Tab/Enter补全", Style::default().fg(Color::Rgb(130, 140, 165))),
        ]));

        for (idx, (cmd, desc)) in match_cmds.iter().enumerate() {
            let is_sel = idx == state.selected_command_index;
            let (prefix, style, desc_style) = if is_sel {
                (" ❯ ", Style::default().fg(Color::Rgb(255, 255, 255)).bg(Color::Rgb(45, 75, 130)).add_modifier(Modifier::BOLD), Style::default().fg(Color::Rgb(180, 215, 255)).bg(Color::Rgb(45, 75, 130)))
            } else {
                ("   ", Style::default().fg(Color::Rgb(120, 200, 240)), Style::default().fg(Color::Rgb(150, 160, 180)))
            };

            menu_lines.push(Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(format!("{:<10}", cmd), style),
                Span::styled(format!(" {}", desc), desc_style),
            ]));
        }

        let menu_popup = Paragraph::new(menu_lines).style(Style::default().bg(Color::Rgb(28, 32, 44)));
        f.render_widget(menu_popup, menu_rect);
    }

    // 如果处于 SelectingSession 弹窗模式，在中央渲染一个无边界柔和浮动选择面板
    if state.mode == InteractionMode::SelectingSession {
        let area = f.area();
        let popup_width = area.width.min(64).max(40);
        let popup_height = area.height.min(18).max(10);
        let popup_x = (area.width.saturating_sub(popup_width)) / 2;
        let popup_y = (area.height.saturating_sub(popup_height)) / 2;
        let popup_rect = ratatui::layout::Rect::new(popup_x, popup_y, popup_width, popup_height);

        let mut lines = Vec::new();
        f.render_widget(ratatui::widgets::Clear, popup_rect);
        lines.push(Line::from(vec![
            Span::styled(" RESUME SESSION ", Style::default().fg(Color::Rgb(15, 17, 23)).bg(Color::Rgb(140, 180, 255)).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled("Select a session to continue (Esc to cancel)", Style::default().fg(Color::Rgb(150, 160, 180))),
        ]));
        lines.push(Line::raw(""));

        if state.session_list.is_empty() {
            lines.push(Line::from(vec![Span::styled("  No previous sessions found.", Style::default().fg(Color::Rgb(140, 145, 160)))]));
        } else {
            for (idx, item) in state.session_list.iter().enumerate() {
                let is_selected = idx == state.selected_session_index;
                let (prefix, row_style) = if is_selected {
                    (" ❯ ", Style::default().fg(Color::Rgb(15, 20, 25)).bg(Color::Rgb(130, 220, 240)).add_modifier(Modifier::BOLD))
                } else {
                    ("   ", Style::default().fg(Color::Rgb(220, 225, 240)))
                };

                let mut title_disp = item.title.clone();
                if title_disp.len() > 30 {
                    title_disp = format!("{}...", &title_disp[..27]);
                }

                lines.push(Line::from(vec![
                    Span::styled(prefix, row_style),
                    Span::styled(format!("{:<30}", title_disp), row_style),
                    Span::raw("  "),
                    Span::styled(format!("[{}]", item.id), Style::default().fg(if is_selected { Color::Rgb(40, 50, 70) } else { Color::Rgb(120, 130, 150) })),
                ]));
            }
        }

        let popup_bg = Color::Rgb(32, 36, 48);
        let popup = Paragraph::new(lines).style(Style::default().bg(popup_bg));
        f.render_widget(popup, popup_rect);
    }

    // Provider 选择弹窗
    if state.mode == InteractionMode::SelectingProvider {
        let area = f.area();
        let popup_width = area.width.min(68).max(40);
        let popup_height = area.height.min(18).max(10);
        let popup_x = (area.width.saturating_sub(popup_width)) / 2;
        let popup_y = (area.height.saturating_sub(popup_height)) / 2;
        let popup_rect = ratatui::layout::Rect::new(popup_x, popup_y, popup_width, popup_height);

        let mut lines = Vec::new();
        f.render_widget(ratatui::widgets::Clear, popup_rect);
        lines.push(Line::from(vec![
            Span::styled(" SELECT PROVIDER ", Style::default().fg(Color::Rgb(15, 17, 23)).bg(Color::Rgb(100, 210, 240)).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled("↑↓ 切换 · Enter 确认 · 'a' 添加新Provider · Esc 取消", Style::default().fg(Color::Rgb(150, 160, 180))),
        ]));
        lines.push(Line::raw(""));

        if state.provider_list.is_empty() {
            lines.push(Line::from(vec![Span::styled("  暂无可用的 Provider 配置。", Style::default().fg(Color::Rgb(140, 145, 160)))]));
        } else {
            for (idx, item) in state.provider_list.iter().enumerate() {
                let is_selected = idx == state.selected_provider_index;
                let is_active = item.id == state.current_provider_id;
                let (prefix, row_style) = if is_selected {
                    (" ❯ ", Style::default().fg(Color::Rgb(15, 20, 25)).bg(Color::Rgb(100, 210, 240)).add_modifier(Modifier::BOLD))
                } else {
                    ("   ", Style::default().fg(Color::Rgb(220, 225, 240)))
                };

                let active_badge = if is_active {
                    Span::styled(" (当前)", Style::default().fg(if is_selected { Color::Rgb(20, 40, 60) } else { Color::Rgb(80, 200, 120) }).add_modifier(Modifier::BOLD))
                } else {
                    Span::raw("")
                };

                lines.push(Line::from(vec![
                    Span::styled(prefix, row_style),
                    Span::styled(format!("{:<20}", item.name), row_style),
                    active_badge,
                    Span::raw(" "),
                    Span::styled(format!("[{}]", item.protocol), Style::default().fg(if is_selected { Color::Rgb(40, 50, 70) } else { Color::Rgb(130, 140, 160) })),
                    Span::raw("  "),
                    Span::styled(format!("{}", item.base_url), Style::default().fg(if is_selected { Color::Rgb(40, 50, 70) } else { Color::Rgb(100, 110, 130) })),
                ]));
            }
        }

        let popup_bg = Color::Rgb(32, 36, 48);
        let popup = Paragraph::new(lines).style(Style::default().bg(popup_bg));
        f.render_widget(popup, popup_rect);
    }

    // Model 选择弹窗
    if state.mode == InteractionMode::SelectingModel {
        let area = f.area();
        let popup_width = area.width.min(64).max(40);
        let popup_height = area.height.min(18).max(10);
        let popup_x = (area.width.saturating_sub(popup_width)) / 2;
        let popup_y = (area.height.saturating_sub(popup_height)) / 2;
        let popup_rect = ratatui::layout::Rect::new(popup_x, popup_y, popup_width, popup_height);

        let mut lines = Vec::new();
        f.render_widget(ratatui::widgets::Clear, popup_rect);
        lines.push(Line::from(vec![
            Span::styled(" SELECT MODEL ", Style::default().fg(Color::Rgb(15, 17, 23)).bg(Color::Rgb(140, 220, 140)).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled("↑↓ 切换 · Enter 确认 · Esc 取消", Style::default().fg(Color::Rgb(150, 160, 180))),
        ]));
        lines.push(Line::raw(""));

        if state.model_list.is_empty() {
            lines.push(Line::from(vec![Span::styled("  当前 Provider 无可选模型。", Style::default().fg(Color::Rgb(140, 145, 160)))]));
        } else {
            for (idx, item) in state.model_list.iter().enumerate() {
                let is_selected = idx == state.selected_model_index;
                let is_active = item.id == state.model;
                let (prefix, row_style) = if is_selected {
                    (" ❯ ", Style::default().fg(Color::Rgb(15, 20, 25)).bg(Color::Rgb(140, 220, 140)).add_modifier(Modifier::BOLD))
                } else {
                    ("   ", Style::default().fg(Color::Rgb(220, 225, 240)))
                };

                let active_badge = if is_active {
                    Span::styled(" (当前)", Style::default().fg(if is_selected { Color::Rgb(20, 40, 60) } else { Color::Rgb(80, 200, 120) }).add_modifier(Modifier::BOLD))
                } else {
                    Span::raw("")
                };

                lines.push(Line::from(vec![
                    Span::styled(prefix, row_style),
                    Span::styled(format!("{:<26}", item.name), row_style),
                    active_badge,
                    Span::raw(" "),
                    Span::styled(format!("[{}]", item.id), Style::default().fg(if is_selected { Color::Rgb(40, 50, 70) } else { Color::Rgb(120, 130, 150) })),
                ]));
            }
        }

        let popup_bg = Color::Rgb(32, 36, 48);
        let popup = Paragraph::new(lines).style(Style::default().bg(popup_bg));
        f.render_widget(popup, popup_rect);
    }

    // AddingProvider 模式分步弹窗渲染
    if state.mode == InteractionMode::AddingProvider {
        let area = f.area();
        let popup_width = area.width.min(68).max(40);
        let popup_height = 11u16;
        let popup_x = (area.width.saturating_sub(popup_width)) / 2;
        let popup_y = (area.height.saturating_sub(popup_height)) / 2;
        let popup_rect = ratatui::layout::Rect::new(popup_x, popup_y, popup_width, popup_height);

        f.render_widget(ratatui::widgets::Clear, popup_rect);

        let (step_title, prompt_text) = match state.add_provider_step {
            AddingProviderStep::Name => ("1/3: 名称", "请输入 Provider 名称 (如 Ollama, OpenRouter):"),
            AddingProviderStep::BaseUrl => ("2/3: 服务地址", "请输入 Base URL (如 http://127.0.0.1:11434/v1):"),
            AddingProviderStep::ApiKey => ("3/3: 凭据密钥", "请输入 API Key (本地服务可留空直接回车):"),
            AddingProviderStep::Commit => ("提交确认", "正在保存 Provider 配置..."),
        };

        let mut lines = Vec::new();
        lines.push(Line::from(vec![
            Span::styled(" ADD PROVIDER ", Style::default().fg(Color::Rgb(15, 17, 23)).bg(Color::Rgb(240, 180, 100)).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
            Span::styled(format!("步骤 {} · Enter 下一步 · Esc 取消", step_title), Style::default().fg(Color::Rgb(150, 160, 180))),
        ]));
        lines.push(Line::raw(""));
        lines.push(Line::from(vec![
            Span::styled(format!("  {}", prompt_text), Style::default().fg(Color::Rgb(200, 210, 230))),
        ]));
        lines.push(Line::raw(""));

        let input_disp = format!("  ❯ {}█", state.input_buffer);
        lines.push(Line::from(vec![
            Span::styled(input_disp, Style::default().fg(Color::Rgb(100, 210, 240)).add_modifier(Modifier::BOLD)),
        ]));

        if !state.status_text.is_empty() && (state.status_text.contains("错误") || state.status_text.contains("不能")) {
            lines.push(Line::raw(""));
            lines.push(Line::from(vec![
                Span::styled(format!("  [!] {}", state.status_text), Style::default().fg(Color::Rgb(240, 100, 100))),
            ]));
        }

        let popup_bg = Color::Rgb(32, 36, 48);
        let popup = Paragraph::new(lines).style(Style::default().bg(popup_bg));
        f.render_widget(popup, popup_rect);
    }
}

fn run_tui() -> Result<(), Box<dyn std::error::Error>> {
    // 确保去除可能残留的 NO_COLOR 环境变量，保证终端呈现完整丰富的高清色彩
    unsafe {
        std::env::remove_var("NO_COLOR");
        if std::env::var("COLORTERM").is_err() {
            std::env::set_var("COLORTERM", "truecolor");
        }
    }
    // 将底层 Rust 内核对 stderr/stdout 的日志直接重定向到文件或忽略，
    // 避免 [pony-runtime]、[pony-provider] 等原生 println/eprintln 打印污染 crossterm 屏幕，
    // 防止全屏 TUI 界面被破坏退化为终端滚屏输出。
    let log_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pony-agent")
        .join("tui-logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_file = log_dir.join("tui-runtime.log");
    if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&log_file) {
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        unsafe {
            libc::dup2(fd, libc::STDERR_FILENO);
        }
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = TuiState::default();
    let control_plane = Arc::new(build_control_plane(&state.workspace, &state.gateway_url, &state.model));

    let (tx, rx): (Sender<UiEvent>, Receiver<UiEvent>) = channel();
    let current_turn_id: Arc<std::sync::Mutex<Option<String>>> = Arc::new(std::sync::Mutex::new(None));

    // 键盘与鼠标事件轮询线程
    let tx_events = tx.clone();
    std::thread::spawn(move || {
        // 确保在此线程中启用 ANSI 颜色支持
        unsafe {
            std::env::remove_var("NO_COLOR");
        }
        loop {
            if event::poll(Duration::from_millis(50)).unwrap_or(false) {
                match event::read() {
                    Ok(Event::Key(key)) => {
                        if key.kind == crossterm::event::KeyEventKind::Press {
                            let _ = tx_events.send(UiEvent::InputKey(key));
                        }
                    }
                    Ok(Event::Mouse(mouse)) => {
                        use crossterm::event::MouseEventKind;
                        match mouse.kind {
                            MouseEventKind::ScrollUp => {
                                let _ = tx_events.send(UiEvent::ScrollWheel(true));
                            }
                            MouseEventKind::ScrollDown => {
                                let _ = tx_events.send(UiEvent::ScrollWheel(false));
                            }
                            _ => {}
                        }
                    }
                    _ => {}
                }
            }
        }
    });

    let mut last_tick_instant = std::time::Instant::now();

    loop {
        // 当处于流式思考状态时，按 80ms 节奏驱动 ASCII Spinner 动态旋转
        if state.is_streaming && last_tick_instant.elapsed() >= Duration::from_millis(80) {
            state.spinner_tick = state.spinner_tick.wrapping_add(1);
            last_tick_instant = std::time::Instant::now();
        }

        terminal.draw(|f| ui(f, &mut state))?;

        // 提高事件响应频率（16ms 约 60fps），确保模型 Token 一旦到达能瞬间呈现打字机 Stream 视觉效果
        if let Ok(event) = rx.recv_timeout(Duration::from_millis(16)) {
            match event {
                UiEvent::ScrollWheel(is_up) => {
                    if is_up {
                        state.scroll_up(3);
                    } else {
                        state.scroll_down(3, u16::MAX);
                    }
                }
                UiEvent::InputKey(key) => {
                    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                        break;
                    }

                    // 1. 会话选择弹窗模式下的按键处理
                    if state.mode == InteractionMode::SelectingSession {
                        match key.code {
                            KeyCode::Esc => {
                                state.cancel_session_selector();
                            }
                            KeyCode::Up => {
                                state.prev_session();
                            }
                            KeyCode::Down => {
                                state.next_session();
                            }
                            KeyCode::Enter => {
                                if !state.session_list.is_empty() {
                                    let chosen = state.session_list[state.selected_session_index].clone();
                                    state.session_id = chosen.id.clone();
                                    state.messages.clear();

                                    // 彻底重构历史会话恢复：优先使用权威会话快照 session.history 与 turn_trace_history，确保 1:1 准确恢复
                                    let runtime_view = control_plane.load_session_runtime_view(pony_agent_core::agent::control_plane::SessionRuntimeViewQuery {
                                        session_id: Some(chosen.id.clone()),
                                        turn_id: None,
                                        node_id: None,
                                        run_id: None,
                                    });
                                    let session_snapshot = runtime_view.session;
                                    let mut trace_map: std::collections::HashMap<String, &pony_agent_core::agent::session::TurnTraceRecord> = std::collections::HashMap::new();
                                    for trace in &session_snapshot.turn_trace_history {
                                        trace_map.insert(trace.turn_id.clone(), trace);
                                    }

                                    for hist in &session_snapshot.history {
                                        match hist.role.as_str() {
                                            "user" => {
                                                state.messages.push(ChatMessage {
                                                    role: MessageRole::User,
                                                    content: hist.content.clone(),
                                                    timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                                });
                                            }
                                            "assistant" => {
                                                // 如果该轮包含工具调用，先将该轮的工具执行结果反显在 Assistant 之前
                                                if let Some(turn_id) = &hist.turn_id {
                                                    if let Some(trace) = trace_map.get(turn_id) {
                                                        for tool in &trace.tool_activities {
                                                            if tool.status != "planned" {
                                                                let detail_preview = tool.arguments_text.as_deref().or(tool.result_text.as_deref());
                                                                let formatted = format_tool_display(
                                                                    &tool.name,
                                                                    tool.display_name_zh.as_deref(),
                                                                    detail_preview,
                                                                    Some(&tool.status),
                                                                );
                                                                state.messages.push(ChatMessage {
                                                                    role: MessageRole::ToolResult,
                                                                    content: formatted,
                                                                    timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                                                });
                                                            }
                                                        }
                                                    }
                                                }

                                                state.messages.push(ChatMessage {
                                                    role: MessageRole::Assistant,
                                                    content: hist.content.clone(),
                                                    timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                                });
                                            }
                                            _ => {}
                                        }
                                    }

                                    // 若 history 为空，再尝试 fallback 到 message_state
                                    if state.messages.is_empty() {
                                        for entry in runtime_view.message_state.messages {
                                            match entry.role.as_str() {
                                                "user" => {
                                                    state.messages.push(ChatMessage {
                                                        role: MessageRole::User,
                                                        content: entry.content,
                                                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                                    });
                                                }
                                                "assistant" => {
                                                    state.messages.push(ChatMessage {
                                                        role: MessageRole::Assistant,
                                                        content: entry.content,
                                                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                                    });
                                                }
                                                "tool" | "tool_call" | "tool_result" => {
                                                    let display = format_tool_display(
                                                        entry.tool_name.as_deref().unwrap_or("tool"),
                                                        entry.display_name_zh.as_deref(),
                                                        entry.detail.as_deref(),
                                                        entry.status.as_deref(),
                                                    );
                                                    state.messages.push(ChatMessage {
                                                        role: MessageRole::ToolResult,
                                                        content: display,
                                                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                                    });
                                                }
                                                _ => {}
                                            }
                                        }
                                    }

                                    state.scroll_offset = u16::MAX;
                                    state.auto_scroll = true; // 确保加载完毕后消息贴底显示
                                    state.mode = InteractionMode::Normal;
                                    state.status_text = format!("Resumed {}", chosen.id);
                                } else {
                                    state.cancel_session_selector();
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // 1b. Provider 选择弹窗模式下的按键处理
                    if state.mode == InteractionMode::SelectingProvider {
                        match key.code {
                            KeyCode::Esc => {
                                state.cancel_provider_selector();
                            }
                            KeyCode::Char('a') | KeyCode::Char('A') | KeyCode::Char('+') => {
                                state.start_add_provider();
                            }
                            KeyCode::Up => {
                                state.prev_provider();
                            }
                            KeyCode::Down => {
                                state.next_provider();
                            }
                            KeyCode::Enter => {
                                state.select_current_provider();
                                state.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!("已切换提供商为: {}", state.current_provider_id),
                                    timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                });
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // 1c. Model 选择弹窗模式下的按键处理
                    if state.mode == InteractionMode::SelectingModel {
                        match key.code {
                            KeyCode::Esc => {
                                state.cancel_model_selector();
                            }
                            KeyCode::Up => {
                                state.prev_model();
                            }
                            KeyCode::Down => {
                                state.next_model();
                            }
                            KeyCode::Enter => {
                                state.select_current_model();
                                state.messages.push(ChatMessage {
                                    role: MessageRole::System,
                                    content: format!("已切换模型为: {}", state.model),
                                    timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                });
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // 1d. AddingProvider 模式下的按键处理
                    if state.mode == InteractionMode::AddingProvider {
                        match key.code {
                            KeyCode::Esc => {
                                state.cancel_add_provider();
                            }
                            KeyCode::Backspace => {
                                state.input_buffer.pop();
                            }
                            KeyCode::Char(c) => {
                                state.input_buffer.push(c);
                            }
                            KeyCode::Enter => {
                                let input_val = state.input_buffer.trim().to_string();
                                state.add_provider_input(&input_val);
                                state.input_buffer.clear();

                                match state.advance_add_provider_step() {
                                    Ok(_) => {
                                        state.status_text.clear();
                                        if state.add_provider_step == AddingProviderStep::Commit {
                                            if let Some(payload) = state.finalize_add_provider() {
                                                // 持久化落地保存至 ProviderRegistryStore
                                                let registry = ProviderRegistryStore::new();
                                                let mut view = registry.load_view();
                                                let new_id = payload.id.clone();
                                                let new_name = payload.name.clone();

                                                let config_view = pony_agent_core::agent::config::ProviderConfigView {
                                                    id: new_id.clone(),
                                                    name: new_name.clone(),
                                                    protocol: pony_agent_core::agent::provider::ProviderProtocol::OpenAiCompletions,
                                                    base_url: payload.base_url.clone(),
                                                    auth_type: pony_agent_core::agent::provider::ProviderAuthType::Auto,
                                                    api_key_env_var: format!("{}_API_KEY", new_id.to_uppercase().replace('-', "_")),
                                                    api_key_value: payload.api_key.clone().unwrap_or_default(),
                                                    api_key_present: payload.api_key.is_some(),
                                                    supported_protocols: vec![pony_agent_core::agent::provider::ProviderProtocol::OpenAiCompletions],
                                                    endpoints: vec![],
                                                    models: vec![],
                                                    selected_model_id: None,
                                                };
                                                view.providers.push(config_view);
                                                view.selected_provider_id = Some(new_id.clone());

                                                match registry.save_view(view) {
                                                    Ok(_) => {
                                                        state.current_provider_id = new_id.clone();
                                                        state.messages.push(ChatMessage {
                                                            role: MessageRole::System,
                                                            content: format!("成功添加并切换至新提供商: {} ({})", new_name, new_id),
                                                            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                                        });
                                                        state.status_text = format!("Added provider {}", new_name);
                                                    }
                                                    Err(e) => {
                                                        state.messages.push(ChatMessage {
                                                            role: MessageRole::System,
                                                            content: format!("保存提供商失败: {}", e),
                                                            timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                                        });
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        state.status_text = err.to_string();
                                    }
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }

                    // 2. 仅在正在运行（Streaming）态下 Esc 触发 Interrupt；静态下 Esc 仅用于清空输入缓冲区，不退出程序
                    if key.code == KeyCode::Esc {
                        if state.is_streaming {
                            if let Some(turn_id) = current_turn_id.lock().unwrap().take() {
                                control_plane.stop_turn(StopTurnCommand { turn_id });
                            }
                            state.is_streaming = false;
                            state.status_text = "Interrupted".to_string();
                            state.messages.push(ChatMessage {
                                role: MessageRole::System,
                                content: "Turn was interrupted.".to_string(),
                                timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                            });
                        } else {
                            // 静态时不退出，只清空输入框
                            state.input_buffer.clear();
                        }
                        continue;
                    }

                    if state.is_streaming {
                        continue;
                    }

                    match key.code {
                        KeyCode::Tab => {
                            let match_cmds = state.matching_commands();
                            if !match_cmds.is_empty() {
                                let idx = state.selected_command_index.min(match_cmds.len() - 1);
                                state.input_buffer = match_cmds[idx].0.to_string();
                            }
                        }
                        KeyCode::Enter => {
                            let match_cmds = state.matching_commands();
                            if !match_cmds.is_empty() && state.input_buffer.starts_with('/') && !state.input_buffer.ends_with(' ') && state.input_buffer != match_cmds[state.selected_command_index.min(match_cmds.len() - 1)].0 {
                                let idx = state.selected_command_index.min(match_cmds.len() - 1);
                                state.input_buffer = match_cmds[idx].0.to_string();
                                continue;
                            }
                            let input = state.input_buffer.trim().to_string();
                            if !input.is_empty() {
                                state.input_buffer.clear();
                                state.selected_command_index = 0;

                                if input == "/resume" {
                                    let raw_sessions = control_plane.list_sessions();
                                    let items: Vec<SessionItem> = raw_sessions
                                        .into_iter()
                                        .map(|s| SessionItem {
                                            id: s.conversation_id,
                                            title: s.title,
                                            updated_at: s.updated_at_ms.to_string(),
                                        })
                                        .collect();
                                    state.open_session_selector(items);
                                    continue;
                                }

                                if input == "/provider" {
                                    let registry = ProviderRegistryStore::new();
                                    let view = registry.load_view();
                                    let items: Vec<ProviderItem> = view
                                        .providers
                                        .into_iter()
                                        .map(|p| ProviderItem {
                                            id: p.id,
                                            name: p.name,
                                            protocol: format!("{:?}", p.protocol).to_lowercase(),
                                            base_url: p.base_url,
                                        })
                                        .collect();
                                    state.open_provider_selector(items);
                                    continue;
                                }

                                if input == "/models" {
                                    let registry = ProviderRegistryStore::new();
                                    let view = registry.load_view();
                                    let current_prov = view
                                        .providers
                                        .into_iter()
                                        .find(|p| p.id == state.current_provider_id)
                                        .or_else(|| {
                                            view.selected_provider_id
                                                .as_ref()
                                                .and_then(|id| registry.load_view().providers.into_iter().find(|p| &p.id == id))
                                        });

                                    let items: Vec<ModelItem> = if let Some(p) = current_prov {
                                        p.models
                                            .into_iter()
                                            .map(|m| ModelItem {
                                                id: m.id,
                                                name: m.name,
                                            })
                                            .collect()
                                    } else {
                                        Vec::new()
                                    };
                                    state.open_model_selector(items);
                                    continue;
                                }

                                // 严格遵从 /quit 或 /exit 退出规范
                                if input == "/exit" || input == "/quit" {
                                    break;
                                }
                                if input == "/clear" {
                                    state.clear_messages();
                                    continue;
                                }
                                if input == "/help" {
                                    state.messages.push(ChatMessage {
                                        role: MessageRole::System,
                                        content: "Commands: /resume (select session), /provider, /models, /clear, /cost, /quit".to_string(),
                                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                    });
                                    continue;
                                }
                                if input == "/cost" {
                                    state.messages.push(ChatMessage {
                                        role: MessageRole::System,
                                        content: format!("Model: {} | Gateway: {}", state.model, state.gateway_url),
                                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                                    });
                                    continue;
                                }

                                state.add_user_message(&input);
                                state.is_streaming = true;
                                state.auto_scroll = true; // 发送新消息时立即恢复自动贴底滚动
                                state.status_text = "Thinking...".to_string();
                                state.transition_runtime_mode(RuntimeMode::Connecting);
                                state.turn_started_at = Some(std::time::Instant::now());

                                let turn_id_str = format!("turn-{}", uuid::Uuid::new_v4());
                                *current_turn_id.lock().unwrap() = Some(turn_id_str.clone());

                                let cp = control_plane.clone();
                                let sess_id = state.session_id.clone();
                                let prov_id = state.current_provider_id.clone();
                                let mdl_id = state.model.clone();
                                let tx_stream = tx.clone();

                                std::thread::spawn(move || {
                                    let _guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
                                    let sink = TuiTurnSink { tx: tx_stream };
                                    let cmd = StartTurnStreamCommand {
                                        turn_id: turn_id_str,
                                        input: TurnInput {
                                            message: input,
                                            display_message: None,
                                            provider_id: Some(prov_id),
                                            model_id: Some(mdl_id),
                                            reasoning_effort: None,
                                            workspace_mode: None,
                                            session_id: Some(sess_id),
                                            node_id: None,
                                            history: Vec::new(),
                                            images: Vec::new(),
                                            workspace_id: None,
                                        },
                                    };
                                    cp.start_turn_stream(&sink, cmd);
                                });
                            }
                        }
                        KeyCode::Char(c) => {
                            if c != '\r' && c != '\n' {
                                state.input_buffer.push(c);
                            }
                        }
                        KeyCode::Backspace => {
                            state.input_buffer.pop();
                        }
                        KeyCode::Up => {
                            let match_cmds = state.matching_commands();
                            if !match_cmds.is_empty() {
                                if state.selected_command_index > 0 {
                                    state.selected_command_index -= 1;
                                }
                            } else {
                                state.scroll_up(2);
                            }
                        }
                        KeyCode::Down => {
                            let match_cmds = state.matching_commands();
                            if !match_cmds.is_empty() {
                                if state.selected_command_index + 1 < match_cmds.len() {
                                    state.selected_command_index += 1;
                                }
                            } else {
                                state.scroll_down(2, u16::MAX);
                            }
                        }
                        KeyCode::PageUp => {
                            state.scroll_up(10);
                        }
                        KeyCode::PageDown => {
                            state.scroll_down(10, u16::MAX);
                        }
                        _ => {}
                    }
                }
                UiEvent::StreamDelta(delta) => {
                    state.append_assistant_chunk(&delta);
                    if !matches!(state.runtime_mode, RuntimeMode::ExecutingTool(_)) {
                        state.transition_runtime_mode(RuntimeMode::Responding);
                    }
                }
                UiEvent::StreamToolCall(call) => {
                    let tool_name = clean_tool_name_display(&call);
                    state.transition_runtime_mode(RuntimeMode::ExecutingTool(tool_name));
                    state.add_or_update_tool_event(&call, false);
                }
                UiEvent::StreamToolResult(res) => {
                    state.add_or_update_tool_event(&res, true);
                    state.transition_runtime_mode(RuntimeMode::Responding);
                }
                UiEvent::StreamCompleted => {
                    state.is_streaming = false;
                    state.transition_runtime_mode(RuntimeMode::Completed);
                    state.status_text = "Turn completed".to_string();
                }
                UiEvent::StreamFailed(err) => {
                    state.is_streaming = false;
                    state.transition_runtime_mode(RuntimeMode::Failed(err.clone()));
                    state.status_text = format!("Turn failed: {}", err);
                    state.messages.push(ChatMessage {
                        role: MessageRole::System,
                        content: format!("Error: {}", err),
                        timestamp_ms: chrono::Utc::now().timestamp_millis() as u64,
                    });
                }
            }
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    Ok(())
}

fn main() {
    let _guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
    if let Err(e) = run_tui() {
        eprintln!("TUI Error: {e}");
        std::process::exit(1);
    }
}
