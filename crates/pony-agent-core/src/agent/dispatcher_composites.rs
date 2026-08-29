//! Governed composite handlers for `workspace_batch` and `workspace_gather_context`
//! (PA-076 phase 3, task 3.5).
//!
//! Legacy `ToolRouter::batch` / `ToolRouter::gather_context` executed their children by calling
//! `execute_internal` directly, bypassing permission, hook, budget, and telemetry pipelines. These
//! `CompositeToolHandler` implementations replace that bypass: every child now flows through the
//! governed child dispatcher, so each child independently resolves its descriptor, validates final
//! arguments, runs the policy evaluator, reserves shared budget, and emits its own lifecycle
//! record.
//!
//! Output contract: both handlers reproduce the exact legacy `aggregate_nested_results` JSON shape
//! (`status`, counts, `meta`, `plan`, `summary`, `results` with per-child
//! `index/tool/canonicalTool/arguments/status/aggregateStatus/ok/durationMs/error/output`). The
//! only additive field is `permissionSummary` (a `CompositePermissionSummary`) so the composite
//! permission surface and per-child decision evidence survive (task 3.6). Children that enter a
//! pending control state are represented as non-executed (`aggregateStatus == "aborted"`) because
//! they have no provider-consumable result; their pending request is persisted in the dispatcher.
//!
//! `GovernedToolExecutor` adapts the legacy `ToolExecutor` contract onto the governed dispatcher so
//! a runtime can route tool execution through `dispatch_governed` without changing its own surface.
//! The actual runtime switchover is a later milestone; this module only provides the adapter.

use crate::agent::child_dispatch::{ChildDispatch, ChildDispatchRequest, ChildDispatchResult};
use crate::agent::dispatcher::{
    aggregate_composite_permission, CompositeToolHandler, CompositeToolHandlerRequest,
    DispatchContext, GovernedDispatcher, PermissionDecision, PermissionVerdict,
};
use crate::agent::tool_runtime::{InvocationOrigin, ToolDispatchRequest};
use crate::agent::tools::{
    canonical_tool_name, explicit_gather_start_line, ToolCall, ToolControlKind,
    ToolExecutionContext, ToolExecutionStatus, ToolExecutor, ToolOutcome, ToolPermissionScope,
    ToolPlan, ToolPlanStep, ToolRegistrySnapshot, ToolResult,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

// Mirror of the private constants in `tools.rs` (kept in sync by contract; the legacy handlers
// still own the canonical values until task 8.1 removes the old execution path).
const TOOL_WORKSPACE_BATCH: &str = "workspace_batch";
const TOOL_WORKSPACE_GATHER_CONTEXT: &str = "workspace_gather_context";
const TOOL_WORKSPACE_PATH_INFO: &str = "workspace_path_info";
const TOOL_WORKSPACE_READ_FILE_SEGMENT: &str = "workspace_read_file_segment";
const TOOL_WORKSPACE_LIST_FILES: &str = "workspace_list_files";
const TOOL_WORKSPACE_SEARCH_TEXT: &str = "workspace_search_text";
const MAX_WORKSPACE_BATCH_CALLS: usize = 24;
/// 复合工具内部子调用统一使用的 `description` 值：`with_description` 要求所有内置
/// 工具参数必带 `description`，内部子调用由代码构造，必须显式补齐，否则子调用会被
/// schema 预检以 `missing required argument description` 拦截。
const GATHER_CHILD_DESCRIPTION: &str = "gather 内部子调用";
const MAX_GATHER_CONTEXT_PATHS: usize = 6;
const MAX_SEGMENT_LINES: usize = 400;
const DEFAULT_SEGMENT_LINES: usize = 80;
const DEFAULT_LIST_LIMIT: usize = 40;
const SUMMARY_ITEM_LIMIT: usize = 3;

// ─────────────────────────────────────────────────────────────────────────────────────────────
// GovernedToolExecutor: legacy ToolExecutor adapter over the governed dispatcher
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Bridges the legacy `ToolExecutor` contract onto a `GovernedDispatcher`. A runtime can route
/// `ToolCall` executions through the governed pipeline by constructing this adapter around a
/// dispatcher that has been given the registry, handlers, and composite handlers it needs.
pub struct GovernedToolExecutor {
    dispatcher: GovernedDispatcher,
    /// Session/run/turn/workspace facts for the current invocation. The runtime sets this per turn
    /// so control requests persist against the real session (phase-4 Ask wiring, P1-1).
    context: Mutex<DispatchContext>,
}

impl GovernedToolExecutor {
    pub fn new(dispatcher: GovernedDispatcher) -> Self {
        Self {
            dispatcher,
            context: Mutex::new(DispatchContext::default()),
        }
    }

    /// The underlying governed dispatcher, for callers that need direct access to the
    /// control-request store or budget configuration.
    pub fn dispatcher(&self) -> &GovernedDispatcher {
        &self.dispatcher
    }

    /// Set the session/run/turn/workspace facts for the current invocation. Called by the runtime
    /// before each turn so a persisted `PendingControlRequest` is bound to the real session.
    pub fn set_context(&self, context: DispatchContext) {
        *self
            .context
            .lock()
            .expect("governed executor context poisoned") = context;
    }

    /// Current invocation context.
    pub fn context(&self) -> DispatchContext {
        self.context
            .lock()
            .expect("governed executor context poisoned")
            .clone()
    }
}

impl ToolExecutor for GovernedToolExecutor {
    fn execute(&self, call: &ToolCall) -> ToolResult {
        self.execute_with_context(call, &ToolExecutionContext::default())
    }

    fn execute_with_context(&self, call: &ToolCall, exec_context: &ToolExecutionContext) -> ToolResult {
        let mut context = self
            .context
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(workspace_root) = &exec_context.workspace_root {
            context.workspace_root = Some(workspace_root.display().to_string());
        }
        let outcome = self.dispatcher.dispatch_governed(
            ToolDispatchRequest {
                origin: InvocationOrigin::Model,
                descriptor_id: call.name.clone(),
                call_id: call
                    .call_id
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string()),
                arguments: call.arguments.clone(),
            },
            &context,
        );
        let mut result = outcome.into_legacy_result(&call.name);
        // A composite that ran children but failed them all reports an `ok` execution status (its
        // handler returned a value), but its payload marks `ok: false`. Surface the aggregate
        // failure on the legacy `status` so consumers branching on `status != "ok"` do not
        // misread an all-failed batch as success (phase-3 review P2-1).
        if result.status == "ok" {
            if let Ok(payload) = serde_json::from_str::<Value>(&result.output) {
                if payload.get("ok").and_then(Value::as_bool) == Some(false) {
                    result.status = "error".to_string();
                }
            }
        }
        result
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// BatchExecuteComposite
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Governed `workspace_batch` composite. Parses the `calls` array, enforces the legacy count and
/// shape limits, and runs every child through the bounded `ChildDispatch` (`dispatch_many` for the
/// parallel path, a serial loop with stop-on-failure for the non-parallel path). The aggregation
/// reproduces the legacy `aggregate_nested_results` output shape and appends a
/// `permissionSummary` for composite permission evidence.
#[derive(Clone, Debug, Default)]
pub struct BatchExecuteComposite;

impl CompositeToolHandler for BatchExecuteComposite {
    fn execute(
        &self,
        request: &CompositeToolHandlerRequest,
        children: &ChildDispatch,
    ) -> Result<Value, String> {
        let arguments = &request.arguments;
        let calls = match arguments.get("calls").and_then(Value::as_array) {
            Some(value) if !value.is_empty() => value,
            _ => {
                return Err(
                    "missing_argument: workspace_batch 缺少必填参数 `calls`，且至少需要一个子调用。"
                        .to_string(),
                );
            }
        };
        if calls.len() > MAX_WORKSPACE_BATCH_CALLS {
            return Err(format!(
                "too_many_calls: 单次 workspace_batch 最多允许 {} 个子调用，当前收到 {} 个。",
                MAX_WORKSPACE_BATCH_CALLS,
                calls.len()
            ));
        }

        let continue_on_error = arguments
            .get("continueOnError")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let parallel = arguments
            .get("parallel")
            .and_then(Value::as_bool)
            .unwrap_or(true)
            && continue_on_error;

        let mut child_requests = Vec::with_capacity(calls.len());
        for (index, item) in calls.iter().enumerate() {
            let Some(name) = item.get("name").and_then(Value::as_str) else {
                return Err(format!(
                    "invalid_call_shape: 第 {} 个子调用缺少字符串类型的 `name`。",
                    index + 1
                ));
            };
            let mut child_arguments = item.get("arguments").cloned().unwrap_or_else(|| json!({}));
            // 子调用同样要求 `description`（with_description 必填）；模型遗漏时注入默认值，
            // 避免内部子调用被 schema 预检拦截（与 gather 内部子调用同一约束）。
            if let Some(object) = child_arguments.as_object_mut() {
                if object.get("description").map_or(true, Value::is_null) {
                    object.insert(
                        "description".to_string(),
                        Value::String(format!("批量执行子调用：{name}")),
                    );
                }
            }
            child_requests.push(ChildDispatchRequest {
                descriptor_id: name.to_string(),
                call_id: format!("child-{index}"),
                arguments: child_arguments,
            });
        }

        let plan = build_batch_tool_plan(&child_requests, parallel, continue_on_error);
        let entries = if parallel {
            let results = children.dispatch_many(child_requests)?;
            results
                .into_iter()
                .map(nested_from_many)
                .collect::<Vec<_>>()
        } else {
            run_serial_batch(children, child_requests, continue_on_error)?
        };

        let payload = aggregate_nested_entries(
            TOOL_WORKSPACE_BATCH,
            json!({
                "parallel": parallel,
                "continueOnError": continue_on_error,
            }),
            Some(plan),
            entries,
            Some(children.registry()),
        );
        Ok(payload)
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// GatherContextComposite
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Governed `workspace_gather_context` composite. Ports the legacy path-budget semantics:
/// multi-path mode aggregates up to `MAX_GATHER_CONTEXT_PATHS` paths (a synthetic
/// `too_many_paths` partial entry covers the rest); single-path mode dispatches
/// `workspace_path_info` first to determine the mode, then the mode-specific children
/// (segment / listing / search with fallback). Every child runs through governed child dispatch.
#[derive(Clone, Debug, Default)]
pub struct GatherContextComposite;

impl CompositeToolHandler for GatherContextComposite {
    fn execute(
        &self,
        request: &CompositeToolHandlerRequest,
        children: &ChildDispatch,
    ) -> Result<Value, String> {
        let arguments = &request.arguments;
        let query = arguments
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let limit = arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 100) as usize)
            .unwrap_or(DEFAULT_LIST_LIMIT);
        let line_count = arguments
            .get("lineCount")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, MAX_SEGMENT_LINES as u64) as usize)
            .unwrap_or(DEFAULT_SEGMENT_LINES);
        // PA-100：显式 startLine 全模式生效；未提供时文件模式=1、搜索模式自动定位。
        let explicit_start_line = explicit_gather_start_line(arguments);
        let paths = arguments
            .get("paths")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|path| !path.is_empty())
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let paths = unique_paths(paths);

        if !paths.is_empty() {
            let requested_path_count = paths.len();
            let gathered_count = paths.len().min(MAX_GATHER_CONTEXT_PATHS);
            let gathered_paths = paths[..gathered_count].to_vec();
            let skipped_paths = paths[gathered_count..].to_vec();

            let mut entries = Vec::with_capacity(gathered_count + 1);
            for (index, path) in gathered_paths.iter().enumerate() {
                let per_path = self.gather_single_path(
                    path,
                    &query,
                    limit,
                    explicit_start_line,
                    line_count,
                    children,
                )?;
                entries.push(NestedEntry {
                    index,
                    tool: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
                    arguments: json!({
                        "path": path,
                        "query": null_or(&query),
                        "limit": limit,
                        "startLine": explicit_start_line.unwrap_or(1),
                        "lineCount": line_count,
                    }),
                    outcome: ToolOutcome::from_legacy_result(per_path),
                });
            }
            if !skipped_paths.is_empty() {
                let message = format!(
                    "单次 workspace_gather_context 最多聚合前 {} 个路径，已跳过其余 {} 个。",
                    MAX_GATHER_CONTEXT_PATHS,
                    requested_path_count.saturating_sub(MAX_GATHER_CONTEXT_PATHS)
                );
                let skipped_result = ToolResult {
                    tool_name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
                    status: "ok".to_string(),
                    output: json_string(json!({
                        "ok": true,
                        "status": "partial",
                        "reason": "too_many_paths",
                        "message": message,
                        "skippedPaths": skipped_paths,
                    })),
                    duration_ms: 0,
                };
                entries.push(NestedEntry {
                    index: gathered_count,
                    tool: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
                    arguments: json!({
                        "paths": skipped_paths,
                        "limit": limit,
                        "startLine": explicit_start_line.unwrap_or(1),
                        "lineCount": line_count,
                    }),
                    outcome: ToolOutcome::from_legacy_result(skipped_result),
                });
            }

            let plan = build_multi_path_gather_plan(
                &gathered_paths,
                &query,
                limit,
                explicit_start_line.unwrap_or(1),
                line_count,
            );
            let payload = aggregate_nested_entries(
                TOOL_WORKSPACE_GATHER_CONTEXT,
                json!({
                    "mode": "multi_path",
                    "paths": gathered_paths,
                    "requestedPathCount": requested_path_count,
                    "skippedPaths": skipped_paths,
                    "limitApplied": requested_path_count > MAX_GATHER_CONTEXT_PATHS,
                    "pathLimit": MAX_GATHER_CONTEXT_PATHS,
                    "query": null_or(&query),
                }),
                Some(plan),
                entries,
                Some(children.registry()),
            );
            return Ok(payload);
        }

        let raw_path = arguments
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .trim();
        let path = if raw_path.is_empty() { "." } else { raw_path };
        let per_path = self.gather_single_path(
            path,
            &query,
            limit,
            explicit_start_line,
            line_count,
            children,
        )?;
        if per_path.status == "ok" {
            Ok(parse_tool_output(&per_path.output))
        } else {
            let (code, message) = error_from_tool_result(&per_path);
            Err(format!("{code}: {message}"))
        }
    }
}

impl GatherContextComposite {
    /// Gather context for one workspace path, producing the same `ToolResult` the legacy
    /// single-path `gather_context` would return: an aggregate payload on success, or a legacy
    /// structured error on path-resolution/metadata failure. The mode is derived from the
    /// `workspace_path_info` child outcome so no filesystem access happens inside the handler.
    fn gather_single_path(
        &self,
        path: &str,
        query: &str,
        limit: usize,
        explicit_start_line: Option<usize>,
        line_count: usize,
        children: &ChildDispatch,
    ) -> Result<ToolResult, String> {
        let path_info_call = ChildDispatchRequest {
            descriptor_id: TOOL_WORKSPACE_PATH_INFO.to_string(),
            call_id: "gather-path-info".to_string(),
            arguments: json!({
                "path": path,
                "description": GATHER_CHILD_DESCRIPTION,
            }),
        };
        let path_info_outcome = children.dispatch(path_info_call.clone())?;
        if child_outcome_legacy_status(&path_info_outcome) != "ok" {
            let (code, message) = child_error(&path_info_outcome);
            return Ok(error_result(TOOL_WORKSPACE_GATHER_CONTEXT, &code, message));
        }
        let path_info_payload = child_outcome_output(&path_info_outcome);
        let kind = path_info_payload
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("other");
        let display_path = path_info_payload
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(path)
            .to_string();
        let mode = if !query.trim().is_empty() {
            "search"
        } else if kind == "directory" {
            "directory"
        } else {
            "file"
        };

        match mode {
            "file" => {
                // PA-100：显式 startLine 生效；未提供时从第 1 行开始（原行为）。
                let file_start_line = explicit_start_line.unwrap_or(1);
                let segment_call = ChildDispatchRequest {
                    descriptor_id: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                    call_id: "gather-segment".to_string(),
                    arguments: json!({
                        "path": display_path,
                        "startLine": file_start_line,
                        "lineCount": line_count,
                        "description": GATHER_CHILD_DESCRIPTION,
                    }),
                };
                let segment_outcome = children.dispatch(segment_call.clone())?;
                let entries = vec![
                    nested_entry(0, &path_info_call, path_info_outcome),
                    nested_entry(1, &segment_call, segment_outcome),
                ];
                let plan = build_nested_gather_plan(mode, &display_path, query, &entries);
                let payload = aggregate_nested_entries(
                    TOOL_WORKSPACE_GATHER_CONTEXT,
                    json!({
                        "mode": "file",
                        "path": display_path,
                        "query": null_or(query),
                    }),
                    Some(plan),
                    entries,
                    Some(children.registry()),
                );
                Ok(ok_tool_result(TOOL_WORKSPACE_GATHER_CONTEXT, payload))
            }
            "directory" => {
                let list_call = ChildDispatchRequest {
                    descriptor_id: TOOL_WORKSPACE_LIST_FILES.to_string(),
                    call_id: "gather-list".to_string(),
                    arguments: json!({
                        "path": display_path,
                        "limit": limit,
                        "description": GATHER_CHILD_DESCRIPTION,
                    }),
                };
                let list_outcome = children.dispatch(list_call.clone())?;
                let entries = vec![
                    nested_entry(0, &path_info_call, path_info_outcome),
                    nested_entry(1, &list_call, list_outcome),
                ];
                let plan = build_nested_gather_plan(mode, &display_path, query, &entries);
                let payload = aggregate_nested_entries(
                    TOOL_WORKSPACE_GATHER_CONTEXT,
                    json!({
                        "mode": "directory",
                        "path": display_path,
                        "query": null_or(query),
                    }),
                    Some(plan),
                    entries,
                    Some(children.registry()),
                );
                Ok(ok_tool_result(TOOL_WORKSPACE_GATHER_CONTEXT, payload))
            }
            _ => {
                let is_file = kind == "file";
                let search_path = if is_file {
                    display_parent(&display_path)
                } else {
                    display_path.clone()
                };
                let file_pattern = if is_file {
                    Some(display_basename(&display_path))
                } else {
                    None
                };
                let mut search_arguments = json!({
                    "query": query,
                    "path": search_path,
                    "limit": limit,
                    "description": GATHER_CHILD_DESCRIPTION,
                });
                if let Some(pattern) = &file_pattern {
                    search_arguments["filePattern"] = Value::String(pattern.clone());
                }
                let search_call = ChildDispatchRequest {
                    descriptor_id: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
                    call_id: "gather-search".to_string(),
                    arguments: search_arguments,
                };
                let search_outcome = children.dispatch(search_call.clone())?;

                let mut entries = vec![nested_entry(0, &path_info_call, path_info_outcome)];
                entries.push(nested_entry(1, &search_call, search_outcome.clone()));

                if is_file {
                    let search_payload = child_outcome_output(&search_outcome);
                    // PA-100：显式 startLine 优先；未提供时才按搜索命中自动定位。
                    let start_line = explicit_start_line
                        .or_else(|| {
                            first_search_match_line(&search_payload, &display_path)
                                .map(|line| line.saturating_sub(line_count / 2).max(1))
                        })
                        .unwrap_or(1);
                    let segment_call = ChildDispatchRequest {
                        descriptor_id: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                        call_id: "gather-segment".to_string(),
                        arguments: json!({
                            "path": display_path,
                            "startLine": start_line,
                            "lineCount": line_count,
                            "description": GATHER_CHILD_DESCRIPTION,
                        }),
                    };
                    let segment_outcome = children.dispatch(segment_call.clone())?;
                    entries.push(nested_entry(2, &segment_call, segment_outcome));
                } else {
                    let search_payload = child_outcome_output(&search_outcome);
                    let should_add_listing = search_outcome.execution_status
                        != ToolExecutionStatus::Ok
                        || search_match_count(&search_payload) == 0;
                    if should_add_listing {
                        let list_call = ChildDispatchRequest {
                            descriptor_id: TOOL_WORKSPACE_LIST_FILES.to_string(),
                            call_id: "gather-list".to_string(),
                            arguments: json!({
                                "path": display_path,
                                "limit": limit,
                                "description": GATHER_CHILD_DESCRIPTION,
                            }),
                        };
                        let list_outcome = children.dispatch(list_call.clone())?;
                        entries.push(nested_entry(2, &list_call, list_outcome));
                    }
                }

                let plan = build_nested_gather_plan(mode, &display_path, query, &entries);
                let payload = aggregate_nested_entries(
                    TOOL_WORKSPACE_GATHER_CONTEXT,
                    json!({
                        "mode": "search",
                        "path": display_path,
                        "query": null_or(query),
                    }),
                    Some(plan),
                    entries,
                    Some(children.registry()),
                );
                Ok(ok_tool_result(TOOL_WORKSPACE_GATHER_CONTEXT, payload))
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Registration helper
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Register the governed `workspace_batch` and `workspace_gather_context` composite handlers
/// against the canonical builtin descriptor ids. The authority lists grant child access; for the
/// migration gate this is the read-only primitive surface, and it can be widened to the full
/// descriptor surface once the governed path fully replaces the legacy composite execution.
pub fn register_governed_composites(
    dispatcher: &GovernedDispatcher,
    batch_authority: Vec<String>,
    gather_authority: Vec<String>,
) {
    dispatcher.register_composite_handler_with_authority(
        format!("builtin:{TOOL_WORKSPACE_BATCH}"),
        batch_authority,
        Arc::new(BatchExecuteComposite),
    );
    dispatcher.register_composite_handler_with_authority(
        format!("builtin:{TOOL_WORKSPACE_GATHER_CONTEXT}"),
        gather_authority,
        Arc::new(GatherContextComposite),
    );
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Aggregation (legacy `aggregate_nested_results` shape)
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// One child execution plus the facts the legacy aggregation surface needs.
struct NestedEntry {
    index: usize,
    tool: String,
    arguments: Value,
    outcome: ToolOutcome,
}

fn nested_entry(index: usize, request: &ChildDispatchRequest, outcome: ToolOutcome) -> NestedEntry {
    NestedEntry {
        index,
        tool: request.descriptor_id.clone(),
        arguments: request.arguments.clone(),
        outcome,
    }
}

fn nested_from_many(result: ChildDispatchResult) -> NestedEntry {
    NestedEntry {
        index: result.index,
        tool: result.request.descriptor_id.clone(),
        arguments: result.request.arguments.clone(),
        outcome: result.outcome,
    }
}

/// Aggregate child outcomes into the exact legacy `aggregate_nested_results` output payload.
/// `permissionSummary` is the only additive field; it carries the conservative composite scope
/// union plus per-child decision evidence reconstructed from the observed outcomes.
fn aggregate_nested_entries(
    tool_name: &str,
    meta: Value,
    plan: Option<ToolPlan>,
    mut entries: Vec<NestedEntry>,
    registry: Option<&ToolRegistrySnapshot>,
) -> Value {
    entries.sort_by_key(|entry| entry.index);

    let decisions = entries
        .iter()
        .map(|entry| infer_decision(&entry.outcome))
        .collect::<Vec<_>>();
    // Conservative composite scope union (design.md Decision 4, phase-3 review P2-3): start from
    // the composite's own declared read scope and extend with every child's declared scopes
    // resolved through the registry, instead of hard-coding `{WorkspaceRead}`. A batch that later
    // gains write/execute children reports the real surface.
    let mut scopes = BTreeSet::from([ToolPermissionScope::WorkspaceRead]);
    if let Some(registry) = registry {
        for entry in &entries {
            if let Some(descriptor) = registry.resolve(&entry.tool) {
                scopes.extend(descriptor.permission_declaration.scopes.iter().cloned());
            }
        }
    }
    let permission = aggregate_composite_permission(scopes, &decisions);

    let mut success_count = 0usize;
    let mut partial_count = 0usize;
    let mut aborted_count = 0usize;
    let results = entries
        .iter()
        .map(|entry| {
            let status = child_outcome_legacy_status(&entry.outcome).to_string();
            let output = child_outcome_output(&entry.outcome);
            let aggregate_status = nested_output_status(&status, &output).to_string();
            match aggregate_status.as_str() {
                "ok" => success_count += 1,
                "partial" => partial_count += 1,
                "aborted" => aborted_count += 1,
                _ => {}
            }
            json!({
                "index": entry.index,
                "tool": entry.tool,
                "canonicalTool": canonical_tool_name(&entry.tool)
                    .map(str::to_string)
                    .unwrap_or_else(|| entry.tool.clone()),
                "arguments": entry.arguments,
                "status": status,
                "aggregateStatus": aggregate_status,
                "ok": aggregate_status == "ok",
                "durationMs": entry.outcome.result.as_ref().map(|result| result.duration_ms).unwrap_or(0),
                "error": output.get("error").cloned(),
                "output": output,
            })
        })
        .collect::<Vec<_>>();

    let error_count = entries
        .len()
        .saturating_sub(success_count + partial_count + aborted_count);
    let aggregate_status = if error_count == 0 && partial_count == 0 && aborted_count == 0 {
        "ok"
    } else if success_count > 0 || partial_count > 0 {
        "partial"
    } else {
        "error"
    };
    let runtime_status = if aggregate_status == "error" {
        "error"
    } else {
        "ok"
    };
    let summary = build_nested_results_summary(tool_name, &results, aggregate_status);

    json!({
        "ok": runtime_status == "ok",
        "status": aggregate_status,
        "plannedCount": results.len(),
        "completedCount": results.len().saturating_sub(aborted_count),
        "successCount": success_count,
        "partialCount": partial_count,
        "errorCount": error_count,
        "abortedCount": aborted_count,
        "meta": meta,
        "plan": plan,
        "summary": summary,
        "permissionSummary": permission,
        "results": results,
    })
}

/// Legacy `nested_result_status`/`nested_output_status` semantics: `aborted` stays `aborted`,
/// non-ok statuses become `error`, and an ok status defers to a `partial`/`error` marker inside
/// the child output.
fn nested_output_status(status: &str, output: &Value) -> &'static str {
    if status == "aborted" {
        return "aborted";
    }
    if status != "ok" {
        return "error";
    }
    match output.get("status").and_then(Value::as_str) {
        Some("partial") => "partial",
        Some("error") => "error",
        _ => "ok",
    }
}

/// Legacy status for a child `ToolOutcome`. A pending control child has no provider-consumable
/// result, so it is represented as non-executed (`aborted`), the same vocabulary
/// `dispatch_many` uses for un-started siblings.
fn child_outcome_legacy_status(outcome: &ToolOutcome) -> &'static str {
    if outcome.control_outcome.is_some() {
        return "aborted";
    }
    outcome.execution_status.as_legacy_status()
}

/// Parsed output value for a child outcome; pending control children fall back to a structured
/// `control_outcome_pending` payload instead of an empty result.
fn child_outcome_output(outcome: &ToolOutcome) -> Value {
    match &outcome.result {
        Some(result) => parse_tool_output(&result.output),
        None => json!({
            "ok": false,
            "error": {
                "code": "control_outcome_pending",
                "message": "子调用处于控制请求状态，尚未产生可供 provider 消费的终态结果。",
            },
        }),
    }
}

/// Error code + message extracted from a failed child outcome.
fn child_error(outcome: &ToolOutcome) -> (String, String) {
    let output = child_outcome_output(outcome);
    let code = output
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("child_failed");
    let message = output
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("子调用执行失败。");
    (code.to_string(), message.to_string())
}

/// Reconstruct a `PermissionDecision` from an observed child outcome so `permissionSummary`
/// preserves per-child evidence. Verdicts are derived from control outcomes and the
/// `permission_denied` marker; the decision source records that this is outcome-derived evidence
/// (the authoritative `decision_source` lives in the dispatcher lifecycle records).
fn infer_decision(outcome: &ToolOutcome) -> PermissionDecision {
    if let Some(control) = &outcome.control_outcome {
        let verdict = match control.kind {
            ToolControlKind::WaitingHost => PermissionVerdict::WaitingHost,
            ToolControlKind::ApprovalRequired | ToolControlKind::WaitingUser => {
                PermissionVerdict::ApprovalRequired
            }
        };
        return PermissionDecision {
            verdict,
            decision_source: "child_control_outcome".to_string(),
            reason: Some(format!("child entered control state {:?}", control.kind)),
        };
    }
    let denied = outcome
        .result
        .as_ref()
        .map(|result| {
            parse_tool_output(&result.output)
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str)
                == Some("permission_denied")
        })
        .unwrap_or(false);
    PermissionDecision {
        verdict: if denied {
            PermissionVerdict::Deny
        } else {
            PermissionVerdict::Allow
        },
        decision_source: if denied {
            "child_deny_outcome".to_string()
        } else {
            "child_execution_observed".to_string()
        },
        reason: None,
    }
}

/// Legacy `build_nested_results_summary` shape: human text, first error, top matches, and the
/// first listing's paths (only present when a child emitted a structured `entries` array).
fn build_nested_results_summary(
    tool_name: &str,
    results: &[Value],
    aggregate_status: &str,
) -> Value {
    let first_error = results
        .iter()
        .find_map(extract_first_error_from_nested_result);

    let top_matches = results
        .iter()
        .flat_map(extract_top_matches_from_nested_result)
        .take(SUMMARY_ITEM_LIMIT)
        .collect::<Vec<_>>();

    let listed_paths = results
        .iter()
        .find(|entry| entry.get("tool").and_then(Value::as_str) == Some(TOOL_WORKSPACE_LIST_FILES))
        .and_then(|entry| entry.get("output"))
        .and_then(|output| output.get("entries"))
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .take(5)
                .filter_map(|entry| entry.get("path").cloned())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let text = match tool_name {
        TOOL_WORKSPACE_BATCH => format!(
            "workspace_batch 已汇总 {} 个子调用，整体状态为 {}。",
            results.len(),
            aggregate_status
        ),
        TOOL_WORKSPACE_GATHER_CONTEXT => format!(
            "workspace_gather_context 已聚合 {} 个上下文子调用，整体状态为 {}。",
            results.len(),
            aggregate_status
        ),
        _ => format!(
            "已聚合 {} 个子调用，整体状态为 {}。",
            results.len(),
            aggregate_status
        ),
    };

    json!({
        "text": text,
        "firstError": first_error,
        "topMatches": top_matches,
        "listedPaths": listed_paths,
    })
}

fn extract_first_error_from_nested_result(result: &Value) -> Option<Value> {
    let status = result.get("aggregateStatus").and_then(Value::as_str)?;
    if matches!(status, "error" | "aborted") {
        return Some(json!({
            "tool": result.get("tool").cloned().unwrap_or(Value::Null),
            "index": result.get("index").cloned().unwrap_or(Value::Null),
            "status": status,
            "error": result.get("error").cloned().unwrap_or(Value::Null),
        }));
    }
    result
        .get("output")
        .and_then(|output| output.get("summary"))
        .and_then(|summary| summary.get("firstError"))
        .cloned()
        .filter(|value| !value.is_null())
}

fn extract_top_matches_from_nested_result(result: &Value) -> Vec<Value> {
    let Some(output) = result.get("output") else {
        return Vec::new();
    };
    if let Some(matches) = output.get("matches").and_then(Value::as_array) {
        return matches
            .iter()
            .take(SUMMARY_ITEM_LIMIT)
            .cloned()
            .collect::<Vec<_>>();
    }
    output
        .get("summary")
        .and_then(|summary| summary.get("topMatches"))
        .and_then(Value::as_array)
        .map(|matches| {
            matches
                .iter()
                .take(SUMMARY_ITEM_LIMIT)
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Plans (legacy shapes)
// ─────────────────────────────────────────────────────────────────────────────────────────────

fn build_batch_tool_plan(
    requests: &[ChildDispatchRequest],
    parallel: bool,
    continue_on_error: bool,
) -> ToolPlan {
    ToolPlan {
        kind: "batch".to_string(),
        summary: format!(
            "workspace_batch 计划执行 {} 个子调用{}{}。",
            requests.len(),
            if parallel {
                "（并发）"
            } else {
                "（串行）"
            },
            if continue_on_error {
                "，失败后继续"
            } else {
                "，失败后中止"
            }
        ),
        parallel,
        continue_on_error,
        steps: requests
            .iter()
            .enumerate()
            .map(|(index, request)| ToolPlanStep {
                name: canonical_tool_name(&request.descriptor_id)
                    .map(str::to_string)
                    .unwrap_or_else(|| request.descriptor_id.clone()),
                arguments: request.arguments.clone(),
                summary: format!("第 {} 个子调用：`{}`。", index + 1, request.descriptor_id),
            })
            .collect(),
    }
}

// PA-100（code-review A 建议3）：本函数与 tools.rs 中的同名拷贝为双实现镜像——
// 修改签名/回显字段必须两处同步（task 8.1 移除 legacy 路径前有效）。
fn build_multi_path_gather_plan(
    paths: &[String],
    query: &str,
    limit: usize,
    start_line: usize,
    line_count: usize,
) -> ToolPlan {
    ToolPlan {
        kind: "gather_context".to_string(),
        summary: format!(
            "workspace_gather_context 将聚合 {} 个路径{}。",
            paths.len(),
            if query.trim().is_empty() {
                ""
            } else {
                "，并带搜索条件"
            }
        ),
        parallel: false,
        continue_on_error: true,
        steps: paths
            .iter()
            .enumerate()
            .map(|(index, path)| ToolPlanStep {
                name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
                arguments: json!({
                    "path": path,
                    "query": null_or(query),
                    "limit": limit,
                    "startLine": start_line,
                    "lineCount": line_count,
                }),
                summary: format!("第 {} 个路径聚合：`{}`。", index + 1, path),
            })
            .collect(),
    }
}

fn build_nested_gather_plan(
    mode: &str,
    display_path: &str,
    query: &str,
    entries: &[NestedEntry],
) -> ToolPlan {
    let steps = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| ToolPlanStep {
            name: canonical_tool_name(&entry.tool)
                .map(str::to_string)
                .unwrap_or_else(|| entry.tool.clone()),
            arguments: entry.arguments.clone(),
            summary: format!("第 {} 个子调用：`{}`。", index + 1, entry.tool),
        })
        .collect::<Vec<_>>();
    ToolPlan {
        kind: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
        summary: match mode {
            "file" => format!("围绕文件 `{}` 聚合元信息与代码片段。", display_path),
            "directory" => format!("围绕目录 `{}` 聚合元信息与目录列表。", display_path),
            "search" => format!(
                "围绕 `{}` 聚合搜索上下文{}。",
                display_path,
                if query.trim().is_empty() {
                    ""
                } else {
                    "，并补充命中附近片段"
                }
            ),
            _ => format!("围绕 `{}` 聚合上下文。", display_path),
        },
        parallel: matches!(mode, "file" | "directory"),
        continue_on_error: true,
        steps,
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Shared helpers
// ─────────────────────────────────────────────────────────────────────────────────────────────

/// Serial (non-parallel) batch execution with the legacy stop-on-failure semantics: once a child
/// fails while `continueOnError` is false, every later sibling is aborted with `batch_aborted`.
fn run_serial_batch(
    children: &ChildDispatch,
    requests: Vec<ChildDispatchRequest>,
    continue_on_error: bool,
) -> Result<Vec<NestedEntry>, String> {
    let mut entries = Vec::with_capacity(requests.len());
    let mut stop_after: Option<usize> = None;
    for (index, request) in requests.iter().cloned().enumerate() {
        let outcome = children.dispatch(request.clone())?;
        let failed = child_outcome_legacy_status(&outcome) != "ok";
        entries.push(nested_entry(index, &request, outcome));
        if failed && !continue_on_error {
            stop_after = Some(index);
            break;
        }
    }
    if let Some(failed_index) = stop_after {
        for (index, request) in requests.into_iter().enumerate().skip(failed_index + 1) {
            entries.push(nested_entry(
                index,
                &request,
                aborted_outcome(
                    TOOL_WORKSPACE_BATCH,
                    "batch_aborted",
                    "前一个子调用失败，且 continueOnError=false，后续子调用未执行。".to_string(),
                ),
            ));
        }
    }
    Ok(entries)
}

fn aborted_outcome(tool_name: &str, code: &str, message: String) -> ToolOutcome {
    ToolOutcome::from_legacy_result(ToolResult {
        tool_name: tool_name.to_string(),
        status: "aborted".to_string(),
        output: json_string(json!({
            "ok": false,
            "tool": tool_name,
            "status": "aborted",
            "error": {
                "code": code,
                "message": message,
                "hint": Value::Null,
            },
        })),
        duration_ms: 0,
    })
}

fn error_result(tool_name: &str, code: &str, message: String) -> ToolResult {
    ToolResult {
        tool_name: tool_name.to_string(),
        status: "error".to_string(),
        output: json_string(json!({
            "ok": false,
            "tool": tool_name,
            "error": {
                "code": code,
                "message": message,
                "hint": Value::Null,
            },
            "summary": {
                "text": message
            }
        })),
        duration_ms: 0,
    }
}

fn ok_tool_result(tool_name: &str, payload: Value) -> ToolResult {
    ToolResult {
        tool_name: tool_name.to_string(),
        status: "ok".to_string(),
        output: json_string(payload),
        duration_ms: 0,
    }
}

fn error_from_tool_result(result: &ToolResult) -> (String, String) {
    let output = parse_tool_output(&result.output);
    let code = output
        .get("error")
        .and_then(|error| error.get("code"))
        .and_then(Value::as_str)
        .unwrap_or("handler_error");
    let message = output
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("工具执行失败。");
    (code.to_string(), message.to_string())
}

fn unique_paths(paths: Vec<String>) -> Vec<String> {
    let mut unique = Vec::new();
    for path in paths {
        if !unique.iter().any(|existing| existing == &path) {
            unique.push(path);
        }
    }
    unique
}

/// First search-match line for `target_path`, mirroring the legacy helper.
fn first_search_match_line(output: &Value, target_path: &str) -> Option<usize> {
    output
        .get("matches")
        .and_then(Value::as_array)
        .and_then(|matches| {
            matches.iter().find_map(|entry| {
                let path = entry.get("path").and_then(Value::as_str)?;
                let line = entry.get("line").and_then(Value::as_u64)? as usize;
                if path == target_path {
                    Some(line)
                } else {
                    None
                }
            })
        })
}

fn search_match_count(output: &Value) -> usize {
    output
        .get("matchCount")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize
}

/// Workspace-relative parent directory of a display path, mirroring the legacy
/// `resolved.parent().map(display_workspace_relative)` semantics for relative paths.
fn display_parent(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(index) => trimmed[..index].to_string(),
        None => ".".to_string(),
    }
}

/// Workspace-relative file basename of a display path, mirroring the legacy
/// `resolved.file_name()` semantics for relative paths.
fn display_basename(path: &str) -> String {
    let trimmed = path.trim_end_matches(['/', '\\']);
    match trimmed.rfind(['/', '\\']) {
        Some(index) => trimmed[index + 1..].to_string(),
        None => trimmed.to_string(),
    }
}

fn parse_tool_output(output: &str) -> Value {
    serde_json::from_str::<Value>(output).unwrap_or_else(|_| Value::String(output.to_string()))
}

fn json_string(value: Value) -> String {
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
}

fn null_or(value: &str) -> Value {
    if value.is_empty() {
        Value::Null
    } else {
        Value::String(value.to_string())
    }
}

// ─────────────────────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::budget::DispatchBudgetConfig;
    use crate::agent::dispatcher::{
        DispatchLifecycleObserver, DispatchLifecyclePhase, DispatchLifecycleRecord,
        ToolPolicyEvaluator,
    };
    use crate::agent::tool_runtime::{
        FakeClock, PrimitiveToolHandler, PrimitiveToolHandlerRequest, ToolDispatcher,
    };
    use crate::agent::tools::{
        ToolDescriptor, ToolDescriptorSource, ToolDisplayMetadata, ToolExecutionPolicy,
        ToolExposure, ToolHandlerProvenance, ToolIdentity, ToolKind, ToolPermissionDeclaration,
        ToolRegistrySnapshot,
    };
    use serde_json::json;
    use std::sync::Mutex;

    // ── test construction helpers ────────────────────────────────────────────────────────────

    fn make_descriptor(
        id: &str,
        kind: ToolKind,
        exposure: ToolExposure,
        schema: Value,
        source: ToolDescriptorSource,
    ) -> ToolDescriptor {
        let source_id = if source == ToolDescriptorSource::Builtin {
            "builtin-tools"
        } else {
            "test-source"
        };
        ToolDescriptor {
            identity: ToolIdentity {
                descriptor_id: id.to_string(),
                model_name: id.to_string(),
                canonical_name: id.to_string(),
                primitive_name: id.to_string(),
                source,
            },
            aliases: vec![id.to_string()],
            description: String::new(),
            input_schema: schema,
            kind,
            exposure,
            permission_declaration: ToolPermissionDeclaration::default(),
            execution_policy: ToolExecutionPolicy::default(),
            display_metadata: ToolDisplayMetadata::default(),
            handler_provenance: ToolHandlerProvenance {
                handler_kind: "test".to_string(),
                source_id: source_id.to_string(),
            },
            source_revision: "test-v1".to_string(),
            composed_descriptor_ids: Vec::new(),
        }
    }

    fn dynamic_descriptor(
        id: &str,
        kind: ToolKind,
        exposure: ToolExposure,
        schema: Value,
    ) -> ToolDescriptor {
        make_descriptor(id, kind, exposure, schema, ToolDescriptorSource::Dynamic)
    }

    /// Builtin-style workspace primitive descriptor. The alias is the primitive name so the
    /// composite children (`workspace_path_info`, ...) resolve exactly as in the real registry.
    fn workspace_primitive_descriptor(
        primitive: &str,
        kind: ToolKind,
        exposure: ToolExposure,
        schema: Value,
    ) -> ToolDescriptor {
        let id = format!("builtin:{primitive}");
        let mut descriptor =
            make_descriptor(&id, kind, exposure, schema, ToolDescriptorSource::Builtin);
        descriptor.aliases = vec![primitive.to_string(), format!("workspace.{primitive}")];
        descriptor
    }

    fn text_schema() -> Value {
        json!({
            "type": "object",
            "properties": { "text": { "type": "string" } },
            "required": ["text"],
            "additionalProperties": false,
        })
    }

    /// Mirrors the production builtin schemas after `with_description`: `description` is a
    /// required property, so the batch composite's injection is exercised instead of masked.
    fn batch_text_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "text": { "type": "string" },
                "description": { "type": "string" },
            },
            "required": ["text", "description"],
            "additionalProperties": false,
        })
    }

    fn batch_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "calls": { "type": "array" },
                "continueOnError": { "type": "boolean" },
                "parallel": { "type": "boolean" },
            },
        })
    }

    fn gather_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "query": { "type": "string" },
                "limit": { "type": "integer" },
                "startLine": { "type": "integer" },
                "lineCount": { "type": "integer" },
                "paths": { "type": "array" },
            },
        })
    }

    /// Mirrors the production builtin child schemas (with_description): `description` is required,
    /// so composites must inject it on every internal child call. Using the permissive schema here
    /// would mask the PA-076 regression where gather children failed with
    /// `missing required argument description`.
    fn description_required_schema() -> Value {
        json!({
            "type": "object",
            "required": ["description"],
        })
    }

    fn registry(descriptors: Vec<ToolDescriptor>) -> Arc<ToolRegistrySnapshot> {
        Arc::new(
            ToolRegistrySnapshot::from_descriptors("test-snapshot-composites", descriptors)
                .expect("test registry must build"),
        )
    }

    fn dispatcher_for(registry: Arc<ToolRegistrySnapshot>) -> GovernedDispatcher {
        GovernedDispatcher::new(registry, Arc::new(FakeClock::new(1_000)))
    }

    fn echo_handler() -> Arc<dyn PrimitiveToolHandler> {
        #[derive(Clone)]
        struct Echo;
        impl PrimitiveToolHandler for Echo {
            fn execute(&self, request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
                Ok(request.arguments.clone())
            }
        }
        Arc::new(Echo)
    }

    #[derive(Clone)]
    struct FakePathInfo {
        kind: &'static str,
    }

    impl PrimitiveToolHandler for FakePathInfo {
        fn execute(&self, request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
            let path = request
                .arguments
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or(".");
            Ok(json!({ "kind": self.kind, "path": path }))
        }
    }

    #[derive(Clone)]
    struct FakeSearch {
        matches: Vec<Value>,
        match_count: u64,
    }

    impl PrimitiveToolHandler for FakeSearch {
        fn execute(&self, request: &PrimitiveToolHandlerRequest) -> Result<Value, String> {
            let path = request
                .arguments
                .get("path")
                .and_then(Value::as_str)
                .unwrap_or(".");
            Ok(json!({
                "path": path,
                "matchCount": self.match_count,
                "matches": self.matches,
            }))
        }
    }

    fn register_gather(dispatcher: &GovernedDispatcher) {
        dispatcher.register_composite_handler_with_authority(
            "dynamic:gather",
            vec![
                "builtin:workspace_path_info".to_string(),
                "builtin:workspace_read_file_segment".to_string(),
                "builtin:workspace_list_files".to_string(),
                "builtin:workspace_search_text".to_string(),
            ],
            Arc::new(GatherContextComposite),
        );
        dispatcher.register_handler(
            "builtin:workspace_path_info",
            Arc::new(FakePathInfo { kind: "file" }),
        );
        dispatcher.register_handler("builtin:workspace_read_file_segment", echo_handler());
        dispatcher.register_handler("builtin:workspace_list_files", echo_handler());
        dispatcher.register_handler(
            "builtin:workspace_search_text",
            Arc::new(FakeSearch {
                matches: Vec::new(),
                match_count: 0,
            }),
        );
    }

    fn dispatch(
        dispatcher: &GovernedDispatcher,
        descriptor_id: &str,
        call_id: &str,
        arguments: Value,
    ) -> ToolOutcome {
        dispatcher.dispatch(ToolDispatchRequest {
            origin: InvocationOrigin::Model,
            descriptor_id: descriptor_id.to_string(),
            call_id: call_id.to_string(),
            arguments,
        })
    }

    fn parsed_output(outcome: &ToolOutcome) -> Value {
        serde_json::from_str(&outcome.result.as_ref().expect("result").output)
            .expect("outcome output must be json")
    }

    struct RecordingObserver(Mutex<Vec<DispatchLifecycleRecord>>);

    impl DispatchLifecycleObserver for RecordingObserver {
        fn record(&self, event: &DispatchLifecycleRecord) {
            self.0
                .lock()
                .expect("observer lock poisoned")
                .push(event.clone());
        }
    }

    // ── batch through governed child dispatch ───────────────────────────────────────────────

    #[test]
    fn batch_executes_children_through_governed_dispatch_and_matches_legacy_shape() {
        let registry = registry(vec![
            dynamic_descriptor(
                "dynamic:batch",
                ToolKind::Composite,
                ToolExposure::ModelVisible,
                batch_schema(),
            ),
            dynamic_descriptor(
                "dynamic:leaf",
                ToolKind::Read,
                ToolExposure::Internal,
                batch_text_schema(),
            ),
        ]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_composite_handler_with_authority(
            "dynamic:batch",
            vec!["dynamic:leaf".to_string()],
            Arc::new(BatchExecuteComposite),
        );
        dispatcher.register_handler("dynamic:leaf", echo_handler());
        let observer = Arc::new(RecordingObserver(Mutex::new(Vec::new())));
        dispatcher.register_lifecycle_observer(
            Arc::clone(&observer) as Arc<dyn DispatchLifecycleObserver>
        );

        let outcome = dispatch(
            &dispatcher,
            "dynamic:batch",
            "batch-1",
            json!({
                "parallel": true,
                "continueOnError": true,
                "calls": [
                    { "name": "dynamic:leaf", "arguments": { "text": "a" } },
                    { "name": "dynamic:leaf", "arguments": { "text": "b" } },
                    { "name": "dynamic:leaf", "arguments": { "text": "c" } },
                ],
            }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        let payload = parsed_output(&outcome);
        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["successCount"].as_u64(), Some(3));
        assert_eq!(payload["errorCount"].as_u64(), Some(0));
        assert_eq!(payload["abortedCount"].as_u64(), Some(0));
        assert_eq!(payload["plannedCount"].as_u64(), Some(3));
        assert_eq!(payload["completedCount"].as_u64(), Some(3));

        let results = payload["results"].as_array().expect("results array");
        assert_eq!(results.len(), 3);
        for (index, result) in results.iter().enumerate() {
            assert_eq!(result["index"].as_u64(), Some(index as u64));
            assert_eq!(result["tool"], "dynamic:leaf");
            assert_eq!(result["canonicalTool"], "dynamic:leaf");
            assert_eq!(result["status"], "ok");
            assert_eq!(result["aggregateStatus"], "ok");
            assert_eq!(result["ok"], true);
            assert!(result["arguments"]["text"].is_string());
            assert!(result["output"]["text"].is_string());
        }
        assert_eq!(payload["meta"]["parallel"], true);
        assert_eq!(payload["meta"]["continueOnError"], true);
        assert_eq!(payload["plan"]["kind"], "batch");
        assert_eq!(payload["plan"]["parallel"], true);
        let summary_text = payload["summary"]["text"].as_str().expect("summary text");
        assert!(summary_text.contains("3"));
        assert_eq!(payload["summary"]["firstError"], Value::Null);

        // The aggregate permission summary is present and reflects the allowed children.
        assert_eq!(
            payload["permissionSummary"]["scopes"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(payload["permissionSummary"]["requiresApproval"], false);
        assert_eq!(payload["permissionSummary"]["hostMediated"], false);
        assert_eq!(
            payload["permissionSummary"]["childDecisions"]
                .as_array()
                .map(Vec::len),
            Some(3)
        );

        // Exactly one lifecycle record per child, each at depth 1 with the parent call id.
        let records = observer.0.lock().expect("observer lock poisoned");
        assert_eq!(records.len(), 4, "one top-level + three children");
        let top = records
            .iter()
            .find(|record| record.depth == 0)
            .expect("top-level record");
        assert_eq!(top.descriptor_id, "dynamic:batch");
        assert_eq!(top.call_id, "batch-1");
        assert_eq!(top.parent_call_id, None);
        let child_records = records
            .iter()
            .filter(|record| record.depth >= 1)
            .collect::<Vec<_>>();
        assert_eq!(child_records.len(), 3);
        for record in &child_records {
            assert_eq!(record.descriptor_id, "dynamic:leaf");
            assert_eq!(record.parent_call_id.as_deref(), Some("batch-1"));
            assert_eq!(record.depth, 1);
            assert_eq!(record.origin, InvocationOrigin::Child);
            assert_eq!(record.phase, DispatchLifecyclePhase::Completed);
            assert_eq!(record.execution_status, Some(ToolExecutionStatus::Ok));
        }
    }

    #[test]
    fn batch_shared_budget_exhaustion_fails_the_overflow_child_closed() {
        let registry = registry(vec![
            dynamic_descriptor(
                "dynamic:batch",
                ToolKind::Composite,
                ToolExposure::ModelVisible,
                batch_schema(),
            ),
            dynamic_descriptor(
                "dynamic:leaf",
                ToolKind::Read,
                ToolExposure::Internal,
                batch_text_schema(),
            ),
        ]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.set_budget_config(DispatchBudgetConfig {
            max_composite_calls: 1,
            ..Default::default()
        });
        dispatcher.register_composite_handler_with_authority(
            "dynamic:batch",
            vec!["dynamic:leaf".to_string()],
            Arc::new(BatchExecuteComposite),
        );
        dispatcher.register_handler("dynamic:leaf", echo_handler());

        let outcome = dispatch(
            &dispatcher,
            "dynamic:batch",
            "batch-1",
            json!({
                "calls": [
                    { "name": "dynamic:leaf", "arguments": { "text": "a" } },
                    { "name": "dynamic:leaf", "arguments": { "text": "b" } },
                ],
            }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        let payload = parsed_output(&outcome);
        assert_eq!(payload["successCount"].as_u64(), Some(1));
        assert_eq!(payload["errorCount"].as_u64(), Some(1));
        let results = payload["results"].as_array().expect("results array");
        let failed = results
            .iter()
            .find(|result| result["aggregateStatus"] == "error")
            .expect("one child fails against the shared budget");
        assert_eq!(failed["error"]["code"], "budget_exhausted");
    }

    #[test]
    fn batch_approval_child_enters_pending_and_unstarted_siblings_are_aborted() {
        let mut approval = ToolPermissionDeclaration::default();
        approval.requires_approval = true;
        let mut approve_me = dynamic_descriptor(
            "dynamic:approve_me",
            ToolKind::Write,
            ToolExposure::Internal,
            batch_text_schema(),
        );
        approve_me.permission_declaration = approval;
        let approval_registry = registry(vec![
            dynamic_descriptor(
                "dynamic:batch",
                ToolKind::Composite,
                ToolExposure::ModelVisible,
                batch_schema(),
            ),
            approve_me,
            dynamic_descriptor(
                "dynamic:leaf",
                ToolKind::Read,
                ToolExposure::Internal,
                batch_text_schema(),
            ),
        ]);
        let dispatcher = dispatcher_for(approval_registry);
        dispatcher.register_composite_handler_with_authority(
            "dynamic:batch",
            vec!["dynamic:approve_me".to_string(), "dynamic:leaf".to_string()],
            Arc::new(BatchExecuteComposite),
        );
        dispatcher.register_handler("dynamic:leaf", echo_handler());

        let outcome = dispatch(
            &dispatcher,
            "dynamic:batch",
            "batch-1",
            json!({
                "parallel": true,
                "continueOnError": true,
                "calls": [
                    { "name": "dynamic:approve_me", "arguments": { "text": "approve" } },
                    { "name": "dynamic:leaf", "arguments": { "text": "leaf" } },
                ],
            }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        let payload = parsed_output(&outcome);
        let results = payload["results"].as_array().expect("results array");
        assert_eq!(results.len(), 2);
        // The approval-required child never executed; the sibling was never started.
        assert_eq!(results[0]["aggregateStatus"], "aborted");
        assert_eq!(results[0]["error"]["code"], "control_outcome_pending");
        assert_eq!(results[1]["aggregateStatus"], "aborted");
        assert_eq!(results[1]["error"]["code"], "not_started");
        assert_eq!(payload["abortedCount"].as_u64(), Some(2));

        // The pending control request was persisted bound to the child invocation facts.
        let pending = dispatcher.pending_requests();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].descriptor_id, "dynamic:approve_me");

        // The composite permission summary reflects the approval surface.
        assert_eq!(payload["permissionSummary"]["requiresApproval"], true);
        let decisions = payload["permissionSummary"]["childDecisions"]
            .as_array()
            .expect("child decisions");
        assert_eq!(decisions[0]["verdict"], "approval_required");
        assert_eq!(decisions[0]["decisionSource"], "child_control_outcome");
    }

    #[test]
    fn batch_denied_child_fails_closed_without_killing_the_batch() {
        struct DenySpecific {
            denied: &'static str,
        }

        impl ToolPolicyEvaluator for DenySpecific {
            fn evaluate(
                &self,
                descriptor: &ToolDescriptor,
                _origin: &InvocationOrigin,
                _final_arguments: &Value,
            ) -> PermissionDecision {
                if descriptor.identity.descriptor_id == self.denied {
                    PermissionDecision {
                        verdict: PermissionVerdict::Deny,
                        decision_source: "test-denier".to_string(),
                        reason: Some("policy says no".to_string()),
                    }
                } else {
                    PermissionDecision {
                        verdict: PermissionVerdict::Allow,
                        decision_source: "descriptor_declaration".to_string(),
                        reason: None,
                    }
                }
            }
        }

        let registry = registry(vec![
            dynamic_descriptor(
                "dynamic:batch",
                ToolKind::Composite,
                ToolExposure::ModelVisible,
                batch_schema(),
            ),
            dynamic_descriptor(
                "dynamic:leaf",
                ToolKind::Read,
                ToolExposure::Internal,
                batch_text_schema(),
            ),
            dynamic_descriptor(
                "dynamic:denied",
                ToolKind::Write,
                ToolExposure::Internal,
                batch_text_schema(),
            ),
        ]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_policy_evaluator(Arc::new(DenySpecific {
            denied: "dynamic:denied",
        }));
        dispatcher.register_composite_handler_with_authority(
            "dynamic:batch",
            vec!["dynamic:leaf".to_string(), "dynamic:denied".to_string()],
            Arc::new(BatchExecuteComposite),
        );
        dispatcher.register_handler("dynamic:leaf", echo_handler());

        let outcome = dispatch(
            &dispatcher,
            "dynamic:batch",
            "batch-1",
            json!({
                "parallel": true,
                "continueOnError": true,
                "calls": [
                    { "name": "dynamic:leaf", "arguments": { "text": "a" } },
                    { "name": "dynamic:denied", "arguments": { "text": "nope" } },
                ],
            }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        let payload = parsed_output(&outcome);
        assert_eq!(payload["status"], "partial");
        assert_eq!(payload["successCount"].as_u64(), Some(1));
        assert_eq!(payload["errorCount"].as_u64(), Some(1));
        let results = payload["results"].as_array().expect("results array");
        assert_eq!(results[0]["status"], "ok");
        assert_eq!(results[1]["status"], "error");
        assert_eq!(results[1]["aggregateStatus"], "error");
        assert_eq!(results[1]["error"]["code"], "permission_denied");

        // The composite summary keeps the denied child's decision evidence.
        let decisions = payload["permissionSummary"]["childDecisions"]
            .as_array()
            .expect("child decisions");
        assert_eq!(decisions.len(), 2);
        assert_eq!(decisions[1]["verdict"], "deny");
        assert_eq!(decisions[1]["decisionSource"], "child_deny_outcome");
    }

    #[test]
    fn batch_rejects_missing_calls_too_many_calls_and_invalid_shapes() {
        let registry = registry(vec![
            dynamic_descriptor(
                "dynamic:batch",
                ToolKind::Composite,
                ToolExposure::ModelVisible,
                batch_schema(),
            ),
            dynamic_descriptor(
                "dynamic:leaf",
                ToolKind::Read,
                ToolExposure::Internal,
                batch_text_schema(),
            ),
        ]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_composite_handler_with_authority(
            "dynamic:batch",
            vec!["dynamic:leaf".to_string()],
            Arc::new(BatchExecuteComposite),
        );
        dispatcher.register_handler("dynamic:leaf", echo_handler());

        // Missing / empty `calls` fails closed before any child dispatch.
        let missing = dispatch(&dispatcher, "dynamic:batch", "batch-1", json!({}));
        assert_eq!(missing.execution_status, ToolExecutionStatus::Error);
        assert_eq!(
            parsed_output(&missing)["error"]["code"].as_str(),
            Some("missing_argument")
        );

        let empty = dispatch(
            &dispatcher,
            "dynamic:batch",
            "batch-2",
            json!({ "calls": [] }),
        );
        assert_eq!(empty.execution_status, ToolExecutionStatus::Error);
        assert_eq!(
            parsed_output(&empty)["error"]["code"].as_str(),
            Some("missing_argument")
        );

        // More than MAX_WORKSPACE_BATCH_CALLS fails closed.
        let too_many_calls = (0..25)
            .map(|index| json!({ "name": "dynamic:leaf", "arguments": { "text": format!("x{index}") } }))
            .collect::<Vec<_>>();
        let overflow = dispatch(
            &dispatcher,
            "dynamic:batch",
            "batch-3",
            json!({ "calls": too_many_calls }),
        );
        assert_eq!(overflow.execution_status, ToolExecutionStatus::Error);
        assert_eq!(
            parsed_output(&overflow)["error"]["code"].as_str(),
            Some("too_many_calls")
        );

        // A child without a string `name` fails closed.
        let invalid = dispatch(
            &dispatcher,
            "dynamic:batch",
            "batch-4",
            json!({
                "calls": [
                    { "name": "dynamic:leaf", "arguments": { "text": "a" } },
                    { "arguments": { "text": "b" } },
                ],
            }),
        );
        assert_eq!(invalid.execution_status, ToolExecutionStatus::Error);
        assert_eq!(
            parsed_output(&invalid)["error"]["code"].as_str(),
            Some("invalid_call_shape")
        );
    }

    // ── GovernedToolExecutor adapter ─────────────────────────────────────────────────────────

    #[test]
    fn governed_tool_executor_adapts_a_primitive_dispatch() {
        let registry = registry(vec![dynamic_descriptor(
            "dynamic:echo",
            ToolKind::Read,
            ToolExposure::ModelVisible,
            text_schema(),
        )]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_handler("dynamic:echo", echo_handler());
        let executor = GovernedToolExecutor::new(dispatcher);

        let result = executor.execute(&ToolCall {
            call_id: Some("call-1".to_string()),
            name: "dynamic:echo".to_string(),
            arguments: json!({ "text": "hello" }),
            plan: None,
        });
        assert_eq!(result.status, "ok");
        assert_eq!(result.tool_name, "dynamic:echo");
        let payload: Value = serde_json::from_str(&result.output).expect("executor output json");
        assert_eq!(payload["text"], "hello");
    }

    // ── gather_context through governed child dispatch ───────────────────────────────────────

    #[test]
    fn gather_context_file_mode_dispatches_path_info_and_segment() {
        let registry = registry(vec![
            dynamic_descriptor(
                "dynamic:gather",
                ToolKind::Composite,
                ToolExposure::ModelVisible,
                gather_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_path_info",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_read_file_segment",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_list_files",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_search_text",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
        ]);
        let dispatcher = dispatcher_for(registry);
        register_gather(&dispatcher);

        let outcome = dispatch(
            &dispatcher,
            "dynamic:gather",
            "gather-1",
            json!({ "path": "demo.rs", "lineCount": 20 }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        let payload = parsed_output(&outcome);
        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["meta"]["mode"], "file");
        assert_eq!(payload["successCount"].as_u64(), Some(2));
        let results = payload["results"].as_array().expect("results array");
        assert_eq!(results[0]["tool"], "workspace_path_info");
        assert_eq!(results[0]["status"], "ok");
        assert_eq!(results[1]["tool"], "workspace_read_file_segment");
        assert_eq!(results[1]["arguments"]["startLine"].as_u64(), Some(1));
        assert_eq!(results[1]["arguments"]["lineCount"].as_u64(), Some(20));
        assert_eq!(payload["plan"]["kind"], "workspace_gather_context");
        assert_eq!(payload["plan"]["parallel"], true);
        assert_eq!(
            payload["permissionSummary"]["scopes"]
                .as_array()
                .map(Vec::len),
            Some(1)
        );
    }

    #[test]
    fn gather_context_directory_mode_dispatches_path_info_and_list() {
        let registry = registry(vec![
            dynamic_descriptor(
                "dynamic:gather",
                ToolKind::Composite,
                ToolExposure::ModelVisible,
                gather_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_path_info",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_read_file_segment",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_list_files",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_search_text",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
        ]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_composite_handler_with_authority(
            "dynamic:gather",
            vec![
                "builtin:workspace_path_info".to_string(),
                "builtin:workspace_read_file_segment".to_string(),
                "builtin:workspace_list_files".to_string(),
                "builtin:workspace_search_text".to_string(),
            ],
            Arc::new(GatherContextComposite),
        );
        dispatcher.register_handler(
            "builtin:workspace_path_info",
            Arc::new(FakePathInfo { kind: "directory" }),
        );
        dispatcher.register_handler("builtin:workspace_read_file_segment", echo_handler());
        dispatcher.register_handler("builtin:workspace_list_files", echo_handler());
        dispatcher.register_handler(
            "builtin:workspace_search_text",
            Arc::new(FakeSearch {
                matches: Vec::new(),
                match_count: 0,
            }),
        );

        let outcome = dispatch(
            &dispatcher,
            "dynamic:gather",
            "gather-1",
            json!({ "path": "src", "limit": 10 }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        let payload = parsed_output(&outcome);
        assert_eq!(payload["meta"]["mode"], "directory");
        assert_eq!(payload["successCount"].as_u64(), Some(2));
        let results = payload["results"].as_array().expect("results array");
        assert_eq!(results[0]["tool"], "workspace_path_info");
        assert_eq!(results[1]["tool"], "workspace_list_files");
        assert_eq!(results[1]["arguments"]["limit"].as_u64(), Some(10));
    }

    #[test]
    fn gather_context_search_file_uses_first_match_to_position_the_segment() {
        let registry = registry(vec![
            dynamic_descriptor(
                "dynamic:gather",
                ToolKind::Composite,
                ToolExposure::ModelVisible,
                gather_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_path_info",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_read_file_segment",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_list_files",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_search_text",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
        ]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_composite_handler_with_authority(
            "dynamic:gather",
            vec![
                "builtin:workspace_path_info".to_string(),
                "builtin:workspace_read_file_segment".to_string(),
                "builtin:workspace_list_files".to_string(),
                "builtin:workspace_search_text".to_string(),
            ],
            Arc::new(GatherContextComposite),
        );
        dispatcher.register_handler(
            "builtin:workspace_path_info",
            Arc::new(FakePathInfo { kind: "file" }),
        );
        dispatcher.register_handler("builtin:workspace_read_file_segment", echo_handler());
        dispatcher.register_handler("builtin:workspace_list_files", echo_handler());
        dispatcher.register_handler(
            "builtin:workspace_search_text",
            Arc::new(FakeSearch {
                match_count: 1,
                matches: vec![json!({ "path": "demo.rs", "line": 50, "preview": "needle" })],
            }),
        );

        let outcome = dispatch(
            &dispatcher,
            "dynamic:gather",
            "gather-1",
            json!({ "path": "demo.rs", "query": "needle", "lineCount": 40, "limit": 10 }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        let payload = parsed_output(&outcome);
        assert_eq!(payload["meta"]["mode"], "search");
        assert_eq!(payload["successCount"].as_u64(), Some(3));
        let results = payload["results"].as_array().expect("results array");
        assert_eq!(results[0]["tool"], "workspace_path_info");
        assert_eq!(results[1]["tool"], "workspace_search_text");
        // The search targets the file's parent directory and restricts to the basename.
        assert_eq!(results[1]["arguments"]["path"], ".");
        assert_eq!(results[1]["arguments"]["filePattern"], "demo.rs");
        assert_eq!(results[2]["tool"], "workspace_read_file_segment");
        // startLine = max(1, 50 - 40/2) = 30.
        assert_eq!(results[2]["arguments"]["startLine"].as_u64(), Some(30));
    }

    #[test]
    fn gather_context_multi_path_aggregates_each_path_and_limits_excess_paths() {
        let registry = registry(vec![
            dynamic_descriptor(
                "dynamic:gather",
                ToolKind::Composite,
                ToolExposure::ModelVisible,
                gather_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_path_info",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_read_file_segment",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_list_files",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_search_text",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
        ]);
        let dispatcher = dispatcher_for(registry);
        register_gather(&dispatcher);

        let outcome = dispatch(
            &dispatcher,
            "dynamic:gather",
            "gather-1",
            json!({
                "paths": ["one.rs", "two.rs"],
                "lineCount": 20,
            }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        let payload = parsed_output(&outcome);
        assert_eq!(payload["status"], "ok");
        assert_eq!(payload["meta"]["mode"], "multi_path");
        assert_eq!(payload["successCount"].as_u64(), Some(2));
        let results = payload["results"].as_array().expect("results array");
        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["tool"], "workspace_gather_context");
        assert_eq!(results[0]["status"], "ok");
        assert_eq!(results[0]["output"]["meta"]["mode"], "file");
        assert_eq!(results[0]["output"]["successCount"].as_u64(), Some(2));
        assert_eq!(results[1]["output"]["meta"]["mode"], "file");

        // More than MAX_GATHER_CONTEXT_PATHS paths become a partial too_many_paths entry.
        let excess_paths = (0..=6)
            .map(|index| Value::String(format!("demo-{index}.rs")))
            .collect::<Vec<_>>();
        let limited = dispatch(
            &dispatcher,
            "dynamic:gather",
            "gather-2",
            json!({ "paths": excess_paths, "lineCount": 20 }),
        );
        assert_eq!(limited.execution_status, ToolExecutionStatus::Ok);
        let payload = parsed_output(&limited);
        assert_eq!(payload["meta"]["requestedPathCount"].as_u64(), Some(7));
        assert_eq!(payload["meta"]["limitApplied"], true);
        assert_eq!(payload["meta"]["pathLimit"].as_u64(), Some(6));
        assert_eq!(
            payload["meta"]["skippedPaths"].as_array().map(Vec::len),
            Some(1)
        );
        assert_eq!(payload["status"], "partial");
        assert_eq!(payload["successCount"].as_u64(), Some(6));
        assert_eq!(payload["partialCount"].as_u64(), Some(1));
        let results = payload["results"].as_array().expect("results array");
        assert_eq!(results.len(), 7);
        assert_eq!(results[6]["output"]["reason"], "too_many_paths");
    }

    // ── PA-100：startLine 端到端透传 ────────────────────────────────────────────────────────

    #[test]
    fn gather_context_threads_explicit_start_line_into_segment_and_plan() {
        let registry = registry(vec![
            dynamic_descriptor(
                "dynamic:gather",
                ToolKind::Composite,
                ToolExposure::ModelVisible,
                gather_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_path_info",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_read_file_segment",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
        ]);
        let dispatcher = dispatcher_for(registry);
        register_gather(&dispatcher);

        let outcome = dispatch(
            &dispatcher,
            "dynamic:gather",
            "gather-start",
            json!({ "path": "demo.rs", "startLine": 80, "lineCount": 40 }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        let payload = parsed_output(&outcome);
        assert_eq!(payload["meta"]["mode"], "file");
        let results = payload["results"].as_array().expect("results array");
        assert_eq!(results[1]["tool"], "workspace_read_file_segment");
        // 嵌套回显真实 startLine——事故根因即模型模仿此处的 `"startLine": 1`。
        assert_eq!(results[1]["arguments"]["startLine"].as_u64(), Some(80));
        let plan_steps = payload["plan"]["steps"].as_array().expect("plan steps");
        let segment_step = plan_steps
            .iter()
            .find(|step| step["name"] == "workspace_read_file_segment")
            .expect("segment step");
        assert_eq!(segment_step["arguments"]["startLine"].as_u64(), Some(80));
    }

    #[test]
    fn gather_context_explicit_start_line_overrides_search_auto_position() {
        let registry = registry(vec![
            dynamic_descriptor(
                "dynamic:gather",
                ToolKind::Composite,
                ToolExposure::ModelVisible,
                gather_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_path_info",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_read_file_segment",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_list_files",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_search_text",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
        ]);
        let dispatcher = dispatcher_for(registry);
        dispatcher.register_composite_handler_with_authority(
            "dynamic:gather",
            vec![
                "builtin:workspace_path_info".to_string(),
                "builtin:workspace_read_file_segment".to_string(),
                "builtin:workspace_list_files".to_string(),
                "builtin:workspace_search_text".to_string(),
            ],
            Arc::new(GatherContextComposite),
        );
        dispatcher.register_handler(
            "builtin:workspace_path_info",
            Arc::new(FakePathInfo { kind: "file" }),
        );
        dispatcher.register_handler("builtin:workspace_read_file_segment", echo_handler());
        dispatcher.register_handler("builtin:workspace_list_files", echo_handler());
        dispatcher.register_handler(
            "builtin:workspace_search_text",
            Arc::new(FakeSearch {
                match_count: 1,
                matches: vec![json!({ "path": "demo.rs", "line": 50, "preview": "needle" })],
            }),
        );

        // 无显式 startLine：自动定位 = max(1, 50 - 40/2) = 30（既有行为）。
        let auto = dispatch(
            &dispatcher,
            "dynamic:gather",
            "gather-auto",
            json!({ "path": "demo.rs", "query": "needle", "lineCount": 40 }),
        );
        let auto_payload = parsed_output(&auto);
        assert_eq!(
            auto_payload["results"][2]["arguments"]["startLine"].as_u64(),
            Some(30)
        );

        // 显式 startLine：全模式生效，覆盖自动定位。
        let explicit = dispatch(
            &dispatcher,
            "dynamic:gather",
            "gather-explicit",
            json!({ "path": "demo.rs", "query": "needle", "lineCount": 40, "startLine": 7 }),
        );
        assert_eq!(explicit.execution_status, ToolExecutionStatus::Ok);
        let payload = parsed_output(&explicit);
        assert_eq!(payload["meta"]["mode"], "search");
        let results = payload["results"].as_array().expect("results array");
        assert_eq!(results[2]["tool"], "workspace_read_file_segment");
        assert_eq!(results[2]["arguments"]["startLine"].as_u64(), Some(7));
    }

    #[test]
    fn gather_context_multi_path_echoes_start_line_in_plan_and_skipped_entries() {
        let registry = registry(vec![
            dynamic_descriptor(
                "dynamic:gather",
                ToolKind::Composite,
                ToolExposure::ModelVisible,
                gather_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_path_info",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
            workspace_primitive_descriptor(
                "workspace_read_file_segment",
                ToolKind::Read,
                ToolExposure::Internal,
                description_required_schema(),
            ),
        ]);
        let dispatcher = dispatcher_for(registry);
        register_gather(&dispatcher);

        let excess_paths = (0..=6)
            .map(|index| Value::String(format!("demo-{index}.rs")))
            .collect::<Vec<_>>();
        let outcome = dispatch(
            &dispatcher,
            "dynamic:gather",
            "gather-multipath-start",
            json!({ "paths": excess_paths, "startLine": 9, "lineCount": 20 }),
        );
        assert_eq!(outcome.execution_status, ToolExecutionStatus::Ok);
        let payload = parsed_output(&outcome);
        let plan_steps = payload["plan"]["steps"].as_array().expect("plan steps");
        for step in plan_steps {
            assert_eq!(step["arguments"]["startLine"].as_u64(), Some(9));
        }
        let results = payload["results"].as_array().expect("results array");
        // 被跳过路径的兜底条目同样回显 startLine。
        assert_eq!(results[6]["arguments"]["startLine"].as_u64(), Some(9));
    }
}
