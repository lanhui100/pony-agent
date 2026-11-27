use super::*;
use super::exec::SpillReadArgs;
use super::exec::error_result;
use super::exec::fill_session_from_context;
use super::exec::job_tool_error;
use super::exec::json_string;

impl ToolRouter {
    pub(super) fn execute_todo_write(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let mut args: Result<TodoWriteArgs, _> = serde_json::from_value(call.arguments.clone());
        if let Ok(parsed) = args.as_mut() {
            fill_session_from_context(&mut parsed.session_id, context);
        }
        match args {
            Ok(args) => match todo_write(args) {
                Ok(res) => ToolResult {
                    tool_name: TOOL_TODO_WRITE.to_string(),
                    status: "ok".to_string(),
                    output: json_string(json!({
                        "ok": true,
                        "tool": TOOL_TODO_WRITE,
                        "result": res,
                    })),
                    duration_ms: 0,
                },
                Err(err) => error_result(TOOL_TODO_WRITE, "todo_write_failed", err, None),
            },
            Err(err) => error_result(
                TOOL_TODO_WRITE,
                "invalid_arguments",
                format!("Failed to parse todo_write arguments: {}", err),
                None,
            ),
        }
    }

    pub(super) fn execute_todo_list(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let mut args: Result<TodoListArgs, _> = serde_json::from_value(call.arguments.clone());
        if let Ok(parsed) = args.as_mut() {
            fill_session_from_context(&mut parsed.session_id, context);
        }
        match args {
            Ok(args) => match todo_list(args) {
                Ok(res) => ToolResult {
                    tool_name: TOOL_TODO_LIST.to_string(),
                    status: "ok".to_string(),
                    output: json_string(json!({
                        "ok": true,
                        "tool": TOOL_TODO_LIST,
                        "result": res,
                    })),
                    duration_ms: 0,
                },
                Err(err) => error_result(TOOL_TODO_LIST, "todo_list_failed", err, None),
            },
            Err(err) => error_result(
                TOOL_TODO_LIST,
                "invalid_arguments",
                format!("Failed to parse todo_list arguments: {}", err),
                None,
            ),
        }
    }

    pub(super) fn execute_spill_read(&self, call: &ToolCall) -> ToolResult {
        let args: Result<SpillReadArgs, _> = serde_json::from_value(call.arguments.clone());
        match args {
            Ok(args) => {
                let offset = args.offset.unwrap_or(0);
                let limit = args
                    .limit
                    .unwrap_or(pony_protocol::spill::DEFAULT_READ_LIMIT);
                match pony_protocol::spill::read_spill(&args.ref_id, offset, limit) {
                    Ok(slice) => ToolResult {
                        tool_name: TOOL_SPILL_READ.to_string(),
                        status: "ok".to_string(),
                        output: json_string(json!({
                            "ok": true,
                            "tool": TOOL_SPILL_READ,
                            "result": {
                                "ref_id": args.ref_id,
                                "offset": offset,
                                // 回显实际生效的上限（spill 模块已 clamp 到 MAX_READ_LIMIT），
                                // 避免模型按未 clamp 的入参翻页而静默跳字节。
                                "limit": slice.limit,
                                "start_offset": slice.start,
                                "end_offset": slice.end,
                                "total_bytes": slice.total_bytes,
                                "content": slice.content,
                                // 真实尾部判定（由 spill 模块给出，避免调用方重算偏移高估）
                                "truncated": slice.truncated(),
                            }
                        })),
                        duration_ms: 0,
                    },
                    Err(err) => error_result(
                        TOOL_SPILL_READ,
                        err.code(),
                        err.message().to_string(),
                        Some(
                            "content_ref 来自大输出工具的 spilled 结果；若已被清理请重新执行产生该输出的工具。"
                                .to_string(),
                        ),
                    ),
                }
            }
            Err(err) => error_result(
                TOOL_SPILL_READ,
                "invalid_arguments",
                format!("Failed to parse spill_read arguments: {}", err),
                None,
            ),
        }
    }

    pub(super) fn execute_create_goal(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        if let Some(dispatch_ctx) = context.dispatch_context.as_ref() {
            if !dispatch_ctx.is_direct_human {
                return error_result(
                    TOOL_CREATE_GOAL,
                    "goal_action_unauthorized",
                    "create_goal requires direct human authority".to_string(),
                    None,
                );
            }
        }
        let mut args: Result<CreateGoalArgs, _> = serde_json::from_value(call.arguments.clone());
        if let Ok(parsed) = args.as_mut() {
            fill_session_from_context(&mut parsed.session_id, context);
        }
        match args {
            Ok(args) => match create_goal(args) {
                Ok(res) => ToolResult {
                    tool_name: TOOL_CREATE_GOAL.to_string(),
                    status: "ok".to_string(),
                    output: json_string(json!({
                        "ok": true,
                        "tool": TOOL_CREATE_GOAL,
                        "result": res,
                    })),
                    duration_ms: 0,
                },
                Err(err) => error_result(TOOL_CREATE_GOAL, "create_goal_failed", err, None),
            },
            Err(err) => error_result(
                TOOL_CREATE_GOAL,
                "invalid_arguments",
                format!("Failed to parse create_goal arguments: {}", err),
                None,
            ),
        }
    }

    pub(super) fn execute_get_goal(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let mut args: Result<GetGoalArgs, _> = serde_json::from_value(call.arguments.clone());
        if let Ok(parsed) = args.as_mut() {
            fill_session_from_context(&mut parsed.session_id, context);
        }
        match args {
            Ok(args) => match get_goal(args) {
                Ok(res) => ToolResult {
                    tool_name: TOOL_GET_GOAL.to_string(),
                    status: "ok".to_string(),
                    output: json_string(json!({
                        "ok": true,
                        "tool": TOOL_GET_GOAL,
                        "result": res,
                    })),
                    duration_ms: 0,
                },
                Err(err) => error_result(TOOL_GET_GOAL, "get_goal_failed", err, None),
            },
            Err(err) => error_result(
                TOOL_GET_GOAL,
                "invalid_arguments",
                format!("Failed to parse get_goal arguments: {}", err),
                None,
            ),
        }
    }

    pub(super) fn execute_update_goal(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let mut args: Result<UpdateGoalArgs, _> = serde_json::from_value(call.arguments.clone());
        if let Ok(parsed) = args.as_mut() {
            fill_session_from_context(&mut parsed.session_id, context);
        }
        let args = match args {
            Ok(args) => args,
            Err(err) => {
                return error_result(
                    TOOL_UPDATE_GOAL,
                    "invalid_arguments",
                    format!("Failed to parse update_goal arguments: {}", err),
                    None,
                );
            }
        };

        if let Some(dispatch_ctx) = context.dispatch_context.as_ref() {
            match args.action {
                UpdateGoalAction::Pause | UpdateGoalAction::Edit => {
                    if !dispatch_ctx.is_direct_human {
                        return error_result(
                            TOOL_UPDATE_GOAL,
                            "goal_action_unauthorized",
                            format!("Action '{:?}' requires direct human authority", args.action),
                            None,
                        );
                    }
                }
                UpdateGoalAction::Resume => {
                    if !dispatch_ctx.is_direct_human {
                        return error_result(
                            TOOL_UPDATE_GOAL,
                            "goal_action_unauthorized",
                            "resume action requires direct human authority (cannot resume without human initiation)".to_string(),
                            None,
                        );
                    }
                }
                UpdateGoalAction::Complete | UpdateGoalAction::Blocked => {
                    if !dispatch_ctx.is_direct_human && !dispatch_ctx.is_goal_round {
                        return error_result(
                            TOOL_UPDATE_GOAL,
                            "goal_action_unauthorized",
                            format!(
                                "Action '{:?}' requires direct human authority or an active goal round",
                                args.action
                            ),
                            None,
                        );
                    }
                }
            }
        }

        match update_goal(args) {
            Ok(res) => {
                // B129：终态（completed / blocked）后，宿主循环下一条必定是「本轮最后一次
                // 模型调用」——把收尾指令随 tool result 一起回灌，模型才能把结论写给用户。
                // 非终态相位不注入（避免每轮 update（edit/pause/resume）都刷一段无关文案）。
                let wrapup = pony_storage::goal_wrapup::wrapup_for_goal(&res.goal);
                let mut payload = json!({
                    "ok": true,
                    "tool": TOOL_UPDATE_GOAL,
                    "result": res,
                });
                if let (Some(wrapup), Some(object)) = (wrapup, payload.as_object_mut()) {
                    object.insert("wrapup".to_string(), json!(wrapup));
                }
                ToolResult {
                    tool_name: TOOL_UPDATE_GOAL.to_string(),
                    status: "ok".to_string(),
                    output: json_string(payload),
                    duration_ms: 0,
                }
            }
            Err(err) => {
                // O5（2026-11-27）：`blocked` 的最小轮次门槛是**结构性拒绝**（目标还没跑够轮），
                // 不是工具执行失败——必须给稳定错误码 `goal_block_threshold`，模型/UI 才能据此
                // 给出「再推进几轮」的指引，而不是当成普通失败盲目重试同一条 blocked 调用。
                // 前缀契约真源：`pony_storage::goal::update_goal` 的 Blocked 分支。
                if err.starts_with("goal_block_threshold") {
                    error_result(TOOL_UPDATE_GOAL, "goal_block_threshold", err, None)
                } else {
                    error_result(TOOL_UPDATE_GOAL, "update_goal_failed", err, None)
                }
            }
        }
    }

    // ===================== W37-C1：七项新工具执行方法 =====================

    pub(super) fn execute_present(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let ws_id = self.extract_call_workspace_id(call, context);
        let mut args: PresentArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => {
                return error_result(
                    TOOL_PRESENT,
                    "invalid_arguments",
                    format!("Failed to parse present arguments: {}", err),
                    None,
                )
            }
        };
        // 未显式提供 cwd 时，用会话 workspace root，避免回退到进程 cwd。
        if args.cwd.is_none() {
            if let Ok(root) = self.resolved_workspace_root(context, ws_id) {
                args.cwd = Some(root.display().to_string());
            }
        }
        match present(args) {
            Ok(res) => ToolResult {
                tool_name: TOOL_PRESENT.to_string(),
                status: "ok".to_string(),
                output: json_string(json!({
                    "ok": true,
                    "tool": TOOL_PRESENT,
                    "result": res,
                })),
                duration_ms: 0,
            },
            Err(err) => error_result(TOOL_PRESENT, "present_failed", err, None),
        }
    }

    pub(super) fn execute_find(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let ws_id = self.extract_call_workspace_id(call, context);
        let mut args: FuzzyFileSearchArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => {
                return error_result(
                    TOOL_FUZZY_FILE_SEARCH,
                    "invalid_arguments",
                    format!("Failed to parse find arguments: {}", err),
                    None,
                )
            }
        };
        if args.cwd.is_none() {
            if let Ok(root) = self.resolved_workspace_root(context, ws_id) {
                args.cwd = Some(root.display().to_string());
            }
        }
        match fuzzy_file_search(args) {
            Ok(res) => ToolResult {
                tool_name: TOOL_FUZZY_FILE_SEARCH.to_string(),
                status: "ok".to_string(),
                output: json_string(json!({
                    "ok": true,
                    "tool": TOOL_FUZZY_FILE_SEARCH,
                    "result": res,
                })),
                duration_ms: 0,
            },
            Err(err) => error_result(TOOL_FUZZY_FILE_SEARCH, "find_failed", err, None),
        }
    }

    pub(super) fn execute_git_diff(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let ws_id = self.extract_call_workspace_id(call, context);
        let mut args: GitDiffRemoteArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => {
                return error_result(
                    TOOL_GIT_DIFF_REMOTE,
                    "invalid_arguments",
                    format!("Failed to parse git_diff arguments: {}", err),
                    None,
                )
            }
        };
        if args.cwd.is_none() {
            if let Ok(root) = self.resolved_workspace_root(context, ws_id) {
                args.cwd = Some(root.display().to_string());
            }
        }
        match git_diff_remote(args) {
            Ok(res) => ToolResult {
                tool_name: TOOL_GIT_DIFF_REMOTE.to_string(),
                status: "ok".to_string(),
                output: json_string(json!({
                    "ok": true,
                    "tool": TOOL_GIT_DIFF_REMOTE,
                    "result": res,
                })),
                duration_ms: 0,
            },
            Err(err) => error_result(TOOL_GIT_DIFF_REMOTE, "git_diff_failed", err, None),
        }
    }

    pub(super) fn execute_job_start(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let args: JobStartArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => {
                return error_result(
                    TOOL_JOB_START,
                    "invalid_arguments",
                    format!("Failed to parse job_start arguments: {}", err),
                    None,
                )
            }
        };
        // 必须把会话 workspace root 传给 lib 层做 cwd 解析与 fail-closed 校验。
        let root = self
            .resolved_workspace_root(context, self.extract_call_workspace_id(call, context))
            .ok();
        // 契约 B012：作业归属取会话上下文（`None` = 无上下文，对任意 caller 放行）。
        let owner = context.session_id.as_deref();
        match job_start_scoped(args, root.as_deref(), owner) {
            Ok(res) => ToolResult {
                tool_name: TOOL_JOB_START.to_string(),
                status: "ok".to_string(),
                output: json_string(json!({
                    "ok": true,
                    "tool": TOOL_JOB_START,
                    "result": res,
                })),
                duration_ms: 0,
            },
            Err(err) => error_result(TOOL_JOB_START, "job_start_failed", err, None),
        }
    }

    pub(super) fn execute_job_output(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let args: JobOutputArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => {
                return error_result(
                    TOOL_JOB_OUTPUT,
                    "invalid_arguments",
                    format!("Failed to parse job_output arguments: {}", err),
                    None,
                )
            }
        };
        match job_output_scoped(args, context.session_id.as_deref()) {
            Ok(res) => ToolResult {
                tool_name: TOOL_JOB_OUTPUT.to_string(),
                status: "ok".to_string(),
                output: json_string(json!({
                    "ok": true,
                    "tool": TOOL_JOB_OUTPUT,
                    "result": res,
                })),
                duration_ms: 0,
            },
            Err(err) => job_tool_error(TOOL_JOB_OUTPUT, "job_output_failed", err),
        }
    }

    pub(super) fn execute_job_kill(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let args: JobKillArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(err) => {
                return error_result(
                    TOOL_JOB_KILL,
                    "invalid_arguments",
                    format!("Failed to parse job_kill arguments: {}", err),
                    None,
                )
            }
        };
        match job_kill_scoped(args, context.session_id.as_deref()) {
            Ok(res) => ToolResult {
                tool_name: TOOL_JOB_KILL.to_string(),
                status: "ok".to_string(),
                output: json_string(json!({
                    "ok": true,
                    "tool": TOOL_JOB_KILL,
                    "result": res,
                })),
                duration_ms: 0,
            },
            Err(err) => job_tool_error(TOOL_JOB_KILL, "job_kill_failed", err),
        }
    }

    pub(super) fn execute_job_list(&self, context: &ToolExecutionContext) -> ToolResult {
        // 契约 B012：非 owner 的作业在 lib 层被过滤掉（列表接口不泄露存在性）。
        match job_list_scoped(JobListArgs {}, context.session_id.as_deref()) {
            Ok(res) => ToolResult {
                tool_name: TOOL_JOB_LIST.to_string(),
                status: "ok".to_string(),
                output: json_string(json!({
                    "ok": true,
                    "tool": TOOL_JOB_LIST,
                    "result": res,
                })),
                duration_ms: 0,
            },
            Err(err) => job_tool_error(TOOL_JOB_LIST, "job_list_failed", err),
        }
    }

}


impl ToolRouter {
    pub(super) fn execute_skill_load(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        let Some(name) = call
            .arguments
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return error_result(
                TOOL_SKILL_LOAD,
                "missing_argument",
                "缺少必填参数 `name`（技能名）。".to_string(),
                Some("参数示例：{\"name\":\"review\"}".to_string()),
            );
        };

        let ws_id = self.extract_call_workspace_id(call, context);
        let workspace_root = match self.resolved_workspace_root(context, ws_id) {
            Ok(root) => root,
            Err((code, message)) => return error_result(TOOL_SKILL_LOAD, &code, message, None),
        };

        let Some(skill) = crate::skill_body::resolve_skill_by_name(&workspace_root, name)
        else {
            return error_result(
                TOOL_SKILL_LOAD,
                "unknown_skill",
                format!("未找到技能 `{name}`。"),
                Some("请先查看可用技能列表中的名字，或确认该技能目录下存在 SKILL.md。".to_string()),
            );
        };

        // 四象限策略拦截（ISSUE-20261111-008 / PLAN-20261111-02 Step 3）：
        // summary 检查：技能声明 disable-model-invocation: true 时禁止模型端工具加载
        if !skill.model_invocable || skill.disable_model_invocation {
            return error_result(
                TOOL_SKILL_LOAD,
                "skill_not_available_for_model_invocation",
                format!("技能 `{name}` 已配置为禁止模型直接调用（disable-model-invocation: true）。"),
                Some("该技能仅供人类用户通过命令显式调用，无法通过 skill_load 工具加载。".to_string()),
            );
        }

        // L3-B F4：项目层技能（workspace/.agents|.claude|.dsh/skills）同样可读，
        // 因此把会话 workspace root 传给白名单判定（全局根仍然受信）。
        match crate::skill_body::load_skill_body_for_workspace(
            &skill,
            crate::skill_body::MAX_SKILL_BODY_BYTES,
            Some(workspace_root.as_path()),
        ) {
            Ok(body) => {
                // definition 检查：从读取的正文 frontmatter 二次确认是否禁用模型调用（双重检查）
                let (_, _, _, _, disable_model, model_inv) =
                    crate::workspace_hotloader::parse_skill_metadata(&body.content, name);
                if disable_model || !model_inv {
                    return error_result(
                        TOOL_SKILL_LOAD,
                        "skill_not_available_for_model_invocation",
                        format!("技能 `{name}` 正文配置为禁止模型直接调用（disable-model-invocation: true）。"),
                        Some("该技能仅供人类用户通过命令显式调用，无法通过 skill_load 工具加载。".to_string()),
                    );
                }
                ToolResult {
                    tool_name: TOOL_SKILL_LOAD.to_string(),
                    status: "ok".to_string(),
                    output: format!("=== SKILL {} ===\n{}", body.name, body.content),
                    duration_ms: 0,
                }
            }
            Err(error) => error_result(TOOL_SKILL_LOAD, error.code(), error.message(), None),
        }
    }

    pub(super) fn time_now(&self) -> ToolResult {
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

    pub(super) fn echo_input(&self, call: &ToolCall) -> ToolResult {
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
}
