//! 运行时数据 → TUI 视图投影（原 `tui.rs` 内联实现，机械外移）。
//! 任务板 / 团队花名册 / Goal / Plan / 待处理控制请求的唯一投影真值源。

use super::*;
use crate::agent::todo::{todo_list, TodoListArgs};

/// 真实 todo 存储 → 任务看板投影。`id` 按存储顺序稳定生成，避免每帧抖动。
/// W3：`/tasks` 已切到**真实 Team 任务看板**（`orchestration::team_task_list`）。
///
/// 本函数保留为 todo 投影能力（todo 模块与 `todo_write`/`todo_list` 工具均未删除，
/// 供需要按会话查看 todo 的调用方复用），因此在此显式标注 dead_code 而非删除。
#[allow(dead_code)]
pub fn tui_tasks_from_todos(todos: Vec<TodoItem>) -> Vec<TuiTask> {
    todos
        .into_iter()
        .enumerate()
        .map(|(index, todo)| TuiTask {
            id: todo
                .step_id
                .clone()
                .filter(|id| !id.trim().is_empty())
                .unwrap_or_else(|| format!("todo-{}", index + 1)),
            subject: todo.content,
            status: match todo.status {
                StoreTodoStatus::Pending => TuiTaskStatus::Pending,
                StoreTodoStatus::InProgress => TuiTaskStatus::InProgress,
                StoreTodoStatus::Completed => TuiTaskStatus::Completed,
            },
            owner: None,
            blocked_by: Vec::new(),
            write_scopes: Vec::new(),
        })
        .collect()
}

/// Team 状态字符串 → TUI 状态枚举（前后端兼容口径）。
///
/// 团队看板的 `status` 是 wire 字符串（`pending/in_progress/completed/blocked`），并兼容
/// 历史/别名写法（`open`/`todo`/`done`/`running`/`failed`…）。**未知值一律降级 `Pending`**，
/// 绝不把状态吞成成功。
pub fn tui_task_status_from_team_status(status: &str) -> TuiTaskStatus {
    match status.trim().to_ascii_lowercase().as_str() {
        "completed" | "complete" | "done" | "closed" | "resolved" => TuiTaskStatus::Completed,
        "in_progress" | "in-progress" | "inprogress" | "running" | "active" | "started" => {
            TuiTaskStatus::InProgress
        }
        "blocked" | "failed" | "cancelled" | "canceled" | "rejected" => TuiTaskStatus::Blocked,
        _ => TuiTaskStatus::Pending,
    }
}

/// 真实 Team 任务存储 → TUI 任务看板投影（owner / blocked_by / write_scopes 原样透传）。
pub fn tui_tasks_from_team_tasks(tasks: Vec<TeamTaskItem>) -> Vec<TuiTask> {
    tasks
        .into_iter()
        .map(|task| TuiTask {
            id: task.task_id,
            subject: task.subject,
            status: tui_task_status_from_team_status(&task.status),
            owner: task.owner,
            blocked_by: task.blocked_by,
            write_scopes: task.write_scopes,
        })
        .collect()
}

/// 真实 **Team 任务**存储 → 任务看板（模型/Agent 调用 `team_task_*` 后由这里同步）。
///
/// W3：数据源从会话级 todo 切换为**团队共享看板**（`team_task_list()`）；空列表保留 `None`，
/// 让面板走「当前没有任务」空状态而不是 0/0 看板（与既有行为一致）。
pub fn sync_tasks_from_store(state: &mut TuiState) {
    match team_task_list() {
        Ok(list) => {
            let tasks = tui_tasks_from_team_tasks(list.tasks);
            if tasks.is_empty() {
                state.task_board_state = None;
            } else {
                state.update_task_board(tasks);
            }
        }
        Err(error) => {
            state.status_text = format!("任务同步失败: {error}");
        }
    }
}

/// 当前会话的 **todo 存储** → `TuiState::todos_state`（抽屉 Tasks 页签的唯一数据源）。
///
/// 数据源是 `todo_write` 写入的会话级注册表（进程内、无网络、无磁盘）：**不是** `task_board_state`
/// 那份进程全局团队看板 —— 单会话 TUI 下团队看板几乎恒空，旧实现让 Tasks 页签「永远没有任务」。
/// 空列表保留 `None`，让抽屉走一行空态而不是 0/0 进度。
pub fn sync_todos_from_store(state: &mut TuiState) {
    match todo_list(TodoListArgs {
        session_id: Some(state.session_id.clone()),
    }) {
        Ok(result) => {
            let todos = tui_tasks_from_todos(result.todos);
            state.todos_state = if todos.is_empty() { None } else { Some(todos) };
        }
        Err(error) => state.status_text = format!("任务同步失败: {error}"),
    }
}

/// control plane 计划存储 → `TuiState::plan_state`（抽屉 Plan 页签）。
///
/// 旧实现只有 `/plan` / `/approve` 两条命令会投影计划——模型在轮内 `plan_control` 建/改的计划
/// 在抽屉里**永远不可见**。本函数是打开抽屉与轮末的统一同步点；无计划 ⇒ 清空，避免把上一会话
/// 的计划残留成当前会话的实施计划。
pub fn sync_plan_from_store(state: &mut TuiState, control_plane: &HostControlPlane) {
    let plans = control_plane.plan_list(&state.session_id);
    match tui_plan_from_value(&plans) {
        Some(plan) => state.update_plan(plan),
        None => state.plan_state = None,
    }
}

/// 抽屉数据源的**常驻**刷新（500ms 与状态条摘要同周期）：todo + plan。
///
/// 两者都是进程内注册表（todo 是 HashMap，plan 是 PlanStore），不含网络/磁盘，可安全节流刷新。
/// goal 不在此列：它有自己的命令/事件回写通道，且读取失败会 fail-closed 清横幅，不宜每 500ms 触发。
pub fn sync_drawer_live_sources(state: &mut TuiState, control_plane: &HostControlPlane) {
    sync_todos_from_store(state);
    sync_plan_from_store(state, control_plane);
}

/// 打开/切换抽屉时的**全量**同步点：live 数据源 + jobs + goal（含 fail-closed 清空语义）。
///
/// `jobs_panel::sync_jobs_from_registry` 放在这里而非 [`sync_drawer_live_sources`]，
/// 因为 `status_bar::sync_task_glance` 在 500ms 周期内已先行刷新 jobs，避免同一节流周期内
/// 对作业子进程做两次 `try_wait`/`poll`。
pub fn sync_drawer_sources(state: &mut TuiState, control_plane: &HostControlPlane) {
    sync_drawer_live_sources(state, control_plane);
    jobs_panel::sync_jobs_from_registry(state);
    hydrate_goal_state(state, &state.session_id.clone());
}

/// 计划是否已终态（`completed` / `aborted`）：终态计划只在「没有未终态计划」时兜底展示。
fn is_terminal_plan(plan: &serde_json::Value) -> bool {
    matches!(
        plan.get("lifecycle").and_then(|lifecycle| lifecycle.as_str()),
        Some("completed" | "aborted")
    )
}


/// W3：名册状态 → 面板展示文案（wire 小写，与 `TeammateStatus` 的 serde 口径一致）。
pub fn teammate_status_label(status: &TeammateStatus) -> String {
    match status {
        TeammateStatus::Provisioning => "provisioning",
        TeammateStatus::Active => "active",
        TeammateStatus::Running => "running",
        TeammateStatus::Inactive => "inactive",
        TeammateStatus::Failed => "failed",
        TeammateStatus::Interrupted => "interrupted",
    }
    .to_string()
}

/// W3：会话 id 的展示尾号（面板宽度有限，只显示末 4 位便于对照日志）。
pub fn session_tail(session_id: &str) -> String {
    let chars: Vec<char> = session_id.chars().collect();
    let start = chars.len().saturating_sub(4);
    chars[start..].iter().collect()
}

/// W3：真实运行时 → `/agents` 花名册快照。
///
/// 数据源全部是真实存储（`list_agents` 名册 / 收件箱 / 驱动态 / 最近成功会话），
/// 不构造任何假成员；`lead` 固定排首位（其余按 target 字典序，保证面板顺序稳定）。
pub fn agent_roster_state_from_runtime() -> AgentRosterState {
    let mut entries: Vec<AgentRosterEntry> = match list_agents() {
        Ok(roster) => roster
            .agents
            .into_iter()
            .map(|member| AgentRosterEntry {
                pending_inbox: inbox_pending_len(&member.target),
                driving: is_driving(&member.target),
                session_id: teammate_session_id(&member.target),
                target: member.target,
                name: member.name,
                role: member.role,
                status: teammate_status_label(&member.status),
            })
            .collect(),
        Err(error) => {
            eprintln!("[tui] /agents 名册读取失败: {error}");
            Vec::new()
        }
    };
    entries.sort_by(|left, right| {
        (left.target != "lead")
            .cmp(&(right.target != "lead"))
            .then_with(|| left.target.cmp(&right.target))
    });
    AgentRosterState::new(entries)
}

/// 活跃 ACP 委派注册表 → TUI 视图行（按当前宿主会话过滤；`None` 宿主的条目不串到任意会话）。
///
/// 为什么按会话过滤：`acp_delegate` 会话由**某个宿主会话的轮次线程**发起，别的会话的 Agents
/// 页签里出现它既不可操作也会误导（与 B012 作业归属同一条纪律）。
pub fn acp_delegates_from_runtime(host_session_id: &str) -> Vec<AcpDelegateRow> {
    let target = host_session_id.trim();
    if target.is_empty() {
        return Vec::new();
    }
    crate::acp::registry::active_delegates_for_session(target)
        .into_iter()
        .map(|delegate| AcpDelegateRow {
            delegate_id: delegate.delegate_id,
            agent_id: delegate.agent_id,
            acp_session_id: delegate.acp_session_id,
            host_session_id: delegate.host_session_id,
            cwd: delegate.cwd,
            started_at_ms: delegate.started_at_ms,
        })
        .collect()
}

/// W3b：名册快照 → 可切换的子会话候选（顺序与输入一致，不重排、不去重）。
///
/// 剔除三类条目：`lead`（即宿主主会话，无需进入）、尚无会话（未成功运行过）、
/// 会话不可由宿主存储加载（hydrate 只会得到空会话）。`loadable` 由调用方注入真实判据。
pub fn sub_session_candidates(
    entries: &[AgentRosterEntry],
    loadable: impl Fn(&str) -> bool,
) -> Vec<SubSessionCandidate> {
    entries
        .iter()
        .filter(|entry| entry.target != "lead")
        .filter_map(|entry| {
            let session_id = entry.session_id.as_deref()?;
            if !loadable(session_id) {
                return None;
            }
            Some(SubSessionCandidate {
                target: entry.target.clone(),
                name: entry.name.clone(),
                role: entry.role.clone(),
                session_id: session_id.to_string(),
            })
        })
        .collect()
}

/// W3b：在候选序列里按方向取下一个目标（命中则循环，未命中/None 取首/末）。
pub fn step_sub_session_target<'a>(
    candidates: &'a [SubSessionCandidate],
    current: Option<&str>,
    step: SubSessionStep,
) -> Option<&'a SubSessionCandidate> {
    if candidates.is_empty() {
        return None;
    }
    let position =
        current.and_then(|target| candidates.iter().position(|item| item.target == target));
    match (position, step) {
        (Some(index), SubSessionStep::Next) => Some(&candidates[(index + 1) % candidates.len()]),
        (Some(index), SubSessionStep::Prev) => {
            let last = candidates.len() - 1;
            Some(&candidates[(index + last) % candidates.len()])
        }
        (None, SubSessionStep::Next) => candidates.first(),
        (None, SubSessionStep::Prev) => candidates.last(),
    }
}

/// W3b：子会话身份徽标文案（**唯一真值**；渲染层只负责样式与截断）。
pub fn sub_session_badge_text(context: &SubSessionContext) -> String {
    let seq = match context.position {
        Some((index, total)) => format!(" {index}/{total}"),
        None => String::new(),
    };
    format!(
        "子智能体 · {}（{}）{} · Ctrl+↑ 返回主会话 · Ctrl+←/→ 切换",
        context.name, context.role, seq
    )
}

/// W3b：`/agents` 及其别名 `/teams` 的判定（`trim()` 后大小写不敏感）。
pub fn is_agent_roster_command(command: &str) -> bool {
    let trimmed = command.trim();
    ["/agents", "/teams"]
        .iter()
        .any(|cmd| cmd.eq_ignore_ascii_case(trimmed))
}

/// 真实 goal 存储 → TUI goal 投影。phase 用现有字符串契约，未知值降级为 `Active`。
pub fn tui_goal_from_goal(goal: &GoalData) -> TuiGoal {
    TuiGoal {
        id: goal.id.clone(),
        objective: goal.objective.clone(),
        phase: match goal.phase.as_str() {
            "completed" => GoalPhase::Completed,
            "blocked" => GoalPhase::Blocked,
            "paused" => GoalPhase::Paused,
            _ => GoalPhase::Active,
        },
        activation: match goal.activation {
            AgentGoalActivation::Armed => GoalActivation::Armed,
            AgentGoalActivation::Disarmed => GoalActivation::Disarmed,
        },
        round_count: goal.rounds_started,
        max_rounds: Some(goal.max_goal_rounds),
        revision: goal.revision,
    }
}

/// `control_plane.plan_list(session)` 的 JSON 投影 → PlanState。空数组/格式不符返回 `None`。
///
/// 版本选择口径（2026-11-22）：优先**最新一份未终态**计划（`draft` / `executing`）；
/// 该会话所有计划都已终态时退回**最新一份**。旧实现固定取 `first()`——会话产生第二份计划后
/// 抽屉会一直显示最早那份，新计划永远不出现（用户侧表现为「plan 从没显示过」）。
pub fn tui_plan_from_value(value: &serde_json::Value) -> Option<PlanState> {
    let plans = value.as_array()?;
    let plan = plans
        .iter()
        .rev()
        .find(|plan| !is_terminal_plan(plan))
        .or_else(|| plans.last())?;
    let steps = plan
        .get("steps")
        .and_then(|steps| steps.as_array())
        .map(|steps| {
            steps
                .iter()
                .enumerate()
                .map(|(index, step)| PlanStep {
                    step_id: step
                        .get("stepId")
                        .and_then(|id| id.as_str())
                        .map(str::to_string)
                        .unwrap_or_else(|| format!("step-{}", index + 1)),
                    name: step
                        .get("name")
                        .and_then(|name| name.as_str())
                        .unwrap_or("未命名步骤")
                        .to_string(),
                    summary: step
                        .get("summary")
                        .and_then(|summary| summary.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    status: match step.get("status").and_then(|status| status.as_str()) {
                        Some("completed") => PlanStepStatus::Completed,
                        Some("in_progress") => PlanStepStatus::InProgress,
                        Some("failed") => PlanStepStatus::Failed,
                        _ => PlanStepStatus::Pending,
                    },
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Some(PlanState {
        plan_id: plan
            .get("planId")
            .and_then(|id| id.as_str())
            .unwrap_or("plan")
            .to_string(),
        revision: plan.get("revision").and_then(|rev| rev.as_u64()).unwrap_or(0),
        summary: plan
            .get("summary")
            .and_then(|summary| summary.as_str())
            .unwrap_or_default()
            .to_string(),
        steps,
        is_active_plan_mode: true,
        selected_step_index: 0,
        awaiting_approval: plan
            .get("lifecycle")
            .and_then(|lifecycle| lifecycle.as_str())
            .is_some_and(|lifecycle| lifecycle == "draft"),
    })
}

/// `list_pending_asks` 数组项 → TUI 句柄。缺 `requestId`/`version` 视为不可消费，返回 `None`。
pub fn pending_ui_request_from_value(value: &serde_json::Value) -> Option<PendingUiRequest> {
    let request_id = value.get("requestId")?.as_str()?.to_string();
    let version = value.get("version")?.as_u64()?;
    let kind = value
        .get("requestKind")
        .and_then(|kind| kind.as_str())
        .unwrap_or("interaction");
    let is_approval = kind == "approval" || kind == "approval_required";
    Some(PendingUiRequest {
        request_id,
        version,
        run_id: value
            .get("runId")
            .and_then(|run_id| run_id.as_str())
            .map(str::to_string),
        // `expiresAtMs` 缺失/null 表示无 TTL；`0` 同样按无 TTL 处理（不是「立刻过期」）。
        expires_at_ms: value
            .get("expiresAtMs")
            .and_then(|ms| ms.as_u64())
            .filter(|ms| *ms > 0),
        is_approval,
    })
}

/// 待决请求 → 问答对话框数据。`options` 若已是 `{label,description}` 数组则直接采用。
pub fn ask_question_from_pending(value: &serde_json::Value) -> AskUserQuestion {
    let question = value
        .get("prompt")
        .and_then(|prompt| prompt.as_str())
        .unwrap_or("（宿主请求缺少提问内容）")
        .to_string();
    let options = value
        .get("options")
        .and_then(|options| options.as_array())
        .map(|options| {
            options
                .iter()
                // 生产 schema 的 `ask_user.options` 是 **string 数组**（见 tools.rs 的 input_schema），
                // 而历史实现只认 `{label, description}` 对象 ⇒ 真实模型给的选项会被整批丢掉，
                // 面板只剩「自定义输入」。这里两种形状都接受：字符串 ⇒ 纯标签，对象 ⇒ 标签+说明。
                .filter_map(|option| match option {
                    serde_json::Value::String(label) => {
                        (!label.trim().is_empty()).then(|| QuestionOption {
                            label: label.clone(),
                            description: None,
                        })
                    }
                    other => other
                        .get("label")
                        .and_then(|label| label.as_str())
                        .map(|label| QuestionOption {
                            label: label.to_string(),
                            description: other
                                .get("description")
                                .and_then(|desc| desc.as_str())
                                .map(str::to_string),
                        }),
                })
                .collect::<Vec<_>>()
        })
        // 契约 F-M①：无 options 的提问**不伪造**「确认/取消」两项。
        // 伪造会让一次 Enter 就回一个用户从没选过的"确认"，是真实缺陷；空列表 ⇒ 走自由输入。
        .unwrap_or_default();
    AskUserQuestion {
        id: "q1".to_string(),
        header: value
            .get("descriptorId")
            .and_then(|id| id.as_str())
            .map(str::to_string),
        question,
        // 多选语义由模型声明的 `multiSelect` 决定（宿主 pending 请求的 additive 线缆键）；
        // 缺失（旧宿主 / 旧记录）⇒ 单选，与历史行为逐字一致。
        multi_select: value
            .get("multiSelect")
            .and_then(|flag| flag.as_bool())
            .unwrap_or(false),
        options,
    }
}

/// 待决审批请求 → 权限对话框数据。
pub fn permission_request_from_pending(
    value: &serde_json::Value,
    session_id: &str,
) -> PermissionRequest {
    let target_path = value
        .get("options")
        .and_then(|opts| opts.get("path").or_else(|| opts.get("file_path")))
        .and_then(|p| p.as_str())
        .map(str::to_string)
        .or_else(|| {
            value
                .get("callId")
                .and_then(|id| id.as_str())
                .map(str::to_string)
        });
    PermissionRequest {
        request_id: value
            .get("requestId")
            .and_then(|id| id.as_str())
            .unwrap_or_default()
            .to_string(),
        session_id: value
            .get("sessionId")
            .and_then(|id| id.as_str())
            .unwrap_or(session_id)
            .to_string(),
        // 工具名只呈现**人类可读**的模型名（`options.tool`，如 `Read`）：
        // `descriptorId` 是运行时身份（`builtin:workspace_read_file`），属工程信息，不上屏。
        // 仅当来源不是内建工具（如 MCP / 插件）且没有结构化 `tool` 时，才用 descriptorId 兜底
        // —— 那些 id 本身就是产品可见的工具名，而不是 `builtin:` 内部原语。
        tool_name: value
            .get("options")
            .and_then(|options| options.get("tool"))
            .and_then(|tool| tool.as_str())
            .map(str::to_string)
            .or_else(|| {
                value
                    .get("descriptorId")
                    .and_then(|id| id.as_str())
                    .filter(|id| !id.starts_with("builtin:"))
                    .map(str::to_string)
            })
            .unwrap_or_default(),
        description: value
            .get("prompt")
            .and_then(|prompt| prompt.as_str())
            .unwrap_or("（宿主请求缺少说明）")
            .to_string(),
        command_or_path: target_path,
        // wave-65 R2（F3）：**授权**用的路径只认结构化 `options.path`（契约 C4 原文「只能取
        // `options.path`」）。刻意不接受的两种来源：`file_path`/`filePath` 是历史别名（C2 只写
        // `path`，别名会把两条真值源并成双份维护），`callId` 更不是路径（canonicalize 必败 ⇒
        // 无 path 审批会变成永远批不掉）。缺省即 `None` ⇒ 决策时跳过授权直接 CAS。
        approval_path: value
            .get("options")
            .and_then(|options| options.get("path"))
            .and_then(|path| path.as_str())
            .map(str::to_string),
        timestamp_ms: value
            .get("expiresAtMs")
            .and_then(|ms| ms.as_u64())
            .unwrap_or(0),
    }
}

/// 契约 C-5：斜杠命令解析口径（纯函数，可单测）。
///
/// 取 `trim()` 后**首个空白分隔 token** 与命令表做**大小写不敏感**比较；命中返回命令表中的
/// 规范写法（如 `"/plan"`、`"/compact"`）。未命中返回 `None`——调用方按普通消息发送
/// （`/etc/hosts 是什么` 之类不新增拦截）。
pub fn match_command_token(input: &str) -> Option<&'static str> {
    let token = input.trim().split_whitespace().next()?;
    AVAILABLE_COMMANDS
        .iter()
        .map(|(cmd, _)| *cmd)
        .find(|cmd| cmd.eq_ignore_ascii_case(token))
}

/// 契约 F-B：**分发口径**的斜杠命令解析（整串匹配）。
///
/// 只有整串（`trim()` 后、大小写不敏感）恰好等于命令表中的命令才返回规范写法；
/// **首 token 命中但存在额外 token ⇒ `None`**，调用方把整串当普通消息发送给模型，
/// 绝不吞掉正文（`/clear 这个需求怎么拆` 不得清空上下文）。
/// 菜单补全/高亮请用 [`match_command_token`]（首 token 口径）。
pub fn parse_command_input(input: &str) -> Option<&'static str> {
    let trimmed = input.trim();
    if trimmed.split_whitespace().nth(1).is_some() {
        return None;
    }
    match_command_token(trimmed)
}

/// 整串（`trim()` 后）与历史别名大小写不敏感相等（别名不在命令表内，如 `/exit`、`/effort`）。
pub fn input_is_alias(input: &str, alias: &str) -> bool {
    input.trim().eq_ignore_ascii_case(alias)
}

/// 历史别名命令（不在 [`AVAILABLE_COMMANDS`] 表内，但仍是命令语义，不得当聊天消息发出去）。
pub const HISTORIC_COMMAND_ALIASES: &[&str] = &["/exit", "/effort"];

/// 契约 F-D：**拼错的斜杠命令**识别（纯函数，可单测）。
///
/// 判定面只取 `trim()` 后的**首个 token**，且要求它「形如命令词」：单个前导 `/` + 仅含
/// 字母/数字/`-`/`_`。命中命令表或历史别名 ⇒ `None`（不是拼错）。
///
/// 刻意返回 `None` 的反例（保持契约 F-B「不吞正文」）：
/// - 含第二个 `/`（`/etc/hosts 是什么`、`/usr/bin`）——路径/表达式，不是命令；
/// - 裸 `/`、`/ ` 这类没有命令词的输入；
/// - 已知命令后跟参数（`/clear 这个需求怎么拆`）——首 token 命中命令表。
pub fn unknown_slash_command(input: &str) -> Option<&str> {
    let token = input.trim().split_whitespace().next()?;
    if token.starts_with("/skill:") {
        return None;
    }
    let name = token.strip_prefix('/')?;
    let command_shaped = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    if !command_shaped {
        return None;
    }
    if match_command_token(token).is_some() {
        return None;
    }
    if HISTORIC_COMMAND_ALIASES
        .iter()
        .any(|alias| alias.eq_ignore_ascii_case(token))
    {
        return None;
    }
    Some(token)
}

/// 契约 F-D：未知斜杠命令被拦下时的用户提示（文案唯一真值）。
pub fn unknown_command_notice(token: &str) -> String {
    format!("未知斜杠命令「{token}」· 已拦截未发送 · 输入 /help 查看全部可用命令")
}
