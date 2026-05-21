use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

const TOOL_TIME_NOW: &str = "time_now";
const TOOL_ECHO_INPUT: &str = "echo_input";
const TOOL_WORKSPACE_LIST_FILES: &str = "workspace_list_files";
const TOOL_WORKSPACE_READ_FILE: &str = "workspace_read_file";
const TOOL_WORKSPACE_READ_FILE_SEGMENT: &str = "workspace_read_file_segment";
const TOOL_WORKSPACE_PATH_INFO: &str = "workspace_path_info";
const TOOL_WORKSPACE_SEARCH_TEXT: &str = "workspace_search_text";

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

    pub fn execute(&self, call: &ToolCall) -> ToolResult {
        let started_at = Instant::now();
        let mut result = match call.name.as_str() {
            TOOL_TIME_NOW | "time.now" => self.time_now(),
            TOOL_ECHO_INPUT | "echo.input" => self.echo_input(call),
            TOOL_WORKSPACE_LIST_FILES | "workspace.list_files" => self.list_files(call),
            TOOL_WORKSPACE_READ_FILE | "workspace.read_file" => self.read_file(call),
            TOOL_WORKSPACE_READ_FILE_SEGMENT | "workspace.read_file_segment" => {
                self.read_file_segment(call)
            }
            TOOL_WORKSPACE_PATH_INFO | "workspace.path_info" => self.path_info(call),
            TOOL_WORKSPACE_SEARCH_TEXT | "workspace.search_text" => self.search_text(call),
            other => error_result(
                other,
                "unsupported_tool",
                format!("当前 runtime 还没有实现工具 `{}`。", other),
                Some("请改用 list_available_tools 中返回的工具名。".to_string()),
            ),
        };
        result.duration_ms = started_at.elapsed().as_millis() as u64;
        result
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
                "当前 UNIX 时间戳（秒）为 {}。如果需要本地格式化时间，可以在下一阶段补充 chrono/time 支持。",
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

        match self.resolve_workspace_path(path) {
            Ok(resolved) => match fs::read_to_string(&resolved) {
                Ok(content) => {
                    let preview = truncate_preview(&content, 4000);
                    ToolResult {
                        tool_name: TOOL_WORKSPACE_READ_FILE.to_string(),
                        status: "ok".to_string(),
                        output: format!("文件 {} 读取成功。\n\n{}", resolved.display(), preview),
                        duration_ms: 0,
                    }
                }
                Err(error) => error_result(
                    TOOL_WORKSPACE_READ_FILE,
                    "read_failed",
                    format!("读取文件失败：{}。", error),
                    Some("请确认目标文件是 UTF-8 文本，且当前进程有读取权限。".to_string()),
                ),
            },
            Err(error) => error_result(TOOL_WORKSPACE_READ_FILE, "invalid_path", error, None),
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
            .map(|value| value.clamp(1, 400) as usize)
            .unwrap_or(40);

        match self.resolve_workspace_path(path) {
            Ok(resolved) => match fs::read_to_string(&resolved) {
                Ok(content) => {
                    let lines = content.lines().collect::<Vec<_>>();
                    if lines.is_empty() {
                        return ToolResult {
                            tool_name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                            status: "ok".to_string(),
                            output: format!("文件 {} 为空。", resolved.display()),
                            duration_ms: 0,
                        };
                    }

                    let start_index = start_line.saturating_sub(1);
                    if start_index >= lines.len() {
                        return error_result(
                            TOOL_WORKSPACE_READ_FILE_SEGMENT,
                            "line_out_of_range",
                            format!("起始行 {} 超出文件总行数 {}。", start_line, lines.len()),
                            Some("请缩小 startLine，或先用 workspace_path_info / workspace_read_file 查看文件概况。".to_string()),
                        );
                    }

                    let end_index = (start_index + line_count).min(lines.len());
                    let segment = lines[start_index..end_index]
                        .iter()
                        .enumerate()
                        .map(|(offset, line)| format!("{:>4} | {}", start_index + offset + 1, line))
                        .collect::<Vec<_>>()
                        .join("\n");

                    ToolResult {
                        tool_name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                        status: "ok".to_string(),
                        output: format!(
                            "文件 {} 第 {} 行到第 {} 行：\n{}",
                            resolved.display(),
                            start_index + 1,
                            end_index,
                            segment
                        ),
                        duration_ms: 0,
                    }
                }
                Err(error) => error_result(
                    TOOL_WORKSPACE_READ_FILE_SEGMENT,
                    "read_failed",
                    format!("读取文件失败：{}。", error),
                    Some("请确认目标文件是 UTF-8 文本，且当前进程有读取权限。".to_string()),
                ),
            },
            Err(error) => error_result(
                TOOL_WORKSPACE_READ_FILE_SEGMENT,
                "invalid_path",
                error,
                None,
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
            .unwrap_or(40);

        match self.resolve_workspace_dir(relative_dir) {
            Ok(dir) => match fs::read_dir(&dir) {
                Ok(entries) => {
                    let mut items = entries
                        .filter_map(Result::ok)
                        .map(|entry| {
                            let path = entry.path();
                            let label = path
                                .strip_prefix(&self.workspace_root)
                                .ok()
                                .map(|relative| relative.display().to_string())
                                .unwrap_or_else(|| path.display().to_string());
                            if path.is_dir() {
                                format!("{}/", label.replace('\\', "/"))
                            } else {
                                label.replace('\\', "/")
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
                            dir.display(),
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
            },
            Err(error) => error_result(TOOL_WORKSPACE_LIST_FILES, "invalid_path", error, None),
        }
    }

    fn path_info(&self, call: &ToolCall) -> ToolResult {
        let relative_path = call
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .trim();

        match self.resolve_workspace_entry(relative_path) {
            Ok(path) => match fs::metadata(&path) {
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
                    let relative = self.display_workspace_relative(&path);

                    ToolResult {
                        tool_name: TOOL_WORKSPACE_PATH_INFO.to_string(),
                        status: "ok".to_string(),
                        output: json_string(json!({
                            "path": relative,
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
            },
            Err(error) => error_result(TOOL_WORKSPACE_PATH_INFO, "invalid_path", error, None),
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
                Some("请提供要搜索的关键字或片段。".to_string()),
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

        match self.resolve_workspace_dir(relative_dir) {
            Ok(root_dir) => {
                let mut files = Vec::new();
                if let Err(error) = collect_files_recursively(&root_dir, &mut files, 500) {
                    return error_result(
                        TOOL_WORKSPACE_SEARCH_TEXT,
                        "walk_failed",
                        format!("遍历目录失败：{}。", error),
                        Some("请缩小 path 范围后重试。".to_string()),
                    );
                }

                let file_filter = call
                    .arguments
                    .get("filePattern")
                    .and_then(Value::as_str)
                    .map(|value| value.trim().to_lowercase())
                    .filter(|value| !value.is_empty());
                let normalized_query = if ignore_case {
                    query.to_lowercase()
                } else {
                    query.to_string()
                };

                let mut matches = Vec::new();
                let mut scanned_files = 0usize;
                let mut skipped_unreadable = 0usize;
                let mut skipped_large = 0usize;

                for file_path in files {
                    if matches.len() >= limit {
                        break;
                    }

                    let relative = self.display_workspace_relative(&file_path);
                    if let Some(pattern) = &file_filter {
                        if !relative.to_lowercase().contains(pattern) {
                            continue;
                        }
                    }

                    let Ok(metadata) = fs::metadata(&file_path) else {
                        skipped_unreadable += 1;
                        continue;
                    };
                    if metadata.len() > 1_000_000 {
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
                        "path": self.display_workspace_relative(&root_dir),
                        "ignoreCase": ignore_case,
                        "filePattern": file_filter,
                        "scannedFiles": scanned_files,
                        "skippedUnreadableFiles": skipped_unreadable,
                        "skippedLargeFiles": skipped_large,
                        "matchCount": matches.len(),
                        "matches": matches,
                    })),
                    duration_ms: 0,
                }
            }
            Err(error) => error_result(TOOL_WORKSPACE_SEARCH_TEXT, "invalid_path", error, None),
        }
    }

    fn resolve_workspace_path(&self, raw_path: &str) -> Result<PathBuf, String> {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return Err("文件路径不能为空。".to_string());
        }

        let candidate = self.workspace_root.join(trimmed);
        let canonical = candidate
            .canonicalize()
            .map_err(|error| format!("无法解析路径 {}：{}", trimmed, error))?;
        let root = self
            .workspace_root
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_root.clone());

        if !is_within_root(&root, &canonical) {
            return Err("只允许读取当前工作区内的相对路径。".to_string());
        }

        if !canonical.is_file() {
            return Err(format!("目标不是文件：{}。", canonical.display()));
        }

        Ok(canonical)
    }

    fn resolve_workspace_entry(&self, raw_path: &str) -> Result<PathBuf, String> {
        let trimmed = if raw_path.trim().is_empty() {
            "."
        } else {
            raw_path.trim()
        };
        let candidate = self.workspace_root.join(trimmed);
        let canonical = candidate
            .canonicalize()
            .map_err(|error| format!("无法解析路径 {}：{}", trimmed, error))?;
        let root = self
            .workspace_root
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_root.clone());

        if !is_within_root(&root, &canonical) {
            return Err("只允许访问当前工作区内的相对路径。".to_string());
        }

        Ok(canonical)
    }

    fn resolve_workspace_dir(&self, raw_path: &str) -> Result<PathBuf, String> {
        let trimmed = if raw_path.is_empty() { "." } else { raw_path };
        let candidate = self.workspace_root.join(trimmed);
        let canonical = candidate
            .canonicalize()
            .map_err(|error| format!("无法解析目录 {}：{}", trimmed, error))?;
        let root = self
            .workspace_root
            .canonicalize()
            .unwrap_or_else(|_| self.workspace_root.clone());

        if !is_within_root(&root, &canonical) {
            return Err("只允许访问当前工作区内的相对目录。".to_string());
        }

        if !canonical.is_dir() {
            return Err(format!("目标不是目录：{}。", canonical.display()));
        }

        Ok(canonical)
    }

    fn display_workspace_relative(&self, path: &Path) -> String {
        path.strip_prefix(&self.workspace_root)
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
            description: "读取当前工作区内的文本文件内容预览，需要提供相对路径。",
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
                        "description": "读取多少行，默认 40"
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
    ]
}

fn truncate_preview(content: &str, max_chars: usize) -> String {
    let mut preview = String::new();
    for chunk in content.chars().take(max_chars) {
        preview.push(chunk);
    }

    if content.chars().count() > max_chars {
        preview.push_str("\n\n[内容已截断，当前仅展示前 4000 个字符]");
    }

    preview
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

fn json_string(value: Value) -> String {
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| value.to_string())
}
