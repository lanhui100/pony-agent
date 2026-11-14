//! 编排桥接面：subagent/team/goal/todo/present/find/git_diff/job_*/skill_load
//! 等处理器，把调用转发给 `crate::agent::orchestration` 等模块。
use super::*;
use super::exec::SpillReadArgs;
use super::exec::capability_unavailable_result;
use super::exec::error_result;
use super::exec::fill_session_from_context;
use super::exec::job_tool_error;
use super::exec::json_string;

impl ToolRouter {
    /// W42/PA-149：按名加载已安装技能正文。
    ///
    /// fail-closed：未知名返回结构化失败（禁止空正文冒充成功）；正文读取经
    /// [`crate::agent::skill_body::load_skill_body`] 的受信只读根白名单与字节上限双重门禁。
    pub(super) fn execute_subagent(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_SUBAGENT);
        }

        // Wave-61 C8.4：与文件工具**同源**解析工作区根（会话上下文 → workspaceId resolver →
        // 构造默认 root），保证子代理与文件工具落在同一根；解析失败按文件工具口径 fail-closed。
        let workspace_root = match self
            .resolved_workspace_root(context, self.extract_call_workspace_id(call, context))
        {
            Ok(root) => root,
            Err((code, message)) => return error_result(TOOL_SUBAGENT, &code, message, None),
        };

        // P3 trace：工具执行前先建 `tool.call` span，子代理 turn 于是成为它的真子节点
        // （旧实现要等工具结束才 backdate 创建 span，子 turn 只能挂到 turn 下）。
        let args_preview = serde_json::to_string(&call.arguments).ok();
        let _tool_span = crate::agent::otel::begin_tool_call_span(
            call.call_id.as_deref(),
            TOOL_SUBAGENT,
            args_preview.as_deref(),
        );

        let args: Result<SubagentArgs, _> = serde_json::from_value(call.arguments.clone());
        match args {
            // Wave-61 C8.4 + W2 + P1/P2：把宿主真实工作区根、**宿主当前 provider 选择**、
            // 父会话 id 与 workspace 归属一并传给子代理（项目指令/技能、workspace 绑定、
            // 会话 id 对齐随之生效；`args` 显式覆盖优先）。
            Ok(args) => match subagent_in_workspace_with_context(
                args,
                Some(workspace_root.as_path()),
                context.provider_selection.as_ref(),
                context.session_id.as_deref(),
                context.workspace_id.as_deref(),
            ) {
                Ok(res) => ToolResult {
                    tool_name: TOOL_SUBAGENT.to_string(),
                    status: "ok".to_string(),
                    output: json_string(json!({
                        "ok": true,
                        "tool": TOOL_SUBAGENT,
                        "result": res,
                    })),
                    duration_ms: 0,
                },
                Err(err) => error_result(TOOL_SUBAGENT, "subagent_failed", err, None),
            },
            Err(err) => error_result(
                TOOL_SUBAGENT,
                "invalid_arguments",
                format!("Failed to parse subagent arguments: {}", err),
                None,
            ),
        }
    }

    /// B150/P1：`acp_delegate` —— 把一段任务委派给外部 ACP agent（空桩）。
    ///
    /// 结构照抄 [`Self::execute_subagent`]（`orchestration.rs:19-72`）：
    /// 解析 `AcpAgentSpec`（`acp::config::resolve_agent`）→ 解析 workspace_root（与 subagent
    /// 同源 `resolved_workspace_root`）→ 建 `tool.call` span → 建 [`AcpAgentSession`] → 跑
    /// prompt → 归约 `ToolResult`。`args: { agent(必填), prompt(必填), cwd?, timeoutMs? }`。
    ///
    /// 错误码沿用 §3.2.9 的 `acp_*` 分源（`acp_spawn_failed` / `acp_initialize_failed` /
    /// `acp_protocol_error` / `acp_agent_error` / `acp_agent_refused` / `acp_auth_required` /
    /// `acp_timeout` / `acp_unavailable`）。
    ///
    /// [`AcpAgentSession`]: crate::acp::session::AcpAgentSession
    #[allow(unused_variables)]
    pub(super) fn execute_acp_delegate(
        &self,
        call: &ToolCall,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        unimplemented!("P1：acp_delegate 工具实现（spec 解析 + workspace root + session.prompt + 归约）")
    }

    pub(super) fn execute_workflow(&self, call: &ToolCall) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_WORKFLOW);
        }

        let args: Result<WorkflowArgs, _> = serde_json::from_value(call.arguments.clone());
        match args {
            Ok(args) => match workflow(args) {
                Ok(res) => ToolResult {
                    tool_name: TOOL_WORKFLOW.to_string(),
                    status: "ok".to_string(),
                    output: json_string(json!({
                        "ok": true,
                        "tool": TOOL_WORKFLOW,
                        "result": res,
                    })),
                    duration_ms: 0,
                },
                Err(err) => error_result(TOOL_WORKFLOW, "workflow_failed", err, None),
            },
            Err(err) => error_result(
                TOOL_WORKFLOW,
                "invalid_arguments",
                format!("Failed to parse workflow arguments: {}", err),
                None,
            ),
        }
    }

    pub(super) fn execute_spawn_teammate(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_SPAWN_TEAMMATE);
        }

        let workspace_root = match self
            .resolved_workspace_root(context, self.extract_call_workspace_id(call, context))
        {
            Ok(root) => root,
            Err((code, message)) => {
                return error_result(TOOL_SPAWN_TEAMMATE, &code, message, None)
            }
        };

        let args: Result<SpawnTeammateArgs, _> = serde_json::from_value(call.arguments.clone());
        // P3 trace：与 subagent 同构——工具执行前先建 `tool.call` span，队友 turn（含
        // 同步驱动的首轮）因此挂在本次派生之下；记账入口按 call_id 复用同一 span。
        let args_preview = serde_json::to_string(&call.arguments).ok();
        let _tool_span = crate::agent::otel::begin_tool_call_span(
            call.call_id.as_deref(),
            TOOL_SPAWN_TEAMMATE,
            args_preview.as_deref(),
        );
        match args {
            // Wave-61 C7 + C8.4 + W2：派生队友后把它的初始 prompt 真正投递并驱动（引擎在位时），
            // 并把**宿主当前 provider 选择**作为继承源（`args` 的 provider/model 覆盖优先；
            // 解析失败在此前置返回 `spawn_failed`，绝不先返回 ok 再异步失败）。
            Ok(args) => match spawn_teammate_in_workspace_with_provider(
                args,
                Some(workspace_root.as_path()),
                context.provider_selection.as_ref(),
            ) {
                Ok(res) => ToolResult {
                    tool_name: TOOL_SPAWN_TEAMMATE.to_string(),
                    status: "ok".to_string(),
                    output: json_string(json!({
                        "ok": true,
                        "tool": TOOL_SPAWN_TEAMMATE,
                        "result": res,
                    })),
                    duration_ms: 0,
                },
                Err(err) => error_result(TOOL_SPAWN_TEAMMATE, "spawn_failed", err, None),
            },
            Err(err) => error_result(
                TOOL_SPAWN_TEAMMATE,
                "invalid_arguments",
                format!("Failed to parse spawn_teammate arguments: {}", err),
                None,
            ),
        }
    }

    pub(super) fn execute_list_agents(&self, _call: &ToolCall) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_LIST_AGENTS);
        }

        match list_agents() {
            Ok(res) => ToolResult {
                tool_name: TOOL_LIST_AGENTS.to_string(),
                status: "ok".to_string(),
                output: json_string(json!({
                    "ok": true,
                    "tool": TOOL_LIST_AGENTS,
                    "result": res,
                })),
                duration_ms: 0,
            },
            Err(err) => error_result(TOOL_LIST_AGENTS, "list_agents_failed", err, None),
        }
    }

    pub(super) fn execute_send_message(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_SEND_MESSAGE);
        }

        let workspace_root = match self
            .resolved_workspace_root(context, self.extract_call_workspace_id(call, context))
        {
            Ok(root) => root,
            Err((code, message)) => {
                return error_result(TOOL_SEND_MESSAGE, &code, message, None)
            }
        };

        let args: Result<SendMessageArgs, _> = serde_json::from_value(call.arguments.clone());
        match args {
            // Wave-61 C7/C8.4：入队成功后由后台线程真实驱动队友 turn，并以同源解析的工作区根建栈。
            Ok(args) => match send_message_in_workspace(args, Some(workspace_root.as_path())) {
                Ok(res) => ToolResult {
                    tool_name: TOOL_SEND_MESSAGE.to_string(),
                    status: "ok".to_string(),
                    output: json_string(json!({
                        "ok": true,
                        "tool": TOOL_SEND_MESSAGE,
                        "result": res,
                    })),
                    duration_ms: 0,
                },
                Err(err) => error_result(TOOL_SEND_MESSAGE, "send_failed", err, None),
            },
            Err(err) => error_result(
                TOOL_SEND_MESSAGE,
                "invalid_arguments",
                format!("Failed to parse send_message arguments: {}", err),
                None,
            ),
        }
    }

    pub(super) fn execute_interrupt_agent(&self, call: &ToolCall) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_INTERRUPT_AGENT);
        }

        let args: Result<InterruptAgentArgs, _> = serde_json::from_value(call.arguments.clone());
        match args {
            Ok(args) => match interrupt_agent(args) {
                Ok(res) => ToolResult {
                    tool_name: TOOL_INTERRUPT_AGENT.to_string(),
                    status: "ok".to_string(),
                    output: json_string(json!({
                        "ok": true,
                        "tool": TOOL_INTERRUPT_AGENT,
                        "result": res,
                    })),
                    duration_ms: 0,
                },
                Err(err) => error_result(TOOL_INTERRUPT_AGENT, "interrupt_failed", err, None),
            },
            Err(err) => error_result(
                TOOL_INTERRUPT_AGENT,
                "invalid_arguments",
                format!("Failed to parse interrupt_agent arguments: {}", err),
                None,
            ),
        }
    }

    pub(super) fn execute_drain_inbox(&self, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_DRAIN_INBOX);
        }

        let args: Result<ReadInboxArgs, _> = serde_json::from_value(call.arguments.clone());
        match args {
            Ok(args) => {
                // D2 修复：模型侧只允许消费**调用者自己**的信箱。队友信箱由驱动循环
                // （drive_teammate_turn_in）消费；Lead 跨成员 drain 会把派发消息抢走
                //（2026-11-07 trace 实证：Lead drain dev-tui 后被迫重发）。
                // 调用者判定：context.session_id 反查 TEAMMATE_SESSIONS；宿主会话未登记 ⇒ lead。
                let target = crate::agent::orchestration::normalize_target(&args.target);
                let caller = context
                    .session_id
                    .as_deref()
                    .and_then(crate::agent::orchestration::teammate_target_for_session)
                    .unwrap_or_else(|| "lead".to_string());
                if target != caller {
                    return error_result(
                        TOOL_DRAIN_INBOX,
                        "inbox_scope_violation",
                        format!(
                            "drain_inbox 只能消费调用者自己的信箱（caller={caller}，请求 target={target}）；队友信箱由驱动循环消费"
                        ),
                        Some(
                            "给该成员派发消息用 send_message；读取其回执请 drain 自己的信箱（target=lead）；观察待处理条数用 /agents 面板"
                                .to_string(),
                        ),
                    );
                }
                match drain_inbox(args) {
                    Ok(res) => ToolResult {
                        tool_name: TOOL_DRAIN_INBOX.to_string(),
                        status: "ok".to_string(),
                        output: json_string(json!({
                            "ok": true,
                            "tool": TOOL_DRAIN_INBOX,
                            "result": res,
                        })),
                        duration_ms: 0,
                    },
                    Err(err) => error_result(TOOL_DRAIN_INBOX, "drain_failed", err, None),
                }
            }
            Err(err) => error_result(
                TOOL_DRAIN_INBOX,
                "invalid_arguments",
                format!("Failed to parse drain_inbox arguments: {}", err),
                None,
            ),
        }
    }

    pub(super) fn execute_team_task_create(&self, call: &ToolCall) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_TEAM_TASK_CREATE);
        }

        let args: Result<TeamTaskCreateArgs, _> = serde_json::from_value(call.arguments.clone());
        match args {
            Ok(args) => match team_task_create(args) {
                Ok(res) => ToolResult {
                    tool_name: TOOL_TEAM_TASK_CREATE.to_string(),
                    status: "ok".to_string(),
                    output: json_string(json!({
                        "ok": true,
                        "tool": TOOL_TEAM_TASK_CREATE,
                        "result": res,
                    })),
                    duration_ms: 0,
                },
                Err(err) => error_result(TOOL_TEAM_TASK_CREATE, "task_create_failed", err, None),
            },
            Err(err) => error_result(
                TOOL_TEAM_TASK_CREATE,
                "invalid_arguments",
                format!("Failed to parse team_task_create arguments: {}", err),
                None,
            ),
        }
    }

    pub(super) fn execute_team_task_update(&self, call: &ToolCall) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_TEAM_TASK_UPDATE);
        }

        let args: Result<TeamTaskUpdateArgs, _> = serde_json::from_value(call.arguments.clone());
        match args {
            Ok(args) => match team_task_update(args) {
                Ok(res) => ToolResult {
                    tool_name: TOOL_TEAM_TASK_UPDATE.to_string(),
                    status: "ok".to_string(),
                    output: json_string(json!({
                        "ok": true,
                        "tool": TOOL_TEAM_TASK_UPDATE,
                        "result": res,
                    })),
                    duration_ms: 0,
                },
                Err(err) => error_result(TOOL_TEAM_TASK_UPDATE, "task_update_failed", err, None),
            },
            Err(err) => error_result(
                TOOL_TEAM_TASK_UPDATE,
                "invalid_arguments",
                format!("Failed to parse team_task_update arguments: {}", err),
                None,
            ),
        }
    }

    pub(super) fn execute_team_task_list(&self, _call: &ToolCall) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_TEAM_TASK_LIST);
        }

        match team_task_list() {
            Ok(res) => ToolResult {
                tool_name: TOOL_TEAM_TASK_LIST.to_string(),
                status: "ok".to_string(),
                output: json_string(json!({
                    "ok": true,
                    "tool": TOOL_TEAM_TASK_LIST,
                    "result": res,
                })),
                duration_ms: 0,
            },
            Err(err) => error_result(TOOL_TEAM_TASK_LIST, "task_list_failed", err, None),
        }
    }

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
                    .unwrap_or(crate::agent::spill::DEFAULT_READ_LIMIT);
                match crate::agent::spill::read_spill(&args.ref_id, offset, limit) {
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
            Ok(res) => ToolResult {
                tool_name: TOOL_UPDATE_GOAL.to_string(),
                status: "ok".to_string(),
                output: json_string(json!({
                    "ok": true,
                    "tool": TOOL_UPDATE_GOAL,
                    "result": res,
                })),
                duration_ms: 0,
            },
            Err(err) => error_result(TOOL_UPDATE_GOAL, "update_goal_failed", err, None),
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

        let Some(skill) = crate::agent::skill_body::resolve_skill_by_name(&workspace_root, name)
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
        match crate::agent::skill_body::load_skill_body_for_workspace(
            &skill,
            crate::agent::skill_body::MAX_SKILL_BODY_BYTES,
            Some(workspace_root.as_path()),
        ) {
            Ok(body) => {
                // definition 检查：从读取的正文 frontmatter 二次确认是否禁用模型调用（双重检查）
                let (_, _, _, _, disable_model, model_inv) =
                    crate::agent::workspace_hotloader::parse_skill_metadata(&body.content, name);
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

