//! 中断轮次清理（C9 片段闭合）的回归用例（从 `runtime/tests.rs` 外移，避开单文件规模棘轮）。
//!
//! 断言口径：被取消/中断的轮次必须留下可序列化的闭合片段——承载正文保留、工具结果闭合、
//! 且不留下未闭合的 tool_call（否则下一轮请求会被上游拒）。

use super::*;

#[test]
fn persist_cancelled_turn_outcome_writes_closed_structure_to_history_and_transcript() {
    use crate::agent::session::interrupted_turn::{
        pending_tool_result_note, PendingToolCall, INTERRUPTED_PENDING_NOTE_PREFIX,
        INTERRUPTED_RESULT_SECTION_HEADER,
    };

    crate::agent::runtime_helper::TestRuntimeGuard::leak();
    let runtime = build_runtime_for_test(test_provider_selection("http://127.0.0.1:1".to_string()));
    let session_id = "c9-close-out-session";
    let activity = |name: &str, result: &str| TurnToolActivity {
        id: format!("activity-{name}"),
        name: name.to_string(),
        canonical_tool_name: None,
        display_name_zh: None,
        status: "done".to_string(),
        description: format!("{name} 已完成"),
        arguments_text: None,
        result_text: Some(result.to_string()),
        duration_seconds: None,
        parent_activity_id: None,
        artifacts: None,
        error: None,
        capability_invocation: None,
    };

    let _ = runtime.persist_cancelled_turn_outcome(
        Some(session_id),
        Some("turn-c9"),
        "第一轮：重启 staging 服务并确认端口释放",
        None,
        Vec::new(),
        None,
        "我先看了 8080 端口占用。",
        vec![
            activity("Read", "C9_RESULT_READ 配置里 port=8080"),
            activity("Bash", "C9_RESULT_BASH systemctl restart staging 已执行"),
        ],
        vec![PendingToolCall {
            name: "Write".to_string(),
        }],
        None,
    );

    let snapshot = runtime.load_session_snapshot(Some(session_id));
    let assistant = snapshot
        .history
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .expect("C9：取消轮必须落一条 assistant 承载行");

    // 1) 承载正文：既有检查点头 + 已交付正文 + 结果段 + 未开始事实段
    assert!(
        assistant.content.starts_with("[中断轮次检查点]"),
        "C9：必须复用既有检查点头，实际 = {:?}",
        assistant.content
    );
    assert!(
        assistant.content.contains("我先看了 8080 端口占用。"),
        "C9：已交付正文必须保留，实际 = {:?}",
        assistant.content
    );
    assert!(
        assistant.content.contains(INTERRUPTED_RESULT_SECTION_HEADER),
        "C9：必须含结果段标题，实际 = {:?}",
        assistant.content
    );
    assert!(
        assistant.content.contains("C9_RESULT_READ 配置里 port=8080")
            && assistant.content
                .contains("C9_RESULT_BASH systemctl restart staging 已执行"),
        "C9：每条已闭合结果都必须进承载正文，实际 = {:?}",
        assistant.content
    );
    assert!(
        assistant.content.contains(INTERRUPTED_PENDING_NOTE_PREFIX)
            && assistant.content.contains("Write"),
        "C9：未开始调用必须带事实注记并点名工具，实际 = {:?}",
        assistant.content
    );

    // 2) native transcript：严格成对闭合（补齐优先于截断），且携带真实结果与悬空注记
    let transcript = &snapshot.provider_native_transcript;
    assert!(
        !transcript.is_empty(),
        "C9：取消轮必须写入 native 闭合片段，实际为空"
    );
    assert!(
        !crate::agent::tool_pruner::needs_tool_roundtrip_repair(transcript),
        "C9：闭合片段不得留下未闭合的 tool_call，实际 = {transcript:?}"
    );
    let serialized = serde_json::to_string(transcript).expect("C9：闭合片段必须可序列化");
    assert!(
        serialized.contains("C9_RESULT_READ 配置里 port=8080")
            && serialized.contains("C9_RESULT_BASH systemctl restart staging 已执行"),
        "C9：闭合片段必须携带真实工具结果，实际 = {serialized}"
    );
    assert!(
        serialized.contains(&pending_tool_result_note("Write")),
        "C9：悬空调用必须以结果未知的 error 结果闭合，实际 = {serialized}"
    );
}
