//! 契约验收测试：Run 超时 promote（真·后台接续）· 契约 v6 · CT-P1..P5 + CT-P7/P7a/P7b
//!
//! 契约真源：`.dev-team/designs/run-timeout-promote.md` §2（CT-P1..P5）＋
//! `.dev-team/contracts/tool-timeout-policy.md` §波次四（CT-P7 / CT-P7a / CT-P7b）。
//! 本文件是**外部黑盒验收**（integration test，独立 crate）：只经 `ToolRouter` 与 job_* 工具族
//! 断言可观测行为，不复制实现结构、不读私有字段、不吞异常、不使用永真断言。
//!
//! ## 覆盖矩阵
//! - CT-P1 → `ct_p1_timeout_promotes_to_background_job`（显式 `=1`）
//! - CT-P2 → `ct_p2_promoted_job_streams_output_after_timeout`（显式 `=1`）
//! - CT-P3 → `ct_p3_job_kill_terminates_promoted_job`（显式 `=1`）
//! - CT-P5 → `ct_p5_kill_after_timer_does_not_fire_on_promoted_job`（显式 `=1`）
//! - CT-P7（默认关闭）→ `ct_p4_promote_default_off_keeps_timeout_error_path`（原 CT-P4 升级：不设 env）
//! - CT-P7a（先 poll 再判超时）→ `ct_p6_already_exited_process_is_not_promoted`（显式 `=1`，负例）
//! - CT-P7b（promoted 输出截断标记）→ `ct_p7_promoted_output_truncation_is_flagged`（显式 `=1`）
//!
//! ## env 串行约束（必读，新增用例必须遵守）
//! v6 起 `PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT` **默认关闭（opt-in）**：依赖 promote 的用例必须显式
//! `=1` 开启，P4 断的则是**默认**（不设 env）。该 env 是**进程级全局**：`cargo test` 默认在同一
//! 测试二进制内多线程并发跑用例，不串行就会互相污染（P4 读到别的用例设的 `1`，或反之）。
//! 故本文件**所有**用例统一持 `GATE_LOCK` 串行执行，env 一律经 `EnvVarGuard`（Drop 复位，
//! panic 安全）。测试二进制之间是独立进程，互不影响；但**本文件内禁止新增不持锁的用例**。
//!
//! ## 真实进程与总时长
//! CT-P1..P7b 必须真的让子进程跨过 deadline（才能验收「到期不杀、转后台继续」），因此依赖本地
//! shell 的 `sleep`（Windows 用 `ping` 计时）真实进程；deadline 统一取 300ms。
//! 其中 `thread::sleep` 仅用于**断言时序不变量**（「1s 后仍存活」「kill 后不再产出」），
//! 不是同步轮询——同步一律用 `job_output(wait:true)` 的条件等待。
//! 总耗时预算 ≈ 10s（CT-P7b 造 3MiB 输出，见该用例注释）。
//!
//! ## 清理约定
//! 谁创建谁清理：每个 promote 出来的 job 都由 `PromotedJobGuard` 在 Drop 中 `job_kill`，
//! 用例 panic 时同样回收，避免残留进程污染后续用例与 CI。
//! 已知 BLOCKING（契约 §波次四 A-3/B009/B012，非本波目标）：promoted 的 `ManagedProcess` 条目与
//! 跨会话归属尚未收口，故测试侧显式 kill，不依赖 session shutdown。

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use pony_agent_core::agent::sandbox::TestSandboxBackend;
use pony_agent_core::agent::tools::{
    job_kill, job_list, job_output, JobKillArgs, JobListArgs, JobOutputArgs, ToolCall, ToolRouter,
};
use serde_json::{json, Value};

/// 串行锁：保护进程级 env 与 job registry 的用例级隔离（原因见文件头）。
static GATE_LOCK: Mutex<()> = Mutex::new(());

fn gate_lock() -> MutexGuard<'static, ()> {
    GATE_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// promote 开关 env（契约 v6 CT-P7：默认关闭，`1`/`true`/`on`/`yes` 开启）。
const PROMOTE_ENV: &str = "PONY_AGENT_RUN_PROMOTE_ON_TIMEOUT";

/// env 守卫：`set_var` + Drop 时 `remove_var`（panic 安全，且用例已持 `GATE_LOCK` 串行）。
struct EnvVarGuard {
    key: &'static str,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: &str) -> Self {
        std::env::set_var(key, value);
        Self { key }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        std::env::remove_var(self.key);
    }
}

/// 契约 v6：promote 改为 opt-in —— 依赖 promote 的用例必须显式开启，Drop 时自动复位。
fn enable_promote() -> EnvVarGuard {
    EnvVarGuard::set(PROMOTE_ENV, "1")
}

/// promoted job 回收守卫：Drop 时尽力 `job_kill`（避免用例失败时残留真实进程树）。
struct PromotedJobGuard {
    job_id: Option<String>,
}

impl PromotedJobGuard {
    fn new(job_id: Option<String>) -> Self {
        Self { job_id }
    }
}

impl Drop for PromotedJobGuard {
    fn drop(&mut self) {
        if let Some(job_id) = self.job_id.take() {
            let _ = job_kill(JobKillArgs {
                job_id,
                reason: Some("test cleanup".to_string()),
            });
        }
    }
}

// ── 平台夹具 ────────────────────────────────────────────────────────────────
// Unix 用 `sleep` / `;` 串联；Windows 无 `sleep.exe` 且 cmd 用 `&` 串联，用 `ping -n N` 计时。

/// 必然超过 300ms deadline 的长命令（P1/P4 用，promote 后由守卫回收）。
#[cfg(windows)]
const LONG_RUNNING_COMMAND: &str = "ping -n 6 127.0.0.1";
#[cfg(not(windows))]
const LONG_RUNNING_COMMAND: &str = "sleep 5";

/// 先产出 A，2s 后产出 B 的命令：用于证明「超时后进程仍活着并继续产出」。
#[cfg(windows)]
const TWO_PHASE_COMMAND: &str = "echo A & ping -n 3 127.0.0.1 >nul & echo B";
#[cfg(not(windows))]
const TWO_PHASE_COMMAND: &str = "echo A; sleep 2; echo B";

/// 2s 后产出 done 的命令：用于证明旧 `kill_after` 定时器不再开火。
#[cfg(windows)]
const DELAYED_DONE_COMMAND: &str = "ping -n 3 127.0.0.1 >nul & echo done";
#[cfg(not(windows))]
const DELAYED_DONE_COMMAND: &str = "sleep 2; echo done";

/// CT-P7b：promote **之后**才产出 >2MiB（promoted RingBuffer 上限 `JOB_BUFFER_CAP = 2MiB`）的命令。
/// 先睡 0.4s 让 300ms deadline 先到期（保证 truncation 发生在 promote 之后的增量轮询里，
/// 而不是首读快照），再灌 3MB 的 'x'。总耗时 <2s。
#[cfg(windows)]
const TRUNCATION_COMMAND: &str =
    "ping -n 2 127.0.0.1 >nul & powershell -NoProfile -Command \"[Console]::Out.Write('x' * 3000000)\"";
#[cfg(not(windows))]
const TRUNCATION_COMMAND: &str = "sleep 0.4; head -c 3000000 /dev/zero | tr '\\0' 'x'";

// ── harness ────────────────────────────────────────────────────────────────

/// 每个用例独立临时 workspace，用例结束即删除（谁创建谁清理）。
fn unique_workspace(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock must be after UNIX_EPOCH")
        .as_nanos();
    std::env::temp_dir().join(format!("run_promote_{}_{}", tag, nanos))
}

/// 经真实 `ToolRouter` 执行 `workspace_run_command`，返回 `(status, 解析后的载荷)`。
/// 输出不是合法 JSON 时 panic —— 禁止静默放行。
fn run_command(command: &str, timeout_ms: u64, tag: &str) -> (String, Value) {
    let workspace = unique_workspace(tag);
    std::fs::create_dir_all(&workspace).expect("create temp workspace");

    let router = ToolRouter::with_workspace_root(workspace.clone())
        .with_sandbox_backend(TestSandboxBackend::available());
    let call = ToolCall {
        name: "workspace_run_command".to_string(),
        arguments: json!({
            "command": command,
            "cwd": workspace.display().to_string(),
            "timeoutMs": timeout_ms,
            "description": "run promote 验收",
        }),
        call_id: Some(format!("call-{}", tag)),
        plan: None,
    };

    let result = router.execute(&call);
    let _ = std::fs::remove_dir_all(&workspace);

    let payload: Value = serde_json::from_str(&result.output).unwrap_or_else(|error| {
        panic!(
            "workspace_run_command 输出必须是合法 JSON（{}）: {}",
            error, result.output
        )
    });
    (result.status, payload)
}

/// 从 promote 结果里取 `jobId`（根级、非空字符串），失败即 panic 并打印完整载荷。
fn promoted_job_id(payload: &Value) -> String {
    let job_id = payload
        .get("jobId")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("CT-P1：promote 结果根级必须含字符串 jobId，payload={payload}"));
    assert!(
        !job_id.trim().is_empty(),
        "CT-P1：promote 结果的 jobId 不得为空串，payload={payload}"
    );
    job_id.to_string()
}

/// 读取 promoted job 的增量输出（`wait` 语义由调用方决定）。
fn read_job(job_id: &str, wait: bool, timeout_ms: u64, offset: usize) -> JobOutputView {
    let result = job_output(JobOutputArgs {
        job_id: job_id.to_string(),
        wait: Some(wait),
        timeout_ms: Some(timeout_ms),
        offset: Some(offset),
    })
    .unwrap_or_else(|error| {
        panic!(
            "job_output 必须能续读 promoted job `{}`（CT-P2：jobId 必须已注册进 job registry）：{}",
            job_id, error
        )
    });
    JobOutputView {
        status: result.status,
        output: result.output,
        cursor: result.cursor,
        exit_code: result.exit_code,
        stdout_truncated: result.stdout_truncated,
        stderr_truncated: result.stderr_truncated,
    }
}

struct JobOutputView {
    status: String,
    output: String,
    cursor: usize,
    exit_code: Option<i32>,
    /// CT-P7b：promoted 输出截断标记（粘性 OR）；`job_start` 自建作业为 `None`。
    stdout_truncated: Option<bool>,
    stderr_truncated: Option<bool>,
}

impl JobOutputView {
    fn is_terminal(&self) -> bool {
        matches!(
            self.status.as_str(),
            "killed" | "exited" | "completed" | "failed"
        )
    }
}

// ── CT-P1 ──────────────────────────────────────────────────────────────────

/// CT-P1：promote **显式开启**（v6 opt-in，`=1`）时，Run 到期返回 **ok**，根级 `timedOut:true` +
/// `promoted:true` + 非空 `jobId`，且不再产生 `error.code=timeout`。
#[test]
fn ct_p1_timeout_promotes_to_background_job() {
    let _gate = gate_lock();
    let _env = enable_promote();

    // Act：300ms deadline 的 5s 长命令必然到期。
    let (status, payload) = run_command(LONG_RUNNING_COMMAND, 300, "p1");
    let guard = PromotedJobGuard::new(payload.get("jobId").and_then(Value::as_str).map(str::to_string));

    // Assert
    assert_eq!(
        status, "ok",
        "CT-P1：promote 开启时到期必须返回 ok（不再是 error），payload={payload}"
    );
    assert_eq!(
        payload.get("ok").and_then(Value::as_bool),
        Some(true),
        "CT-P1：promote 结果 ok 必须为 true，payload={payload}"
    );
    assert_eq!(
        payload.get("timedOut").and_then(Value::as_bool),
        Some(true),
        "CT-P1：promote 结果必须保留 timedOut=true（到期事实不得被抹掉），payload={payload}"
    );
    assert_eq!(
        payload.get("promoted").and_then(Value::as_bool),
        Some(true),
        "CT-P1：promote 结果必须显式带 promoted=true，payload={payload}"
    );
    let job_id = promoted_job_id(&payload);
    assert_ne!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("timeout"),
        "CT-P1：promote 后不得再产生 error.code=timeout，payload={payload}"
    );
    assert!(
        payload.get("error").map(Value::is_null).unwrap_or(true),
        "CT-P1：promote 成功载荷的 error 必须缺失或为 null，payload={payload}"
    );

    // 反证：该 jobId 必须真实存在（否则「promote」只是改了个字段名）。
    let probe = read_job(&job_id, false, 0, 0);
    assert!(
        probe.status == "running" || probe.is_terminal(),
        "CT-P1：promoted job 的 status 必须是运行中或终态，实际 `{}`",
        probe.status
    );

    drop(guard);
}

// ── CT-P2 ──────────────────────────────────────────────────────────────────

/// CT-P2：promoted job 可通过 `job_output` 续读 —— timeout 之后继续产出的内容能读到，
/// 证明进程未被杀。命令 `echo A; sleep 2; echo B`（deadline 300ms）⇒ 先拿 A，续读拿到 B。
#[test]
fn ct_p2_promoted_job_streams_output_after_timeout() {
    let _gate = gate_lock();
    let _env = enable_promote();

    // Arrange + Act：A 在 deadline 前产出，B 在 timeout 之后约 2s 产出。
    let (status, payload) = run_command(TWO_PHASE_COMMAND, 300, "p2");
    let job_id = promoted_job_id(&payload);
    let _guard = PromotedJobGuard::new(Some(job_id.clone()));

    // Assert 1：到期返回 ok 且已带上 timeout 前读到的 A（证明首读快照被保留）。
    assert_eq!(
        status, "ok",
        "CT-P2：promote 开启时到期必须返回 ok，payload={payload}"
    );
    assert!(
        payload
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains('A'),
        "CT-P2：promote 结果必须保留 timeout 前已读到的 stdout（含 A），payload={payload}"
    );

    // Assert 2：续读必须等到 B —— 这是「进程未被杀、继续产出」的可观测定义。
    let tail = read_job(&job_id, true, 5_000, 0);
    assert!(
        tail.output.contains('B'),
        "CT-P2：job_output 必须能读到 timeout 之后的新输出 B（进程不得被杀），\
         实际 status=`{}` output={:?} exitCode={:?}",
        tail.status,
        tail.output,
        tail.exit_code
    );
    assert!(
        tail.is_terminal(),
        "CT-P2：命令自然结束后 job 必须是终态，实际 status=`{}`",
        tail.status
    );
    assert_eq!(
        tail.exit_code,
        Some(0),
        "CT-P2：自然结束的命令 exitCode 必须为 0，实际 {:?}（output={:?}）",
        tail.exit_code,
        tail.output
    );
    assert!(
        tail.cursor > 0,
        "CT-P2：读取游标必须前进（>0），实际 cursor={}",
        tail.cursor
    );
}

// ── CT-P3 ──────────────────────────────────────────────────────────────────

/// CT-P3：`job_kill` 对 promoted job 生效 —— kill 后 `job_output` 显示终态，且进程真的死了
/// （2s 后本该产出的 B 永不出现）。
///
/// 「无残留进程」用行为代理验收：kill 时刻 ≈ 0.3s，B 应在 ≈2s 产出；若 kill 无效，续读一定
/// 能看到 B。进程表级断言（/proc、pid 复用）不具跨平台稳定性，故不采用。
#[test]
fn ct_p3_job_kill_terminates_promoted_job() {
    let _gate = gate_lock();
    let _env = enable_promote();

    // Arrange + Act
    let started = Instant::now();
    let (_status, payload) = run_command(TWO_PHASE_COMMAND, 300, "p3");
    let job_id = promoted_job_id(&payload);
    let _guard = PromotedJobGuard::new(Some(job_id.clone()));

    let kill = job_kill(JobKillArgs {
        job_id: job_id.clone(),
        reason: Some("ct_p3".to_string()),
    })
    .unwrap_or_else(|error| panic!("CT-P3：job_kill 必须接受 promoted job `{job_id}`：{error}"));
    assert!(
        kill.killed,
        "CT-P3：job_kill 对 promoted job 必须返回 killed=true（实际 false，job={job_id}）"
    );

    // Assert 1：kill 后状态进入终态。
    let after_kill = read_job(&job_id, true, 3_000, 0);
    assert!(
        after_kill.is_terminal(),
        "CT-P3：kill 后 job_output 必须显示终态（killed/exited/completed/failed），实际 status=`{}`",
        after_kill.status
    );

    // Assert 2：跨过 B 本应产出的时刻（≈2s）后仍无 B ⇒ 进程确已被终止。
    let elapsed = started.elapsed();
    let until_after_b = Duration::from_millis(2_500).saturating_sub(elapsed);
    if !until_after_b.is_zero() {
        std::thread::sleep(until_after_b);
    }
    let later = read_job(&job_id, false, 0, 0);
    assert!(
        !later.output.contains('B'),
        "CT-P3：kill 后进程不得继续产出（B 不应出现，说明仍有残留进程在跑），\
         实际 status=`{}` output={:?}",
        later.status,
        later.output
    );
    assert!(
        later.is_terminal(),
        "CT-P3：kill 后 status 必须保持终态，实际 `{}`",
        later.status
    );
}

// ── CT-P7（含原 CT-P4 升级）────────────────────────────────────────────────

/// CT-P7：v6 起 promote **默认关闭（opt-in）** —— **不设 env** 时行为与波次一完全一致：
/// `error.code=timeout`、无 `promoted`/`jobId`、进程被清理（无后台作业可续读）。
///
/// 与 `ct_p1/p2/p3/p5` 的区别即契约核心：那四例必须显式 `=1` 才会 promote，本用例断默认态。
#[test]
fn ct_p4_promote_default_off_keeps_timeout_error_path() {
    let _gate = gate_lock();

    // Arrange：默认态自检 —— 本用例不得读到任何「被别的用例设过」的 env（GATE_LOCK 串行保证）。
    assert!(
        std::env::var_os(PROMOTE_ENV).is_none(),
        "CT-P7：默认关闭路径要求 env `{PROMOTE_ENV}` 未被设置；\
         读到值说明本文件的用例串行/复位约束失效"
    );

    // Act
    let (status, payload) = run_command(LONG_RUNNING_COMMAND, 300, "p4");

    // Assert：回到现状错误路径。
    assert_eq!(
        status, "error",
        "CT-P7：promote 关闭时到期必须返回 error（波次一现状），payload={payload}"
    );
    assert_eq!(
        payload
            .get("error")
            .and_then(|error| error.get("code"))
            .and_then(Value::as_str),
        Some("timeout"),
        "CT-P7：promote 关闭时必须保留 error.code=timeout，payload={payload}"
    );
    assert_eq!(
        payload.get("timedOut").and_then(Value::as_bool),
        Some(true),
        "CT-P7：promote 关闭时仍须保留 CT-4 的 timedOut=true 证据，payload={payload}"
    );
    assert!(
        payload.get("jobId").map(Value::is_null).unwrap_or(true),
        "CT-P7：promote 关闭时不得注册/返回任何 jobId，payload={payload}"
    );
    assert_ne!(
        payload.get("promoted").and_then(Value::as_bool),
        Some(true),
        "CT-P7：promote 关闭时不得出现 promoted=true，payload={payload}"
    );

    // 「进程被清理」的可观测形式：job registry 里不得存在仍在运行的该命令作业。
    // （不依赖 jobId —— 恰恰因为关闭路径不该注册任何作业。）
    let jobs = job_list(JobListArgs {})
        .expect("CT-P7：job_list 必须可用")
        .jobs;
    let survivor = jobs
        .iter()
        .find(|job| job.command.contains(LONG_RUNNING_COMMAND) && job.status == "running");
    assert!(
        survivor.is_none(),
        "CT-P7：promote 关闭时到期必须清理进程，不得留有仍运行的作业，\
         实测残留 job={:?}",
        survivor.map(|job| (job.job_id.clone(), job.command.clone(), job.status.clone()))
    );
}

// ── CT-P5 ──────────────────────────────────────────────────────────────────

/// CT-P5：原 `kill_after` 定时器对已 promote 的进程不再开火 —— 超时时刻之后 2×timeout
/// （2×300ms=600ms，本用例取 1s 采样点）进程仍存活；随后能读到尾部输出 `done`。
#[test]
fn ct_p5_kill_after_timer_does_not_fire_on_promoted_job() {
    let _gate = gate_lock();
    let _env = enable_promote();

    // Arrange + Act：`sleep 2; echo done`，deadline 300ms。
    let (status, payload) = run_command(DELAYED_DONE_COMMAND, 300, "p5");
    let job_id = promoted_job_id(&payload);
    let _guard = PromotedJobGuard::new(Some(job_id.clone()));
    assert_eq!(
        status, "ok",
        "CT-P5：promote 开启时到期必须返回 ok，payload={payload}"
    );

    // Assert 1：越过 2×timeout（600ms）的采样点（≈1s，从 promote 返回后计时）仍存活。
    // 这里是**时序不变量抽样**（证明 600ms 时点之后没被杀），不是同步轮询。
    std::thread::sleep(Duration::from_millis(700));
    let alive = read_job(&job_id, false, 0, 0);
    assert_eq!(
        alive.status, "running",
        "CT-P5：2×timeout 之后 promoted 进程必须仍存活（旧 kill_after 定时器不得开火），\
         实际 status=`{}` output={:?}",
        alive.status, alive.output
    );
    assert!(
        !alive.output.contains("done"),
        "CT-P5：采样点（≈1s）时命令尚未产出 done（命令 2s 后才产出），实际 output={:?}",
        alive.output
    );

    // Assert 2：命令自然跑完，续读到尾部输出 done。
    let tail = read_job(&job_id, true, 5_000, 0);
    assert!(
        tail.output.contains("done"),
        "CT-P5：promoted 进程必须跑完并产出 done，实际 status=`{}` output={:?}",
        tail.status,
        tail.output
    );
    assert!(
        tail.is_terminal(),
        "CT-P5：命令结束后 job 必须是终态，实际 status=`{}`",
        tail.status
    );
    assert_eq!(
        tail.exit_code,
        Some(0),
        "CT-P5：自然结束 exitCode 必须为 0，实际 {:?}",
        tail.exit_code
    );
}

// ── CT-P7a ─────────────────────────────────────────────────────────────────

/// CT-P7a：**先 poll 再判超时** —— 进程在极短 deadline 内已自然退出时，必须返回正常成功结果，
/// 不得被报成 promote。命令 `echo hi` + `timeoutMs:1`：poll 循环 20ms 一醒，此时 `echo` 早已退出。
///
/// 本用例在 promote **显式开启**下跑，语义是「已退出优先于到期」：若实现先判 deadline 再看进程，
/// 就会把这条正常命令错报成 `promoted`（这正是 L3 复核 B 路指出的缺陷）。
#[test]
fn ct_p6_already_exited_process_is_not_promoted() {
    let _gate = gate_lock();
    let _env = enable_promote();

    // Act
    let (status, payload) = run_command("echo hi", 1, "p7a");
    // 万一实现违反了 CT-P7a 真把进程 promote 了，也要回收，避免残留进程污染后续用例。
    let _guard =
        PromotedJobGuard::new(payload.get("jobId").and_then(Value::as_str).map(str::to_string));

    // Assert
    assert_eq!(
        status, "ok",
        "CT-P7a：已退出的命令必须以 ok 返回（正常成功结果），payload={payload}"
    );
    assert_ne!(
        payload.get("promoted").and_then(Value::as_bool),
        Some(true),
        "CT-P7a：进程已退出时不得报 promoted=true，payload={payload}"
    );
    assert!(
        payload.get("jobId").map(Value::is_null).unwrap_or(true),
        "CT-P7a：进程已退出时不得注册/返回 jobId，payload={payload}"
    );
    assert_eq!(
        payload.get("timedOut").and_then(Value::as_bool),
        Some(false),
        "CT-P7a：正常成功结果必须带 timedOut=false（不得把到期事实伪造成超时），payload={payload}"
    );
    assert_eq!(
        payload.get("exitCode").and_then(Value::as_i64),
        Some(0),
        "CT-P7a：成功路径必须回显真实 exitCode=0，payload={payload}"
    );
    assert!(
        payload
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .contains("hi"),
        "CT-P7a：命令必须真实执行并保留 stdout，payload={payload}"
    );
    assert!(
        payload.get("error").map(Value::is_null).unwrap_or(true),
        "CT-P7a：成功结果不得带 error 对象，payload={payload}"
    );
}

// ── CT-P7b ─────────────────────────────────────────────────────────────────

/// CT-P7b：promoted 的 RingBuffer（`JOB_BUFFER_CAP = 2MiB`）溢出必须置截断标记并透传到
/// `job_output`（`stdout_truncated = Some(true)`），禁止静默丢字节。
///
/// 命令刻意在 deadline **之后**（≈0.4s）才灌 3MiB，使溢出发生在 promote 后的增量轮询里，
/// 而不是首读快照里。总时长 <6s。
#[test]
fn ct_p7_promoted_output_truncation_is_flagged() {
    let _gate = gate_lock();
    let _env = enable_promote();

    // Act 1：300ms 到期 ⇒ promote（此时子进程还在 sleep，尚未产出大输出）。
    let (status, payload) = run_command(TRUNCATION_COMMAND, 300, "p7b");
    let job_id = promoted_job_id(&payload);
    let _guard = PromotedJobGuard::new(Some(job_id.clone()));
    assert_eq!(
        status, "ok",
        "CT-P7b：promote 必须返回 ok，payload={payload}"
    );

    // Act 2：条件等待到作业终态（job_output(wait:true) 持续轮询并追加增量，无盲 sleep）。
    let tail = read_job(&job_id, true, 6_000, 0);

    // Assert：截断标记必须置位。
    assert!(
        tail.is_terminal(),
        "CT-P7b：3MiB 输出结束后作业必须进入终态，实际 status=`{}`",
        tail.status
    );
    assert_eq!(
        tail.stdout_truncated,
        Some(true),
        "CT-P7b：promoted RingBuffer（2MiB）溢出后 `job_output` 必须报 stdout_truncated=true\
         （禁止静默丢字节）；实际 status=`{}` cursor={} output_len={} flags=(stdout={:?}, stderr={:?})",
        tail.status,
        tail.cursor,
        tail.output.len(),
        tail.stdout_truncated,
        tail.stderr_truncated
    );
    assert!(
        !tail.output.is_empty(),
        "CT-P7b：截断不得导致输出整体为空（必须保留边界内字节），实际 output_len={}",
        tail.output.len()
    );
}
