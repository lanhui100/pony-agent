//! 工具名映射与权限/暴露面判定：canonical/model-visible/product 名称、别名、
//! 执行策略与默认权限事实。
use super::*;
use super::run::DEFAULT_RUN_TIMEOUT_MS;
use super::workspace::MAX_FULL_READ_BYTES;

pub fn canonical_tool_name(name: &str) -> Option<&'static str> {
    match name {
        "Run" => Some(TOOL_WORKSPACE_RUN_COMMAND),
        "TodoWrite" => Some(TOOL_TODO_WRITE),
        "TodoList" => Some(TOOL_TODO_LIST),
        "SpillRead" => Some(TOOL_SPILL_READ),
        // PA-114: the product name `Ask` is now owned by the first-class `ask_user` primitive.
        // `echo_input` keeps resolving to itself below (legacy compat, B2-1).
        "Ask" => Some(TOOL_ASK_USER),
        "List" => Some(TOOL_WORKSPACE_LIST_FILES),
        "Read" => Some(TOOL_WORKSPACE_GATHER_CONTEXT),
        "Search" => Some(TOOL_WORKSPACE_SEARCH_TEXT),
        "Glob" => Some(TOOL_WORKSPACE_GLOB_FILES),
        "WebFetch" => Some(TOOL_WEB_FETCH_URL),
        "WebSearch" => Some(TOOL_WEB_SEARCH_QUERY),
        "MCPResource" => Some(TOOL_MCP_RESOURCE_READ),
        "ToolSearch" => Some(TOOL_TOOL_SEARCH),
        "Write" => Some(TOOL_WORKSPACE_WRITE_FILE),
        "Edit" => Some(TOOL_WORKSPACE_EDIT_FILE),
        "BatchExecute" => Some(TOOL_WORKSPACE_BATCH),
        "Plan" => Some(TOOL_PLAN_CONTROL),
        "ViewImage" => Some(TOOL_VIEW_IMAGE),
        TOOL_TIME_NOW | "time.now" => Some(TOOL_TIME_NOW),
        TOOL_ECHO_INPUT | "echo.input" => Some(TOOL_ECHO_INPUT),
        TOOL_ASK_USER | "ask.user" => Some(TOOL_ASK_USER),
        TOOL_WORKSPACE_LIST_FILES | "workspace.list_files" => Some(TOOL_WORKSPACE_LIST_FILES),
        TOOL_WORKSPACE_READ_FILE | "workspace.read_file" => Some(TOOL_WORKSPACE_READ_FILE),
        TOOL_WORKSPACE_READ_FILE_SEGMENT | "workspace.read_file_segment" => {
            Some(TOOL_WORKSPACE_READ_FILE_SEGMENT)
        }
        TOOL_WORKSPACE_PATH_INFO | "workspace.path_info" => Some(TOOL_WORKSPACE_PATH_INFO),
        TOOL_WORKSPACE_SEARCH_TEXT | "workspace.search_text" => Some(TOOL_WORKSPACE_SEARCH_TEXT),
        TOOL_WORKSPACE_GLOB_FILES | "workspace.glob_files" => Some(TOOL_WORKSPACE_GLOB_FILES),
        TOOL_WEB_FETCH_URL | "web.fetch_url" => Some(TOOL_WEB_FETCH_URL),
        TOOL_WEB_SEARCH_QUERY | "web.search_query" => Some(TOOL_WEB_SEARCH_QUERY),
        TOOL_MCP_RESOURCE_READ | "mcp.resource_read" => Some(TOOL_MCP_RESOURCE_READ),
        TOOL_TOOL_SEARCH | "tool.search" => Some(TOOL_TOOL_SEARCH),
        TOOL_WORKSPACE_WRITE_FILE | "workspace.write_file" => Some(TOOL_WORKSPACE_WRITE_FILE),
        TOOL_WORKSPACE_EDIT_FILE | "workspace.edit_file" => Some(TOOL_WORKSPACE_EDIT_FILE),
        TOOL_WORKSPACE_RUN_COMMAND | "workspace.run_command" => Some(TOOL_WORKSPACE_RUN_COMMAND),
        TOOL_WORKSPACE_BATCH | "workspace.batch" => Some(TOOL_WORKSPACE_BATCH),
        TOOL_WORKSPACE_GATHER_CONTEXT | "workspace.gather_context" => {
            Some(TOOL_WORKSPACE_GATHER_CONTEXT)
        }
        TOOL_PLAN_CONTROL | "plan.control" => Some(TOOL_PLAN_CONTROL),
        TOOL_VIEW_IMAGE | "view.image" => Some(TOOL_VIEW_IMAGE),
        TOOL_WORKSPACE_READ_DOCUMENT | "workspace.read_document" => {
            Some(TOOL_WORKSPACE_READ_DOCUMENT)
        }
        TOOL_SUBAGENT => Some(TOOL_SUBAGENT),
        TOOL_WORKFLOW => Some(TOOL_WORKFLOW),
        TOOL_SPAWN_TEAMMATE => Some(TOOL_SPAWN_TEAMMATE),
        TOOL_LIST_AGENTS => Some(TOOL_LIST_AGENTS),
        TOOL_SEND_MESSAGE => Some(TOOL_SEND_MESSAGE),
        TOOL_INTERRUPT_AGENT => Some(TOOL_INTERRUPT_AGENT),
        TOOL_DRAIN_INBOX => Some(TOOL_DRAIN_INBOX),
        TOOL_TEAM_TASK_CREATE => Some(TOOL_TEAM_TASK_CREATE),
        TOOL_TEAM_TASK_UPDATE => Some(TOOL_TEAM_TASK_UPDATE),
        TOOL_TEAM_TASK_LIST => Some(TOOL_TEAM_TASK_LIST),
        TOOL_TODO_WRITE => Some(TOOL_TODO_WRITE),
        TOOL_TODO_LIST => Some(TOOL_TODO_LIST),
        TOOL_SPILL_READ => Some(TOOL_SPILL_READ),
        TOOL_CREATE_GOAL | "create.goal" => Some(TOOL_CREATE_GOAL),
        TOOL_GET_GOAL | "get.goal" => Some(TOOL_GET_GOAL),
        TOOL_UPDATE_GOAL | "update.goal" => Some(TOOL_UPDATE_GOAL),
        // W37-C1：产品名 → primitive 名映射（七项新工具，产品名即工具名）。
        "Present" => Some(TOOL_PRESENT),
        "Find" => Some(TOOL_FUZZY_FILE_SEARCH),
        "GitDiff" => Some(TOOL_GIT_DIFF_REMOTE),
        "JobStart" => Some(TOOL_JOB_START),
        "JobOutput" => Some(TOOL_JOB_OUTPUT),
        "JobKill" => Some(TOOL_JOB_KILL),
        "JobList" => Some(TOOL_JOB_LIST),
        TOOL_PRESENT | "present.tool" => Some(TOOL_PRESENT),
        TOOL_FUZZY_FILE_SEARCH | "fuzzy.file_search" => Some(TOOL_FUZZY_FILE_SEARCH),
        TOOL_GIT_DIFF_REMOTE | "git.diff_remote" => Some(TOOL_GIT_DIFF_REMOTE),
        TOOL_JOB_START | "job.start" => Some(TOOL_JOB_START),
        TOOL_JOB_OUTPUT | "job.output" => Some(TOOL_JOB_OUTPUT),
        TOOL_JOB_KILL | "job.kill" => Some(TOOL_JOB_KILL),
        TOOL_JOB_LIST | "job.list" => Some(TOOL_JOB_LIST),
        // W42/PA-149：技能正文加载（产品名即工具名 `skill_load`）。
        TOOL_SKILL_LOAD | "skill.load" => Some(TOOL_SKILL_LOAD),
        // B150/P1：外部 ACP agent 委派（产品名即工具名 `acp_delegate`；驼峰旧拼写归一）。
        TOOL_ACP_DELEGATE | "AcpDelegate" | "acp.delegate" => Some(TOOL_ACP_DELEGATE),
        _ => None,
    }
}

pub(super) fn builtin_aliases_for_primitive(primitive: &str) -> Vec<&'static str> {
    match primitive {
        TOOL_TIME_NOW => vec![TOOL_TIME_NOW, "time.now"],
        TOOL_ECHO_INPUT => vec![TOOL_ECHO_INPUT, "echo.input"],
        // PA-114: `Ask` is included explicitly so legacy/partner integrations that know the
        // product spelling keep resolving even before the registry attaches the model name.
        TOOL_ASK_USER => vec![TOOL_ASK_USER, "ask.user", "Ask"],
        TOOL_WORKSPACE_LIST_FILES => vec![TOOL_WORKSPACE_LIST_FILES, "workspace.list_files"],
        TOOL_WORKSPACE_READ_FILE => vec![TOOL_WORKSPACE_READ_FILE, "workspace.read_file"],
        TOOL_WORKSPACE_READ_FILE_SEGMENT => {
            vec![
                TOOL_WORKSPACE_READ_FILE_SEGMENT,
                "workspace.read_file_segment",
            ]
        }
        TOOL_WORKSPACE_PATH_INFO => vec![TOOL_WORKSPACE_PATH_INFO, "workspace.path_info"],
        TOOL_WORKSPACE_SEARCH_TEXT => vec![TOOL_WORKSPACE_SEARCH_TEXT, "workspace.search_text"],
        TOOL_WORKSPACE_GLOB_FILES => vec![TOOL_WORKSPACE_GLOB_FILES, "workspace.glob_files"],
        TOOL_WEB_FETCH_URL => vec![TOOL_WEB_FETCH_URL, "web.fetch_url"],
        TOOL_WEB_SEARCH_QUERY => vec![TOOL_WEB_SEARCH_QUERY, "web.search_query"],
        TOOL_MCP_RESOURCE_READ => vec![TOOL_MCP_RESOURCE_READ, "mcp.resource_read"],
        TOOL_TOOL_SEARCH => vec![TOOL_TOOL_SEARCH, "tool.search"],
        TOOL_WORKSPACE_WRITE_FILE => vec![TOOL_WORKSPACE_WRITE_FILE, "workspace.write_file"],
        TOOL_WORKSPACE_EDIT_FILE => vec![TOOL_WORKSPACE_EDIT_FILE, "workspace.edit_file"],
        TOOL_WORKSPACE_RUN_COMMAND => vec![TOOL_WORKSPACE_RUN_COMMAND, "workspace.run_command"],
        TOOL_WORKSPACE_BATCH => vec![TOOL_WORKSPACE_BATCH, "workspace.batch"],
        TOOL_WORKSPACE_GATHER_CONTEXT => {
            vec![TOOL_WORKSPACE_GATHER_CONTEXT, "workspace.gather_context"]
        }
        TOOL_PLAN_CONTROL => vec![TOOL_PLAN_CONTROL, "plan.control"],
        TOOL_VIEW_IMAGE => vec![TOOL_VIEW_IMAGE, "view.image"],
        TOOL_WORKSPACE_READ_DOCUMENT => {
            vec![TOOL_WORKSPACE_READ_DOCUMENT, "workspace.read_document"]
        }
        TOOL_SUBAGENT => vec![TOOL_SUBAGENT],
        TOOL_WORKFLOW => vec![TOOL_WORKFLOW],
        TOOL_SPAWN_TEAMMATE => vec![TOOL_SPAWN_TEAMMATE],
        TOOL_LIST_AGENTS => vec![TOOL_LIST_AGENTS],
        TOOL_SEND_MESSAGE => vec![TOOL_SEND_MESSAGE],
        TOOL_INTERRUPT_AGENT => vec![TOOL_INTERRUPT_AGENT],
        TOOL_DRAIN_INBOX => vec![TOOL_DRAIN_INBOX],
        TOOL_TEAM_TASK_CREATE => vec![TOOL_TEAM_TASK_CREATE],
        TOOL_TEAM_TASK_UPDATE => vec![TOOL_TEAM_TASK_UPDATE],
        TOOL_TEAM_TASK_LIST => vec![TOOL_TEAM_TASK_LIST],
        TOOL_TODO_WRITE => vec![TOOL_TODO_WRITE],
        TOOL_TODO_LIST => vec![TOOL_TODO_LIST],
        TOOL_SPILL_READ => vec![TOOL_SPILL_READ],
        TOOL_PRESENT => vec![TOOL_PRESENT, "present.tool"],
        TOOL_FUZZY_FILE_SEARCH => vec![TOOL_FUZZY_FILE_SEARCH, "fuzzy.file_search"],
        TOOL_GIT_DIFF_REMOTE => vec![TOOL_GIT_DIFF_REMOTE, "git.diff_remote"],
        TOOL_JOB_START => vec![TOOL_JOB_START, "job.start"],
        TOOL_JOB_OUTPUT => vec![TOOL_JOB_OUTPUT, "job.output"],
        TOOL_JOB_KILL => vec![TOOL_JOB_KILL, "job.kill"],
        TOOL_JOB_LIST => vec![TOOL_JOB_LIST, "job.list"],
        TOOL_SKILL_LOAD => vec![TOOL_SKILL_LOAD, "skill.load"],
        // B150/P1：外部 ACP agent 委派（点号别名与产品名同源）。
        TOOL_ACP_DELEGATE => vec![TOOL_ACP_DELEGATE, "acp.delegate"],
        _ => Vec::new(),
    }
}

pub(super) fn execution_policy_for_primitive(primitive: &str) -> ToolExecutionPolicy {
    let mut policy = ToolExecutionPolicy::default();
    match primitive {
        TOOL_WORKSPACE_LIST_FILES
        | TOOL_WORKSPACE_READ_FILE
        | TOOL_WORKSPACE_READ_FILE_SEGMENT
        | TOOL_WORKSPACE_PATH_INFO
        | TOOL_WORKSPACE_SEARCH_TEXT
        | TOOL_WORKSPACE_GLOB_FILES
        | TOOL_WEB_FETCH_URL
        | TOOL_WEB_SEARCH_QUERY
        | TOOL_MCP_RESOURCE_READ
        | TOOL_TOOL_SEARCH
        | TOOL_WORKSPACE_GATHER_CONTEXT => policy.concurrent_safe = true,
        TOOL_SPILL_READ => {
            policy.concurrent_safe = true;
            // 单次回读上限与 spill::MAX_READ_LIMIT 对齐，避免结果预算把切片二次截断。
            policy.result_budget_bytes = crate::agent::spill::MAX_READ_LIMIT;
        }
        TOOL_VIEW_IMAGE => {
            policy.concurrent_safe = true;
            // Image artifact with include_bytes=true can be up to the default 2 MiB byte cap, so
            // the result budget should not truncate a valid bounded payload (design Decision 11:
            // reference-based by default, but caller-adjustable).
            policy.result_budget_bytes = 2 * 1024 * 1024;
        }
        TOOL_WORKSPACE_READ_DOCUMENT => {
            policy.concurrent_safe = true;
            // Converted Markdown can reach the default 512 KiB output cap, so the result budget
            // must not truncate a valid bounded payload.
            policy.result_budget_bytes =
                crate::agent::document_conversion::DEFAULT_MAX_OUTPUT_BYTES as usize;
        }
        TOOL_SKILL_LOAD => {
            policy.concurrent_safe = true;
            // 技能正文本身有 MAX_SKILL_BODY_BYTES 硬上限；结果预算留出标题与分隔余量，
            // 避免一次合法加载被结果预算二次截断。
            policy.result_budget_bytes = crate::agent::skill_body::MAX_SKILL_BODY_BYTES + 8 * 1024;
        }
        TOOL_WORKSPACE_RUN_COMMAND => {
            policy.default_timeout_ms = DEFAULT_RUN_TIMEOUT_MS;
            policy.result_budget_bytes = MAX_FULL_READ_BYTES as usize;
        }
        TOOL_SUBAGENT | TOOL_WORKFLOW => {
            policy.concurrent_safe = true;
            policy.default_timeout_ms = 120_000;
        }
        // B151/P2（ADVISORY#5）：ACP 委派的默认预算只认 `acp` 层单一真源
        // （`ACP_DEFAULT_TURN_BUDGET`），CLI / 工具入口 / 执行策略三处同源。
        TOOL_ACP_DELEGATE => {
            policy.concurrent_safe = true;
            policy.default_timeout_ms = crate::acp::ACP_DEFAULT_TURN_BUDGET.as_millis() as u64;
        }
        TOOL_SPAWN_TEAMMATE
        | TOOL_LIST_AGENTS
        | TOOL_SEND_MESSAGE
        | TOOL_INTERRUPT_AGENT
        | TOOL_DRAIN_INBOX
        | TOOL_TEAM_TASK_CREATE
        | TOOL_TEAM_TASK_UPDATE
        | TOOL_TEAM_TASK_LIST => {
            policy.concurrent_safe = true;
        }
        _ => {}
    }
    policy
}

/// L3-B F5：只读专用权限面。这些 fact 只授予读语义，**绝不**映射到 write/execute，
/// 因此权限评估 fail-closed（不会因 `skill_load` 读取 workspace 外全局技能根而放行写/执行）。
pub(super) const READ_ONLY_PERMISSION_SCOPES: [&str; 2] = ["workspace.read", "skills.read"];

/// 技能正文读取的专用只读 fact（L3-B F5）：读取面包含 workspace 外的全局技能根，
/// 所以不能谎报为 `workspace.read`；同时该 fact 在 `default_permission_declaration_for_name`
/// 中只映射到 `ToolPermissionScope::WorkspaceRead`（只读语义）。
pub(super) const SKILLS_READ_PERMISSION_SCOPE: &str = "skills.read";

pub(super) fn default_permission_declaration_for_name(name: &str) -> ToolPermissionDeclaration {
    let facts = default_permission_facts_for_name(name);
    let mut scopes = std::collections::BTreeSet::new();
    if let Some(scope) = facts.permission_scope.as_deref() {
        let typed_scope = match scope {
            // 只读白名单（含 skills.read）→ 统一映射到唯一的只读 typed scope。
            scope if READ_ONLY_PERMISSION_SCOPES.contains(&scope) => {
                Some(ToolPermissionScope::WorkspaceRead)
            }
            "workspace.write" => Some(ToolPermissionScope::WorkspaceWrite),
            "workspace.execute" => Some(ToolPermissionScope::WorkspaceExecute),
            "capability.discovery" => Some(ToolPermissionScope::CapabilityDiscovery),
            _ => None,
        };
        if let Some(scope) = typed_scope {
            scopes.insert(scope);
        }
    }
    ToolPermissionDeclaration {
        scopes,
        requires_approval: facts.requires_approval.unwrap_or(true),
        host_mediated: facts.host_mediated.unwrap_or(true),
        approval_mode: facts.approval_mode,
    }
}

/// Maps a known builtin primitive to its product-visible name. Unknown names have no builtin
/// product identity, so callers must keep the caller-supplied name instead of inventing one.
pub fn model_visible_tool_name_opt(name: &str) -> Option<&'static str> {
    let mapped = match canonical_tool_name(name).unwrap_or(name) {
        TOOL_WORKSPACE_RUN_COMMAND => "Run",
        TOOL_TIME_NOW => "Run",
        TOOL_ECHO_INPUT => "Ask",
        TOOL_ASK_USER => "Ask",
        TOOL_WORKSPACE_LIST_FILES => "List",
        TOOL_WORKSPACE_READ_FILE | TOOL_WORKSPACE_READ_FILE_SEGMENT => "Read",
        TOOL_WORKSPACE_PATH_INFO => "List",
        TOOL_WORKSPACE_SEARCH_TEXT => "Search",
        TOOL_WORKSPACE_GLOB_FILES => "Glob",
        TOOL_WEB_FETCH_URL => "WebFetch",
        TOOL_WEB_SEARCH_QUERY => "WebSearch",
        TOOL_MCP_RESOURCE_READ => "MCPResource",
        TOOL_TOOL_SEARCH => "ToolSearch",
        TOOL_WORKSPACE_WRITE_FILE => "Write",
        TOOL_WORKSPACE_EDIT_FILE => "Edit",
        TOOL_WORKSPACE_BATCH => "BatchExecute",
        TOOL_WORKSPACE_GATHER_CONTEXT => "Read",
        TOOL_PLAN_CONTROL => "Plan",
        TOOL_VIEW_IMAGE => "ViewImage",
        TOOL_WORKSPACE_READ_DOCUMENT => "ReadDocument",
        // W26：spill 回读工具的产品名（Deferred，经 tool_search 提升后出现）。
        TOOL_SPILL_READ => "SpillRead",
        TOOL_TODO_WRITE => "TodoWrite",
        TOOL_TODO_LIST => "TodoList",
        // W37-C1：七项新工具的产品名（与 builtin_tools 注册的模型可见面一一对应）。
        TOOL_PRESENT => "Present",
        TOOL_FUZZY_FILE_SEARCH => "Find",
        TOOL_GIT_DIFF_REMOTE => "GitDiff",
        TOOL_JOB_START => "JobStart",
        TOOL_JOB_OUTPUT => "JobOutput",
        TOOL_JOB_KILL => "JobKill",
        TOOL_JOB_LIST => "JobList",
        // W42/PA-149：`skill_load` 的产品名保持小写原样（契约面按此名断言，见 C1-5）。
        TOOL_SKILL_LOAD => TOOL_SKILL_LOAD,
        // B150/P1：`acp_delegate` 同样小写原名透传（与 subagent 同构，避免 telemetry 记成 Run）。
        TOOL_ACP_DELEGATE => TOOL_ACP_DELEGATE,
        // Wave-61 C1.6：10 项团队/子代理工具同样「小写原名透传」（与 TOOL_SKILL_LOAD 同构）。
        // 不登记会让 `model_visible_tool_name()` 回落字面量 "Run"，把 telemetry 的 tool activity
        //（telemetry.rs:306/332/508/576）记成 `Run`，污染 trace 取证面；也避免 descriptor 的
        // canonical_name 被误标成 "Run"。注意：**不新增 CamelCase 别名**（契约 C1.1，别名表不变）。
        TOOL_SUBAGENT => TOOL_SUBAGENT,
        TOOL_WORKFLOW => TOOL_WORKFLOW,
        TOOL_SPAWN_TEAMMATE => TOOL_SPAWN_TEAMMATE,
        TOOL_LIST_AGENTS => TOOL_LIST_AGENTS,
        TOOL_SEND_MESSAGE => TOOL_SEND_MESSAGE,
        TOOL_INTERRUPT_AGENT => TOOL_INTERRUPT_AGENT,
        TOOL_DRAIN_INBOX => TOOL_DRAIN_INBOX,
        TOOL_TEAM_TASK_CREATE => TOOL_TEAM_TASK_CREATE,
        TOOL_TEAM_TASK_UPDATE => TOOL_TEAM_TASK_UPDATE,
        TOOL_TEAM_TASK_LIST => TOOL_TEAM_TASK_LIST,
        // W20 goal 工具族同样「小写原名透传」：`product_visible_tool_name()` 本就把未知名原样
        // 返回，但 telemetry 记录点（telemetry.rs:314/340/516/584）走的是
        // `model_visible_tool_name()`，不登记会回落字面量 "Run"——trace 里目标工具被记成 `Run`，
        // 工具消息行再经 `tool_display_metadata_for_name` 反查成「运行」，双重失真。
        TOOL_CREATE_GOAL => TOOL_CREATE_GOAL,
        TOOL_GET_GOAL => TOOL_GET_GOAL,
        TOOL_UPDATE_GOAL => TOOL_UPDATE_GOAL,
        _ => return None,
    };
    Some(mapped)
}

pub fn model_visible_tool_name(name: &str) -> &'static str {
    // Preserved for builtin call sites that already hold a known primitive. Unknown names keep
    // the legacy `Run` fallback only where a `&'static str` is required; prefer the `_opt` form.
    model_visible_tool_name_opt(name).unwrap_or("Run")
}

/// Product-visible name for contract views. Unknown (external/dynamic) tool names pass through
/// unchanged so no external tool is ever mislabeled as a builtin product tool.
pub fn product_visible_tool_name(name: &str) -> String {
    model_visible_tool_name_opt(name)
        .map(str::to_string)
        .unwrap_or_else(|| name.to_string())
}

pub fn product_canonical_tool_name(name: &str) -> &'static str {
    model_visible_tool_name(name)
}

pub fn tool_kind_for_name(name: &str) -> ToolKind {
    match canonical_tool_name(name).unwrap_or(name) {
        TOOL_WORKSPACE_READ_FILE
        | TOOL_WORKSPACE_READ_FILE_SEGMENT
        | TOOL_WORKSPACE_PATH_INFO
        | TOOL_WORKSPACE_GATHER_CONTEXT
        | TOOL_WORKSPACE_GLOB_FILES
        | TOOL_WEB_FETCH_URL
        | TOOL_MCP_RESOURCE_READ => ToolKind::Read,
        TOOL_WORKSPACE_SEARCH_TEXT | TOOL_WEB_SEARCH_QUERY | TOOL_TOOL_SEARCH => ToolKind::Search,
        TOOL_WORKSPACE_WRITE_FILE | TOOL_WORKSPACE_EDIT_FILE => ToolKind::Write,
        TOOL_WORKSPACE_LIST_FILES => ToolKind::Read,
        TOOL_WORKSPACE_BATCH => ToolKind::Composite,
        TOOL_ECHO_INPUT | TOOL_ASK_USER => ToolKind::Interactive,
        // `time_now` is a pure clock read, not a side-effecting Execute (phase-4..7 review P2-10:
        // the dispatcher's sandbox gate keys off Execute kind, so misclassifying the clock made it
        // fail `sandbox_unavailable` through the governed path).
        TOOL_TIME_NOW => ToolKind::Read,
        TOOL_WORKSPACE_RUN_COMMAND => ToolKind::Execute,
        TOOL_PLAN_CONTROL => ToolKind::Write,
        TOOL_VIEW_IMAGE => ToolKind::Read,
        TOOL_WORKSPACE_READ_DOCUMENT => ToolKind::Read,
        TOOL_SUBAGENT | TOOL_WORKFLOW => ToolKind::Composite,
        TOOL_SPAWN_TEAMMATE
        | TOOL_LIST_AGENTS
        | TOOL_SEND_MESSAGE
        | TOOL_INTERRUPT_AGENT
        | TOOL_DRAIN_INBOX => ToolKind::Interactive,
        TOOL_TEAM_TASK_CREATE | TOOL_TEAM_TASK_UPDATE => ToolKind::Write,
        TOOL_TEAM_TASK_LIST => ToolKind::Read,
        TOOL_TODO_WRITE => ToolKind::Write,
        TOOL_TODO_LIST => ToolKind::Read,
        TOOL_SPILL_READ => ToolKind::Read,
        // W37-C1：Present/Find/GitDiff 只读或检索；Job* 属 Execute（side-effecting）。
        TOOL_PRESENT | TOOL_GIT_DIFF_REMOTE => ToolKind::Read,
        TOOL_FUZZY_FILE_SEARCH => ToolKind::Search,
        TOOL_JOB_START | TOOL_JOB_OUTPUT | TOOL_JOB_KILL | TOOL_JOB_LIST => ToolKind::Execute,
        // W42/PA-149：技能正文加载是纯只读。
        TOOL_SKILL_LOAD => ToolKind::Read,
        // B150/P1：外部 ACP agent 委派是复合执行（外部 agent 自带工具循环，与 subagent 同构）。
        TOOL_ACP_DELEGATE => ToolKind::Composite,
        _ => ToolKind::External,
    }
}

/// 工具是否「并发安全」（只读/无共享副作用），供并发执行引擎分区。
///
/// 语义由 descriptor 的执行策略 `concurrent_safe` 决定（与派发层同一真源），
/// 因此新增工具只需在 `execution_policy_for_primitive` 声明即可自动生效。
pub fn tool_is_concurrent_safe(name: &str) -> bool {
    // 与派发层同源：优先用 builtin registry 的 descriptor.execution_policy（覆盖全部模型名/别名，
    // 例如 `ReadDocument` 这类未进 canonical 表的模型可见名），未知工具回退到策略函数的默认值
    // （默认 `concurrent_safe=false`，宁可串行不可并发误判）。
    if let Some(descriptor) = builtin_registry_cached().resolve(name) {
        return descriptor.execution_policy.concurrent_safe;
    }
    let primitive = canonical_tool_name(name).unwrap_or(name);
    execution_policy_for_primitive(primitive).concurrent_safe
}

/// 进程内缓存的 builtin registry 快照（避免每次并发判定重建整张表）。
pub(super) fn builtin_registry_cached() -> &'static ToolRegistrySnapshot {
    static CACHE: std::sync::OnceLock<ToolRegistrySnapshot> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| {
        ToolRegistrySnapshot::builtin().expect("static builtin registry must build")
    })
}

pub fn tool_exposure_for_name(name: &str) -> ToolExposure {
    match canonical_tool_name(name).unwrap_or(name) {
        TOOL_WORKSPACE_BATCH
        | TOOL_WORKSPACE_GATHER_CONTEXT
        | TOOL_WORKSPACE_LIST_FILES
        | TOOL_WORKSPACE_READ_FILE
        | TOOL_WORKSPACE_READ_FILE_SEGMENT
        | TOOL_WORKSPACE_GLOB_FILES
        | TOOL_WORKSPACE_SEARCH_TEXT
        | TOOL_WEB_FETCH_URL
        | TOOL_WEB_SEARCH_QUERY
        | TOOL_MCP_RESOURCE_READ
        | TOOL_WORKSPACE_WRITE_FILE
        | TOOL_WORKSPACE_EDIT_FILE
        | TOOL_WORKSPACE_RUN_COMMAND
        | TOOL_ECHO_INPUT
        | TOOL_ASK_USER
        | TOOL_TIME_NOW => ToolExposure::ModelVisible,
        TOOL_TOOL_SEARCH => ToolExposure::Deferred,
        TOOL_WORKSPACE_PATH_INFO => ToolExposure::Deferred,
        TOOL_PLAN_CONTROL => ToolExposure::ModelVisible,
        TOOL_VIEW_IMAGE => ToolExposure::ModelVisible,
        TOOL_WORKSPACE_READ_DOCUMENT => ToolExposure::ModelVisible,
        TOOL_TODO_WRITE
        | TOOL_TODO_LIST
        | TOOL_CREATE_GOAL
        | TOOL_GET_GOAL
        | TOOL_UPDATE_GOAL => ToolExposure::ModelVisible,
        // W26：spill 回读工具按需曝光（tool_search 提升），不进常驻工具面。
        TOOL_SPILL_READ => ToolExposure::Deferred,
        // W37-C1：七项新工具全部常驻模型可见面。
        TOOL_PRESENT
        | TOOL_FUZZY_FILE_SEARCH
        | TOOL_GIT_DIFF_REMOTE
        | TOOL_JOB_START
        | TOOL_JOB_OUTPUT
        | TOOL_JOB_KILL
        | TOOL_JOB_LIST => ToolExposure::ModelVisible,
        // W42/PA-149：技能正文按需加载进入常驻模型可见面。
        TOOL_SKILL_LOAD => ToolExposure::ModelVisible,
        // B150/P1：`acp_delegate` 进入常驻模型可见面（工具面 38 → 39）。
        TOOL_ACP_DELEGATE => ToolExposure::ModelVisible,
        // Wave-61 C1：9 项团队/子代理工具进入常驻模型可见面（29 → 38）。
        // 可见性**不**随 `subagent_runtime_wired` 变化（契约 C1.4）：工具面逐 turn 恒定，
        // 「可用/可执行」才由 :152 的运行时判定收口（10 处 fail-closed 门保持不变）。
        //
        // `workflow` **不**在可见面：`orchestration::workflow()` 目前是桩实现（只回显 step.id
        // 并返回 status="success"，无真实 DAG 执行），接线后模型会拿到**伪造成功**回执
        // —— 诚实性优先（L3-P 裁决）。恢复条件＝真实 DAG 执行引擎落地并独立验收；
        // 届时把它加回本分支，并把模型可见面计数改回 39。
        TOOL_SUBAGENT
        | TOOL_SPAWN_TEAMMATE
        | TOOL_LIST_AGENTS
        | TOOL_SEND_MESSAGE
        | TOOL_INTERRUPT_AGENT
        | TOOL_DRAIN_INBOX
        | TOOL_TEAM_TASK_CREATE
        | TOOL_TEAM_TASK_UPDATE
        | TOOL_TEAM_TASK_LIST => ToolExposure::ModelVisible,
        _ => ToolExposure::Internal,
    }
}

pub fn tool_display_metadata_for_name(name: &str) -> ToolDisplayMetadata {
    let display_name_zh = match model_visible_tool_name(name) {
        "Read" => Some("读取".to_string()),
        "Search" => Some("搜索".to_string()),
        "List" => Some("列表".to_string()),
        "Glob" => Some("匹配".to_string()),
        "WebFetch" => Some("抓取".to_string()),
        "WebSearch" => Some("外搜".to_string()),
        "MCPResource" => Some("资源".to_string()),
        "ToolSearch" => Some("找工具".to_string()),
        "BatchExecute" => Some("批量执行".to_string()),
        "Ask" => Some("提问".to_string()),
        "Run" => Some("运行".to_string()),
        "Write" => Some("写入".to_string()),
        "Edit" => Some("编辑".to_string()),
        "Plan" => Some("计划".to_string()),
        "ViewImage" => Some("看图".to_string()),
        "ReadDocument" => Some("读文档".to_string()),
        "SpillRead" => Some("回读溢出".to_string()),
        // W37-C1：七项新工具的中文展示名。
        "Present" => Some("呈报".to_string()),
        "Find" => Some("找文件".to_string()),
        "GitDiff" => Some("看差异".to_string()),
        "JobStart" => Some("后台任务".to_string()),
        "JobOutput" => Some("任务输出".to_string()),
        "JobKill" => Some("终止任务".to_string()),
        "JobList" => Some("任务列表".to_string()),
        // W42/PA-149：技能正文加载的中文展示名。
        TOOL_SKILL_LOAD => Some("载技能".to_string()),
        // Wave-61 C1.7：团队/子代理工具的产品名是小写原名透传（见 `model_visible_tool_name_opt`
        // 的 C1.6 分支），因此这里必须用同一常量匹配；否则 `display_name_zh` 恒为 `None`，
        // 工具消息行只能回落到英文 `canonicalToolName`/`toolName` 展示。
        TOOL_SUBAGENT => Some("子代理".to_string()),
        TOOL_SPAWN_TEAMMATE => Some("派生队友".to_string()),
        TOOL_LIST_AGENTS => Some("队友列表".to_string()),
        TOOL_SEND_MESSAGE => Some("发消息".to_string()),
        TOOL_INTERRUPT_AGENT => Some("中断队友".to_string()),
        TOOL_DRAIN_INBOX => Some("读信箱".to_string()),
        TOOL_TEAM_TASK_CREATE => Some("建任务".to_string()),
        TOOL_TEAM_TASK_UPDATE => Some("更新任务".to_string()),
        TOOL_TEAM_TASK_LIST => Some("任务板".to_string()),
        // B150/P1：外部 ACP agent 委派（小写原名透传，按产品名匹配）。
        TOOL_ACP_DELEGATE => Some("外部代理".to_string()),
        // PA-112 / W20：`todo_write` / `todo_list` 与 goal 工具族同样走「小写原名透传」，
        // 但注意本函数的匹配对象是 **产品名**（`model_visible_tool_name(name)` 的输出）：
        // todo 工具的 `canonical_tool_name` 归一后仍是小写 primitive，而 `model_visible_tool_name_opt`
        // 把二者映射回 CamelCase 产品名 `TodoWrite`/`TodoList`，故此处必须按产品名匹配；
        // goal 工具族没有 CamelCase 产品名，按小写常量匹配即可。
        "TodoWrite" => Some("记待办".to_string()),
        "TodoList" => Some("待办列表".to_string()),
        TOOL_CREATE_GOAL => Some("建目标".to_string()),
        TOOL_GET_GOAL => Some("看目标".to_string()),
        TOOL_UPDATE_GOAL => Some("更新目标".to_string()),
        _ => None,
    };
    ToolDisplayMetadata { display_name_zh }
}

pub fn default_permission_facts_for_name(name: &str) -> ToolPermissionFacts {
    let scope = match canonical_tool_name(name).unwrap_or(name) {
        TOOL_WORKSPACE_LIST_FILES
        | TOOL_WORKSPACE_READ_FILE
        | TOOL_WORKSPACE_READ_FILE_SEGMENT
        | TOOL_WORKSPACE_PATH_INFO
        | TOOL_WORKSPACE_GLOB_FILES
        | TOOL_WORKSPACE_SEARCH_TEXT
        | TOOL_WEB_FETCH_URL
        | TOOL_WEB_SEARCH_QUERY
        | TOOL_MCP_RESOURCE_READ
        | TOOL_WORKSPACE_GATHER_CONTEXT
        | TOOL_WORKSPACE_BATCH => Some("workspace.read".to_string()),
        TOOL_TOOL_SEARCH => Some("capability.discovery".to_string()),
        TOOL_WORKSPACE_WRITE_FILE | TOOL_WORKSPACE_EDIT_FILE => Some("workspace.write".to_string()),
        TOOL_WORKSPACE_RUN_COMMAND => Some("workspace.execute".to_string()),
        TOOL_TIME_NOW | TOOL_ECHO_INPUT | TOOL_PLAN_CONTROL | TOOL_ASK_USER => None,
        TOOL_VIEW_IMAGE => Some("workspace.read".to_string()),
        // W26：spill 回读只读本地落盘内容，权限面等同 workspace.read。
        TOOL_SPILL_READ => Some("workspace.read".to_string()),
        TOOL_WORKSPACE_READ_DOCUMENT => Some("workspace.read".to_string()),
        // W37-C1：Present/Find/GitDiff 只读工作区；Job* 是进程级副作用，归 workspace.execute。
        TOOL_PRESENT | TOOL_FUZZY_FILE_SEARCH | TOOL_GIT_DIFF_REMOTE => {
            Some("workspace.read".to_string())
        }
        TOOL_JOB_START | TOOL_JOB_OUTPUT | TOOL_JOB_KILL | TOOL_JOB_LIST => {
            Some("workspace.execute".to_string())
        }
        // W42/PA-149 / L3-B F5：技能正文读取面含 workspace 外全局根，使用只读专用 fact；
        // 该 fact 只映射到只读 typed scope（见 `READ_ONLY_PERMISSION_SCOPES`）。
        TOOL_SKILL_LOAD => Some(SKILLS_READ_PERMISSION_SCOPE.to_string()),
        _ => None,
    };
    // PA-114: `ask_user` is host-mediated (WaitingHost) by declaration; every other builtin keeps
    // the legacy non-mediated facts. `echo_input` stays unchanged (its host mediation is decided
    // by the policy evaluator, not the declaration).
    let host_mediated = canonical_tool_name(name).unwrap_or(name) == TOOL_ASK_USER;
    ToolPermissionFacts {
        requires_approval: Some(false),
        permission_scope: scope,
        host_mediated: Some(host_mediated),
        permission_profile: Some("builtin".to_string()),
        approval_mode: Some("none".to_string()),
        decision_source: Some("runtime".to_string()),
    }
}
