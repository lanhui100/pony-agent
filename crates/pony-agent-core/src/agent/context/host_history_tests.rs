//! B235 回归：宿主续轮历史行（`host` 角色）投影到 provider 时**仍是模型可见指令**，
//! 且不再伪造用户发言；旧日志的 `user` / `assistant` 投影逐字不变。

use super::to_provider_history_message;
use crate::agent::provider::ProviderRole;
use crate::agent::session::host_resume::{
    compose_host_continuation, HOST_HISTORY_ROLE,
};
use crate::agent::session::TurnHistoryMessage;

const INSTRUCTION: &str = "把审批续轮的落库角色改成 host";

fn message(role: &str, content: String) -> TurnHistoryMessage {
    TurnHistoryMessage {
        role: role.to_string(),
        content,
        ..TurnHistoryMessage::default()
    }
}

#[test]
fn host_history_row_projects_to_developer_with_instruction_visible() {
    let composed = compose_host_continuation(INSTRUCTION, "（已批准该工具调用）");
    let projected = to_provider_history_message(&message(HOST_HISTORY_ROLE, composed))
        .expect("host 角色历史行必须投影出消息（否则模型看不到续轮指令）");

    assert!(
        matches!(projected.role, ProviderRole::Developer),
        "B235：host 行必须投影为 developer 消息，不得伪装成 user"
    );
    assert!(
        projected.content.contains(INSTRUCTION),
        "B235：模型面必须仍能看到被挂起回合的原指令，实际 = {:?}",
        projected.content
    );
    assert!(
        projected.content.contains("（已批准该工具调用）"),
        "B235：模型面必须仍能看到宿主续轮文案，实际 = {:?}",
        projected.content
    );
}

#[test]
fn legacy_user_history_row_projects_as_user_unchanged() {
    // 旧日志（无新字段）：user 行投影逐字不变（回归基线）。
    let projected = to_provider_history_message(&message("user", "你好".to_string()))
        .expect("user 行必须投影");
    assert!(
        matches!(projected.role, ProviderRole::User),
        "旧日志 user 行必须仍是 user 消息"
    );
    assert_eq!(projected.content, "你好");
}

#[test]
fn unknown_history_role_is_skipped_as_before() {
    assert!(
        to_provider_history_message(&message("observer", "x".to_string())).is_none(),
        "未知角色不得新增投影（additive 边界）"
    );
}
