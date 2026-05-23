use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

const TOOL_TIME_NOW: &str = "time_now";
const TOOL_ECHO_INPUT: &str = "echo_input";
const TOOL_WORKSPACE_LIST_FILES: &str = "workspace_list_files";
const TOOL_WORKSPACE_READ_FILE: &str = "workspace_read_file";
const TOOL_WORKSPACE_READ_FILE_SEGMENT: &str = "workspace_read_file_segment";
const TOOL_WORKSPACE_PATH_INFO: &str = "workspace_path_info";
const TOOL_WORKSPACE_SEARCH_TEXT: &str = "workspace_search_text";
const TOOL_WORKSPACE_BATCH: &str = "workspace_batch";
const TOOL_WORKSPACE_GATHER_CONTEXT: &str = "workspace_gather_context";

const MAX_FULL_READ_BYTES: u64 = 120_000;
const MAX_SEARCH_FILE_BYTES: u64 = 1_000_000;
const MAX_SEARCH_FILES: usize = 800;
const MAX_BATCH_CALLS: usize = 6;
const MAX_SEGMENT_LINES: usize = 400;
const DEFAULT_SEGMENT_LINES: usize = 80;
const DEFAULT_LIST_LIMIT: usize = 40;
const SUMMARY_ITEM_LIMIT: usize = 3;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub call_id: Option<String>,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_name: String,
    pub status: String,
    pub output: String,
    pub duration_ms: u64,
}

pub trait ToolExecutor: Send {
    fn execute(&self, call: &ToolCall) -> ToolResult;
}

pub struct ToolRouter {
    workspace_root: PathBuf,
}

impl ToolRouter {
    pub fn new() -> Self {
        Self {
            workspace_root: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
        }
    }

    #[cfg(test)]
    pub fn with_workspace_root(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    pub fn execute(&self, call: &ToolCall) -> ToolResult {
        let started_at = Instant::now();
        let mut result = self.execute_internal(call, true);
        result.duration_ms = started_at.elapsed().as_millis() as u64;
        result
    }

    fn execute_internal(&self, call: &ToolCall, allow_batch: bool) -> ToolResult {
        match canonical_tool_name(&call.name) {
            Some(TOOL_TIME_NOW) => self.time_now(),
            Some(TOOL_ECHO_INPUT) => self.echo_input(call),
            Some(TOOL_WORKSPACE_LIST_FILES) => self.list_files(call),
            Some(TOOL_WORKSPACE_READ_FILE) => self.read_file(call),
            Some(TOOL_WORKSPACE_READ_FILE_SEGMENT) => self.read_file_segment(call),
            Some(TOOL_WORKSPACE_PATH_INFO) => self.path_info(call),
            Some(TOOL_WORKSPACE_SEARCH_TEXT) => self.search_text(call),
            Some(TOOL_WORKSPACE_GATHER_CONTEXT) => self.gather_context(call),
            Some(TOOL_WORKSPACE_BATCH) if allow_batch => self.batch(call),
            Some(TOOL_WORKSPACE_BATCH) => error_result(
                TOOL_WORKSPACE_BATCH,
                "nested_batch_not_allowed",
                "workspace_batch 不允许递归调用 workspace_batch。".to_string(),
                Some("请把嵌套批量调用拆成多个叶子工具调用。".to_string()),
            ),
            _ => error_result(
                &call.name,
                "unsupported_tool",
                format!("当前 runtime 尚未实现工具 `{}`。", call.name),
                Some("请改用 list_available_tools 中返回的工具名。".to_string()),
            ),
        }
    }

    fn time_now(&self) -> ToolResult {
        let unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);

        ToolResult {
            tool_name: TOOL_TIME_NOW.to_string(),
            status: "ok".to_string(),
            output: format!(
                "当前 UNIX 时间戳（秒）是 {}。如需本地格式化时间，可在下一阶段补充 chrono/time 支持。",
                unix_seconds
            ),
            duration_ms: 0,
        }
    }

    fn echo_input(&self, call: &ToolCall) -> ToolResult {
        let text = call
            .arguments
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();

        if text.is_empty() {
            return error_result(
                TOOL_ECHO_INPUT,
                "missing_argument",
                "缺少必填参数 `text`。".to_string(),
                Some("参数示例：{\"text\":\"hello\"}".to_string()),
            );
        }

        ToolResult {
            tool_name: TOOL_ECHO_INPUT.to_string(),
            status: "ok".to_string(),
            output: format!("echo_input 返回：{}", text),
            duration_ms: 0,
        }
    }

    fn read_file(&self, call: &ToolCall) -> ToolResult {
        let Some(path) = call.arguments.get("path").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_READ_FILE,
                "missing_argument",
                "缺少必填参数 `path`。".to_string(),
                Some("参数示例：{\"path\":\"src-tauri/src/agent/tools.rs\"}".to_string()),
            );
        };

        let resolved = match self.resolve_workspace_path(path) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WORKSPACE_READ_FILE, "invalid_path", error, None)
            }
        };

        let metadata = match fs::metadata(&resolved) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WORKSPACE_READ_FILE,
                    "metadata_failed",
                    format!("读取文件元信息失败：{}。", error),
                    Some("请确认目标文件存在，且当前进程有读取权限。".to_string()),
                )
            }
        };

        if metadata.len() > MAX_FULL_READ_BYTES {
            return error_result(
                TOOL_WORKSPACE_READ_FILE,
                "file_too_large",
                format!(
                    "文件 {} 大小为 {} bytes，超过整文件读取上限 {} bytes。",
                    self.display_workspace_relative(&resolved),
                    metadata.len(),
                    MAX_FULL_READ_BYTES
                ),
                Some(
                    "请改用 workspace_read_file_segment，或用 workspace_gather_context 获取概览。"
                        .to_string(),
                ),
            );
        }

        match fs::read_to_string(&resolved) {
            Ok(content) => ToolResult {
                tool_name: TOOL_WORKSPACE_READ_FILE.to_string(),
                status: "ok".to_string(),
                output: format!(
                    "文件 {} 读取成功。\n\n{}",
                    self.display_workspace_relative(&resolved),
                    truncate_preview(&content, 4000)
                ),
                duration_ms: 0,
            },
            Err(error) => error_result(
                TOOL_WORKSPACE_READ_FILE,
                "read_failed",
                format!("读取文件失败：{}。", error),
                Some(
                    "请确认目标文件是 UTF-8 文本，或改用 workspace_path_info 判断文件类型。"
                        .to_string(),
                ),
            ),
        }
    }

    fn read_file_segment(&self, call: &ToolCall) -> ToolResult {
        let Some(path) = call.arguments.get("path").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_READ_FILE_SEGMENT,
                "missing_argument",
                "缺少必填参数 `path`。".to_string(),
                Some(
                    "参数示例：{\"path\":\"src/main.rs\",\"startLine\":1,\"lineCount\":40}"
                        .to_string(),
                ),
            );
        };

        let start_line = call
            .arguments
            .get("startLine")
            .and_then(Value::as_u64)
            .map(|value| value.max(1) as usize)
            .unwrap_or(1);
        let line_count = call
            .arguments
            .get("lineCount")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, MAX_SEGMENT_LINES as u64) as usize)
            .unwrap_or(40);

        let resolved = match self.resolve_workspace_path(path) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WORKSPACE_READ_FILE_SEGMENT,
                    "invalid_path",
                    error,
                    None,
                )
            }
        };

        match read_file_lines(&resolved, start_line, line_count) {
            Ok(FileSegment::Empty) => ToolResult {
                tool_name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                status: "ok".to_string(),
                output: format!("文件 {} 为空。", self.display_workspace_relative(&resolved)),
                duration_ms: 0,
            },
            Ok(FileSegment::Range {
                start_line,
                end_line,
                lines,
                total_lines,
            }) => {
                let segment = lines
                    .iter()
                    .map(|(line_number, line)| format!("{:>4} | {}", line_number, line))
                    .collect::<Vec<_>>()
                    .join("\n");

                ToolResult {
                    tool_name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                    status: "ok".to_string(),
                    output: format!(
                        "文件 {} 第 {} 行到第 {} 行（总行数约 {}）：\n{}",
                        self.display_workspace_relative(&resolved),
                        start_line,
                        end_line,
                        total_lines,
                        segment
                    ),
                    duration_ms: 0,
                }
            }
            Err(ReadSegmentError::StartOutOfRange { total_lines }) => error_result(
                TOOL_WORKSPACE_READ_FILE_SEGMENT,
                "line_out_of_range",
                format!("起始行 {} 超出文件总行数 {}。", start_line, total_lines),
                Some(
                    "请缩小 startLine，或先用 workspace_path_info / workspace_gather_context 查看文件概况。"
                        .to_string(),
                ),
            ),
            Err(ReadSegmentError::Io(error)) => error_result(
                TOOL_WORKSPACE_READ_FILE_SEGMENT,
                "read_failed",
                format!("读取文件失败：{}。", error),
                Some("请确认目标文件是 UTF-8 文本，且当前进程有读取权限。".to_string()),
            ),
        }
    }

    fn list_files(&self, call: &ToolCall) -> ToolResult {
        let relative_dir = call
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .trim();
        let limit = call
            .arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 200) as usize)
            .unwrap_or(DEFAULT_LIST_LIMIT);

        let dir = match self.resolve_workspace_dir(relative_dir) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WORKSPACE_LIST_FILES, "invalid_path", error, None)
            }
        };

        match fs::read_dir(&dir) {
            Ok(entries) => {
                let mut items = entries
                    .filter_map(Result::ok)
                    .map(|entry| {
                        let path = entry.path();
                        let label = self.display_workspace_relative(&path);
                        if path.is_dir() {
                            format!("{}/", label)
                        } else {
                            label
                        }
                    })
                    .collect::<Vec<_>>();
                items.sort();

                let total = items.len();
                let preview = items.into_iter().take(limit).collect::<Vec<_>>();
                ToolResult {
                    tool_name: TOOL_WORKSPACE_LIST_FILES.to_string(),
                    status: "ok".to_string(),
                    output: format!(
                        "目录 {} 下共发现 {} 个条目，当前展示前 {} 个：\n{}",
                        self.display_workspace_relative(&dir),
                        total,
                        preview.len(),
                        preview.join("\n")
                    ),
                    duration_ms: 0,
                }
            }
            Err(error) => error_result(
                TOOL_WORKSPACE_LIST_FILES,
                "read_failed",
                format!("读取目录失败：{}。", error),
                Some("请确认目标目录存在，且当前进程有访问权限。".to_string()),
            ),
        }
    }

    fn path_info(&self, call: &ToolCall) -> ToolResult {
        let relative_path = call
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .trim();

        let path = match self.resolve_workspace_entry(relative_path) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WORKSPACE_PATH_INFO, "invalid_path", error, None)
            }
        };

        match fs::metadata(&path) {
            Ok(metadata) => {
                let path_type = if metadata.is_dir() {
                    "directory"
                } else if metadata.is_file() {
                    "file"
                } else {
                    "other"
                };
                let child_count = if metadata.is_dir() {
                    fs::read_dir(&path).ok().map(|entries| entries.count())
                } else {
                    None
                };
                let modified_unix = metadata
                    .modified()
                    .ok()
                    .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
                    .map(|value| value.as_secs());

                ToolResult {
                    tool_name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                    status: "ok".to_string(),
                    output: json_string(json!({
                        "path": self.display_workspace_relative(&path),
                        "absolutePath": path.display().to_string(),
                        "kind": path_type,
                        "sizeBytes": metadata.len(),
                        "modifiedUnixSeconds": modified_unix,
                        "childCount": child_count,
                        "isReadableTextHint": metadata.is_file(),
                    })),
                    duration_ms: 0,
                }
            }
            Err(error) => error_result(
                TOOL_WORKSPACE_PATH_INFO,
                "metadata_failed",
                format!("读取路径元信息失败：{}。", error),
                Some("请确认目标路径存在，且当前进程有访问权限。".to_string()),
            ),
        }
    }

    fn search_text(&self, call: &ToolCall) -> ToolResult {
        let Some(query) = call.arguments.get("query").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_SEARCH_TEXT,
                "missing_argument",
                "缺少必填参数 `query`。".to_string(),
                Some(
                    "参数示例：{\"query\":\"ToolRouter\",\"path\":\"src-tauri/src\",\"limit\":20}"
                        .to_string(),
                ),
            );
        };

        let query = query.trim();
        if query.is_empty() {
            return error_result(
                TOOL_WORKSPACE_SEARCH_TEXT,
                "empty_query",
                "参数 `query` 不能为空字符串。".to_string(),
                Some("请提供要搜索的关键字或文本片段。".to_string()),
            );
        }

        let relative_dir = call
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .trim();
        let limit = call
            .arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 100) as usize)
            .unwrap_or(20);
        let ignore_case = call
            .arguments
            .get("ignoreCase")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let file_filter = call
            .arguments
            .get("filePattern")
            .and_then(Value::as_str)
            .map(|value| value.trim().to_lowercase())
            .filter(|value| !value.is_empty());

        let root_entry = match self.resolve_workspace_entry(relative_dir) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WORKSPACE_SEARCH_TEXT, "invalid_path", error, None)
            }
        };

        let searched_path = self.display_workspace_relative(&root_entry);
        let path_kind = if root_entry.is_file() {
            "file"
        } else if root_entry.is_dir() {
            "directory"
        } else {
            "other"
        };

        let mut files = if root_entry.is_file() {
            vec![root_entry.clone()]
        } else if root_entry.is_dir() {
            let mut collected = Vec::new();
            if let Err(error) =
                collect_files_recursively(&root_entry, &mut collected, MAX_SEARCH_FILES)
            {
                return error_result(
                    TOOL_WORKSPACE_SEARCH_TEXT,
                    "walk_failed",
                    format!("遍历目录失败：{}。", error),
                    Some("请缩小 path 范围后重试。".to_string()),
                );
            }
            collected
        } else {
            return error_result(
                TOOL_WORKSPACE_SEARCH_TEXT,
                "unsupported_path_kind",
                format!("当前路径类型不支持文本搜索：{}。", searched_path),
                Some("请传入工作区内的文件或目录路径。".to_string()),
            );
        };
        files.sort();

        let normalized_query = if ignore_case {
            query.to_lowercase()
        } else {
            query.to_string()
        };

        let mut matches = Vec::new();
        let mut scanned_files = 0usize;
        let mut skipped_unreadable = 0usize;
        let mut skipped_large = 0usize;
        let mut skipped_by_budget = 0usize;

        for file_path in files {
            if matches.len() >= limit {
                break;
            }

            let relative = self.display_workspace_relative(&file_path);
            if let Some(pattern) = &file_filter {
                if !path_matches_filter(&relative, pattern) {
                    continue;
                }
            }

            let Ok(metadata) = fs::metadata(&file_path) else {
                skipped_unreadable += 1;
                continue;
            };
            if metadata.len() > MAX_SEARCH_FILE_BYTES {
                skipped_large += 1;
                continue;
            }

            scanned_files += 1;
            let Ok(content) = fs::read_to_string(&file_path) else {
                skipped_unreadable += 1;
                continue;
            };

            for (index, line) in content.lines().enumerate() {
                let haystack = if ignore_case {
                    line.to_lowercase()
                } else {
                    line.to_string()
                };
                if haystack.contains(&normalized_query) {
                    matches.push(json!({
                        "path": relative,
                        "line": index + 1,
                        "preview": preview_text(line, 160),
                    }));
                    if matches.len() >= limit {
                        skipped_by_budget += 1;
                        break;
                    }
                }
            }
        }

        ToolResult {
            tool_name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
            status: "ok".to_string(),
            output: json_string(json!({
                "query": query,
                "path": searched_path,
                "pathKind": path_kind,
                "ignoreCase": ignore_case,
                "filePattern": file_filter,
                "scannedFiles": scanned_files,
                "skippedUnreadableFiles": skipped_unreadable,
                "skippedLargeFiles": skipped_large,
                "skippedByBudget": skipped_by_budget,
                "matchCount": matches.len(),
                "matches": matches,
            })),
            duration_ms: 0,
        }
    }

    fn batch(&self, call: &ToolCall) -> ToolResult {
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

        if calls.len() > MAX_BATCH_CALLS {
            return error_result(
                TOOL_WORKSPACE_BATCH,
                "too_many_calls",
                format!(
                    "单次 workspace_batch 最多允许 {} 个子调用，当前收到 {} 个。",
                    MAX_BATCH_CALLS,
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

            if canonical_tool_name(name) == Some(TOOL_WORKSPACE_BATCH) {
                return error_result(
                    TOOL_WORKSPACE_BATCH,
                    "nested_batch_not_allowed",
                    "workspace_batch 不允许递归调用自身。".to_string(),
                    Some("请把嵌套批量调用展开为普通子调用。".to_string()),
                );
            }

            let arguments = item.get("arguments").cloned().unwrap_or_else(|| json!({}));

            nested_calls.push(ToolCall {
                call_id: None,
                name: name.to_string(),
                arguments,
            });
        }

        let results = if parallel {
            thread::scope(|scope| {
                let mut handles = Vec::with_capacity(nested_calls.len());
                for (index, nested_call) in nested_calls.iter().cloned().enumerate() {
                    let worker_call = nested_call.clone();
                    handles.push((
                        index,
                        nested_call,
                        scope.spawn(move || self.execute_internal(&worker_call, false)),
                    ));
                }

                let mut collected = Vec::with_capacity(handles.len());
                for (index, nested_call, handle) in handles {
                    let result = handle.join().unwrap_or_else(|_| {
                        error_result(
                            canonical_tool_name(&nested_call.name).unwrap_or(&nested_call.name),
                            "join_failed",
                            format!("并发执行子调用 `{}` 失败。", nested_call.name),
                            Some("请改为串行执行，或检查该子调用是否触发了内部 panic。".to_string()),
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
                let result = self.execute_internal(&nested_call, false);
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
            results,
        )
    }

    fn gather_context(&self, call: &ToolCall) -> ToolResult {
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
            if paths.len() > MAX_BATCH_CALLS {
                return error_result(
                    TOOL_WORKSPACE_GATHER_CONTEXT,
                    "too_many_paths",
                    format!(
                        "单次 workspace_gather_context 最多允许 {} 个路径，当前收到 {} 个。",
                        MAX_BATCH_CALLS,
                        paths.len()
                    ),
                    Some("请缩小本次聚合范围，或拆成多个 gather/batch 调用。".to_string()),
                );
            }

            let nested = paths
                .iter()
                .enumerate()
                .map(|(index, path)| {
                    let nested_call = ToolCall {
                        call_id: None,
                        name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
                        arguments: json!({
                            "path": path,
                            "query": if query.is_empty() { Value::Null } else { Value::String(query.clone()) },
                            "limit": limit,
                            "lineCount": line_count,
                        }),
                    };
                    let result = self.gather_context(&nested_call);
                    (index, nested_call, result)
                })
                .collect::<Vec<_>>();

            return self.aggregate_nested_results(
                TOOL_WORKSPACE_GATHER_CONTEXT,
                json!({
                    "mode": "multi_path",
                    "paths": paths,
                    "query": if query.is_empty() { Value::Null } else { Value::String(query) },
                }),
                nested,
            );
        }

        let path = if raw_path.is_empty() { "." } else { raw_path };
        let resolved = match self.resolve_workspace_entry(path) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WORKSPACE_GATHER_CONTEXT,
                    "invalid_path",
                    error,
                    Some("请提供工作区内存在的相对路径。".to_string()),
                )
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

        let display_path = self.display_workspace_relative(&resolved);
        let mode = if !query.is_empty() {
            "search"
        } else if metadata.is_dir() {
            "directory"
        } else {
            "file"
        };

        let nested = match mode {
            "file" => thread::scope(|scope| {
                let path_info_call = ToolCall {
                    call_id: None,
                    name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                    arguments: json!({ "path": display_path }),
                };
                let segment_call = ToolCall {
                    call_id: None,
                    name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                    arguments: json!({
                        "path": display_path,
                        "startLine": 1,
                        "lineCount": line_count,
                    }),
                };
                let info_handle = scope.spawn(|| {
                    let result = self.path_info(&path_info_call);
                    (0usize, path_info_call, result)
                });
                let segment_handle = scope.spawn(|| {
                    let result = self.read_file_segment(&segment_call);
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
                                arguments: json!({
                                    "path": display_path,
                                    "startLine": 1,
                                    "lineCount": line_count,
                                }),
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
                };
                let list_call = ToolCall {
                    call_id: None,
                    name: TOOL_WORKSPACE_LIST_FILES.to_string(),
                    arguments: json!({
                        "path": display_path,
                        "limit": limit,
                    }),
                };
                let info_handle = scope.spawn(|| {
                    let result = self.path_info(&path_info_call);
                    (0usize, path_info_call, result)
                });
                let list_handle = scope.spawn(|| {
                    let result = self.list_files(&list_call);
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
                    };

                    let mut collected = Vec::with_capacity(3);
                    collected.push((
                        0usize,
                        path_info_call.clone(),
                        self.path_info(&path_info_call),
                    ));
                    let search_result = self.search_text(&search_call);
                    let segment_line =
                        first_search_match_line(&search_result.output, &display_path);
                    collected.push((1usize, search_call, search_result));

                    let start_line = segment_line
                        .map(|line| line.saturating_sub(line_count / 2).max(1))
                        .unwrap_or(1);
                    let segment_call = ToolCall {
                        call_id: None,
                        name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                        arguments: json!({
                            "path": display_path,
                            "startLine": start_line,
                            "lineCount": line_count,
                        }),
                    };
                    collected.push((
                        2usize,
                        segment_call.clone(),
                        self.read_file_segment(&segment_call),
                    ));

                    collected
                } else {
                    let path_info_call = ToolCall {
                        call_id: None,
                        name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                        arguments: json!({ "path": display_path }),
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
                    };

                    let mut collected = Vec::with_capacity(3);
                    collected.push((
                        0usize,
                        path_info_call.clone(),
                        self.path_info(&path_info_call),
                    ));
                    let search_result = self.search_text(&search_call);
                    let should_add_listing =
                        search_result.status != "ok" || search_match_count(&search_result.output) == 0;
                    collected.push((1usize, search_call, search_result));

                    if should_add_listing {
                        let list_call = ToolCall {
                            call_id: None,
                            name: TOOL_WORKSPACE_LIST_FILES.to_string(),
                            arguments: json!({
                                "path": display_path,
                                "limit": limit,
                            }),
                        };
                        collected.push((2usize, list_call.clone(), self.list_files(&list_call)));
                    }

                    collected
                }
            }
        };

        self.aggregate_nested_results(
            TOOL_WORKSPACE_GATHER_CONTEXT,
            json!({
                "mode": mode,
                "path": display_path,
                "query": if query.is_empty() { Value::Null } else { Value::String(query) },
            }),
            nested,
        )
    }

    fn aggregate_nested_results(
        &self,
        tool_name: &str,
        meta: Value,
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

        let runtime_status = if success_count > 0 || partial_count > 0 || error_count == 0 {
            "ok"
        } else {
            "error"
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
                "summary": summary,
                "results": results,
            })),
            duration_ms: 0,
        }
    }

    fn resolve_workspace_path(&self, raw_path: &str) -> Result<PathBuf, String> {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return Err("文件路径不能为空。".to_string());
        }

        let canonical = self.canonicalize_workspace_target(trimmed)?;
        if !canonical.is_file() {
            return Err(format!(
                "目标不是文件：{}。",
                self.display_workspace_relative(&canonical)
            ));
        }

        Ok(canonical)
    }

    fn resolve_workspace_entry(&self, raw_path: &str) -> Result<PathBuf, String> {
        let trimmed = if raw_path.trim().is_empty() {
            "."
        } else {
            raw_path.trim()
        };
        self.canonicalize_workspace_target(trimmed)
    }

    fn resolve_workspace_dir(&self, raw_path: &str) -> Result<PathBuf, String> {
        let trimmed = if raw_path.trim().is_empty() {
            "."
        } else {
            raw_path.trim()
        };
        let canonical = self.canonicalize_workspace_target(trimmed)?;
        if !canonical.is_dir() {
            return Err(format!(
                "目标不是目录：{}。",
                self.display_workspace_relative(&canonical)
            ));
        }

        Ok(canonical)
    }

    fn canonicalize_workspace_target(&self, raw_path: &str) -> Result<PathBuf, String> {
        let input = PathBuf::from(raw_path);
        let candidate = if input.is_absolute() {
            input
        } else {
            self.workspace_root.join(raw_path)
        };
        let canonical = candidate
            .canonicalize()
            .map_err(|error| format!("无法解析路径 {}：{}", raw_path, error))?;
        let root = self.canonical_workspace_root();

        if !is_within_root(&root, &canonical) {
            return Err("只允许访问当前工作区内的相对路径。".to_string());
        }

        Ok(canonical)
    }

    fn canonical_workspace_root(&self) -> PathBuf {
        self.workspace_root
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_root.clone())
    }

    fn display_workspace_relative(&self, path: &Path) -> String {
        let root = self.canonical_workspace_root();
        path.strip_prefix(&root)
            .ok()
            .map(|value| {
                let display = value.display().to_string().replace('\\', "/");
                if display.is_empty() {
                    ".".to_string()
                } else {
                    display
                }
            })
            .unwrap_or_else(|| path.display().to_string().replace('\\', "/"))
    }
}

impl ToolExecutor for ToolRouter {
    fn execute(&self, call: &ToolCall) -> ToolResult {
        ToolRouter::execute(self, call)
    }
}

pub fn builtin_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: TOOL_TIME_NOW,
            description: "返回当前本机 UNIX 时间戳，适合最小时间查询演示。",
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_ECHO_INPUT,
            description: "把传入的 text 原样返回，适合验证 tool roundtrip。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "text": {
                        "type": "string",
                        "description": "需要原样回显给用户的文本"
                    }
                },
                "required": ["text"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_READ_FILE,
            description: "读取当前工作区内的文本文件内容预览，需要提供相对路径；大文件会被拒绝并引导改用分段读取。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对文件路径"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_READ_FILE_SEGMENT,
            description: "按行读取当前工作区文件的一段内容，适合大文件局部查看。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对文件路径"
                    },
                    "startLine": {
                        "type": "integer",
                        "description": "从第几行开始读取，最小为 1"
                    },
                    "lineCount": {
                        "type": "integer",
                        "description": "读取多少行，默认 40，最大 400"
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_LIST_FILES,
            description: "列出当前工作区目录下的文件和子目录，可指定相对路径和返回条数。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对目录路径，默认为 ."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "最多返回多少个条目，默认 40"
                    }
                },
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_PATH_INFO,
            description: "返回工作区内文件或目录的路径元信息，适合快速判断它是什么、大小和层级。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对路径，默认为 ."
                    }
                },
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_SEARCH_TEXT,
            description: "递归搜索工作区目录内的文本内容，返回命中的路径、行号和预览。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "要搜索的关键字或文本片段"
                    },
                    "path": {
                        "type": "string",
                        "description": "搜索起点目录，默认为 ."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "最多返回多少条命中，默认 20"
                    },
                    "ignoreCase": {
                        "type": "boolean",
                        "description": "是否忽略大小写，默认 true"
                    },
                    "filePattern": {
                        "type": "string",
                        "description": "可选的路径子串过滤，例如 .rs 或 src/agent"
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_BATCH,
            description: "批量执行多个工具子调用，可选并发和 continueOnError，用于一次性收集多个上下文片段。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "parallel": {
                        "type": "boolean",
                        "description": "是否并发执行；当 continueOnError=false 时会自动退回串行"
                    },
                    "continueOnError": {
                        "type": "boolean",
                        "description": "某个子调用失败后是否继续执行其余子调用"
                    },
                    "calls": {
                        "type": "array",
                        "description": "子调用列表，每个元素包含 name 和可选 arguments",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "arguments": { "type": "object" }
                            },
                            "required": ["name"],
                            "additionalProperties": false
                        }
                    }
                },
                "required": ["calls"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_GATHER_CONTEXT,
            description: "围绕一个路径自动收集最合适的上下文：文件会拿 path info 和首段内容，目录会拿 path info 和文件列表，带 query 时会连同搜索结果一起返回。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "目标路径，默认为 ."
                    },
                    "paths": {
                        "type": "array",
                        "description": "可选的多路径聚合输入；提供后会依次对每个路径执行单路径 gather 并汇总结果",
                        "items": {
                            "type": "string"
                        }
                    },
                    "query": {
                        "type": "string",
                        "description": "可选搜索词；提供后会进入搜索模式"
                    },
                    "lineCount": {
                        "type": "integer",
                        "description": "文件模式下读取多少行，默认 80，最大 400"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "目录模式和搜索模式下的条数限制，默认 40"
                    }
                },
                "additionalProperties": false
            }),
        },
    ]
}

fn canonical_tool_name(name: &str) -> Option<&'static str> {
    match name {
        TOOL_TIME_NOW | "time.now" => Some(TOOL_TIME_NOW),
        TOOL_ECHO_INPUT | "echo.input" => Some(TOOL_ECHO_INPUT),
        TOOL_WORKSPACE_LIST_FILES | "workspace.list_files" => Some(TOOL_WORKSPACE_LIST_FILES),
        TOOL_WORKSPACE_READ_FILE | "workspace.read_file" => Some(TOOL_WORKSPACE_READ_FILE),
        TOOL_WORKSPACE_READ_FILE_SEGMENT | "workspace.read_file_segment" => {
            Some(TOOL_WORKSPACE_READ_FILE_SEGMENT)
        }
        TOOL_WORKSPACE_PATH_INFO | "workspace.path_info" => Some(TOOL_WORKSPACE_PATH_INFO),
        TOOL_WORKSPACE_SEARCH_TEXT | "workspace.search_text" => Some(TOOL_WORKSPACE_SEARCH_TEXT),
        TOOL_WORKSPACE_BATCH | "workspace.batch" => Some(TOOL_WORKSPACE_BATCH),
        TOOL_WORKSPACE_GATHER_CONTEXT | "workspace.gather_context" => {
            Some(TOOL_WORKSPACE_GATHER_CONTEXT)
        }
        _ => None,
    }
}

fn truncate_preview(content: &str, max_chars: usize) -> String {
    let preview = content.chars().take(max_chars).collect::<String>();
    if content.chars().count() > max_chars {
        format!(
            "{}\n\n[内容已截断，当前仅展示前 {} 个字符]",
            preview, max_chars
        )
    } else {
        preview
    }
}

fn is_within_root(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
}

fn preview_text(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        text.to_string()
    } else {
        let preview = text.chars().take(max_chars).collect::<String>();
        format!("{}...(+{} chars)", preview, count - max_chars)
    }
}

fn collect_files_recursively(
    dir: &Path,
    files: &mut Vec<PathBuf>,
    max_files: usize,
) -> std::io::Result<()> {
    if files.len() >= max_files {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if should_skip_dir(&path) {
                continue;
            }
            collect_files_recursively(&path, files, max_files)?;
            if files.len() >= max_files {
                break;
            }
        } else if path.is_file() {
            files.push(path);
            if files.len() >= max_files {
                break;
            }
        }
    }

    Ok(())
}

fn should_skip_dir(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|value| value.to_str()),
        Some(".git" | "node_modules" | "target" | "dist" | "build")
    )
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

fn path_matches_filter(relative_path: &str, pattern: &str) -> bool {
    let path = relative_path.replace('\\', "/").to_lowercase();
    let pattern = pattern.replace('\\', "/").trim().to_lowercase();
    if pattern.is_empty() {
        return true;
    }

    if !pattern.contains('*') {
        return path.contains(&pattern);
    }

    wildcard_match(&path, &pattern)
}

fn wildcard_match(input: &str, pattern: &str) -> bool {
    let parts = pattern.split('*').collect::<Vec<_>>();
    if parts.len() == 1 {
        return input == pattern;
    }

    let mut remainder = input;
    let anchored_start = !pattern.starts_with('*');
    let anchored_end = !pattern.ends_with('*');

    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }

        if index == 0 && anchored_start {
            let Some(next) = remainder.strip_prefix(part) else {
                return false;
            };
            remainder = next;
            continue;
        }

        if index == parts.len() - 1 && anchored_end {
            return remainder.ends_with(part);
        }

        let Some(position) = remainder.find(part) else {
            return false;
        };
        remainder = &remainder[position + part.len()..];
    }

    true
}

fn error_result(tool_name: &str, code: &str, message: String, hint: Option<String>) -> ToolResult {
    ToolResult {
        tool_name: tool_name.to_string(),
        status: "error".to_string(),
        output: json_string(json!({
            "ok": false,
            "tool": tool_name,
            "error": {
                "code": code,
                "message": message,
                "hint": hint,
            }
        })),
        duration_ms: 0,
    }
}

fn aborted_result(tool_name: &str, code: &str, message: String) -> ToolResult {
    ToolResult {
        tool_name: tool_name.to_string(),
        status: "aborted".to_string(),
        output: json_string(json!({
            "ok": false,
            "tool": tool_name,
            "status": "aborted",
            "error": {
                "code": code,
                "message": message,
                "hint": Option::<String>::None,
            }
        })),
        duration_ms: 0,
    }
}

fn json_string(value: Value) -> String {
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
}

fn parse_tool_output(output: &str) -> Value {
    serde_json::from_str::<Value>(output).unwrap_or_else(|_| Value::String(output.to_string()))
}

fn nested_result_status(result: &ToolResult) -> String {
    let output = parse_tool_output(&result.output);
    nested_output_status(result, &output)
}

fn nested_output_status(result: &ToolResult, output: &Value) -> String {
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

fn first_search_match_line(output: &str, target_path: &str) -> Option<usize> {
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

fn search_match_count(output: &str) -> usize {
    parse_tool_output(output)
        .get("matchCount")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize
}

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

enum FileSegment {
    Empty,
    Range {
        start_line: usize,
        end_line: usize,
        lines: Vec<(usize, String)>,
        total_lines: usize,
    },
}

enum ReadSegmentError {
    StartOutOfRange { total_lines: usize },
    Io(std::io::Error),
}

fn read_file_lines(
    path: &Path,
    start_line: usize,
    line_count: usize,
) -> Result<FileSegment, ReadSegmentError> {
    let file = File::open(path).map_err(ReadSegmentError::Io)?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    let mut total_lines = 0usize;

    for (index, line) in reader.lines().enumerate() {
        let line_number = index + 1;
        total_lines = line_number;
        let line = line.map_err(ReadSegmentError::Io)?;
        if line_number < start_line {
            continue;
        }
        if lines.len() < line_count {
            lines.push((line_number, line));
        }
    }

    if total_lines == 0 {
        return Ok(FileSegment::Empty);
    }

    if start_line > total_lines {
        return Err(ReadSegmentError::StartOutOfRange { total_lines });
    }

    let end_line = lines
        .last()
        .map(|(line_number, _)| *line_number)
        .unwrap_or(start_line);

    Ok(FileSegment::Range {
        start_line,
        end_line,
        lines,
        total_lines,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_workspace() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("pony-agent-tools-test-{}", unique));
        fs::create_dir_all(&dir).expect("create temp workspace");
        dir
    }

    #[test]
    fn batch_returns_partial_success_without_failing_runtime_status() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "hello\nworld\n").expect("write demo file");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_BATCH.to_string(),
            arguments: json!({
                "parallel": true,
                "continueOnError": true,
                "calls": [
                    {
                        "name": TOOL_WORKSPACE_PATH_INFO,
                        "arguments": { "path": "demo.txt" }
                    },
                    {
                        "name": TOOL_WORKSPACE_READ_FILE,
                        "arguments": { "path": "missing.txt" }
                    }
                ]
            }),
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
        assert_eq!(
            payload.get("status").and_then(Value::as_str),
            Some("partial")
        );
        assert_eq!(payload.get("successCount").and_then(Value::as_u64), Some(1));
    }

    #[test]
    fn gather_context_reads_file_info_and_segment() {
        let workspace = temp_workspace();
        fs::write(
            workspace.join("demo.rs"),
            "fn main() {}\nprintln!(\"hi\");\n",
        )
        .expect("write file");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
            arguments: json!({ "path": "demo.rs", "lineCount": 20 }),
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
        assert_eq!(
            payload
                .get("meta")
                .and_then(Value::as_object)
                .and_then(|meta| meta.get("mode"))
                .and_then(Value::as_str),
            Some("file")
        );
        assert_eq!(payload.get("successCount").and_then(Value::as_u64), Some(2));
    }

    #[test]
    fn gather_context_can_aggregate_multiple_paths() {
        let workspace = temp_workspace();
        fs::write(workspace.join("one.rs"), "fn one() {}\n").expect("write one");
        fs::write(workspace.join("two.rs"), "fn two() {}\n").expect("write two");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
            arguments: json!({
                "paths": ["one.rs", "two.rs"],
                "lineCount": 20
            }),
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
        assert_eq!(
            payload
                .get("meta")
                .and_then(Value::as_object)
                .and_then(|meta| meta.get("mode"))
                .and_then(Value::as_str),
            Some("multi_path")
        );
        assert_eq!(payload.get("successCount").and_then(Value::as_u64), Some(2));
    }

    #[test]
    fn gather_context_rejects_too_many_paths() {
        let workspace = temp_workspace();
        for index in 0..=MAX_BATCH_CALLS {
            fs::write(workspace.join(format!("demo-{index}.rs")), "fn demo() {}\n")
                .expect("write demo");
        }
        let router = ToolRouter::with_workspace_root(workspace.clone());
        let paths = (0..=MAX_BATCH_CALLS)
            .map(|index| Value::String(format!("demo-{index}.rs")))
            .collect::<Vec<_>>();

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
            arguments: json!({
                "paths": paths,
                "lineCount": 20
            }),
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("too_many_paths")
        );
    }

    #[test]
    fn search_text_supports_file_path_input_and_wildcard_filter() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("src")).expect("create src dir");
        fs::write(workspace.join("src/lib.rs"), "pub struct ToolRouter;\n").expect("write lib");
        fs::write(workspace.join("src/lib.txt"), "ToolRouter in text\n").expect("write txt");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
            arguments: json!({
                "query": "ToolRouter",
                "path": "src/lib.rs",
                "filePattern": "*.rs"
            }),
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("search output json");
        assert_eq!(payload.get("pathKind").and_then(Value::as_str), Some("file"));
        assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(1));
        assert_eq!(
            payload
                .get("matches")
                .and_then(Value::as_array)
                .and_then(|matches| matches.first())
                .and_then(|entry| entry.get("path"))
                .and_then(Value::as_str),
            Some("src/lib.rs")
        );
    }

    #[test]
    fn gather_context_search_file_includes_segment_even_without_match() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.rs"), "fn main() {}\nprintln!(\"hi\");\n")
            .expect("write demo");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
            arguments: json!({
                "path": "demo.rs",
                "query": "missing_symbol",
                "lineCount": 20
            }),
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
        assert_eq!(payload.get("plannedCount").and_then(Value::as_u64), Some(3));
        assert!(payload
            .get("results")
            .and_then(Value::as_array)
            .map(|results| {
                results.iter().any(|entry| {
                    entry.get("tool").and_then(Value::as_str)
                        == Some(TOOL_WORKSPACE_READ_FILE_SEGMENT)
                })
            })
            .unwrap_or(false));
    }

    #[test]
    fn gather_context_search_directory_falls_back_to_listing_on_empty_match() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("src")).expect("create src dir");
        fs::write(workspace.join("src/lib.rs"), "pub struct ToolRouter;\n").expect("write lib");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
            arguments: json!({
                "path": "src",
                "query": "missing_symbol",
                "limit": 10
            }),
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
        assert_eq!(payload.get("plannedCount").and_then(Value::as_u64), Some(3));
        assert!(payload
            .get("results")
            .and_then(Value::as_array)
            .map(|results| {
                results.iter().any(|entry| {
                    entry.get("tool").and_then(Value::as_str) == Some(TOOL_WORKSPACE_LIST_FILES)
                })
            })
            .unwrap_or(false));
    }
}
