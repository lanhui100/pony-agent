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
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame, Terminal,
};
use std::io::IsTerminal;
use std::io;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// T2c：SIGINT/SIGTERM 信号到达时不直接杀死进程，而是置位该标志；
// 主循环每个 tick（≤16ms）检测后正常 break，走统一的终端恢复保证器。
static SHUTDOWN_REQUESTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn handle_shutdown_signal(sig: libc::c_int) {
    SHUTDOWN_REQUESTED.store(true, Ordering::SeqCst);
    eprintln!("[pony-tui] shutdown signal {} received", sig);
}

/// `libc` 是 `[target.'cfg(unix)'.dependencies]` 依赖，Windows 目标不存在该 crate，
/// 因此整套信号接管必须 cfg(unix) 门控（否则 Windows 编译报 E0433）。
#[cfg(unix)]
fn install_shutdown_signal_handlers() {
    unsafe {
        // Rust 禁止把函数项直接 cast 成整数（"direct cast of function item into an integer"），
        // 必须先经 `*const ()` 中转再转成 sighandler_t。
        let handler = handle_shutdown_signal as *const () as libc::sighandler_t;
        libc::signal(libc::SIGINT, handler);
        libc::signal(libc::SIGTERM, handler);
        // pty 会话被关闭（如 `script` 收到 SIGINT 后退出）时，TUI 进程会收到 SIGHUP；
        // 同样走优雅退出路径以执行终端恢复，而不是默认终止并留下 alternate screen。
        libc::signal(libc::SIGHUP, handler);
    }
}

/// 非 unix（Windows）无 POSIX 信号语义：不接管信号，保持控制台默认行为。
#[cfg(not(unix))]
fn install_shutdown_signal_handlers() {}

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
    /// 终端尺寸变化（SSH 初次上报晚到 / 用户拉伸窗口）：必须立即整屏重绘，否则保留旧尺寸画面。
    Resized,
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
    /// 输入光标（按 char 计数的插入点），与 input_buffer 严格同步。
    pub input_cursor: usize,
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
            input_cursor: 0,
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

// 两帧低频呼吸点：任何终端字体都有这两个字形，且不抢正文注意力。
const SPINNER_FRAMES: &[&str] = &["·", "•"];

pub fn spinner_frame(tick: usize) -> &'static str {
    SPINNER_FRAMES[tick % SPINNER_FRAMES.len()]
}

impl TuiState {
    // ── 输入编辑（光标感知，按 char 索引，保证 CJK 安全）──────────────────
    pub fn input_char_len(&self) -> usize {
        self.input_buffer.chars().count()
    }

    fn input_byte_index(&self, char_idx: usize) -> usize {
        self.input_buffer
            .char_indices()
            .nth(char_idx)
            .map(|(byte, _)| byte)
            .unwrap_or(self.input_buffer.len())
    }

    /// 整体替换输入内容，并把光标移到末尾（命令补全/回填专用）。
    pub fn set_input(&mut self, text: &str) {
        self.input_buffer = text.to_string();
        self.input_cursor = self.input_buffer.chars().count();
    }

    pub fn insert_char(&mut self, c: char) {
        let cursor = self.input_cursor.min(self.input_char_len());
        let byte = self.input_byte_index(cursor);
        self.input_buffer.insert(byte, c);
        self.input_cursor = cursor + 1;
    }

    pub fn insert_newline(&mut self) {
        self.insert_char('\n');
    }

    pub fn backspace(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        let cursor = self.input_cursor.min(self.input_char_len());
        let byte = self.input_byte_index(cursor - 1);
        self.input_buffer.remove(byte);
        self.input_cursor = cursor - 1;
    }

    pub fn delete_forward(&mut self) {
        let cursor = self.input_cursor.min(self.input_char_len());
        if cursor >= self.input_char_len() {
            return;
        }
        let byte = self.input_byte_index(cursor);
        self.input_buffer.remove(byte);
    }

    pub fn move_cursor_left(&mut self) {
        if self.input_cursor > 0 {
            self.input_cursor -= 1;
        }
    }

    pub fn move_cursor_right(&mut self) {
        if self.input_cursor < self.input_char_len() {
            self.input_cursor += 1;
        }
    }

    pub fn cursor_line_home(&mut self) {
        let chars: Vec<char> = self.input_buffer.chars().collect();
        let mut idx = self.input_cursor.min(chars.len());
        while idx > 0 && chars[idx - 1] != '\n' {
            idx -= 1;
        }
        self.input_cursor = idx;
    }

    pub fn cursor_line_end(&mut self) {
        let chars: Vec<char> = self.input_buffer.chars().collect();
        let mut idx = self.input_cursor.min(chars.len());
        while idx < chars.len() && chars[idx] != '\n' {
            idx += 1;
        }
        self.input_cursor = idx;
    }

    /// Ctrl+W：向左删除一个词。
    pub fn delete_word_backward(&mut self) {
        let chars: Vec<char> = self.input_buffer.chars().collect();
        let mut idx = self.input_cursor.min(chars.len());
        while idx > 0 && chars[idx - 1].is_whitespace() {
            self.backspace();
            idx -= 1;
        }
        while idx > 0 && !chars[idx - 1].is_whitespace() {
            self.backspace();
            idx -= 1;
        }
    }

    pub fn clear_input(&mut self) {
        self.input_buffer.clear();
        self.input_cursor = 0;
    }

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
        // 标题不用符号噪音，只用字重与微小的色阶差异表达层级。
        let clean_text = trimmed[hash_count..].trim_start();
        let style = match hash_count {
            1 => Style::default().fg(theme::INK).add_modifier(Modifier::BOLD),
            2 => Style::default().fg(theme::INK).add_modifier(Modifier::BOLD),
            3 => Style::default().fg(theme::INK_SOFT).add_modifier(Modifier::BOLD),
            _ => Style::default().fg(theme::INK_SOFT),
        };
        return vec![Span::styled(clean_text.to_string(), style)];
    }

    if let Some(rest) = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")) {
        let mut spans = vec![Span::styled("  · ", faint())];
        spans.extend(parse_inline_formatting(rest));
        return spans;
    }

    // 数字有序列表例如 "1. ", "2. "
    if let Some(dot_idx) = trimmed.find(". ") {
        let num_part = &trimmed[..dot_idx];
        if !num_part.is_empty() && num_part.chars().all(|c| c.is_ascii_digit()) {
            let mut spans = vec![Span::styled(format!("  {}. ", num_part), faint())];
            spans.extend(parse_inline_formatting(&trimmed[dot_idx + 2..]));
            return spans;
        }
    }

    // 分割线 --- 或 ***：一条安静的发丝线
    if trimmed == "---" || trimmed == "***" || trimmed == "___" {
        return vec![Span::styled(
            "─".repeat(48),
            Style::default().fg(theme::HAIRLINE),
        )];
    }

    // 代码块围栏 ```lang 只显示语言标签；裸围栏保持安静（不占视觉噪音）
    if trimmed.starts_with("```") {
        let lang = trimmed.trim_start_matches('`').trim();
        if lang.is_empty() {
            return vec![Span::raw("")];
        }
        return vec![Span::styled(lang.to_string(), faint())];
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
                    spans.push(Span::styled(rest[..bold_start].to_string(), ink_soft()));
                }
                spans.push(Span::styled(
                    rest[bold_start + 2..actual_end].to_string(),
                    Style::default().fg(theme::INK).add_modifier(Modifier::BOLD),
                ));
                rest = &rest[actual_end + 2..];
                continue;
            }
        }

        if let Some(code_start) = rest.find('`') {
            if let Some(code_end) = rest[code_start + 1..].find('`') {
                let actual_end = code_start + 1 + code_end;
                if code_start > 0 {
                    spans.push(Span::styled(rest[..code_start].to_string(), ink_soft()));
                }
                spans.push(Span::styled(
                    rest[code_start + 1..actual_end].to_string(),
                    Style::default()
                        .fg(theme::INK)
                        .bg(theme::SURFACE),
                ));
                rest = &rest[actual_end + 1..];
                continue;
            }
        }

        spans.push(Span::styled(rest.to_string(), ink_soft()));
        break;
    }
    spans
}

/// 彩色 init 头部（参考 Antigravity CLI banner）：
/// 左侧是品牌渐变色的几何「P」标识 —— 22×28 像素网格，用「▀」半块字符逐格上/下双色渲染
/// （实心块、无点阵断裂），右侧是版本 / 模型(提供商) / 工作区信息列，垂直居中。
/// 整块是会话流的第一条内容，随对话向上滚动。
fn pony_banner_lines(state: &TuiState) -> Vec<Line<'static>> {
    // 「P」字形像素表：每行 22 格，'.' 为空，其余为调色板索引（0-9a）。
    const P_ROWS: [&str; 28] = [
        "......................",
        "...000112234456677....",
        ".0000112234455667788..",
        ".0000112334556677888..",
        ".00011223445667778899.",
        ".00112233455667788899.",
        ".0011233......7888999.",
        ".01122..........88999.",
        ".11123..........8999a.",
        ".11223..........9999a.",
        ".12234..........999aa.",
        ".12334..........99aaa.",
        ".223445........999aaa.",
        ".2344556677888999aaaa.",
        ".334556677888999aaaaa.",
        ".344566777889999aaaaa.",
        ".34556677888999aaaaa..",
        ".4556677888999aaaaa...",
        ".45667................",
        ".55667................",
        ".56677................",
        ".56677................",
        ".66778................",
        ".67778................",
        ".67788................",
        ".77888................",
        "...8..................",
        "......................",
    ];
    const P_PALETTE: [(u8, u8, u8); 12] = [
        (240, 112, 62),
        (237, 97, 63),
        (235, 88, 63),
        (233, 80, 64),
        (230, 73, 67),
        (223, 70, 76),
        (215, 66, 86),
        (205, 62, 97),
        (194, 57, 112),
        (181, 52, 128),
        (160, 49, 136),
        (0, 0, 0),
    ];
    const COLS: usize = 22;
    const ROWS: usize = 28;

    let color_of = |ch: u8| -> Option<ratatui::style::Color> {
        if ch == b'.' {
            return None;
        }
        let idx = match ch {
            b'0'..=b'9' => (ch - b'0') as usize,
            b'a' => 10,
            b'b' => 11,
            _ => return None,
        };
        let (r, g, b) = P_PALETTE[idx.min(P_PALETTE.len() - 1)];
        Some(ratatui::style::Color::Rgb(r, g, b))
    };

    let info = [
        Line::from(vec![
            Span::styled(
                "pony",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" v{}", env!("CARGO_PKG_VERSION")),
                Style::default().fg(theme::INK),
            ),
        ]),
        Line::from(Span::styled(
            format!("{} ({})", state.model, state.current_provider_id),
            Style::default().fg(theme::MUTED),
        )),
        Line::from(Span::styled(
            state.workspace.display().to_string(),
            Style::default().fg(theme::MUTED),
        )),
    ];

    let mut out: Vec<Line<'static>> = Vec::new();
    for line in 0..(ROWS / 2) {
        let top = P_ROWS[line * 2].as_bytes();
        let bottom = P_ROWS[line * 2 + 1].as_bytes();
        let mut spans: Vec<Span<'static>> = vec![Span::raw("  ")];
        for x in 0..COLS {
            let up = top.get(x).and_then(|c| color_of(*c));
            let dn = bottom.get(x).and_then(|c| color_of(*c));
            let span = match (up, dn) {
                (Some(u), Some(d)) => Span::styled("▀", Style::default().fg(u).bg(d)),
                (Some(u), None) => Span::styled("▀", Style::default().fg(u)),
                (None, Some(d)) => Span::styled("▄", Style::default().fg(d)),
                (None, None) => Span::raw(" "),
            };
            spans.push(span);
        }
        // 信息列垂直居中于 14 行标识（占第 6~8 行）。
        if let Some(info_line) = line.checked_sub(5).and_then(|i| info.get(i)) {
            spans.push(Span::raw("   "));
            spans.extend(info_line.spans.clone());
        }
        out.push(Line::from(spans));
    }
    out
}

pub fn ascii_logo() -> Vec<Line<'static>> {
    // 空态由 ui() 组合：pony_banner_lines(state) + 引导块；
    // 此函数保留独立可测的引导块。
    let mut lines = vec![
        Line::from(Span::styled("描述一个任务开始，或输入命令：", ink_soft())),
        blank(),
    ];
    // 直接复用命令表，保证空态提示与斜杠菜单永不漂移。
    for (cmd, desc) in AVAILABLE_COMMANDS.iter().take(4) {
        lines.push(Line::from(vec![
            Span::styled(pad_display(cmd, 11), ink_soft()),
            Span::styled(desc.to_string(), muted()),
        ]));
    }
    lines
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

// ─────────────────────────────────────────────────────────────────────────────
// Design tokens —— 极简克制的高级感：
//   · 单色为骨：一块近乎无色的暖黑画布 + 暖白文字，拒绝高饱和撞色；
//   · 单一强调色（骨色/bone）只用于「光标、选中、进行中」三处语义；
//   · 层级靠留白与字重，而非描边、胶囊、色块；
//   · 分隔只用 1px 发丝线（hairline），永不画整块边框卡片。
// ─────────────────────────────────────────────────────────────────────────────
mod theme {
    use ratatui::style::Color;

    pub const CANVAS: Color = Color::Rgb(13, 14, 17);
    pub const SURFACE: Color = Color::Rgb(24, 26, 31);
    pub const SURFACE_SEL: Color = Color::Rgb(38, 41, 48);
    pub const INK: Color = Color::Rgb(232, 229, 224);
    pub const INK_SOFT: Color = Color::Rgb(203, 200, 195);
    pub const MUTED: Color = Color::Rgb(154, 157, 163);
    pub const FAINT: Color = Color::Rgb(112, 116, 124);
    pub const HAIRLINE: Color = Color::Rgb(48, 51, 58);
    pub const ACCENT: Color = Color::Rgb(198, 167, 126);
    pub const SUCCESS: Color = Color::Rgb(140, 172, 137);
    pub const DANGER: Color = Color::Rgb(199, 130, 125);
    pub const WARN: Color = Color::Rgb(199, 176, 126);
}

/// 左右统一沟槽宽度（视觉留白，禁止内容贴边）。
const GUTTER: u16 = 2;

/// 极简字形集：只用三种标记，避免图标噪音。
const GLYPH_RUNNING: &str = "◦";
const GLYPH_DONE: &str = "✓";
const GLYPH_FAIL: &str = "✕";
const CARET: &str = "▏";

fn ink() -> Style {
    Style::default().fg(theme::INK)
}

fn ink_soft() -> Style {
    Style::default().fg(theme::INK_SOFT)
}

fn muted() -> Style {
    Style::default().fg(theme::MUTED)
}

fn faint() -> Style {
    Style::default().fg(theme::FAINT)
}

/// 东亚宽字符按 2 列计宽，保证中文/emoji 折行与滚动偏移精确。
fn char_width(c: char) -> usize {
    let cp = c as u32;
    let wide = matches!(cp,
        0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA000..=0xA4CF
            | 0xA960..=0xA97F
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE10..=0xFE19
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x3FFFD);
    if wide {
        2
    } else {
        1
    }
}

fn str_width(text: &str) -> usize {
    text.chars().map(char_width).sum()
}

/// 行首禁则：这些标点不允许出现在行首（悬挂到上一行末尾）。
fn is_no_line_start(c: char) -> bool {
    matches!(
        c,
        '，' | '。'
            | '、'
            | '；'
            | '：'
            | '？'
            | '！'
            | '）'
            | '】'
            | '》'
            | '」'
            | '』'
            | '…'
            | ','
            | '.'
            | ';'
            | ':'
            | '?'
            | '!'
            | ')'
            | ']'
            | '%'
    )
}

/// 1px 发丝分隔线（不是边框，只是一条极淡的横线）。
fn hairline(width: usize) -> Line<'static> {
    Line::from(Span::styled(
        "─".repeat(width.max(1)),
        Style::default().fg(theme::HAIRLINE),
    ))
}

fn blank() -> Line<'static> {
    Line::raw("")
}

/// 按显示宽度折行，保留 span 样式；用于所有正文（CJK 不会错位）。
fn wrap_line(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    if line.spans.is_empty() {
        return vec![blank()];
    }
    let mut out: Vec<Line<'static>> = Vec::new();
    let mut current: Vec<Span<'static>> = Vec::new();
    let mut col = 0usize;
    for span in &line.spans {
        if span.content.is_empty() {
            continue;
        }
        let style = span.style;
        let mut buf = String::new();
        for ch in span.content.chars() {
            let w = char_width(ch);
            if col + w > width && col > 0 && !is_no_line_start(ch) {
                if !buf.is_empty() {
                    current.push(Span::styled(std::mem::take(&mut buf), style));
                }
                out.push(Line::from(std::mem::take(&mut current)));
                col = 0;
            }
            buf.push(ch);
            col += w;
        }
        if !buf.is_empty() {
            current.push(Span::styled(buf, style));
        }
    }
    if !current.is_empty() || out.is_empty() {
        out.push(Line::from(current));
    }
    out
}

/// 保证末尾恰好有 n 个空行（用于区分「段间距 1 行」与「轮间距 2 行」的节拍）。
fn push_gap(lines: &mut Vec<Line<'static>>, n: usize) {
    if lines.is_empty() {
        return;
    }
    let mut blanks = 0usize;
    for line in lines.iter().rev() {
        if line.spans.iter().all(|s| s.content.trim().is_empty()) {
            blanks += 1;
        } else {
            break;
        }
    }
    for _ in blanks..n {
        lines.push(blank());
    }
}

fn push_wrapped(lines: &mut Vec<Line<'static>>, line: Line<'static>, width: usize) {
    let mut wrapped = wrap_line(&line, width);
    // 行首禁则（跨行）：模型自带换行后若行首是标点，悬挂回上一行行尾。
    if !lines.is_empty() {
        let mut pulled: Option<(char, Style)> = None;
        if let Some(first) = wrapped.first_mut() {
            if let Some(span) = first.spans.first_mut() {
                if let Some(ch) = span.content.chars().next() {
                    if is_no_line_start(ch) {
                        span.content = span.content.chars().skip(1).collect();
                        let style = span.style;
                        pulled = Some((ch, style));
                    }
                }
            }
        }
        if let Some((ch, style)) = pulled {
            if let Some(prev) = lines.last_mut() {
                if let Some(last) = prev.spans.last_mut() {
                    let merged = format!("{}{}", last.content, ch);
                    last.content = merged.into();
                } else {
                    prev.spans.push(Span::styled(ch.to_string(), style));
                }
            }
        }
    }
    lines.extend(wrapped);
}

/// 只在不重复的前提下插入空行，保持「行间距呼吸」而不产生连续空行噪音。
fn push_blank(lines: &mut Vec<Line<'static>>) {
    if let Some(last) = lines.last() {
        let is_blank = last.spans.iter().all(|s| s.content.trim().is_empty());
        if !is_blank {
            lines.push(blank());
        }
    }
}

/// 输入框视觉折行：按显示宽度硬折行 + 尊重显式换行符，返回字符区间。
fn layout_input(text: &str, width: usize) -> Vec<(usize, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let width = width.max(1);
    let mut ranges: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    let mut col = 0usize;
    for (i, ch) in chars.iter().enumerate() {
        if *ch == '\n' {
            ranges.push((start, i));
            start = i + 1;
            col = 0;
            continue;
        }
        let w = char_width(*ch);
        if col + w > width && col > 0 {
            ranges.push((start, i));
            start = i;
            col = 0;
        }
        col += w;
    }
    ranges.push((start, chars.len()));
    ranges
}

/// 浮动面板：发丝描边 + 表面底色 + 标题行（hint 右对齐），返回标题下方的内容区。
fn draw_panel(f: &mut Frame, area: Rect, title: &str, hint: &str) -> Rect {
    f.render_widget(ratatui::widgets::Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Plain)
        .border_style(Style::default().fg(theme::HAIRLINE))
        .style(Style::default().bg(theme::SURFACE));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let title_w = str_width(title) + 1;
    let hint_w = str_width(hint);
    let gap = (inner.width as usize)
        .saturating_sub(title_w + hint_w + 2)
        .max(1);
    let head = Line::from(vec![
        Span::styled(
            format!(" {}", title),
            Style::default().fg(theme::INK).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" ".repeat(gap)),
        Span::styled(hint.to_string(), faint()),
        Span::raw(" "),
    ]);
    let head_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: inner.height.min(2),
    };
    f.render_widget(
        Paragraph::new(vec![head, blank()]).style(Style::default().bg(theme::SURFACE)),
        head_area,
    );

    Rect {
        x: inner.x,
        y: inner.y.saturating_add(2),
        width: inner.width,
        height: inner.height.saturating_sub(2),
    }
}

fn truncate_width(text: &str, width: usize) -> String {
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        let w = char_width(ch);
        if used + w > width {
            break;
        }
        out.push(ch);
        used += w;
    }
    out
}

fn pad_display(text: &str, width: usize) -> String {
    let used = str_width(text);
    if used >= width {
        text.to_string()
    } else {
        format!("{}{}", text, " ".repeat(width - used))
    }
}

/// 列表行：左侧 emphasis 竖条 + 主文本 + 右侧次要信息；选中行用极淡底色铺满整行。
fn list_row(selected: bool, left: &str, right: &str, width: usize) -> Line<'static> {
    let bg = if selected {
        theme::SURFACE_SEL
    } else {
        theme::SURFACE
    };
    let bar = if selected { "▏" } else { " " };
    let left_style = if selected {
        Style::default()
            .fg(theme::INK)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::INK_SOFT).bg(bg)
    };
    // 右侧状态/次要信息保持可读（FAINT 只留给纯装饰性 chrome）
    let right_style = Style::default().fg(theme::MUTED).bg(bg);

    let right_w = str_width(right);
    let left_w = width.saturating_sub(3 + right_w);
    let left_txt = pad_display(&truncate_width(left, left_w), left_w);
    let used = 2 + left_w + right_w;
    Line::from(vec![
        Span::styled(
            bar,
            if selected {
                Style::default().fg(theme::ACCENT).bg(bg)
            } else {
                Style::default().bg(bg)
            },
        ),
        Span::styled(format!(" {}", left_txt), left_style),
        Span::styled(right.to_string(), right_style),
        Span::styled(" ".repeat(width.saturating_sub(used)), Style::default().bg(bg)),
    ])
}

fn fill(f: &mut Frame, area: Rect, color: ratatui::style::Color) {
    f.render_widget(Block::default().style(Style::default().bg(color)), area);
}

/// 渲染带光标的多行输入区（⌘ 提示符 + 软折行 + 细竖线光标）。
fn render_input(f: &mut Frame, area: Rect, state: &TuiState) {
    let chars: Vec<char> = state.input_buffer.chars().collect();
    let cursor = state.input_cursor.min(chars.len());
    let text_width = area.width.saturating_sub(2).max(1) as usize;
    let ranges = layout_input(&state.input_buffer, text_width);
    let active_line = ranges
        .iter()
        .position(|(s, e)| cursor >= *s && cursor < *e)
        .unwrap_or(ranges.len().saturating_sub(1));

    let mut lines: Vec<Line<'static>> = Vec::new();
    for (idx, (s, e)) in ranges.iter().enumerate() {
        let mut spans: Vec<Span<'static>> = vec![if idx == 0 {
            Span::styled(
                "› ",
                Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw("  ")
        }];

        let text: String = chars[*s..*e].iter().collect();
        if idx == active_line {
            let rel = cursor.saturating_sub(*s).min(*e - *s);
            let before: String = chars[*s..*s + rel].iter().collect();
            let after: String = chars[*s + rel..*e].iter().collect();
            if state.input_buffer.is_empty() {
                spans.push(Span::styled(CARET, Style::default().fg(theme::ACCENT)));
                let placeholder = truncate_width(
                    " 输入任务，或输入 / 查看命令",
                    text_width.saturating_sub(1),
                );
                spans.push(Span::styled(
                    placeholder,
                    Style::default().fg(theme::FAINT),
                ));
            } else {
                if !before.is_empty() {
                    spans.push(Span::styled(before, ink()));
                }
                spans.push(Span::styled(CARET, Style::default().fg(theme::ACCENT)));
                if !after.is_empty() {
                    spans.push(Span::styled(after, ink()));
                }
            }
        } else if !text.is_empty() {
            spans.push(Span::styled(text, ink()));
        }

        lines.push(Line::from(spans));
    }

    let widget = Paragraph::new(lines).style(Style::default().bg(theme::SURFACE));
    f.render_widget(widget, area);
}

/// 底部状态词：克制到只有一个词（耗时与动画只在正文活动行出现一次）。
fn status_word(state: &TuiState) -> (String, Style) {
    match &state.runtime_mode {
        RuntimeMode::Ready => ("就绪".to_string(), faint()),
        RuntimeMode::Connecting => ("连接中".to_string(), muted()),
        RuntimeMode::Thinking => ("思考中".to_string(), muted()),
        RuntimeMode::Responding => ("写入中".to_string(), muted()),
        RuntimeMode::ExecutingTool(name) => {
            let short: String = name.chars().take(28).collect();
            (short, muted())
        }
        RuntimeMode::WaitingApproval => ("待确认".to_string(), Style::default().fg(theme::WARN)),
        RuntimeMode::Completed => ("完成".to_string(), Style::default().fg(theme::SUCCESS)),
        RuntimeMode::Failed(_) => ("失败".to_string(), Style::default().fg(theme::DANGER)),
    }
}

fn elapsed_label(state: &TuiState) -> String {
    state
        .turn_started_at
        .map(|t| format_elapsed_seconds(t.elapsed().as_millis() as u64))
        .unwrap_or_default()
}

pub fn ui(f: &mut Frame, state: &mut TuiState) {
    let area = f.area();
    fill(f, area, theme::CANVAS);

    // 极小终端降级：不 panic，只给一行提示。
    if area.width < 24 || area.height < 8 {
        let msg = Line::from(Span::styled("终端窗口过小", muted()));
        f.render_widget(
            Paragraph::new(msg).style(Style::default().bg(theme::CANVAS)),
            area,
        );
        return;
    }

    let content = Rect {
        x: area.x + GUTTER,
        y: area.y,
        width: area.width.saturating_sub(GUTTER * 2),
        height: area.height,
    };

    // ── Header：一行、无底色带、无徽章 ─────────────────────────────────────
    let ws_name = state
        .workspace
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("~");
    let session_short: String = state
        .session_id
        .rsplit('-')
        .next()
        .unwrap_or(&state.session_id)
        .chars()
        .take(6)
        .collect();
    let right_meta = format!("{}  ·  {}", state.model, session_short);
    let left_w = 4 + 5 + str_width(ws_name);
    let right_w = str_width(&right_meta);
    let pad = content.width as usize;
    let header_line = if pad > left_w + right_w {
        Line::from(vec![
            Span::styled("pony", Style::default().fg(theme::INK).add_modifier(Modifier::BOLD)),
            Span::styled("  ·  ", faint()),
            Span::styled(ws_name.to_string(), muted()),
            Span::raw(" ".repeat(pad - left_w - right_w)),
            Span::styled(right_meta, faint()),
        ])
    } else {
        Line::from(vec![
            Span::styled("pony", Style::default().fg(theme::INK).add_modifier(Modifier::BOLD)),
            Span::styled("  ·  ", faint()),
            Span::styled(ws_name.to_string(), muted()),
        ])
    };
    let header_rect = Rect { height: 1, ..content };
    f.render_widget(Paragraph::new(header_line), header_rect);

    // ── 发丝线 + 呼吸留白 ─────────────────────────────────────────────────
    let rule_rect = Rect { y: content.y + 1, height: 1, ..content };
    f.render_widget(Paragraph::new(hairline(content.width as usize)), rule_rect);

    // ── 底部区块尺寸（输入区高随内容自适应，1~6 行）───────────────────────
    let text_width = content.width.saturating_sub(2).max(1) as usize;
    let visual_lines = layout_input(&state.input_buffer, text_width).len() as u16;
    // 输入区至少 3 行高：既是视觉「舞台」，也为多行输入预留空间
    let input_h = visual_lines.clamp(3, 6);
    let bottom_h = input_h + 1; // 输入行 + 状态行

    let chat_top = content.y + 3;
    let chat_h = area
        .y
        .saturating_add(area.height)
        .saturating_sub(bottom_h)
        .saturating_sub(chat_top)
        .saturating_sub(1); // 输入区上方留白
    let chat_rect = Rect {
        x: content.x,
        y: chat_top,
        width: content.width,
        height: chat_h.max(1),
    };

    // ── 会话正文：角色标签 + 单色正文 + 极简工具行 ───────────────────────
    let width = chat_rect.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();

    if state.messages.is_empty() {
        let mut splash: Vec<Line<'static>> = Vec::new();
        splash.push(blank());
        splash.extend(pony_banner_lines(state));
        splash.push(blank());
        splash.push(Line::from(Span::styled("终端里的自主智能体。", Style::default().fg(theme::MUTED))));
        splash.push(blank());
        splash.extend(ascii_logo());
        // 空态留白构图：按正文区 1/3 高度锚定，避免顶部挤、底部空。
        let top_pad = (chat_h as usize).saturating_sub(splash.len()) / 3;
        for _ in 0..top_pad {
            lines.push(blank());
        }
        for line in splash {
            push_wrapped(&mut lines, line, width);
        }
    } else {
        let mut prev_role: Option<MessageRole> = None;
        let last_index = state.messages.len().saturating_sub(1);
        for (idx, msg) in state.messages.iter().enumerate() {
            let prev_is_tool = matches!(
                prev_role,
                Some(MessageRole::ToolCall) | Some(MessageRole::ToolResult)
            );
            match msg.role {
                MessageRole::User => {
                    // 轮间距 2 行：长会话里一眼能扫到「新一轮提问」的节拍
                    push_gap(&mut lines, 2);
                    lines.push(Line::from(Span::styled("你", muted())));
                    for raw in msg.content.lines() {
                        push_wrapped(
                            &mut lines,
                            Line::from(Span::styled(raw.to_string(), ink())),
                            width,
                        );
                    }
                    prev_role = Some(MessageRole::User);
                }
                MessageRole::Assistant => {
                    // 一个 assistant turn 只在「首次出现」时打一次标签，工具行与正文共享同一 turn。
                    if !prev_is_tool {
                        push_gap(&mut lines, 1);
                        lines.push(Line::from(Span::styled(
                            "pony",
                            Style::default().fg(theme::ACCENT),
                        )));
                    } else {
                        // 工具行结束后回到正文：留一行呼吸，避免正文贴着工具行
                        push_blank(&mut lines);
                    }
                    let mut in_code = false;
                    for raw in msg.content.lines() {
                        let trimmed = raw.trim_start();
                        if trimmed.starts_with("```") {
                            in_code = !in_code;
                            if in_code {
                                let lang = trimmed.trim_start_matches('`').trim();
                                if !lang.is_empty() {
                                    lines.push(Line::from(vec![
                                        Span::styled("  ▏ ", Style::default().fg(theme::HAIRLINE)),
                                        Span::styled(lang.to_string(), faint()),
                                    ]));
                                }
                            }
                            continue;
                        }
                        if in_code {
                            push_wrapped(
                                &mut lines,
                                Line::from(vec![
                                    Span::styled("  ▏ ", Style::default().fg(theme::HAIRLINE)),
                                    Span::styled(
                                        raw.to_string(),
                                        Style::default().fg(theme::INK_SOFT),
                                    ),
                                ]),
                                width,
                            );
                            continue;
                        }
                        if trimmed.is_empty() {
                            push_blank(&mut lines);
                            continue;
                        }
                        push_wrapped(&mut lines, Line::from(render_markdown_line(raw)), width);
                    }
                    prev_role = Some(MessageRole::Assistant);
                }
                MessageRole::ToolCall | MessageRole::ToolResult => {
                    // 工具行本身属于 assistant turn：若无正文先行，则此处开启 turn 并打标签。
                    if !prev_is_tool && !matches!(prev_role, Some(MessageRole::Assistant)) {
                        push_gap(&mut lines, 2);
                        lines.push(Line::from(Span::styled(
                            "pony",
                            Style::default().fg(theme::ACCENT),
                        )));
                    }
                    let running = msg.role == MessageRole::ToolCall && idx == last_index;
                    let (glyph, glyph_style) = if msg.role == MessageRole::ToolCall {
                        if running {
                            (
                                spinner_frame(state.spinner_tick).to_string(),
                                Style::default().fg(theme::ACCENT),
                            )
                        } else {
                            (GLYPH_RUNNING.to_string(), faint())
                        }
                    } else if idx == last_index {
                        (GLYPH_DONE.to_string(), Style::default().fg(theme::SUCCESS))
                    } else {
                        (GLYPH_DONE.to_string(), faint())
                    };
                    // 工具行内部也分层：动作名提亮，参数/结果压暗。
                    let (head, detail) = match msg.content.split_once("  ") {
                        Some((h, d)) => (h.to_string(), d.to_string()),
                        None => (msg.content.clone(), String::new()),
                    };
                    let head_style = if msg.role == MessageRole::ToolCall {
                        ink_soft()
                    } else {
                        Style::default().fg(theme::MUTED)
                    };
                    let mut spans = vec![
                        Span::styled(format!("  {} ", glyph), glyph_style),
                        Span::styled(head, head_style),
                    ];
                    if !detail.is_empty() {
                        spans.push(Span::styled(format!("  {}", detail), muted()));
                    }
                    push_wrapped(&mut lines, Line::from(spans), width);
                    prev_role = Some(msg.role.clone());
                }
                MessageRole::System => {
                    let is_error = msg.content.contains("Error")
                        || msg.content.contains("fail")
                        || msg.content.contains("失败");
                    let (glyph, glyph_style) = if is_error {
                        (GLYPH_FAIL, Style::default().fg(theme::DANGER))
                    } else {
                        ("·", faint())
                    };
                    let text_style = if is_error {
                        Style::default().fg(theme::DANGER)
                    } else {
                        muted()
                    };
                    if !lines.is_empty() && !prev_is_tool {
                        push_blank(&mut lines);
                    }
                    push_wrapped(
                        &mut lines,
                        Line::from(vec![
                            Span::styled(format!("{} ", glyph), glyph_style),
                            Span::styled(msg.content.clone(), text_style),
                        ]),
                        width,
                    );
                    prev_role = Some(MessageRole::System);
                }
            }
        }

        // 进行中的一行极简状态（不打断阅读节奏，只占一行）
        if state.is_streaming && !matches!(state.runtime_mode, RuntimeMode::ExecutingTool(_)) {
            push_blank(&mut lines);
            let (word, style) = status_word(state);
            let elapsed = elapsed_label(state);
            let text = if elapsed.is_empty() {
                format!("{} {}", spinner_frame(state.spinner_tick), word)
            } else {
                format!("{} {}  {}", spinner_frame(state.spinner_tick), word, elapsed)
            };
            lines.push(Line::from(Span::styled(text, style)));
            lines.push(blank());
        }
    }

    // 模态时压暗背板正文，让浮层成为唯一焦点
    if state.mode != InteractionMode::Normal {
        for line in &mut lines {
            for span in &mut line.spans {
                span.style = span.style.fg(theme::FAINT);
            }
        }
    }

    // 精确滚动：行数即渲染行数（折行已按显示宽度预先完成）
    let visible = chat_rect.height as usize;
    let total = lines.len();
    let max_offset = total.saturating_sub(visible);
    let offset = if state.auto_scroll {
        state.scroll_offset = max_offset as u16;
        max_offset
    } else {
        let clamped = (state.scroll_offset as usize).min(max_offset);
        state.scroll_offset = clamped as u16;
        clamped
    };
    f.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(theme::CANVAS))
            .scroll((offset as u16, 0)),
        chat_rect,
    );

    // ── Composer：整宽表面色带，靠色块本身与正文分区（不用发丝线）──────────
    let bottom_top = area.y.saturating_add(area.height).saturating_sub(bottom_h);
    let band = Rect {
        x: area.x,
        y: bottom_top,
        width: area.width,
        height: bottom_h,
    };
    fill(f, band, theme::SURFACE);

    let input_rect = Rect {
        y: bottom_top,
        height: input_h,
        ..content
    };
    render_input(f, input_rect, state);

    // 状态行：色带最底行只放右侧状态词（快捷键提示已移除，键位靠 --help 与空态引导）
    let (phrase, phrase_style) = status_word(state);
    let footer_y = input_rect.y + input_h;
    let footer_rect = Rect { y: footer_y, height: 1, ..content };
    if !phrase.is_empty() {
        let phrase_w = str_width(&phrase);
        let space = content.width as usize;
        let footer_line = if space > phrase_w + 2 {
            Line::from(vec![
                Span::raw(" ".repeat(space - phrase_w)),
                Span::styled(phrase, phrase_style),
            ])
        } else {
            Line::from(Span::styled(
                truncate_width(&phrase, space.saturating_sub(1)),
                phrase_style,
            ))
        };
        f.render_widget(Paragraph::new(footer_line), footer_rect);
    }

    // ── 斜杠命令面板：浮于输入区之上，两条发丝线夹一段浅表面 ──────────────
    let match_cmds = state.matching_commands();
    if !match_cmds.is_empty() && state.mode == InteractionMode::Normal {
        let rows = match_cmds.len().min(7) as u16;
        let menu_h = rows + 2;
        let desc_w = match_cmds
            .iter()
            .take(rows as usize)
            .map(|(_, d)| str_width(d))
            .max()
            .unwrap_or(0);
        // 几何：竖条 1 + 空格 1 + 命令列 12 + 描述 + 右侧留白 2
        let need_w = (16 + desc_w + 2) as u16;
        let menu_w = need_w.clamp(28, content.width.min(64));
        let menu_y = band.y.saturating_sub(menu_h + 1); // 与输入区留一行呼吸
        let menu_rect = Rect {
            x: content.x,
            y: menu_y,
            width: menu_w,
            height: menu_h,
        };
        if menu_rect.y > chat_rect.y {
            f.render_widget(ratatui::widgets::Clear, menu_rect);
            let block = Block::default()
                .borders(Borders::TOP | Borders::BOTTOM)
                .border_type(BorderType::Plain)
                .border_style(Style::default().fg(theme::HAIRLINE))
                .style(Style::default().bg(theme::SURFACE));
            let inner = block.inner(menu_rect);
            f.render_widget(block, menu_rect);

            let selected = state
                .selected_command_index
                .min(match_cmds.len().saturating_sub(1));
            let inner_w = inner.width as usize;
            let mut menu_lines: Vec<Line<'static>> = Vec::new();
            for (idx, (cmd, desc)) in match_cmds.iter().enumerate().take(rows as usize) {
                let is_sel = idx == selected;
                let bg = if is_sel { theme::SURFACE_SEL } else { theme::SURFACE };
                let cmd_col = pad_display(cmd, 12);
                let used = 2 + str_width(&cmd_col) + str_width(desc);
                let mut spans = vec![
                    Span::styled(
                        if is_sel { "▏" } else { " " },
                        if is_sel {
                            Style::default().fg(theme::ACCENT).bg(bg)
                        } else {
                            Style::default().bg(bg)
                        },
                    ),
                    Span::styled(
                        format!(" {}", cmd_col),
                        if is_sel {
                            Style::default()
                                .fg(theme::INK)
                                .bg(bg)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(theme::INK_SOFT).bg(bg)
                        },
                    ),
                    Span::styled(
                        desc.to_string(),
                        if is_sel {
                            Style::default().fg(theme::INK_SOFT).bg(bg)
                        } else {
                            Style::default().fg(theme::MUTED).bg(bg)
                        },
                    ),
                    Span::styled(
                        " ".repeat(inner_w.saturating_sub(used)),
                        Style::default().bg(bg),
                    ),
                ];
                spans.retain(|s| !s.content.is_empty());
                menu_lines.push(Line::from(spans));
            }
            f.render_widget(
                Paragraph::new(menu_lines).style(Style::default().bg(theme::SURFACE)),
                inner,
            );
        }
    }

    // ── 模态面板（会话 / 提供商 / 模型 / 新增提供商）──────────────────────
    let panel_rect = |w: u16, h: u16| -> Rect {
        let pw = w.min(area.width.saturating_sub(4)).max(20);
        let ph = h.min(area.height.saturating_sub(2)).max(6);
        Rect {
            x: area.x + (area.width.saturating_sub(pw)) / 2,
            y: area.y + (area.height.saturating_sub(ph)) / 2,
            width: pw,
            height: ph,
        }
    };

    if state.mode == InteractionMode::SelectingSession {
        let rows = state.session_list.len().max(1) as u16;
        let rect = panel_rect(58, rows + 4);
        let body = draw_panel(f, rect, "会话", "↑↓ 选择 · 回车 确认 · esc 取消");
        let inner_w = body.width as usize;
        let mut lines_out: Vec<Line<'static>> = Vec::new();
        if state.session_list.is_empty() {
            lines_out.push(Line::from(Span::styled("  暂无历史会话", faint())));
        } else {
            for (idx, item) in state.session_list.iter().enumerate() {
                let selected = idx == state.selected_session_index;
                let id_short: String = item
                    .id
                    .rsplit('-')
                    .next()
                    .unwrap_or(&item.id)
                    .chars()
                    .take(8)
                    .collect();
                let title = truncate_width(&item.title, inner_w.saturating_sub(14));
                lines_out.push(list_row(selected, &title, &id_short, inner_w));
            }
        }
        f.render_widget(
            Paragraph::new(lines_out).style(Style::default().bg(theme::SURFACE)),
            body,
        );
    }

    if state.mode == InteractionMode::SelectingProvider {
        let rows = state.provider_list.len().max(1) as u16;
        let rect = panel_rect(60, rows + 4);
        let body = draw_panel(f, rect, "提供商", "↑↓ 选择 · 回车 确认 · a 新增 · esc 取消");
        let inner_w = body.width as usize;
        let mut lines_out: Vec<Line<'static>> = Vec::new();
        if state.provider_list.is_empty() {
            lines_out.push(Line::from(Span::styled(
                "  尚未配置，按 a 新增",
                faint(),
            )));
        } else {
            for (idx, item) in state.provider_list.iter().enumerate() {
                let selected = idx == state.selected_provider_index;
                let active = item.id == state.current_provider_id;
                let right = if active {
                    format!("{}  当前", item.protocol)
                } else {
                    item.protocol.clone()
                };
                let name = truncate_width(&item.name, inner_w.saturating_sub(24));
                lines_out.push(list_row(selected, &name, &right, inner_w));
            }
        }
        f.render_widget(
            Paragraph::new(lines_out).style(Style::default().bg(theme::SURFACE)),
            body,
        );
    }

    if state.mode == InteractionMode::SelectingModel {
        let rows = state.model_list.len().max(1) as u16;
        let rect = panel_rect(58, rows + 4);
        let body = draw_panel(f, rect, "模型", "↑↓ 选择 · 回车 确认 · esc 取消");
        let inner_w = body.width as usize;
        let mut lines_out: Vec<Line<'static>> = Vec::new();
        if state.model_list.is_empty() {
            lines_out.push(Line::from(Span::styled(
                "  当前提供商没有可用模型",
                faint(),
            )));
        } else {
            for (idx, item) in state.model_list.iter().enumerate() {
                let selected = idx == state.selected_model_index;
                let active = item.id == state.model;
                let name = truncate_width(&item.name, inner_w.saturating_sub(20));
                let right = if active {
                    format!("{}  当前", item.id)
                } else {
                    item.id.clone()
                };
                lines_out.push(list_row(selected, &name, &right, inner_w));
            }
        }
        f.render_widget(
            Paragraph::new(lines_out).style(Style::default().bg(theme::SURFACE)),
            body,
        );
    }

    if state.mode == InteractionMode::AddingProvider {
        let rect = panel_rect(60, 11);
        let body = draw_panel(f, rect, "新增提供商", "回车 下一步 · esc 取消");
        let (step, prompt) = match state.add_provider_step {
            AddingProviderStep::Name => ("1 / 3  name", "Provider 名称"),
            AddingProviderStep::BaseUrl => ("2 / 3  base url", "Base URL（http:// 或 https://）"),
            AddingProviderStep::ApiKey => ("3 / 3  api key", "API Key（本地服务可留空）"),
            AddingProviderStep::Commit => ("saving", "正在保存…"),
        };
        let mut rows: Vec<Line<'static>> = Vec::new();
        rows.push(Line::from(vec![
            Span::styled(format!("  {:<18}", step), faint()),
            Span::styled(prompt.to_string(), muted()),
        ]));
        rows.push(blank());
        let value = if state.add_provider_step == AddingProviderStep::ApiKey && !state.input_buffer.is_empty()
        {
            "•".repeat(state.input_buffer.chars().count())
        } else {
            state.input_buffer.clone()
        };
        rows.push(Line::from(vec![
            Span::styled("  › ", Style::default().fg(theme::ACCENT)),
            Span::styled(value, ink()),
            Span::styled(CARET, Style::default().fg(theme::ACCENT)),
        ]));
        if !state.status_text.is_empty()
            && (state.status_text.contains("错误") || state.status_text.contains("不能"))
        {
            rows.push(blank());
            rows.push(Line::from(Span::styled(
                format!("  {}", state.status_text),
                Style::default().fg(theme::DANGER),
            )));
        }
        f.render_widget(
            Paragraph::new(rows).style(Style::default().bg(theme::SURFACE)),
            body,
        );
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
    // wave-14：非 TTY 环境（stdin 已关闭 / tmux 非交互管道）下 enable_raw_mode 会报
    // "No such device or address (os error 6)" 且诊断信息会被后续 dup2 吞进日志文件，用户侧零输出。
    // 必须在 dup2 重定向之前提前优雅退出并直接向真实 stderr 打印诊断信息。
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        eprintln!(
            "pony TUI: stdin/stdout 不是交互式终端，TUI 无法启动。\n请在真实终端中运行 `pony`，或使用 `ponycli`（非交互 CLI 入口）。"
        );
        std::process::exit(1);
    }

    // 将底层 Rust 内核对 stderr/stdout 的日志直接重定向到文件或忽略，
    // 避免 [pony-runtime]、[pony-provider] 等原生 println/eprintln 打印污染 crossterm 屏幕，
    // 防止全屏 TUI 界面被破坏退化为终端滚屏输出。
    let log_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pony-agent")
        .join("tui-logs");
    let _ = std::fs::create_dir_all(&log_dir);
    // wave-13：日志文件绑定只在 unix 分支使用（dup2 重定向）。声明移入 #[cfg(unix)] 块内，
    // 使非 unix 目标（Windows CI）不存在该绑定，避免 `unused variable: log_file`。
    #[cfg(unix)]
    {
        let log_file = log_dir.join("tui-runtime.log");
        if let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&log_file) {
            use std::os::unix::io::AsRawFd;
            let fd = file.as_raw_fd();
            unsafe {
                libc::dup2(fd, libc::STDERR_FILENO);
            }
        }
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let state = TuiState::default();
    let control_plane = Arc::new(build_control_plane(&state.workspace, &state.gateway_url, &state.model));

    let (tx, rx): (Sender<UiEvent>, Receiver<UiEvent>) = channel();

    // T2c：无论主循环以何种方式退出（Ctrl-C、/quit、错误 `?` 传播、panic），
    // 都必须恢复终端（disable raw mode + LeaveAlternateScreen + DisableMouseCapture + 光标可见）。
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        tui_event_loop(&mut terminal, state, control_plane, rx, tx)
    }));

    eprintln!("[pony-tui] running terminal restore (T2c)");
    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture);
    let _ = terminal.show_cursor();

    match result {
        Ok(inner) => inner,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

fn tui_event_loop(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    mut state: TuiState,
    control_plane: Arc<pony_agent_core::agent::control_plane::HostControlPlane>,
    rx: Receiver<UiEvent>,
    tx: Sender<UiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
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
                    Ok(Event::Resize(_, _)) => {
                        let _ = tx_events.send(UiEvent::Resized);
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
    install_shutdown_signal_handlers();

    // 事件驱动重绘：无输入、无流式时不做无意义的 60fps 全屏重绘（省 CPU / 省电）。
    let mut dirty = true;
    loop {
        // T2c：SIGINT/SIGTERM 置位后优雅退出主循环，走统一的终端恢复路径。
        if SHUTDOWN_REQUESTED.load(Ordering::SeqCst) {
            break;
        }
        // 当处于流式思考状态时，按 400ms 节奏驱动呼吸点与耗时刷新（安静、不抢注意力）
        if state.is_streaming && last_tick_instant.elapsed() >= Duration::from_millis(400) {
            state.spinner_tick = state.spinner_tick.wrapping_add(1);
            last_tick_instant = std::time::Instant::now();
            dirty = true;
        }

        if dirty {
            terminal.draw(|f| ui(f, &mut state))?;
            dirty = false;
        }

        let poll_ms = if state.is_streaming { 16 } else { 120 };
        match rx.recv_timeout(Duration::from_millis(poll_ms)) {
            Ok(event) => {
                dirty = true;
                match event {
                UiEvent::Resized => {
                    // 尺寸变化后先清空物理终端，避免旧尺寸余晖与新画面叠加成「乱排」。
                    let _ = terminal.clear();
                    dirty = true;
                }
                UiEvent::ScrollWheel(is_up) => {
                    if is_up {
                        state.scroll_up(3);
                    } else {
                        state.scroll_down(3, u16::MAX);
                    }
                }
                UiEvent::InputKey(key) => {
                    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                        // 有草稿时先清空，避免一次误触丢掉整段输入；空输入才真正退出。
                        if !state.input_buffer.is_empty() && !state.is_streaming {
                            state.clear_input();
                            continue;
                        }
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
                                state.backspace();
                            }
                            KeyCode::Char(c) => {
                                state.insert_char(c);
                            }
                            KeyCode::Enter => {
                                let input_val = state.input_buffer.trim().to_string();
                                state.add_provider_input(&input_val);
                                state.clear_input();

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
                            state.clear_input();
                        }
                        continue;
                    }

                    if state.is_streaming {
                        continue;
                    }

                    // 编辑快捷键（优先于字符插入，避免 Ctrl 组合键被当作普通字符）
                    if key.modifiers.contains(KeyModifiers::CONTROL) {
                        match key.code {
                            KeyCode::Char('j') => {
                                state.insert_newline();
                                continue;
                            }
                            KeyCode::Char('u') => {
                                state.clear_input();
                                continue;
                            }
                            KeyCode::Char('w') => {
                                state.delete_word_backward();
                                continue;
                            }
                            KeyCode::Char('a') => {
                                state.cursor_line_home();
                                continue;
                            }
                            KeyCode::Char('e') => {
                                state.cursor_line_end();
                                continue;
                            }
                            _ => {}
                        }
                    }

                    // Shift+Enter / Alt+Enter：换行而不发送
                    if key.code == KeyCode::Enter
                        && (key.modifiers.contains(KeyModifiers::SHIFT)
                            || key.modifiers.contains(KeyModifiers::ALT))
                    {
                        state.insert_newline();
                        continue;
                    }

                    match key.code {
                        KeyCode::Tab => {
                            let match_cmds = state.matching_commands();
                            if !match_cmds.is_empty() {
                                let idx = state.selected_command_index.min(match_cmds.len() - 1);
                                let completed = match_cmds[idx].0.to_string();
                                state.set_input(&completed);
                            }
                        }
                        KeyCode::Enter => {
                            // 行尾反斜杠 + Enter：换行（不给网关发送半句话）
                            if state.input_buffer.ends_with('\\') {
                                state.backspace();
                                state.insert_newline();
                                continue;
                            }
                            let match_cmds = state.matching_commands();
                            if !match_cmds.is_empty() && state.input_buffer.starts_with('/') && !state.input_buffer.ends_with(' ') && state.input_buffer != match_cmds[state.selected_command_index.min(match_cmds.len() - 1)].0 {
                                let idx = state.selected_command_index.min(match_cmds.len() - 1);
                                let completed = match_cmds[idx].0.to_string();
                                state.set_input(&completed);
                                continue;
                            }
                            let input = state.input_buffer.trim().to_string();
                            if !input.is_empty() {
                                state.clear_input();
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
        plan_execution_approval: None,
                                        },
                                    };
                                    cp.start_turn_stream(&sink, cmd);
                                });
                            }
                        }
                        KeyCode::Char(c) => {
                            if c == '\n' {
                                state.insert_newline();
                            } else if c != '\r' {
                                state.insert_char(c);
                            }
                        }
                        KeyCode::Backspace => {
                            state.backspace();
                        }
                        KeyCode::Delete => {
                            state.delete_forward();
                        }
                        KeyCode::Left => {
                            state.move_cursor_left();
                        }
                        KeyCode::Right => {
                            state.move_cursor_right();
                        }
                        KeyCode::Home => {
                            state.cursor_line_home();
                        }
                        KeyCode::End => {
                            state.cursor_line_end();
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
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    // 终端恢复已统一由 run_tui 的 T2c 保证器执行（catch_unwind 外层）。
    Ok(())
}

// wave-34：非交互 CLI 入口。`pony --version / --help / --doctor` 在不初始化终端
//（不 enable_raw_mode、不进出备屏）的前提下输出信息后退出，消除 SSH/脚本排障盲点。
#[derive(Debug)]
pub enum CliAction {
    RunTui,
    Version,
    Help,
    Doctor,
    Invalid(String),
}

pub fn parse_cli_action(args: &[String]) -> CliAction {
    match args.iter().next() {
        None => CliAction::RunTui,
        Some(a) if a == "--version" || a == "-V" => CliAction::Version,
        Some(a) if a == "--help" || a == "-h" => CliAction::Help,
        Some(a) if a == "--doctor" => CliAction::Doctor,
        Some(a) if a.starts_with('-') => CliAction::Invalid(a.clone()),
        Some(_) => CliAction::RunTui,
    }
}

fn cli_version_string() -> String {
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
    format!(
        "pony {} · {}-{}\nbinary: {}",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        exe
    )
}

fn cli_help_string() -> String {
    format!(
        "pony {} — 交互式 TUI Agent\n\n\
用法: pony [选项]\n\n\
选项:\n  --version, -V   显示版本与二进制路径后退出\n  --help, -h      显示本帮助后退出\n  --doctor        终端环境自检（TTY/颜色/尺寸/locale/日志目录/PATH 冲突）\n\n\
交互键位:\n  Enter           发送输入\n  Alt+Enter       输入内换行\n  /               命令前缀（/help /clear /quit /exit 等）\n  Esc             清空输入 / 中断弹窗\n  Ctrl-C          退出 TUI\n  ↑ / ↓           历史翻页\n  PageUp/PageDown 消息滚动\n",
        env!("CARGO_PKG_VERSION")
    )
}

fn run_doctor() -> i32 {
    let mut fail = 0usize;

    // a) TTY：stdin 必须是 TTY（TUI 读取键盘）；stdout 允许被重定向（便于把报告存档/管道断言）。
    let in_tty = std::io::stdin().is_terminal();
    let out_tty = std::io::stdout().is_terminal();
    match (in_tty, out_tty) {
        (true, true) => println!("[ok] stdio: stdin=TTY stdout=TTY · 交互终端就绪"),
        (true, false) => println!("[warn] stdio: stdout 已重定向 · 报告可正常输出，TUI 内仍使用真实终端"),
        (false, _) => {
            fail += 1;
            println!("[fail] stdio: stdin 非交互终端（TUI 需要 TTY；CLI 场景可用 ponycli）");
        }
    }

    // b) 颜色环境
    let term = std::env::var("TERM").unwrap_or_default();
    let colorterm = std::env::var("COLORTERM").unwrap_or_default();
    let nocolor = std::env::var("NO_COLOR").unwrap_or_default();
    if term.is_empty() {
        println!("[warn] TERM: <unset> · 真彩色可能降级");
    } else {
        println!("[ok] TERM: {term}");
    }
    if colorterm.contains("truecolor") || colorterm.contains("24bit") {
        println!("[ok] COLORTERM: {colorterm} · 真彩色可用");
    } else {
        let shown = if colorterm.is_empty() { "<unset>" } else { &colorterm };
        println!("[warn] COLORTERM: {shown} · 未启用 truecolor");
    }
    if nocolor.is_empty() {
        println!("[ok] NO_COLOR: <unset> · 颜色输出未被禁用");
    } else {
        println!("[warn] NO_COLOR: {nocolor} · 颜色输出被禁用");
    }

    // c) 终端尺寸
    match crossterm::terminal::size() {
        Ok((w, h)) if w >= 8 && h >= 24 => println!("[ok] size: {w}x{h} · ≥24 行 × 8 列"),
        Ok((w, h)) => {
            fail += 1;
            println!("[fail] size: {w}x{h} · 低于最低要求 24 行 × 8 列，界面将错乱");
        }
        Err(e) => {
            fail += 1;
            println!("[fail] size: <n/a> · 无法读取终端尺寸: {e}");
        }
    }

    // d) locale UTF-8
    let mut utf8_ok = false;
    let mut seen = String::new();
    for v in ["LC_ALL", "LC_CTYPE", "LANG"] {
        if let Ok(val) = std::env::var(v) {
            if !val.is_empty() {
                seen.push_str(&format!("{v}={val} "));
                let lower = val.to_lowercase();
                if lower.contains("utf-8") || lower.contains("utf8") {
                    utf8_ok = true;
                }
            }
        }
    }
    let seen = seen.trim().to_string();
    if utf8_ok {
        println!("[ok] locale: {seen} · UTF-8 就绪");
    } else if seen.is_empty() {
        println!("[warn] locale: <unset> · LC_ALL/LC_CTYPE/LANG 未设置，中文/CJK 可能错位");
    } else {
        println!("[warn] locale: {seen} · 非 UTF-8 locale，中文/CJK 可能错位");
    }

    // e) tui-logs 可写
    let log_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".pony-agent")
        .join("tui-logs");
    if std::fs::create_dir_all(&log_dir).is_ok() {
        let probe = log_dir.join(".doctor-probe");
        match std::fs::write(&probe, b"ok") {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
                println!("[ok] tui-logs: {} · 可写", log_dir.display());
            }
            Err(e) => {
                fail += 1;
                println!("[fail] tui-logs: {} · 不可写: {e}", log_dir.display());
            }
        }
    } else {
        fail += 1;
        println!("[fail] tui-logs: {} · 无法创建目录", log_dir.display());
    }

    // f) PATH 冲突：按 canonicalize 去重（同一 realpath 只列一条），且仅列「与当前 exe 不同」者。
    let exe_self = std::env::current_exe().ok().and_then(|p| std::fs::canonicalize(p).ok());
    let mut conflicts: Vec<PathBuf> = Vec::new();
    let mut seen_canon: Vec<PathBuf> = Vec::new();
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let cand = dir.join("pony");
            if !cand.is_file() {
                continue;
            }
            let canon = std::fs::canonicalize(&cand).unwrap_or_else(|_| cand.clone());
            if Some(&canon) == exe_self.as_ref() || seen_canon.contains(&canon) {
                continue;
            }
            seen_canon.push(canon);
            conflicts.push(cand);
        }
    }
    if conflicts.is_empty() {
        println!("[ok] PATH: 无 pony 冲突");
    } else {
        let list = conflicts.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join(" ");
        println!("[warn] PATH: {list} · PATH 冲突：SSH 下可能调到其它 pony 二进制");
    }

    if fail == 0 { 0 } else { 1 }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse_cli_action(&args) {
        CliAction::Version => {
            println!("{}", cli_version_string());
            std::process::exit(0);
        }
        CliAction::Help => {
            println!("{}", cli_help_string());
            std::process::exit(0);
        }
        CliAction::Doctor => {
            std::process::exit(run_doctor());
        }
        CliAction::Invalid(arg) => {
            eprintln!("pony: 未知参数 {arg}\n用法: pony [--version | --help | --doctor]");
            std::process::exit(2);
        }
        CliAction::RunTui => {
            let _guard = pony_agent_core::agent::runtime_helper::TestRuntimeGuard::new();
            if let Err(e) = run_tui() {
                eprintln!("TUI Error: {e}");
                std::process::exit(1);
            }
        }
    }
}
