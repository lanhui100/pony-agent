//! 复合工具面：`workspace_batch` / `workspace_gather_context` 的嵌套计划、
//! 子结果聚合与摘要。
use super::*;
use super::exec::aborted_result;
use super::exec::error_result;
use super::exec::json_string;
use super::exec::parse_tool_output;
use super::paths::unique_paths;
use super::workspace::DEFAULT_LIST_LIMIT;
use super::workspace::DEFAULT_SEGMENT_LINES;
use super::workspace::MAX_SEGMENT_LINES;
use super::workspace::SUMMARY_ITEM_LIMIT;

pub(super) const MAX_WORKSPACE_BATCH_CALLS: usize = 24;

/// Primitives permitted as `workspace_batch` children while the legacy composite
/// still bypasses the governed child dispatcher (PA-076 task 3.5). Fail-closed
/// safety gate per design Decision 3: every other scope is rejected with
/// `unsupported_composite_child` until child dispatch takes over.
pub(super) const READ_ONLY_BATCH_CHILD_PRIMITIVES: &[&str] = &[
    TOOL_WORKSPACE_LIST_FILES,
    TOOL_WORKSPACE_READ_FILE,
    TOOL_WORKSPACE_READ_FILE_SEGMENT,
    TOOL_WORKSPACE_READ_DOCUMENT,
    TOOL_WORKSPACE_PATH_INFO,
    TOOL_WORKSPACE_SEARCH_TEXT,
    TOOL_WORKSPACE_GLOB_FILES,
];

pub(super) const MAX_GATHER_CONTEXT_PATHS: usize = 6;

impl ToolRouter {
    pub(super) fn batch(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let calls = match call.arguments.get("calls").and_then(Value::as_array) {
            Some(value) if !value.is_empty() => value,
            _ => {
                return error_result(
                    TOOL_WORKSPACE_BATCH,
                    "missing_argument",
                    "缺少必填参数 `calls`，且至少需要一个子调用。".to_string(),
                    Some(
                        "参数示例：{\"calls\":[{\"name\":\"workspace_path_info\",\"arguments\":{\"path\":\"src\"}}]}"
                            .to_string(),
                    ),
                )
            }
        };

        if calls.len() > MAX_WORKSPACE_BATCH_CALLS {
            return error_result(
                TOOL_WORKSPACE_BATCH,
                "too_many_calls",
                format!(
                    "单次 workspace_batch 最多允许 {} 个子调用，当前收到 {} 个。",
                    MAX_WORKSPACE_BATCH_CALLS,
                    calls.len()
                ),
                Some("请把批量请求拆小后重试。".to_string()),
            );
        }

        let continue_on_error = call
            .arguments
            .get("continueOnError")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let parallel = call
            .arguments
            .get("parallel")
            .and_then(Value::as_bool)
            .unwrap_or(true)
            && continue_on_error;

        let mut nested_calls = Vec::with_capacity(calls.len());
        for (index, item) in calls.iter().enumerate() {
            let Some(name) = item.get("name").and_then(Value::as_str) else {
                return error_result(
                    TOOL_WORKSPACE_BATCH,
                    "invalid_call_shape",
                    format!("第 {} 个子调用缺少字符串类型的 `name`。", index + 1),
                    Some("每个子调用都需要提供 `name` 和可选 `arguments`。".to_string()),
                );
            };

            let primitive = canonical_tool_name(name);
            let read_only_ok = match primitive {
                Some(primitive) => is_read_only_batch_child_primitive(primitive),
                None => false,
            };
            if !read_only_ok {
                return error_result(
                    TOOL_WORKSPACE_BATCH,
                    "unsupported_composite_child",
                    format!(
                        "workspace_batch 暂不允许执行非只读子调用 `{}`（scope: {}）。",
                        name,
                        composite_child_scope(primitive)
                    ),
                    Some(
                        "请把该子调用拆成独立的顶层工具调用；governed child dispatch 上线前只读子调用才能聚合。"
                            .to_string(),
                    ),
                );
            }

            let arguments = item.get("arguments").cloned().unwrap_or_else(|| json!({}));

            nested_calls.push(ToolCall {
                call_id: None,
                name: name.to_string(),
                arguments,
                plan: None,
            });
        }
        let plan = build_batch_tool_plan(&nested_calls, parallel, continue_on_error);

        let results = if parallel {
            thread::scope(|scope| {
                let mut handles = Vec::with_capacity(nested_calls.len());
                for (index, nested_call) in nested_calls.iter().cloned().enumerate() {
                    let worker_call = nested_call.clone();
                    handles.push((
                        index,
                        nested_call,
                        scope.spawn(move || self.execute_internal(&worker_call, false, context)),
                    ));
                }

                let mut collected = Vec::with_capacity(handles.len());
                for (index, nested_call, handle) in handles {
                    let result = handle.join().unwrap_or_else(|_| {
                        error_result(
                            canonical_tool_name(&nested_call.name).unwrap_or(&nested_call.name),
                            "join_failed",
                            format!("并发执行子调用 `{}` 失败。", nested_call.name),
                            Some(
                                "请改为串行执行，或检查该子调用是否触发了内部 panic。".to_string(),
                            ),
                        )
                    });
                    collected.push((index, nested_call, result));
                }
                collected.sort_by_key(|(index, _, _)| *index);
                collected
            })
        } else {
            let mut collected = Vec::with_capacity(nested_calls.len());
            let mut stop_after_index = None;
            for (index, nested_call) in nested_calls.iter().cloned().enumerate() {
                let result = self.execute_internal(&nested_call, false, context);
                let should_stop = result.status != "ok" && !continue_on_error;
                collected.push((index, nested_call, result));
                if should_stop {
                    stop_after_index = Some(index);
                    break;
                }
            }
            if let Some(failed_index) = stop_after_index {
                for (index, nested_call) in nested_calls
                    .iter()
                    .cloned()
                    .enumerate()
                    .skip(failed_index + 1)
                {
                    collected.push((
                        index,
                        nested_call,
                        aborted_result(
                            TOOL_WORKSPACE_BATCH,
                            "batch_aborted",
                            "前一个子调用失败，且 continueOnError=false，后续子调用未执行。"
                                .to_string(),
                        ),
                    ));
                }
            }
            collected
        };

        self.aggregate_nested_results(
            TOOL_WORKSPACE_BATCH,
            json!({
                "parallel": parallel,
                "continueOnError": continue_on_error,
            }),
            Some(plan),
            results,
        )
    }

    pub(super) fn gather_context(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let raw_path = call
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .trim();
        let query = call
            .arguments
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        let limit = call
            .arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 100) as usize)
            .unwrap_or(DEFAULT_LIST_LIMIT);
        let line_count = call
            .arguments
            .get("lineCount")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, MAX_SEGMENT_LINES as u64) as usize)
            .unwrap_or(DEFAULT_SEGMENT_LINES);
        // PA-100：显式 startLine 全模式生效；未提供时文件模式=1、搜索模式自动定位。
        // T2-A 三态契约：`explicit_start_line` 供现有 `unwrap_or(1)` 链不动（零行为漂移），
        // `start_line_arg` 另行承载 Missing/Valid/Invalid 可观测回显。
        let start_line_arg = parse_start_line_arg(&call.arguments);
        let explicit_start_line = explicit_gather_start_line(&call.arguments);
        let paths = call
            .arguments
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
            let skipped_paths = if requested_path_count > MAX_GATHER_CONTEXT_PATHS {
                paths[MAX_GATHER_CONTEXT_PATHS..].to_vec()
            } else {
                Vec::new()
            };
            let gathered_paths = paths
                .iter()
                .take(MAX_GATHER_CONTEXT_PATHS)
                .cloned()
                .collect::<Vec<_>>();

            let mut nested = gathered_paths
                .iter()
                .enumerate()
                .map(|(index, path)| {
                    let nested_call = ToolCall {
                        call_id: None,
                        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
                        arguments: with_requested_start_line(
                            json!({
                            "path": path,
                            "query": if query.is_empty() { Value::Null } else { Value::String(query.clone()) },
                            "limit": limit,
                            "startLine": explicit_start_line.unwrap_or(1),
                            "lineCount": line_count,
                        }),
                            &start_line_arg,
                        ),
                        plan: None,
                    };
                    let result = self.gather_context(&nested_call, context);
                    (index, nested_call, result)
                })
                .collect::<Vec<_>>();
            if !skipped_paths.is_empty() {
                let skipped_call = ToolCall {
                    call_id: None,
                    name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
                    arguments: with_requested_start_line(
                        json!({
                            "paths": skipped_paths,
                            "limit": limit,
                            "startLine": explicit_start_line.unwrap_or(1),
                            "lineCount": line_count,
                        }),
                        &start_line_arg,
                    ),
                    plan: None,
                };
                nested.push((
                    gathered_paths.len(),
                    skipped_call,
                    ToolResult {
                        tool_name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
                        status: "ok".to_string(),
                        output: json_string(json!({
                            "ok": true,
                            "status": "partial",
                            "reason": "too_many_paths",
                            "message": format!(
                                "单次 workspace_gather_context 最多聚合前 {} 个路径，已跳过其余 {} 个。",
                                MAX_GATHER_CONTEXT_PATHS,
                                requested_path_count.saturating_sub(MAX_GATHER_CONTEXT_PATHS)
                            ),
                            "skippedPaths": skipped_paths,
                        })),
                        duration_ms: 0,
                    },
                ));
            }
            let plan = build_multi_path_gather_plan(
                &gathered_paths,
                &query,
                limit,
                explicit_start_line.unwrap_or(1),
                line_count,
                &start_line_arg,
            );

            return self.aggregate_nested_results(
                TOOL_WORKSPACE_GATHER_CONTEXT,
                with_start_line_fallback_meta(
                    json!({
                        "mode": "multi_path",
                        "paths": gathered_paths,
                        "requestedPathCount": requested_path_count,
                        "skippedPaths": skipped_paths,
                        "limitApplied": requested_path_count > MAX_GATHER_CONTEXT_PATHS,
                        "pathLimit": MAX_GATHER_CONTEXT_PATHS,
                        "query": if query.is_empty() { Value::Null } else { Value::String(query) },
                    }),
                    &start_line_arg,
                ),
                Some(plan),
                nested,
            );
        }

        let ws_id = self.extract_call_workspace_id(call, context);
        let path = if raw_path.is_empty() { "." } else { raw_path };
        let resolved = match self.resolve_workspace_entry(path, context, ws_id) {
            Ok(value) => value,
            Err((code, message)) => {
                return error_result(
                    TOOL_WORKSPACE_GATHER_CONTEXT,
                    &code,
                    message,
                    Some("请提供工作区内存在的相对路径。".to_string()),
                );
            }
        };

        let metadata = match fs::metadata(&resolved) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WORKSPACE_GATHER_CONTEXT,
                    "metadata_failed",
                    format!("读取路径元信息失败：{}。", error),
                    Some("请确认目标路径存在，且当前进程有访问权限。".to_string()),
                )
            }
        };

        let display_path = self.display_resolved_workspace_relative(&resolved, context, ws_id);
        let mode = if !query.is_empty() {
            "search"
        } else if metadata.is_dir() {
            "directory"
        } else {
            "file"
        };

        let nested = match mode {
            "file" => thread::scope(|scope| {
                // PA-100：显式 startLine 生效；未提供时从第 1 行开始（原行为）。
                let file_start_line = explicit_start_line.unwrap_or(1);
                let path_info_call = ToolCall {
                    call_id: None,
                    name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                    arguments: json!({ "path": display_path }),
                    plan: None,
                };
                let segment_call = ToolCall {
                    call_id: None,
                    name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                    arguments: with_requested_start_line(
                        json!({
                            "path": display_path,
                            "startLine": file_start_line,
                            "lineCount": line_count,
                        }),
                        &start_line_arg,
                    ),
                    plan: None,
                };
                let info_handle = scope.spawn(|| {
                    let result = self.path_info(&path_info_call, context);
                    (0usize, path_info_call, result)
                });
                let segment_handle = scope.spawn(|| {
                    let result = self.read_file_segment(&segment_call, context);
                    (1usize, segment_call, result)
                });
                vec![
                    info_handle.join().unwrap_or_else(|_| {
                        (
                            0,
                            ToolCall {
                                call_id: None,
                                name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                                arguments: json!({ "path": display_path }),
                                plan: None,
                            },
                            error_result(
                                TOOL_WORKSPACE_PATH_INFO,
                                "join_failed",
                                "并发执行 workspace_path_info 失败。".to_string(),
                                None,
                            ),
                        )
                    }),
                    segment_handle.join().unwrap_or_else(|_| {
                        (
                            1,
                            ToolCall {
                                call_id: None,
                                name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                                arguments: with_requested_start_line(
                                    json!({
                                        "path": display_path,
                                        "startLine": file_start_line,
                                        "lineCount": line_count,
                                    }),
                                    &start_line_arg,
                                ),
                                plan: None,
                            },
                            error_result(
                                TOOL_WORKSPACE_READ_FILE_SEGMENT,
                                "join_failed",
                                "并发执行 workspace_read_file_segment 失败。".to_string(),
                                None,
                            ),
                        )
                    }),
                ]
            }),
            "directory" => thread::scope(|scope| {
                let path_info_call = ToolCall {
                    call_id: None,
                    name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                    arguments: json!({ "path": display_path }),
                    plan: None,
                };
                let list_call = ToolCall {
                    call_id: None,
                    name: TOOL_WORKSPACE_LIST_FILES.to_string(),
                    arguments: json!({
                        "path": display_path,
                        "limit": limit,
                    }),
                    plan: None,
                };
                let info_handle = scope.spawn(|| {
                    let result = self.path_info(&path_info_call, context);
                    (0usize, path_info_call, result)
                });
                let list_handle = scope.spawn(|| {
                    let result = self.list_files(&list_call, context);
                    (1usize, list_call, result)
                });
                vec![
                    info_handle.join().unwrap_or_else(|_| {
                        (
                            0,
                            ToolCall {
                                call_id: None,
                                name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                                arguments: json!({ "path": display_path }),
                                plan: None,
                            },
                            error_result(
                                TOOL_WORKSPACE_PATH_INFO,
                                "join_failed",
                                "并发执行 workspace_path_info 失败。".to_string(),
                                None,
                            ),
                        )
                    }),
                    list_handle.join().unwrap_or_else(|_| {
                        (
                            1,
                            ToolCall {
                                call_id: None,
                                name: TOOL_WORKSPACE_LIST_FILES.to_string(),
                                arguments: json!({
                                    "path": display_path,
                                    "limit": limit,
                                }),
                                plan: None,
                            },
                            error_result(
                                TOOL_WORKSPACE_LIST_FILES,
                                "join_failed",
                                "并发执行 workspace_list_files 失败。".to_string(),
                                None,
                            ),
                        )
                    }),
                ]
            }),
            _ => {
                let search_path = if metadata.is_file() {
                    resolved
                        .parent()
                        .map(|value| self.display_workspace_relative(value))
                        .unwrap_or_else(|| ".".to_string())
                } else {
                    display_path.clone()
                };
                let file_pattern = if metadata.is_file() {
                    resolved
                        .file_name()
                        .map(|value| value.to_string_lossy().to_string())
                } else {
                    None
                };

                if metadata.is_file() {
                    let path_info_call = ToolCall {
                        call_id: None,
                        name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                        arguments: json!({ "path": display_path }),
                        plan: None,
                    };
                    let search_call = ToolCall {
                        call_id: None,
                        name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
                        arguments: json!({
                            "query": query,
                            "path": search_path,
                            "limit": limit,
                            "filePattern": file_pattern,
                        }),
                        plan: None,
                    };

                    let mut collected = Vec::with_capacity(3);
                    collected.push((
                        0usize,
                        path_info_call.clone(),
                        self.path_info(&path_info_call, context),
                    ));
                    let search_result = self.search_text(&search_call, context);
                    let segment_line =
                        first_search_match_line(&search_result.output, &display_path);
                    collected.push((1usize, search_call, search_result));

                    // PA-100：显式 startLine 优先；未提供时才按搜索命中自动定位。
                    // T2-B F1 语义B：Invalid 强制 1 并跳过搜索自动定位（与 file 一致）；
                    // Valid 保持 explicit 优先，Missing 才走 auto。
                    let start_line = match &start_line_arg {
                        StartLineArg::Invalid { .. } => 1,
                        _ => explicit_start_line
                            .or_else(|| {
                                segment_line.map(|line| line.saturating_sub(line_count / 2).max(1))
                            })
                            .unwrap_or(1),
                    };
                    let segment_call = ToolCall {
                        call_id: None,
                        name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                        arguments: with_requested_start_line(
                            json!({
                                "path": display_path,
                                "startLine": start_line,
                                "lineCount": line_count,
                            }),
                            &start_line_arg,
                        ),
                        plan: None,
                    };
                    collected.push((
                        2usize,
                        segment_call.clone(),
                        self.read_file_segment(&segment_call, context),
                    ));

                    collected
                } else {
                    let path_info_call = ToolCall {
                        call_id: None,
                        name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                        arguments: json!({ "path": display_path }),
                        plan: None,
                    };
                    let search_call = ToolCall {
                        call_id: None,
                        name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
                        arguments: json!({
                            "query": query,
                            "path": search_path,
                            "limit": limit,
                            "filePattern": file_pattern,
                        }),
                        plan: None,
                    };

                    let mut collected = Vec::with_capacity(3);
                    collected.push((
                        0usize,
                        path_info_call.clone(),
                        self.path_info(&path_info_call, context),
                    ));
                    let search_result = self.search_text(&search_call, context);
                    let should_add_listing = search_result.status != "ok"
                        || search_match_count(&search_result.output) == 0;
                    collected.push((1usize, search_call, search_result));

                    if should_add_listing {
                        let list_call = ToolCall {
                            call_id: None,
                            name: TOOL_WORKSPACE_LIST_FILES.to_string(),
                            arguments: json!({
                                "path": display_path,
                                "limit": limit,
                            }),
                            plan: None,
                        };
                        collected.push((
                            2usize,
                            list_call.clone(),
                            self.list_files(&list_call, context),
                        ));
                    }

                    collected
                }
            }
        };
        let plan = build_nested_tool_plan(
            TOOL_WORKSPACE_GATHER_CONTEXT,
            mode,
            &display_path,
            query.as_str(),
            &nested,
        );

        // T2-B F1：directory 模式 startLine 未消费，双实现统一不打标；
        // file/search 才打标（composite directory 分支本来就不加，此处镜像同步）。
        let gather_meta = {
            let base = json!({
                "mode": mode,
                "path": display_path,
                "query": if query.is_empty() { Value::Null } else { Value::String(query) },
            });
            if mode == "directory" {
                base
            } else {
                with_start_line_fallback_meta(base, &start_line_arg)
            }
        };
        self.aggregate_nested_results(
            TOOL_WORKSPACE_GATHER_CONTEXT,
            gather_meta,
            Some(plan),
            nested,
        )
    }

    pub(super) fn aggregate_nested_results(
        &self,
        tool_name: &str,
        meta: Value,
        plan: Option<ToolPlan>,
        mut nested: Vec<(usize, ToolCall, ToolResult)>,
    ) -> ToolResult {
        nested.sort_by_key(|(index, _, _)| *index);

        let success_count = nested
            .iter()
            .filter(|(_, _, result)| nested_result_status(result) == "ok")
            .count();
        let partial_count = nested
            .iter()
            .filter(|(_, _, result)| nested_result_status(result) == "partial")
            .count();
        let aborted_count = nested
            .iter()
            .filter(|(_, _, result)| nested_result_status(result) == "aborted")
            .count();
        let error_count = nested
            .len()
            .saturating_sub(success_count + partial_count + aborted_count);
        let aggregate_status = if error_count == 0 && partial_count == 0 && aborted_count == 0 {
            "ok"
        } else if success_count > 0 || partial_count > 0 {
            "partial"
        } else {
            "error"
        };

        let results = nested
            .into_iter()
            .map(|(index, nested_call, result)| {
                let output = parse_tool_output(&result.output);
                let aggregate_status = nested_output_status(&result, &output).to_string();
                json!({
                    "index": index,
                    "tool": nested_call.name,
                    "canonicalTool": canonical_tool_name(&nested_call.name).unwrap_or(&nested_call.name),
                    "arguments": nested_call.arguments,
                    "status": result.status,
                    "aggregateStatus": aggregate_status,
                    "ok": aggregate_status == "ok",
                    "durationMs": result.duration_ms,
                    "error": output.get("error").cloned(),
                    "output": output,
                })
            })
            .collect::<Vec<_>>();

        let runtime_status = if aggregate_status == "error" {
            "error"
        } else {
            "ok"
        };
        let summary = build_nested_results_summary(tool_name, &results, aggregate_status);

        ToolResult {
            tool_name: tool_name.to_string(),
            status: runtime_status.to_string(),
            output: json_string(json!({
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
                "results": results,
            })),
            duration_ms: 0,
        }
    }
}


pub(super) fn nested_result_status(result: &ToolResult) -> String {
    let output = parse_tool_output(&result.output);
    nested_output_status(result, &output)
}

pub(super) fn nested_output_status(result: &ToolResult, output: &Value) -> String {
    if result.status == "aborted" {
        return "aborted".to_string();
    }
    if result.status != "ok" {
        return "error".to_string();
    }
    match output.get("status").and_then(Value::as_str) {
        Some("partial") => "partial".to_string(),
        Some("error") => "error".to_string(),
        _ => "ok".to_string(),
    }
}

pub(super) fn first_search_match_line(output: &str, target_path: &str) -> Option<usize> {
    let parsed = parse_tool_output(output);
    parsed
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

pub(super) fn search_match_count(output: &str) -> usize {
    parse_tool_output(output)
        .get("matchCount")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize
}

pub(super) fn build_nested_results_summary(
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

pub(super) fn is_read_only_batch_child_primitive(primitive: &str) -> bool {
    READ_ONLY_BATCH_CHILD_PRIMITIVES.contains(&primitive)
}

/// Human-readable permission scope for a `workspace_batch` child, used in the
/// fail-closed `unsupported_composite_child` rejection. Unknown (non-builtin)
/// names fail closed as `external` instead of being executed.
pub(super) fn composite_child_scope(primitive: Option<&'static str>) -> &'static str {
    match primitive {
        Some(TOOL_WORKSPACE_WRITE_FILE | TOOL_WORKSPACE_EDIT_FILE) => "workspace.write",
        Some(TOOL_WORKSPACE_RUN_COMMAND) => "workspace.execute",
        Some(TOOL_WEB_FETCH_URL | TOOL_WEB_SEARCH_QUERY) => "web",
        Some(TOOL_MCP_RESOURCE_READ) => "mcp",
        Some(TOOL_TOOL_SEARCH) => "capability.discovery",
        Some(TOOL_WORKSPACE_BATCH | TOOL_WORKSPACE_GATHER_CONTEXT) => "composite",
        Some(TOOL_ECHO_INPUT) => "interactive",
        Some(TOOL_TIME_NOW) => "workspace.execute",
        Some(_) => "workspace.read",
        None => "external",
    }
}

pub(super) fn build_batch_tool_plan(
    nested_calls: &[ToolCall],
    parallel: bool,
    continue_on_error: bool,
) -> ToolPlan {
    ToolPlan {
        kind: "batch".to_string(),
        summary: format!(
            "workspace_batch 计划执行 {} 个子调用{}{}。",
            nested_calls.len(),
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
        steps: nested_calls
            .iter()
            .enumerate()
            .map(|(index, call)| ToolPlanStep {
                name: canonical_tool_name(&call.name)
                    .unwrap_or(&call.name)
                    .to_string(),
                arguments: call.arguments.clone(),
                summary: format!("第 {} 个子调用：`{}`。", index + 1, call.name),
            })
            .collect(),
    }
}

pub(super) fn build_multi_path_gather_plan(
    paths: &[String],
    query: &str,
    limit: usize,
    start_line: usize,
    line_count: usize,
    start_line_arg: &StartLineArg,
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
                arguments: with_requested_start_line(
                    json!({
                    "path": path,
                    "query": if query.trim().is_empty() { Value::Null } else { Value::String(query.to_string()) },
                    "limit": limit,
                    "startLine": start_line,
                    "lineCount": line_count,
                }),
                    start_line_arg,
                ),
                summary: format!("第 {} 个路径聚合：`{}`。", index + 1, path),
            })
            .collect(),
    }
}

pub(super) fn build_nested_tool_plan(
    tool_name: &str,
    mode: &str,
    display_path: &str,
    query: &str,
    nested: &[(usize, ToolCall, ToolResult)],
) -> ToolPlan {
    let steps = nested
        .iter()
        .map(|(index, call, _)| ToolPlanStep {
            name: canonical_tool_name(&call.name)
                .unwrap_or(&call.name)
                .to_string(),
            arguments: call.arguments.clone(),
            summary: format!("第 {} 个子调用：`{}`。", index + 1, call.name),
        })
        .collect::<Vec<_>>();

    ToolPlan {
        kind: tool_name.to_string(),
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

pub(super) fn extract_first_error_from_nested_result(result: &Value) -> Option<Value> {
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

pub(super) fn extract_top_matches_from_nested_result(result: &Value) -> Vec<Value> {
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
