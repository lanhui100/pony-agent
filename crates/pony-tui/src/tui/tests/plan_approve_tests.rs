//! 单测：`/approve` 宿主盖章入口（ADR 0092 §Decision 3）。
//!
//! 回归面：`Draft` 计划会锁死所有写/执行类工具，解锁只能靠「宿主盖章
//! `plan_execution_approve`」或「模型 `abort`」。TUI 此前没有任何盖章入口，
//! 于是用户只看到模型反复请求批准。这里锁住 `/approve` 的三个可观测行为：
//! 没有 Draft 时说清现状、有 Draft 时真的把计划推进到 `executing`、批准后不再残留 Draft。

use super::*;
use pony_agent_core::agent::plan_state::PlanStore;
use serde_json::json;

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

/// 需要同时驱动「模型侧 `abort`」时留住 `PlanStore` 句柄（`PlanStore` 克隆共享同一状态）。
fn isolated_control_plane_with_store() -> (HostControlPlane, PlanStore) {
    isolate_process_data_dir();
    let plan_store = PlanStore::new();
    let control_plane = HostControlPlaneBuilder::new()
        .plan_store(plan_store.clone())
        .build();
    (control_plane, plan_store)
}

fn create_draft(control_plane: &HostControlPlane, session_id: &str) -> String {
    let created = control_plane
        .plan_create(
            session_id,
            json!({
                "kind": "task",
                "summary": "待批准的草稿计划",
                "steps": [{ "name": "第一步", "summary": "" }]
            }),
        )
        .expect("create draft plan");
    assert_eq!(created["lifecycle"], "draft", "fixture 必须是 Draft：{created}");
    created["planId"]
        .as_str()
        .expect("planId")
        .to_string()
}

#[test]
fn approve_without_draft_reports_current_state_instead_of_failing() {
    let control_plane = isolated_control_plane();
    let session = "session-approve-no-draft";

    let notice = approve_earliest_draft_plan(&control_plane, session);

    // 空会话是常态（用户随时可能敲 /approve）：必须是可读的现状说明，而不是错误或空白。
    assert!(
        notice.contains("没有待批准的草稿计划"),
        "/approve 在无 Draft 时必须说清现状，实际 = {notice:?}"
    );
    assert!(
        !notice.contains("失败"),
        "/approve 在无 Draft 时不得报「失败」（那不是错误），实际 = {notice:?}"
    );
}

#[test]
fn approve_moves_earliest_draft_plan_to_executing() {
    let control_plane = isolated_control_plane();
    let session = "session-approve-draft";
    let plan_id = create_draft(&control_plane, session);

    let notice = approve_earliest_draft_plan(&control_plane, session);

    assert!(
        notice.contains(&plan_id),
        "/approve 的回执必须点名被批准的计划，实际 = {notice:?}"
    );
    assert!(
        notice.contains("已批准"),
        "/approve 必须报告成功，实际 = {notice:?}"
    );

    let plans = control_plane.plan_list(session);
    let plan = plans
        .as_array()
        .and_then(|plans| plans.iter().find(|plan| plan["planId"] == plan_id.as_str()))
        .expect("approved plan must stay listed");
    // 盖章的唯一语义就是 Draft -> Executing；写工具解锁依赖 `has_active_draft_plan() == false`。
    assert_eq!(
        plan["lifecycle"], "executing",
        "宿主盖章后计划必须进入 executing：{plan}"
    );
    assert_eq!(
        plan["revision"], 2,
        "start_execution 是一次 CAS 迁移，revision 必须从 1 递增到 2：{plan}"
    );
}

#[test]
fn approve_reports_failure_when_no_draft_remains_after_abort() {
    let (control_plane, plan_store) = isolated_control_plane_with_store();
    let session = "session-approve-after-abort";
    let plan_id = create_draft(&control_plane, session);

    // 模型侧自助退出（ADR 0092 §Decision 3）：撤销后 `/approve` 不得再假装有事可做。
    plan_store
        .abort(session, &plan_id, 1)
        .expect("abort draft");

    let notice = approve_earliest_draft_plan(&control_plane, session);
    assert!(
        notice.contains("没有待批准的草稿计划"),
        "Draft 已被撤销后 /approve 必须如实汇报「无事可批」，实际 = {notice:?}"
    );
}
