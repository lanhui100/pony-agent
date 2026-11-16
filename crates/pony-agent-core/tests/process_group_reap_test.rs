//! B205 验收（组 A）：超时 / 取消等**终结路径返回后**，受管命令的**整个进程组**必须清空，
//! 不得把孙进程泄漏成 PPID=1 孤儿。
//!
//! ## 事故动机（2026-11-13）
//! `.dev-team/review-tool-timeout.json` 里一条 CONCURRENCY 规则的 `counterexample_cmd` 用
//! `for i in $(seq 1 $(nproc)); do (...) & done; ...` 造后台作业，清理语句排在链末且无
//! `trap 'kill 0' EXIT`，结果 40 个 `bash -c 'while :; do :; done'` 变成 PPID=1 孤儿跑了
//! 2 天 16 小时（load 204 / swap 31G）。同病灶在本仓的代码面是：受管子进程已是进程组长
//! （`process.rs` 的 `command.process_group(0)`），但**终结路径**只保证「我被要求杀的时候」
//! 收组，不保证「我观察到受管命令已结束」时收组 —— 后者派生的孙进程会逃逸成孤儿。
//!
//! ## 判据（禁止「sleep 一段时间后主观认为已清理」）
//! - 主判据：扫 `/proc/<pid>/stat` 的 `pgrp` 字段，**非僵尸（state != 'Z'）**成员必须为 0；
//!   轮询有有限上限（[`B205_REAP_TIMEOUT`]），每次失败点名存活 pid 并附 `kill(-pgid,0)` 证据。
//! - 辅助判据：`kill(-pgid, 0)` 的返回码与 errno 一并打进 panic 文案（僵尸尚未被 init 收尸时
//!   该调用仍可能成功，因此它只作证据，不作唯一判据）。
//! - **反空跑（不许「断言恒成立」）**：每条用例都先断言「组长 + 两个孙进程此刻确实同属
//!   `pgid == 受管子进程 pid` 这个进程组」，用正值证明 pgid 推导正确；否则「组内无存活成员」
//!   这类断言只要 pgid 猜错就永远为真 —— 那是假绿。
//!
//! ## 正控（control）为什么必须有
//! 若只有「孙进程泄漏」用例，红相时分不清是「实现有缺陷」还是「断言/环境本身坏了」。
//! 因此每个终结路径都配一条 `_while_shell_alive` 正控：命令本体仍在前台等待（shell 存活），
//! 该形态下现有 `killpg` 已能收组 —— 正控必须绿，泄漏用例必须红，二者共同证明判据有效。
//!
//! ## 「受管命令自行退出」为什么要用哨兵文件，而不是裸 `sleep 120 & ... &`
//! 事故形态是「shell 自己结束了，后台孙进程还活着」。若直接让 shell 立刻退出，测试无法确定
//! 「何时该观察孙进程」，而**正确的修复**完全可以在「观察到受管命令退出」的瞬间就收组 ——
//! 那样任何「退出后仍须存活」的前置断言都会把好实现判红（假红）。因此让 shell 轮询一个
//! 哨兵文件：测试先在 shell 存活期取全标记与组归属正值，再落哨兵让 shell **按自己的节奏**
//! 正常退出，然后再走终结路径并断言组清空。两种合法实现（退出时即收组 / 终结路径时收组）
//! 都能通过。
//!
//! ## 安全边界（事故后红线）
//! - 只用 `sleep` 这类**可被信号终止**的进程，**绝不**写 `while :; do :; done` 之类 CPU 死循环。
//! - 不写 `&`-fork 压测脚本，不按名字杀进程（禁止 `pkill`/`killall`）。
//! - 每个用例自带 [`B205GroupGuard`]：Drop 时按 `/proc` 中 `pgrp == 本组 pgid` 精确回收本用例
//!   创建的成员，并 `shutdown` 本用例的 session、删除哨兵文件；panic 路径同样回收，不留孤儿。
//!
//! ## 分权约束
//! 本文件是 executor **只读**的外部黑盒验收（integration test / 独立 crate）：只经
//! `ProcessBackend` 公开 API 操作，不读私有字段、不吞异常、不做永真断言。

#![cfg(unix)]

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pony_agent_core::agent::process::ProcessManager;
use pony_agent_core::agent::tool_runtime::{
    ProcessBackend, ProcessStartRequest, ProcessState, SandboxRequest,
};

/// 工单标记：本轮 backlog 编号。
const B205_TICKET: &str = "B205";

/// 受管进程「自成进程组长」（`process_group(0)`）时，pgid 等于它自己的 pid。
const B205_PGID_IS_CHILD_PID: &str = "受管子进程必须是进程组长（pgid == 子进程 pid）";

/// 「等到标记全部出现在 stdout」的上限（命令启动阶段，远小于 sleep 时长）。
const B205_MARKER_TIMEOUT: Duration = Duration::from_secs(15);

/// 终结路径返回后，等待进程组清空的**有限**上限。超时即失败并点名存活 pid。
const B205_REAP_TIMEOUT: Duration = Duration::from_secs(8);

/// 轮询间隔（显式条件轮询，不是「睡到自以为清干净」）。
const B205_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// 超时路径的 `kill_after` 时长：足够覆盖「哨兵落地 → 受管命令退出」的窗口。
const B205_KILL_AFTER: Duration = Duration::from_millis(400);

/// 孙进程的存活时长（远大于用例时长；终态由终结路径决定，不靠它自己退出）。
const B205_GRANDCHILD_SLEEP_SECS: u32 = 120;

// ── 受管命令形态 ─────────────────────────────────────────────────────────────────────────────

/// 受管命令脚本：先打印自身 pid（= 进程组长 pid = pgid）与两个孙进程 pid，再派两个孙进程。
///
/// - `sentinel = Some(path)`：打印标记后**轮询该哨兵文件**，文件出现即正常退出 —— 让测试
///   掌握「shell 何时自行退出」的时刻，同时保证取标记时 shell 一定还活着；
/// - `sentinel = None`：`wait` 等孙进程（正控形态：shell 一直活着）。
fn b205_script(sentinel: Option<&str>) -> Vec<String> {
    let mut script = String::new();
    script.push_str("echo \"B205_LEADER=$$\"\n");
    for _ in 0..2 {
        script.push_str("/bin/sleep ");
        script.push_str(&B205_GRANDCHILD_SLEEP_SECS.to_string());
        script.push_str(" &\n");
        script.push_str("echo \"B205_GC=$!\"\n");
    }
    match sentinel {
        Some(path) => {
            script.push_str(&format!("while [ ! -e '{path}' ]; do /bin/sleep 0.05; done\n"));
            script.push_str("exit 0\n");
        }
        None => script.push_str("wait\n"),
    }
    vec!["-c".to_string(), script]
}

/// 唯一哨兵文件路径（落在系统临时目录，绝不在仓库内）。
fn b205_sentinel_path(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after UNIX_EPOCH")
        .as_nanos();
    std::env::temp_dir()
        .join(format!("b205-sentinel-{}-{}-{nanos}", tag, std::process::id()))
        .display()
        .to_string()
}

/// 起一个受管 `bash -c`（经 `ProcessManager`，即真实生产端口），返回 `(session, handle)`。
fn b205_spawn(manager: &ProcessManager, tag: &str, script: Vec<String>) -> (String, String) {
    let session = format!("{}-{}-{}", B205_TICKET.to_lowercase(), tag, std::process::id());
    let handle = manager
        .start(&ProcessStartRequest {
            session_id: session.clone(),
            program: "/bin/bash".to_string(),
            arguments: script,
            cwd: None,
            sandbox: SandboxRequest {
                workspace_root: std::env::temp_dir().display().to_string(),
                allow_network: false,
                environment_allowlist: Vec::new(),
                isolate_environment: true,
            },
        })
        .unwrap_or_else(|error| panic!("{B205_TICKET}: ProcessManager::start 必须成功：{error}"));
    (session, handle)
}

// ── stdout 标记解析 ─────────────────────────────────────────────────────────────────────────

/// 一次 `poll` 的累计视图。
struct B205Polled {
    stdout: String,
    exited: bool,
    exit_code: Option<i32>,
}

/// 从累计 stdout 解析出的进程身份：`leader`（受管子进程 pid，即 pgid）与两个孙进程 pid。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct B205Markers {
    leader: i32,
    grandchildren: [i32; 2],
}

impl B205Markers {
    /// 本用例应有的全体成员（排序后便于与 `/proc` 扫描结果逐值比较）。
    fn expected_members(&self) -> Vec<i32> {
        let mut members = vec![self.leader];
        members.extend(self.grandchildren);
        members.sort_unstable();
        members
    }
}

/// 解析标记；缺失时返回 `None`（由调用方在超时后 panic 并带上原始 stdout）。
fn b205_try_parse_markers(stdout: &str) -> Option<B205Markers> {
    let mut leader = None;
    let mut grandchildren = Vec::new();
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("B205_LEADER=") {
            leader = rest.trim().parse::<i32>().ok().or(leader);
        } else if let Some(rest) = line.strip_prefix("B205_GC=") {
            if let Ok(pid) = rest.trim().parse::<i32>() {
                grandchildren.push(pid);
            }
        }
    }
    if grandchildren.len() < 2 {
        return None;
    }
    Some(B205Markers {
        leader: leader?,
        grandchildren: [grandchildren[0], grandchildren[1]],
    })
}

/// 轮询 `poll` 直到 `ready` 成立；有界等待，超时 panic。
fn b205_poll_until(
    manager: &ProcessManager,
    session: &str,
    handle: &str,
    timeout: Duration,
    what: &str,
    mut ready: impl FnMut(&B205Polled) -> bool,
) -> B205Polled {
    let deadline = Instant::now() + timeout;
    let mut acc = String::new();
    let mut exited = false;
    let mut exit_code = None;
    loop {
        let result = manager
            .poll(session, handle)
            .unwrap_or_else(|error| panic!("{B205_TICKET}: poll 必须成功（{what}）：{error}"));
        acc.push_str(&result.stdout);
        if result.state == ProcessState::Exited {
            exited = true;
            exit_code = result.exit_code;
        }
        let polled = B205Polled {
            stdout: acc.clone(),
            exited,
            exit_code,
        };
        if ready(&polled) {
            return polled;
        }
        assert!(
            Instant::now() < deadline,
            "{B205_TICKET}: 等待超时（{what}）；已累计 stdout={:?}、exited={exited}、exit_code={exit_code:?}",
            polled.stdout
        );
        std::thread::sleep(B205_POLL_INTERVAL);
    }
}

/// 读到全部标记即返回（不要求 shell 退出）。
fn b205_read_markers(
    manager: &ProcessManager,
    session: &str,
    handle: &str,
    what: &str,
) -> B205Markers {
    let polled = b205_poll_until(manager, session, handle, B205_MARKER_TIMEOUT, what, |polled| {
        b205_try_parse_markers(&polled.stdout).is_some()
    });
    b205_try_parse_markers(&polled.stdout).unwrap_or_else(|| {
        panic!(
            "{B205_TICKET}: 标记解析失败（{what}）；stdout={:?}",
            polled.stdout
        )
    })
}

// ── /proc 探针（进程组是否真的清空）──────────────────────────────────────────────────────────

/// `/proc/<pid>/stat` 的 `(pgrp, state)`。
fn b205_proc_pgrp_state(pid: i32) -> Option<(i32, char)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // comm 字段可含空格与括号，因此以最后一个 ')' 为界，其后依次是 state / ppid / pgrp。
    let tail = &stat[stat.rfind(')')? + 1..];
    let mut fields = tail.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let _ppid = fields.next()?;
    let pgrp = fields.next()?.parse::<i32>().ok()?;
    Some((pgrp, state))
}

/// 扫描 `/proc`，返回仍处于 `pgid` 进程组内的**非僵尸** pid（升序）。
///
/// 僵尸（state == 'Z'）不算存活：它已无 CPU/内存占用，只等父进程（或被 init 收养后由 init）
/// 收尸；把它计入会造成「已经杀干净却判失败」的假红相。
fn b205_live_group_members(pgid: i32) -> Vec<i32> {
    let mut members = Vec::new();
    let entries = match std::fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(error) => {
            panic!("{B205_TICKET}: 无法读取 /proc（探针不可用即失败，不得静默放行）：{error}")
        }
    };
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        if let Some((pgrp, state)) = b205_proc_pgrp_state(pid) {
            if pgrp == pgid && state != 'Z' {
                members.push(pid);
            }
        }
    }
    members.sort_unstable();
    members
}

/// `kill(pid, 0)` 是否报告「进程存在」（仅作证据，不作唯一判据）。
fn b205_pid_exists(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

/// `kill(-pgid, 0)` 的证据串（errno 说明进程组是否还有成员）。
fn b205_group_probe_evidence(pgid: i32) -> String {
    let rc = unsafe { libc::kill(-pgid, 0) };
    let error = std::io::Error::last_os_error();
    format!(
        "kill(-{pgid},0)={rc}, errno={:?} ({error})",
        error.raw_os_error().unwrap_or(0)
    )
}

/// 有界轮询直到进程组清空；超时 panic 并点名存活成员 + 辅助证据。
fn b205_wait_group_empty(pgid: i32, timeout: Duration, what: &str) {
    let deadline = Instant::now() + timeout;
    loop {
        let members = b205_live_group_members(pgid);
        if members.is_empty() {
            return;
        }
        let evidence = b205_group_probe_evidence(pgid);
        assert!(
            Instant::now() < deadline,
            "{B205_TICKET}: {what}：进程组 pgid={pgid} 在 {timeout:?} 内未清空，仍存活的成员 pid={members:?}（{evidence}）\
             —— 这些进程的父进程已退出，它们就是 PPID=1 孤儿，正是 2026-11-13 事故的病灶"
        );
        std::thread::sleep(B205_POLL_INTERVAL);
    }
}

// ── 用例级回收守卫 ─────────────────────────────────────────────────────────────────────────

/// 谁创建谁清理：Drop 时精确回收本用例的进程组成员 + `shutdown` 本用例的 session + 删哨兵。
///
/// 只按 `/proc` 里 `pgrp == self.pgid` 的成员逐个 `SIGKILL`（不整组 `killpg`、不按名字杀），
/// 避免 pgid 被复用时误杀无关进程；不使用 `while :; do :; done` 之类不可终止的负载。
struct B205GroupGuard {
    manager: ProcessManager,
    session: String,
    pgid: Option<i32>,
    known_pids: Vec<i32>,
    sentinel: Option<String>,
}

impl B205GroupGuard {
    fn new(manager: &ProcessManager, session: &str, sentinel: Option<String>) -> Self {
        Self {
            manager: manager.clone(),
            session: session.to_string(),
            pgid: None,
            known_pids: Vec::new(),
            sentinel,
        }
    }

    fn track(&mut self, markers: B205Markers) {
        self.pgid = Some(markers.leader);
        self.known_pids = markers.grandchildren.to_vec();
    }
}

impl Drop for B205GroupGuard {
    fn drop(&mut self) {
        if let Some(pgid) = self.pgid {
            for pid in b205_live_group_members(pgid) {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
        for pid in &self.known_pids {
            // 仍属本组的 pid 才动手（pid 复用防护）。
            if let Some((pgrp, _)) = b205_proc_pgrp_state(*pid) {
                if Some(pgrp) == self.pgid {
                    unsafe {
                        libc::kill(*pid, libc::SIGKILL);
                    }
                }
            }
        }
        if let Some(sentinel) = self.sentinel.as_deref() {
            let _ = std::fs::remove_file(sentinel);
        }
        // 管理器的 session 级兜底回收（失败不影响其它用例）。
        self.manager.shutdown(&self.session);
    }
}

/// 反空跑前缀：断言「组长 + 两个孙进程」此刻确实同属 `pgid == leader` 这个进程组。
///
/// 必须在使用这条判据的**最早**时刻调用（命令刚打印完标记、shell 仍在自己掌控中），
/// 它同时证明三件事：① 两个孙进程真的被 fork 出来了；② pgid 推导正确（pgid == 受管子进程 pid）；
/// ③ 后续「组内无存活成员」不是恒真断言。
///
/// 用**子集**判定而非等值：哨兵形态下 shell 每 50ms fork 一个短命 `/bin/sleep 0.05`，
/// 组内瞬时成员可能多出一个；这不影响判据（多出的短命进程会自行退出，且同样被终结路径收组）。
fn b205_assert_group_intact(markers: B205Markers, what: &str) {
    let live = b205_live_group_members(markers.leader);
    let expected = markers.expected_members();
    let missing: Vec<i32> = expected
        .iter()
        .copied()
        .filter(|pid| !live.contains(pid))
        .collect();
    assert!(
        missing.is_empty(),
        "{B205_TICKET}: 前置失败（{what}）—— 进程组 pgid={} 必须含 组长+两个孙进程 {expected:?}\
         （{B205_PGID_IS_CHILD_PID}），缺失 {missing:?}；实际存活成员={live:?}",
        markers.leader
    );
}

// ── 用例 1（红相）：超时路径 · 受管命令自行退出后，孙进程必须仍被收组 ─────────────────────────

/// 事故同源形态：受管 shell 派两个孙进程（`/bin/sleep 120`），随后**自行正常退出（exit 0）**，
/// 孙进程仍在跑。此后超时安全网 `kill_after` 开火（`process.rs` 里超时/取消统一走
/// `kill_if_exists → kill_inner`）。契约要求：终结路径返回后整个进程组必须清空。
///
/// 现状（红相）：`kill_inner` 在 `already_exited` 分支只做「关闭 stdin + 删条目」直接 `Ok(())`，
/// 非 Windows 侧**不再** `killpg`（对照 Windows 侧同一分支仍 `job.terminate(1)` 收整个 Job），
/// 于是两个孙进程逃逸成 PPID=1 孤儿。
#[test]
fn b205_timeout_path_reaps_group_after_shell_exit() {
    let manager = ProcessManager::new();
    let sentinel = b205_sentinel_path("timeout");
    let (session, handle) = b205_spawn(&manager, "timeout-exited", b205_script(Some(&sentinel)));
    let mut guard = B205GroupGuard::new(&manager, &session, Some(sentinel.clone()));

    // shell 仍在轮询哨兵：此刻取标记并做「组内成员完整」正值断言。
    let markers = b205_read_markers(&manager, &session, &handle, "读取超时用例的进程身份标记");
    guard.track(markers);
    b205_assert_group_intact(markers, "受管 shell 存活期");

    // Act ①：落哨兵，让受管 shell **按自己的节奏**正常退出（事故形态：工具侧看到「命令成功结束」）。
    std::fs::write(&sentinel, b"go")
        .unwrap_or_else(|error| panic!("{B205_TICKET}: 写哨兵文件必须成功（{sentinel}）：{error}"));
    let exited = b205_poll_until(
        &manager,
        &session,
        &handle,
        B205_MARKER_TIMEOUT,
        "等待受管 shell 自行退出",
        |polled| polled.exited,
    );
    assert_eq!(
        exited.exit_code,
        Some(0),
        "{B205_TICKET}: 前置失败 —— 受管 shell 应以 0 正常退出（事故形态：工具侧看到「命令已成功结束」）"
    );

    // 证据（不作断言）：此刻孙进程是否仍存活。实现若选择「一观察到受管命令退出就收组」，
    // 这里会是 0 个存活者 —— 那是允许的合法实现，不应判红。
    let survivors: Vec<i32> = markers
        .grandchildren
        .iter()
        .copied()
        .filter(|pid| b205_pid_exists(*pid))
        .collect();
    eprintln!(
        "{B205_TICKET}: 受管 shell 退出后仍存活的孙进程 = {survivors:?}（空 = 实现已在退出时即收组）"
    );

    // Act ②：走真实超时路径（`kill_after` 安全网）。
    manager.kill_after(&session, &handle, B205_KILL_AFTER);

    // Assert：超时终结路径开火后，整个进程组必须清空。
    b205_wait_group_empty(
        markers.leader,
        B205_REAP_TIMEOUT,
        "超时终结路径返回后进程组未清空（孙进程泄漏）",
    );
    for pid in markers.grandchildren {
        assert!(
            !b205_pid_exists(pid),
            "{B205_TICKET}: 超时终结后孙进程 pid={pid} 仍存活（孤儿泄漏，事故病灶）"
        );
    }
}

// ── 用例 2（正控）：超时路径 · shell 仍在前台等待时收组 ─────────────────────────────────────

/// 正控：受管 shell 留在前台 `wait`，超时安全网开火时 `killpg` 可直接收掉整组 ——
/// 该形态必须绿，用来证明「进程组清空」这条判据本身有效（不是因为探针坏了才让用例 1 变红）。
#[test]
fn b205_control_timeout_path_reaps_group_while_shell_alive() {
    let manager = ProcessManager::new();
    let (session, handle) = b205_spawn(&manager, "timeout-alive", b205_script(None));
    let mut guard = B205GroupGuard::new(&manager, &session, None);

    let markers = b205_read_markers(&manager, &session, &handle, "读取正控用例的进程身份标记");
    guard.track(markers);
    b205_assert_group_intact(markers, "正控：超时路径开火前");

    // Act：真实超时路径。
    manager.kill_after(&session, &handle, B205_KILL_AFTER);

    // Assert：进程组清空（正控必须绿）。
    b205_wait_group_empty(
        markers.leader,
        B205_REAP_TIMEOUT,
        "正控：超时终结路径返回后进程组未清空",
    );
    for pid in markers.grandchildren {
        assert!(
            !b205_pid_exists(pid),
            "{B205_TICKET}: 正控失败 —— 孙进程 pid={pid} 仍存活"
        );
    }
}

// ── 用例 3（红相）：显式取消路径 · 受管命令自行退出后，孙进程必须仍被收组 ───────────────────

/// 与用例 1 同形态，但改走**显式取消**入口（`ProcessManager::cancel`，即 RUN 的 Stop / 会话
/// shutdown 路径的公开别名）。契约要求：取消路径返回后整个进程组必须清空。
#[test]
fn b205_cancel_path_reaps_group_after_shell_exit() {
    let manager = ProcessManager::new();
    let sentinel = b205_sentinel_path("cancel");
    let (session, handle) = b205_spawn(&manager, "cancel-exited", b205_script(Some(&sentinel)));
    let mut guard = B205GroupGuard::new(&manager, &session, Some(sentinel.clone()));

    let markers = b205_read_markers(&manager, &session, &handle, "读取取消用例的进程身份标记");
    guard.track(markers);
    b205_assert_group_intact(markers, "受管 shell 存活期");

    // Act ①：落哨兵 → 受管 shell 自行正常退出。
    std::fs::write(&sentinel, b"go")
        .unwrap_or_else(|error| panic!("{B205_TICKET}: 写哨兵文件必须成功（{sentinel}）：{error}"));
    let exited = b205_poll_until(
        &manager,
        &session,
        &handle,
        B205_MARKER_TIMEOUT,
        "等待受管 shell 自行退出",
        |polled| polled.exited,
    );
    assert_eq!(
        exited.exit_code,
        Some(0),
        "{B205_TICKET}: 前置失败 —— 受管 shell 应以 0 正常退出（事故形态：工具侧看到「命令已成功结束」）"
    );

    let survivors: Vec<i32> = markers
        .grandchildren
        .iter()
        .copied()
        .filter(|pid| b205_pid_exists(*pid))
        .collect();
    eprintln!(
        "{B205_TICKET}: 受管 shell 退出后仍存活的孙进程 = {survivors:?}（空 = 实现已在退出时即收组）"
    );

    // Act ②：真实显式取消路径。
    manager
        .cancel(&session, &handle)
        .unwrap_or_else(|error| panic!("{B205_TICKET}: cancel 必须成功：{error}"));

    // Assert：取消路径返回后进程组必须清空。
    b205_wait_group_empty(
        markers.leader,
        B205_REAP_TIMEOUT,
        "显式取消路径返回后进程组未清空（孙进程泄漏）",
    );
    for pid in markers.grandchildren {
        assert!(
            !b205_pid_exists(pid),
            "{B205_TICKET}: 取消后孙进程 pid={pid} 仍存活（孤儿泄漏，事故病灶）"
        );
    }
}

// ── 用例 4（正控）：显式取消路径 · shell 仍存活时收组 ───────────────────────────────────────

/// 正控：`wait` 形态下显式取消必须收掉整组（证明取消路径的判据有效）。
#[test]
fn b205_control_cancel_path_reaps_group_while_shell_alive() {
    let manager = ProcessManager::new();
    let (session, handle) = b205_spawn(&manager, "cancel-alive", b205_script(None));
    let mut guard = B205GroupGuard::new(&manager, &session, None);

    let markers = b205_read_markers(
        &manager,
        &session,
        &handle,
        "读取取消正控用例的进程身份标记",
    );
    guard.track(markers);
    b205_assert_group_intact(markers, "正控：取消开火前");

    manager
        .cancel(&session, &handle)
        .unwrap_or_else(|error| panic!("{B205_TICKET}: cancel 必须成功：{error}"));

    b205_wait_group_empty(
        markers.leader,
        B205_REAP_TIMEOUT,
        "正控：显式取消路径返回后进程组未清空",
    );
    for pid in markers.grandchildren {
        assert!(
            !b205_pid_exists(pid),
            "{B205_TICKET}: 正控失败 —— 孙进程 pid={pid} 仍存活"
        );
    }
}
