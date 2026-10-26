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
    JobOutputResult, JobStartArgs, TerminalCloseArgs, TerminalOpenArgs, TerminalReadArgs,
    TerminalSendArgs, ToolCall, ToolExecutor, ToolRouter,
};
use pony_agent_core::agent::workspace::compute_default_workspace_root;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn unique_tag(prefix: &str) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{prefix}-{}-{nanos}", std::process::id())
}

// ── 平台 shell 夹具（Windows 兼容）──────────────────────────────────────────
//
// 原写法硬编码 `sh`（`sh -c pwd` / `sh -c "exit 0"` / 交互式 `sh`）：Windows 没有 `sh.exe`
// （Git Bash 的 sh 不在默认 PATH），`job_start*` / `terminal_open*` 会在 spawn 阶段失败。
// 平台分支：Unix `sh -c <cmd>`，Windows `cmd /C <cmd>`。

/// 平台 shell 程序名。
fn platform_shell() -> String {
    if cfg!(windows) {
        "cmd".to_string()
    } else {
        "sh".to_string()
    }
}

/// 「执行单条命令」的平台参数：Unix `-c <cmd>`，Windows `/C <cmd>`。
fn platform_shell_exec(command: &str) -> Vec<String> {
    if cfg!(windows) {
        vec!["/C".to_string(), command.to_string()]
    } else {
        vec!["-c".to_string(), command.to_string()]
    }
}

/// 打印当前工作目录的 shell 命令：Unix `pwd`；Windows `echo %CD%`。
///
/// 为什么 Windows 选 `echo %CD%` 而不是裸 `cd`：
/// - 产品 `terminal_open` / `job_start` 用的都是**管道 stdio**（`terminal.rs:414-416` 的
///   `Stdio::piped()`，不是 PTY），子进程是非交互模式的 cmd：无提示符、但会先打启动 banner；
/// - 裸 `cd` 的"无参即打印"是 cmd 的内建行为，输出与命令回显/提示符混在一起时更难界定边界；
/// - `%CD%` 是 cmd 维护的动态变量（每条命令行重新展开），`echo %CD%` 在 `cmd /C`（job）与
///   管道 stdin（terminal）两种上下文都只输出一行纯路径 → 同一 helper 两处复用、断言最稳。
fn platform_print_cwd() -> &'static str {
    if cfg!(windows) {
        "echo %CD%"
    } else {
        "pwd"
    }
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

/// Windows 路径形态归一化核心（平台参数化纯函数，供平台包装与 Linux 形态测试复用）：
/// `windows_forms=true` 时执行以下转换（否则恒等）：
/// - MSYS/Cygwin 盘符根 `/x/...` → `X:\...`（斜杠转反斜杠，盘符大写）；
/// - 剥 Win32 扩展长度前缀 `\\?\`（含 `\\?\UNC\` → `\\share`）；
/// - 统一小写（Windows 路径比较大小写不敏感）。
fn normalize_path_forms(s: &str, windows_forms: bool) -> String {
    if !windows_forms {
        return s.to_string();
    }
    let mut s = s.to_string();
    let bytes = s.as_bytes();
    // MSYS 根形态：/x/...（x 为盘符字母）。Unix 绝对路径如 /home/... 的第三字符
    // 不是 '/'，不会误命中。
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[2] == b'/' && bytes[1].is_ascii_alphabetic() {
        let drive = (bytes[1] as char).to_ascii_uppercase();
        s = format!("{drive}:{}", s[2..].replace('/', "\\"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        s = format!(r"\\{rest}");
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        s = rest.to_string();
    }
    s.to_lowercase()
}

/// Windows 路径形态归一化（跨形态断言比较辅助；非 Windows 恒等）。
///
/// 背景：CI windows-latest 上子进程 `pwd` 输出是 MSYS 根形态（`/c/Users/...`），
/// 而会话 workspace 是 `make_temp_dir`/canonicalize 后的 Win32 扩展形态
/// （`\\?\C:\Users\...`）——两者指向同一目录，直接字符串比较必然形态错配假红。
/// 处理：先 canonicalize（失败原样透传——MSYS 形态在 Win32 API 下不可解析）；
/// Windows 下对**多行输出逐行**归一化（MSYS 根形态只作用于每行行首，terminal
/// 输出中的 pwd 行可能在多行文本中部），Linux 恒等。
fn normalize_path_for_compare(raw: &str) -> String {
    normalize_path_for_compare_with(raw, cfg!(windows))
}

/// [`normalize_path_for_compare`] 的参数化核心（`windows_forms=true` 时按 Windows 形态归一）。
///
/// 参数化的唯一目的是**让 Windows 形态能在 Linux 上镜像测试**（与 `normalize_path_forms`
/// 同一手法）：normalize 本身仍然只有这一份实现，不另造第二套。
fn normalize_path_for_compare_with(raw: &str, windows_forms: bool) -> String {
    let trimmed = raw.trim();
    let canonical = canonicalize_lossy(trimmed);
    if !windows_forms {
        // 非 Windows：`normalize_path_forms(_, false)` 是恒等变换，直接返回 canonical 结果。
        return canonical;
    }
    canonical
        .lines()
        .map(|line| normalize_path_forms(line.trim(), true))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 路径断言辅助：双方经 `normalize_path_for_compare` 归一化后比较
/// （Windows 形态统一 + 大小写不敏感；Linux 恒等走 canonicalize 后字符串相等）。
fn assert_paths_equal_for_platform(actual: &str, expected: &str, message: &str) {
    let norm_actual = normalize_path_for_compare(actual);
    let norm_expected = normalize_path_for_compare(expected);
    assert_eq!(
        norm_actual, norm_expected,
        "{message}\nactual={actual}\nexpected={expected}"
    );
}

/// 路径包含断言辅助：`output` 归一化后必须包含归一化的 `expected` 路径
/// （terminal 输出含换行/回显等额外文本，故用包含而非相等）。
fn assert_output_contains_path_for_platform(output: &str, expected: &str, message: &str) {
    let norm_output = normalize_path_for_compare(output);
    let norm_expected = normalize_path_for_compare(expected);
    assert!(
        norm_output.contains(&norm_expected),
        "{message}\noutput={output}\nexpected={expected}"
    );
}

/// 有界轮询取回 job 输出，直到非空或预算耗尽（返回最后一次结果）。
///
/// **原断言错在哪**：`job_output(wait=true)` 只等待**进程退出**（`jobs.rs::job_output` 的
/// `session.update_status()` 在 `try_wait` 报退出的那一轮就把状态置为 exited），而
/// stdout/stderr 是由 `job_start*` 起的独立 drain 线程异步写进 RingBuffer 的
/// （`jobs.rs` 的 `job-stdout-drain`/`job-stderr-drain`）。进程退出瞬间 drain 线程可能还没把
/// 最后一批字节搬进缓冲区，于是这一次读取拿到空字符串。原用例把这一次读取当作"最终输出"
/// 直接做路径相等比较 → 偶发假红（本地 20 连跑 8 次失败，actual="" ，见 task-10 诊断）。
///
/// **新断言为什么仍能抓住回归**：路径相等判定原封不动；空输出**不**视为通过——只在预算内
/// 轮询到非空再比较，预算耗尽仍为空则照旧让用例失败。若"cwd 缺省锚定会话 workspace"回归成
/// 锚 current_dir，读到的是非空的 current_dir 路径，断言立即失败，鉴别力不降。
fn job_output_until_nonempty(job_id: &str, budget: Duration) -> JobOutputResult {
    let deadline = Instant::now() + budget;
    loop {
        let out = job_output(JobOutputArgs {
            job_id: job_id.to_string(),
            wait: Some(true),
            timeout_ms: Some(8_000),
            offset: Some(0),
        })
        .expect("job_output should succeed");
        if !out.output.trim().is_empty() || Instant::now() >= deadline {
            return out;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// 有界轮询 terminal 输出，直到（平台归一化后）含期望路径，或预算耗尽；返回最后一次累计输出。
///
/// **Windows 坑 1（时序）**：产品终端是**管道 stdio**（非 PTY），`cmd` 启动时会先输出
/// `Microsoft Windows [Version ...]` 启动 banner（windows-latest 实测如此），命令的输出要晚一步
/// 才进环形缓冲。原先「send 之后立刻单次 `terminal_read` 就断言」在 Windows 上只拿到 banner，
/// 输出里根本没有路径 —— 这是本用例 Windows 假红的直接原因（banner 与路径谁先到是竞态）。
/// 故改为按 50ms 粒度轮询累计输出（`offset=0` 每次取全量），命中即返回，否则超时返回最后一次
/// 输出（供断言打印全貌）。
///
/// **Windows 坑 2（形态）**：期望值来自 `canonicalize()`，Windows 上是 verbatim 前缀
/// `\\?\C:\...`，而 shell 打印的是 `C:\...`；两侧一律走本文件既有的
/// [`normalize_path_for_compare`]（剥 `\\?\` + 大小写归一）再比，不另造第二套规范化。
fn terminal_read_until_contains_path(
    terminal_id: &str,
    expected_path: &str,
    budget: Duration,
) -> String {
    let deadline = Instant::now() + budget;
    let norm_expected = normalize_path_for_compare(expected_path);
    loop {
        let read = terminal_read(TerminalReadArgs {
            terminal_id: terminal_id.to_string(),
            timeout_ms: Some(500),
            offset: Some(0),
        })
        .expect("terminal_read should succeed");
        let output = read.output;
        if normalize_path_for_compare(&output).contains(&norm_expected) {
            return output;
        }
        if Instant::now() >= deadline {
            return output;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Windows 形态转换逻辑的 Linux 可测镜像（复现 CI 实际值，见 ci_final.log）：
/// MSYS 根形态与 `\\?\` 扩展形态必须归一为同一字形，避免 windows-latest 形态假红。
#[test]
fn normalize_path_forms_windows_shapes_are_testable_on_linux() {
    // CI 实测对：patch4_job_some_ws_none_cwd_must_default_to_ws 的 left（MSYS pwd）
    // 与 right（canonicalize 后 \\?\ 形态）归一后必须相等。
    let msys = r"/c/Users/runneradmin/AppData/Local/Temp/patch4-job-ws-9172-1791368094212710700";
    let extended = r"\\?\C:\Users\runneradmin\AppData\Local\Temp\patch4-job-ws-9172-1791368094212710700";
    assert_eq!(
        normalize_path_forms(msys, true),
        normalize_path_forms(extended, true),
        "MSYS 根形态与 \\?\\ 扩展形态必须归一为同一字形"
    );
    // 常规 Win32 形态（无前缀、盘符大写）也与前两者一致。
    let plain = r"C:\Users\runneradmin\AppData\Local\Temp\patch4-job-ws-9172-1791368094212710700";
    assert_eq!(normalize_path_forms(plain, true), normalize_path_forms(msys, true));
    // UNC 扩展形态 → 常规 UNC。
    assert_eq!(
        normalize_path_forms(r"\\?\UNC\server\share\dir\file", true),
        r"\\server\share\dir\file"
    );
    // Unix 绝对路径不得被 MSYS 分支误转换（第三字符非 '/'）。
    assert_eq!(normalize_path_forms("/home/user/pony", true), "/home/user/pony");
}

/// CI windows-latest 实据镜像：terminal 输出是「banner 多行 + `C:\...` 纯盘符路径」，
/// 而期望值来自 `canonicalize()` 的 verbatim 形态 `\\?\C:\...`。
/// 断言：两侧经同一份 `normalize_path_for_compare_with(_, true)` 后，输出确实「包含」期望路径。
#[test]
fn normalize_path_for_compare_handles_banner_plus_verbatim_expected() {
    let expected = r"\\?\C:\Users\runneradmin\AppData\Local\Temp\red-phase-ws-3152-1792995962970334900";
    // Windows 上 ConPTY 输出的样子：启动 banner（CRLF）+ 命令打印的裸盘符路径。
    let output = "Microsoft Windows [Version 10.0.26100.33650]\r\n(c) Microsoft Corporation. All rights reserved.\r\n\r\nC:\\Users\\runneradmin\\AppData\\Local\\Temp\\red-phase-ws-3152-1792995962970334900\r\n";

    let norm_expected = normalize_path_for_compare_with(expected, true);
    let norm_output = normalize_path_for_compare_with(output, true);
    assert!(
        norm_output.contains(&norm_expected),
        "verbatim 前缀期望值必须与 banner 多行输出中的裸盘符路径归一后命中\nnorm_output={norm_output:?}\nnorm_expected={norm_expected:?}"
    );
    // 反例：期望路径不存在时不得命中（防止 contains 退化为恒真）。
    assert!(
        !norm_output.contains(&normalize_path_for_compare_with(
            r"\\?\C:\Users\runneradmin\AppData\Local\Temp\other-ws",
            true
        )),
        "不存在的路径不得命中"
    );
    // 恒等分支。
    assert_eq!(normalize_path_forms("/tmp/a/b", false), "/tmp/a/b");
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
    // TASK-2-WIRE 完成：task-2 已给 job_start 接入会话 workspace root 入参，
    // 本用例按约定走新签名 job_start_with_workspace_root(..., Some(&ws)) 转绿。
    let ws = make_temp_dir(&unique_tag("red-phase-ws"));
    let cwd = current_dir_canonical();
    assert_ne!(ws, cwd, "前置条件失效：会话 workspace 与 cwd 重合，红相无区分度");

    let started = job_start_with_workspace_root(
        JobStartArgs {
            command: platform_shell(),
            args: Some(platform_shell_exec(platform_print_cwd())),
            cwd: None,
            timeout_ms: Some(10_000),
        },
        Some(ws.as_path()),
    )
    .expect("job_start_with_workspace_root should succeed");
    // 有界轮询：drain 线程异步落缓冲，进程刚退出时首读可能为空（原断言假红根因）。
    let out = job_output_until_nonempty(&started.job_id, Duration::from_secs(5));
    let _ = job_kill(JobKillArgs {
        job_id: started.job_id,
        reason: Some("cleanup".to_string()),
    });
    let actual = out.output.trim().to_string();
    let _ = std::fs::remove_dir_all(&ws);
    // 归一化比较：Windows 下 pwd 为 MSYS 形态（/c/...）、ws 为 \\?\ 形态，
    // 归一后大小写不敏感相等；Linux 恒等。
    assert_paths_equal_for_platform(
        &actual,
        &ws.display().to_string(),
        "job cwd 缺省必须返回会话 workspace root，而非 current_dir",
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
        command: platform_shell(),
        args: Some(platform_shell_exec("exit 0")),
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
        command: Some(platform_shell()),
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
    // TASK-2-WIRE 完成：task-2 已给 terminal_open 接入会话 workspace root 入参，
    // 本用例按约定走新签名 terminal_open_with_workspace_root(..., Some(&ws)) 转绿。
    let ws = make_temp_dir(&unique_tag("red-phase-ws"));
    let cwd = current_dir_canonical();
    assert_ne!(ws, cwd, "前置条件失效：会话 workspace 与 cwd 重合，红相无区分度");

    let opened = terminal_open_with_workspace_root(
        TerminalOpenArgs {
            command: Some(platform_shell()),
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
        // 原写法 `pwd`：Windows 上打印当前目录用 `echo %CD%`（理由见 platform_print_cwd）。
        input: format!("{}\n", platform_print_cwd()),
    });
    assert!(send_ok.is_ok(), "terminal_send should succeed: {send_ok:?}");
    // 有界等待（5s 预算）：Windows 上首读只会拿到 shell banner，命令输出要晚一步才进缓冲。
    let output = terminal_read_until_contains_path(
        &opened.terminal_id,
        &ws.display().to_string(),
        Duration::from_secs(5),
    );
    let _ = terminal_close(TerminalCloseArgs {
        terminal_id: opened.terminal_id,
        force: Some(true),
    });
    let _ = std::fs::remove_dir_all(&ws);
    // 归一化包含比较（断言未放宽）：Windows 下 shell 打印 `C:\...`、ws 为 `\\?\` verbatim
    // 形态，两侧都过既有 normalize_path_for_compare 后比较。
    assert_output_contains_path_for_platform(
        &output,
        &ws.display().to_string(),
        "terminal cwd 缺省必须为会话 workspace root",
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
            command: platform_shell(),
            args: Some(platform_shell_exec(platform_print_cwd())),
            cwd: None,
            timeout_ms: Some(10_000),
        },
        Some(ws.as_path()),
    )
    .expect("job_start_with_workspace_root should succeed");
    // 有界轮询：drain 线程异步落缓冲，进程刚退出时首读可能为空（原断言假红根因）。
    let out = job_output_until_nonempty(&started.job_id, Duration::from_secs(5));
    let _ = job_kill(JobKillArgs {
        job_id: started.job_id,
        reason: Some("cleanup".to_string()),
    });
    let actual = out.output.trim().to_string();
    let _ = std::fs::remove_dir_all(&ws);
    // 归一化比较：Windows 下 pwd 为 MSYS 形态（/c/...）、ws 为 \\?\ 形态；
    // 实现已生效（job 落在会话 ws），仅形态错配，归一后相等。
    assert_paths_equal_for_platform(
        &actual,
        &ws.display().to_string(),
        "Some(ws)+cwd缺省必须返回 ws（会话锚定）",
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
            command: platform_shell(),
            args: Some(platform_shell_exec("exit 0")),
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
            command: Some(platform_shell()),
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
        // 原写法 `pwd`：Windows 上打印当前目录用 `echo %CD%`（理由见 platform_print_cwd）。
        input: format!("{}\n", platform_print_cwd()),
    });
    assert!(send_ok.is_ok(), "terminal_send should succeed: {send_ok:?}");
    // 有界等待（5s 预算）：Windows 上首读只会拿到 shell banner，命令输出要晚一步才进缓冲。
    let output = terminal_read_until_contains_path(
        &opened.terminal_id,
        &ws.display().to_string(),
        Duration::from_secs(5),
    );
    let _ = terminal_close(TerminalCloseArgs {
        terminal_id: opened.terminal_id,
        force: Some(true),
    });
    let _ = std::fs::remove_dir_all(&ws);
    // 归一化包含比较（断言未放宽）：Windows 下 shell 打印 `C:\...`、ws 为 `\\?\` verbatim
    // 形态，两侧都过既有 normalize_path_for_compare 后比较。
    assert_output_contains_path_for_platform(
        &output,
        &ws.display().to_string(),
        "Some(ws)+cwd缺省必须为 ws",
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
            command: Some(platform_shell()),
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
        command: platform_shell(),
        args: Some(platform_shell_exec("exit 0")),
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
