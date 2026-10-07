//! task-3 默认工作区分叉红相验收契约（只写测试，不改业务）。
//!
//! 契约（黑盒）：
//! 1. `get_workspace_root` 必须读注册表 default（`SessionStore::resolve_workspace_root(None)`），
//!    而非 `std::env::current_dir`。验证：含 default 记录的 store 中，current_dir 与 default
//!    不同时仍返回 default。
//! 2. `ToolRouter::new` / `build_governed_executor(None)` 兜底必须等于
//!    `compute_default_workspace_root()`（Windows Documents/pony_agent，Unix ~/pony_agent），
//!    而非 current_dir。
//! 3. terminal/jobs 的 cwd 解析必须锚定会话 workspace root：cwd 缺省返回会话 root 而非
//!    current_dir；跨边界拒绝。
//!
//! 红相走查结论（对照现状代码，未跑 cargo，见 task-3 汇报）：
//! - 契约 1：`control_plane/mod.rs` L60-65 `default_workspace_root()` 取 current_dir，
//!   L1227-1233 `get_workspace_root` 回退到它，全程不读注册表 → test 1 必 fail。
//! - 契约 2：`tools.rs` L1009-1011 `ToolRouter::new()` 取 current_dir；
//!   `governed_executor.rs` L168-170 `build_governed_executor(None)` 取 current_dir；
//!   而 `workspace.rs` L101-108 `compute_default_workspace_root()` 取 ~/pony_agent →
//!   test 2a/2b 必 fail（测试机 cwd=仓库目录，default=~/pony_agent，两者不同）。
//! - 契约 3：`terminal.rs` L323-332 / `jobs.rs` L318-327 `validate_and_resolve_cwd`
//!   以 current_dir 为锚，且 `terminal_open`/`job_start` 根本没有会话 root 入参 →
//!   test 3a/3b/3c/3d 必 fail。注意：task-2 修复时若给这两个入口新增 workspace root
//!   入参，需同步更新本文件 4 个用例的调用点（已在各用例内标注 `TASK-2-WIRE`）。

use pony_agent_core::agent::config::ProviderRegistryStore;
use pony_agent_core::agent::context::DefaultTurnContextBuilder;
use pony_agent_core::agent::control_plane::HostControlPlane;
use pony_agent_core::agent::governed_executor::build_governed_executor;
use pony_agent_core::agent::planner::LocalTurnPlanner;
use pony_agent_core::agent::runtime::AgentRuntime;
use pony_agent_core::agent::session::{FileSessionBackend, SessionStore};
use pony_agent_core::agent::telemetry::DefaultTurnTelemetryBuilder;
use pony_agent_core::agent::tools::{
    job_kill, job_output, job_start, job_start_with_workspace_root, terminal_close, terminal_open,
    terminal_open_with_workspace_root, terminal_read, terminal_send, JobKillArgs, JobOutputArgs,
    JobStartArgs, TerminalCloseArgs, TerminalOpenArgs, TerminalReadArgs, TerminalSendArgs,
    ToolCall, ToolExecutor, ToolRouter,
};
use pony_agent_core::agent::workspace::compute_default_workspace_root;
use serde_json::{json, Value};
use std::path::PathBuf;

fn unique_tag(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{}-{nanos}", std::process::id())
}

fn current_dir_canonical() -> PathBuf {
    let cwd = std::env::current_dir().expect("current_dir must be available");
    cwd.canonicalize().unwrap_or(cwd)
}

fn make_temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(tag);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir.canonicalize().unwrap_or(dir)
}

fn canonicalize_lossy(raw: &str) -> String {
    PathBuf::from(raw)
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(raw))
        .display()
        .to_string()
}

// ── 契约 1：get_workspace_root 读注册表 default ──────────────────────────

#[test]
fn red_get_workspace_root_must_read_registry_default_not_current_dir() {
    let tag = unique_tag("red-phase-sessions");
    let sessions_path = std::env::temp_dir().join(format!("{tag}.json"));
    let _ = std::fs::remove_file(&sessions_path);
    let store = SessionStore::with_backend(Box::new(FileSessionBackend::new(sessions_path.clone())));
    // 注册表 default（bootstrap 已登记；current_dir 与它不同是本用例的前置条件）。
    let registry_default = store
        .resolve_workspace_root(None)
        .expect("registry must have bootstrapped default");
    let cwd = current_dir_canonical();
    assert_ne!(
        cwd.display().to_string(),
        registry_default,
        "前置条件失效：cwd 与注册表 default 重合，红相无区分度"
    );

    let runtime = AgentRuntime::with_dependencies(
        store,
        Box::new(ProviderRegistryStore::new()),
        Box::new(ToolRouter::new()),
        Box::new(LocalTurnPlanner),
        Box::new(DefaultTurnContextBuilder),
        Box::new(DefaultTurnTelemetryBuilder),
    );
    let plane = HostControlPlane::with_runtime(runtime);
    let actual = plane.get_workspace_root();
    let _ = std::fs::remove_file(&sessions_path);
    assert_eq!(
        actual, registry_default,
        "get_workspace_root 必须返回注册表 default，而非 current_dir"
    );
}

// ── 契约 2：ToolRouter::new / governed(None) 兜底 ────────────────────────

#[test]
fn red_tool_router_new_fallback_must_equal_compute_default_workspace_root() {
    let expected_raw =
        compute_default_workspace_root().expect("compute_default_workspace_root must resolve");
    let expected = canonicalize_lossy(&expected_raw.display().to_string());
    let cwd = current_dir_canonical();
    assert_ne!(
        cwd.display().to_string(),
        expected,
        "前置条件失效：cwd 与 compute_default_workspace_root 重合，红相无区分度"
    );

    // 黑盒观测：path_info(".").absolutePath 即 router 生效 root。
    let router = ToolRouter::new();
    let res = router.execute(&ToolCall {
        call_id: None,
        name: "workspace_path_info".to_string(),
        arguments: json!({ "path": "." }),
        plan: None,
    });
    assert_eq!(res.status, "ok", "path_info(.) should succeed: {}", res.output);
    let payload: Value = serde_json::from_str(&res.output).expect("path_info output must be json");
    let absolute = payload
        .get("absolutePath")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    assert_eq!(
        canonicalize_lossy(&absolute),
        expected,
        "ToolRouter::new 兜底必须等于 compute_default_workspace_root()"
    );
}

#[test]
fn red_governed_executor_none_fallback_must_equal_compute_default_workspace_root() {
    let expected_raw =
        compute_default_workspace_root().expect("compute_default_workspace_root must resolve");
    let expected = PathBuf::from(canonicalize_lossy(&expected_raw.display().to_string()));
    let cwd = current_dir_canonical();
    assert_ne!(
        cwd.display().to_string(),
        expected.display().to_string(),
        "前置条件失效：cwd 与 compute_default_workspace_root 重合，红相无区分度"
    );

    // 黑盒观测：Write 探针文件的实际落点即 executor 生效 root。
    let probe = format!("{}.txt", unique_tag("red-phase-probe"));
    let governed = build_governed_executor(None, None);
    let res = governed.execute(&ToolCall {
        call_id: None,
        name: "Write".to_string(),
        arguments: json!({
            "path": probe,
            "content": "red-phase probe\n",
            "description": "red phase probe",
        }),
        plan: None,
    });
    assert_eq!(res.status, "ok", "governed Write should succeed: {}", res.output);
    let in_expected = expected.join(&probe);
    let in_cwd = cwd.join(&probe);
    let verdict = (in_expected.exists(), in_cwd.exists());
    let _ = std::fs::remove_file(&in_expected);
    let _ = std::fs::remove_file(&in_cwd);
    assert!(
        verdict.0,
        "探针文件必须落在 compute_default_workspace_root 下；实际落点 expected={} cwd={}",
        verdict.0, verdict.1
    );
    assert!(
        !verdict.1,
        "探针文件不得落在 current_dir 下（兜底分叉证据）"
    );
}

// ── 契约 3：terminal/jobs cwd 锚定会话 workspace ─────────────────────────
// TASK-2-WIRE：以下 4 个用例当前只能调用无会话-root 入参的旧签名， desired
// 行为必然失败；task-2 给 terminal_open/job_start 接入会话 workspace root 后，
// 把 `ws` 按新签名传入即可转绿。

#[test]
fn red_job_cwd_none_must_default_to_session_workspace_root() {
    // TASK-2-WIRE：新签名应为 job_start(args, session_workspace_root=Some(&ws))。
    let ws = make_temp_dir(&unique_tag("red-phase-ws"));
    let cwd = current_dir_canonical();
    assert_ne!(ws, cwd, "前置条件失效：会话 workspace 与 cwd 重合，红相无区分度");

    let started = job_start(JobStartArgs {
        command: "sh".to_string(),
        args: Some(vec!["-c".to_string(), "pwd".to_string()]),
        cwd: None,
        timeout_ms: Some(10_000),
    })
    .expect("job_start should succeed");
    let out = job_output(JobOutputArgs {
        job_id: started.job_id.clone(),
        wait: Some(true),
        timeout_ms: Some(8_000),
        offset: Some(0),
    })
    .expect("job_output should succeed");
    let _ = job_kill(JobKillArgs {
        job_id: started.job_id,
        reason: Some("cleanup".to_string()),
    });
    let actual = out.output.trim().to_string();
    let _ = std::fs::remove_dir_all(&ws);
    assert_eq!(
        canonicalize_lossy(&actual),
        ws.display().to_string(),
        "job cwd 缺省必须返回会话 workspace root，而非 current_dir"
    );
}

#[test]
fn red_job_cwd_outside_session_workspace_must_be_rejected() {
    // TASK-2-WIRE：新签名应为 job_start(args, session_workspace_root=Some(&ws))，
    // 本用例的 outside 届时相对 ws 判定。
    let ws = make_temp_dir(&unique_tag("red-phase-ws"));
    // outside 选在 current_dir 下：相对 current_dir 锚合法、相对会话 ws 越界。
    let outside = current_dir_canonical().join(unique_tag("red-phase-outside"));
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).expect("create outside dir");
    assert!(
        !outside.starts_with(&ws),
        "前置条件失效：outside 落在会话 workspace 内"
    );

    let attempt = job_start(JobStartArgs {
        command: "sh".to_string(),
        args: Some(vec!["-c".to_string(), "exit 0".to_string()]),
        cwd: Some(outside.display().to_string()),
        timeout_ms: Some(10_000),
    });
    match attempt {
        Err(_) => {}
        Ok(started) => {
            let _ = job_kill(JobKillArgs {
                job_id: started.job_id,
                reason: Some("cleanup".to_string()),
            });
            panic!(
                "跨会话边界 cwd 必须被拒绝，当前被放行：{}",
                outside.display()
            );
        }
    }
    let _ = std::fs::remove_dir_all(&outside);
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn red_terminal_cwd_outside_session_workspace_must_be_rejected() {
    // TASK-2-WIRE：新签名应为 terminal_open(args, session_workspace_root=Some(&ws))。
    let ws = make_temp_dir(&unique_tag("red-phase-ws"));
    let outside = current_dir_canonical().join(unique_tag("red-phase-outside"));
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).expect("create outside dir");
    assert!(
        !outside.starts_with(&ws),
        "前置条件失效：outside 落在会话 workspace 内"
    );

    let attempt = terminal_open(TerminalOpenArgs {
        command: Some("sh".to_string()),
        args: None,
        cwd: Some(outside.display().to_string()),
        cols: Some(80),
        rows: Some(24),
        env: None,
    });
    match attempt {
        Err(_) => {}
        Ok(opened) => {
            let _ = terminal_close(TerminalCloseArgs {
                terminal_id: opened.terminal_id,
                force: Some(true),
            });
            panic!(
                "跨会话边界 cwd 必须被拒绝，当前被放行：{}",
                outside.display()
            );
        }
    }
    let _ = std::fs::remove_dir_all(&outside);
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn red_terminal_cwd_none_must_default_to_session_workspace_root() {
    // TASK-2-WIRE：新签名应为 terminal_open(args, session_workspace_root=Some(&ws))。
    let ws = make_temp_dir(&unique_tag("red-phase-ws"));
    let cwd = current_dir_canonical();
    assert_ne!(ws, cwd, "前置条件失效：会话 workspace 与 cwd 重合，红相无区分度");

    let opened = terminal_open(TerminalOpenArgs {
        command: Some("sh".to_string()),
        args: None,
        cwd: None,
        cols: Some(80),
        rows: Some(24),
        env: None,
    })
    .expect("terminal_open should succeed");
    let send_ok = terminal_send(TerminalSendArgs {
        terminal_id: opened.terminal_id.clone(),
        input: "pwd\n".to_string(),
    });
    assert!(send_ok.is_ok(), "terminal_send should succeed: {send_ok:?}");
    let read = terminal_read(TerminalReadArgs {
        terminal_id: opened.terminal_id.clone(),
        timeout_ms: Some(5_000),
        offset: Some(0),
    })
    .expect("terminal_read should succeed");
    let _ = terminal_close(TerminalCloseArgs {
        terminal_id: opened.terminal_id,
        force: Some(true),
    });
    let _ = std::fs::remove_dir_all(&ws);
    assert!(
        read.output.contains(&ws.display().to_string()),
        "terminal cwd 缺省必须为会话 workspace root；实际输出：{}",
        read.output
    );
}

// ── task-4 补丁：Some(ws) 直接锚定用例（走新 with_workspace_root 入参） ───
// 说明：旧签名红相 4 用例（red_job_*/red_terminal_*）继续保留；以下 4 个直接
// 命中业务新 API。当前业务状态预期：4 个 Some(ws) 用例应绿（task-2 已接线）；
// None 分支 create_dir_all 污染（terminal.rs/jobs.rs resolve_cwd_anchor None 分支）
// 尚未移除——待 Executor 修，该条记为待修断言而非假绿（见本文件末尾注释用例）。

#[test]
fn patch4_job_some_ws_none_cwd_must_default_to_ws() {
    let ws = make_temp_dir(&unique_tag("patch4-job-ws"));
    let cwd = current_dir_canonical();
    assert_ne!(ws, cwd, "前置条件失效：会话 workspace 与 cwd 重合，无区分度");

    let started = job_start_with_workspace_root(
        JobStartArgs {
            command: "sh".to_string(),
            args: Some(vec!["-c".to_string(), "pwd".to_string()]),
            cwd: None,
            timeout_ms: Some(10_000),
        },
        Some(ws.as_path()),
    )
    .expect("job_start_with_workspace_root should succeed");
    let out = job_output(JobOutputArgs {
        job_id: started.job_id.clone(),
        wait: Some(true),
        timeout_ms: Some(8_000),
        offset: Some(0),
    })
    .expect("job_output should succeed");
    let _ = job_kill(JobKillArgs {
        job_id: started.job_id,
        reason: Some("cleanup".to_string()),
    });
    let actual = out.output.trim().to_string();
    let _ = std::fs::remove_dir_all(&ws);
    assert_eq!(
        canonicalize_lossy(&actual),
        ws.display().to_string(),
        "Some(ws)+cwd缺省必须返回 ws（会话锚定）"
    );
}

#[test]
fn patch4_job_some_ws_cross_boundary_cwd_must_be_rejected() {
    let ws = make_temp_dir(&unique_tag("patch4-job-ws"));
    let outside = current_dir_canonical().join(unique_tag("patch4-job-outside"));
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).expect("create outside dir");
    assert!(
        !outside.starts_with(&ws),
        "前置条件失效：outside 落在会话 workspace 内"
    );

    let attempt = job_start_with_workspace_root(
        JobStartArgs {
            command: "sh".to_string(),
            args: Some(vec!["-c".to_string(), "exit 0".to_string()]),
            cwd: Some(outside.display().to_string()),
            timeout_ms: Some(10_000),
        },
        Some(ws.as_path()),
    );
    match attempt {
        Err(_) => {}
        Ok(started) => {
            let _ = job_kill(JobKillArgs {
                job_id: started.job_id,
                reason: Some("cleanup".to_string()),
            });
            panic!(
                "Some(ws)+跨界 cwd 必须被拒绝，当前被放行：{}",
                outside.display()
            );
        }
    }
    let _ = std::fs::remove_dir_all(&outside);
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn patch4_terminal_some_ws_none_cwd_must_default_to_ws() {
    let ws = make_temp_dir(&unique_tag("patch4-term-ws"));
    let cwd = current_dir_canonical();
    assert_ne!(ws, cwd, "前置条件失效：会话 workspace 与 cwd 重合，无区分度");

    let opened = terminal_open_with_workspace_root(
        TerminalOpenArgs {
            command: Some("sh".to_string()),
            args: None,
            cwd: None,
            cols: Some(80),
            rows: Some(24),
            env: None,
        },
        Some(ws.as_path()),
    )
    .expect("terminal_open_with_workspace_root should succeed");
    let send_ok = terminal_send(TerminalSendArgs {
        terminal_id: opened.terminal_id.clone(),
        input: "pwd\n".to_string(),
    });
    assert!(send_ok.is_ok(), "terminal_send should succeed: {send_ok:?}");
    let read = terminal_read(TerminalReadArgs {
        terminal_id: opened.terminal_id.clone(),
        timeout_ms: Some(5_000),
        offset: Some(0),
    })
    .expect("terminal_read should succeed");
    let _ = terminal_close(TerminalCloseArgs {
        terminal_id: opened.terminal_id,
        force: Some(true),
    });
    let _ = std::fs::remove_dir_all(&ws);
    assert!(
        read.output.contains(&ws.display().to_string()),
        "Some(ws)+cwd缺省必须为 ws；实际输出：{}",
        read.output
    );
}

#[test]
fn patch4_terminal_some_ws_cross_boundary_cwd_must_be_rejected() {
    let ws = make_temp_dir(&unique_tag("patch4-term-ws"));
    let outside = current_dir_canonical().join(unique_tag("patch4-term-outside"));
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).expect("create outside dir");
    assert!(
        !outside.starts_with(&ws),
        "前置条件失效：outside 落在会话 workspace 内"
    );

    let attempt = terminal_open_with_workspace_root(
        TerminalOpenArgs {
            command: Some("sh".to_string()),
            args: None,
            cwd: Some(outside.display().to_string()),
            cols: Some(80),
            rows: Some(24),
            env: None,
        },
        Some(ws.as_path()),
    );
    match attempt {
        Err(_) => {}
        Ok(opened) => {
            let _ = terminal_close(TerminalCloseArgs {
                terminal_id: opened.terminal_id,
                force: Some(true),
            });
            panic!(
                "Some(ws)+跨界 cwd 必须被拒绝，当前被放行：{}",
                outside.display()
            );
        }
    }
    let _ = std::fs::remove_dir_all(&outside);
    let _ = std::fs::remove_dir_all(&ws);
}

// 待修断言（非假绿声明）：resolve_cwd_anchor 的 None 分支当前含
// `create_dir_all(compute_default_workspace_root())` 副作用（terminal.rs L369 /
// jobs.rs L365），task-4 要求改为"缺失时回退而非主动建目录"。以下用例在该副作用
// 未移除前必然失败——Executor 修完业务后它应转绿；当前跑 cargo test 时请把它的
// 失败解读为"待修"，不要解读为回归。
#[test]
fn patch4_none_branch_must_not_create_default_dir_as_side_effect() {
    let def = compute_default_workspace_root().expect("compute default must resolve");
    // 前置：若真实 ~/pony_agent 已存在则跳过（不删用户目录，只做存在性旁证）。
    if def.exists() {
        eprintln!(
            "SKIP-SIDEEFFECT-PROBE: {} 已存在，无法做缺失旁证；待 Executor 修后 code-review 确认",
            def.display()
        );
        return;
    }
    let _ = job_start(JobStartArgs {
        command: "sh".to_string(),
        args: Some(vec!["-c".to_string(), "exit 0".to_string()]),
        cwd: None,
        timeout_ms: Some(10_000),
    });
    assert!(
        !def.exists(),
        "None 分支不得以 create_dir_all 副作用创建 {}",
        def.display()
    );
}

// ── PA-114 workspace-cwd 第二轮（fix-round2-workspace-cwd）红相验收 F4/F5/F6A ──
// 契约：`.dev-team/fix-round2-workspace-cwd.md` §3 F4/F5/F6。
// 红相走查：F4/F5/F6A 目标均为已实现函数（bootstrap_default_workspace_inner、
// resolve_default_workspace_base 参数化、ToolRouter::new 兜底语义）→ 预期本轮即绿，
// 如实记录 PASS。

/// F4（E5/R2-2）：`bootstrap_default_workspace_inner(&mut records, None)` → false
/// 且注册表保持空（base 解析失败不得注册、不得回退）。
#[test]
fn f4_bootstrap_default_workspace_inner_none_is_noop() {
    use pony_agent_core::agent::workspace::bootstrap_default_workspace_inner;
    let mut records = Vec::new();
    let registered = bootstrap_default_workspace_inner(&mut records, None);
    assert!(!registered, "None base must not register a default workspace");
    assert!(records.is_empty(), "records must stay empty on the None branch");
}

/// F5（R2-1）：`resolve_default_workspace_base` Windows 形态在 Linux 可测
/// （windows=true 参数化，path_exists 注入）——doc/存在性/兜底/全 None 矩阵。
#[test]
fn f5_resolve_default_workspace_base_windows_forms_are_testable_on_linux() {
    use pony_agent_core::agent::workspace::resolve_default_workspace_base;
    use std::path::Path;
    let exists = |_p: &Path| true;
    let missing = |_p: &Path| false;

    // windows=true：doc Some → doc（Known-Folder 优先，不落到 Documents）。
    assert_eq!(
        resolve_default_workspace_base(
            Some(Path::new("/doc")),
            Some(Path::new("/home")),
            true,
            &exists,
        ),
        Some(PathBuf::from("/doc"))
    );
    // doc None + home/Documents 存在 → home/Documents。
    assert_eq!(
        resolve_default_workspace_base(None, Some(Path::new("/home")), true, &exists),
        Some(PathBuf::from("/home/Documents"))
    );
    // doc None + Documents 不存在 → home（R2-3）。
    assert_eq!(
        resolve_default_workspace_base(None, Some(Path::new("/home")), true, &missing),
        Some(PathBuf::from("/home"))
    );
    // 全 None → None（任一存在性判定下）。
    assert_eq!(resolve_default_workspace_base(None, None, true, &exists), None);
    assert_eq!(resolve_default_workspace_base(None, None, true, &missing), None);
    // windows=false（Unix 形态）：home Some → home；全 None → None。
    assert_eq!(
        resolve_default_workspace_base(None, Some(Path::new("/home")), false, &exists),
        Some(PathBuf::from("/home"))
    );
    assert_eq!(resolve_default_workspace_base(None, None, false, &exists), None);
}

/// F6A（W2 语义锁定，可观测部分）：`ToolRouter::new` 默认 root = compute 结果
/// （compute 可解析时进程 cwd 兜底分支不可达；None 分支告警见 F6 seam 测试）。
#[test]
fn f6a_tool_router_new_fallback_priority_is_computed_then_cwd() {
    let expected_raw =
        compute_default_workspace_root().expect("compute_default_workspace_root must resolve");
    let expected = canonicalize_lossy(&expected_raw.display().to_string());

    let router = ToolRouter::new();
    let res = router.execute(&ToolCall {
        call_id: None,
        name: "workspace_path_info".to_string(),
        arguments: json!({ "path": "." }),
        plan: None,
    });
    assert_eq!(res.status, "ok", "path_info probe failed: {}", res.output);
    let payload: Value = serde_json::from_str(&res.output).expect("path_info json output");
    let absolute = payload
        .get("absolutePath")
        .and_then(Value::as_str)
        .expect("absolutePath present");
    assert_eq!(
        canonicalize_lossy(absolute),
        expected,
        "ToolRouter::new fallback priority must be compute_default_workspace_root() → cwd"
    );
}
