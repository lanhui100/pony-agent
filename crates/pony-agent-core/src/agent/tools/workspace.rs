//! workspace 文件工具面：读/写/编辑/列目录/path_info/glob/搜索，以及分段读取与
//! start_line 三态解析。路径解析与判定见 `paths`。
use super::*;
use super::exec::error_result;
use super::exec::json_string;
use super::paths::compile_path_glob;
use super::paths::glob_matches_path_or_basename;
use super::paths::truncate_preview;

pub(super) const MAX_FULL_READ_BYTES: u64 = 120_000;
pub(super) const MAX_SEGMENT_LINES: usize = 400;
pub(super) const DEFAULT_SEGMENT_LINES: usize = 80;
pub(super) const DEFAULT_LIST_LIMIT: usize = 40;
pub(super) const SUMMARY_ITEM_LIMIT: usize = 3;

impl ToolRouter {
    pub(super) fn write_file(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let ws_id = self.extract_call_workspace_id(call, context);
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

        let target = match self.prepare_workspace_file_path(relative_path, context, ws_id) {
            Ok(value) => value,
            Err((code, message)) => {
                return error_result(TOOL_WORKSPACE_WRITE_FILE, &code, message, None)
            }
        };

        let existed_before = target.exists();
        let expected_hash = call.arguments.get("expectedHash").or_else(|| call.arguments.get("expected_hash")).and_then(Value::as_str);

        if existed_before {
            // W37 barrier 修补（Lead）：`overwrite=false` 是调用方显式拒绝覆盖，
            // 必须先于观察门判定——否则该语义路径被 fs_not_observed 吞掉，
            // 既有契约 `file_exists` 错误码不可达（wave-37 lib 回归暴露）。
            if expected_hash.is_none() && !overwrite {
                return error_result(
                    TOOL_WORKSPACE_WRITE_FILE,
                    "file_exists",
                    format!(
                        "目标文件已存在：{}。",
                        self.display_resolved_workspace_relative(&target, context, ws_id)
                    ),
                    Some("如需覆盖，请显式传入 {\"overwrite\": true}；覆盖写入需先 Read 获取 expectedHash。".to_string()),
                );
            }
            // W37-C2 读前写观察门（与 edit_file wave-22 同语义）：对已存在文件的覆盖写入，
            // 在未携带 expectedHash 且环境未显式豁免（PONY_ALLOW_UNOBSERVED_EDIT）时 fail-closed，
            // 杜绝"未观察旧版本就静默覆盖"的丢写风险。新文件路径不设门。
            if expected_hash.is_none()
                && std::env::var("PONY_ALLOW_UNOBSERVED_EDIT")
                    .map(|v| v != "1" && v != "true")
                    .unwrap_or(true)
            {
                let display_path =
                    self.display_resolved_workspace_relative(&target, context, ws_id);
                return ToolResult {
                    tool_name: TOOL_WORKSPACE_WRITE_FILE.to_string(),
                    status: "error".to_string(),
                    output: json_string(json!({
                        "ok": false,
                        "code": "fs_not_observed",
                        "message": format!("文件 {} 已存在但未传入 expectedHash，禁止未观察覆盖写入。", display_path),
                        "details": {
                            "path": display_path,
                            "recoveryHint": {
                                "action": "read_then_write",
                                "instruction": "请先调用 workspace_read_file 读取当前文件内容并取其 contentHash，随后携带该 expectedHash 重试写入；或显式设置环境变量 PONY_ALLOW_UNOBSERVED_EDIT=1 豁免本门。"
                            }
                        }
                    })),
                    duration_ms: 0,
                };
            }
            if let Some(expected) = expected_hash {
                let current_content = match fs::read_to_string(&target) {
                    Ok(c) => c,
                    Err(e) => {
                        return error_result(
                            TOOL_WORKSPACE_WRITE_FILE,
                            "read_for_cas_failed",
                            format!("无法读取现有文件以执行 CAS 校验：{}。", e),
                            None,
                        )
                    }
                };
                let actual_hash = compute_content_sha256(&current_content);
                if actual_hash != expected {
                    return ToolResult {
                        tool_name: TOOL_WORKSPACE_WRITE_FILE.to_string(),
                        status: "error".to_string(),
                        output: json_string(json!({
                            "ok": false,
                            "code": "cas_conflict",
                            "message": "文件已被外部修改（CAS 校验失败）。",
                            "details": {
                                "path": self.display_resolved_workspace_relative(&target, context, ws_id),
                                "expectedHash": expected,
                                "actualHash": actual_hash,
                                "recoveryHint": {
                                    "action": "rebase_and_retry",
                                    "instruction": "请先调用 workspace_read_file 重新拉取最新内容，在最新版本基础上重试写入。",
                                    "nextSuggestedCall": {
                                        "name": "workspace_read_file",
                                        "arguments": { "path": self.display_resolved_workspace_relative(&target, context, ws_id) }
                                    }
                                }
                            }
                        })),
                        duration_ms: 0,
                    };
                }
            } else if !overwrite {
                return error_result(
                    TOOL_WORKSPACE_WRITE_FILE,
                    "file_exists",
                    format!(
                        "目标文件已存在：{}。",
                        self.display_resolved_workspace_relative(&target, context, ws_id)
                    ),
                    Some("如需覆盖，请显式传入 {\"overwrite\": true}。".to_string()),
                );
            }
        } else if let Some(expected) = expected_hash {
            return ToolResult {
                tool_name: TOOL_WORKSPACE_WRITE_FILE.to_string(),
                status: "error".to_string(),
                output: json_string(json!({
                    "ok": false,
                    "code": "cas_conflict",
                    "message": "CAS 校验指定了 expectedHash，但目标文件不存在。",
                    "details": {
                        "path": self.display_resolved_workspace_relative(&target, context, ws_id),
                        "expectedHash": expected,
                        "actualHash": null,
                        "recoveryHint": {
                            "action": "remove_hash_for_new_file",
                            "instruction": "新建文件无需传入 expectedHash，请移除后重试。"
                        }
                    }
                })),
                duration_ms: 0,
            };
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

        if let Err(error) = super::write_with_rewind_capture(context, &target, content.as_bytes()) {
            return error_result(
                TOOL_WORKSPACE_WRITE_FILE,
                "write_failed",
                format!("写入文件失败：{}。", error),
                Some("请确认目标文件可写，且当前进程有写权限。".to_string()),
            );
        }

        let content_hash = compute_content_sha256(content);

        ToolResult {
            tool_name: TOOL_WORKSPACE_WRITE_FILE.to_string(),
            status: "ok".to_string(),
            output: json_string(json!({
                "ok": true,
                "path": self.display_resolved_workspace_relative(&target, context, ws_id),
                "absolutePath": target.display().to_string(),
                "contentHash": content_hash,
                "bytesWritten": content.len(),
                "overwroteExisting": existed_before,
                "summary": {
                    "text": format!("已写入文件 {}。", self.display_resolved_workspace_relative(&target, context, ws_id))
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

    pub(super) fn edit_file(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let ws_id = self.extract_call_workspace_id(call, context);
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

        // PA-080 修复（code-review P0）：edit_file 是写操作，目标解析必须走 Write 判定
        // （classify_path(Write)），否则"只读授权"的外部路径会被改写，击穿写边界。
        // 与 tasks.md「workspace_edit_file 改调 classify_path(Write)」一致。
        let target = match self.prepare_workspace_file_path(path, context, ws_id) {
            Ok(value) => value,
            Err((code, message)) => {
                return error_result(TOOL_WORKSPACE_EDIT_FILE, &code, message, None)
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

        // PA-076 wave-22 读前写物理硬不变量：
        // 若环境未开启遗留宽松模式（PONY_ALLOW_UNOBSERVED_EDIT），任何编辑操作必须校验 expected_hash，
        // 或者如果既没有 expected_hash 且环境要求观察，则必须通过 fs_not_observed 阻断。
        let expected_hash = call.arguments.get("expectedHash").or_else(|| call.arguments.get("expected_hash")).and_then(Value::as_str);
        if expected_hash.is_none() && std::env::var("PONY_ALLOW_UNOBSERVED_EDIT").map(|v| v != "1" && v != "true").unwrap_or(true) {
            return error_result(
                TOOL_WORKSPACE_EDIT_FILE,
                "fs_not_observed",
                format!("文件 {} 尚未被观察（未传入 expectedHash），禁止直接编辑覆盖。", self.display_workspace_relative(&target)),
                Some("请先调用 workspace_read_file 读取文件并传入预期的内容 hash，确保在最新版本上进行编辑。".to_string()),
            );
        }

        if let Some(expected) = expected_hash {
            let actual_hash = compute_content_sha256(&original);
            if actual_hash != expected {
                return ToolResult {
                    tool_name: TOOL_WORKSPACE_EDIT_FILE.to_string(),
                    status: "error".to_string(),
                    output: json_string(json!({
                        "ok": false,
                        "code": "cas_conflict",
                        "message": "文件已被外部修改（CAS 校验失败）。",
                        "details": {
                            "path": self.display_workspace_relative(&target),
                            "expectedHash": expected,
                            "actualHash": actual_hash,
                            "recoveryHint": {
                                "action": "rebase_and_retry",
                                "instruction": "请先调用 workspace_read_file 重新拉取最新内容，在最新版本基础上重新计算 oldText 与 newText 后重试。",
                                "nextSuggestedCall": {
                                    "name": "workspace_read_file",
                                    "arguments": { "path": self.display_workspace_relative(&target) }
                                }
                            }
                        }
                    })),
                    duration_ms: 0,
                };
            }
        }

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

        if let Err(error) =
            super::write_with_rewind_capture(context, &target, updated.as_bytes())
        {
            return error_result(
                TOOL_WORKSPACE_EDIT_FILE,
                "write_failed",
                format!("写回文件失败：{}。", error),
                Some("请确认目标文件可写，且当前进程有写权限。".to_string()),
            );
        }

        let content_hash = compute_content_sha256(&updated);

        ToolResult {
            tool_name: TOOL_WORKSPACE_EDIT_FILE.to_string(),
            status: "ok".to_string(),
            output: json_string(json!({
                "ok": true,
                "path": self.display_workspace_relative(&target),
                "absolutePath": target.display().to_string(),
                "contentHash": content_hash,
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
}

impl ToolRouter {
    pub(super) fn read_file(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let ws_id = self.extract_call_workspace_id(call, context);
        let Some(path) = call.arguments.get("path").and_then(Value::as_str) else {
            return error_result(
                TOOL_WORKSPACE_READ_FILE,
                "missing_argument",
                "缺少必填参数 `path`。".to_string(),
                Some("参数示例：{\"path\":\"src-tauri/src/agent/tools.rs\"}".to_string()),
            );
        };

        let resolved = match self.resolve_workspace_path(path, context, ws_id) {
            Ok(value) => value,
            Err((code, message)) => {
                return error_result(TOOL_WORKSPACE_READ_FILE, &code, message, None)
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
            // W26：超过整文件内联上限时不再直接失败，而是落盘溢出并返回 content_ref，
            // 由 `spill_read` 按需回读（对比基线：claude-code toolResultStorage / DSH spill）。
            if metadata.len() > crate::agent::spill::MAX_SPILL_BYTES {
                return error_result(
                    TOOL_WORKSPACE_READ_FILE,
                    "file_too_large",
                    format!(
                        "文件 {} 大小为 {} bytes，超过落盘溢出上限 {} bytes。",
                        self.display_resolved_workspace_relative(&resolved, context, ws_id),
                        metadata.len(),
                        crate::agent::spill::MAX_SPILL_BYTES
                    ),
                    Some(
                        "请改用 workspace_read_file_segment 分段读取，或用 workspace_gather_context 获取概览。"
                            .to_string(),
                    ),
                );
            }

            let bytes = match fs::read(&resolved) {
                Ok(value) => value,
                Err(error) => {
                    return error_result(
                        TOOL_WORKSPACE_READ_FILE,
                        "read_failed",
                        format!("读取文件失败：{}。", error),
                        Some("请确认目标文件存在，且当前进程有读取权限。".to_string()),
                    )
                }
            };

            return match crate::agent::spill::spill_bytes(&bytes) {
                Ok(spilled) => ToolResult {
                    tool_name: TOOL_WORKSPACE_READ_FILE.to_string(),
                    status: "ok".to_string(),
                    output: json_string(json!({
                        "ok": true,
                        "tool": TOOL_WORKSPACE_READ_FILE,
                        "spilled": true,
                        "content_ref": spilled.ref_id,
                        "bytes": spilled.bytes,
                        "preview": spilled.preview,
                        "note": "文件超过整文件内联上限，已落盘溢出。回读方式：先调用 tool_search（关键词 spill/SpillRead）把 `SpillRead` 提升到本回合工具面，再用 spill_read(ref_id, offset, limit) 按需读取全文。"
                    })),
                    duration_ms: 0,
                },
                Err(err) => error_result(
                    TOOL_WORKSPACE_READ_FILE,
                    err.code(),
                    err.message().to_string(),
                    Some("请改用 workspace_read_file_segment 分段读取。".to_string()),
                ),
            };
        }

        match fs::read_to_string(&resolved) {
            Ok(content) => {
                let content_hash = compute_content_sha256(&content);
                ToolResult {
                    tool_name: TOOL_WORKSPACE_READ_FILE.to_string(),
                    status: "ok".to_string(),
                    output: format!(
                        "文件 {} 读取成功 (content_hash: {})。\n\n{}",
                        self.display_resolved_workspace_relative(&resolved, context, ws_id),
                        content_hash,
                        truncate_preview(&content, 4000)
                    ),
                    duration_ms: 0,
                }
            }
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

    pub(super) fn read_file_segment(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let ws_id = self.extract_call_workspace_id(call, context);
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

        let start_line = match parse_start_line_arg(&call.arguments) {
            StartLineArg::Valid(value) => value,
            // 三态契约：Missing/Invalid 统一回退到 1（Invalid 留痕在 gather 层，
            // segment 本体沿用原“静默回退”行为，不加字段）。
            StartLineArg::Missing | StartLineArg::Invalid { .. } => 1,
        };
        let line_count = call
            .arguments
            .get("lineCount")
            .and_then(Value::as_u64)
            .map(|value| value.clamp(1, MAX_SEGMENT_LINES as u64) as usize)
            .unwrap_or(40);

        let resolved = match self.resolve_workspace_path(path, context, ws_id) {
            Ok(value) => value,
            Err((code, message)) => {
                return error_result(TOOL_WORKSPACE_READ_FILE_SEGMENT, &code, message, None)
            }
        };

        match read_file_lines(&resolved, start_line, line_count) {
            Ok(FileSegment::Empty) => ToolResult {
                tool_name: TOOL_WORKSPACE_READ_FILE_SEGMENT.to_string(),
                status: "ok".to_string(),
                output: format!("文件 {} 为空。", self.display_resolved_workspace_relative(&resolved, context, ws_id)),
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
                        "文件 {} 读取成功（第 {} - {} 行，共 {} 行）：\n\n{}",
                        self.display_resolved_workspace_relative(&resolved, context, ws_id),
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

    pub(super) fn list_files(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let ws_id = self.extract_call_workspace_id(call, context);
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

        let dir = match self.resolve_workspace_dir(relative_dir, context, ws_id) {
            Ok(value) => value,
            Err((code, message)) => {
                return error_result(TOOL_WORKSPACE_LIST_FILES, &code, message, None)
            }
        };

        match fs::read_dir(&dir) {
            Ok(entries) => {
                let mut items = entries
                    .filter_map(Result::ok)
                    .map(|entry| {
                        let path = entry.path();
                        let label = self.display_resolved_workspace_relative(&path, context, ws_id);
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
                        self.display_resolved_workspace_relative(&dir, context, ws_id),
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

    pub(super) fn path_info(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let ws_id = self.extract_call_workspace_id(call, context);
        let relative_path = call
            .arguments
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .trim();

        let path = match self.resolve_workspace_entry(relative_path, context, ws_id) {
            Ok(value) => value,
            Err((code, message)) => {
                return error_result(TOOL_WORKSPACE_PATH_INFO, &code, message, None)
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
                        "path": self.display_resolved_workspace_relative(&path, context, ws_id),
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

    pub(super) fn glob_files(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let ws_id = self.extract_call_workspace_id(call, context);
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

        // Validate the glob up front (mirrors the engine's globset compilation) so a malformed
        // pattern fails with a structured error instead of a panic.
        if let Err(error) = compile_path_glob(pattern) {
            return error_result(
                TOOL_WORKSPACE_GLOB_FILES,
                "invalid_pattern",
                format!("无效的 glob 模式：{}。", error),
                Some("请使用合法 glob 模式，例如 `src/**/*.rs`。".to_string()),
            );
        }

        let root_entry = match self.resolve_workspace_entry(relative_dir, context, ws_id) {
            Ok(value) => value,
            Err((code, message)) => {
                return error_result(TOOL_WORKSPACE_GLOB_FILES, &code, message, None)
            }
        };

        let (paths, truncated, truncation_reason) = if root_entry.is_file() {
            // A file root matches (or not) the single workspace-relative path against the glob,
            // using the same path-or-basename semantics as the engine.
            let relative = self.display_workspace_relative(&root_entry);
            let matched = glob_matches_path_or_basename(&relative, pattern);
            let paths = if matched { vec![relative] } else { Vec::new() };
            (paths, false, None)
        } else if root_entry.is_dir() {
            let engine = SearchEngine::new();
            let result = match engine.glob_files(pattern, &root_entry, limit) {
                Ok(value) => value,
                Err(error) => {
                    return error_result(
                        TOOL_WORKSPACE_GLOB_FILES,
                        "invalid_pattern",
                        error,
                        Some("请使用合法 glob 模式，例如 `src/**/*.rs`。".to_string()),
                    )
                }
            };
            // Re-map engine-relative paths to workspace-relative paths (the legacy output
            // contract).
            let paths = result
                .paths
                .into_iter()
                .map(|path| self.display_workspace_relative(&root_entry.join(&path)))
                .collect::<Vec<_>>();
            (paths, result.truncated, result.truncation_reason)
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

        let matches = paths
            .iter()
            .map(|path| {
                json!({
                    "path": path,
                    "kind": "file"
                })
            })
            .collect::<Vec<_>>();

        ToolResult {
            tool_name: TOOL_WORKSPACE_GLOB_FILES.to_string(),
            status: "ok".to_string(),
            output: json_string(json!({
                "pattern": pattern,
                "path": self.display_workspace_relative(&root_entry),
                "matchCount": matches.len(),
                "matches": matches,
                "truncated": truncated,
                "truncationReason": truncation_reason,
            })),
            duration_ms: 0,
        }
    }

    pub(super) fn search_text(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
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
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());

        // The query is now a real regular expression when `regex` is set (design Decision 9,
        // PA-076 task 6.4). When `regex` is unset the query is treated as a literal substring and
        // escaped so it matches exactly (case folding still applies via `ignoreCase`).
        let effective_query = if regex_mode {
            query.to_string()
        } else {
            regex::escape(query)
        };

        // The user's `filePattern` is a real glob (globset semantics) under the new engine.
        // Validate it up front so a malformed pattern fails consistently for both file and
        // directory roots.
        if let Some(pattern) = &file_filter {
            if let Err(error) = compile_path_glob(pattern) {
                return error_result(
                    TOOL_WORKSPACE_SEARCH_TEXT,
                    "invalid_file_pattern",
                    format!("无效的 filePattern glob：{}。", error),
                    Some("请使用合法 glob 模式，例如 `*.rs` 或 `src/**/*.rs`。".to_string()),
                );
            }
        }

        let ws_id = self.extract_call_workspace_id(call, context);
        let root_entry = match self.resolve_workspace_entry(relative_dir, context, ws_id) {
            Ok(value) => value,
            Err((code, message)) => {
                return error_result(TOOL_WORKSPACE_SEARCH_TEXT, &code, message, None)
            }
        };

        let searched_path = self.display_resolved_workspace_relative(&root_entry, context, ws_id);
        let path_kind = if root_entry.is_file() {
            "file"
        } else if root_entry.is_dir() {
            "directory"
        } else {
            "other"
        };

        // A file root is searched by running the engine over its parent directory with the file's
        // basename as the scan-scoping glob (the same convention `workspace_gather_context`
        // uses). A user-supplied `filePattern` is then applied as a post-filter so both
        // constraints hold.
        let (search_root, forced_filter) = if root_entry.is_file() {
            let parent = root_entry
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| root_entry.clone());
            let basename = root_entry
                .file_name()
                .map(|value| value.to_string_lossy().to_string());
            (parent, basename)
        } else if root_entry.is_dir() {
            (root_entry.clone(), None)
        } else {
            return error_result(
                TOOL_WORKSPACE_SEARCH_TEXT,
                "unsupported_path_kind",
                format!("当前路径类型不支持文本搜索：{}。", searched_path),
                Some("请传入工作区内的文件或目录路径。".to_string()),
            );
        };

        let options = SearchOptions {
            max_matches: limit,
            ignore_case,
            file_pattern: forced_filter.clone().or_else(|| file_filter.clone()),
            ..SearchOptions::default()
        };

        let engine = SearchEngine::new();
        let result = match engine.search_text(&effective_query, &search_root, &options) {
            Ok(value) => value,
            Err(error) => {
                return error_result(
                    TOOL_WORKSPACE_SEARCH_TEXT,
                    "invalid_regex",
                    error,
                    Some("请检查 query 是否符合正则语法。".to_string()),
                )
            }
        };

        // Re-map engine-relative match paths to workspace-relative paths (the legacy output
        // contract and what `first_search_match_line`/consumers branch on).
        let mut matches = result
            .matches
            .into_iter()
            .map(|mut matched| {
                let absolute = search_root.join(&matched.path);
                matched.path = self.display_workspace_relative(&absolute);
                matched
            })
            .collect::<Vec<_>>();

        // Post-filter when a file root scoped the scan with the basename glob and the user also
        // supplied a filePattern: both must hold.
        if forced_filter.is_some() {
            if let Some(pattern) = &file_filter {
                matches.retain(|matched| glob_matches_path_or_basename(&matched.path, pattern));
            }
        }

        let match_count = matches.len();

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
                "scannedFiles": result.scanned_files,
                // Legacy per-category skip counters are preserved for compatibility but always 0:
                // the engine does not break skips out by category, and any scan-level budget
                // truncation is reported honestly via `truncated`/`truncationReason`.
                "skippedUnreadableFiles": 0,
                "skippedLargeFiles": 0,
                "skippedByBudget": 0,
                "scannedBytes": result.scanned_bytes,
                "truncated": result.truncated,
                "truncationReason": result.truncation_reason,
                "durationMs": result.duration_ms,
                "matchCount": match_count,
                "matches": matches.iter().map(|matched| json!({
                    "path": matched.path,
                    "line": matched.line,
                    "preview": matched.preview,
                    "captures": matched.captures,
                })).collect::<Vec<_>>(),
            })),
            duration_ms: 0,
        }
    }
}


/// `workspace_gather_context` 起始行三态可观测契约（T2-A）。
///
/// - dispatcher 硬门（`dispatcher.rs:validate_against_schema` integer 分支）只拒
///   浮点/字符串/bool（`as_i64||as_u64` 失败→`invalid_arguments`）；负整数等
///   `integer` 类型过门，进入 parser 统一处理。
/// - parser（本函数族）处理过门整数 + direct-call（`ToolRouter::execute` 不过硬门）
///   全类型：缺席/`null`→`Missing`；`as_u64` 成功→`Valid(max(1))`（`0→Valid(1)` clamp，
///   无 fallback 标记）；其余（负数/浮点/字符串/bool/数组/对象）→`Invalid{raw}`，
///   应用层回退到 `1` 并在 nested `requestedStartLine` + `meta.startLineFallback` 留痕。
/// - EOF 越界（`read_file_lines` `StartOutOfRange`→`line_out_of_range`）与 fallback
///   正交组合覆盖（T2-B F3 修订：此前“同 payload 共存”措辞过声明，实际是两个
///   payload 各断一半——Valid 大值→partial 无标；Invalid→有标无 partial；
///   multi_path 混合 payload 可同现两者条目）。顶层 `ToolResult.status` 语义不动
///   （`ok`/`partial` 裁决5）。
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum StartLineArg {
    Missing,
    Valid(usize),
    Invalid { raw: Value },
}

/// 解析 `arguments["startLine"]` 为三态（见 [`StartLineArg`] 契约注释）。
/// composite（dispatcher_composites）与 legacy（`ToolRouter::gather_context`）双实现
/// 及 `read_file_segment` 共用本函数，避免解析规则漂移。
pub(crate) fn parse_start_line_arg(arguments: &Value) -> StartLineArg {
    match arguments.get("startLine") {
        None | Some(Value::Null) => StartLineArg::Missing,
        Some(raw) => match raw.as_u64() {
            Some(value) => StartLineArg::Valid(value.max(1) as usize),
            None => StartLineArg::Invalid { raw: raw.clone() },
        },
    }
}

/// 合成 nested/plan `arguments` 回显规则：`"startLine": applied` 保持不变；
/// 仅 `Invalid` 时追加 `"requestedStartLine": raw`（`Valid`/`Missing` 不加字段）。
pub(crate) fn with_requested_start_line(mut args: Value, arg: &StartLineArg) -> Value {
    if let StartLineArg::Invalid { raw } = arg {
        if let Some(object) = args.as_object_mut() {
            object.insert("requestedStartLine".to_string(), raw.clone());
        }
    }
    args
}

/// 聚合 `meta` 回显规则：仅 `Invalid` 时追加
/// `"startLineFallback": {"requested": raw, "applied": 1, "reason": "invalid_start_line_fallback_to_1"}`。
/// `Valid`/`Missing` 不加字段；顶层 `status` 不动。
pub(crate) fn with_start_line_fallback_meta(mut meta: Value, arg: &StartLineArg) -> Value {
    if let StartLineArg::Invalid { raw } = arg {
        if let Some(object) = meta.as_object_mut() {
            object.insert(
                "startLineFallback".to_string(),
                json!({
                    "requested": raw.clone(),
                    "applied": 1,
                    "reason": "invalid_start_line_fallback_to_1",
                }),
            );
        }
    }
    meta
}

/// 解析模型显式提供的分页起始行（`workspace_gather_context` 语境，PA-100）。
/// 本函数为三态契约的兼容薄层：`Valid(v)`→`Some(v)`，`Missing`/`Invalid`→`None`
/// （调用方 `unwrap_or(1)` 即静默回退到默认，而非报错；返回值保证 ≥ 1）。
/// composite（dispatcher_composites）与 legacy（ToolRouter::gather_context）双实现
/// 共用本函数，避免解析规则漂移。
pub(crate) fn explicit_gather_start_line(arguments: &Value) -> Option<usize> {
    match parse_start_line_arg(arguments) {
        StartLineArg::Valid(value) => Some(value),
        StartLineArg::Missing | StartLineArg::Invalid { .. } => None,
    }
}

// PA-100（code-review A 建议3）：本函数与 dispatcher_composites.rs 中的同名拷贝
// 为双实现镜像——修改签名/回显字段必须两处同步（task 8.1 移除 legacy 路径前有效）。
// T2-A：新增 `start_line_arg` 参数，仅 Invalid 时在 plan step arguments 追加
// `requestedStartLine`（`with_requested_start_line`），`startLine: applied` 不变。

pub(super) enum FileSegment {
    Empty,
    Range {
        start_line: usize,
        end_line: usize,
        lines: Vec<(usize, String)>,
        total_lines: usize,
    },
}

pub(super) enum ReadSegmentError {
    StartOutOfRange { total_lines: usize },
    Io(std::io::Error),
}

pub(super) fn read_file_lines(
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
