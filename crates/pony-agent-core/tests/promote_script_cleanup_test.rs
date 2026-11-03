//! B013 红相验收：promote 的临时脚本清理（Linux 可验证形态）。
//!
//! 契约真源：`.dev-team/contracts/tool-timeout-policy.md` §波次四 · CT-P6c / B013
//! （promote 路径不得 `remove_windows_batch_script` —— 进程仍在读该脚本；脚本清理责任移交
//! **job 生命周期**）。本文件在 Linux 上用「任意临时文件」模拟 Windows `.cmd` 残留，验收：
//!
//! - CT-B013-1：promoted 作业收敛到终态后，注册时交付的临时脚本**必须被删除**；
//! - CT-B013-2：未提供脚本路径时 `job_kill` 不得报错（向后兼容，既有调用点零改动）；
//! - CT-B013-3：脚本清理失败（路径不可删）**不得** panic、**不得**把作业状态改成失败。
//!
//! ## 可测接缝假设（本文件先冻结，实现侧对齐）
//! `jobs::register_promoted` 追加**最后一个**参数 `cleanup_script: Option<std::path::PathBuf>`。
//! 现有 10 参不动（manager + 租借三元组 + 命令行 + 首读快照 + 两个截断标记 + 取消令牌 + owner）。
//! 契约点是**行为**（终态后删除 / 缺省不报错 / 失败不影响状态），不是参数名；若实现另选类型
//! （如 `Option<&Path>`），按实现同步本文件即可。
//!
//! 本文件是**外部黑盒验收**（integration test，独立 crate）：只经 `ProcessBackend` 与
//! `jobs` 公开 API 操作，不读私有字段、不吞异常、不使用永真断言。

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pony_agent_core::agent::jobs::{
    job_kill, job_output, register_promoted, JobKillArgs, JobOutputArgs,
};
use pony_agent_core::agent::process::ProcessManager;
use pony_agent_core::agent::tool_runtime::{ProcessBackend, ProcessStartRequest, SandboxRequest};

/// 「Windows 批处理残留」的模拟文件名前缀（Linux 上只是普通临时文件）。
const SCRIPT_PREFIX: &str = "pony-agent-run-test";

fn unique_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after UNIX_EPOCH")
        .as_nanos()
}

/// 唯一会话 id（`ProcessManager` 的句柄绑定会话，跨用例不得复用）。
fn unique_session(tag: &str) -> String {
    format!("b013-{}-{}", tag, unique_nanos())
}

/// 临时「批处理脚本」路径（不建目录，直接落在系统临时目录）。
fn unique_script_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("{}-{}-{}.cmd", SCRIPT_PREFIX, tag, unique_nanos()))
}

/// 造一个模拟 `.cmd` 残留：写入若干字节并返回路径。
fn write_fake_script(tag: &str) -> PathBuf {
    let path = unique_script_path(tag);
    std::fs::write(&path, b"@echo off\r\nsleep 5\r\n").expect("write fake batch script");
    assert!(
        path.exists(),
        "前置条件：模拟脚本必须已落盘：{}",
        path.display()
    );
    path
}

/// 起一个「仍在前台运行」的进程，返回 `(session_id, handle)` —— 满足 `register_promoted`
/// 的 `(ProcessManager, session_id, handle)` 租借三元组前提。
#[cfg(windows)]
fn spawn_long_lived(manager: &ProcessManager, session: &str) -> String {
    spawn_with(manager, session, "ping", vec!["-n", "6", "127.0.0.1"])
}

#[cfg(not(windows))]
fn spawn_long_lived(manager: &ProcessManager, session: &str) -> String {
    spawn_with(manager, session, "sleep", vec!["5"])
}

fn spawn_with(manager: &ProcessManager, session: &str, program: &str, args: Vec<&str>) -> String {
    let workspace = std::env::temp_dir().display().to_string();
    manager
        .start(&ProcessStartRequest {
            session_id: session.to_string(),
            program: program.to_string(),
            arguments: args.into_iter().map(str::to_string).collect(),
            sandbox: SandboxRequest {
                workspace_root: workspace,
                allow_network: false,
                environment_allowlist: Vec::new(),
                isolate_environment: true,
            },
        })
        .expect("ProcessManager::start 必须成功（B013 前置：需要一条真实存活进程）")
}

/// 读作业状态；作业必须存在，否则直接 panic（禁止静默放行）。
fn job_status(job_id: &str) -> String {
    job_output(JobOutputArgs {
        job_id: job_id.to_string(),
        wait: Some(false),
        timeout_ms: Some(0),
        offset: Some(0),
    })
    .unwrap_or_else(|error| panic!("job_output 必须能读到作业 `{job_id}`：{error}"))
    .status
}

/// 显式条件轮询（禁止盲 sleep）：`cond` 成立即返回，超时则 panic 并给出 `what`。
fn wait_until(mut cond: impl FnMut() -> bool, timeout: Duration, what: &str) {
    let deadline = Instant::now() + timeout;
    while !cond() {
        assert!(Instant::now() < deadline, "等待超时：{what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// 用例级回收守卫：Drop 时尽力 kill 进程 + 删除脚本（panic 路径同样回收，不污染后续用例）。
struct PromotedCleanup<'a> {
    manager: &'a ProcessManager,
    session: String,
    handle: Option<String>,
    script: Option<PathBuf>,
}

impl Drop for PromotedCleanup<'_> {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = self.manager.kill(&self.session, &handle);
        }
        if let Some(script) = self.script.take() {
            if script.exists() {
                let _ = std::fs::remove_file(&script);
            }
        }
    }
}

// ── CT-B013-1 ──────────────────────────────────────────────────────────────

/// CT-B013-1：promoted 作业收敛到终态后，注册时交付的临时脚本必须被删除。
#[test]
fn promoted_job_removes_cleanup_script_on_terminal_state() {
    // Arrange
    let manager = ProcessManager::new();
    let session = unique_session("cleanup");
    let handle = spawn_long_lived(&manager, &session);
    let script = write_fake_script("cleanup");
    let mut guard = PromotedCleanup {
        manager: &manager,
        session: session.clone(),
        handle: Some(handle.clone()),
        script: Some(script.clone()),
    };

    // Act 1：把存活进程登记为 promoted 作业，并把模拟脚本的清理责任交给 job 生命周期。
    let job_id = register_promoted(
        manager.clone(),
        session.clone(),
        handle.clone(),
        "sleep 5".to_string(),
        "",
        "",
        false,
        false,
        None,
        Some(session.as_str()),
        Some(script.clone()),
    )
    .expect("register_promoted 必须接受 cleanup 脚本路径并成功注册");
    guard.handle = Some(handle.clone());

    // Act 2：收敛到终态。
    let kill = job_kill(JobKillArgs {
        job_id: job_id.clone(),
        reason: Some("b013 cleanup".to_string()),
    })
    .expect("job_kill 对 promoted 作业必须成功");
    assert!(
        kill.killed,
        "CT-B013-1：job_kill 必须返回 killed=true，实际 false（job={job_id}）"
    );

    // Assert
    wait_until(
        || !script.exists(),
        Duration::from_secs(2),
        &format!(
            "CT-B013-1：promoted 作业终态后必须删除临时脚本，实际仍残留 {}",
            script.display()
        ),
    );
    assert!(
        !script.exists(),
        "CT-B013-1：promoted 作业终态后必须删除临时脚本（残留路径 {}）",
        script.display()
    );
    let status = job_status(&job_id);
    assert!(
        status != "running",
        "CT-B013-1：job_kill 之后作业必须离开 running，实际 `{status}`"
    );
}

// ── CT-B013-2 ──────────────────────────────────────────────────────────────

/// CT-B013-2：未提供 cleanup 脚本路径时（既有调用点），`job_kill` 不得报错（向后兼容）。
#[test]
fn job_kill_without_cleanup_script_stays_backward_compatible() {
    // Arrange
    let manager = ProcessManager::new();
    let session = unique_session("noclean");
    let handle = spawn_long_lived(&manager, &session);
    let mut guard = PromotedCleanup {
        manager: &manager,
        session: session.clone(),
        handle: Some(handle.clone()),
        script: None,
    };

    // Act
    let job_id = register_promoted(
        manager.clone(),
        session.clone(),
        handle.clone(),
        "sleep 5".to_string(),
        "",
        "",
        false,
        false,
        None,
        Some(session.as_str()),
        None,
    )
    .expect("register_promoted 必须接受 `None` cleanup 路径（向后兼容）");
    guard.handle = Some(handle.clone());

    // Assert：不报错，且作业确实被终止。
    let kill = job_kill(JobKillArgs {
        job_id: job_id.clone(),
        reason: Some("b013 no-cleanup".to_string()),
    })
    .expect("CT-B013-2：未提供 cleanup 路径时 job_kill 不得报错");
    assert!(
        kill.killed,
        "CT-B013-2：job_kill 必须返回 killed=true，实际 false（job={job_id}）"
    );
    let status = job_status(&job_id);
    assert!(
        status != "running",
        "CT-B013-2：job_kill 之后作业必须离开 running，实际 `{status}`"
    );
}

// ── CT-B013-3 ──────────────────────────────────────────────────────────────

/// 不可删除的脚本路径：Linux 用 `/proc/1/<不存在文件>`（EPERM/ENOENT）；
/// Windows 用不存在的盘符路径（真实平台无法在本机验证，仅保证可编译）。
#[cfg(windows)]
fn undeletable_script_path() -> PathBuf {
    PathBuf::from(r"Z:\pony-agent-nonexistent-dir\b013-does-not-exist.cmd")
}

#[cfg(not(windows))]
fn undeletable_script_path() -> PathBuf {
    Path::new("/proc/1/pony-agent-b013-does-not-exist.cmd").to_path_buf()
}

/// CT-B013-3：脚本清理失败不得 panic、不得把作业状态改成失败。
#[test]
fn cleanup_failure_does_not_poison_job_state() {
    // Arrange
    let manager = ProcessManager::new();
    let session = unique_session("badclean");
    let handle = spawn_long_lived(&manager, &session);
    let mut guard = PromotedCleanup {
        manager: &manager,
        session: session.clone(),
        handle: Some(handle.clone()),
        script: None,
    };
    let bogus = undeletable_script_path();

    // Act：注册一个「名义上要清理、实际删不掉」的脚本路径。
    let job_id = register_promoted(
        manager.clone(),
        session.clone(),
        handle.clone(),
        "sleep 5".to_string(),
        "",
        "",
        false,
        false,
        None,
        Some(session.as_str()),
        Some(bogus),
    )
    .expect("register_promoted 必须接受 cleanup 路径（即使它将来删不掉）");
    guard.handle = Some(handle.clone());

    // Act：kill（清理必失败）。允许实现返回 Ok 或 Err —— 契约只要求「不 panic、不改状态」。
    let kill_result = job_kill(JobKillArgs {
        job_id: job_id.clone(),
        reason: Some("b013 bad-cleanup".to_string()),
    });

    // Assert 1：作业仍是终态且**不是** failed（清理失败不得污染生命周期状态）。
    let status = job_status(&job_id);
    assert_ne!(
        status, "failed",
        "CT-B013-3：脚本清理失败不得把作业状态改成 failed，实际 `{status}`"
    );
    assert_eq!(
        status, "killed",
        "CT-B013-3：脚本清理失败不得影响 kill 语义（仍应为 killed），实际 `{status}`"
    );

    // Assert 2：错误（若有）必须可读，且进程确实已终止 —— 不得因清理失败而留下活进程。
    if let Err(error) = kill_result {
        assert!(
            !error.trim().is_empty(),
            "CT-B013-3：若 job_kill 因清理失败返回错误，错误信息不得为空"
        );
    }
    assert_eq!(
        status, "killed",
        "CT-B013-3：清理失败后作业必须已终止（killed），实际 `{status}`"
    );
}
