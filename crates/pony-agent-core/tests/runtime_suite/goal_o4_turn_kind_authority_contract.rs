//! O4（P1）验收：goal 元工具的授权真值源是**结构化类别** `TurnKind`，不是文案嗅探。
//!
//! 契约真值源：`docs/analysis/goal-ask-blocking-audit-2026-11-26.md` §4 O4
//! （「把 `DispatchContext.is_goal_round: bool` 升级为
//! `TurnKind { DirectHuman, GoalRound, AskResume, ApprovalResume, SubAgent }`，
//! 由宿主在构造 `TurnInput` 时写入（不靠文案嗅探）。`GhostCreate/Pause/Edit/Resume`
//! 仅 `DirectHuman` 通过。」）
//!
//! 要防的回归（审核报告 P1-1）：旧实现用 `is_goal_round_prompt(message)` 文案嗅探推导
//! `is_direct_human = !is_goal_round`，而 Ask/审批**恢复轮**的输入是用户原文而非
//! `<goal_round ...>` 模板 ⇒ 被判成「人类直发轮」⇒ 模型在恢复轮里拿到
//! `create_goal` / `pause` / `edit` / `resume` 的权威。
//!
//! 铁律：每条用例真实断言；无 `#[ignore]` / skip；不修改 `src/`。

use pony_agent_core::agent::dispatcher::{DispatchContext, TurnKind};
use pony_agent_core::agent::goal::test_support::TempGoalStore;
use pony_agent_core::agent::goal::{create_goal, CreateGoalArgs};
use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolRouter};
use serde_json::json;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_workspace(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("pony-o4-{tag}-{nanos}"));
    std::fs::create_dir_all(&dir).expect("temp workspace");
    dir
}

fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        name: name.to_string(),
        arguments,
        call_id: Some("o4-test".to_string()),
        plan: None,
    }
}

/// `update_goal` 的 CAS 需要 revision，故必须现读目标快照。
fn current_goal(session: &str) -> pony_agent_core::agent::goal::GoalData {
    pony_agent_core::agent::goal::active_goal_for_session(session)
        .unwrap_or_else(|| panic!("Arrange：session={session} 应存在 active+Armed 目标"))
}

fn ctx_with_kind(workspace: &PathBuf, session: &str, kind: TurnKind) -> ToolExecutionContext {
    ToolExecutionContext {
        workspace_root: Some(workspace.clone()),
        session_id: Some(session.to_string()),
        dispatch_context: Some(DispatchContext {
            session_id: Some(session.to_string()),
            turn_kind: kind,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// 冻结契约 1：`TurnKind` 变体集合恰好五项（新增变体必须同步登记，防止授权真值表悄悄长分支）。
#[test]
fn o4_turn_kind_variant_set_is_frozen() {
    assert_eq!(
        TurnKind::all_variants().len(),
        5,
        "O4：TurnKind 冻结为 DirectHuman/GoalRound/AskResume/ApprovalResume/SubAgent 五项"
    );
    let codes: Vec<&str> = TurnKind::all_variants().iter().map(|k| k.code()).collect();
    assert_eq!(
        codes,
        vec![
            "direct_human",
            "goal_round",
            "ask_resume",
            "approval_resume",
            "sub_agent"
        ],
        "O4：类别字面量是稳定线缆面，改名即破契约"
    );
}

/// 冻结契约 2：只有 `DirectHuman` 授予 ghost 权威；`GoalRound` 只额外放行 complete/blocked。
#[test]
fn o4_authority_matrix_matches_contract() {
    assert!(TurnKind::DirectHuman.is_direct_human());
    assert!(!TurnKind::DirectHuman.is_goal_round());

    assert!(!TurnKind::GoalRound.is_direct_human());
    assert!(TurnKind::GoalRound.is_goal_round());

    for resume in [TurnKind::AskResume, TurnKind::ApprovalResume, TurnKind::SubAgent] {
        assert!(
            !resume.is_direct_human(),
            "O4：`{}` 不是人类直发轮，不得授予 ghost 权威",
            resume.code()
        );
        assert!(
            !resume.is_goal_round(),
            "O4：`{}` 不是 goal 自动续轮，不得放行 complete/blocked",
            resume.code()
        );
    }
}

/// 回归钉子（P1-1 本体）：Ask 恢复轮里 `create_goal` 必须被拒。
///
/// 旧实现在这里放行——恢复轮输入是用户原文，文案嗅探判成 DirectHuman。
#[test]
fn o4_ask_resume_turn_cannot_create_goal() {
    let _env_lock = crate::env_lock::lock();
    let _store = TempGoalStore::install("o4-ask-resume");
    let workspace = temp_workspace("ask-resume");
    let router = ToolRouter::with_workspace_root(workspace.clone());
    let session = "o4-ask-resume-session";

    let result = router.execute_with_context(
        &call(
            "create_goal",
            json!({ "session_id": session, "objective": "恢复轮偷偷建目标" }),
        ),
        &ctx_with_kind(&workspace, session, TurnKind::AskResume),
    );

    assert_eq!(
        result.status, "error",
        "O4：Ask 恢复轮不得创建目标，实际输出 {}",
        result.output
    );
    assert!(
        result.output.contains("goal_action_unauthorized"),
        "O4：必须回稳定错误码 goal_action_unauthorized，实际 {}",
        result.output
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

/// 回归钉子：审批恢复轮同样不得 `create_goal`。
#[test]
fn o4_approval_resume_turn_cannot_create_goal() {
    let _env_lock = crate::env_lock::lock();
    let _store = TempGoalStore::install("o4-approval-resume");
    let workspace = temp_workspace("approval-resume");
    let router = ToolRouter::with_workspace_root(workspace.clone());
    let session = "o4-approval-resume-session";

    let result = router.execute_with_context(
        &call(
            "create_goal",
            json!({ "session_id": session, "objective": "审批恢复轮偷偷建目标" }),
        ),
        &ctx_with_kind(&workspace, session, TurnKind::ApprovalResume),
    );

    assert_eq!(result.status, "error", "O4：审批恢复轮不得创建目标");
    assert!(result.output.contains("goal_action_unauthorized"));

    let _ = std::fs::remove_dir_all(&workspace);
}

/// 反向钉子：人类直发轮仍然可以 `create_goal`（不能把门关死）。
#[test]
fn o4_direct_human_turn_can_create_goal() {
    let _env_lock = crate::env_lock::lock();
    let _store = TempGoalStore::install("o4-direct-human");
    let workspace = temp_workspace("direct-human");
    let router = ToolRouter::with_workspace_root(workspace.clone());
    let session = "o4-direct-human-session";

    let result = router.execute_with_context(
        &call(
            "create_goal",
            json!({ "session_id": session, "objective": "人类直发轮建目标" }),
        ),
        &ctx_with_kind(&workspace, session, TurnKind::DirectHuman),
    );

    assert_eq!(
        result.status, "ok",
        "O4：DirectHuman 必须保留 create_goal 权威，实际 {}",
        result.output
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

/// 回归钉子：恢复轮不得 `resume` / `pause` / `edit` 目标（ghost 动作全封闭）。
#[test]
fn o4_resume_turn_cannot_pause_or_resume_goal() {
    let _env_lock = crate::env_lock::lock();
    let _store = TempGoalStore::install("o4-resume-ghost");
    let workspace = temp_workspace("resume-ghost");
    let router = ToolRouter::with_workspace_root(workspace.clone());
    let session = "o4-resume-ghost-session";

    // 先由人类直发轮建目标（否则 pause/resume 会因「无目标」而非「无权威」失败）。
    let created = create_goal(CreateGoalArgs {
        session_id: Some(session.to_string()),
        objective: "被恢复轮觊觎的目标".to_string(),
        max_goal_rounds: None,
    })
    .expect("create goal");
    let goal_id = created.goal.id;

    for action in ["pause", "resume", "edit"] {
        let revision = current_goal(session).revision;
        let mut args = json!({
            "session_id": session,
            "goal_id": goal_id,
            "revision": revision,
            "action": action,
        });
        if action == "edit" {
            args["objective"] = json!("被篡改的目标描述");
        }
        let result = router.execute_with_context(
            &call("update_goal", args),
            &ctx_with_kind(&workspace, session, TurnKind::AskResume),
        );
        assert_eq!(
            result.status, "error",
            "O4：Ask 恢复轮不得执行 `{action}`，实际 {}",
            result.output
        );
        assert!(
            result.output.contains("goal_action_unauthorized"),
            "O4：`{action}` 必须回 goal_action_unauthorized，实际 {}",
            result.output
        );
    }

    let _ = std::fs::remove_dir_all(&workspace);
}

/// 反向钉子：goal 自动续轮允许 `complete`（推进既定目标是续轮的正当动作）。
#[test]
fn o4_goal_round_turn_can_complete_goal() {
    let _env_lock = crate::env_lock::lock();
    let _store = TempGoalStore::install("o4-goal-round-complete");
    let workspace = temp_workspace("goal-round-complete");
    let router = ToolRouter::with_workspace_root(workspace.clone());
    let session = "o4-goal-round-complete-session";

    let created = create_goal(CreateGoalArgs {
        session_id: Some(session.to_string()),
        objective: "续轮内收口的目标".to_string(),
        max_goal_rounds: None,
    })
    .expect("create goal");

    let revision = current_goal(session).revision;
    let result = router.execute_with_context(
        &call(
            "update_goal",
            json!({
                "session_id": session,
                "goal_id": created.goal.id,
                "revision": revision,
                "action": "complete",
            }),
        ),
        &ctx_with_kind(&workspace, session, TurnKind::GoalRound),
    );

    assert_eq!(
        result.status, "ok",
        "O4：GoalRound 必须允许 complete，实际 {}",
        result.output
    );

    let _ = std::fs::remove_dir_all(&workspace);
}

/// 回归钉子：`GoalRound` **不**授予 ghost 权威（续轮里不能自己 pause/edit）。
#[test]
fn o4_goal_round_turn_cannot_pause_goal() {
    let _env_lock = crate::env_lock::lock();
    let _store = TempGoalStore::install("o4-goal-round-pause");
    let workspace = temp_workspace("goal-round-pause");
    let router = ToolRouter::with_workspace_root(workspace.clone());
    let session = "o4-goal-round-pause-session";

    let created = create_goal(CreateGoalArgs {
        session_id: Some(session.to_string()),
        objective: "续轮内不许自停的目标".to_string(),
        max_goal_rounds: None,
    })
    .expect("create goal");

    let revision = current_goal(session).revision;
    let result = router.execute_with_context(
        &call(
            "update_goal",
            json!({
                "session_id": session,
                "goal_id": created.goal.id,
                "revision": revision,
                "action": "pause",
            }),
        ),
        &ctx_with_kind(&workspace, session, TurnKind::GoalRound),
    );

    assert_eq!(
        result.status, "error",
        "O4：GoalRound 不得 pause（ghost 权威只属于 DirectHuman），实际 {}",
        result.output
    );
    assert!(result.output.contains("goal_action_unauthorized"));

    let _ = std::fs::remove_dir_all(&workspace);
}
