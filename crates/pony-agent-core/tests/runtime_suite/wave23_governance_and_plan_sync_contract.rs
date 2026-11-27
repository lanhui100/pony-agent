//! W23 契约（2026-11-27 重构）：**Plan 模式状态是进程级单例**，控制面与工具门禁读同一份。
//!
//! 旧用例验证「`HostControlPlane` 与 `governed_executor` 共享同一个 `PlanStore` 实例」。
//! 步骤机下线后，Plan 状态收敛为进程级 `plan_mode` 注册表（ADR
//! `2026-11-27-plan-mode-dsh-alignment-and-task-count`），共享性由「同进程同键」保证：
//! 控制面写进去的模式开关，工具门禁必须立刻看见——这正是本用例要锁住的事实。

use pony_agent_core::agent::control_plane::HostControlPlaneBuilder;
use pony_agent_core::agent::dispatcher::{DispatchContext, PermissionVerdict, ToolPolicyEvaluator};
use pony_agent_core::agent::governed_executor::GovernedPolicyEvaluator;
use pony_agent_core::agent::runtime::AgentRuntimeBuilder;
use pony_agent_core::agent::session::{FileSessionBackend, SessionStore};
use pony_agent_core::agent::tool_runtime::InvocationOrigin;
use pony_agent_core::agent::tools::{ToolKind, ToolRegistrySnapshot};
use serde_json::json;

#[test]
fn test_plan_mode_state_is_shared_between_control_plane_and_executor() {
    // Arrange
    let dir = std::env::temp_dir().join(format!("pony-wave23-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("create dir");
    let runtime = AgentRuntimeBuilder::new()
        .session_store(SessionStore::with_backend(Box::new(
            FileSessionBackend::new(dir.join("sessions.db")),
        )))
        .build();
    let cp = HostControlPlaneBuilder::new().runtime(runtime).build();

    let session_id = format!("test-session-plan-mode-sync-{}", uuid::Uuid::new_v4());
    let write_desc = ToolRegistrySnapshot::builtin()
        .expect("builtin registry must validate")
        .descriptors
        .iter()
        .find(|descriptor| descriptor.identity.descriptor_id == "builtin:workspace_write_file")
        .expect("内建写工具必须在 registry 中")
        .clone();
    let evaluator = GovernedPolicyEvaluator::new(Vec::new());
    let context = DispatchContext {
        session_id: Some(session_id.clone()),
        ..DispatchContext::default()
    };

    // Act 1 — 控制面进入 Plan 模式
    cp.plan_mode_set(&session_id, true).expect("enter plan mode");
    cp.plan_mode_commit(&session_id);

    // Assert 1 — 工具门禁立刻看到同一份状态：写工具被拒
    let blocked =
        evaluator.evaluate_with_context(&write_desc, &InvocationOrigin::Model, &json!({}), &context);
    assert_eq!(
        blocked.verdict,
        PermissionVerdict::Deny,
        "控制面进入 Plan 模式后，工具门禁必须读同一份状态并拒绝写工具"
    );
    assert_eq!(blocked.decision_source, "plan_mode_gate");

    // Act 2 — 控制面退出 Plan 模式
    cp.plan_mode_set(&session_id, false).expect("leave plan mode");

    // Assert 2 — 写工具立刻解锁
    let allowed =
        evaluator.evaluate_with_context(&write_desc, &InvocationOrigin::Model, &json!({}), &context);
    assert_eq!(
        allowed.verdict,
        PermissionVerdict::Allow,
        "退出 Plan 模式后写工具必须立刻解锁（同一份进程级状态）"
    );
    assert_eq!(write_desc.kind, ToolKind::Write, "对照：本用例必须选一个写类工具");

    let _ = std::fs::remove_dir_all(&dir);
}
