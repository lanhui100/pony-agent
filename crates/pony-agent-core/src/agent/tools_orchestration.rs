use crate::agent::orchestration::*;
use crate::agent::tools::exec::{capability_unavailable_result, error_result, json_string};
use crate::agent::tools::*;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

pub(crate) struct CoreOrchestrationBridge;

impl pony_tools::tools::OrchestrationToolBridge for CoreOrchestrationBridge {
    fn execute_primitive(
        &self,
        primitive: &str,
        router: &pony_tools::tools::ToolRouter,
        call: &ToolCall,
        context: &ToolExecutionContext,
    ) -> Option<ToolResult> {
        let res = match primitive {
            TOOL_SUBAGENT => execute_subagent(router, call, context),
            TOOL_ACP_DELEGATE => execute_acp_delegate(router, call, context),
            TOOL_WORKFLOW => execute_workflow(router, call),
            TOOL_SPAWN_TEAMMATE => execute_spawn_teammate(router, call, context),
            TOOL_LIST_AGENTS => execute_list_agents(router, call, context),
            TOOL_SEND_MESSAGE => execute_send_message(router, call, context),
            TOOL_INTERRUPT_AGENT => execute_interrupt_agent(router, call, context),
            TOOL_DRAIN_INBOX => execute_drain_inbox(router, call, context),
            TOOL_TEAM_TASK_CREATE => execute_team_task_create(router, call, context),
            TOOL_TEAM_TASK_UPDATE => execute_team_task_update(router, call, context),
            TOOL_TEAM_TASK_LIST => execute_team_task_list(router, call),
            _ => return None,
        };
        Some(res)
    }
}

    /// W42/PA-149：按名加载已安装技能正文。
    ///
    /// fail-closed：未知名返回结构化失败（禁止空正文冒充成功）；正文读取经
    /// [`crate::agent::skill_body::load_skill_body`] 的受信只读根白名单与字节上限双重门禁。
    fn execute_subagent(router: &pony_tools::tools::ToolRouter, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_SUBAGENT);
        }

        // Wave-61 C8.4：与文件工具**同源**解析工作区根（会话上下文 → workspaceId resolver →
        // 构造默认 root），保证子代理与文件工具落在同一根；解析失败按文件工具口径 fail-closed。
        let workspace_root = match router
            .resolved_workspace_root(context, router.extract_call_workspace_id(call, context))
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
    pub(super) fn execute_acp_delegate(
        router: &pony_tools::tools::ToolRouter,
        call: &ToolCall,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        // Wave-61 C8.4 同源：cwd 缺省时用与文件工具同一工作区根（解析失败按文件工具口径 fail-closed）。
        let workspace_root = match router
            .resolved_workspace_root(context, router.extract_call_workspace_id(call, context))
        {
            Ok(root) => root,
            Err((code, message)) => return error_result(TOOL_ACP_DELEGATE, &code, message, None),
        };

        // P3 trace：与 subagent 同构——执行前先建 `tool.call` span，外部 agent 的
        // `session/prompt` 因此挂到本次委派之下。
        let args_preview = serde_json::to_string(&call.arguments).ok();
        let _tool_span = crate::agent::otel::begin_tool_call_span(
            call.call_id.as_deref(),
            TOOL_ACP_DELEGATE,
            args_preview.as_deref(),
        );

        let args: AcpDelegateArgs = match serde_json::from_value(call.arguments.clone()) {
            Ok(args) => args,
            Err(error) => {
                return error_result(
                    TOOL_ACP_DELEGATE,
                    "invalid_arguments",
                    format!("Failed to parse acp_delegate arguments: {error}"),
                    None,
                )
            }
        };
        if args.prompt.trim().is_empty() {
            return error_result(
                TOOL_ACP_DELEGATE,
                "invalid_arguments",
                "acp_delegate 的 prompt 不能为空。".to_string(),
                None,
            );
        }

        // B170：纯输入校验必须在 spawn 之前（否则不可启动 agent 时会先报 acp_spawn_failed / 先付子进程开销）。
        if matches!(args.timeout_ms, Some(0)) {
            return error_result(
                TOOL_ACP_DELEGATE,
                "invalid_arguments",
                "acp_delegate 的 timeoutMs 必须大于 0。".to_string(),
                None,
            );
        }

        // 清单解析（用户清单 → 内置默认）；未知 agent 保留 `acp_config_agent_not_found` 来源。
        let spec = match crate::acp::config::resolve_agent(args.agent.trim()) {
            Ok(spec) => spec,
            Err(error) => {
                return error_result(
                    TOOL_ACP_DELEGATE,
                    error.code(),
                    error.to_string(),
                    Some("先在本机配置该 ACP agent，或改用内置 opencode / antigravity。".to_string()),
                )
            }
        };

        let cwd = match resolve_delegate_cwd(
            args.cwd.as_deref(),
            spec.default_cwd.as_deref(),
            &workspace_root,
        ) {
            Ok(cwd) => cwd,
            Err(message) => {
                return error_result(TOOL_ACP_DELEGATE, "invalid_arguments", message, None)
            }
        };

        // B175：fs/terminal 能力 handler 必须在握手**前**决定存在性（initialize 是第一条报文）。
        // 本层同时持有 `authorize_store`（PA-080 授权清单）与 `process_manager`（进程编排唯一口），
        // 因此可以真装配：`caps = hint ∧ handler 存在`（hint 只是上限；hint=false 时即使 handler
        // 可用也不声明）。绝不出现「声明了但没装」——缺失即由 session 侧 fail-closed。
        let fs_handler: Option<Arc<dyn crate::acp::client::FsHandler>> =
            spec.capabilities_hint.fs.then(|| {
                Arc::new(crate::acp::handlers::WorkspaceFsHandler::new(
                    cwd.clone(),
                    Arc::clone(router.authorize_store()),
                    // purpose 是审计标签；read/write 各自按自己的 PathPurpose 判定。
                    crate::agent::path_permission::PathPurpose::Read,
                )) as Arc<dyn crate::acp::client::FsHandler>
            });
        let terminal_handler: Option<Arc<dyn crate::acp::client::TerminalHandler>> =
            spec.capabilities_hint.terminal.then(|| {
                Arc::new(crate::acp::handlers::ManagedTerminalHandler::with_workspace_root(
                    Arc::new(router.process_manager().clone()),
                    cwd.clone(),
                )) as Arc<dyn crate::acp::client::TerminalHandler>
            });
        let fs_declared = fs_handler.is_some();
        let terminal_declared = terminal_handler.is_some();

        // B174：mcpServers 三态（inherit / 显式 [] / 显式条目）在本层解析后随 session/new 下发。
        let mcp_servers = match crate::acp::config::resolve_mcp_servers_in(&spec, &cwd) {
            Ok(servers) => servers,
            Err(error) => {
                return error_result(TOOL_ACP_DELEGATE, error.code(), error.to_string(), None)
            }
        };

        // 每任务一个子进程（§3.2.1：不并发打同一 session）。
        // B161：把 turn 级 ACP 权限 handler 一并装配（`None` ⇒ 会话侧
        // FailClosedPermissionHandler 兜底，权限请求一律 cancelled）。
        let permission = context.acp_permission.clone();
        let mut session = match crate::acp::session::AcpAgentSession::spawn_with_full_options(
            &spec,
            &cwd,
            mcp_servers,
            fs_handler,
            terminal_handler,
            permission,
        ) {
            Ok(session) => session,
            Err(error) => {
                return error_result(TOOL_ACP_DELEGATE, error.code(), error.to_string(), None)
            }
        };

        // 预算与取消令牌统一由 session 裁决（单一真源）：本层只做注入。宿主 `stop_turn` 的令牌
        // 必须进 session 的 100ms 轮询，否则生产路径上外部 agent 杀不掉；timeoutMs 进整轮预算，
        // 这里**不再**自建第二套超时/取消触发（双份预算会与 session 抢发 session/cancel）。
        //
        // timeoutMs 缺省 ⇒ 取 `acp` 层单一真源的执行策略默认预算（120s，
        // 与 `names.rs` 的 ToolExecutionPolicy / CLI `--timeout-secs` 同源），
        // **不得** leave 成 transport 的单请求 5s 硬顶（否则 >5s 的真实 agent 被误判 acp_timeout）。
        let turn_budget = match args.timeout_ms {
            Some(ms) => Duration::from_millis(ms),
            None => crate::acp::ACP_DEFAULT_TURN_BUDGET,
        };
        session.set_turn_budget(turn_budget);
        if let Some(token) = context.cancellation.as_ref() {
            session.set_cancellation_token(std::sync::Arc::clone(token));
        }
        // wave-acp-delegate C1（发射端）：把父 session/turn 与本次 call id 注入会话，委派期间的
        // ACP `tool_call` / `tool_call_update` 因此以**父 turn 身份**落 `turn:tool`（C1.1）。
        // 三要素缺一（无父 turn 上下文）⇒ 不注入，归集静默跳过（C1.3）；文本 chunk 不入父正文（C1.2）。
        let parent_turn = parent_turn_ref(context, call);
        if let Some(parent_turn) = parent_turn.as_ref() {
            session.set_parent_turn(parent_turn.clone());
        }
        // 可见性面（2026-11-22）：把本次委派登记进**活跃 ACP 注册表**，宿主 TUI 的 Agents 页签
        // 因此能看到「现在有哪些 ACP 子智能体在跑」。guard 覆盖 spawn 之后的全部收口路径
        // （成功/错误/取消/panic），Drop 即注销；注册表只做只读快照，不参与委派裁决。
        let _active_delegate = crate::acp::registry::ActiveDelegateGuard::register_with_description(
            crate::acp::registry::DelegateRegistration {
                agent_id: spec.id.clone(),
                acp_session_id: session.session_id().to_string(),
                host_session_id: context.session_id.clone(),
                cwd: cwd.display().to_string(),
            },
            args.description.clone(),
        );
        crate::agent::otel::log::log_json_with_fields(
            "info",
            "acp_delegate 起轮（budget/能力/父 turn 归属）",
            None,
            &[
                ("agent", json!(spec.id)),
                ("acp_session_id", json!(session.session_id())),
                ("budget_ms", json!(turn_budget.as_millis() as u64)),
                ("host_cancellation", json!(context.cancellation.is_some())),
                ("fs_handler", json!(fs_declared)),
                ("terminal_handler", json!(terminal_declared)),
                ("parent_turn_collected", json!(parent_turn.is_some())),
            ],
        );

        let blocks = vec![crate::acp::protocol::ContentBlock::Text {
            text: args.prompt,
        }];
        let result = run_delegate_prompt(&session, blocks);
        session.close();
        result
    }

    fn execute_workflow(_router: &pony_tools::tools::ToolRouter, call: &ToolCall) -> ToolResult {
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

    fn execute_spawn_teammate(router: &pony_tools::tools::ToolRouter, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_SPAWN_TEAMMATE);
        }

        let workspace_root = match router
            .resolved_workspace_root(context, router.extract_call_workspace_id(call, context))
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
            Ok(args) => match crate::agent::orchestration::spawn_teammate_in_workspace_with_provider_as(
                args,
                Some(workspace_root.as_path()),
                context.provider_selection.as_ref(),
                &crate::agent::orchestration::TeamCaller::from_session(context.session_id.as_deref()),
            ) {
                Ok(res) => {
                    // 会话隔离：登记该队友归属的宿主会话，TUI Agents 页签据此只显示本会话的队友。
                    if let Some(host) =
                        crate::agent::orchestration::host_session_for_caller(context.session_id.as_deref())
                    {
                        crate::agent::orchestration::set_teammate_host_session(
                            &res.member.target,
                            &host,
                        );
                    }
                    ToolResult {
                        tool_name: TOOL_SPAWN_TEAMMATE.to_string(),
                        status: "ok".to_string(),
                        output: json_string(json!({
                            "ok": true,
                            "tool": TOOL_SPAWN_TEAMMATE,
                            "result": res,
                        })),
                        duration_ms: 0,
                    }
                }
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

    pub(super) fn execute_list_agents(
        _router: &pony_tools::tools::ToolRouter,
        call: &ToolCall,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_LIST_AGENTS);
        }

        // 可选 wait_ms：把内核真边沿等待暴露给模型（工具名不变，避免工具面清单漂移）。
        // 等待期间任一团队边沿（成员状态 / 消息入队出队 / 任务流转）都会醒来；
        // 无活跃同伴时内核立即返回 no_progress，不会阻塞。
        //
        // B1795535505312：等待结果必须回传。此前 `Ok(_) => {}` 丢弃 `WaitAgentResult`，
        // 模型无法区分「等到了边沿」与「超时了」，只能把超时当进展继续猜。
        let mut wait: Option<WaitAgentResult> = None;
        if let Some(wait_ms) = call
            .arguments
            .get("wait_ms")
            .and_then(|value| value.as_u64())
        {
            match crate::agent::orchestration::wait_agent_as(
                WaitAgentArgs {
                    timeout_ms: Some(wait_ms),
                },
                &crate::agent::orchestration::TeamCaller::from_session(context.session_id.as_deref()),
            ) {
                Ok(result) => wait = Some(result),
                Err(err) => return error_result(TOOL_LIST_AGENTS, "list_agents_failed", err, None),
            }
        }

        match list_agents() {
            Ok(res) => ToolResult {
                tool_name: TOOL_LIST_AGENTS.to_string(),
                status: "ok".to_string(),
                output: json_string(json!({
                    "ok": true,
                    "tool": TOOL_LIST_AGENTS,
                    "result": res,
                    // `wait` 仅在调用方给了 `wait_ms` 时出现：`timed_out` / `no_progress`
                    // 是「这次等待发生了什么」的唯一可判读凭据。
                    "wait": wait,
                })),
                duration_ms: 0,
            },
            Err(err) => error_result(TOOL_LIST_AGENTS, "list_agents_failed", err, None),
        }
    }

    fn execute_send_message(router: &pony_tools::tools::ToolRouter, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_SEND_MESSAGE);
        }

        let workspace_root = match router
            .resolved_workspace_root(context, router.extract_call_workspace_id(call, context))
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

    pub(super) fn execute_interrupt_agent(
        _router: &pony_tools::tools::ToolRouter,
        call: &ToolCall,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_INTERRUPT_AGENT);
        }

        let args: Result<InterruptAgentArgs, _> = serde_json::from_value(call.arguments.clone());
        match args {
            Ok(args) => match crate::agent::orchestration::interrupt_agent_as(
                args,
                &crate::agent::orchestration::TeamCaller::from_session(context.session_id.as_deref()),
            ) {
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

    fn execute_drain_inbox(_router: &pony_tools::tools::ToolRouter, call: &ToolCall, context: &ToolExecutionContext) -> ToolResult {
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

    pub(super) fn execute_team_task_create(
        _router: &pony_tools::tools::ToolRouter,
        call: &ToolCall,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_TEAM_TASK_CREATE);
        }

        let args: Result<TeamTaskCreateArgs, _> = serde_json::from_value(call.arguments.clone());
        match args {
            Ok(args) => match crate::agent::orchestration::team_task_create_as(
                args,
                &crate::agent::orchestration::TeamCaller::from_session(context.session_id.as_deref()),
            ) {
                Ok(res) => {
                    // 会话隔离：队友（`team-*` 会话）创建的任务反查回宿主会话，否则团队任务会与
                    // 主会话失联（TUI Tasks 页签按宿主会话过滤）。
                    if let Some(host) =
                        crate::agent::orchestration::host_session_for_caller(context.session_id.as_deref())
                    {
                        crate::agent::orchestration::set_team_task_host_session(
                            &res.task_id,
                            &host,
                        );
                    }
                    ToolResult {
                        tool_name: TOOL_TEAM_TASK_CREATE.to_string(),
                        status: "ok".to_string(),
                        output: json_string(json!({
                            "ok": true,
                            "tool": TOOL_TEAM_TASK_CREATE,
                            "result": res,
                        })),
                        duration_ms: 0,
                    }
                }
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

    pub(super) fn execute_team_task_update(
        _router: &pony_tools::tools::ToolRouter,
        call: &ToolCall,
        context: &ToolExecutionContext,
    ) -> ToolResult {
        if !subagent_runtime_wired() {
            return capability_unavailable_result(TOOL_TEAM_TASK_UPDATE);
        }

        let args: Result<TeamTaskUpdateArgs, _> = serde_json::from_value(call.arguments.clone());
        match args {
            Ok(args) => match crate::agent::orchestration::team_task_update_as(
                args,
                &crate::agent::orchestration::TeamCaller::from_session(context.session_id.as_deref()),
            ) {
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

    fn execute_team_task_list(_router: &pony_tools::tools::ToolRouter, _call: &ToolCall) -> ToolResult {
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
// ───────────────────────────── B150/P1：acp_delegate 实现细节 ─────────────────────────────

/// wave-acp-delegate C1.1/C1.3/C1.5：父 turn 身份三要素的解析（发射端唯一入口）。
///
/// 真源：`ToolExecutionContext.session_id`（父会话）+ `dispatch_context.turn_id`（父 turn）
/// + `ToolCall.call_id`（本次委派调用 id）。任一缺失/空白 ⇒ `None` ⇒ 归集静默跳过，
/// **绝不**用 ACP session id 冒充父 turn id（C1.5）。
fn parent_turn_ref(
    context: &ToolExecutionContext,
    call: &ToolCall,
) -> Option<crate::acp::delegate_events::ParentTurnRef> {
    let dispatch_session = context
        .dispatch_context
        .as_ref()
        .and_then(|dispatch| dispatch.session_id.as_deref());
    let session_id = context
        .session_id
        .as_deref()
        .or(dispatch_session)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let turn_id = context
        .dispatch_context
        .as_ref()
        .and_then(|dispatch| dispatch.turn_id.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    let call_id = call
        .call_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())?;
    Some(crate::acp::delegate_events::ParentTurnRef {
        session_id: session_id.to_string(),
        turn_id: turn_id.to_string(),
        call_id: call_id.to_string(),
    })
}

/// `acp_delegate` 参数（schema 见 `agent/tools/schema.rs`；`description` 由全局
/// `with_description()` 注入，故不在此声明——未知字段 serde 默认忽略）。
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct AcpDelegateArgs {
    agent: String,
    prompt: String,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    /// 面向用户的中文描述（schema 已强制模型提供）：TUI Agents 页签优先展示它。
    #[serde(default)]
    description: Option<String>,
}

/// 委派 cwd 解析：显式 `cwd` > 清单 `default_cwd` > 会话 workspace root；一律 canonicalize。
fn resolve_delegate_cwd(
    explicit: Option<&str>,
    spec_default: Option<&str>,
    workspace_root: &std::path::Path,
) -> Result<std::path::PathBuf, String> {
    let candidate = match explicit.map(str::trim).filter(|value| !value.is_empty()) {
        Some(value) => {
            let path = std::path::PathBuf::from(value);
            if !path.is_absolute() {
                // 不静默 canonicalize 相对路径（§3.2.2）。
                return Err(format!(
                    "acp_delegate 的 cwd 必须是绝对路径（§3.2.2），实际 `{value}`。"
                ));
            }
            path
        }
        None => match spec_default.map(str::trim).filter(|value| !value.is_empty()) {
            Some(value) => std::path::PathBuf::from(value),
            None => workspace_root.to_path_buf(),
        },
    };
    let canonical = std::fs::canonicalize(&candidate).map_err(|error| {
        format!(
            "acp_delegate 的工作目录 `{}` 不可用（{error}）。",
            candidate.display()
        )
    })?;
    if !canonical.is_dir() {
        return Err(format!(
            "acp_delegate 的工作目录 `{}` 不是目录。",
            canonical.display()
        ));
    }
    Ok(canonical)
}

/// 跑一轮 prompt。整轮预算（`timeoutMs` → `session.set_turn_budget`）与取消（宿主令牌 →
/// session 内轮询）都由 [`acp::session::AcpAgentSession`] 裁决，超预算即归约 `acp_timeout`。
///
/// 事件上行说明：工具层没有 `TurnEventSink` 注入点（`ToolExecutionContext` 无该字段），
/// 因此这里用 `NoopTurnEventSink`；外部 agent 的可见性由 `tool.call` span + 归约后的
/// `ToolResult`（含 update 计数）承担，交互式实时渲染属 P2 宿主形态（CLI/TUI 持有 sink）。
fn run_delegate_prompt(
    session: &crate::acp::session::AcpAgentSession,
    blocks: Vec<crate::acp::protocol::ContentBlock>,
) -> ToolResult {
    // 预算（`timeoutMs` → `session.set_turn_budget`）与取消（`ToolExecutionContext.cancellation`
    // → session 内 `turn_cancel_poll_ms` 轮询）全部由 session 统一裁决；本函数只跑一轮 prompt。
    // 原先这里的 `recv_timeout + session.cancel` 兜底已删除：它与 session 内轮询并发触发，
    // 会形成双重 session/cancel 竞态与双份预算（哪一份先生效不确定）。
    let sink = crate::agent::turn_flow::NoopTurnEventSink;
    session.prompt(blocks, &sink)
}
