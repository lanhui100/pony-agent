use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Serialize, Deserialize)]
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
        match call.name.as_str() {
            "time.now" => self.time_now(),
            "echo.input" => self.echo_input(call),
            "workspace.list_files" => self.list_files(call),
            "workspace.read_file" => self.read_file(call),
            "workspace.read_file_segment" => self.read_file_segment(call),
            other => ToolResult {
                tool_name: other.to_string(),
                status: "error".to_string(),
                output: format!("当前 runtime 还没有实现工具：{}", other),
            },
        }
    }

    fn time_now(&self) -> ToolResult {
        let unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_secs())
            .unwrap_or(0);

        ToolResult {
            tool_name: "time.now".to_string(),
            status: "ok".to_string(),
            output: format!(
                "当前 UNIX 时间戳（秒）为 {}。如果需要本地格式化时间，可以在下一阶段补充 chrono/time 支持。",
                unix_seconds
            ),
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
            return ToolResult {
                tool_name: "echo.input".to_string(),
                status: "error".to_string(),
                output: "echo.input 需要参数 {\"text\":\"...\"}。".to_string(),
            };
        }

        ToolResult {
            tool_name: "echo.input".to_string(),
            status: "ok".to_string(),
            output: format!("echo.input 返回：{}", text),
        }
    }

    fn read_file(&self, call: &ToolCall) -> ToolResult {
        let Some(path) = call.arguments.get("path").and_then(Value::as_str) else {
            return ToolResult {
                tool_name: "workspace.read_file".to_string(),
                status: "error".to_string(),
                output: "workspace.read_file 需要参数 {\"path\":\"相对工作区路径\"}。".to_string(),
            };
        };

        match self.resolve_workspace_path(path) {
            Ok(resolved) => match fs::read_to_string(&resolved) {
                Ok(content) => {
                    let preview = truncate_preview(&content, 4000);
                    ToolResult {
                        tool_name: "workspace.read_file".to_string(),
                        status: "ok".to_string(),
                        output: format!(
                            "文件 {} 读取成功。\n\n{}",
                            resolved.display(),
                            preview
                        ),
                    }
                }
                Err(error) => ToolResult {
                    tool_name: "workspace.read_file".to_string(),
                    status: "error".to_string(),
                    output: format!("读取文件失败：{}。", error),
                },
            },
            Err(error) => ToolResult {
                tool_name: "workspace.read_file".to_string(),
                status: "error".to_string(),
                output: error,
            },
        }
    }

    fn read_file_segment(&self, call: &ToolCall) -> ToolResult {
        let Some(path) = call.arguments.get("path").and_then(Value::as_str) else {
            return ToolResult {
                tool_name: "workspace.read_file_segment".to_string(),
                status: "error".to_string(),
                output: "workspace.read_file_segment 需要参数 {\"path\":\"相对工作区路径\",\"startLine\":1,\"lineCount\":40}。".to_string(),
            };
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
                            tool_name: "workspace.read_file_segment".to_string(),
                            status: "ok".to_string(),
                            output: format!("文件 {} 为空。", resolved.display()),
                        };
                    }

                    let start_index = start_line.saturating_sub(1);
                    if start_index >= lines.len() {
                        return ToolResult {
                            tool_name: "workspace.read_file_segment".to_string(),
                            status: "error".to_string(),
                            output: format!(
                                "起始行 {} 超出文件总行数 {}。",
                                start_line,
                                lines.len()
                            ),
                        };
                    }

                    let end_index = (start_index + line_count).min(lines.len());
                    let segment = lines[start_index..end_index]
                        .iter()
                        .enumerate()
                        .map(|(offset, line)| format!("{:>4} | {}", start_index + offset + 1, line))
                        .collect::<Vec<_>>()
                        .join("\n");

                    ToolResult {
                        tool_name: "workspace.read_file_segment".to_string(),
                        status: "ok".to_string(),
                        output: format!(
                            "文件 {} 第 {} 行到第 {} 行：\n{}",
                            resolved.display(),
                            start_index + 1,
                            end_index,
                            segment
                        ),
                    }
                }
                Err(error) => ToolResult {
                    tool_name: "workspace.read_file_segment".to_string(),
                    status: "error".to_string(),
                    output: format!("读取文件失败：{}。", error),
                },
            },
            Err(error) => ToolResult {
                tool_name: "workspace.read_file_segment".to_string(),
                status: "error".to_string(),
                output: error,
            },
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
                        tool_name: "workspace.list_files".to_string(),
                        status: "ok".to_string(),
                        output: format!(
                            "目录 {} 下共发现 {} 个条目，当前展示前 {} 个：\n{}",
                            dir.display(),
                            total,
                            preview.len(),
                            preview.join("\n")
                        ),
                    }
                }
                Err(error) => ToolResult {
                    tool_name: "workspace.list_files".to_string(),
                    status: "error".to_string(),
                    output: format!("读取目录失败：{}。", error),
                },
            },
            Err(error) => ToolResult {
                tool_name: "workspace.list_files".to_string(),
                status: "error".to_string(),
                output: error,
            },
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
}

pub fn builtin_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "time.now",
            description: "返回当前本机 UNIX 时间戳，适合最小时间查询演示。",
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: "echo.input",
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
            name: "workspace.read_file",
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
            name: "workspace.read_file_segment",
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
            name: "workspace.list_files",
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
