use pony_agent_core::agent::control_plane::HostControlPlaneBuilder;
use pony_agent_core::agent::plan_state::PlanStore;
use serde_json::json;

#[test]
fn test_plan_store_is_unified_between_control_plane_and_executor() {
    let shared_store = PlanStore::new();
    let cp = HostControlPlaneBuilder::new()
        .plan_store(shared_store.clone())
        .build();

    let session_id = "test-session-plan-sync";
    let payload = json!({
        "kind": "implementation",
        "summary": "测试跨控制面同步",
        "steps": [
            { "name": "step-1", "summary": "第1步" }
        ]
    });

    let created = cp.plan_create(session_id, payload).expect("create plan");
    let plan_id = created["planId"].as_str().expect("plan_id");

    // Check directly in shared_store
    let plan = shared_store.plan(session_id, plan_id).expect("must find plan");
    assert_eq!(plan.plan_id, plan_id, "HostControlPlane and shared store must see the same active plan");
}
