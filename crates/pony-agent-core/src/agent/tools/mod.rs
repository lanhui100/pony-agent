//! Legacy `ToolRouter` builtin tool handlers (PA-076 migration surface).
//!
//! This module is the compatibility `ToolRouter` the runtime still routes tool calls through
//! until the governed dispatcher takes over. The `Run`/`WebFetch`/`Search`/`Glob` handlers are
//! wired to the hardened modules from phase 5/6 (PA-076 tasks 5.5, 6.3, 6.4):
//!
//! - [`crate::agent::process::ProcessManager`] backs `workspace_run_command` with the
//!   session-scoped process lifecycle (start / bounded poll / kill / timeout, PA-076 tasks
//!   5.3-5.5). The legacy Run fails closed on sandbox grounds: it always runs
//!   `enforce_sandbox` against the registered backend, and with no backend registered it
//!   treats the request as `NoSandboxBackend` (Unavailable) so the Run is `sandbox_denied` —
//!   never a silent downgrade to a full-parent-environment shell (phase-5 review P1-2).
//! - [`crate::agent::web_access::WebAccessPolicy`] + [`crate::agent::web_access::PinnedConnector`]
//!   gate `web_fetch_url` before any connection and pin every connection to the validated
//!   addresses (design Decision 8, PA-076 tasks 6.1-6.3 + item 2): the initial URL and each of up
//!   to 5 redirect hops are validated for scheme, credentials, host, resolved addresses, port and
//!   redirect budget, and every hop is sent through a per-hop client whose DNS resolution for the
//!   URL's hostname is overridden to the validated addresses (the `Host` header and TLS SNI
//!   preserve the original authority; only the socket target is pinned). Arbitrary hostname URLs
//!   still fail closed until a production `WebResolver` is injected (a governed-path follow-up).
//!   The client is built with no ambient proxy and no auto-redirects; there is no
//!   "validate then hand off to a re-resolving client" weak mode.
//! - [`crate::agent::search::SearchEngine`] replaces the wildcard pseudo-regex and custom
//!   traversal in `workspace_search_text` / `workspace_glob_files` with real regex, globset,
//!   `.gitignore`-aware traversal and explicit truncation evidence (design Decision 9).
//!
//! 模块地图（本文件只保留 `ToolRouter` 结构与 primitive 分发表）：
//!
//! - `contract`：契约元数据（`ToolKind`/权限声明/描述符/注册表快照/`ToolSurface`）；
//! - `exec`：执行面类型（`ToolCall`/`ToolResult`/`ToolOutcome`/上下文）与结果构造器；
//! - `schema`：`builtin_tools()` JSON schema 装配与 contract view；
//! - `names`：名称映射、别名、执行策略与默认权限事实；
//! - `workspace` / `paths`：文件工具面与路径解析判定；
//! - `run` / `web` / `composite`：命令执行、web 抓取、复合工具；
//! - `instructions` / `orchestration`：附加指令注入与团队/目标/任务桥接。

mod contract;
mod exec;
mod schema;
mod names;
pub(crate) mod prompt_contract;
mod workspace;
mod paths;
mod run;
mod web;
mod composite;
mod instructions;
mod orchestration;

#[cfg(test)]
mod contract_view_tests;
#[cfg(test)]
mod tests;

use crate::agent::process::ProcessManager;
pub use crate::agent::jobs::{
    job_kill, job_kill_scoped, job_list, job_list_scoped, job_output, job_output_scoped,
    register_promoted, job_start, job_start_scoped, job_start_with_workspace_root,
    JobKillArgs, JobKillResult, JobListArgs, JobListItem, JobListResult,
    JobOutputArgs, JobOutputResult, JobStartArgs, JobStartResult,
    JOB_BUFFER_CAP, MAX_HISTORY_JOBS,
};
pub use crate::agent::terminal::{
    terminal_close, terminal_open, terminal_open_with_workspace_root, terminal_read, terminal_send, terminal_signal,
    MAX_ACTIVE_TERMINALS, TERMINAL_BUFFER_CAP,
    TerminalCloseArgs, TerminalCloseResult, TerminalOpenArgs, TerminalOpenResult,
    TerminalReadArgs, TerminalReadResult, TerminalSendArgs, TerminalSendResult,
    TerminalSignal, TerminalSignalArgs, TerminalSignalResult,
};
pub use crate::agent::patch::{apply_patch, ApplyPatchArgs, ApplyPatchResult};
pub use crate::agent::lsp::{
    lsp_definition, lsp_diagnostics, lsp_hover, lsp_references, LspDefinitionArgs,
    LspDefinitionResult, LspDiagnosticItem, LspDiagnosticsArgs, LspDiagnosticsResult,
    LspHoverArgs, LspHoverResult, LspLocation, LspPosition, LspRange, LspReferencesArgs,
    LspReferencesResult,
};
pub use crate::agent::git_search::{
    fuzzy_file_search, git_diff_remote, FuzzyFileSearchArgs, FuzzyFileSearchResult,
    FuzzyMatchItem, GitDiffRemoteArgs, GitDiffRemoteResult,
};
pub use crate::agent::goal::{
    create_goal, get_goal, update_goal, CreateGoalArgs, CreateGoalResult, GetGoalArgs, GetGoalResult,
    GoalActivation, GoalData, UpdateGoalAction, UpdateGoalArgs, UpdateGoalResult,
};
pub use crate::agent::orchestration::{
    drain_inbox, interrupt_agent, list_agents, send_message, send_message_in_workspace,
    spawn_teammate, spawn_teammate_in_workspace, spawn_teammate_in_workspace_with_provider,
    subagent, subagent_in_workspace, subagent_in_workspace_with_provider, team_task_create,
    team_task_get, team_task_list, team_task_update, wait_agent, workflow,
    InterruptAgentArgs, InterruptAgentResult, ListAgentsResult, MessageDeliveryMode, QueuedMessage,
    ReadInboxArgs, ReadInboxResult, SendMessageArgs, SendMessageResult, SpawnTeammateArgs,
    SpawnTeammateResult, SubagentArgs, SubagentResult, TeamTaskAction, TeamTaskCreateArgs,
    TeamTaskGetArgs, TeamTaskItem, TeamTaskListResult, TeamTaskUpdateArgs, TeammateMember,
    TeammateStatus, WaitAgentArgs, WaitAgentNoProgress, WaitAgentResult, WorkflowArgs,
    WorkflowResult, WorkflowStep,
};
pub use crate::agent::present::{
    present, PresentArgs, PresentFileInput, PresentResult, PresentedFileItem,
};
pub use crate::agent::todo::{
    todo_list, todo_write, TodoItem, TodoListArgs, TodoListResult, TodoStatus, TodoWriteArgs,
    TodoWriteResult,
};
use crate::agent::runtime_helper::block_on;
use crate::agent::sandbox::{enforce_sandbox, NoSandboxBackend};
use crate::agent::search::{SearchEngine, SearchOptions};
use crate::agent::tool_runtime::{
    ProcessBackend, ProcessStartRequest, ProcessState, SandboxBackend, SandboxRequest,
    TurnToolView, WebResolver,
};
use crate::agent::web_access::{
    PinnedConnector, WebAccessDecision, WebAccessDenyReason, WebAccessPolicy,
};
use encoding_rs::{Encoding, GBK};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error;
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::net::{IpAddr, SocketAddr};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};
use sha2::{Digest, Sha256};
use url::Url;
use self::exec::error_result;
use self::web::FailClosedResolver;



pub use self::contract::{ToolDefinition, ToolDefinitionContractView, ToolDescriptor, ToolDescriptorSource, ToolDisplayMetadata, ToolError, ToolExecutionPolicy, ToolExposure, ToolHandlerProvenance, ToolIdentity, ToolKind, ToolPermissionDeclaration, ToolPermissionFacts, ToolPermissionScope, ToolProvider, ToolProviderError, ToolRegistrySnapshot, ToolSurface, builtin_tool_surface, builtin_turn_tool_contract_views};
pub use self::exec::{ToolCall, ToolExecutor, ToolControlKind, ToolControlOutcome, ToolExecutionContext, ToolExecutionStatus, ToolOutcome, ToolPlan, ToolPlanStep, ToolResult};
pub use self::schema::builtin_tools;
pub use self::prompt_contract::{prompt_non_tool_tokens, prompt_tool_references};
pub use self::names::{canonical_tool_name, default_permission_facts_for_name, model_visible_tool_name, model_visible_tool_name_opt, product_canonical_tool_name, product_visible_tool_name, tool_display_metadata_for_name, tool_exposure_for_name, tool_is_concurrent_safe, tool_kind_for_name};
pub use self::run::{classify_exit_code, classify_timeout_hint, resolve_run_timeout_ms};
pub(crate) use self::exec::{tool_error_from_output};
pub(crate) use self::workspace::{StartLineArg, explicit_gather_start_line, parse_start_line_arg, with_requested_start_line, with_start_line_fallback_meta};

pub(crate) fn compute_content_sha256(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// 写路径单点：先抓 pre-image 再落盘（保持调用点行数中性，见 ADR 0113）。
///
/// 返回 `io::Error` 语义与 `std::fs::write` 完全一致——抓取失败不影响写入结果。
pub(crate) fn write_with_rewind_capture(
    context: &ToolExecutionContext,
    target: &Path,
    bytes: &[u8],
) -> std::io::Result<()> {
    capture_rewind_pre_image(context, target);
    std::fs::write(target, bytes)
}

/// 「撤回」抓取点（唯一）：写工具落盘**前**把目标文件的变更前内容记进轮次快照。
///
/// 约定（ADR 0113）：
/// - 身份取自 `ToolExecutionContext` 的 turn 事实（`session_id` + `turn_id`）；缺任一项
///   ⇒ 不抓取（没有轮次键就无从归属，绝不猜）；
/// - **fail-open**：抓取失败只留一行 stderr，绝不阻断写入——撤回是便利能力，不是写门；
/// - 同一 `(session, turn, path)` 只抓第一次（存储侧幂等），多文件写不重复拷贝。
pub(crate) fn capture_rewind_pre_image(context: &ToolExecutionContext, target: &Path) {
    let Some(dispatch) = context.dispatch_context.as_ref() else {
        return;
    };
    let session_id = dispatch
        .session_id
        .as_deref()
        .or(context.session_id.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let turn_id = dispatch
        .turn_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let (Some(session_id), Some(turn_id)) = (session_id, turn_id) else {
        return;
    };
    if let Err(error) =
        crate::agent::session::RewindFileStore::shared().record_pre_image(session_id, turn_id, target)
    {
        tracing::error!(target: "pony_core", error = %error, "[pony-agent] rewind pre-image skipped");
    }
}

const TOOL_TIME_NOW: &str = "time_now";
const TOOL_ECHO_INPUT: &str = "echo_input";
/// First-class Ask tool (PA-114): product name `Ask` is owned by this primitive, replacing the
/// legacy `echo_input` placeholder on the model surface. Host-mediated (WaitingHost), never a
/// direct execution tool.
pub(crate) const TOOL_ASK_USER: &str = "ask_user";
const TOOL_WORKSPACE_LIST_FILES: &str = "workspace_list_files";
const TOOL_WORKSPACE_READ_FILE: &str = "workspace_read_file";
const TOOL_WORKSPACE_READ_FILE_SEGMENT: &str = "workspace_read_file_segment";
const TOOL_WORKSPACE_PATH_INFO: &str = "workspace_path_info";
const TOOL_WORKSPACE_SEARCH_TEXT: &str = "workspace_search_text";
const TOOL_WORKSPACE_GLOB_FILES: &str = "workspace_glob_files";
const TOOL_WORKSPACE_BATCH: &str = "workspace_batch";
const TOOL_WORKSPACE_GATHER_CONTEXT: &str = "workspace_gather_context";
const TOOL_WORKSPACE_WRITE_FILE: &str = "workspace_write_file";
const TOOL_WORKSPACE_EDIT_FILE: &str = "workspace_edit_file";
const TOOL_WORKSPACE_RUN_COMMAND: &str = "workspace_run_command";
const TOOL_WEB_FETCH_URL: &str = "web_fetch_url";
const TOOL_WEB_SEARCH_QUERY: &str = "web_search_query";
const TOOL_MCP_RESOURCE_READ: &str = "mcp_resource_read";
const TOOL_TOOL_SEARCH: &str = "tool_search";
/// Plan state control (PA-076 phase 7 P2-6): create/replace/merge/complete_step of a
/// session-owned plan — never executes arbitrary child calls. The handler lives in
/// [`crate::agent::plan_state::PlanControlHandler`] and is registered in
/// [`crate::agent::governed_executor::build_governed_executor`].
pub(crate) const TOOL_PLAN_CONTROL: &str = "plan_control";
/// Workspace-scoped image reading (PA-076 phase 7 P2-6): returns a reference-based
/// [`crate::agent::image_artifact::ImageArtifact`] with default `include_bytes=false`.
/// Whether the encoded bytes are sent to a model is a provider modality decision
/// (design.md Decision 11). The handler lives in
/// [`crate::agent::image_artifact::ViewImageHandler`].
pub(crate) const TOOL_VIEW_IMAGE: &str = "view_image";
/// Workspace-scoped office document conversion (Firecrawl anydoc): converts Word/PowerPoint/
/// Excel/ODF/RTF/EPUB/CSV/PDF to GitHub-Flavored Markdown, local and offline. The handler
/// lives in [`crate::agent::document_conversion::ReadDocumentHandler`].
pub(crate) const TOOL_WORKSPACE_READ_DOCUMENT: &str = "workspace_read_document";

/// W25 单一能力开关：子代理/团队运行时是否已真实接线。
///
/// wave-37 C3 落地后改为运行时判定：wired 语义 = 全局引擎槽非空（见
/// [`crate::agent::subagent_runtime::subagent_engine`]）。保留同名 `pub const`
/// 值 `true` 供外部二进制兼容引用；运行时判定请使用 [`subagent_runtime_wired`]。
pub const SUBAGENT_RUNTIME_WIRED: bool = true;

/// 运行时判定：子代理引擎槽是否已接线（引擎非空即 wired）。
pub fn subagent_runtime_wired() -> bool {
    crate::agent::subagent_runtime::subagent_engine().is_some()
}

pub(crate) const TOOL_SUBAGENT: &str = "subagent";
pub(crate) const TOOL_WORKFLOW: &str = "workflow";
pub(crate) const TOOL_SPAWN_TEAMMATE: &str = "spawn_teammate";
pub(crate) const TOOL_LIST_AGENTS: &str = "list_agents";
pub(crate) const TOOL_SEND_MESSAGE: &str = "send_message";
pub(crate) const TOOL_INTERRUPT_AGENT: &str = "interrupt_agent";
pub(crate) const TOOL_DRAIN_INBOX: &str = "drain_inbox";
pub(crate) const TOOL_TEAM_TASK_CREATE: &str = "team_task_create";
pub(crate) const TOOL_TEAM_TASK_UPDATE: &str = "team_task_update";
pub(crate) const TOOL_TEAM_TASK_LIST: &str = "team_task_list";
pub(crate) const TOOL_TODO_WRITE: &str = "todo_write";
pub(crate) const TOOL_TODO_LIST: &str = "todo_list";
/// W26：大输出落盘溢出（spill）的按引用回读工具（Deferred，经 tool_search 曝光）。
pub(crate) const TOOL_SPILL_READ: &str = "spill_read";
pub(crate) const TOOL_CREATE_GOAL: &str = "create_goal";
pub(crate) const TOOL_GET_GOAL: &str = "get_goal";
pub(crate) const TOOL_UPDATE_GOAL: &str = "update_goal";

// W37-C1：工具面扩展 21 → 28，七项新 primitive（lib 函数已在顶部 pub use re-export 区引入）。
pub(crate) const TOOL_PRESENT: &str = "present";
pub(crate) const TOOL_FUZZY_FILE_SEARCH: &str = "fuzzy_file_search";
pub(crate) const TOOL_GIT_DIFF_REMOTE: &str = "git_diff_remote";
pub(crate) const TOOL_JOB_START: &str = "job_start";
pub(crate) const TOOL_JOB_OUTPUT: &str = "job_output";
pub(crate) const TOOL_JOB_KILL: &str = "job_kill";
pub(crate) const TOOL_JOB_LIST: &str = "job_list";

// W42/PA-149：技能正文按需加载（只读；路径经 `path_permission::is_within_trusted_readonly_roots`
// 白名单判定，fail-closed）。
pub(crate) const TOOL_SKILL_LOAD: &str = "skill_load";

pub struct ToolRouter {
    workspace_root: PathBuf,
    process_manager: ProcessManager,
    sandbox_backend: Option<Arc<dyn SandboxBackend>>,
    web_resolver: Arc<dyn WebResolver>,
    /// 路径权限判定器（PA-080）：所有文件工具的路径解析统一走 `classify_path`。
    path_checker: crate::agent::path_permission::PathPermissionChecker,
    /// 显式读授权清单（PA-080）：workspace 外读取需命中授权才放行。
    authorize_store: Arc<crate::agent::path_permission::AuthorizeStore>,
    /// 会话级 root 解析器（PA-080）：由 host 注入（按 workspaceId 解析 root）；
    /// 无上下文时回退构造时的默认 root。
    root_resolver: Option<Arc<dyn Fn(&str) -> Option<std::path::PathBuf> + Send + Sync>>,
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Primitive dispatch table（ADR 0065 decision 2）
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// One primitive dispatch arm: `(router, call, execution context) -> legacy ToolResult`.
///
/// A plain `fn` pointer (no `dyn`, no allocation): the table is a compile-time inventory of the
/// legacy `ToolRouter` surface, so `execute_internal` is only a table lookup plus the two special
/// arms (`allow_batch` gate / `deferred_to_registry`).
pub type PrimitiveDispatchFn = fn(&ToolRouter, &ToolCall, &ToolExecutionContext) -> ToolResult;

/// `&'static str`（canonical primitive 名）→ handler。
///
/// 表顺序 = `builtin_tools()` 定义序（`primitive_dispatch_table_covers_every_builtin_primitive`
/// 的全覆盖遍历断言依赖该等价关系；查表本身与顺序无关）。
///
/// 不变量（ADR 0065）：表必须**恰好**覆盖 `builtin_tools()` 的 45 个 primitive，一个不多一个
/// 不少——这条不变量是失去 `match` 穷尽性检查后的替代门禁。
static PRIMITIVE_DISPATCH_TABLE: &[(&str, PrimitiveDispatchFn)] = &[
    // `time_now`：无参 primitive，签名适配为忽略 call/context。
    (TOOL_TIME_NOW, |router, _call, _context| router.time_now()),
    (TOOL_ECHO_INPUT, |router, call, _context| router.echo_input(call)),
    // `ask_user` 由 host-mediated Ask 通道（控制面）执行，legacy router 只负责 fail-closed。
    (TOOL_ASK_USER, unsupported_primitive_handler),
    (TOOL_WORKSPACE_READ_FILE, |router, call, context| {
        router.with_touched_instructions(call, context, router.read_file(call, context))
    }),
    // `workspace_read_document` 绑定 registry 内的 `ReadDocumentHandler`（governed executor）。
    (TOOL_WORKSPACE_READ_DOCUMENT, unsupported_primitive_handler),
    (TOOL_WORKSPACE_READ_FILE_SEGMENT, |router, call, context| {
        router.read_file_segment(call, context)
    }),
    (TOOL_WORKSPACE_LIST_FILES, |router, call, context| {
        router.list_files(call, context)
    }),
    (TOOL_WORKSPACE_PATH_INFO, |router, call, context| {
        router.path_info(call, context)
    }),
    (TOOL_WORKSPACE_SEARCH_TEXT, |router, call, context| {
        router.search_text(call, context)
    }),
    (TOOL_WORKSPACE_GLOB_FILES, |router, call, context| {
        router.glob_files(call, context)
    }),
    (TOOL_WEB_FETCH_URL, |router, call, _context| router.web_fetch(call)),
    (TOOL_WEB_SEARCH_QUERY, |router, call, _context| {
        router.web_search(call)
    }),
    (TOOL_MCP_RESOURCE_READ, mcp_resource_read_deferred_handler),
    (TOOL_TOOL_SEARCH, tool_search_deferred_handler),
    (TOOL_WORKSPACE_WRITE_FILE, |router, call, context| {
        router.with_touched_instructions(call, context, router.write_file(call, context))
    }),
    (TOOL_WORKSPACE_EDIT_FILE, |router, call, context| {
        router.with_touched_instructions(call, context, router.edit_file(call, context))
    }),
    (TOOL_WORKSPACE_RUN_COMMAND, |router, call, context| {
        router.run_command(call, context)
    }),
    (TOOL_WORKSPACE_BATCH, |router, call, context| {
        router.batch(call, context)
    }),
    (TOOL_WORKSPACE_GATHER_CONTEXT, |router, call, context| {
        router.gather_context(call, context)
    }),
    // `plan_control` 绑定 registry 内的 `PlanControlHandler`（governed executor）。
    (TOOL_PLAN_CONTROL, unsupported_primitive_handler),
    // `view_image` 绑定 registry 内的 `ViewImageHandler`（governed executor）。
    (TOOL_VIEW_IMAGE, unsupported_primitive_handler),
    (TOOL_SUBAGENT, |router, call, context| {
        router.execute_subagent(call, context)
    }),
    (TOOL_WORKFLOW, |router, call, _context| {
        router.execute_workflow(call)
    }),
    (TOOL_SPAWN_TEAMMATE, |router, call, context| {
        router.execute_spawn_teammate(call, context)
    }),
    (TOOL_LIST_AGENTS, |router, call, _context| {
        router.execute_list_agents(call)
    }),
    (TOOL_SEND_MESSAGE, |router, call, context| {
        router.execute_send_message(call, context)
    }),
    (TOOL_INTERRUPT_AGENT, |router, call, _context| {
        router.execute_interrupt_agent(call)
    }),
    (TOOL_DRAIN_INBOX, |router, call, context| {
        router.execute_drain_inbox(call, context)
    }),
    (TOOL_TEAM_TASK_CREATE, |router, call, _context| {
        router.execute_team_task_create(call)
    }),
    (TOOL_TEAM_TASK_UPDATE, |router, call, _context| {
        router.execute_team_task_update(call)
    }),
    (TOOL_TEAM_TASK_LIST, |router, call, _context| {
        router.execute_team_task_list(call)
    }),
    (TOOL_TODO_WRITE, |router, call, context| {
        router.execute_todo_write(call, context)
    }),
    (TOOL_TODO_LIST, |router, call, context| {
        router.execute_todo_list(call, context)
    }),
    (TOOL_SPILL_READ, |router, call, _context| {
        router.execute_spill_read(call)
    }),
    (TOOL_CREATE_GOAL, |router, call, context| {
        router.execute_create_goal(call, context)
    }),
    (TOOL_GET_GOAL, |router, call, context| {
        router.execute_get_goal(call, context)
    }),
    (TOOL_UPDATE_GOAL, |router, call, context| {
        router.execute_update_goal(call, context)
    }),
    (TOOL_PRESENT, |router, call, context| {
        router.execute_present(call, context)
    }),
    (TOOL_FUZZY_FILE_SEARCH, |router, call, context| {
        router.execute_find(call, context)
    }),
    (TOOL_GIT_DIFF_REMOTE, |router, call, context| {
        router.execute_git_diff(call, context)
    }),
    (TOOL_JOB_START, |router, call, context| {
        router.execute_job_start(call, context)
    }),
    (TOOL_JOB_OUTPUT, |router, call, context| {
        router.execute_job_output(call, context)
    }),
    (TOOL_JOB_KILL, |router, call, context| {
        router.execute_job_kill(call, context)
    }),
    (TOOL_JOB_LIST, |router, _call, context| {
        router.execute_job_list(context)
    }),
    (TOOL_SKILL_LOAD, |router, call, context| {
        router.execute_skill_load(call, context)
    }),
];

/// Every canonical primitive the dispatch table covers, in table order. Feeds the coverage
/// assertion that replaces the compiler's exhaustive `match` check.
pub fn primitive_dispatch_table() -> Vec<&'static str> {
    PRIMITIVE_DISPATCH_TABLE
        .iter()
        .map(|(primitive, _)| *primitive)
        .collect()
}

/// Resolve one canonical primitive to its dispatch arm. Unknown primitives have no arm.
fn primitive_dispatch_fn(primitive: &str) -> Option<PrimitiveDispatchFn> {
    PRIMITIVE_DISPATCH_TABLE
        .iter()
        .find(|(name, _)| *name == primitive)
        .map(|(_, handler)| *handler)
}

/// Fallback arm shared by primitives whose execution lives in the governed dispatcher (a dedicated
/// registry handler) instead of the legacy `ToolRouter`. Byte-identical to the legacy
/// `unsupported_tool` outcome, so no observable behaviour changes.
fn unsupported_primitive_handler(
    _router: &ToolRouter,
    call: &ToolCall,
    _context: &ToolExecutionContext,
) -> ToolResult {
    unsupported_tool_result(call)
}

/// `deferred_to_registry` arm (ADR 0065): MCPResource is executed through the capability/registry
/// proxy, never by the legacy router.
fn mcp_resource_read_deferred_handler(
    _router: &ToolRouter,
    _call: &ToolCall,
    _context: &ToolExecutionContext,
) -> ToolResult {
    error_result(
        TOOL_MCP_RESOURCE_READ,
        "deferred_to_registry",
        "MCPResource 由 capability registry 代理执行。".to_string(),
        Some("请通过 runtime 注册工具执行入口调用该工具。".to_string()),
    )
}

/// `deferred_to_registry` arm (ADR 0065): ToolSearch is executed through the capability/registry
/// proxy, never by the legacy router.
fn tool_search_deferred_handler(
    _router: &ToolRouter,
    _call: &ToolCall,
    _context: &ToolExecutionContext,
) -> ToolResult {
    error_result(
        TOOL_TOOL_SEARCH,
        "deferred_to_registry",
        "ToolSearch 由 capability registry 代理执行。".to_string(),
        Some("请通过 runtime 注册工具执行入口调用该工具。".to_string()),
    )
}

fn unsupported_tool_result(call: &ToolCall) -> ToolResult {
    error_result(
        &call.name,
        "unsupported_tool",
        format!("当前 runtime 尚未实现工具 `{}`。", call.name),
        Some("请改用 list_available_tools 中返回的工具名。".to_string()),
    )
}

impl ToolRouter {
    pub fn new() -> Self {
        Self {
            // 默认工作区分叉收敛：compute_default_workspace_root()
            //（Windows Documents~/pony_agent，Unix ~/pony_agent）优先；解析失败 → seam
            // 输出与 control_plane 对齐的瞬时兜底告警并以进程 cwd 兜底（W2/R2-4）。
            workspace_root: crate::agent::workspace::default_root_or_cwd_with_warning(
                crate::agent::workspace::compute_default_workspace_root(),
                || std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            ),
            process_manager: ProcessManager::new(),
            sandbox_backend: None,
            web_resolver: Arc::new(FailClosedResolver),
            path_checker: crate::agent::path_permission::PathPermissionChecker::with_default(),
            authorize_store: Arc::new(crate::agent::path_permission::AuthorizeStore::new()),
            root_resolver: None,
        }
    }

    pub fn with_workspace_root(workspace_root: PathBuf) -> Self {
        Self {
            workspace_root,
            process_manager: ProcessManager::new(),
            sandbox_backend: None,
            web_resolver: Arc::new(FailClosedResolver),
            path_checker: crate::agent::path_permission::PathPermissionChecker::with_default(),
            authorize_store: Arc::new(crate::agent::path_permission::AuthorizeStore::new()),
            root_resolver: None,
        }
    }

    /// 注入共享授权清单（PA-080）：host control plane 与工具执行器共享同一 `Arc`，
    /// `authorize_path` / `revoke_authorization` 的变更即时对工具判定生效。

    pub fn with_authorize_store(
        mut self,
        authorize_store: Arc<crate::agent::path_permission::AuthorizeStore>,
    ) -> Self {
        self.authorize_store = authorize_store;
        self
    }

    /// 注入会话级 root 解析器（PA-080）：`Fn(workspaceId) -> Option<root>`，
    /// 由 host 在构造时注入（按注册表解析）；无上下文（legacy 会话、直接调用、回归套件）
    /// 回退构造时的默认 root。

    pub fn with_root_resolver<F>(mut self, resolver: F) -> Self
    where
        F: Fn(&str) -> Option<std::path::PathBuf> + Send + Sync + 'static,
    {
        self.root_resolver = Some(Arc::new(resolver));
        self
    }

    /// 当前生效的授权清单（PA-080）。

    pub fn authorize_store(&self) -> &Arc<crate::agent::path_permission::AuthorizeStore> {
        &self.authorize_store
    }

    /// Register an explicit sandbox backend for the legacy `workspace_run_command` path.
    ///
    /// When a backend is registered the legacy handler runs `enforce_sandbox` against it and
    /// fails closed on denial. When `None` (the default) the legacy handler still fails closed:
    /// it treats the request as `NoSandboxBackend` (Unavailable), so an autonomous Run is
    /// `sandbox_denied` — never a silent downgrade to a full-parent-environment shell
    /// (design Decision 7, phase-5 review P1-2). See the module documentation.

    pub fn with_sandbox_backend<B: SandboxBackend + 'static>(mut self, backend: B) -> Self {
        self.sandbox_backend = Some(Arc::new(backend));
        self
    }

    /// Inject a `WebResolver` for the legacy `web_fetch_url` path (design Decision 8).
    ///
    /// The default [`FailClosedResolver`] keeps every hostname URL fail-closed until a production
    /// resolver is wired in; this builder is the injection point that a governed/production
    /// resolver uses. When injected, hostname URLs are resolved through it, validated, and every
    /// connection is pinned to the validated addresses — there is no "validate then let the
    /// default client re-resolve" weak mode.

    pub fn with_web_resolver<R: WebResolver + 'static>(mut self, resolver: R) -> Self {
        self.web_resolver = Arc::new(resolver);
        self
    }

    pub fn execute(&self, call: &ToolCall) -> ToolResult {
        self.execute_with_workspace_root(call, None)
    }

    /// 会话级 root 接线（PA-080 修复，code-review P1-2 / consultant 方案 A）：工具执行时携带
    /// 会话 workspace root，权限判定锚定在会话 workspace 而非进程 cwd。`None` 时回退构造时的
    /// 默认 root（legacy 直接调用 / 无上下文路径）。
    ///
    /// 实现采用**显式不可变调用上下文**（`ToolExecutionContext`）贯穿整条工具方法链，
    /// 不使用共享可变状态：并发多会话工具执行（同一 `Arc<ToolRouter>`）不会串扰，
    /// `workspace_batch` 并行子调用克隆同一上下文，panic 也不会残留状态。

    pub fn execute_with_workspace_root(
        &self,
        call: &ToolCall,
        workspace_root: Option<&Path>,
    ) -> ToolResult {
        let started_at = Instant::now();
        let context = ToolExecutionContext {
            workspace_root: workspace_root.map(|path| path.to_path_buf()),
            ..Default::default()
        };
        let mut result = self.execute_internal(call, true, &context);
        result.duration_ms = started_at.elapsed().as_millis() as u64;
        result
    }

    pub fn execute_with_context(
        &self,
        call: &ToolCall,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        let started_at = Instant::now();
        let mut result = self.execute_internal(call, true, context);
        result.duration_ms = started_at.elapsed().as_millis() as u64;
        result
    }

    fn execute_internal(
        &self,
        call: &ToolCall,
        allow_batch: bool,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        // ADR 0065 decision 2：名称 → 分派臂改为 `&'static str → fn` 表查（零 `dyn`）。
        // 仅保留两条特殊臂：`allow_batch` 递归闸门与 `deferred_to_registry`（registry 代理执行）。
        let Some(primitive) = canonical_tool_name(&call.name) else {
            return unsupported_tool_result(call);
        };
        match primitive {
            TOOL_WORKSPACE_BATCH if allow_batch => return self.batch(call, context),
            TOOL_WORKSPACE_BATCH => {
                return error_result(
                    TOOL_WORKSPACE_BATCH,
                    "nested_batch_not_allowed",
                    "workspace_batch 不允许递归调用 workspace_batch。".to_string(),
                    Some("请把嵌套批量调用拆成多个叶子工具调用。".to_string()),
                )
            }
            TOOL_MCP_RESOURCE_READ => {
                return mcp_resource_read_deferred_handler(self, call, context)
            }
            TOOL_TOOL_SEARCH => return tool_search_deferred_handler(self, call, context),
            _ => {}
        }
        match primitive_dispatch_fn(primitive) {
            Some(handler) => handler(self, call, context),
            // 表缺臂即 fail-closed：名称已知但没有分派实现，绝不静默放行。
            None => unsupported_tool_result(call),
        }
    }
}

impl ToolExecutor for ToolRouter {
    fn execute(&self, call: &ToolCall) -> ToolResult {
        ToolRouter::execute(self, call)
    }

    fn execute_with_context(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        ToolRouter::execute_with_context(self, call, context)
    }
}
