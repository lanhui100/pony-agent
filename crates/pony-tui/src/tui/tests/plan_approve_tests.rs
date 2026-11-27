//! 单测：`/approve` 宿主审查入口（ADR 2026-11-27-plan-mode-dsh-alignment-and-task-count）。
//!
//! 回归面：Plan 模式会锁死所有写/执行类工具，解锁只能靠「用户批准模型通过 `ExitPlanMode`
//! 提交的方案」或「`/plan off`」。TUI 此前没有任何审查入口，用户只能看着模型反复请求批准。
//!
//! 本文件锁住 `/approve` 的四个可观测行为：
//! 1. 不在 Plan 模式 ⇒ 如实说清现状（不是错误、不是空白）；
//! 2. 在 Plan 模式但方案还没提交 ⇒ 说清「等模型提交方案」；
//! 3. 有已提交方案 ⇒ 真的批准并退出模式（写权限解锁）；
//! 4. 批准后再次执行 ⇒ 状态行不再声称「已批准」（幂等且如实）。

use super::*;

/// 隔离进程级数据目录（`Once` 保证只做一次，且早于任何 control plane 构造）。
fn isolate_process_data_dir() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let sandbox = std::env::temp_dir().join(format!(
            "pony-agent-tui-plan-approve-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or_default()
        ));
        std::fs::create_dir_all(&sandbox).expect("create isolated data dir");
        std::env::set_var("XDG_DATA_HOME", &sandbox);
        std::env::set_var("HOME", &sandbox);
        std::env::set_var("LOCALAPPDATA", &sandbox);
    });
}

fn isolated_control_plane() -> HostControlPlane {
    isolate_process_data_dir();
    HostControlPlane::new()
}

/// 每例唯一会话 id（Plan 模式注册表是进程级单例，靠会话键隔离）。
fn session(label: &str) -> String {
    format!("{label}-{}", uuid::Uuid::new_v4())
}

/// 让会话进入 Plan 模式并提交一份方案（模拟模型调用 `ExitPlanMode`）。
fn enter_plan_mode_with_plan(control_plane: &HostControlPlane, session_id: &str) {
    control_plane
        .plan_mode_set(session_id, pony_agent_core::agent::plan_mode::ModeToggle::Enter)
        .expect("enter plan mode");
    control_plane.plan_mode_commit(session_id);
    pony_agent_core::agent::plan_mode::submit_plan(session_id, "# 待批准的方案\n- 第一步");
}

#[test]
fn approve_outside_plan_mode_reports_state_instead_of_failing() {
    let control_plane = isolated_control_plane();
    let session = session("approve-outside");

    let notice = approve_pending_plan(&control_plane, &session);

    // 空会话是常态（用户随时可能敲 /approve）：必须是可读的现状说明，而不是错误或空白。
    assert!(
        notice.contains("不在 Plan 模式"),
        "/approve 在非 Plan 模式时必须说清现状，实际 = {notice:?}"
    );
    assert!(
        !notice.contains("失败"),
        "/approve 在非 Plan 模式时不得报「失败」（那不是错误），实际 = {notice:?}"
    );
}

#[test]
fn approve_without_a_submitted_plan_reports_state_instead_of_failing() {
    let control_plane = isolated_control_plane();
    let session = session("approve-no-plan");
    control_plane
        .plan_mode_set(&session, pony_agent_core::agent::plan_mode::ModeToggle::Enter)
        .expect("enter plan mode");
    control_plane.plan_mode_commit(&session);

    let notice = approve_pending_plan(&control_plane, &session);

    assert!(
        notice.contains("还没有提交待审方案"),
        "方案未提交时必须说清「等模型提交」，实际 = {notice:?}"
    );
    assert!(
        !notice.contains("失败"),
        "方案未提交不是错误，不得报「失败」，实际 = {notice:?}"
    );
}

#[test]
fn approve_exits_plan_mode_and_unlocks_writes() {
    let control_plane = isolated_control_plane();
    let session = session("approve-ok");
    enter_plan_mode_with_plan(&control_plane, &session);

    let notice = approve_pending_plan(&control_plane, &session);

    assert!(
        notice.contains("已批准"),
        "/approve 必须报告成功，实际 = {notice:?}"
    );
    let state = control_plane.plan_mode_get(&session);
    assert_eq!(
        state["active"].as_bool(),
        Some(false),
        "批准后必须退出 Plan 模式（写/执行解锁）：{state}"
    );
    assert_eq!(
        state["planApproved"].as_bool(),
        Some(true),
        "批准后方案必须标记为已批准：{state}"
    );
}

#[test]
fn approve_is_not_reported_twice_after_the_mode_is_left() {
    let control_plane = isolated_control_plane();
    let session = session("approve-twice");
    enter_plan_mode_with_plan(&control_plane, &session);
    approve_pending_plan(&control_plane, &session);

    let notice = approve_pending_plan(&control_plane, &session);

    assert!(
        notice.contains("不在 Plan 模式"),
        "批准后再次 /approve 必须如实说明已不在模式内（不得重复声称批准），实际 = {notice:?}"
    );
}
