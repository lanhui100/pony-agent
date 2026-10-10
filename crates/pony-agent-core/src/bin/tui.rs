//! pony-tui: Claude Code 风格的终端全屏 Agent 交互界面
//! 基于 ratatui + crossterm 构建，支持对话历史、流式输出、工具状态与多行输入。

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use pony_agent_core::agent::config::{
    ProviderModelCapabilities, ProviderSelectionResolver, ResolvedProviderSelection,
    ThinkingParamPattern,
};
use pony_agent_core::agent::control_plane::{
    HostControlPlane, HostControlPlaneBuilder, StartTurnStreamCommand, StopTurnCommand,
};
use pony_agent_core::agent::graph::GraphRunStore;
use pony_agent_core::agent::provider::{ProviderAuthType, ProviderProtocol};
use pony_agent_core::agent::runtime::{AgentRuntimeBuilder, TurnInput, TurnStreamEvent};
use pony_agent_core::agent::session::{FileSessionBackend, SessionStore};
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
pub enum InteractionMode {
    Normal,
    SelectingSession,
}

#[derive(Debug, Clone)]
pub struct SessionItem {
    pub id: String,
    pub title: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub struct TuiState {
    pub messages: Vec<ChatMessage>,
    pub input_buffer: String,
    pub is_streaming: bool,
    pub spinner_tick: usize,
    pub session_id: String,
    pub model: String,
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
        }
    }
}

impl TuiState {
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
    if trimmed.starts_with("### ") {
        let clean_text = &trimmed[4..];
        return vec![
            Span::styled("### ", Style::default().fg(Color::Rgb(100, 160, 240))),
            Span::styled(
                clean_text.to_string(),
                Style::default()
                    .fg(Color::Rgb(160, 210, 255))
                    .add_modifier(Modifier::BOLD),
            ),
        ];
    } else if trimmed.starts_with("## ") {
        let clean_text = &trimmed[3..];
        return vec![
            Span::styled("## ", Style::default().fg(Color::Rgb(120, 180, 255))),
            Span::styled(
                clean_text.to_string(),
                Style::default()
                    .fg(Color::Rgb(180, 220, 255))
                    .add_modifier(Modifier::BOLD),
            ),
        ];
    } else if trimmed.starts_with("# ") {
        let clean_text = &trimmed[2..];
        return vec![
            Span::styled("# ", Style::default().fg(Color::Rgb(140, 195, 255))),
            Span::styled(
                clean_text.to_string(),
                Style::default()
                    .fg(Color::Rgb(200, 230, 255))
                    .add_modifier(Modifier::BOLD),
            ),
        ];
    } else if trimmed.starts_with('#') {
        let clean_text = trimmed.trim_start_matches('#').trim_start();
        return vec![Span::styled(
            clean_text.to_string(),
            Style::default()
                .fg(Color::Rgb(160, 210, 255))
                .add_modifier(Modifier::BOLD),
        )];
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
            "代码块".to_string()
        } else {
            format!("代码 [{}]", lang)
        };
        return vec![
            Span::styled("  ┌─ ", Style::default().fg(Color::Rgb(80, 90, 115))),
            Span::styled(label, Style::default().fg(Color::Rgb(140, 160, 195)).add_modifier(Modifier::BOLD)),
            Span::styled(" ─────────────────────────────────", Style::default().fg(Color::Rgb(60, 70, 90))),
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
}

impl ProviderSelectionResolver for DynamicProviderResolver {
    fn resolve_provider_selection(
        &self,
        _provider_id: Option<&str>,
        _model_id: Option<&str>,
    ) -> ResolvedProviderSelection {
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
            model: self.model.clone(),
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
            "turn:tool_call" => {
                let (name, display_zh, detail) = payload
                    .tool_activities
                    .as_ref()
                    .and_then(|a| a.last())
                    .map(|act| {
                        (
                            act.name.clone(),
                            act.display_name_zh.clone(),
                            act.description.clone(),
                        )
                    })
                    .unwrap_or_else(|| ("Tool".to_string(), None, payload.text.unwrap_or_default()));
                let formatted = format_tool_display(&name, display_zh.as_deref(), Some(&detail), None);
                let _ = self.tx.send(UiEvent::StreamToolCall(formatted));
            }
            "turn:tool_result" => {
                let (name, display_zh, status, detail) = payload
                    .tool_activities
                    .as_ref()
                    .and_then(|a| a.last())
                    .map(|act| {
                        (
                            act.name.clone(),
                            act.display_name_zh.clone(),
                            act.status.clone(),
                            act.result_text.clone().unwrap_or_default(),
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
            Constraint::Length(4),      // 输入容器：1行提示输入 + 1行空隙 + 1行底栏状态
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
                        Span::styled("⚡ ", Style::default().fg(Color::Rgb(240, 190, 80))),
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

        if state.is_streaming {
            let spinner = spinner_frame(state.spinner_tick);
            message_lines.push(Line::from(vec![
                Span::raw("  "),
                Span::styled(format!("{} 思考执行中...", spinner), Style::default().fg(Color::Rgb(120, 195, 255)).add_modifier(Modifier::BOLD)),
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

    // 3. Input Box: 极简纯净的深色输入面板，带有清晰的底栏状态指标
    let input_container_bg = Color::Rgb(26, 29, 38);
    let prompt_prefix = Span::styled(" ❯ ", Style::default().fg(Color::Rgb(100, 210, 240)).add_modifier(Modifier::BOLD));

    let total_width = chunks[2].width as usize;
    let model_tag = format!("  {}  ", state.model);
    let (status_text_disp, status_bg, status_fg) = if state.is_streaming {
        (" ⠋ 执行中 ", Color::Rgb(70, 130, 230), Color::Rgb(255, 255, 255))
    } else if state.status_text.contains("completed") {
        (" ✔ 完成 ", Color::Rgb(40, 160, 90), Color::Rgb(255, 255, 255))
    } else if state.status_text.contains("failed") || state.status_text.contains("Interrupted") {
        (" ✕ 停止 ", Color::Rgb(200, 60, 60), Color::Rgb(255, 255, 255))
    } else {
        (" ● 就绪 ", Color::Rgb(45, 95, 60), Color::Rgb(190, 240, 205))
    };

    let left_info_len = status_text_disp.chars().count() + 4;
    let right_info_len = model_tag.chars().count() + 4;
    let pad_spaces = if total_width > left_info_len + right_info_len {
        total_width - left_info_len - right_info_len
    } else {
        1
    };

    let input_lines = vec![
        Line::from(vec![
            Span::raw("  "),
            Span::styled("● ", Style::default().fg(Color::Rgb(100, 220, 240))),
            Span::styled(
                if state.input_buffer.is_empty() && !state.is_streaming {
                    "输入指令开始对话，或键入 /resume (恢复会话), /clear, /quit..."
                } else {
                    state.input_buffer.as_str()
                },
                if state.input_buffer.is_empty() {
                    Style::default().fg(Color::Rgb(110, 120, 140))
                } else {
                    Style::default().fg(Color::Rgb(245, 248, 255))
                },
            ),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::raw("  "),
            Span::styled(status_text_disp, Style::default().fg(status_fg).bg(status_bg).add_modifier(Modifier::BOLD)),
            Span::raw(" ".repeat(pad_spaces)),
            Span::styled(model_tag, Style::default().fg(Color::Rgb(170, 195, 230)).bg(Color::Rgb(38, 44, 58))),
            Span::raw("  "),
        ]),
    ];

    let input_widget = Paragraph::new(input_lines).style(Style::default().bg(input_container_bg));
    f.render_widget(input_widget, chunks[2]);

    // 如果处于 SelectingSession 弹窗模式，在中央渲染一个无边界柔和浮动选择面板
    if state.mode == InteractionMode::SelectingSession {
        let area = f.area();
        let popup_width = area.width.min(64).max(40);
        let popup_height = area.height.min(18).max(10);
        let popup_x = (area.width.saturating_sub(popup_width)) / 2;
        let popup_y = (area.height.saturating_sub(popup_height)) / 2;
        let popup_rect = ratatui::layout::Rect::new(popup_x, popup_y, popup_width, popup_height);

        let mut lines = Vec::new();
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
                        let _ = tx_events.send(UiEvent::InputKey(key));
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
                        KeyCode::Enter => {
                            let input = state.input_buffer.trim().to_string();
                            if !input.is_empty() {
                                state.input_buffer.clear();

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
                                        content: "Commands: /resume (select session), /clear, /compact, /cost, /quit".to_string(),
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

                                let turn_id_str = format!("turn-{}", uuid::Uuid::new_v4());
                                *current_turn_id.lock().unwrap() = Some(turn_id_str.clone());

                                let cp = control_plane.clone();
                                let sess_id = state.session_id.clone();
                                let tx_stream = tx.clone();

                                std::thread::spawn(move || {
                                    let _guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
                                    let sink = TuiTurnSink { tx: tx_stream };
                                    let cmd = StartTurnStreamCommand {
                                        turn_id: turn_id_str,
                                        input: TurnInput {
                                            message: input,
                                            display_message: None,
                                            provider_id: Some("ponyllm".to_string()),
                                            model_id: None,
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
                            state.input_buffer.push(c);
                        }
                        KeyCode::Backspace => {
                            state.input_buffer.pop();
                        }
                        KeyCode::Up => {
                            state.scroll_up(2);
                        }
                        KeyCode::Down => {
                            state.scroll_down(2, u16::MAX);
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
                }
                UiEvent::StreamToolCall(call) => {
                    state.add_or_update_tool_event(&call, false);
                }
                UiEvent::StreamToolResult(res) => {
                    state.add_or_update_tool_event(&res, true);
                }
                UiEvent::StreamCompleted => {
                    state.is_streaming = false;
                    state.status_text = "Turn completed".to_string();
                }
                UiEvent::StreamFailed(err) => {
                    state.is_streaming = false;
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
