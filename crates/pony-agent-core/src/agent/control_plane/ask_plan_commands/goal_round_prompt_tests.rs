//! 自锁回归（ADR 0092）：宿主 goal 续轮文案不得让仍待作答的 Ask 过期。
//!
//! 为什么单列文件：被测实现（`note_user_message_entry`）住在 `ask_plan_commands.rs`，
//! 而 `control_plane/mod.rs` 已触发代码规模门禁棘轮（只许降不许升），回归不得再往里堆行数。
//!
//! 现场证据：某会话 17 条 Ask 全部 `superseded`，每条都在下一条 goal 轮开始前 4-7ms 被
//! 作废；面板随即上屏「该提问已失效，已同步最新待处理请求」，模型只能重复提问。

use super::super::tests::control_plane_test_lock;
use super::super::{HostControlPlane, HostControlPlaneBuilder};
use crate::agent::dispatcher::{DispatchContext, GovernedDispatcher};
use crate::agent::runtime::TurnInput;
use crate::agent::tool_runtime::{FakeClock, InvocationOrigin, ToolDispatchRequest};
use crate::agent::tools::{
    ToolDescriptor, ToolDescriptorSource, ToolDisplayMetadata, ToolExecutionPolicy, ToolExposure,
    ToolHandlerProvenance, ToolIdentity, ToolKind, ToolPermissionDeclaration, ToolRegistrySnapshot,
};
use serde_json::json;
use std::sync::Arc;

const SESSION: &str = "session-goal-round";

/// host-mediated 的 Ask 描述符（与生产 `ask_user` 同形：Interactive + host_mediated）。
fn host_mediated_ask_descriptor() -> ToolDescriptor {
    let mut declaration = ToolPermissionDeclaration::default();
    declaration.host_mediated = true;
    ToolDescriptor {
        identity: ToolIdentity {
            descriptor_id: "builtin:ask".to_string(),
            model_name: "Ask".to_string(),
            canonical_name: "ask".to_string(),
            primitive_name: "echo_input".to_string(),
            source: ToolDescriptorSource::Builtin,
        },
        aliases: vec!["ask".to_string(), "builtin:ask".to_string()],
        description: String::new(),
        input_schema: json!({
            "type": "object",
            "properties": { "text": { "type": "string" } },
            "required": ["text"],
            "additionalProperties": false,
        }),
        kind: ToolKind::Interactive,
        exposure: ToolExposure::ModelVisible,
        permission_declaration: declaration,
        execution_policy: ToolExecutionPolicy::default(),
        display_metadata: ToolDisplayMetadata::default(),
        handler_provenance: ToolHandlerProvenance {
            handler_kind: "test".to_string(),
            source_id: "builtin-tools".to_string(),
        },
        source_revision: "test-v1".to_string(),
        composed_descriptor_ids: Vec::new(),
    }
}

/// 构造一个「会话里已挂起一条待作答 Ask」的真实控制面。
fn control_plane_with_pending_ask() -> HostControlPlane {
    let registry = Arc::new(
        ToolRegistrySnapshot::from_descriptors(
            "test-ask-snapshot",
            vec![host_mediated_ask_descriptor()],
        )
        .expect("ask registry must build"),
    );
    let dispatcher = Arc::new(GovernedDispatcher::new(registry, Arc::new(FakeClock::new(1_000))));
    let control_plane = HostControlPlaneBuilder::new()
        .ask_dispatcher(Arc::clone(&dispatcher))
        .build();

    let context = DispatchContext {
        session_id: Some(SESSION.to_string()),
        run_id: Some("run-goal-round".to_string()),
        turn_id: Some("turn-1".to_string()),
        // 宿主在场：否则 host-mediated 的 ask 不会产出 control outcome。
        host_control_available: true,
        ..Default::default()
    };
    dispatcher
        .dispatch_governed(
            ToolDispatchRequest {
                origin: InvocationOrigin::Model,
                descriptor_id: "builtin:ask".to_string(),
                call_id: "call-goal-round".to_string(),
                arguments: json!({ "text": "要不要继续执行？" }),
            },
            &context,
        )
        .control_outcome
        .expect("ask control outcome");
    control_plane
}

fn pending_ask_count(control_plane: &HostControlPlane) -> usize {
    control_plane
        .list_pending_asks(Some(SESSION))
        .as_array()
        .map(Vec::len)
        .unwrap_or(0)
}

fn turn_input(message: String, display: Option<String>) -> TurnInput {
    TurnInput {
        message,
        display_message: display,
        provider_id: None,
        model_id: None,
        reasoning_effort: None,
        workspace_mode: None,
        tool_authorization_mode: None,
        session_id: Some(SESSION.to_string()),
        node_id: None,
        history: Vec::new(),
        images: Vec::new(),
        workspace_id: None,
        plan_execution_approval: None,
    }
}

#[test]
fn goal_round_continuation_does_not_supersede_pending_ask() {
    let _cp_lock = control_plane_test_lock();
    let control_plane = control_plane_with_pending_ask();
    assert_eq!(
        pending_ask_count(&control_plane),
        1,
        "fixture ask must be pending"
    );

    // 宿主续轮：真正下发的是 `<goal_round …>`，display 形态是 `[goal round N] …`；两者都不是人。
    control_plane.note_user_message_entry(&turn_input(
        crate::agent::goal::build_goal_round_message("修好自锁循环", 2, 10),
        Some("[goal round 2] 推进目标：修好自锁循环".to_string()),
    ));
    assert_eq!(
        pending_ask_count(&control_plane),
        1,
        "goal 续轮文案不得让仍待作答的 Ask 过期"
    );

    control_plane.note_user_message_entry(&turn_input("换成另一个方案吧".to_string(), None));
    assert_eq!(
        pending_ask_count(&control_plane),
        0,
        "真人新消息必须让旧提问过期（ADR 0074 §3）"
    );
}
