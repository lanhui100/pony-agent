//! 队友子进程的角色 git 身份（`backlog:B1795571360795`）。
//!
//! ## 要解决的问题
//!
//! 队友 turn 内跑的 `git commit` 与 Lead 完全同源：Run 走 `SandboxRequest` 的
//! `environment_allowlist`（`isolate_environment = true` 时 `env_clear` 后只留必需变量），
//! JobStart 直接继承父环境 —— 两条路径都没有任何身份信息，于是作者的 `user.name` 恒为宿主
//! 配置。实证：`~/job_copilot` 批次 4 个提交 author 全是占位 `t <a@b>`，`git log` 分不出
//! 哪笔是 Lead 写的、哪笔是队友写的（agent team 原缺陷证据③）。
//!
//! ## 判据（单源）
//!
//! 「某个会话是不是队友」的唯一真源是 [`teammate_target_for_session`]（由
//! `register_teammate` 预分配并登记固定会话 id）。本模块只把它翻译成 git 认的环境变量名，
//! **不自己维护任何成员表**：宿主会话（Lead）/ 未登记会话一律返回空，绝不改写用户的提交习惯。
//!
//! 注入的四个变量是 git 的官方钩子：`GIT_AUTHOR_*` 决定 commit 的 author，
//! `GIT_COMMITTER_*` 决定 committer；两者同时给，避免出现「author 是队友、committer 是宿主」
//! 的半截身份。

use crate::agent::orchestration::teammate_target_for_session;

/// 队友身份的名字前缀：`teammate:<target>`。
pub const TEAMMATE_AUTHOR_NAME_PREFIX: &str = "teammate:";
/// 队友身份的邮箱域名：`<target>@teammate.pony-agent.local`。
pub const TEAMMATE_AUTHOR_EMAIL_DOMAIN: &str = "teammate.pony-agent.local";

/// 把 teammate target 翻译成 git 身份四元组 `(name, email)`。
///
/// 纯函数（不读任何全局状态）：空 target 返回 `None`，其余一律给
/// `teammate:<target>` / `<target>@teammate.pony-agent.local`。
pub fn teammate_git_identity(target: &str) -> Option<(String, String)> {
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    Some((
        format!("{TEAMMATE_AUTHOR_NAME_PREFIX}{target}"),
        format!("{target}@{TEAMMATE_AUTHOR_EMAIL_DOMAIN}"),
    ))
}

/// 由会话 id 反查队友身份，产出 git 认的环境变量对（名字排序稳定，便于断言与 diff）。
///
/// 返回空 `Vec` 表示「不是队友会话」——调用方据此**不注入**任何变量（宿主 / 未知会话零影响）。
pub fn git_identity_env_for_session(session_id: Option<&str>) -> Vec<(String, String)> {
    let Some(target) = session_id.and_then(teammate_target_for_session) else {
        return Vec::new();
    };
    let Some((name, email)) = teammate_git_identity(&target) else {
        return Vec::new();
    };
    vec![
        ("GIT_AUTHOR_NAME".to_string(), name.clone()),
        ("GIT_AUTHOR_EMAIL".to_string(), email.clone()),
        ("GIT_COMMITTER_NAME".to_string(), name),
        ("GIT_COMMITTER_EMAIL".to_string(), email),
    ]
}

/// Run 通路的形态：`SandboxRequest::environment_allowlist` 接受 `KEY=VALUE` 字符串。
///
/// 与 [`git_identity_env_for_session`] 同源，只是把对子拼成沙箱契约认识的形状。
pub fn git_identity_allowlist_for_session(session_id: Option<&str>) -> Vec<String> {
    git_identity_env_for_session(session_id)
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_derived_from_target() {
        let (name, email) = teammate_git_identity("tester").expect("非空 target 必须有身份");
        assert_eq!(name, "teammate:tester");
        assert_eq!(email, "tester@teammate.pony-agent.local");
    }

    #[test]
    fn blank_target_has_no_identity() {
        assert!(teammate_git_identity("   ").is_none());
        assert!(teammate_git_identity("").is_none());
    }

    #[test]
    fn unknown_or_host_session_injects_nothing() {
        assert!(git_identity_env_for_session(None).is_empty());
        assert!(git_identity_env_for_session(Some("")).is_empty());
        assert!(git_identity_env_for_session(Some("not-a-teammate-session")).is_empty());
        assert!(git_identity_allowlist_for_session(None).is_empty());
    }
}
