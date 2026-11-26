//! B1795571360795：队友 turn 的子进程必须带角色 git 身份。
//!
//! 事故链（`~/job_copilot` 2026-11-24 实测，转登本仓 `backlog:B1795571360795`）：dev-team 批次
//! 的 4 个提交 author 全是占位 `t <a@b>`，`git log` 分不出哪笔是 Lead 写的、哪笔是队友写的，
//! 交付归属不可审计（`backlog:B1795535505312` 原证据③）。
//!
//! 判定口径（`.agents/notes/implemented/bug-fix/2026-11-25-teammate-git-identity.md`）：
//! 1. 队友会话起的子进程带 `GIT_AUTHOR_*` / `GIT_COMMITTER_*`，值为
//!    `teammate:<target>` / `<target>@teammate.pony-agent.local`。
//! 2. 宿主（Lead）会话与未登记会话**不注入**任何身份变量——绝不改写用户的提交习惯。
//! 3. Run（沙箱 allowlist）与 JobStart（继承环境 + 显式 env）两条通路同源同判据。
//!
//! 隔离：团队名册/会话登记是进程级静态表 ⇒ 用唯一 target 名避免与同进程其他用例串扰。

use std::time::{Duration, Instant};

use pony_agent_core::agent::jobs::{job_output_scoped, job_start_scoped, JobOutputArgs, JobStartArgs};
use pony_agent_core::agent::orchestration::{
    spawn_teammate, teammate_session_id, SpawnTeammateArgs,
};
use pony_agent_core::agent::teammate_identity::{
    git_identity_allowlist_for_session, git_identity_env_for_session, teammate_git_identity,
};

/// 登记一个队友并回显「target → 固定会话 id」（纯登记，不投递、不驱动）。
fn register_teammate(tag: &str) -> (String, String) {
    let name = format!("b1795571360795-{tag}");
    let result = spawn_teammate(SpawnTeammateArgs {
        name: name.clone(),
        role: Some("executor".to_string()),
        description: format!("{tag} 身份注入验收队友"),
        prompt: String::new(),
        context: None,
        provider_id: None,
        model_id: None,
    })
    .expect("纯登记必须成功");
    let target = result.member.target.clone();
    let session = teammate_session_id(&target).expect("派生后必须预分配并登记固定会话 id");
    (target, session)
}

/// 读出子进程里 `GIT_AUTHOR_NAME` 的值（跨平台：unix 用 sh，Windows 用 cmd）。
fn read_author_name(owner: Option<&str>) -> String {
    #[cfg(unix)]
    let args = JobStartArgs {
        command: "sh".to_string(),
        args: Some(vec![
            "-c".to_string(),
            "printf %s \"$GIT_AUTHOR_NAME\"".to_string(),
        ]),
        cwd: None,
        timeout_ms: None,
        description: None,
    };
    #[cfg(windows)]
    let args = JobStartArgs {
        command: "cmd".to_string(),
        args: Some(vec![
            "/C".to_string(),
            "echo %GIT_AUTHOR_NAME%".to_string(),
        ]),
        cwd: None,
        timeout_ms: None,
        description: None,
    };

    let started = job_start_scoped(args, None, owner).expect("后台作业必须能启动");
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut collected = String::new();
    loop {
        let out = job_output_scoped(
            JobOutputArgs {
                job_id: started.job_id.clone(),
                wait: Some(true),
                timeout_ms: Some(500),
                offset: None,
            },
            owner,
        )
        .expect("读取自己作业的输出必须成功");
        collected.push_str(&out.output);
        if out.status != "running" {
            return collected.trim().to_string();
        }
        if Instant::now() >= deadline {
            panic!("子进程 10s 内未收口；已收集输出 {collected:?}");
        }
    }
}

#[test]
fn identity_is_teammate_prefixed() {
    let (name, email) = teammate_git_identity("executor-a").expect("非空 target 必须有身份");
    assert_eq!(name, "teammate:executor-a");
    assert_eq!(email, "executor-a@teammate.pony-agent.local");
}

#[test]
fn host_session_injects_no_identity() {
    assert!(git_identity_env_for_session(None).is_empty());
    assert!(git_identity_env_for_session(Some("pa_host_session")).is_empty());
    assert!(git_identity_allowlist_for_session(Some("pa_host_session")).is_empty());
    // 端到端反证：宿主会话起的作业读不到队友身份（未注入 ≠ 读到别人的身份）。
    let value = read_author_name(Some("pa_b1795571360795_host"));
    assert!(
        !value.starts_with("teammate:"),
        "宿主会话不得被注入队友身份，实际读到 {value:?}"
    );
}

#[test]
fn teammate_session_env_carries_role_identity() {
    let (target, session) = register_teammate("env");

    let pairs = git_identity_env_for_session(Some(&session));
    assert_eq!(
        pairs,
        vec![
            (
                "GIT_AUTHOR_NAME".to_string(),
                format!("teammate:{target}")
            ),
            (
                "GIT_AUTHOR_EMAIL".to_string(),
                format!("{target}@teammate.pony-agent.local")
            ),
            (
                "GIT_COMMITTER_NAME".to_string(),
                format!("teammate:{target}")
            ),
            (
                "GIT_COMMITTER_EMAIL".to_string(),
                format!("{target}@teammate.pony-agent.local")
            ),
        ],
        "四个身份变量必须同源成对给出（author 与 committer 不得半截）"
    );

    // Run 通路形状：沙箱 allowlist 只吃 `KEY=VALUE`。
    let allowlist = git_identity_allowlist_for_session(Some(&session));
    assert_eq!(allowlist.len(), 4);
    assert!(allowlist
        .iter()
        .any(|entry| entry == &format!("GIT_AUTHOR_NAME=teammate:{target}")));
}

#[test]
fn teammate_job_subprocess_sees_role_identity() {
    let (target, session) = register_teammate("job");

    // 端到端：队友会话起的真实子进程读环境变量 ⇒ 证明注入真的落到了 spawn 上，
    // 而不只是「函数返回了一个好看的 Vec」。
    let value = read_author_name(Some(&session));
    assert_eq!(
        value,
        format!("teammate:{target}"),
        "队友会话的子进程 GIT_AUTHOR_NAME 必须是 teammate:<target>"
    );
}

/// Run 通路端到端：沙箱 `isolate_environment = true` 时 `env_clear` 后只留必需变量 +
/// allowlist，所以身份变量**必须**经 allowlist 注入才可能被看到 —— 本用例证明它真的到了。
#[cfg(unix)]
#[test]
fn teammate_run_subprocess_sees_role_identity() {
    use pony_agent_core::agent::sandbox::TestSandboxBackend;
    use pony_agent_core::agent::tools::{ToolCall, ToolExecutionContext, ToolRouter};
    use serde_json::json;

    let (target, session) = register_teammate("run");

    let workspace = std::env::temp_dir().join(format!(
        "pony-b1795571360795-run-{}-{}",
        std::process::id(),
        target
    ));
    std::fs::create_dir_all(&workspace).expect("建临时工作区");

    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_sandbox_backend(TestSandboxBackend::available());
    let call = ToolCall {
        name: "workspace_run_command".to_string(),
        arguments: json!({
            "command": "printf %s \"$GIT_AUTHOR_NAME\"",
            "cwd": workspace.display().to_string(),
            "description": "b1795571360795 Run 身份注入验收",
        }),
        call_id: Some("call-b1795571360795-run".to_string()),
        plan: None,
    };
    let context = ToolExecutionContext {
        workspace_root: Some(workspace.clone()),
        session_id: Some(session.clone()),
        ..Default::default()
    };

    let result = router.execute_with_context(&call, &context);
    let _ = std::fs::remove_dir_all(&workspace);

    assert_eq!(result.status, "ok", "Run 必须成功：{}", result.output);
    let payload: serde_json::Value =
        serde_json::from_str(&result.output).expect("Run 输出必须是合法 JSON");
    let stdout = payload
        .get("result")
        .and_then(|value| value.get("stdout"))
        .and_then(|value| value.as_str())
        .or_else(|| payload.get("stdout").and_then(|value| value.as_str()))
        .unwrap_or_else(|| panic!("Run 输出缺 stdout：{}", result.output));
    assert_eq!(
        stdout.trim(),
        format!("teammate:{target}"),
        "队友会话的 Run 子进程必须看到角色身份（沙箱 env_clear 后仅 allowlist 生效）"
    );
}
