use crate::agent::runtime_helper::block_on;
use encoding_rs::{Encoding, GBK};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::error::Error;
use std::env;
use std::fs;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

const TOOL_TIME_NOW: &str = "time_now";
const TOOL_ECHO_INPUT: &str = "echo_input";
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

const MAX_FULL_READ_BYTES: u64 = 120_000;
const MAX_SEARCH_FILE_BYTES: u64 = 1_000_000;
const MAX_SEARCH_FILES: usize = 800;
const MAX_PATH_REPAIR_SEARCH_FILES: usize = 2_000;
const MAX_WORKSPACE_BATCH_CALLS: usize = 24;
const MAX_GATHER_CONTEXT_PATHS: usize = 6;
const MAX_SEGMENT_LINES: usize = 400;
const DEFAULT_SEGMENT_LINES: usize = 80;
const DEFAULT_LIST_LIMIT: usize = 40;
const SUMMARY_ITEM_LIMIT: usize = 3;
const DEFAULT_RUN_TIMEOUT_MS: u64 = 10_000;
const MAX_RUN_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_WEB_TIMEOUT_MS: u64 = 15_000;
const TOOL_TIMEOUT_RETRY_MAX_ATTEMPTS: u32 = 5;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Search,
    Write,
    Execute,
    Plan,
    Interactive,
    Composite,
    External,
}

impl ToolKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Search => "search",
            Self::Write => "write",
            Self::Execute => "execute",
            Self::Plan => "plan",
            Self::Interactive => "interactive",
            Self::Composite => "composite",
            Self::External => "external",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolExposure {
    ModelVisible,
    Internal,
    Deferred,
}

impl ToolExposure {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ModelVisible => "model_visible",
            Self::Internal => "internal",
            Self::Deferred => "deferred",
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ToolDisplayMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name_zh: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolPermissionFacts {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires_approval: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_mediated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_source: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolError {
    pub kind: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retryable: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceContext {
    pub root: PathBuf,
    pub display_name: String,
    pub writable: bool,
    pub default_shell_cwd: PathBuf,
    pub policy: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDefinitionContractView {
    pub name: String,
    pub canonical_tool_name: String,
    pub execution_primitive: String,
    pub description: String,
    pub input_schema: Value,
    pub kind: String,
    pub exposure: String,
    pub display_metadata: ToolDisplayMetadata,
    pub permission_facts: ToolPermissionFacts,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallContractView {
    pub call_id: Option<String>,
    pub name: String,
    pub canonical_tool_name: String,
    pub execution_primitive: String,
    pub arguments: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<ToolPlan>,
    pub kind: String,
    pub exposure: String,
    pub display_metadata: ToolDisplayMetadata,
    pub permission_facts: ToolPermissionFacts,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultContractView {
    pub tool_name: String,
    pub canonical_tool_name: String,
    pub execution_primitive: String,
    pub status: String,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ToolError>,
    #[serde(default)]
    pub child_results: Vec<Value>,
    #[serde(default)]
    pub artifacts: Vec<Value>,
    pub duration_ms: u64,
    pub display_metadata: ToolDisplayMetadata,
    pub permission_facts: ToolPermissionFacts,
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<ToolPlan>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_name: String,
    pub status: String,
    pub output: String,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPlanStep {
    pub name: String,
    pub arguments: Value,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolPlan {
    pub kind: String,
    pub summary: String,
    #[serde(default)]
    pub parallel: bool,
    #[serde(default)]
    pub continue_on_error: bool,
    pub steps: Vec<ToolPlanStep>,
}

pub trait ToolExecutor: Send + Sync {
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

    pub fn with_workspace_root(workspace_root: PathBuf) -> Self {
        Self { workspace_root }
    }

    pub fn workspace_context(&self) -> WorkspaceContext {
        WorkspaceContext {
            root: self.workspace_root.clone(),
            display_name: self
                .workspace_root
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("workspace")
                .to_string(),
            writable: true,
            default_shell_cwd: self.workspace_root.clone(),
            policy: "workspace_root".to_string(),
        }
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
            Some(TOOL_WORKSPACE_GLOB_FILES) => self.glob_files(call),
            Some(TOOL_WORKSPACE_WRITE_FILE) => self.write_file(call),
            Some(TOOL_WORKSPACE_EDIT_FILE) => self.edit_file(call),
            Some(TOOL_WORKSPACE_RUN_COMMAND) => self.run_command(call),
            Some(TOOL_WEB_FETCH_URL) => self.web_fetch(call),
            Some(TOOL_WEB_SEARCH_QUERY) => self.web_search(call),
            Some(TOOL_MCP_RESOURCE_READ) => error_result(
                TOOL_MCP_RESOURCE_READ,
                "deferred_to_registry",
                "MCPResource 由 capability registry 代理执行。".to_string(),
                Some("请通过 runtime 注册工具执行入口调用该工具。".to_string()),
            ),
            Some(TOOL_TOOL_SEARCH) => error_result(
                TOOL_TOOL_SEARCH,
                "deferred_to_registry",
                "ToolSearch 由 capability registry 代理执行。".to_string(),
                Some("请通过 runtime 注册工具执行入口调用该工具。".to_string()),
            ),
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
        let local_now = chrono::Local::now();
        let local_iso = local_now.format("%Y-%m-%d %H:%M:%S %z").to_string();
        let timezone = local_now.format("%z").to_string();

        ToolResult {
            tool_name: TOOL_TIME_NOW.to_string(),
            status: "ok".to_string(),
            output: json_string(json!({
                "unixTimestampSeconds": unix_seconds,
                "localIso": local_iso,
                "timezone": timezone,
            })),
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
            let fallback = call
                .arguments
                .get("question")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());

            let Some(fallback) = fallback else {
                return error_result(
                    TOOL_ECHO_INPUT,
                    "missing_argument",
                    "缺少必填参数 `text`，且未提供 fallback `question`。".to_string(),
                    Some(
                        "参数示例：{\"text\":\"请确认是否继续\"} 或 {\"question\":\"请确认下一步要执行什么？\"}"
                            .to_string(),
                    ),
                );
            };

            return ToolResult {
                tool_name: TOOL_ECHO_INPUT.to_string(),
                status: "ok".to_string(),
                output: json_string(json!({
                    "ok": true,
                    "tool": TOOL_ECHO_INPUT,
                    "mode": "fallback_clarification",
                    "prompt": fallback,
                    "summary": {
                        "text": fallback
                    },
                    "permission": {
                        "requiresApproval": false,
                        "permissionScope": "workspace.read",
                        "hostMediated": false,
                        "permissionProfile": "builtin",
                        "approvalMode": "none",
                        "decisionSource": "runtime"
                    }
                })),
                duration_ms: 0,
            };
        }

        ToolResult {
            tool_name: TOOL_ECHO_INPUT.to_string(),
            status: "ok".to_string(),
            output: format!("echo_input 返回：{}", text),
            duration_ms: 0,
        }
    }

    fn write_file(&self, call: &ToolCall) -> ToolResult {
        let Some(path) = call.arguments.get("path").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_WRITE_FILE,
                "missing_argument",
                "缺少必填参数 `path`。".to_string(),
                Some("参数示例：{\"path\":\"src/demo.txt\",\"content\":\"hello\"}".to_string()),
            );
        };
        let Some(content) = call.arguments.get("content").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_WRITE_FILE,
                "missing_argument",
                "缺少必填参数 `content`。".to_string(),
                Some("参数示例：{\"path\":\"src/demo.txt\",\"content\":\"hello\"}".to_string()),
            );
        };

        let overwrite = call
            .arguments
            .get("overwrite")
            .and_then(Value::as_bool)
            .unwrap_or(true);

        let relative_path = path.trim();
        if relative_path.is_empty() {
            return error_result(
                TOOL_WORKSPACE_WRITE_FILE,
                "empty_path",
                "参数 `path` 不能为空字符串。".to_string(),
                Some("请传入工作区内的相对文件路径。".to_string()),
            );
        }

        let target = match self.prepare_workspace_file_path(relative_path) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WORKSPACE_WRITE_FILE, "invalid_path", error, None)
            }
        };

        let existed_before = target.exists();
        if existed_before && !overwrite {
            return error_result(
                TOOL_WORKSPACE_WRITE_FILE,
                "file_exists",
                format!(
                    "目标文件已存在：{}。",
                    self.display_workspace_relative(&target)
                ),
                Some("如需覆盖，请显式传入 {\"overwrite\": true}。".to_string()),
            );
        }

        if let Some(parent) = target.parent() {
            if let Err(error) = fs::create_dir_all(parent) {
                return error_result(
                    TOOL_WORKSPACE_WRITE_FILE,
                    "create_parent_failed",
                    format!("创建父目录失败：{}。", error),
                    Some("请确认目标目录在当前工作区内且进程有写权限。".to_string()),
                );
            }
        }

        if let Err(error) = fs::write(&target, content) {
            return error_result(
                TOOL_WORKSPACE_WRITE_FILE,
                "write_failed",
                format!("写入文件失败：{}。", error),
                Some("请确认目标文件可写，且当前进程有写权限。".to_string()),
            );
        }

        ToolResult {
            tool_name: TOOL_WORKSPACE_WRITE_FILE.to_string(),
            status: "ok".to_string(),
            output: json_string(json!({
                "ok": true,
                "path": self.display_workspace_relative(&target),
                "absolutePath": target.display().to_string(),
                "bytesWritten": content.len(),
                "overwroteExisting": existed_before,
                "summary": {
                    "text": format!("已写入文件 {}。", self.display_workspace_relative(&target))
                },
                "permission": {
                    "requiresApproval": false,
                    "permissionScope": "workspace.write",
                    "hostMediated": false,
                    "permissionProfile": "builtin",
                    "approvalMode": "none",
                    "decisionSource": "runtime"
                }
            })),
            duration_ms: 0,
        }
    }

    fn edit_file(&self, call: &ToolCall) -> ToolResult {
        let Some(path) = call.arguments.get("path").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_EDIT_FILE,
                "missing_argument",
                "缺少必填参数 `path`。".to_string(),
                Some(
                    "参数示例：{\"path\":\"src/demo.txt\",\"oldText\":\"foo\",\"newText\":\"bar\"}"
                        .to_string(),
                ),
            );
        };
        let Some(old_text) = call.arguments.get("oldText").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_EDIT_FILE,
                "missing_argument",
                "缺少必填参数 `oldText`。".to_string(),
                Some(
                    "参数示例：{\"path\":\"src/demo.txt\",\"oldText\":\"foo\",\"newText\":\"bar\"}"
                        .to_string(),
                ),
            );
        };
        let Some(new_text) = call.arguments.get("newText").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_EDIT_FILE,
                "missing_argument",
                "缺少必填参数 `newText`。".to_string(),
                Some(
                    "参数示例：{\"path\":\"src/demo.txt\",\"oldText\":\"foo\",\"newText\":\"bar\"}"
                        .to_string(),
                ),
            );
        };

        if old_text.is_empty() {
            return error_result(
                TOOL_WORKSPACE_EDIT_FILE,
                "empty_old_text",
                "参数 `oldText` 不能为空字符串。".to_string(),
                Some("请提供需要被替换的原始文本。".to_string()),
            );
        }

        let replace_all = call
            .arguments
            .get("replaceAll")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let target = match self.resolve_workspace_path(path) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WORKSPACE_EDIT_FILE, "invalid_path", error, None)
            }
        };

        let original = match fs::read_to_string(&target) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WORKSPACE_EDIT_FILE,
                    "read_failed",
                    format!("读取文件失败：{}。", error),
                    Some("请确认目标文件存在且为可读文本文件。".to_string()),
                )
            }
        };

        let match_count = original.matches(old_text).count();
        if match_count == 0 {
            return error_result(
                TOOL_WORKSPACE_EDIT_FILE,
                "no_match",
                format!(
                    "在文件 {} 中没有找到 `oldText`。",
                    self.display_workspace_relative(&target)
                ),
                Some("请先读取文件确认原始文本，再执行编辑。".to_string()),
            );
        }
        if match_count > 1 && !replace_all {
            return error_result(
                TOOL_WORKSPACE_EDIT_FILE,
                "multiple_matches",
                format!(
                    "在文件 {} 中找到 {} 处匹配；未显式允许批量替换。",
                    self.display_workspace_relative(&target),
                    match_count
                ),
                Some("如需全部替换，请传入 {\"replaceAll\": true}。".to_string()),
            );
        }

        let updated = if replace_all {
            original.replace(old_text, new_text)
        } else {
            original.replacen(old_text, new_text, 1)
        };

        if let Err(error) = fs::write(&target, updated.as_bytes()) {
            return error_result(
                TOOL_WORKSPACE_EDIT_FILE,
                "write_failed",
                format!("写回文件失败：{}。", error),
                Some("请确认目标文件可写，且当前进程有写权限。".to_string()),
            );
        }

        ToolResult {
            tool_name: TOOL_WORKSPACE_EDIT_FILE.to_string(),
            status: "ok".to_string(),
            output: json_string(json!({
                "ok": true,
                "path": self.display_workspace_relative(&target),
                "absolutePath": target.display().to_string(),
                "matchCount": match_count,
                "replacedCount": if replace_all { match_count } else { 1 },
                "replaceAll": replace_all,
                "summary": {
                    "text": format!("已编辑文件 {}。", self.display_workspace_relative(&target))
                },
                "permission": {
                    "requiresApproval": false,
                    "permissionScope": "workspace.write",
                    "hostMediated": false,
                    "permissionProfile": "builtin",
                    "approvalMode": "none",
                    "decisionSource": "runtime"
                }
            })),
            duration_ms: 0,
        }
    }

    fn run_command(&self, call: &ToolCall) -> ToolResult {
        let Some(command) = call.arguments.get("command").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_RUN_COMMAND,
                "missing_argument",
                "缺少必填参数 `command`。".to_string(),
                Some(
                    "参数示例：{\"command\":\"git status\",\"cwd\":\".\",\"timeoutMs\":5000}"
                        .to_string(),
                ),
            );
        };

        let command = command.trim();
        if command.is_empty() {
            return error_result(
                TOOL_WORKSPACE_RUN_COMMAND,
                "empty_command",
                "参数 `command` 不能为空字符串。".to_string(),
                Some("请提供要执行的命令文本。".to_string()),
            );
        }

        if let Some(reason) = denied_run_command_reason(command) {
            return error_result(
                TOOL_WORKSPACE_RUN_COMMAND,
                "command_denied",
                reason,
                Some("请改用只读、非破坏性的工作区内命令。".to_string()),
            );
        }

        let cwd_input = call
            .arguments
            .get("cwd")
            .and_then(Value::as_str)
            .unwrap_or(".");
        let cwd = match self.resolve_workspace_dir(cwd_input) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WORKSPACE_RUN_COMMAND, "invalid_cwd", error, None)
            }
        };
        let timeout_ms = call
            .arguments
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, MAX_RUN_TIMEOUT_MS))
            .unwrap_or(DEFAULT_RUN_TIMEOUT_MS);

        let mut child = match spawn_workspace_command(command, &cwd) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WORKSPACE_RUN_COMMAND,
                    "spawn_failed",
                    format!("启动命令失败：{}。", error),
                    Some("请确认命令语法正确，且当前环境存在对应可执行文件。".to_string()),
                )
            }
        };

        let deadline = Instant::now() + Duration::from_millis(timeout_ms);
        let output = loop {
            match child.try_wait() {
                Ok(Some(_status)) => match child.wait_with_output() {
                    Ok(value) => break Ok(value),
                    Err(error) => {
                        break Err(error_result(
                            TOOL_WORKSPACE_RUN_COMMAND,
                            "wait_failed",
                            format!("等待命令输出失败：{}。", error),
                            Some("请重试，或缩短命令输出与执行时长。".to_string()),
                        ))
                    }
                },
                Ok(None) => {
                    if Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        break Err(error_result(
                            TOOL_WORKSPACE_RUN_COMMAND,
                            "timeout",
                            format!("命令执行超过超时上限 {} ms，已终止。", timeout_ms),
                            Some("请缩短命令执行时间，或显式传入更大的 timeoutMs。".to_string()),
                        ));
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                Err(error) => {
                    break Err(error_result(
                        TOOL_WORKSPACE_RUN_COMMAND,
                        "wait_failed",
                        format!("轮询命令状态失败：{}。", error),
                        Some("请重试，或更换更简单的命令。".to_string()),
                    ))
                }
            }
        };

        let output = match output {
            Ok(value) => value,
            Err(result) => return result,
        };

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let exit_code = output.status.code();
        let succeeded = output.status.success();
        let error_payload = (!succeeded).then(|| {
            json!({
                "code": "non_zero_exit",
                "message": format!(
                    "命令执行完成，但退出码为 {}。",
                    exit_code.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string())
                ),
                "exitCode": exit_code
            })
        });

        ToolResult {
            tool_name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
            status: if succeeded {
                "ok".to_string()
            } else {
                "error".to_string()
            },
            output: json_string(json!({
                "ok": succeeded,
                "cwd": self.display_workspace_relative(&cwd),
                "absoluteCwd": cwd.display().to_string(),
                "command": command,
                "delegateTool": "run_shell",
                "timeoutMs": timeout_ms,
                "exitCode": exit_code,
                "stdout": stdout,
                "stderr": stderr,
                "error": error_payload,
                "summary": {
                    "text": format!(
                        "命令在 {} 执行完成，退出码为 {}。",
                        self.display_workspace_relative(&cwd),
                        exit_code.map(|value| value.to_string()).unwrap_or_else(|| "null".to_string())
                    )
                },
                "permission": {
                    "requiresApproval": false,
                    "permissionScope": "workspace.execute",
                    "hostMediated": false,
                    "permissionProfile": "builtin",
                    "approvalMode": "none",
                    "decisionSource": "runtime"
                }
            })),
            duration_ms: 0,
        }
    }

    fn web_fetch(&self, call: &ToolCall) -> ToolResult {
        let Some(url) = call.arguments.get("url").and_then(Value::as_str) else {
            return error_result(
                TOOL_WEB_FETCH_URL,
                "missing_argument",
                "缺少必填参数 `url`。".to_string(),
                Some("参数示例：{\"url\":\"https://example.com\",\"timeoutMs\":15000}".to_string()),
            );
        };
        let url = url.trim();
        if !is_http_url(url) {
            return error_result(
                TOOL_WEB_FETCH_URL,
                "invalid_url",
                "只允许抓取 http/https URL。".to_string(),
                Some("请传入以 http:// 或 https:// 开头的地址。".to_string()),
            );
        }

        let timeout_ms = call
            .arguments
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 60_000))
            .unwrap_or(DEFAULT_WEB_TIMEOUT_MS);

        let client = match build_web_client(timeout_ms) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WEB_FETCH_URL, "client_build_failed", error, None)
            }
        };

        let response = match retry_tool_timeout(TOOL_WEB_FETCH_URL, || {
            block_on(client.get(url).send()).map_err(|error| {
                if is_reqwest_timeout_error(&error) {
                    format!("timeout: 抓取 URL 超时：{}。", error)
                } else {
                    format!("抓取 URL 失败：{}。", error)
                }
            })
        }) {
            Ok(value) => value,
            Err(error) => {
                let code = if is_tool_timeout_message(&error) {
                    "timeout"
                } else {
                    "request_failed"
                };
                return error_result(
                    TOOL_WEB_FETCH_URL,
                    code,
                    error,
                    Some("请确认目标地址可访问，或稍后重试。".to_string()),
                )
            }
        };

        let status = response.status();
        let final_url = response.url().to_string();
        let headers = response.headers().clone();
        let bytes = match block_on(response.bytes()) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WEB_FETCH_URL,
                    "read_body_failed",
                    format!("读取响应正文失败：{}。", error),
                    Some("请确认目标地址返回的是可读取文本内容。".to_string()),
                )
            }
        };

        let body = decode_content(&headers, &bytes);

        ToolResult {
            tool_name: TOOL_WEB_FETCH_URL.to_string(),
            status: if status.is_success() {
                "ok".to_string()
            } else {
                "error".to_string()
            },
            output: json_string(json!({
                "ok": status.is_success(),
                "url": final_url,
                "statusCode": status.as_u16(),
                "contentPreview": preview_text(&body, 2000),
                "contentLength": body.len(),
                "error": (!status.is_success()).then(|| json!({
                    "code": "http_error",
                    "message": format!("抓取 URL 返回非成功状态码 {}。", status.as_u16()),
                    "hint": "请确认目标地址可访问，或检查服务端响应状态。"
                })),
                "summary": {
                    "text": format!("已抓取 URL {}，状态码 {}。", url, status.as_u16())
                }
            })),
            duration_ms: 0,
        }
    }

    fn web_search(&self, call: &ToolCall) -> ToolResult {
        let Some(query) = call.arguments.get("query").and_then(Value::as_str) else {
            return error_result(
                TOOL_WEB_SEARCH_QUERY,
                "missing_argument",
                "缺少必填参数 `query`。".to_string(),
                Some("参数示例：{\"query\":\"rust reqwest tutorial\",\"limit\":5}".to_string()),
            );
        };
        let query = query.trim();
        if query.is_empty() {
            return error_result(
                TOOL_WEB_SEARCH_QUERY,
                "empty_query",
                "参数 `query` 不能为空字符串。".to_string(),
                Some("请提供外部搜索关键词。".to_string()),
            );
        }

        let limit = call
            .arguments
            .get("limit")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 10) as usize)
            .unwrap_or(5);
        let timeout_ms = call
            .arguments
            .get("timeoutMs")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, 60_000))
            .unwrap_or(DEFAULT_WEB_TIMEOUT_MS);

        let api_key = match crate::agent::config::ProviderRegistryStore::new().get_service_api_key("exa") {
            Some(key) => key,
            None => {
                return error_result(
                    TOOL_WEB_SEARCH_QUERY,
                    "missing_api_key",
                    "未设置 EXA_API_KEY。请在设置页面输入 Exa API Key，或设置 EXA_API_KEY 环境变量。".to_string(),
                    Some("访问 https://dashboard.exa.ai 获取 API Key。".to_string()),
                )
            }
        };

        let client = match build_web_client(timeout_ms) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WEB_SEARCH_QUERY, "client_build_failed", error, None)
            }
        };

        let body = json!({
            "query": query,
            "type": "auto",
            "numResults": limit,
            "contents": {
                "highlights": true
            }
        });

        let response = match retry_tool_timeout(TOOL_WEB_SEARCH_QUERY, || {
            block_on(
                client
                    .post("https://api.exa.ai/search")
                    .header("x-api-key", &api_key)
                    .header("Content-Type", "application/json")
                    .body(body.to_string())
                    .send(),
            )
            .map_err(|error| {
                if is_reqwest_timeout_error(&error) {
                    format!("timeout: Exa 搜索请求超时：{}。", error)
                } else {
                    format!("Exa 搜索请求失败：{}。", error)
                }
            })
        }) {
            Ok(value) => value,
            Err(error) => {
                let code = if is_tool_timeout_message(&error) {
                    "timeout"
                } else {
                    "request_failed"
                };
                return error_result(
                    TOOL_WEB_SEARCH_QUERY,
                    code,
                    error,
                    Some("请确认 API Key 有效且网络可访问 api.exa.ai。".to_string()),
                )
            }
        };

        let status = response.status();
        let response_body = match block_on(response.text()) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WEB_SEARCH_QUERY,
                    "read_body_failed",
                    format!("读取 Exa 响应失败：{}。", error),
                    None,
                )
            }
        };

        let parsed: Value = match serde_json::from_str(&response_body) {
            Ok(value) => value,
            Err(_) => {
                return error_result(
                    TOOL_WEB_SEARCH_QUERY,
                    "api_error",
                    format!(
                        "Exa API 返回异常（状态码 {}）：{}",
                        status.as_u16(),
                        preview_text(&response_body, 200)
                    ),
                    Some("请检查 API Key 和查询参数。".to_string()),
                )
            }
        };

        if !status.is_success() {
            let error_msg = parsed
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("未知错误");
            return error_result(
                TOOL_WEB_SEARCH_QUERY,
                "api_error",
                format!(
                    "Exa API 错误（状态码 {}）：{}",
                    status.as_u16(),
                    error_msg
                ),
                Some("请检查 API Key 和查询参数。".to_string()),
            );
        }

        let results: Vec<Value> = parsed
            .get("results")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .map(|r| {
                        json!({
                            "title": r.get("title").and_then(Value::as_str).unwrap_or(""),
                            "url": r.get("url").and_then(Value::as_str).unwrap_or(""),
                            "snippet": r.get("highlights")
                                .and_then(Value::as_array)
                                .and_then(|h| h.first())
                                .and_then(Value::as_str)
                                .unwrap_or(""),
                            "score": r.get("score").and_then(Value::as_f64).unwrap_or(0.0),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        ToolResult {
            tool_name: TOOL_WEB_SEARCH_QUERY.to_string(),
            status: "ok".to_string(),
            output: json_string(json!({
                "ok": true,
                "query": query,
                "statusCode": status.as_u16(),
                "resultCount": results.len(),
                "results": results,
                "summary": {
                    "text": format!("已完成 Exa 搜索 `{}`，返回 {} 条结果。", query, results.len())
                }
            })),
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

    fn glob_files(&self, call: &ToolCall) -> ToolResult {
        let pattern = call
            .arguments
            .get("pattern")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if pattern.is_empty() {
            return error_result(
                TOOL_WORKSPACE_GLOB_FILES,
                "missing_argument",
                "缺少必填参数 `pattern`。".to_string(),
                Some(
                    "参数示例：{\"pattern\":\"src/**/*.rs\",\"path\":\".\",\"limit\":50}"
                        .to_string(),
                ),
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
            .map(|value| value.clamp(1, 200) as usize)
            .unwrap_or(50);

        let root_entry = match self.resolve_workspace_entry(relative_dir) {
            Ok(value) => value,
            Err(error) => {
                return error_result(TOOL_WORKSPACE_GLOB_FILES, "invalid_path", error, None)
            }
        };

        let mut files = if root_entry.is_file() {
            vec![root_entry.clone()]
        } else if root_entry.is_dir() {
            let mut collected = Vec::new();
            if let Err(error) =
                collect_files_recursively(&root_entry, &mut collected, MAX_SEARCH_FILES)
            {
                return error_result(
                    TOOL_WORKSPACE_GLOB_FILES,
                    "walk_failed",
                    format!("遍历目录失败：{}。", error),
                    Some("请缩小 path 范围后重试。".to_string()),
                );
            }
            collected
        } else {
            return error_result(
                TOOL_WORKSPACE_GLOB_FILES,
                "unsupported_path_kind",
                format!(
                    "当前路径类型不支持路径模式匹配：{}。",
                    self.display_workspace_relative(&root_entry)
                ),
                Some("请传入工作区内的文件或目录路径。".to_string()),
            );
        };
        files.sort();

        let mut matches = Vec::new();
        for file_path in files {
            if matches.len() >= limit {
                break;
            }
            let relative = self.display_workspace_relative(&file_path);
            if path_matches_filter(&relative, pattern) {
                matches.push(json!({
                    "path": relative,
                    "kind": "file"
                }));
            }
        }

        ToolResult {
            tool_name: TOOL_WORKSPACE_GLOB_FILES.to_string(),
            status: "ok".to_string(),
            output: json_string(json!({
                "pattern": pattern,
                "path": self.display_workspace_relative(&root_entry),
                "matchCount": matches.len(),
                "matches": matches,
            })),
            duration_ms: 0,
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
        let regex_mode = call
            .arguments
            .get("regex")
            .and_then(Value::as_bool)
            .unwrap_or(false);
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
                let matched = if regex_mode {
                    wildcard_match(&haystack, &normalized_query)
                } else {
                    haystack.contains(&normalized_query)
                };
                if matched {
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
                "regex": regex_mode,
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
            Some(plan),
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
                        arguments: json!({
                            "path": path,
                            "query": if query.is_empty() { Value::Null } else { Value::String(query.clone()) },
                            "limit": limit,
                            "lineCount": line_count,
                        }),
                        plan: None,
                    };
                    let result = self.gather_context(&nested_call);
                    (index, nested_call, result)
                })
                .collect::<Vec<_>>();
            if !skipped_paths.is_empty() {
                let skipped_call = ToolCall {
                    call_id: None,
                    name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
                    arguments: json!({
                        "paths": skipped_paths,
                        "limit": limit,
                        "lineCount": line_count,
                    }),
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
            let plan = build_multi_path_gather_plan(&gathered_paths, &query, limit, line_count);

            return self.aggregate_nested_results(
                TOOL_WORKSPACE_GATHER_CONTEXT,
                json!({
                    "mode": "multi_path",
                    "paths": gathered_paths,
                    "requestedPathCount": requested_path_count,
                    "skippedPaths": skipped_paths,
                    "limitApplied": requested_path_count > MAX_GATHER_CONTEXT_PATHS,
                    "pathLimit": MAX_GATHER_CONTEXT_PATHS,
                    "query": if query.is_empty() { Value::Null } else { Value::String(query) },
                }),
                Some(plan),
                nested,
            );
        }

        let path = if raw_path.is_empty() { "." } else { raw_path };
        let resolved = match self.resolve_workspace_entry(path) {
            Ok(value) => value,
            Err(error) => {
                let code = if error.contains("只允许访问当前工作区内的相对路径") {
                    "out_of_scope"
                } else {
                    "invalid_path"
                };
                return error_result(
                    TOOL_WORKSPACE_GATHER_CONTEXT,
                    code,
                    error,
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
                    plan: None,
                };
                let segment_call = ToolCall {
                    call_id: None,
                    name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                    arguments: json!({
                        "path": display_path,
                        "startLine": 1,
                        "lineCount": line_count,
                    }),
                    plan: None,
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
                                arguments: json!({
                                    "path": display_path,
                                    "startLine": 1,
                                    "lineCount": line_count,
                                }),
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
                        plan: None,
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
                        self.path_info(&path_info_call),
                    ));
                    let search_result = self.search_text(&search_call);
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
                        collected.push((2usize, list_call.clone(), self.list_files(&list_call)));
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

        self.aggregate_nested_results(
            TOOL_WORKSPACE_GATHER_CONTEXT,
            json!({
                "mode": mode,
                "path": display_path,
                "query": if query.is_empty() { Value::Null } else { Value::String(query) },
            }),
            Some(plan),
            nested,
        )
    }

    fn aggregate_nested_results(
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

    fn resolve_workspace_path(&self, raw_path: &str) -> Result<PathBuf, String> {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return Err("文件路径不能为空。".to_string());
        }

        let canonical = match self.canonicalize_workspace_target(trimmed) {
            Ok(canonical) => canonical,
            Err(primary_error) => match self.try_repair_file_path(trimmed) {
                Ok(Some(repaired)) => repaired,
                Ok(None) => return Err(primary_error),
                Err(repair_error) => return Err(repair_error),
            },
        };
        if !canonical.is_file() {
            return Err(format!(
                "目标不是文件：{}。",
                self.display_workspace_relative(&canonical)
            ));
        }

        Ok(canonical)
    }

    fn prepare_workspace_file_path(&self, raw_path: &str) -> Result<PathBuf, String> {
        let trimmed = raw_path.trim();
        if trimmed.is_empty() {
            return Err("文件路径不能为空。".to_string());
        }

        let input = PathBuf::from(trimmed);
        let candidate = if input.is_absolute() {
            input
        } else {
            self.workspace_root.join(trimmed)
        };

        let root = self.canonical_workspace_root();
        let parent = candidate.parent().unwrap_or(&self.workspace_root);
        let existing_ancestor = existing_workspace_ancestor_path(parent)
            .ok_or_else(|| format!("无法解析目标父目录 {}。", parent.display()))?;
        let canonical_ancestor = existing_ancestor.canonicalize().map_err(|error| {
            format!(
                "无法解析目标父目录 {}：{}",
                existing_ancestor.display(),
                error
            )
        })?;

        if !is_within_root(&root, &canonical_ancestor) {
            return Err("只允许写入当前工作区内的相对路径。".to_string());
        }

        let relative_suffix = candidate
            .strip_prefix(&existing_ancestor)
            .map_err(|_| "无法计算工作区内的目标路径后缀。".to_string())?;

        Ok(canonical_ancestor.join(relative_suffix))
    }

    fn resolve_workspace_entry(&self, raw_path: &str) -> Result<PathBuf, String> {
        let trimmed = if raw_path.trim().is_empty() {
            "."
        } else {
            raw_path.trim()
        };
        match self.canonicalize_workspace_target(trimmed) {
            Ok(canonical) => Ok(canonical),
            Err(primary_error) => match self.try_repair_file_path(trimmed) {
                Ok(Some(repaired)) => Ok(repaired),
                Ok(None) => Err(primary_error),
                Err(repair_error) => Err(repair_error),
            },
        }
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

    fn try_repair_file_path(&self, raw_path: &str) -> Result<Option<PathBuf>, String> {
        let normalized_raw_path = raw_path
            .trim()
            .trim_end_matches(['/', '\\'])
            .replace('\\', "/");
        let raw_path_object = Path::new(raw_path);
        let file_name = raw_path_object
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::trim)
            .filter(|value| !value.is_empty() && *value != "." && *value != "..");

        let Some(file_name) = file_name else {
            return Ok(None);
        };

        let root = self.canonical_workspace_root();
        let mut files = Vec::new();
        if collect_files_recursively(&root, &mut files, MAX_PATH_REPAIR_SEARCH_FILES).is_err() {
            return Ok(None);
        }

        let exact_name_matches = files
            .iter()
            .filter(|path| {
                path.file_name()
                    .and_then(|value| value.to_str())
                    .map(|value| value.eq_ignore_ascii_case(file_name))
                    .unwrap_or(false)
            })
            .cloned()
            .collect::<Vec<_>>();

        match exact_name_matches.len() {
            0 => {}
            1 => return Ok(exact_name_matches.into_iter().next()),
            _ => {
                let candidates = exact_name_matches
                    .iter()
                    .take(5)
                    .map(|path| self.display_workspace_relative(path))
                    .collect::<Vec<_>>()
                    .join(", ");
                return Err(format!(
                    "无法解析路径 {}：工作区内发现多个同名文件 {}，请提供更精确的相对路径。",
                    raw_path, candidates
                ));
            }
        }

        let stem_matches = files
            .iter()
            .filter(|path| {
                let relative = match path.strip_prefix(&root) {
                    Ok(value) => value,
                    Err(_) => return false,
                };
                let relative_without_extension = relative
                    .parent()
                    .map(|parent| parent.join(relative.file_stem().unwrap_or_default()))
                    .unwrap_or_else(|| PathBuf::from(relative.file_stem().unwrap_or_default()));

                relative_without_extension
                    .display()
                    .to_string()
                    .replace('\\', "/")
                    .eq_ignore_ascii_case(&normalized_raw_path)
                    || path
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .map(|value| value.eq_ignore_ascii_case(file_name))
                        .unwrap_or(false)
            })
            .cloned()
            .collect::<Vec<_>>();

        match stem_matches.len() {
            0 => Ok(None),
            1 => Ok(stem_matches.into_iter().next()),
            _ => {
                let candidates = stem_matches
                    .iter()
                    .take(5)
                    .map(|path| self.display_workspace_relative(path))
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(format!(
                    "无法解析路径 {}：工作区内发现多个缺扩展名候选文件 {}，请提供更精确的相对路径。",
                    raw_path, candidates
                ))
            }
        }
    }
}

impl ToolExecutor for ToolRouter {
    fn execute(&self, call: &ToolCall) -> ToolResult {
        ToolRouter::execute(self, call)
    }
}

impl ToolDefinition {
    pub fn contract_view(&self) -> ToolDefinitionContractView {
        ToolDefinitionContractView {
            name: model_visible_tool_name(self.name).to_string(),
            canonical_tool_name: product_canonical_tool_name(self.name).to_string(),
            execution_primitive: canonical_tool_name(self.name)
                .unwrap_or(self.name)
                .to_string(),
            description: self.description.to_string(),
            input_schema: self.input_schema.clone(),
            kind: tool_kind_for_name(self.name).as_str().to_string(),
            exposure: tool_exposure_for_name(self.name).as_str().to_string(),
            display_metadata: tool_display_metadata_for_name(self.name),
            permission_facts: default_permission_facts_for_name(self.name),
        }
    }
}

impl ToolCall {
    pub fn contract_view(&self) -> ToolCallContractView {
        ToolCallContractView {
            call_id: self.call_id.clone(),
            name: model_visible_tool_name(&self.name).to_string(),
            canonical_tool_name: product_canonical_tool_name(&self.name).to_string(),
            execution_primitive: canonical_tool_name(&self.name)
                .unwrap_or(self.name.as_str())
                .to_string(),
            arguments: self.arguments.clone(),
            plan: self.plan.clone(),
            kind: tool_kind_for_name(&self.name).as_str().to_string(),
            exposure: tool_exposure_for_name(&self.name).as_str().to_string(),
            display_metadata: tool_display_metadata_for_name(&self.name),
            permission_facts: default_permission_facts_for_name(&self.name),
        }
    }
}

impl ToolResult {
    pub fn contract_view(&self) -> ToolResultContractView {
        let parsed = parse_tool_output(&self.output);
        let summary = tool_result_summary_text(self, &parsed);
        ToolResultContractView {
            tool_name: model_visible_tool_name(&self.tool_name).to_string(),
            canonical_tool_name: product_canonical_tool_name(&self.tool_name).to_string(),
            execution_primitive: canonical_tool_name(&self.tool_name)
                .unwrap_or(self.tool_name.as_str())
                .to_string(),
            status: self.status.clone(),
            summary,
            data: parsed
                .is_object()
                .then_some(parsed.clone())
                .or_else(|| Some(Value::String(self.output.clone()))),
            error: tool_error_from_output(&self.status, &parsed),
            child_results: parsed
                .get("results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            artifacts: extract_tool_artifacts(&parsed),
            duration_ms: self.duration_ms,
            display_metadata: tool_display_metadata_for_name(&self.tool_name),
            permission_facts: permission_facts_from_output(&parsed)
                .unwrap_or_else(|| default_permission_facts_for_name(&self.tool_name)),
        }
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
                    "regex": {
                        "type": "boolean",
                        "description": "是否按增强模式匹配 query；当前 v1 使用通配符式匹配，默认 false"
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
            name: TOOL_WORKSPACE_GLOB_FILES,
            description: "按路径 pattern 递归匹配工作区内文件，适合大代码库中的文件发现。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "pattern": {
                        "type": "string",
                        "description": "要匹配的路径模式，例如 src/*.rs 或 *tool*"
                    },
                    "path": {
                        "type": "string",
                        "description": "搜索起点目录，默认为 ."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "最多返回多少条路径命中，默认 50"
                    }
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_WEB_FETCH_URL,
            description: "抓取指定 http/https URL 的正文内容预览，不承担搜索排序职责。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "要抓取的 http/https URL"
                    },
                    "timeoutMs": {
                        "type": "integer",
                        "description": "请求超时毫秒数，默认 15000"
                    }
                },
                "required": ["url"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_WEB_SEARCH_QUERY,
            description: "执行外部搜索并返回结构化结果列表，不把抓取和搜索混为一个工具。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "外部搜索关键词"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "最多返回多少条搜索结果，默认 5"
                    },
                    "timeoutMs": {
                        "type": "integer",
                        "description": "请求超时毫秒数，默认 15000"
                    }
                },
                "required": ["query"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_MCP_RESOURCE_READ,
            description: "通过 capability registry 读取指定 MCP 资源 capability 的只读内容，不混入普通工具执行。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "capabilityId": {
                        "type": "string",
                        "description": "目标 resource capability id，例如 mcp:resource:repo-index"
                    },
                    "arguments": {
                        "type": "object",
                        "description": "传给 resource capability 的结构化参数"
                    }
                },
                "required": ["capabilityId"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_TOOL_SEARCH,
            description: "搜索 capability registry 中可用的工具候选，作为 deferred / dynamic tool discovery 入口。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "可选查询词；为空时返回默认候选列表"
                    },
                    "sourceId": {
                        "type": "string",
                        "description": "可选 source id，用于缩小 discovery 范围"
                    },
                    "limit": {
                        "type": "integer",
                        "description": "最多返回多少条候选，默认 8"
                    }
                },
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_WRITE_FILE,
            description: "在当前工作区内新建或整文件覆写文本文件，可控制是否允许覆盖现有文件。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对文件路径"
                    },
                    "content": {
                        "type": "string",
                        "description": "要写入文件的完整文本内容"
                    },
                    "overwrite": {
                        "type": "boolean",
                        "description": "是否允许覆盖已存在文件，默认 true"
                    }
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_EDIT_FILE,
            description: "在当前工作区内按 oldText/newText 对文本文件做受控替换；默认只允许单一匹配。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "当前工作区内的相对文件路径"
                    },
                    "oldText": {
                        "type": "string",
                        "description": "需要被替换的原始文本"
                    },
                    "newText": {
                        "type": "string",
                        "description": "替换后的新文本"
                    },
                    "replaceAll": {
                        "type": "boolean",
                        "description": "是否允许替换全部匹配，默认 false"
                    }
                },
                "required": ["path", "oldText", "newText"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: TOOL_WORKSPACE_RUN_COMMAND,
            description: "在当前工作区内受控执行命令，返回 cwd、timeout、exitCode、stdout 和 stderr。",
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "要执行的命令文本"
                    },
                    "cwd": {
                        "type": "string",
                        "description": "执行命令时的工作区内相对目录，默认 ."
                    },
                    "timeoutMs": {
                        "type": "integer",
                        "description": "命令超时毫秒数，默认 10000，最大 120000"
                    }
                },
                "required": ["command"],
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

fn contract_priority(view: &ToolDefinitionContractView) -> u8 {
    match view.execution_primitive.as_str() {
        TOOL_TIME_NOW => 10,
        TOOL_ECHO_INPUT => 100,
        TOOL_WORKSPACE_LIST_FILES => 100,
        TOOL_WORKSPACE_SEARCH_TEXT => 100,
        TOOL_WORKSPACE_GLOB_FILES => 100,
        TOOL_WEB_FETCH_URL => 100,
        TOOL_WEB_SEARCH_QUERY => 100,
        TOOL_MCP_RESOURCE_READ => 100,
        TOOL_TOOL_SEARCH => 100,
        TOOL_WORKSPACE_WRITE_FILE => 100,
        TOOL_WORKSPACE_EDIT_FILE => 100,
        TOOL_WORKSPACE_RUN_COMMAND => 100,
        TOOL_WORKSPACE_BATCH => 100,
        TOOL_WORKSPACE_GATHER_CONTEXT => 100,
        TOOL_WORKSPACE_READ_FILE => 40,
        TOOL_WORKSPACE_READ_FILE_SEGMENT => 30,
        TOOL_WORKSPACE_PATH_INFO => 20,
        _ => 50,
    }
}

pub fn builtin_tool_contract_views() -> Vec<ToolDefinitionContractView> {
    let mut ordered_names: Vec<String> = Vec::new();
    let mut deduped: std::collections::HashMap<String, ToolDefinitionContractView> =
        std::collections::HashMap::new();

    for tool in builtin_tools() {
        let contract = tool.contract_view();
        let key = contract.name.clone();
        if let Some(existing) = deduped.get(&key) {
            if contract_priority(&contract) > contract_priority(existing) {
                deduped.insert(key, contract);
            }
            continue;
        }

        ordered_names.push(key.clone());
        deduped.insert(key, contract);
    }

    ordered_names
        .into_iter()
        .filter_map(|name| deduped.remove(&name))
        .collect()
}

#[cfg(test)]
mod contract_view_tests {
    use super::{
        builtin_tool_contract_views, canonical_tool_name, model_visible_tool_name,
        product_canonical_tool_name, TOOL_WORKSPACE_RUN_COMMAND,
    };

    #[test]
    fn builtin_tool_contract_views_deduplicate_to_model_surface() {
        let views = builtin_tool_contract_views();
        let names = views
            .iter()
            .map(|view| view.name.as_str())
            .collect::<Vec<_>>();
        let primitives = views
            .iter()
            .map(|view| view.execution_primitive.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "Run",
                "Ask",
                "Read",
                "List",
                "Search",
                "Glob",
                "WebFetch",
                "WebSearch",
                "MCPResource",
                "ToolSearch",
                "Write",
                "Edit",
                "Plan"
            ]
        );
        assert_eq!(
            primitives,
            vec![
                "workspace_run_command",
                "echo_input",
                "workspace_gather_context",
                "workspace_list_files",
                "workspace_search_text",
                "workspace_glob_files",
                "web_fetch_url",
                "web_search_query",
                "mcp_resource_read",
                "tool_search",
                "workspace_write_file",
                "workspace_edit_file",
                "workspace_batch"
            ]
        );
    }

    #[test]
    fn canonical_tool_name_keeps_run_as_product_name_but_allows_internal_run_shell() {
        assert_eq!(canonical_tool_name("Run"), Some(TOOL_WORKSPACE_RUN_COMMAND));
        assert_eq!(model_visible_tool_name(TOOL_WORKSPACE_RUN_COMMAND), "Run");
        assert_eq!(
            product_canonical_tool_name(TOOL_WORKSPACE_RUN_COMMAND),
            "Run"
        );
    }
}

pub(crate) fn canonical_tool_name(name: &str) -> Option<&'static str> {
    match name {
        "Run" => Some(TOOL_WORKSPACE_RUN_COMMAND),
        "Ask" => Some(TOOL_ECHO_INPUT),
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
        "Plan" => Some(TOOL_WORKSPACE_BATCH),
        TOOL_TIME_NOW | "time.now" => Some(TOOL_TIME_NOW),
        TOOL_ECHO_INPUT | "echo.input" => Some(TOOL_ECHO_INPUT),
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
        _ => None,
    }
}

pub fn model_visible_tool_name(name: &str) -> &'static str {
    match canonical_tool_name(name).unwrap_or(name) {
        TOOL_WORKSPACE_RUN_COMMAND => "Run",
        TOOL_TIME_NOW => "Run",
        TOOL_ECHO_INPUT => "Ask",
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
        TOOL_WORKSPACE_BATCH => "Plan",
        TOOL_WORKSPACE_GATHER_CONTEXT => "Read",
        _ => "Run",
    }
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
        TOOL_ECHO_INPUT => ToolKind::Interactive,
        TOOL_TIME_NOW | TOOL_WORKSPACE_RUN_COMMAND => ToolKind::Execute,
        _ => ToolKind::External,
    }
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
        | TOOL_TIME_NOW => ToolExposure::ModelVisible,
        TOOL_TOOL_SEARCH => ToolExposure::Deferred,
        TOOL_WORKSPACE_PATH_INFO => ToolExposure::Deferred,
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
        "Plan" => Some("计划".to_string()),
        "Ask" => Some("提问".to_string()),
        "Run" => Some("运行".to_string()),
        "Write" => Some("写入".to_string()),
        "Edit" => Some("编辑".to_string()),
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
        TOOL_TIME_NOW | TOOL_ECHO_INPUT => None,
        _ => None,
    };
    ToolPermissionFacts {
        requires_approval: Some(false),
        permission_scope: scope,
        host_mediated: Some(false),
        permission_profile: Some("builtin".to_string()),
        approval_mode: Some("none".to_string()),
        decision_source: Some("runtime".to_string()),
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

fn denied_run_command_reason(command: &str) -> Option<String> {
    let normalized = command.trim().to_lowercase();
    let tokens = normalized
        .split_whitespace()
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .collect::<Vec<_>>();

    if tokens.is_empty() {
        return None;
    }

    if is_rm_destructive_command(&tokens) {
        return Some("命令包含高风险删除模式 `rm -rf`，当前 runtime 已拒绝执行。".to_string());
    }
    if is_git_reset_hard_command(&tokens) {
        return Some(
            "命令包含高风险片段 `git reset --hard`，当前 runtime 已拒绝执行。".to_string(),
        );
    }
    if is_git_clean_force_command(&tokens) {
        return Some("命令包含高风险片段 `git clean -fd`，当前 runtime 已拒绝执行。".to_string());
    }

    let deny_markers = [
        "remove-item",
        " del ",
        "erase ",
        "rmdir /s",
        "mkfs",
        "shutdown",
        "reboot",
        "halt",
        "poweroff",
        "diskpart",
    ];

    let padded = format!(" {normalized} ");
    deny_markers
        .iter()
        .find(|marker| padded.contains(**marker))
        .map(|marker| format!("命令包含高风险片段 `{marker}`，当前 runtime 已拒绝执行。"))
        .or_else(|| {
            if is_windows_format_command(&tokens) {
                Some(
                    "命令包含高风险磁盘格式化模式 `format <drive>`，当前 runtime 已拒绝执行。"
                        .to_string(),
                )
            } else {
                None
            }
        })
}

fn is_rm_destructive_command(tokens: &[&str]) -> bool {
    if tokens.first().copied() != Some("rm") {
        return false;
    }
    let mut has_recursive = false;
    let mut has_force = false;
    for token in tokens.iter().skip(1) {
        if let Some(flags) = token.strip_prefix('-') {
            has_recursive |= flags.contains('r');
            has_force |= flags.contains('f');
        }
    }
    has_recursive && has_force
}

fn is_git_reset_hard_command(tokens: &[&str]) -> bool {
    if tokens.first().copied() != Some("git") {
        return false;
    }
    let has_reset = tokens.contains(&"reset");
    let has_hard = tokens
        .iter()
        .any(|token| *token == "--hard" || *token == "-h");
    has_reset && has_hard
}

fn is_git_clean_force_command(tokens: &[&str]) -> bool {
    if tokens.first().copied() != Some("git") {
        return false;
    }
    if !tokens.contains(&"clean") {
        return false;
    }
    tokens.iter().any(|token| {
        if let Some(flags) = token.strip_prefix('-') {
            flags.contains('f') && flags.contains('d')
        } else {
            false
        }
    })
}

fn is_windows_format_command(tokens: &[&str]) -> bool {
    if tokens.first().copied() != Some("format") {
        return false;
    }
    tokens.iter().skip(1).any(|token| {
        token.len() == 2
            && token.ends_with(':')
            && token
                .chars()
                .next()
                .map(|value| value.is_ascii_alphabetic())
                .unwrap_or(false)
    })
}

fn is_http_url(url: &str) -> bool {
    let normalized = url.trim().to_lowercase();
    normalized.starts_with("http://") || normalized.starts_with("https://")
}

fn build_web_client(timeout_ms: u64) -> Result<Client, String> {
    let mut builder = Client::builder()
        .timeout(Duration::from_millis(timeout_ms))
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36");

    if let Ok(proxy_url) = env::var("HTTPS_PROXY")
        .or_else(|_| env::var("https_proxy"))
        .or_else(|_| env::var("ALL_PROXY"))
        .or_else(|_| env::var("all_proxy"))
    {
        if let Ok(proxy) = reqwest::Proxy::https(&proxy_url) {
            builder = builder.proxy(proxy);
        }
    }

    builder
        .build()
        .map_err(|error| format!("创建 HTTP 客户端失败：{}。", error))
}

fn is_reqwest_timeout_error(error: &reqwest::Error) -> bool {
    error.is_timeout()
        || error
            .source()
            .map(|source| source.to_string().to_ascii_lowercase().contains("timed out"))
            .unwrap_or(false)
}

fn is_tool_timeout_message(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("timeout")
        || normalized.contains("timed out")
        || normalized.contains("deadline has elapsed")
}

fn retry_tool_timeout<T, F>(tool_name: &str, mut operation: F) -> Result<T, String>
where
    F: FnMut() -> Result<T, String>,
{
    let mut last_error = String::new();
    for attempt in 1..=TOOL_TIMEOUT_RETRY_MAX_ATTEMPTS {
        if attempt > 1 {
            let delay_ms = std::cmp::min(500 * (1 << (attempt - 2)), 8000);
            thread::sleep(Duration::from_millis(delay_ms));
        }

        match operation() {
            Ok(value) => return Ok(value),
            Err(error) => {
                last_error = error;
                if !is_tool_timeout_message(&last_error) {
                    return Err(last_error);
                }
                if attempt == TOOL_TIMEOUT_RETRY_MAX_ATTEMPTS {
                    return Err(last_error);
                }
                let _ = (tool_name, preview_text(&last_error, 180));
            }
        }
    }

    Err(last_error)
}

fn decode_content(headers: &reqwest::header::HeaderMap, bytes: &[u8]) -> String {
    let encoding = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|ct| {
            ct.split(';')
                .nth(1)
                .and_then(|s| s.split('=').nth(1))
                .map(|s| s.trim().trim_matches('"').to_lowercase())
                .filter(|s| !s.is_empty())
        })
        .and_then(|charset| Encoding::for_label(charset.as_bytes()));

    if let Some(enc) = encoding {
        let (cow, _, _) = enc.decode(bytes);
        return cow.into_owned();
    }

    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }

    let (cow, _, _) = GBK.decode(bytes);
    cow.into_owned()
}



fn spawn_workspace_command(command: &str, cwd: &Path) -> std::io::Result<std::process::Child> {
    if cfg!(windows) {
        Command::new("cmd")
            .arg("/C")
            .arg(command)
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    } else {
        Command::new("sh")
            .arg("-lc")
            .arg(command)
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
    }
}

fn existing_workspace_ancestor_path(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.exists() {
            return Some(candidate.to_path_buf());
        }
        current = candidate.parent();
    }
    None
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
            },
            "summary": {
                "text": message
            }
        })),
        duration_ms: 0,
    }
}

fn tool_result_summary_text(result: &ToolResult, parsed: &Value) -> String {
    parsed
        .get("summary")
        .and_then(|summary| summary.get("text"))
        .and_then(Value::as_str)
        .map(ToString::to_string)
        .or_else(|| {
            parsed
                .get("error")
                .and_then(|error| error.get("message"))
                .and_then(Value::as_str)
                .map(ToString::to_string)
        })
        .unwrap_or_else(|| {
            format!(
                "Tool `{}` finished with status `{}`.",
                result.tool_name, result.status
            )
        })
}

pub(crate) fn tool_error_from_output(status: &str, parsed: &Value) -> Option<ToolError> {
    if status == "ok" {
        return None;
    }

    let error = parsed.get("error")?;
    Some(ToolError {
        kind: error
            .get("kind")
            .or_else(|| error.get("code"))
            .and_then(Value::as_str)
            .unwrap_or("tool_error")
            .to_string(),
        message: error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("Tool execution failed.")
            .to_string(),
        details: error.get("details").cloned(),
        retryable: error.get("retryable").and_then(Value::as_bool),
        source: error
            .get("source")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

fn permission_facts_from_output(parsed: &Value) -> Option<ToolPermissionFacts> {
    let permission = parsed.get("permission")?;
    Some(ToolPermissionFacts {
        requires_approval: permission.get("requiresApproval").and_then(Value::as_bool),
        permission_scope: permission
            .get("permissionScope")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        host_mediated: permission.get("hostMediated").and_then(Value::as_bool),
        permission_profile: permission
            .get("permissionProfile")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        approval_mode: permission
            .get("approvalMode")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        decision_source: permission
            .get("decisionSource")
            .and_then(Value::as_str)
            .map(ToString::to_string),
    })
}

fn extract_tool_artifacts(parsed: &Value) -> Vec<Value> {
    let Some(path) = parsed.get("path").cloned() else {
        return Vec::new();
    };
    vec![json!({
        "kind": "path",
        "value": path,
    })]
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

fn build_batch_tool_plan(
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

fn build_multi_path_gather_plan(
    paths: &[String],
    query: &str,
    limit: usize,
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
                    "query": if query.trim().is_empty() { Value::Null } else { Value::String(query.to_string()) },
                    "limit": limit,
                    "lineCount": line_count,
                }),
                summary: format!("第 {} 个路径聚合：`{}`。", index + 1, path),
            })
            .collect(),
    }
}

fn build_nested_tool_plan(
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
    use std::io::{Read, Write};
    use std::net::TcpListener;
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
    fn time_now_returns_unix_timestamp_and_local_iso() {
        let router = ToolRouter::with_workspace_root(temp_workspace());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_TIME_NOW.to_string(),
            arguments: json!({}),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload: Value =
            serde_json::from_str(&result.output).expect("time_now output should be json");
        let unix_ts = payload
            .get("unixTimestampSeconds")
            .and_then(Value::as_u64)
            .expect("unixTimestampSeconds should exist");
        assert!(
            unix_ts > 1_700_000_000,
            "unix timestamp should be reasonable: {}",
            unix_ts
        );
        let local_iso = payload
            .get("localIso")
            .and_then(Value::as_str)
            .expect("localIso should exist");
        assert!(
            local_iso.contains("2026"),
            "localIso should contain current year: {}",
            local_iso
        );
        assert!(
            local_iso.len() > 16,
            "localIso should be a full datetime string"
        );
        let tz = payload
            .get("timezone")
            .and_then(Value::as_str)
            .expect("timezone should exist");
        assert!(
            tz.len() >= 3,
            "timezone offset should be present: {}",
            tz
        );
    }

    fn serve_single_http_response(status_line: &str, body: &str, content_type: &str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test http server");
        let address = listener.local_addr().expect("read test server addr");
        let status_line = status_line.to_string();
        let body = body.to_string();
        let content_type = content_type.to_string();

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept test request");
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "{status_line}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream
                .write_all(response.as_bytes())
                .expect("write test response");
            stream.flush().expect("flush test response");
        });

        format!("http://{}", address)
    }

    fn serve_timeout_http_response(delay_ms: u64) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind test http server");
        let address = listener.local_addr().expect("read test server addr");

        thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept test request");
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer);
            thread::sleep(Duration::from_millis(delay_ms));
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: 7\r\nConnection: close\r\n\r\ntimeout",
            );
            let _ = stream.flush();
        });

        format!("http://{}", address)
    }

    fn with_env_var<R>(key: &str, value: &str, operation: impl FnOnce() -> R) -> R {
        let previous = env::var(key).ok();
        unsafe {
            env::set_var(key, value);
        }
        let result = operation();
        match previous {
            Some(previous_value) => unsafe {
                env::set_var(key, previous_value);
            },
            None => unsafe {
                env::remove_var(key);
            },
        }
        result
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
            plan: None,
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
            plan: None,
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
            plan: None,
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
    fn gather_context_limits_too_many_paths_as_partial_result() {
        let workspace = temp_workspace();
        for index in 0..=MAX_GATHER_CONTEXT_PATHS {
            fs::write(workspace.join(format!("demo-{index}.rs")), "fn demo() {}\n")
                .expect("write demo");
        }
        let router = ToolRouter::with_workspace_root(workspace.clone());
        let paths = (0..=MAX_GATHER_CONTEXT_PATHS)
            .map(|index| Value::String(format!("demo-{index}.rs")))
            .collect::<Vec<_>>();

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
            arguments: json!({
                "paths": paths,
                "lineCount": 20
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
        assert_eq!(
            payload
                .get("meta")
                .and_then(|meta| meta.get("requestedPathCount"))
                .and_then(Value::as_u64),
            Some((MAX_GATHER_CONTEXT_PATHS + 1) as u64)
        );
        assert_eq!(
            payload.get("status").and_then(Value::as_str),
            Some("partial")
        );
        assert_eq!(
            payload.get("successCount").and_then(Value::as_u64),
            Some(MAX_GATHER_CONTEXT_PATHS as u64)
        );
        assert_eq!(payload.get("partialCount").and_then(Value::as_u64), Some(1));
        assert_eq!(
            payload
                .get("meta")
                .and_then(|meta| meta.get("skippedPaths"))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );
        assert_eq!(
            payload
                .get("results")
                .and_then(Value::as_array)
                .and_then(|results| results.last())
                .and_then(|entry| entry.get("output"))
                .and_then(|output| output.get("reason"))
                .and_then(Value::as_str),
            Some("too_many_paths")
        );
    }

    #[test]
    fn workspace_batch_accepts_seventeen_subcalls_for_repo_exploration() {
        let workspace = temp_workspace();
        for index in 0..17 {
            fs::write(
                workspace.join(format!("demo-{index}.txt")),
                format!("file {index}\n"),
            )
            .expect("write demo");
        }
        let router = ToolRouter::with_workspace_root(workspace.clone());
        let calls = (0..17)
            .map(|index| {
                json!({
                    "name": TOOL_WORKSPACE_PATH_INFO,
                    "arguments": { "path": format!("demo-{index}.txt") }
                })
            })
            .collect::<Vec<_>>();

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_BATCH.to_string(),
            arguments: json!({
                "parallel": true,
                "continueOnError": true,
                "calls": calls
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
        assert_eq!(
            payload.get("successCount").and_then(Value::as_u64),
            Some(17)
        );
        assert_eq!(payload.get("status").and_then(Value::as_str), Some("ok"));
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
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("search output json");
        assert_eq!(
            payload.get("pathKind").and_then(Value::as_str),
            Some("file")
        );
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
    fn glob_files_matches_paths_by_pattern() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("src/agent")).expect("create agent dir");
        fs::write(workspace.join("src/agent/tools.rs"), "pub fn demo() {}\n").expect("write tools");
        fs::write(
            workspace.join("src/agent/context.rs"),
            "pub struct AgentContext;\n",
        )
        .expect("write context");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GLOB_FILES.to_string(),
            arguments: json!({
                "pattern": "src/agent/*.rs",
                "path": ".",
                "limit": 10
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("glob output json");
        assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(2));
    }

    #[test]
    fn glob_files_respects_limit() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("src/agent")).expect("create agent dir");
        fs::write(workspace.join("src/agent/tools.rs"), "pub fn demo() {}\n").expect("write tools");
        fs::write(
            workspace.join("src/agent/context.rs"),
            "pub struct AgentContext;\n",
        )
        .expect("write context");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GLOB_FILES.to_string(),
            arguments: json!({
                "pattern": "src/agent/*.rs",
                "path": ".",
                "limit": 1
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("glob output json");
        assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(1));
    }

    #[test]
    fn search_text_supports_regex_like_wildcard_mode() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "alpha beta gamma\n").expect("write demo");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_SEARCH_TEXT.to_string(),
            arguments: json!({
                "query": "*beta*",
                "path": "demo.txt",
                "regex": true
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("search output json");
        assert_eq!(payload.get("matchCount").and_then(Value::as_u64), Some(1));
        assert_eq!(payload.get("regex").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn gather_context_search_file_includes_segment_even_without_match() {
        let workspace = temp_workspace();
        fs::write(
            workspace.join("demo.rs"),
            "fn main() {}\nprintln!(\"hi\");\n",
        )
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
            plan: None,
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
            plan: None,
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

    #[test]
    fn read_file_segment_repairs_unique_same_name_file_path() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("src-tauri")).expect("create src-tauri dir");
        fs::write(
            workspace.join("tauri.conf.json"),
            "{\n  \"productName\": \"Pony Agent\"\n}\n",
        )
        .expect("write tauri config");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
            arguments: json!({
                "path": "src-tauri/tauri.conf.json",
                "startLine": 1,
                "lineCount": 5
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        assert!(result.output.contains("tauri.conf.json"));
        assert!(result.output.contains("productName"));
    }

    #[test]
    fn read_file_segment_keeps_error_when_same_name_file_is_ambiguous() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("src-tauri")).expect("create src-tauri dir");
        fs::create_dir_all(workspace.join("nested")).expect("create nested dir");
        fs::write(workspace.join("tauri.conf.json"), "{}\n").expect("write root tauri config");
        fs::write(workspace.join("nested/tauri.conf.json"), "{}\n")
            .expect("write nested tauri config");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
            arguments: json!({
                "path": "src-tauri/tauri.conf.json",
                "startLine": 1,
                "lineCount": 5
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        assert!(result.output.contains("多个同名文件"));
    }

    #[test]
    fn gather_context_repairs_unique_missing_extension_file_path() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("src/agent")).expect("create agent dir");
        fs::write(
            workspace.join("src/agent/context.rs"),
            "pub struct AgentContext;\n",
        )
        .expect("write context file");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
            arguments: json!({
                "path": "src/agent/context"
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("gather output json");
        assert_eq!(payload.get("status").and_then(Value::as_str), Some("ok"));
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
    fn gather_context_keeps_error_when_missing_extension_path_is_ambiguous() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("src/agent")).expect("create agent dir");
        fs::create_dir_all(workspace.join("nested")).expect("create nested dir");
        fs::write(
            workspace.join("src/agent/context.rs"),
            "pub struct AgentContext;\n",
        )
        .expect("write first context file");
        fs::write(
            workspace.join("nested/context.rs"),
            "pub struct OtherContext;\n",
        )
        .expect("write second context file");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_GATHER_CONTEXT.to_string(),
            arguments: json!({
                "path": "context"
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        assert!(result.output.contains("多个缺扩展名候选文件"));
    }

    #[test]
    fn write_file_creates_new_file_in_workspace() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_WRITE_FILE.to_string(),
            arguments: json!({
                "path": "notes/demo.txt",
                "content": "hello pony"
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload = serde_json::from_str::<Value>(&result.output).expect("write output json");
        assert_eq!(
            payload.get("path").and_then(Value::as_str),
            Some("notes/demo.txt")
        );
        assert_eq!(
            fs::read_to_string(workspace.join("notes/demo.txt")).expect("read written file"),
            "hello pony"
        );
    }

    #[test]
    fn write_file_respects_overwrite_false_for_existing_file() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "original").expect("write original");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_WRITE_FILE.to_string(),
            arguments: json!({
                "path": "demo.txt",
                "content": "changed",
                "overwrite": false
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("write output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("file_exists")
        );
        assert_eq!(
            fs::read_to_string(workspace.join("demo.txt")).expect("read original"),
            "original"
        );
    }

    #[test]
    fn edit_file_requires_replace_all_for_multiple_matches() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "foo\nfoo\n").expect("write demo");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_EDIT_FILE.to_string(),
            arguments: json!({
                "path": "demo.txt",
                "oldText": "foo",
                "newText": "bar"
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("edit output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("multiple_matches")
        );
    }

    #[test]
    fn edit_file_replaces_all_matches_when_enabled() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "foo\nfoo\n").expect("write demo");
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_EDIT_FILE.to_string(),
            arguments: json!({
                "path": "demo.txt",
                "oldText": "foo",
                "newText": "bar",
                "replaceAll": true
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        assert_eq!(
            fs::read_to_string(workspace.join("demo.txt")).expect("read edited file"),
            "bar\nbar\n"
        );
    }

    #[test]
    fn edit_file_returns_no_match_when_old_text_missing() {
        let workspace = temp_workspace();
        fs::write(workspace.join("demo.txt"), "alpha\nbeta\n").expect("write demo");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_EDIT_FILE.to_string(),
            arguments: json!({
                "path": "demo.txt",
                "oldText": "missing",
                "newText": "gamma"
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("edit output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("no_match")
        );
    }

    #[test]
    fn run_command_returns_stdout_and_exit_code() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace.clone());

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
            arguments: json!({
                "command": "echo hello",
                "cwd": ".",
                "timeoutMs": 5000
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        assert_eq!(result.tool_name, TOOL_WORKSPACE_RUN_COMMAND);
        let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
        assert_eq!(
            payload.get("delegateTool").and_then(Value::as_str),
            Some("run_shell")
        );
        assert_eq!(payload.get("exitCode").and_then(Value::as_i64), Some(0));
        assert!(payload
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_lowercase()
            .contains("hello"));
    }

    #[test]
    fn ask_returns_fallback_clarification_when_text_missing() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_ECHO_INPUT.to_string(),
            arguments: json!({
                "question": "请确认要继续执行哪一步？"
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload =
            serde_json::from_str::<Value>(&result.output).expect("ask fallback output json");
        assert_eq!(
            payload.get("mode").and_then(Value::as_str),
            Some("fallback_clarification")
        );
        assert_eq!(
            payload.get("prompt").and_then(Value::as_str),
            Some("请确认要继续执行哪一步？")
        );
    }

    #[test]
    fn ask_keeps_plain_text_output_for_normal_echo_path() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_ECHO_INPUT.to_string(),
            arguments: json!({
                "text": "hello"
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        assert_eq!(result.output, "echo_input 返回：hello");
    }

    #[test]
    fn ask_returns_error_when_text_and_question_both_missing() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_ECHO_INPUT.to_string(),
            arguments: json!({}),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("ask error output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("missing_argument")
        );
    }

    #[test]
    fn run_command_denies_high_risk_commands() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
            arguments: json!({
                "command": "rm -rf ."
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("command_denied")
        );
    }

    #[test]
    fn run_command_denies_rm_with_split_flags() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
            arguments: json!({
                "command": "rm -r -f ."
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("command_denied")
        );
    }

    #[test]
    fn run_command_returns_error_for_non_zero_exit_code() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        let command = if cfg!(windows) { "exit /b 7" } else { "exit 7" };

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_RUN_COMMAND.to_string(),
            arguments: json!({
                "command": command
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("run output json");
        assert_eq!(payload.get("exitCode").and_then(Value::as_i64), Some(7));
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("non_zero_exit")
        );
    }

    #[test]
    fn batch_rejects_nested_workspace_batch_calls() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_BATCH.to_string(),
            arguments: json!({
                "calls": [
                    {
                        "name": "workspace_batch",
                        "arguments": {
                            "calls": [
                                {
                                    "name": "workspace_list_files",
                                    "arguments": { "path": "." }
                                }
                            ]
                        }
                    }
                ]
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("batch output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("nested_batch_not_allowed")
        );
    }

    #[test]
    fn web_fetch_rejects_non_http_urls() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WEB_FETCH_URL.to_string(),
            arguments: json!({
                "url": "file:///etc/passwd"
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output).expect("web fetch output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("invalid_url")
        );
    }

    #[test]
    fn web_search_rejects_empty_query() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WEB_SEARCH_QUERY.to_string(),
            arguments: json!({
                "query": "   "
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload =
            serde_json::from_str::<Value>(&result.output).expect("web search output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("empty_query")
        );
    }

    #[test]
    fn web_fetch_returns_success_payload_for_http_response() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        let url = serve_single_http_response(
            "HTTP/1.1 200 OK",
            "<html><body><h1>Hello Pony</h1><p>Fetch success path.</p></body></html>",
            "text/html; charset=utf-8",
        );

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WEB_FETCH_URL.to_string(),
            arguments: json!({
                "url": url,
                "timeoutMs": 5000
            }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload =
            serde_json::from_str::<Value>(&result.output).expect("web fetch success output json");
        assert_eq!(payload.get("statusCode").and_then(Value::as_u64), Some(200));
        assert!(payload
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .starts_with("http://127.0.0.1:"));
        assert!(payload
            .get("contentPreview")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("Hello Pony"));
        assert!(payload.get("error").is_none() || payload.get("error") == Some(&Value::Null));
    }

    #[test]
    fn web_fetch_returns_structured_http_error_for_non_2xx_response() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        let url = serve_single_http_response(
            "HTTP/1.1 404 Not Found",
            "<html><body>missing</body></html>",
            "text/html; charset=utf-8",
        );

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WEB_FETCH_URL.to_string(),
            arguments: json!({
                "url": url,
                "timeoutMs": 5000
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output)
            .expect("web fetch http error output json");
        assert_eq!(payload.get("statusCode").and_then(Value::as_u64), Some(404));
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("http_error")
        );
        assert!(payload
            .get("contentPreview")
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("missing"));
    }

    #[test]
    fn web_fetch_returns_timeout_error_after_retries() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        let url = serve_timeout_http_response(150);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WEB_FETCH_URL.to_string(),
            arguments: json!({
                "url": url,
                "timeoutMs": 50
            }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output)
            .expect("web fetch timeout output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("timeout")
        );
        assert!(payload
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("timeout"));
    }

    #[test]
    fn web_search_returns_timeout_error_after_retries() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);
        let base_url = serve_timeout_http_response(150);

        let result = with_env_var("EXA_API_KEY", "test-key", || {
            with_env_var("EXA_API_BASE_URL", &base_url, || {
                router.execute(&ToolCall {
                    call_id: None,
                    name: TOOL_WEB_SEARCH_QUERY.to_string(),
                    arguments: json!({
                        "query": "pony agent",
                        "timeoutMs": 50
                    }),
                    plan: None,
                })
            })
        });

        assert_eq!(result.status, "error");
        let payload = serde_json::from_str::<Value>(&result.output)
            .expect("web search timeout output json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|error| error.get("code"))
                .and_then(Value::as_str),
            Some("timeout")
        );
        assert!(payload
            .get("error")
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .contains("timeout"));
    }

    // ── workspace_read_file ─────────────────────────────────────────

    #[test]
    fn read_file_returns_file_content() {
        let workspace = temp_workspace();
        fs::write(workspace.join("hello.txt"), "Hello Pony Agent!\n").expect("write file");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE.to_string(),
            arguments: json!({ "path": "hello.txt" }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        assert!(result.output.contains("Hello Pony Agent!"));
    }

    #[test]
    fn read_file_rejects_missing_path_argument() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE.to_string(),
            arguments: json!({}),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload: Value =
            serde_json::from_str(&result.output).expect("error output should be json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str),
            Some("missing_argument")
        );
    }

    #[test]
    fn read_file_rejects_path_traversal() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE.to_string(),
            arguments: json!({ "path": "../../../etc/passwd" }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload: Value =
            serde_json::from_str(&result.output).expect("error output should be json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str),
            Some("invalid_path")
        );
    }

    #[test]
    fn read_file_rejects_nonexistent_file() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE.to_string(),
            arguments: json!({ "path": "does-not-exist.txt" }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload: Value =
            serde_json::from_str(&result.output).expect("error output should be json");
        // canonicalize fails first → invalid_path (not metadata_failed)
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str),
            Some("invalid_path")
        );
    }

    #[test]
    fn read_file_rejects_file_too_large() {
        let workspace = temp_workspace();
        let large = vec![b'X'; 120_001];
        fs::write(workspace.join("large.bin"), &large).expect("write large file");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE.to_string(),
            arguments: json!({ "path": "large.bin" }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload: Value =
            serde_json::from_str(&result.output).expect("error output should be json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str),
            Some("file_too_large")
        );
    }

    #[test]
    fn read_file_rejects_non_utf8_content() {
        let workspace = temp_workspace();
        // Write invalid UTF-8 bytes that fs::read_to_string will reject
        let bad_bytes = [0xFF, 0xFE, 0x00, 0x68, 0x69];
        fs::write(workspace.join("binary.bin"), &bad_bytes).expect("write binary file");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_READ_FILE.to_string(),
            arguments: json!({ "path": "binary.bin" }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload: Value =
            serde_json::from_str(&result.output).expect("error output should be json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str),
            Some("read_failed")
        );
    }

    // ── workspace_path_info ─────────────────────────────────────────

    #[test]
    fn path_info_returns_file_metadata() {
        let workspace = temp_workspace();
        fs::write(workspace.join("info.txt"), "metadata test").expect("write file");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_PATH_INFO.to_string(),
            arguments: json!({ "path": "info.txt" }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload: Value =
            serde_json::from_str(&result.output).expect("path_info output should be json");
        assert_eq!(
            payload.get("kind").and_then(Value::as_str),
            Some("file")
        );
        assert!(payload.get("sizeBytes").and_then(Value::as_u64).unwrap_or(0) > 0);
        assert!(payload.get("modifiedUnixSeconds").and_then(Value::as_u64).is_some());
        assert_eq!(payload.get("childCount"), Some(&Value::Null));
    }

    #[test]
    fn path_info_returns_directory_metadata() {
        let workspace = temp_workspace();
        fs::create_dir_all(workspace.join("subdir")).expect("create subdir");
        fs::write(workspace.join("subdir/a.txt"), "a").expect("write a");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_PATH_INFO.to_string(),
            arguments: json!({ "path": "subdir" }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        let payload: Value =
            serde_json::from_str(&result.output).expect("path_info output should be json");
        assert_eq!(
            payload.get("kind").and_then(Value::as_str),
            Some("directory")
        );
        assert_eq!(payload.get("childCount").and_then(Value::as_u64), Some(1));
    }

    #[test]
    fn path_info_rejects_invalid_path() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_PATH_INFO.to_string(),
            arguments: json!({ "path": "../../../etc" }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload: Value =
            serde_json::from_str(&result.output).expect("error output should be json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str),
            Some("invalid_path")
        );
    }

    // ── workspace_list_files ────────────────────────────────────────

    #[test]
    fn list_files_returns_directory_entries() {
        let workspace = temp_workspace();
        fs::write(workspace.join("alpha.txt"), "alpha").expect("write alpha");
        fs::create_dir_all(workspace.join("beta")).expect("create beta dir");
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_LIST_FILES.to_string(),
            arguments: json!({ "path": "." }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        assert!(result.output.contains("alpha.txt"));
        assert!(result.output.contains("beta/"));
    }

    #[test]
    fn list_files_respects_limit() {
        let workspace = temp_workspace();
        for i in 0..5 {
            fs::write(workspace.join(format!("file-{i}.txt")), &format!("{i}"))
                .expect("write file");
        }
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_LIST_FILES.to_string(),
            arguments: json!({ "path": ".", "limit": 2 }),
            plan: None,
        });

        assert_eq!(result.status, "ok");
        assert!(result.output.contains("当前展示前 2 个"));
    }

    #[test]
    fn list_files_rejects_path_traversal() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_LIST_FILES.to_string(),
            arguments: json!({ "path": "../../../etc" }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload: Value =
            serde_json::from_str(&result.output).expect("error output should be json");
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str),
            Some("invalid_path")
        );
    }

    #[test]
    fn list_files_rejects_nonexistent_directory() {
        let workspace = temp_workspace();
        let router = ToolRouter::with_workspace_root(workspace);

        let result = router.execute(&ToolCall {
            call_id: None,
            name: TOOL_WORKSPACE_LIST_FILES.to_string(),
            arguments: json!({ "path": "nonexistent-dir" }),
            plan: None,
        });

        assert_eq!(result.status, "error");
        let payload: Value =
            serde_json::from_str(&result.output).expect("error output should be json");
        // canonicalize fails first → invalid_path (not read_failed;
        // read_failed would require an existing directory with denied
        // read permission, which is platform-specific to test)
        assert_eq!(
            payload
                .get("error")
                .and_then(|e| e.get("code"))
                .and_then(Value::as_str),
            Some("invalid_path")
        );
    }

    #[test]
    fn tool_timeout_message_classifier_matches_timeout_errors() {
        assert!(is_tool_timeout_message("timeout: request timed out"));
        assert!(is_tool_timeout_message("deadline has elapsed"));
        assert!(!is_tool_timeout_message("request failed"));
    }

    #[test]
    fn retry_tool_timeout_stops_after_non_timeout_error() {
        let attempts = std::cell::Cell::new(0);
        let result: Result<(), String> = retry_tool_timeout(TOOL_WEB_FETCH_URL, || {
            attempts.set(attempts.get() + 1);
            Err("request failed".to_string())
        });

        assert!(result.is_err());
        assert_eq!(attempts.get(), 1);
    }

    #[test]
    fn retry_tool_timeout_retries_until_success() {
        let attempts = std::cell::Cell::new(0);
        let result = retry_tool_timeout(TOOL_WEB_FETCH_URL, || {
            let next = attempts.get() + 1;
            attempts.set(next);
            if next < 3 {
                Err("timeout: request timed out".to_string())
            } else {
                Ok("ok")
            }
        })
        .expect("timeout retry should eventually succeed");

        assert_eq!(result, "ok");
        assert_eq!(attempts.get(), 3);
    }
}
