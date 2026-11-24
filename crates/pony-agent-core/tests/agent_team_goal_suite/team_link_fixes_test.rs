//! 团队链路修复回归（2026-11-08 诊断 D1 / D3）。
//!
//! - D1：target 查找必须与登记侧同一归一化（trim + 小写 + 空格转连字符），
//!   `list_agents` 暴露的 name 写法（"Lead"/"Case Probe"）可直接作为 target 命中。
//! - D3：软删除任务不得再出现在 `team_task_list`（TUI /tasks 与模型看板共用该真值源）。
//!
//! 本文件**不注入引擎**：spawn 用空 prompt、入队用纯 seam `enqueue_message`，
//! 全程无后台驱动，观测面确定性（与 pa110 AC05 同一口径）。

use pony_agent_core::agent::orchestration::{
    drain_inbox, enqueue_message, interrupt_agent, list_agents, normalize_target,
    spawn_teammate, team_task_create, team_task_list, team_task_update, InterruptAgentArgs,
    MessageDeliveryMode, ReadInboxArgs, SendMessageArgs, SpawnTeammateArgs, TeamTaskAction,
    TeamTaskCreateArgs, TeamTaskUpdateArgs, TeammateStatus,
};

fn unique_name(tag: &str) -> String {
    format!(
        "{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )
}

#[test]
fn d1_normalize_target_matches_registration_rule() {
    assert_eq!(normalize_target("Lead"), "lead");
    assert_eq!(normalize_target("Case Probe"), "case-probe");
    assert_eq!(normalize_target("  MiXeD  Name "), "mixed--name");
}

#[test]
fn d1_send_interrupt_drain_accept_name_style_targets() {
    let name = unique_name("Case Probe");
    let spawned = spawn_teammate(SpawnTeammateArgs {
        name: name.clone(),
        role: Some("tester".to_string()),
        description: "D1 归一化回归".to_string(),
        prompt: String::new(),
        context: None,
        provider_id: None,
        model_id: None,
    })
    .expect("spawn 必须成功");
    let target = spawned.member.target.clone();
    assert_eq!(target, name.to_lowercase().replace(' ', "-"));

    // 1) name 原样写法与大写 target 写法都必须命中同一信箱（纯入队，不驱动）。
    enqueue_message(SendMessageArgs {
        target: name.clone(),
        message: "via-name".to_string(),
        mode: Some(MessageDeliveryMode::Queue),
    })
    .expect("D1：以登记 name 作为 target 必须命中");
    enqueue_message(SendMessageArgs {
        target: target.to_uppercase(),
        message: "via-uppercase".to_string(),
        mode: Some(MessageDeliveryMode::Queue),
    })
    .expect("D1：大写 target 必须命中");

    // 2) drain 用 name 写法读取：两条消息同箱、FIFO 原序。
    let inbox = drain_inbox(ReadInboxArgs {
        target: name.clone(),
    })
    .expect("D1：drain 以 name 写法必须命中");
    assert_eq!(inbox.messages.len(), 2);
    assert_eq!(inbox.messages[0].content, "via-name");
    assert_eq!(inbox.messages[1].content, "via-uppercase");
    assert_eq!(inbox.target, target, "回显 target 必须是归一化后的键");

    // 3) interrupt 用混合大小写写法必须命中。
    let interrupted = interrupt_agent(InterruptAgentArgs {
        target: name.clone(),
        reason: Some("D1 回归".to_string()),
    })
    .expect("D1：interrupt 以 name 写法必须命中");
    assert!(interrupted.success);
    assert_eq!(interrupted.new_status, TeammateStatus::Inactive);

    // 4) "Lead" 写法必须落进 lead 信箱（trace 中 verifier 的真实失败场景）。
    enqueue_message(SendMessageArgs {
        target: "Lead".to_string(),
        message: "d1-lead-marker".to_string(),
        mode: Some(MessageDeliveryMode::Queue),
    })
    .expect("D1：target=Lead 必须命中 lead 信箱");
    let lead_inbox = drain_inbox(ReadInboxArgs {
        target: "lead".to_string(),
    })
    .expect("lead 信箱必须可读");
    assert!(
        lead_inbox
            .messages
            .iter()
            .any(|msg| msg.content == "d1-lead-marker"),
        "Lead 写法投递的消息必须出现在 lead 信箱"
    );

    // 5) 花名册 target 键保持归一化（list_agents 的 target 字段是查找真值）。
    let roster = list_agents().expect("list_agents 必须成功");
    assert!(roster.agents.iter().any(|member| member.target == target));
}

#[test]
fn d3_deleted_tasks_are_hidden_from_task_list() {
    let subject = unique_name("d3-deleted-task");
    let created = team_task_create(TeamTaskCreateArgs {
        subject: subject.clone(),
        description: "D3 软删除过滤回归".to_string(),
        write_scopes: None,
        blocked_by: None,
    })
    .expect("建任务必须成功");
    assert_eq!(created.revision, 1);

    let listed = team_task_list().expect("看板必须可读");
    assert!(
        listed.tasks.iter().any(|task| task.task_id == created.task_id),
        "删除前任务必须在看板可见"
    );

    let deleted = team_task_update(TeamTaskUpdateArgs {
        task_id: created.task_id.clone(),
        expected_revision: 1,
        action: TeamTaskAction::Delete,
        owner: None,
        subject: None,
        description: None,
        write_scopes: None,
        blocked_by: None,
    })
    .expect("软删除必须成功");
    assert_eq!(deleted.status, "deleted");
    assert_eq!(deleted.revision, 2);

    let after = team_task_list().expect("删除后看板必须可读");
    assert!(
        !after.tasks.iter().any(|task| task.task_id == created.task_id),
        "D3：软删除任务不得再出现在 team_task_list（否则 /tasks 会把它降级显示为 Pending）"
    );
}