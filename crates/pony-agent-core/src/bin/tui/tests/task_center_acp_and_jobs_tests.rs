//! Ctrl+T 任务抽屉的**归属与可见性**（2026-11-22 批次 C）：
//! 1. Jobs 页签只显示**当前会话**的后台任务（契约 B012，`job_list_scoped`）；
//! 2. Agents 页签同时显示 teammate 名册与**活跃 ACP 委派**（`acp_delegate` 运行期可见）。

use super::*;
use pony_agent_core::acp::registry::{ActiveDelegateGuard, DelegateRegistration};
use pony_agent_core::agent::jobs::{
    job_kill_scoped, job_start_scoped, JobKillArgs, JobStartArgs,
};

/// 打开抽屉并切到指定页签（其余状态保持默认空）。
fn drawer_state_on(tab: task_center::TaskCenterTab) -> TuiState {
    let mut state = TuiState::default();
    state.task_center = Some(task_center::TaskCenterState::new());
    state.task_center.as_mut().expect("抽屉已打开").tab = tab;
    state
}

/// 登记一条活跃 ACP 委派（真实注册表，不做 mock；guard 在用例结束自动注销）。
fn register_delegate(host_session_id: Option<&str>, agent_id: &str) -> ActiveDelegateGuard {
    ActiveDelegateGuard::register(DelegateRegistration {
        agent_id: agent_id.to_string(),
        acp_session_id: format!("acp-{agent_id}-session"),
        host_session_id: host_session_id.map(str::to_string),
        cwd: "/tmp".to_string(),
    })
}

#[test]
fn agents_tab_lists_active_acp_delegate_for_current_session() {
    let session = "b270-acp-visible";
    let guard = register_delegate(Some(session), "opencode");

    let mut state = drawer_state_on(task_center::TaskCenterTab::Agents);
    state.session_id = session.to_string();
    state.acp_delegates = acp_delegates_from_runtime(session);
    assert_eq!(state.acp_delegates.len(), 1, "当前会话必须看到自己的 ACP 委派");
    assert_eq!(state.acp_delegates[0].agent_id, "opencode");

    let lines = task_center::content_lines(&state);
    let joined = lines
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("ACP 委派 · 外部进程"),
        "Agents 页签必须有 ACP 委派分区：{joined}"
    );
    assert!(
        !joined.contains("running"),
        "Agents 页签不得出现英文状态词（改用语义图标）：{joined}"
    );
    assert!(joined.contains("opencode"), "必须显示外部 agent 标识：{joined}");

    drop(guard);
    // 委派结束后（guard drop）同一快照立即消失：页签不残留「跑完还挂着」的条目。
    state.acp_delegates = acp_delegates_from_runtime(session);
    assert!(state.acp_delegates.is_empty(), "委派结束后不得残留");
    let lines = task_center::content_lines(&state);
    assert_eq!(lines.len(), 1, "无委派且无名册 ⇒ 一行空态：{lines:?}");
}

#[test]
fn acp_delegate_of_another_session_is_not_shown() {
    let guard = register_delegate(Some("b270-other-session"), "antigravity");
    let mine = "b270-my-session";
    assert!(
        acp_delegates_from_runtime(mine).is_empty(),
        "别的会话的 ACP 委派不得出现在本会话页签（跨会话可见性也算泄露）"
    );
    // 反例：宿主会话为空的无归属委派同样不串到任意会话。
    let unattributed = register_delegate(None, "opencode");
    assert!(
        acp_delegates_from_runtime(mine).is_empty(),
        "无宿主会话的委派不归任何会话"
    );
    drop(unattributed);
    drop(guard);
}

#[test]
fn jobs_tab_is_scoped_to_current_session() {
    let owner = "b269-jobs-owner";
    let other = "b269-jobs-other";
    // 真实起一个短命后台作业（用 sleep 保证在本用例期间仍 running），结束后显式 kill。
    let started = job_start_scoped(
        JobStartArgs {
            command: "sleep".to_string(),
            args: Some(vec!["5".to_string()]),
            cwd: None,
            timeout_ms: None,
            description: None,
        },
        None,
        Some(owner),
    )
    .expect("测试后台作业必须能启动");
    let job_id = started.job_id.clone();

    let mut state = drawer_state_on(task_center::TaskCenterTab::Jobs);
    state.session_id = owner.to_string();
    jobs_panel::sync_jobs_from_registry(&mut state);
    let view = state.jobs_state.as_ref().expect("同步后必须有作业快照");
    assert!(
        view.jobs.iter().any(|job| job.id == job_id),
        "作业 owner 会话必须能看到自己的后台任务：{view:?}"
    );

    // 换一个会话：同一条作业不可见（B012：非 owner 连存在性都不暴露）。
    state.session_id = other.to_string();
    jobs_panel::sync_jobs_from_registry(&mut state);
    let view = state.jobs_state.as_ref().expect("同步后必须有作业快照");
    assert!(
        !view.jobs.iter().any(|job| job.id == job_id),
        "非 owner 会话不得看到该后台任务：{view:?}"
    );

    // 清场：显式杀作业（失败不 panic，避免泄漏子进程）。
    let _ = job_kill_scoped(
        JobKillArgs {
            job_id,
            reason: Some("test cleanup".to_string()),
        },
        Some(owner),
    );
}

// ---------------------------------------------------------------------------
// 2026-11-25：Agents 分区准确性（伪成员 lead / 子代理 / 纯图标）与 Jobs kill
// ---------------------------------------------------------------------------

/// 伪成员 `lead` 不得出现在抽屉 Agents 页签（它就是宿主主会话）。
#[test]
fn agents_tab_hides_lead_pseudo_member() {
    let mut state = drawer_state_on(task_center::TaskCenterTab::Agents);
    state.agent_roster_state = Some(AgentRosterState::new(vec![
        AgentRosterEntry {
            target: "lead".to_string(),
            name: "Lead".to_string(),
            role: "lead".to_string(),
            status: "running".to_string(),
            pending_inbox: 0,
            driving: true,
            session_id: None,
        },
        AgentRosterEntry {
            target: "worker-1".to_string(),
            name: "Ada".to_string(),
            role: "executor".to_string(),
            status: "inactive".to_string(),
            pending_inbox: 0,
            driving: false,
            session_id: Some("sess-worker-1".to_string()),
        },
    ]));
    let joined = task_center::content_lines(&state)
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("Ada"), "队友必须可见：{joined}");
    assert!(!joined.contains("Lead"), "伪成员 lead 不得上屏：{joined}");
    assert!(
        !joined.contains("running") && !joined.contains("inactive"),
        "不得出现英文状态词（改用语义图标）：{joined}"
    );
    assert_eq!(
        task_center::selectable_count(&state),
        1,
        "只有具备会话的队友才可进入"
    );
}

/// 子智能体分区必须展示本会话的 subagent 运行记录，且可被选为进入目标。
#[test]
fn agents_tab_lists_subagent_runs_and_allows_entering() {
    let mut state = drawer_state_on(task_center::TaskCenterTab::Agents);
    state.subagent_runs = vec![SubagentViewRow {
        session_id: "subagent-abc".to_string(),
        description: "梳理调用链".to_string(),
        status: pony_agent_core::agent::orchestration::SubagentRunStatus::Completed,
        started_at_ms: 0,
        error: None,
    }];
    let joined = task_center::content_lines(&state)
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("子智能体"), "必须有子智能体分区：{joined}");
    assert!(joined.contains("梳理调用链"), "必须展示中文描述：{joined}");

    let picks = task_center::agent_picks(&state);
    assert_eq!(picks.len(), 1, "子代理必须可被选中：{picks:?}");
    assert_eq!(
        picks[0],
        task_center::AgentPick::Subagent {
            session_id: "subagent-abc".to_string()
        }
    );
    assert_eq!(task_center::selectable_count(&state), 1);
}

/// ACP 委派标注「外部进程 · 不可切入」，且不进入可选序列。
#[test]
fn acp_delegate_is_marked_external_and_not_selectable() {
    let mut state = drawer_state_on(task_center::TaskCenterTab::Agents);
    state.acp_delegates = vec![AcpDelegateRow {
        delegate_id: 7,
        agent_id: "opencode".to_string(),
        acp_session_id: "acp-1".to_string(),
        host_session_id: Some("host".to_string()),
        cwd: "/tmp/project".to_string(),
        started_at_ms: 0,
        description: Some("重构工具链".to_string()),
    }];
    let joined = task_center::content_lines(&state)
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("重构工具链"), "优先展示中文描述：{joined}");
    assert!(
        joined.contains("外部进程") && joined.contains("不可切入"),
        "必须标注外部进程不可切入：{joined}"
    );
    assert_eq!(
        task_center::selectable_count(&state),
        0,
        "ACP 委派不可被选中进入"
    );
}

/// 无描述时 ACP 行回退 cwd 末段目录名。
#[test]
fn acp_delegate_falls_back_to_cwd_segment() {
    let mut state = drawer_state_on(task_center::TaskCenterTab::Agents);
    state.acp_delegates = vec![AcpDelegateRow {
        delegate_id: 8,
        agent_id: "antigravity".to_string(),
        acp_session_id: "acp-2".to_string(),
        host_session_id: Some("host".to_string()),
        cwd: "/home/dm/projects/pony".to_string(),
        started_at_ms: 0,
        description: None,
    }];
    let joined = task_center::content_lines(&state)
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("pony"), "无描述时回退 cwd 末段：{joined}");
    assert!(!joined.contains("/home/dm"), "不得展示完整路径：{joined}");
}

/// Jobs 页签：优先中文描述，且不得出现 pid / owned / promoted / 英文状态词。
#[test]
fn jobs_tab_prefers_description_and_hides_internal_fields() {
    let mut state = drawer_state_on(task_center::TaskCenterTab::Jobs);
    state.jobs_state = Some(jobs_panel::JobsViewState {
        jobs: vec![jobs_panel::JobViewRow {
            id: "job-desc".to_string(),
            command: "npm run build -- --watch --mode development".to_string(),
            description: Some("构建前端产物".to_string()),
            status: jobs_panel::JobStatus::Running,
            started_at: 0,
        }],
    });
    let joined = task_center::content_lines(&state)
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("构建前端产物"), "必须优先展示中文描述：{joined}");
    for forbidden in ["pid", "owned", "promoted", "running"] {
        assert!(
            !joined.contains(forbidden),
            "不得展示内部字段 {forbidden:?}：{joined}"
        );
    }
    assert!(
        !joined.contains("npm run build"),
        "有描述时不得再摊原始命令：{joined}"
    );
}

/// Jobs 页签：无描述时回退精简命令，并在运行中显示底部终止提示。
#[test]
fn jobs_tab_falls_back_to_compact_command_and_shows_kill_hint() {
    let mut state = drawer_state_on(task_center::TaskCenterTab::Jobs);
    state.jobs_state = Some(jobs_panel::JobsViewState {
        jobs: vec![jobs_panel::JobViewRow {
            id: "job-cmd".to_string(),
            command: "cargo   test\n  --all".to_string(),
            description: None,
            status: jobs_panel::JobStatus::Running,
            started_at: 0,
        }],
    });
    let joined = task_center::content_lines(&state)
        .iter()
        .map(|(text, _)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(joined.contains("cargo test --all"), "命令必须单行化：{joined}");
    assert!(
        joined.contains("x / Ctrl+K 终止后台任务"),
        "有运行中作业时必须给终止提示：{joined}"
    );
}

#[cfg(windows)]
fn long_sleep_args() -> (String, Vec<String>) {
    (
        "cmd".to_string(),
        vec!["/C".to_string(), "ping -n 6 127.0.0.1 >nul".to_string()],
    )
}

#[cfg(not(windows))]
fn long_sleep_args() -> (String, Vec<String>) {
    ("sleep".to_string(), vec!["5".to_string()])
}

/// 进程级无归属作业（`owner = None`）与空白 `session_id` 不得串入具体会话的 Jobs 页签；
/// 同时内部无作用域直调 `job_list` 仍可看到无归属作业。
#[test]
fn jobs_tab_hides_unowned_jobs_and_blank_session_caller() {
    use pony_agent_core::agent::jobs::{job_kill, job_list, JobListArgs};

    let owner = "b269-jobs-strict-owner";
    let other = "b269-jobs-strict-other";
    let (cmd, args) = long_sleep_args();

    let unowned = job_start_scoped(
        JobStartArgs {
            command: cmd.clone(),
            args: Some(args.clone()),
            cwd: None,
            timeout_ms: None,
            description: Some("无归属测试任务".to_string()),
        },
        None,
        None,
    )
    .expect("无归属测试后台作业必须能启动");

    let owned = job_start_scoped(
        JobStartArgs {
            command: cmd,
            args: Some(args),
            cwd: None,
            timeout_ms: None,
            description: Some("当前会话专属任务".to_string()),
        },
        None,
        Some(owner),
    )
    .expect("归属测试后台作业必须能启动");

    let mut state = drawer_state_on(task_center::TaskCenterTab::Jobs);

    // 1) owner 会话：只见自己归属的作业，不见无归属作业。
    state.session_id = owner.to_string();
    jobs_panel::sync_jobs_from_registry(&mut state);
    let view = state.jobs_state.as_ref().expect("同步后必须有作业快照");
    assert!(
        view.jobs.iter().any(|job| job.id == owned.job_id),
        "owner 会话必须看到自己的后台任务：{view:?}"
    );
    assert!(
        !view.jobs.iter().any(|job| job.id == unowned.job_id),
        "无归属作业（owner=None）不得串入具体会话的 Jobs 页签：{view:?}"
    );

    // 2) 其他会话：既不见 owner 作业，也不见无归属作业。
    state.session_id = other.to_string();
    jobs_panel::sync_jobs_from_registry(&mut state);
    let view = state.jobs_state.as_ref().expect("同步后必须有作业快照");
    assert!(
        !view.jobs.iter().any(|job| job.id == owned.job_id || job.id == unowned.job_id),
        "其他会话不得看到归属他人的或无归属的作业：{view:?}"
    );

    // 3) 空白 session_id：fail-closed 返回空列表。
    state.session_id = "   ".to_string();
    jobs_panel::sync_jobs_from_registry(&mut state);
    let view = state.jobs_state.as_ref().expect("同步后必须有作业快照");
    assert!(
        view.jobs.is_empty(),
        "空白 session_id 必须 fail-closed 返回空列表：{view:?}"
    );

    // 4) 内部无作用域直调 `job_list`：仍保留全量语义（可见无归属作业）。
    let unscoped = job_list(JobListArgs {}).expect("job_list 必须成功");
    assert!(
        unscoped.jobs.iter().any(|job| job.job_id == unowned.job_id),
        "无作用域 job_list 仍应能看到无归属作业"
    );

    let _ = job_kill(JobKillArgs {
        job_id: unowned.job_id,
        reason: Some("test cleanup".to_string()),
    });
    let _ = job_kill_scoped(
        JobKillArgs {
            job_id: owned.job_id,
            reason: Some("test cleanup".to_string()),
        },
        Some(owner),
    );
}

/// 切换会话（`reset_session_scope` / `clear_messages`）必须清掉旧会话的 `jobs_state` /
/// `task_glance` / `last_glance_sync`，且打开抽屉 `sync_drawer_sources` 时立即同步新会话 jobs。
#[test]
fn reset_session_scope_clears_jobs_state_and_drawer_sync_refreshes_jobs() {
    let session_a = "jobs-reset-session-a";
    let session_b = "jobs-reset-session-b";
    let (cmd, args) = long_sleep_args();

    let job_a = job_start_scoped(
        JobStartArgs {
            command: cmd.clone(),
            args: Some(args.clone()),
            cwd: None,
            timeout_ms: None,
            description: Some("会话A后台任务".to_string()),
        },
        None,
        Some(session_a),
    )
    .expect("会话 A 测试作业必须启动");
    let job_b = job_start_scoped(
        JobStartArgs {
            command: cmd,
            args: Some(args),
            cwd: None,
            timeout_ms: None,
            description: Some("会话B后台任务".to_string()),
        },
        None,
        Some(session_b),
    )
    .expect("会话 B 测试作业必须启动");

    let mut state = drawer_state_on(task_center::TaskCenterTab::Jobs);
    state.session_id = session_a.to_string();
    jobs_panel::sync_jobs_from_registry(&mut state);
    state.task_glance.agents_running = 2;
    state.last_glance_sync = Some(std::time::Instant::now());
    assert!(
        state
            .jobs_state
            .as_ref()
            .is_some_and(|view| view.jobs.iter().any(|job| job.id == job_a.job_id)),
        "前置条件：会话 A 快照必须包含 job_a"
    );

    // 切到会话 B 并复位作用域：旧快照与节流锚点必须全部清空。
    state.session_id = session_b.to_string();
    state.reset_session_scope();
    assert!(state.jobs_state.is_none(), "reset_session_scope 必须清空 jobs_state");
    assert_eq!(
        state.task_glance,
        TaskGlance::default(),
        "reset_session_scope 必须清空 task_glance"
    );
    assert!(
        state.last_glance_sync.is_none(),
        "reset_session_scope 必须清空 last_glance_sync"
    );

    // 立刻打开抽屉触发 sync_drawer_sources：必须马上只看到会话 B 的作业，不含会话 A。
    let control_plane = HostControlPlane::new();
    projection::sync_drawer_sources(&mut state, &control_plane);
    let view = state
        .jobs_state
        .as_ref()
        .expect("sync_drawer_sources 必须同步填充 jobs_state");
    assert!(
        view.jobs.iter().any(|job| job.id == job_b.job_id),
        "sync_drawer_sources 必须立即呈现会话 B 的作业：{view:?}"
    );
    assert!(
        !view.jobs.iter().any(|job| job.id == job_a.job_id),
        "sync_drawer_sources 后不得残留会话 A 的作业：{view:?}"
    );

    // 另一换会话入口 clear_messages 也必须清空 jobs_state / task_glance / last_glance_sync。
    state.task_glance.team_open = 1;
    state.last_glance_sync = Some(std::time::Instant::now());
    state.clear_messages();
    assert!(state.jobs_state.is_none(), "clear_messages 必须清空 jobs_state");
    assert_eq!(
        state.task_glance,
        TaskGlance::default(),
        "clear_messages 必须清空 task_glance"
    );
    assert!(
        state.last_glance_sync.is_none(),
        "clear_messages 必须清空 last_glance_sync"
    );

    let _ = job_kill_scoped(
        JobKillArgs {
            job_id: job_a.job_id,
            reason: Some("test cleanup".to_string()),
        },
        Some(session_a),
    );
    let _ = job_kill_scoped(
        JobKillArgs {
            job_id: job_b.job_id,
            reason: Some("test cleanup".to_string()),
        },
        Some(session_b),
    );
}
