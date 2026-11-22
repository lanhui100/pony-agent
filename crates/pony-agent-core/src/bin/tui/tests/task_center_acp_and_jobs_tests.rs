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
        joined.contains("ACP 委派 1 · 运行中"),
        "Agents 页签必须有 ACP 委派分区：{joined}"
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
