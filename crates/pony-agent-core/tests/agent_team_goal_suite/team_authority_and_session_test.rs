//! Agent Team 授权与队友会话登记回归测试（对标审计 P0-2/P0-3/P0-5）。
//!
//! 说明：团队状态是进程级全局单例，本文件用唯一名字隔离，并以 `--test-threads=1` 运行。

use pony_agent_core::agent::orchestration::{
    interrupt_agent_as, spawn_teammate, spawn_teammate_in_workspace_with_provider_as,
    teammate_session_id, teammate_target_for_session, wait_agent_as, InterruptAgentArgs,
    SpawnTeammateArgs, TeamCaller, WaitAgentArgs,
};
use std::time::Instant;

fn spawn_args(name: &str) -> SpawnTeammateArgs {
    SpawnTeammateArgs {
        name: name.to_string(),
        role: Some("tester".to_string()),
        description: "audit regression".to_string(),
        prompt: "noop".to_string(),
        context: Some("fresh".to_string()),
        provider_id: None,
        model_id: None,
    }
}

fn unique(prefix: &str) -> String {
    format!(
        "{prefix}-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    )
}

/// P0-2：派生即登记固定会话 id，且能反查回成员（队友 turn 内的身份判定依赖它）。
#[test]
fn spawn_registers_stable_session_identity() {
    let name = unique("audit-session");
    let result = spawn_teammate(spawn_args(&name)).expect("spawn must register the teammate");
    assert_eq!(result.member.target, name);

    let session = teammate_session_id(&name).expect("session id must be registered at spawn");
    assert!(
        session.starts_with("team-"),
        "teammate session id must be the stable team- prefix, got {session}"
    );
    assert_eq!(
        teammate_target_for_session(&session).as_deref(),
        Some(name.as_str()),
        "session id must resolve back to the owning teammate"
    );
}

/// P0-5：只有 Team Lead 可以派生队友与中断成员。
#[test]
fn teammate_cannot_spawn_or_interrupt() {
    let caller = TeamCaller {
        name: unique("audit-teammate"),
        is_lead: false,
    };

    let spawn_error =
        spawn_teammate_in_workspace_with_provider_as(spawn_args("audit-forbidden"), None, None, &caller)
            .expect_err("teammate must not spawn another teammate");
    assert!(
        spawn_error.contains("Team Lead"),
        "spawn rejection must name the Lead requirement, got: {spawn_error}"
    );

    let interrupt_error = interrupt_agent_as(
        InterruptAgentArgs {
            target: "lead".to_string(),
            reason: Some("audit".to_string()),
        },
        &caller,
    )
    .expect_err("teammate must not interrupt the Lead");
    assert!(
        interrupt_error.contains("Team Lead"),
        "interrupt rejection must name the Lead requirement, got: {interrupt_error}"
    );
}

/// P0-1：无活跃同伴时等待必须快速失败（no-active-peer），不能空转到超时。
#[test]
fn wait_fast_returns_without_active_peer() {
    let name = unique("audit-wait");
    spawn_teammate(spawn_args(&name)).expect("spawn must register the teammate");
    let caller = TeamCaller {
        name: name.clone(),
        is_lead: false,
    };

    let started = Instant::now();
    let result = wait_agent_as(
        WaitAgentArgs {
            timeout_ms: Some(10_000),
        },
        &caller,
    )
    .expect("wait must not error");
    assert!(
        started.elapsed().as_millis() < 2_000,
        "wait must fast-return without active peers, elapsed={:?}",
        started.elapsed()
    );
    let no_progress = result
        .no_progress
        .expect("no active peer must report no_progress");
    assert_eq!(no_progress.reason, "no-active-peer");
    assert!(!result.timed_out, "no_progress is not a timeout");
}
