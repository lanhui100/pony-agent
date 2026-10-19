use pony_agent_core::agent::tools::{
    drain_inbox, list_agents, send_message, spawn_teammate,
    MessageDeliveryMode, ReadInboxArgs, SendMessageArgs, SpawnTeammateArgs, TeammateStatus,
};
use pony_agent_core::agent::orchestration::drive_teammate_turn;

#[test]
fn test_teammate_bi_directional_communication_and_reply() {
    // 1. 动态生成两名队员：alice 与 bob
    let alice = spawn_teammate(SpawnTeammateArgs {
        name: "alice".to_string(),
        role: Some("frontend-specialist".to_string()),
        description: "Focus on UI/UX and styling".to_string(),
        prompt: "You are alice".to_string(),
        context: None,
    }).expect("spawn alice should succeed");
    assert_eq!(alice.member.target, "alice");

    let bob = spawn_teammate(SpawnTeammateArgs {
        name: "bob".to_string(),
        role: Some("backend-specialist".to_string()),
        description: "Focus on database and APIs".to_string(),
        prompt: "You are bob".to_string(),
        context: None,
    }).expect("spawn bob should succeed");
    assert_eq!(bob.member.target, "bob");

    // 2. alice 向 bob 发送技术咨询
    let send_res = send_message(SendMessageArgs {
        target: "bob".to_string(),
        message: "Hello Bob, please provide the API schema for tasks.".to_string(),
        mode: Some(MessageDeliveryMode::Steer),
    }).expect("send_message from alice to bob should succeed");

    assert!(send_res.delivered);
    assert_eq!(send_res.pending_inbox_len, 1);

    // 验证 bob 的信箱已正确接收
    let bob_inbox = drain_inbox(ReadInboxArgs { target: "bob".to_string() }).expect("drain bob inbox");
    assert_eq!(bob_inbox.messages.len(), 1);
    assert!(bob_inbox.messages[0].content.contains("API schema"));
    assert!(bob_inbox.messages[0].is_steer);

    // 3. 再次向 bob 投递消息并触发 drive_teammate_turn
    send_message(SendMessageArgs {
        target: "bob".to_string(),
        message: "Bob, please complete review task #102.".to_string(),
        mode: Some(MessageDeliveryMode::Queue),
    }).expect("send should succeed");

    let turn_output = drive_teammate_turn("bob").expect("drive turn should succeed");
    assert!(turn_output.is_some());

    // 4. 验证 bob 处理完成后向 lead 自动发送回执信件
    let lead_inbox = drain_inbox(ReadInboxArgs { target: "lead".to_string() }).expect("drain lead inbox");
    assert!(!lead_inbox.messages.is_empty(), "Bob should automatically report to lead");
    assert!(lead_inbox.messages[0].content.contains("Teammate 'bob' finished turn"));

    // 5. 验证名册中 Alice 和 Bob 依然健康活跃
    let roster = list_agents().expect("list agents");
    assert!(roster.agents.iter().any(|a| a.target == "alice" && a.status == TeammateStatus::Active));
    assert!(roster.agents.iter().any(|a| a.target == "bob" && a.status == TeammateStatus::Active));
}
