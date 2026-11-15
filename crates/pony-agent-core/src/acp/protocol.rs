//! ACP v1 协议类型（Pony as ACP Client）。
//!
//! 真源：`docs/architecture/acp-client-integration.md` §1/§3/§4 P0 表 + 官方
//! `agent-client-protocol` v1 `schema.json`（`ProtocolVersion` = 整数 `1`）。
//!
//! 冻结的序列化不变量（Executor / Test Agent 只读，不得就地改）：
//! - 结构体字段 → `camelCase`（`mimeType` / `toolCallId` / `sessionId` / `optionId` …）；
//! - 枚举值 → `snake_case`（`end_turn` / `tool_call` / `allow_once` / `in_progress` …）；
//! - 判别键：`ContentBlock.type`、`SessionUpdate.sessionUpdate`、`ToolCallContent.type`、
//!   `RequestPermissionOutcome.outcome`、`McpServer.type`、`AuthMethod.type`；
//! - `protocolVersion` 是**整数**（`u32`，v1 == 1），不是字符串；
//! - `session/cancel`、`session/update` 是 notification；`session/request_permission`
//!   是 **agent→client 的请求**（客户端必须应答，取消时回 `cancelled`）。
//!
//! 本文件只承载类型契约，不含传输 / 会话 / 治理逻辑（那是 `transport` / P1 的职责）。

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// ACP v1 协议版本号。官方为整数；不匹配时客户端必须 fail-closed。
pub const PROTOCOL_VERSION: u32 = 1;

/// ACP v1 方法名（client→agent 请求/通知 + agent→client 请求/通知）。
pub mod method {
    // client → agent（请求）
    pub const INITIALIZE: &str = "initialize";
    pub const SESSION_NEW: &str = "session/new";
    pub const SESSION_PROMPT: &str = "session/prompt";
    // client → agent（B179 会话生命周期）
    pub const SESSION_LIST: &str = "session/list";
    pub const SESSION_LOAD: &str = "session/load";
    pub const SESSION_RESUME: &str = "session/resume";
    pub const SESSION_CLOSE: &str = "session/close";
    pub const SESSION_DELETE: &str = "session/delete";
    /// `session/fork`（官方 v1；实测仅 opencode 支持，agy 回 `-32601`）。
    pub const SESSION_FORK: &str = "session/fork";
    // client → agent（认证：本轮只做类型 + 不支持时降级，不做交互式登录）
    pub const AUTHENTICATE: &str = "authenticate";
    pub const LOGOUT: &str = "logout";
    // client → agent（notification）
    pub const SESSION_CANCEL: &str = "session/cancel";
    // agent → client（notification）
    pub const SESSION_UPDATE: &str = "session/update";
    // agent → client（请求，客户端必须应答）
    pub const SESSION_REQUEST_PERMISSION: &str = "session/request_permission";
    pub const FS_READ_TEXT_FILE: &str = "fs/read_text_file";
    pub const FS_WRITE_TEXT_FILE: &str = "fs/write_text_file";
    pub const TERMINAL_CREATE: &str = "terminal/create";
    pub const TERMINAL_OUTPUT: &str = "terminal/output";
    pub const TERMINAL_WAIT_FOR_EXIT: &str = "terminal/wait_for_exit";
    pub const TERMINAL_KILL: &str = "terminal/kill";
    pub const TERMINAL_RELEASE: &str = "terminal/release";
}

// ───────────────────────────── 初始化 / 能力 ─────────────────────────────

/// 客户端 / agent 的实现自述（`clientInfo` / `agentInfo`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Implementation {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub version: String,
}

/// 认证方式类型；官方默认（字段缺省）为 `agent`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethodType {
    #[default]
    Agent,
    Terminal,
}

/// `initialize` 响应里的认证方式（`authMethods`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthMethod {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(
        rename = "type",
        default,
        skip_serializing_if = "is_default_auth_method_type"
    )]
    pub method_type: AuthMethodType,
}

fn is_default_auth_method_type(value: &AuthMethodType) -> bool {
    *value == AuthMethodType::Agent
}

/// 客户端能力（`initialize` 请求的 `clientCapabilities`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClientCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fs: Option<FileSystemCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth: Option<AuthCapabilities>,
    /// `_meta` 扩展位（B180）：与 agent 协商私有能力。ACP v1 的 `_meta` 约定为「实现方不得
    /// 假设其内容」的透传面包；opencode 要求客户端回显
    /// `{"opencode/child-session-updates": true}` 才推送子会话更新，另有
    /// `session.compaction` / `elicitation.form` 等私有键。宿主只透传，不解读。
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// 文件系统能力：仅在 Pony 真正能执行 `fs/*` 治理链时才声明（fail-closed）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FileSystemCapabilities {
    #[serde(default)]
    pub read_text_file: bool,
    #[serde(default)]
    pub write_text_file: bool,
}

/// 终端型认证能力（`auth.terminal`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AuthCapabilities {
    #[serde(default)]
    pub terminal: bool,
}

/// agent 能力（`initialize` 响应的 `agentCapabilities`）；缺字段一律按 `false` 处理。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    #[serde(default)]
    pub load_session: bool,
    #[serde(default)]
    pub prompt_capabilities: PromptCapabilities,
    #[serde(default)]
    pub mcp_capabilities: McpCapabilities,
    #[serde(default)]
    pub session_capabilities: SessionCapabilities,
}

/// 提示内容能力（官方 `promptCapabilities`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptCapabilities {
    #[serde(default)]
    pub image: bool,
    #[serde(default)]
    pub audio: bool,
    #[serde(default)]
    pub embedded_context: bool,
}

/// MCP 传输能力（官方 `mcpCapabilities`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct McpCapabilities {
    #[serde(default)]
    pub http: bool,
    #[serde(default)]
    pub sse: bool,
}

/// 会话能力广告的取值形态：官方 v1 用 `{}` 表示「支持」，布尔式广告也一并容忍。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CapabilityPresence {
    Marker(serde_json::Map<String, Value>),
    Flag(bool),
}

/// 会话生命周期能力（官方 `sessionCapabilities`）。
///
/// 注：`fork` 来自本仓库真源 §1 的 opencode 能力清单（官方最新 schema 已移除该项），
/// 解析保留以兼容旧版 agent 广告；未知能力字段由 serde 默认忽略。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionCapabilities {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_directories: Option<CapabilityPresence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub close: Option<CapabilityPresence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delete: Option<CapabilityPresence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fork: Option<CapabilityPresence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub list: Option<CapabilityPresence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<CapabilityPresence>,
}

/// `initialize` 请求（client→agent）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeRequest {
    pub protocol_version: u32,
    #[serde(default)]
    pub client_capabilities: ClientCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_info: Option<Implementation>,
}

/// `initialize` 响应（agent→client）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResponse {
    pub protocol_version: u32,
    #[serde(default)]
    pub agent_capabilities: AgentCapabilities,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_info: Option<Implementation>,
    #[serde(default)]
    pub auth_methods: Vec<AuthMethod>,
}

// ───────────────────────────── 会话建立 / 提示 ─────────────────────────────

/// MCP server 描述（`session/new` 的 `mcpServers`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpServer {
    Stdio(McpServerStdio),
    Http(McpServerHttp),
    Sse(McpServerSse),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStdio {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<EnvVariable>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerHttp {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<HttpHeader>,
    #[serde(rename = "type")]
    pub transport: McpHttpType,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerSse {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub headers: Vec<HttpHeader>,
    #[serde(rename = "type")]
    pub transport: McpSseType,
}

/// `McpServerHttp` 的 `type` 判别值（`"http"`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpHttpType {
    Http,
}

/// `McpServerSse` 的 `type` 判别值（`"sse"`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpSseType {
    Sse,
}

/// 环境变量（`terminal/create` 与 `McpServerStdio` 共用）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvVariable {
    pub name: String,
    pub value: String,
}

/// HTTP 头。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpHeader {
    pub name: String,
    pub value: String,
}

/// `session/new` 请求（client→agent）；`cwd` 必须是绝对路径。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNewRequest {
    pub cwd: String,
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_directories: Option<Vec<String>>,
}

/// `session/new` 响应（agent→client）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNewResponse {
    pub session_id: String,
}

/// `session/prompt` 请求（client→agent）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptRequest {
    pub session_id: String,
    pub prompt: Vec<ContentBlock>,
}

/// `session/prompt` 响应（agent→client）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptResponse {
    pub stop_reason: StopReason,
}

/// 五个停止原因（官方 v1 全量）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    MaxTokens,
    MaxTurnRequests,
    Refusal,
    Cancelled,
}

/// `session/cancel` **notification**（client→agent）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelNotification {
    pub session_id: String,
}

// ───────────────────────────── 内容块 ─────────────────────────────

/// 提示内容块（`ContentBlock[]`，判别键 `type`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    #[serde(rename_all = "camelCase")]
    Text { text: String },
    #[serde(rename_all = "camelCase")]
    Image {
        data: String,
        mime_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        uri: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Audio { data: String, mime_type: String },
    #[serde(rename_all = "camelCase")]
    ResourceLink {
        name: String,
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mime_type: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        size: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Resource { resource: EmbeddedResourceResource },
}

/// 内嵌资源载荷：文本或二进制（判别靠必填字段，故用 untagged）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum EmbeddedResourceResource {
    Text(TextResourceContents),
    Blob(BlobResourceContents),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextResourceContents {
    pub uri: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobResourceContents {
    pub uri: String,
    pub blob: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

// ───────────────────────────── session/update ─────────────────────────────

/// `session/update` **notification** 载荷。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionNotification {
    pub session_id: String,
    pub update: SessionUpdate,
}

/// 8 个 v1 update 变体（判别键 `sessionUpdate`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "sessionUpdate", rename_all = "snake_case")]
pub enum SessionUpdate {
    #[serde(rename_all = "camelCase")]
    UserMessageChunk {
        content: ContentBlock,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    AgentMessageChunk {
        content: ContentBlock,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    AgentThoughtChunk {
        content: ContentBlock,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message_id: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    ToolCall {
        tool_call_id: String,
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<ToolKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ToolCallStatus>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        content: Vec<ToolCallContent>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        locations: Vec<ToolCallLocation>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw_input: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw_output: Option<Value>,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallUpdate {
        tool_call_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<ToolKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<ToolCallStatus>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        content: Option<Vec<ToolCallContent>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        locations: Option<Vec<ToolCallLocation>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw_input: Option<Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw_output: Option<Value>,
    },
    #[serde(rename_all = "camelCase")]
    Plan { entries: Vec<PlanEntry> },
    #[serde(rename_all = "camelCase")]
    AvailableCommandsUpdate {
        available_commands: Vec<AvailableCommand>,
    },
    #[serde(rename_all = "camelCase")]
    CurrentModeUpdate { current_mode_id: String },
    // ── B166：官方 v1 余下 6 个变体（此前封闭枚举会把整条 update 反序列化失败并丢弃）──
    /// `config_option_update`：会话配置项全量更新。
    ConfigOptionUpdate(ConfigOptionUpdate),
    /// `session_info_update`：会话元数据（标题 / 最近活动时间）部分更新。
    SessionInfoUpdate(SessionInfoUpdate),
    /// `usage_update`：上下文窗口用量与累计 cost（opencode 实测在发）。
    UsageUpdate(UsageUpdate),
    /// `notice`：面向用户的即时通知（需客户端声明 `session.notices` 私有能力）。
    Notice(Notice),
    /// `compaction_update`：上下文压缩实体 upsert（需 `session.compaction` 私有能力）。
    CompactionUpdate(CompactionUpdate),
    /// `compaction_summary_chunk`：压缩摘要的内容块追加。
    CompactionSummaryChunk(CompactionSummaryChunk),
    /// **未知变体兜底**：未来/厂商私有 `sessionUpdate` 一律降级到本变体，不整条丢弃；
    /// 调用方按诊断记录（stderr + warning），不得 panic。
    #[serde(other)]
    Unknown,
}

/// 工具调用类别（官方 v1 全量）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Edit,
    Delete,
    Move,
    Search,
    Execute,
    Think,
    Fetch,
    SwitchMode,
    Other,
}

/// 工具调用状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

/// 工具调用产出的内容（判别键 `type`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolCallContent {
    #[serde(rename_all = "camelCase")]
    Content { content: ContentBlock },
    #[serde(rename_all = "camelCase")]
    Diff {
        path: String,
        new_text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        old_text: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Terminal { terminal_id: String },
}

/// 受影响的文件位置（`follow-along` 用）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallLocation {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
}

/// 计划条目（`plan` update）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    pub content: String,
    pub priority: PlanEntryPriority,
    pub status: PlanEntryStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanEntryPriority {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanEntryStatus {
    Pending,
    InProgress,
    Completed,
}

/// 可用命令（`available_commands_update` update）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableCommand {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<AvailableCommandInput>,
}

/// 命令输入规格（官方 `unstructured` 形态）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableCommandInput {
    pub hint: String,
}

// ───────────────────────── agent→client：权限请求 ─────────────────────────

/// `session/request_permission` 请求（agent→client）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPermissionRequest {
    pub session_id: String,
    pub tool_call: ToolCallUpdate,
    pub options: Vec<PermissionOption>,
}

/// 工具调用更新（权限请求里携带的 `toolCall` 形态）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallUpdate {
    pub tool_call_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ToolKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ToolCallStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ToolCallContent>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locations: Option<Vec<ToolCallLocation>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_output: Option<Value>,
}

/// 权限选项（`optionId` / `name` / `kind`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionOption {
    pub option_id: String,
    pub name: String,
    pub kind: PermissionOptionKind,
}

/// 权限选项类别（官方 v1 四变体）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionOptionKind {
    AllowOnce,
    AllowAlways,
    RejectOnce,
    RejectAlways,
}

/// `session/request_permission` 响应（client→agent）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RequestPermissionResponse {
    pub outcome: RequestPermissionOutcome,
}

/// 审批结果（判别键 `outcome`）；`cancelled` 是取消链路的强制应答。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RequestPermissionOutcome {
    #[serde(rename_all = "camelCase")]
    Selected {
        option_id: String,
    },
    Cancelled,
}

// ───────────────────────── agent→client：文件系统 ─────────────────────────

/// `fs/read_text_file` 请求（agent→client，仅当声明 `fs.readTextFile`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadTextFileRequest {
    pub session_id: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// `fs/read_text_file` 响应。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadTextFileResponse {
    pub content: String,
}

/// `fs/write_text_file` 请求（agent→client，仅当声明 `fs.writeTextFile`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteTextFileRequest {
    pub session_id: String,
    pub path: String,
    pub content: String,
}

/// `fs/write_text_file` 响应（空对象）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct WriteTextFileResponse {}

// ───────────────────────── agent→client：终端 ─────────────────────────

/// `terminal/create` 请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTerminalRequest {
    pub session_id: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<Vec<EnvVariable>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_byte_limit: Option<u64>,
}

/// `terminal/create` 响应。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateTerminalResponse {
    pub terminal_id: String,
}

/// `terminal/output` 请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutputRequest {
    pub session_id: String,
    pub terminal_id: String,
}

/// `terminal/output` 响应。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TerminalOutputResponse {
    pub output: String,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_status: Option<TerminalExitStatus>,
}

/// `terminal/wait_for_exit` 请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitForTerminalExitRequest {
    pub session_id: String,
    pub terminal_id: String,
}

/// `terminal/wait_for_exit` 响应。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WaitForTerminalExitResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
}

/// `terminal/kill` 请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KillTerminalRequest {
    pub session_id: String,
    pub terminal_id: String,
}

/// `terminal/kill` 响应（空对象）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct KillTerminalResponse {}

/// `terminal/release` 请求。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseTerminalRequest {
    pub session_id: String,
    pub terminal_id: String,
}

/// `terminal/release` 响应（空对象）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ReleaseTerminalResponse {}

/// 终端退出状态（`terminal/output` 的 `exitStatus`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TerminalExitStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signal: Option<String>,
}

// ───────────── B166/B180：SessionUpdate 扩展类型（官方 v1 schema 对齐） ─────────────

/// `config_option_update` 载荷：会话配置项全量快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigOptionUpdate {
    pub config_options: Vec<SessionConfigOption>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// 会话配置项：`type` 判别的 select / boolean 两形态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionConfigOption {
    #[serde(rename_all = "camelCase")]
    Select {
        id: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        /// 语义分类（`mode` / `model` / `model_config` / `thought_level` / 私有 `_*`）；
        /// 客户端必须容忍缺失与未知值。
        #[serde(default, skip_serializing_if = "Option::is_none")]
        category: Option<String>,
        current_value: String,
        options: SessionConfigSelectOptions,
        #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
        meta: Option<Value>,
    },
    #[serde(rename_all = "camelCase")]
    Boolean {
        id: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        category: Option<String>,
        current_value: bool,
        #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
        meta: Option<Value>,
    },
}

/// select 选项集合：扁平数组或「分组 → 组内数组」两种官方形态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SessionConfigSelectOptions {
    Grouped(Vec<SessionConfigSelectGroup>),
    Ungrouped(Vec<SessionConfigSelectOption>),
}

/// 分组的 select 选项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfigSelectGroup {
    pub name: String,
    pub options: Vec<SessionConfigSelectOption>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// 单个 select 选项值。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfigSelectOption {
    pub value: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// `session_info_update` 载荷：全部字段可选（支持部分更新；`null` 表示清除）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfoUpdate {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// `usage_update` 载荷：上下文窗口用量 + 可选累计 cost（opencode 实测）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageUpdate {
    /// 当前上下文内 token 数。
    pub used: u64,
    /// 上下文窗口总大小（token）。
    pub size: u64,
    /// 会话累计 cost（可选）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<Cost>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// 累计 cost（`amount` + `currency`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cost {
    pub amount: f64,
    pub currency: String,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// `notice` 载荷：面向用户的即时通知。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub severity: NoticeSeverity,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// notice 严重级别：官方 3 值 + 未知值兜底（保留原始字符串）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeSeverity {
    Info,
    Warning,
    Error,
    /// 未知/厂商扩展值（原样保留）。
    Unknown(String),
}

impl Serialize for NoticeSeverity {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Self::Info => "info",
            Self::Warning => "warning",
            Self::Error => "error",
            Self::Unknown(raw) => raw,
        })
    }
}

impl<'de> Deserialize<'de> for NoticeSeverity {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "info" => Self::Info,
            "warning" => Self::Warning,
            "error" => Self::Error,
            _ => Self::Unknown(raw),
        })
    }
}

/// `compaction_update` 载荷：压缩实体 upsert（patch 语义：省略=不变、null=清除）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionUpdate {
    pub compaction_id: String,
    pub status: CompactionStatus,
    /// 完整替换的摘要内容块；`None` = 字段缺省（patch 语义保留旧值），
    /// 「显式 null 清除」由后续 executor 用三态解析补（当前按缺省处理）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<Vec<ContentBlock>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// 压缩生命周期状态：官方 4 值 + 未知值兜底。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionStatus {
    InProgress,
    Completed,
    Failed,
    Cancelled,
    /// 未知/厂商扩展值（原样保留）。
    Unknown(String),
}

impl Serialize for CompactionStatus {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match self {
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Unknown(raw) => raw,
        })
    }
}

impl<'de> Deserialize<'de> for CompactionStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "in_progress" => Self::InProgress,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Unknown(raw),
        })
    }
}

/// `compaction_summary_chunk` 载荷：向压缩摘要追加一个内容块。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionSummaryChunk {
    pub compaction_id: String,
    pub content: ContentBlock,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

// ───────────── B179：会话生命周期类型（官方 v1 schema 对齐） ─────────────

/// `session/list` 请求：`cwd` / `cursor` / `limit` 全部可选（agy 只列已落盘会话）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// `session/list` 返回的单条会话元数据（官方 `SessionInfo`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfo {
    pub session_id: String,
    pub cwd: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_directories: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<String>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// `session/list` 响应：会话数组 + 可选 nextCursor（缺省 = 无更多页）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListSessionsResponse {
    pub sessions: Vec<SessionInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// `session/load` 请求：`cwd` 必须绝对路径；`additionalDirectories` 由 client 校验。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadSessionRequest {
    pub cwd: String,
    pub session_id: String,
    #[serde(default)]
    pub mcp_servers: Vec<McpServer>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_directories: Option<Vec<String>>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// 会话模式状态（`session/load` / `session/resume` 响应的 `modes`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionModeState {
    pub current_mode_id: String,
    #[serde(default)]
    pub available_modes: Vec<SessionMode>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// 单个会话模式。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMode {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// `session/load` 响应（**两形态兼容**）：
///
/// - opencode 只回 `configOptions` / `modes`，**不含 `sessionId`**；
/// - agy 回 `sessionId` + `models` + `modes` 超集（`models` 等未知字段由 serde 忽略）。
///
/// 因此 `session_id` 必须是 `Option<String>`：缺失 = 沿用请求里的 session id（调用方负责回填）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LoadSessionResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_options: Option<Vec<SessionConfigOption>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modes: Option<SessionModeState>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// `session/resume` 请求：不重放历史；参数字段与 load 基本同形（`mcpServers` 可选）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeSessionRequest {
    pub cwd: String,
    pub session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<Vec<McpServer>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub additional_directories: Option<Vec<String>>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// `session/resume` 响应：与 load 同形（无 sessionId ⇒ 沿用请求 id）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ResumeSessionResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_options: Option<Vec<SessionConfigOption>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modes: Option<SessionModeState>,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// `session/close` 请求（opencode/agy 均幂等：不存在的 id 回 `{}`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloseSessionRequest {
    pub session_id: String,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// `session/close` 响应（空对象）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct CloseSessionResponse {}

/// `session/delete` 请求（agy 回 `-32601` ⇒ 客户端须降级为「不支持」，见 `Unsupported` 错误码）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteSessionRequest {
    pub session_id: String,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// `session/delete` 响应（空对象）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DeleteSessionResponse {}

/// `authenticate` 请求（本轮只做类型；交互式登录不实现）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthenticateRequest {
    pub method_id: String,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<Value>,
}

/// `authenticate` 响应（空对象）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AuthenticateResponse {}

/// `logout` 请求（本轮只做类型；不支持时按 `-32601` 降级）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LogoutRequest {}

/// `logout` 响应（空对象）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LogoutResponse {}
