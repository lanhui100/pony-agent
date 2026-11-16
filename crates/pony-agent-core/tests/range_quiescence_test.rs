//! B206 验收（组 A）：终结路径的**完成条件**从「信号已发送」改为「**进程范围已证明为空**」。
//!
//! ## 工单契约（逐条对应本文件的用例）
//! - **C1** `terminate`/`kill`/`cancel`/`shutdown` 返回 `Ok` ⟺ 范围为空
//!   （`kill(-pgid,0)` 返回 `ESRCH`，或 `/proc` 中无 `pgrp == pgid` 的**非僵尸**成员）。
//! - **C2** 阶梯：`SIGTERM` → `term_grace`(250ms) → `SIGKILL` → `quiesce_deadline`(5s) 仍未空 ⇒
//!   **返回 `Err`**（fail loud，不许谎报 Ok）。
//! - **C3** 直系子进程已 `Exited` 但范围非空时**保留**条目供补刀；范围空了才 `remove`。
//! - **C4** 幂等不变：未知 handle + `kill_if_exists` ⇒ `Ok`；未加 `if_exists` 的 `kill` ⇒ `Err`。
//! - **C5** start-time 身份栅栏（`/proc/<pid>/stat` 第 22 字段不匹配 ⇒ 不发信号）—— 见「覆盖缺口」。
//! - **C6** Windows 语义不变 —— 本文件 `#![cfg(unix)]`，不在本机可验范围。
//!
//! 本文件（组 A）只覆盖 C1/C2/C3/C4 的 **POSIX 黑盒面**；C5/C6 的判据由组 B 的静态门禁与
//! 单元测试承担（见「覆盖缺口」）。
//!
//! ## 事故动机（2026-11-13 孤儿事故的第四道缺口）
//! B205 已经补上了三条终结路径（`kill_after` 超时、`cancel`、`kill` 的 already-exited 分支）的
//! 收组动作，但它们都只保证「**信号发出去了**」。真正的完成条件是「**组里没人了**」：
//! 信号是异步投递的，返回 `Ok` 的那一刻范围完全可能仍有存活成员 —— 调用方（Run 收尾、
//! 会话关闭、静默清理）据此认为「已清理干净」，下一步就去删工作目录 / 释放锁 / 起新进程，
//! 而老进程仍在跑，于是又变成 PPID=1 孤儿。
//!
//! 本文件挖出的**第四条**缺口（主红相，见
//! [`b206_shutdown_reaps_group_after_shell_exit`]）：`ProcessManager::terminate` 的
//! `ExitState::Exited` 分支在非 Windows 上**根本不再发任何信号**（只有 Windows 侧收 Job
//! Object），而 `ProcessManager::shutdown(session)` 走的正是 `terminate()`。于是
//! 「受管 shell 已 `exit 0`、孙进程还活着」时，`shutdown` 直接返回（并把条目移除），
//! 孙进程原地变成孤儿 —— 这是 B205 三条路径之外、当前无人看守的一条泄漏路径。
//!
//! ## 判据（禁止「sleep 一段时间后主观认为已清理」）
//! - 主判据：扫 `/proc/<pid>/stat` 的 `pgrp` 字段，**非僵尸（state != 'Z'）**成员必须为 0；
//!   轮询有有限上限（[`B206_REAP_TIMEOUT`]），失败时点名存活 pid 并附 `kill(-pgid,0)` 证据。
//! - 辅助判据：`kill(-pgid, 0)` 的返回码与 errno 打进 panic 文案（僵尸未收尸时该调用仍可能
//!   成功，因此它只作证据，不作唯一判据）。
//! - **反空跑（不许「断言恒成立」）**：每条用例都先断言「受管 shell（组长）+ 孙进程此刻确实
//!   同属 `pgid == 受管子进程 pid` 这个进程组」。否则「组内无存活成员」这类断言只要 pgid 猜错
//!   就永远为真 —— 那是假绿。
//!
//! ## 为什么「仍然回 `Ok`」能被稳定证伪，而「5s 未静默 ⇒ `Err`」不能
//! 实测（python 探针，`pgid` 内 3 个成员，其中一个 `trap "" TERM`）：
//! `killpg(SIGKILL)` 之后**第一次** `/proc` 扫描（12.4ms）成员数就已经是 0
//! （`elapsed_ms=12.420 samples=1 nonempty_samples=0`）。也就是说「SIGKILL 之后范围仍非空」
//! 的可观测窗口小于一次 `/proc` 扫描：任何「返回后仍非空」的断言都只能靠睡一会儿猜时机 = flaky；
//! 同理，5s 死线下的 `Err` 分支在**本机不可构造**（PID-namespace init 实测被外部 SIGKILL 杀死、
//! cgroup v2 `freeze` 无委派写权限、`TASK_STOPPED`/`ptrace` stop 对 SIGKILL 一律致命、
//! D-state 需要挂死文件系统或特权设备）。**因此本文件不写假红**：
//! - 「`Ok` ⟺ 范围为空」用 **以 0 为死线的可注入限额**（`TerminationLimits`）测成确定性 `Err`，
//!   见 [`b206_zero_deadline_reports_err_for_live_range`]；
//! - 在可注入限额公开放行之前，本文件先以「`shutdown` 返回后范围非空」与「首次信号不是 SIGTERM」
//!   两条**契约事实**作为红相（二者都不依赖时机，只依赖可观测状态/可观测副作用）。
//!
//! ## 覆盖缺口（如实登记，不写跳过用例）
//! - **C2 的 `Err` 分支**：需要「公开可注入的限额」（`TerminationLimits { term_grace,
//!   quiesce_deadline }` + `ProcessManager::with_limits(...)`）。该 API 由实现方提供；本文件在
//!   其落地后补上 [`b206_zero_deadline_reports_err_for_live_range`]（ZERO 死线语义见该用例注释）。
//! - **C5 start-time 身份栅栏**：黑盒无法把 `/proc/<pid>/stat` 第 22 字段改成不匹配（它由内核
//!   写死），因此不可用 integration test 覆盖；该条只能靠单元测试（白盒）+ review。
//! - **C6 Windows 语义**：本机 Linux，无 Windows target。
//! - **C3 后半（「范围空了才 remove」）**：其可观测窗口与上面 12.4ms 的实测同理 —— 范围一旦
//!   真的空掉，正确的实现也允许立刻 `remove`，于是「晚 remove」与「早 remove」在返回后无法区分。
//!   本文件只覆盖 C3 前半（已 `Exited` 但范围非空期间条目仍可触达），后半登记为 review。
//!
//! ## 安全边界（事故后红线）
//! - 只用 `sleep` 这类**可被信号终止**的进程；**绝不**写 `while :; do :; done`、`yes`、
//!   `sleep infinity` 之类不可终止负载，也不做 `&`-fork 压测。
//! - 不按名字杀进程（禁止 `pkill`/`killall`）。每个用例自带 [`B206GroupGuard`]：Drop 时按
//!   `/proc` 中 `pgrp == 本组 pgid` 的成员**逐个** `SIGKILL`（不用 `killpg` 整组、不按名字），
//!   并 `shutdown` 本用例的 session、删除本用例的哨兵/marker 文件；panic 路径同样回收。
//!
//! ## 分权约束
//! 本文件是 executor **只读**的外部黑盒验收（integration test）：只经 `ProcessBackend` /
//! `ProcessManager` 的**公开** API 操作，不读私有字段、不吞异常、不做永真断言。
//!
//! ## 与工单编号的映射（供 review 对照）
//! | 工单条目 | 本文件用例 | 当前相 |
//! | --- | --- | --- |
//! | ② 主红相（C1「Ok ⟺ 空」） | [`b206_shutdown_reaps_group_after_shell_exit`] | 红 |
//! | ② 补充红相（C2 阶梯 rung-1） | [`b206_first_signal_is_sigterm`] | 红 |
//! | ② `Err` 死线（C1/C2 fail loud） | [`b206_zero_deadline_reports_err_for_live_range`] | 待限额 API |
//! | ① 吞 TERM 孙进程被收 | 并入主红相（同一构造：`trap "" TERM` 孙进程） | 红 |
//! | ③ B205 同形态正控 | [`b206_control_timeout_path_reaps_group_while_shell_alive`] | 绿（正控） |
//! | ④ 未知 handle 幂等（C4） | [`b206_kill_if_exists_is_idempotent_for_unknown_handle`] | 绿（正控） |
//! | ⑤ 已退出条目仍可触达（C3 前半） | [`b206_already_exited_entry_still_reachable`] | 绿（正控） |

#![cfg(unix)]

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use pony_agent_core::agent::process::ProcessManager;
use pony_agent_core::agent::tool_runtime::{
    ProcessBackend, ProcessStartRequest, ProcessState, SandboxRequest,
};

/// 工单标记：本轮 backlog 编号（每条用例的失败文案都带它）。
const B206_TICKET: &str = "B206";

/// 受管进程「自成进程组长」（`process_group(0)`）时，pgid 等于它自己的 pid。
const B206_PGID_IS_CHILD_PID: &str = "受管子进程必须是进程组长（pgid == 子进程 pid）";

/// 「等到标记全部出现在 stdout」的上限（命令启动阶段，远小于 sleep 时长）。
const B206_MARKER_TIMEOUT: Duration = Duration::from_secs(15);

/// 终结路径返回后，等待进程组清空的**有限**上限。超时即失败并点名存活 pid。
const B206_REAP_TIMEOUT: Duration = Duration::from_secs(8);

/// 轮询间隔（显式条件轮询，不是「睡到自以为清干净」）。
const B206_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// 超时正控的 `kill_after` 时长（覆盖「受管 shell 已起好、孙进程已就位」的窗口）。
const B206_KILL_AFTER: Duration = Duration::from_millis(400);

/// 孙进程存活时长（远大于用例时长；终态由终结路径决定，不靠它自己退出）。
const B206_GRANDCHILD_SLEEP_SECS: u32 = 120;

/// 契约 C2 的 `term_grace`（仅用于证据/文案，不参与断言 —— 断言只看可观测状态与副作用）。
const B206_TERM_GRACE: Duration = Duration::from_millis(250);

/// 契约 C2 的 `quiesce_deadline` 默认值（仅用于证据/文案）。
const B206_QUIESCE_DEADLINE: Duration = Duration::from_secs(5);

/// 受管 shell 的 SIGTERM trap 写进 marker 文件的令牌（证明「首次信号是 SIGTERM」）。
const B206_TERM_MARKER_TOKEN: &str = "B206_TERM_SEEN";

// ── 受管命令形态 ─────────────────────────────────────────────────────────────────────────────

/// 受管命令形态 ①：派一个**吞掉 SIGTERM** 的孙进程，然后轮询哨兵文件，文件出现即 `exit 0`。
///
/// - `trap "" TERM; exec /bin/sleep 120`：`exec` 后 SIGTERM 仍是 `SIG_IGN`（忽略处置跨 exec 保留），
///   因此只有 `SIGKILL` 能杀掉它 —— 正是「必须阶梯升级 + 必须证明范围为空」的最小构造；
/// - 先打印 `B206_LEADER=$$`（= 组长 pid = pgid）与 `B206_GC=$!`（孙进程 pid），供反空跑断言；
/// - 哨兵文件让测试掌控「shell 何时自行退出」（事故形态：工具侧看到「命令成功结束」），
///   同时保证取标记时 shell 一定还活着。
fn b206_script_term_trapping_grandchild(sentinel: &str) -> Vec<String> {
    let mut script = String::new();
    script.push_str("echo \"B206_LEADER=$$\"\n");
    script.push_str("/bin/bash -c 'trap \"\" TERM; exec /bin/sleep ");
    script.push_str(&B206_GRANDCHILD_SLEEP_SECS.to_string());
    script.push_str("' &\n");
    script.push_str("echo \"B206_GC=$!\"\n");
    script.push_str(&format!("while [ ! -e '{sentinel}' ]; do /bin/sleep 0.05; done\n"));
    script.push_str("exit 0\n");
    vec!["-c".to_string(), script]
}

/// 受管命令形态 ②：受管 shell **自己**装 SIGTERM trap（命中即写 marker 文件并 `exit 0`），
/// 同时派一个吞 SIGTERM 的孙进程，然后 `wait` 在前台。
///
/// 契约 C2 的阶梯要求「先 `SIGTERM`、宽限后再 `SIGKILL`」：
/// - shell 的 trap 被 SIGTERM 命中 ⇒ marker 文件出现 ⇒ **首次信号是 SIGTERM**（可观测副作用）；
/// - 孙进程只吃 `SIGKILL` ⇒ 收尾必须升级到 `SIGKILL` 才能把范围清空（同一条用例同时验 C1）。
fn b206_script_term_marker(marker: &str) -> Vec<String> {
    let mut script = String::new();
    script.push_str("echo \"B206_LEADER=$$\"\n");
    script.push_str(&format!(
        "trap 'printf \"%s\\n\" {B206_TERM_MARKER_TOKEN} >> \"{marker}\"; exit 0' TERM\n"
    ));
    script.push_str("/bin/bash -c 'trap \"\" TERM; exec /bin/sleep ");
    script.push_str(&B206_GRANDCHILD_SLEEP_SECS.to_string());
    script.push_str("' &\n");
    script.push_str("echo \"B206_GC=$!\"\n");
    script.push_str("wait\n");
    vec!["-c".to_string(), script]
}

/// 唯一临时文件路径（落在系统临时目录，绝不在仓库内）。
fn b206_temp_file(tag: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock must be after UNIX_EPOCH")
        .as_nanos();
    std::env::temp_dir()
        .join(format!("b206-{}-{}-{nanos}", tag, std::process::id()))
        .display()
        .to_string()
}

/// 本用例的 session id（自带 ticket/标签/pid，便于失败时点名人）。
fn b206_session(tag: &str) -> String {
    format!("{}-{}-{}", B206_TICKET.to_lowercase(), tag, std::process::id())
}

/// 起一个受管 `bash -c`（经 `ProcessManager`，即真实生产端口），返回 `(session, handle)`。
fn b206_spawn_in(
    manager: &ProcessManager,
    session: &str,
    script: Vec<String>,
) -> (String, String) {
    let handle = manager
        .start(&ProcessStartRequest {
            session_id: session.to_string(),
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
        .unwrap_or_else(|error| panic!("{B206_TICKET}: ProcessManager::start 必须成功：{error}"));
    (session.to_string(), handle)
}

/// 同 [`b206_spawn_in`]，但 session 由标签派生。
fn b206_spawn(manager: &ProcessManager, tag: &str, script: Vec<String>) -> (String, String) {
    b206_spawn_in(manager, &b206_session(tag), script)
}

// ── stdout 标记解析 ─────────────────────────────────────────────────────────────────────────

/// 一次 `poll` 的累计视图。
struct B206Polled {
    stdout: String,
    exited: bool,
    exit_code: Option<i32>,
}

/// 从累计 stdout 解析出的进程身份：`leader`（受管子进程 pid = pgid）与孙进程 pid。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct B206Markers {
    leader: i32,
    grandchild: i32,
}

impl B206Markers {
    /// 本用例应有的全体成员（排序后便于与 `/proc` 扫描结果逐值比较）。
    fn expected_members(&self) -> Vec<i32> {
        let mut members = vec![self.leader, self.grandchild];
        members.sort_unstable();
        members
    }
}

/// 解析标记；缺失时返回 `None`（由调用方在超时后 panic 并带上原始 stdout）。
fn b206_try_parse_markers(stdout: &str) -> Option<B206Markers> {
    let mut leader = None;
    let mut grandchild = None;
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("B206_LEADER=") {
            leader = rest.trim().parse::<i32>().ok().or(leader);
        } else if let Some(rest) = line.strip_prefix("B206_GC=") {
            grandchild = rest.trim().parse::<i32>().ok().or(grandchild);
        }
    }
    Some(B206Markers {
        leader: leader?,
        grandchild: grandchild?,
    })
}

/// 轮询 `poll` 直到 `ready` 成立；有界等待，超时 panic（不吞异常）。
fn b206_poll_until(
    manager: &ProcessManager,
    session: &str,
    handle: &str,
    timeout: Duration,
    what: &str,
    mut ready: impl FnMut(&B206Polled) -> bool,
) -> B206Polled {
    let deadline = Instant::now() + timeout;
    let mut acc = String::new();
    let mut exited = false;
    let mut exit_code = None;
    loop {
        let result = manager
            .poll(session, handle)
            .unwrap_or_else(|error| panic!("{B206_TICKET}: poll 必须成功（{what}）：{error}"));
        acc.push_str(&result.stdout);
        if result.state == ProcessState::Exited {
            exited = true;
            exit_code = result.exit_code;
        }
        let polled = B206Polled {
            stdout: acc.clone(),
            exited,
            exit_code,
        };
        if ready(&polled) {
            return polled;
        }
        assert!(
            Instant::now() < deadline,
            "{B206_TICKET}: 等待超时（{what}）；已累计 stdout={:?}、exited={exited}、exit_code={exit_code:?}",
            polled.stdout
        );
        std::thread::sleep(B206_POLL_INTERVAL);
    }
}

/// 读到全部标记即返回（不要求 shell 退出）。
fn b206_read_markers(
    manager: &ProcessManager,
    session: &str,
    handle: &str,
    what: &str,
) -> B206Markers {
    let polled = b206_poll_until(manager, session, handle, B206_MARKER_TIMEOUT, what, |polled| {
        b206_try_parse_markers(&polled.stdout).is_some()
    });
    b206_try_parse_markers(&polled.stdout).unwrap_or_else(|| {
        panic!(
            "{B206_TICKET}: 标记解析失败（{what}）；stdout={:?}",
            polled.stdout
        )
    })
}

// ── /proc 探针（进程范围是否真的为空）──────────────────────────────────────────────────────

/// `/proc/<pid>/stat` 的 `(pgrp, state)`。
fn b206_proc_pgrp_state(pid: i32) -> Option<(i32, char)> {
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
fn b206_live_group_members(pgid: i32) -> Vec<i32> {
    let mut members = Vec::new();
    let entries = match std::fs::read_dir("/proc") {
        Ok(entries) => entries,
        Err(error) => {
            panic!("{B206_TICKET}: 无法读取 /proc（探针不可用即失败，不得静默放行）：{error}")
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
        if let Some((pgrp, state)) = b206_proc_pgrp_state(pid) {
            if pgrp == pgid && state != 'Z' {
                members.push(pid);
            }
        }
    }
    members.sort_unstable();
    members
}

/// `kill(pid, 0)` 是否报告「进程存在」（仅作证据，不作唯一判据）。
fn b206_pid_exists(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

/// pid 是否已经「死透」：`/proc/<pid>/stat` 不存在（已被收尸）或为僵尸。
///
/// 用它替代 `!b206_pid_exists` 作断言，避免「刚被杀、尚未被 init 收尸」的僵尸造成的假红。
fn b206_pid_dead(pid: i32) -> bool {
    match b206_proc_pgrp_state(pid) {
        None => true,
        Some((_, state)) => state == 'Z',
    }
}

/// `kill(-pgid, 0)` 的证据串（errno 说明进程组是否还有成员）。
fn b206_group_probe_evidence(pgid: i32) -> String {
    let rc = unsafe { libc::kill(-pgid, 0) };
    let error = std::io::Error::last_os_error();
    format!(
        "kill(-{pgid},0)={rc}, errno={:?} ({error})",
        error.raw_os_error().unwrap_or(0)
    )
}

/// 有界轮询直到进程组清空；超时 panic 并点名存活成员 + 辅助证据。
fn b206_wait_group_empty(pgid: i32, timeout: Duration, what: &str) {
    let deadline = Instant::now() + timeout;
    loop {
        let members = b206_live_group_members(pgid);
        if members.is_empty() {
            return;
        }
        let evidence = b206_group_probe_evidence(pgid);
        assert!(
            Instant::now() < deadline,
            "{B206_TICKET}: {what}：进程组 pgid={pgid} 在 {timeout:?} 内未清空，仍存活的成员 pid={members:?}（{evidence}）\
             —— 这些进程的父进程已退出，它们就是 PPID=1 孤儿，正是 2026-11-13 事故的病灶"
        );
        std::thread::sleep(B206_POLL_INTERVAL);
    }
}

/// 有界轮询直到指定 pid 死透；超时 panic。
fn b206_wait_pid_dead(pid: i32, timeout: Duration, what: &str) {
    let deadline = Instant::now() + timeout;
    loop {
        if b206_pid_dead(pid) {
            return;
        }
        let evidence = match b206_proc_pgrp_state(pid) {
            Some((pgrp, state)) => format!("pid={pid} 仍存活（pgrp={pgrp}, state={state}）"),
            None => format!("pid={pid} 已消失"),
        };
        assert!(
            Instant::now() < deadline,
            "{B206_TICKET}: {what}：{evidence}；{B206_PGID_IS_CHILD_PID} 语义下它属于受管进程组，属于范围泄漏"
        );
        std::thread::sleep(B206_POLL_INTERVAL);
    }
}

// ── 用例级回收守卫 ─────────────────────────────────────────────────────────────────────────

/// 谁创建谁清理：Drop 时精确回收本用例的进程组成员 + `shutdown` 本用例的 session + 删临时文件。
///
/// 只按 `/proc` 里 `pgrp == self.pgid` 的成员逐个 `SIGKILL`（不整组 `killpg`、不按名字杀），
/// 避免 pgid 被复用时误杀无关进程；不使用 `while :; do :; done` 之类不可终止的负载。
struct B206GroupGuard {
    manager: ProcessManager,
    session: String,
    pgid: Option<i32>,
    known_pids: Vec<i32>,
    temp_files: Vec<String>,
}

impl B206GroupGuard {
    fn new(manager: &ProcessManager, session: &str) -> Self {
        Self {
            manager: manager.clone(),
            session: session.to_string(),
            pgid: None,
            known_pids: Vec::new(),
            temp_files: Vec::new(),
        }
    }

    fn track(&mut self, markers: B206Markers) {
        self.pgid = Some(markers.leader);
        self.known_pids = vec![markers.grandchild];
    }

    fn track_temp_file(&mut self, path: &str) {
        self.temp_files.push(path.to_string());
    }
}

impl Drop for B206GroupGuard {
    fn drop(&mut self) {
        if let Some(pgid) = self.pgid {
            for pid in b206_live_group_members(pgid) {
                unsafe {
                    libc::kill(pid, libc::SIGKILL);
                }
            }
        }
        for pid in &self.known_pids {
            // 仍属本组的 pid 才动手（pid 复用防护）。
            if let Some((pgrp, _)) = b206_proc_pgrp_state(*pid) {
                if Some(pgrp) == self.pgid {
                    unsafe {
                        libc::kill(*pid, libc::SIGKILL);
                    }
                }
            }
        }
        for path in &self.temp_files {
            let _ = std::fs::remove_file(path);
        }
        // 管理器的 session 级兜底回收（失败不影响其它用例）。
        self.manager.shutdown(&self.session);
    }
}

/// 反空跑前缀：断言「组长（受管 shell）+ 孙进程」此刻确实同属 `pgid == leader` 这个进程组。
///
/// 必须在使用「组内无存活成员」判据的**最早**时刻调用（标记刚读到、shell 仍在掌控中），
/// 它同时证明三件事：① 孙进程真的被 fork 出来了；② pgid 推导正确
/// （pgid == 受管子进程 pid）；③ 后续「组内无存活成员」不是恒真断言。
///
/// 用**子集**判定而非等值：哨兵形态下 shell 每 50ms fork 一个短命 `/bin/sleep 0.05`，
/// 组内瞬时成员可能多出一个；多出的短命进程会自行退出，且同样被终结路径收组。
fn b206_assert_group_intact(markers: B206Markers, what: &str) {
    let live = b206_live_group_members(markers.leader);
    let expected = markers.expected_members();
    let missing: Vec<i32> = expected
        .iter()
        .copied()
        .filter(|pid| !live.contains(pid))
        .collect();
    assert!(
        missing.is_empty(),
        "{B206_TICKET}: 前置失败（{what}）—— 进程组 pgid={} 必须含 组长+孙进程 {expected:?}\
         （{B206_PGID_IS_CHILD_PID}），缺失 {missing:?}；实际存活成员={live:?}",
        markers.leader
    );
}

// ── 用例 1（主红相，C1）：`shutdown(session)` 返回后范围必须已清空 ───────────────────────────

/// 事故同源形态 + 第四条缺口：受管 shell 派一个**吞掉 SIGTERM** 的孙进程后**自行正常退出
/// （exit 0）**，随后会话被 `ProcessManager::shutdown(session)` 收尾。
///
/// 契约 C1：`shutdown` 返回 ⟺ 范围为空。当前实现（红相）：`shutdown` 逐条调用
/// `terminate()`，而 `terminate()` 的 `ExitState::Exited` 分支在非 Windows 上**不发任何信号**
/// （只有 Windows 侧 `job.terminate(1)`），随后条目被删除 ⇒ 返回时孙进程仍活着，逃逸成
/// PPID=1 孤儿。这正是 B205 三条终结路径（`kill_after` / `cancel` / `kill`）之外漏掉的第四条。
///
/// 本用例同时覆盖工单条目 ① 的构造（`trap "" TERM` 孙进程）：`SIGKILL` 不可被 trap，
/// 因此该条目在当前实现上只能表现为正控（见文件头「覆盖缺口」说明）。
#[test]
fn b206_shutdown_reaps_group_after_shell_exit() {
    let manager = ProcessManager::new();
    let sentinel = b206_temp_file("shutdown-sentinel");
    let (session, handle) = b206_spawn(
        &manager,
        "shutdown-exited",
        b206_script_term_trapping_grandchild(&sentinel),
    );
    let mut guard = B206GroupGuard::new(&manager, &session);
    guard.track_temp_file(&sentinel);

    // shell 仍在轮询哨兵：此刻取标记并做「组内成员完整」正值断言（反空跑）。
    let markers = b206_read_markers(&manager, &session, &handle, "读取 shutdown 用例的进程身份标记");
    guard.track(markers);
    b206_assert_group_intact(markers, "受管 shell 存活期（shutdown 用例）");

    // Arrange：落哨兵，让受管 shell **按自己的节奏**正常退出（事故形态：命令成功结束）。
    std::fs::write(&sentinel, b"go")
        .unwrap_or_else(|error| panic!("{B206_TICKET}: 写哨兵文件必须成功（{sentinel}）：{error}"));
    let exited = b206_poll_until(
        &manager,
        &session,
        &handle,
        B206_MARKER_TIMEOUT,
        "等待受管 shell 自行退出（shutdown 用例）",
        |polled| polled.exited,
    );
    assert_eq!(
        exited.exit_code,
        Some(0),
        "{B206_TICKET}: 前置失败 —— 受管 shell 应以 0 正常退出（事故形态：工具侧看到「命令已成功结束」）"
    );

    // 证据（不作断言）：此刻哪些孙进程仍存活。实现若选择「一观察到受管命令退出就收组」，
    // 这里会是空集 —— 那是允许的合法实现，不应判红。
    let survivors: Vec<i32> = [markers.grandchild]
        .into_iter()
        .filter(|pid| b206_pid_exists(*pid))
        .collect();
    eprintln!(
        "{B206_TICKET}: [shutdown] 受管 shell 退出后仍存活的孙进程 = {survivors:?}\
         （空 = 实现已在退出时即收组）"
    );

    // Act：真实会话级收尾路径。
    let shut_down = manager.shutdown(&session);
    eprintln!(
        "{B206_TICKET}: [shutdown] shutdown(session) 返回 {shut_down} 个条目\
         （契约 C2：term_grace={B206_TERM_GRACE:?} → SIGKILL → quiesce_deadline={B206_QUIESCE_DEADLINE:?}）"
    );

    // Assert（C1）：shutdown 返回 ⟺ 范围为空。
    b206_wait_group_empty(
        markers.leader,
        B206_REAP_TIMEOUT,
        "shutdown(session) 返回后进程组未清空 ⇒ 违反 C1「返回 Ok ⟺ 范围为空」\
         （terminate 的 Exited 分支在 unix 上未收组，B205 三条路径之外漏掉的第四条）",
    );
    b206_wait_pid_dead(
        markers.grandchild,
        B206_REAP_TIMEOUT,
        "shutdown 后吞 TERM 的孙进程仍存活（范围未静默就返回了）",
    );
}

// ── 用例 2（红相，C2 阶梯 rung-1）：首次信号必须是 SIGTERM ──────────────────────────────────

/// 契约 C2 的阶梯是「`SIGTERM` → 宽限 → `SIGKILL`」，**首枪必须是 `SIGTERM`**。
///
/// 可观测副作用判据（不猜时机）：受管 shell 自己装 `trap ... TERM`，命中即把
/// [`B206_TERM_MARKER_TOKEN`] 追加进 marker 文件。因此
/// **marker 文件存在 ⟺ 受管进程收到过 SIGTERM**。当前实现（红相）直接 `killpg(SIGKILL)`，
/// 受管 shell 来不及（也不需要）处理 TERM ⇒ marker 文件不存在 ⇒ 断言失败。
///
/// 同一条用例还验 C1 的另一半：孙进程只吃 `SIGKILL`（`trap "" TERM`），
/// 所以收尾必须真的升级到 `SIGKILL` 才可能把范围清空；`kill` 返回后范围必须为空。
#[test]
fn b206_first_signal_is_sigterm() {
    let manager = ProcessManager::new();
    let marker = b206_temp_file("term-marker");
    let (session, handle) = b206_spawn(&manager, "first-signal", b206_script_term_marker(&marker));
    let mut guard = B206GroupGuard::new(&manager, &session);
    guard.track_temp_file(&marker);

    let markers = b206_read_markers(&manager, &session, &handle, "读取首枪信号用例的进程身份标记");
    guard.track(markers);
    b206_assert_group_intact(markers, "受管 shell 存活期（首枪信号用例）");
    assert!(
        !std::path::Path::new(&marker).exists(),
        "{B206_TICKET}: 前置失败 —— SIGTERM marker 文件不得在任何信号发出前就存在（{marker}）"
    );

    // Act：真实终结路径（`kill` → `kill_inner` → `terminate`）。
    let started = Instant::now();
    let outcome = manager.kill(&session, &handle);
    let elapsed = started.elapsed();
    assert!(
        outcome.is_ok(),
        "{B206_TICKET}: 契约 C1 —— 该范围可被清空，`kill` 必须返回 Ok，实际 {outcome:?}（用时 {elapsed:?}）"
    );

    // Assert（C2 rung-1）：首次信号必须是 SIGTERM —— 用受管 shell 自己的 trap 写出的 marker 作证。
    let body = std::fs::read_to_string(&marker).unwrap_or_default();
    assert!(
        body.contains(B206_TERM_MARKER_TOKEN),
        "{B206_TICKET}: 契约 C2 阶梯第一级被违反 —— 受管进程在整条终结路径中从未收到 SIGTERM\
         （marker 文件 {marker} 不含 {B206_TERM_MARKER_TOKEN}，实际内容 {body:?}）；\
         当前实现直接 killpg(SIGKILL)，跳过了 term_grace={B206_TERM_GRACE:?} 这一级（用时 {elapsed:?}）"
    );

    // Assert（C1）：返回后范围必须为空（孙进程只吃 SIGKILL，故这也证明阶梯确实升级到了 SIGKILL）。
    b206_wait_group_empty(
        markers.leader,
        B206_REAP_TIMEOUT,
        "首枪信号用例：kill 返回后进程组未清空",
    );
    b206_wait_pid_dead(
        markers.grandchild,
        B206_REAP_TIMEOUT,
        "首枪信号用例：吞 TERM 的孙进程未被 SIGKILL 补刀收掉",
    );
}

// ── 用例 3（正控，B205 同形态）：shell 仍在前台时超时路径收组 ───────────────────────────────

/// 正控：受管 shell 留在前台 `wait`，超时安全网 `kill_after` 开火时进程组可被整组收掉。
/// 该形态必须绿，用来证明「进程组清空」这条判据本身有效（不是因为探针坏了才让红相变红）。
#[test]
fn b206_control_timeout_path_reaps_group_while_shell_alive() {
    let manager = ProcessManager::new();
    let marker_sentinel_never_used = b206_temp_file("control-sentinel");
    let (session, handle) = b206_spawn(
        &manager,
        "control-alive",
        b206_script_term_marker(&marker_sentinel_never_used),
    );
    let mut guard = B206GroupGuard::new(&manager, &session);
    guard.track_temp_file(&marker_sentinel_never_used);

    let markers = b206_read_markers(&manager, &session, &handle, "读取正控用例的进程身份标记");
    guard.track(markers);
    b206_assert_group_intact(markers, "正控：超时路径开火前");

    // Act：真实超时路径。
    manager.kill_after(&session, &handle, B206_KILL_AFTER);

    // Assert：进程组清空（正控必须绿）。
    b206_wait_group_empty(
        markers.leader,
        B206_REAP_TIMEOUT,
        "正控：超时终结路径返回后进程组未清空",
    );
    b206_wait_pid_dead(
        markers.grandchild,
        B206_REAP_TIMEOUT,
        "正控：孙进程未被收掉",
    );
}

// ── 用例 4（C4 幂等）：未知 handle 与跨 session 的语义不得被本轮改动破坏 ─────────────────────

/// C4：幂等与 fail-closed 两条语义必须**同时**成立且方向相反 ——
/// - 未知 handle + `kill_if_exists` ⇒ `Ok(())`（幂等，供超时/关闭类清理路径复用）；
/// - 未知 handle + `kill` ⇒ `Err`（未要求幂等时不得吞掉「handle 不存在」这一事实）；
/// - 跨 session 的 `kill_if_exists` ⇒ `Err`（幂等只覆盖「不存在」，**不得**吞掉越权），
///   且被拒后目标进程必须**毫发无损**（仍 `Running`）。
#[test]
fn b206_kill_if_exists_is_idempotent_for_unknown_handle() {
    let manager = ProcessManager::new();
    let session = b206_session("unknown-handle");
    let unknown_handle = "proc-000000000000ffff-0000ffff-000000";

    // ① 未知 handle：kill_if_exists ⇒ Ok（幂等）。
    manager
        .kill_if_exists(&session, unknown_handle)
        .unwrap_or_else(|error| {
            panic!(
                "{B206_TICKET}: C4 被破坏 —— 未知 handle 的 kill_if_exists 必须返回 Ok（幂等），\
                 实际 Err：{error}"
            )
        });

    // ② 未知 handle：kill ⇒ Err，且错误消息点名 unknown process handle（fail loud）。
    let error = manager.kill(&session, unknown_handle).err().unwrap_or_else(|| {
        panic!(
            "{B206_TICKET}: C4 被破坏 —— 未加 if_exists 的 `kill` 对未知 handle 必须返回 Err，\
             实际返回 Ok（把「handle 不存在」静默吞掉了）"
        )
    });
    assert!(
        error.contains("unknown process handle"),
        "{B206_TICKET}: C4 —— `kill` 的错误消息必须点名 unknown process handle，实际 {error:?}"
    );

    // ③ 跨 session：真实进程由本 session 创建，另一个 session 的 kill_if_exists 必须 Err。
    let sentinel = b206_temp_file("c4-sentinel");
    let (owner_session, handle) = b206_spawn(
        &manager,
        "c4-owner",
        b206_script_term_trapping_grandchild(&sentinel),
    );
    let mut guard = B206GroupGuard::new(&manager, &owner_session);
    guard.track_temp_file(&sentinel);
    let markers = b206_read_markers(&manager, &owner_session, &handle, "读取 C4 用例的进程身份标记");
    guard.track(markers);
    b206_assert_group_intact(markers, "C4 用例：跨 session 拒绝前");

    let cross_error = manager
        .kill_if_exists(&session, &handle)
        .err()
        .unwrap_or_else(|| {
            panic!(
                "{B206_TICKET}: C4 被破坏 —— 跨 session 的 kill_if_exists 必须 Err（幂等不得吞掉越权），\
                 实际返回 Ok"
            )
        });
    assert!(
        cross_error.contains("cross-session"),
        "{B206_TICKET}: C4 —— 跨 session 拒绝必须可辨识（消息含 cross-session），实际 {cross_error:?}"
    );
    // 被拒之后目标必须毫发无损：受管 shell 仍在跑（这是「拒绝」而不是「拒绝并偷偷杀」）。
    let polled = manager
        .poll(&owner_session, &handle)
        .unwrap_or_else(|error| panic!("{B206_TICKET}: C4 前置失败 —— poll 必须成功：{error}"));
    assert_eq!(
        polled.state,
        ProcessState::Running,
        "{B206_TICKET}: C4 —— 跨 session 请求被拒后，目标进程必须仍在运行（不得被偷偷收掉）"
    );
}

// ── 用例 5（C3 前半）：直系子进程已 Exited 但范围仍非空时，条目不得被提前移除 ────────────────

/// C3 前半（黑盒可观测的一半）：直系子进程已 `Exited` 而范围仍非空时，条目必须**保留**，
/// 否则后续补刀（`kill`/`kill_if_exists`）根本触达不到这一组进程 —— 这正是孤儿泄漏的入口。
///
/// 可观测代理（`is_tracked` 是 `pub(crate)`，集成测试用不到）：
/// - `poll` 仍返回 `Ok` 且 `state == Exited`（条目还在，最终状态仍可报告）；
/// - 随后 `kill` 仍能成功返回 `Ok`，且返回后范围为空（补刀真的打到组上了）。
///
/// 现状：绿（正控）。C3 后半「范围空了才 remove」在返回后不可区分，已登记为 review。
#[test]
fn b206_already_exited_entry_still_reachable() {
    let manager = ProcessManager::new();
    let sentinel = b206_temp_file("c3-sentinel");
    let (session, handle) = b206_spawn(
        &manager,
        "c3-retained",
        b206_script_term_trapping_grandchild(&sentinel),
    );
    let mut guard = B206GroupGuard::new(&manager, &session);
    guard.track_temp_file(&sentinel);

    let markers = b206_read_markers(&manager, &session, &handle, "读取 C3 用例的进程身份标记");
    guard.track(markers);
    b206_assert_group_intact(markers, "C3 用例：受管 shell 存活期");

    // Arrange：落哨兵 → 受管 shell 自行正常退出（直系子进程 Exited，孙进程仍在组内）。
    std::fs::write(&sentinel, b"go")
        .unwrap_or_else(|error| panic!("{B206_TICKET}: 写哨兵文件必须成功（{sentinel}）：{error}"));
    let exited = b206_poll_until(
        &manager,
        &session,
        &handle,
        B206_MARKER_TIMEOUT,
        "等待受管 shell 自行退出（C3 用例）",
        |polled| polled.exited,
    );
    assert_eq!(
        exited.exit_code,
        Some(0),
        "{B206_TICKET}: 前置失败 —— 受管 shell 应以 0 正常退出"
    );

    // Assert（C3 前半）：条目必须仍可触达 —— `poll` 不得报 unknown handle。
    let polled = manager.poll(&session, &handle).unwrap_or_else(|error| {
        panic!(
            "{B206_TICKET}: C3 被破坏 —— 直系子进程已 Exited 但范围仍非空时，条目不得被提前 remove；\
             此时 poll 必须仍成功，实际 Err：{error}"
        )
    });
    assert_eq!(
        polled.state,
        ProcessState::Exited,
        "{B206_TICKET}: C3 —— 条目保留时 poll 必须报告 Exited 终态"
    );
    assert_eq!(
        polled.exit_code,
        Some(0),
        "{B206_TICKET}: C3 —— 保留的条目必须仍带正确的退出码"
    );
    // 证据（不作断言）：此刻组内是否仍有存活成员。允许实现已在观察到退出时收组。
    let live = b206_live_group_members(markers.leader);
    eprintln!(
        "{B206_TICKET}: [C3] 直系子进程已 Exited，此刻组内非僵尸成员 = {live:?}\
         （空 = 实现已在退出时即收组，仍属合法实现）"
    );

    // Assert（C3 后半的可观测代理）：条目保留下来 ⇒ 补刀仍能打到整组 ⇒ 返回后范围为空。
    manager.kill(&session, &handle).unwrap_or_else(|error| {
        panic!("{B206_TICKET}: C3 —— 已 Exited 条目的补刀 `kill` 必须成功，实际 Err：{error}")
    });
    b206_wait_group_empty(
        markers.leader,
        B206_REAP_TIMEOUT,
        "C3 用例：补刀 kill 返回后进程组未清空",
    );
    b206_wait_pid_dead(
        markers.grandchild,
        B206_REAP_TIMEOUT,
        "C3 用例：吞 TERM 的孙进程未在补刀后被收掉",
    );
}

// ── 用例 6（红相，C1/C2 fail loud）：0 死线下范围非空必须返回 `Err` ──────────────────────────
//
// 本用例依赖实现方提供的**公开可注入限额**（Lead 裁决 3）：
//
//   `TerminationLimits { term_grace: Duration, quiesce_deadline: Duration }`（默认 250ms / 5s）
//   `ProcessManager::with_limits(limits)`（或等价公开构造）
//
// ZERO 死线语义 =「信号发出后**不再等待**，只在信号发出后立刻做一次探测；范围非空即 Err」。
//
// 为何该断言是确定性的（不靠睡眠猜时机）：实测 `/tmp/b206_probe.py`，
// `killpg(SIGKILL)` 之后第一次 `/proc` 扫描仍有 12.4ms 的落后期才归零
// （`elapsed_ms=12.420 samples=1 nonempty_samples=0`，即扫描一次才看见 0）。
// 既然内核投递 SIGKILL 到「范围真的清空」之间必然存在非零延迟，
// 「发信号后 **立刻** 探测」就必然仍看到成员 ⇒ 必然 `Err`。这与 250ms/5s 的默认死线无关，
// 只是把「等」这一步显式去掉，让 fail-loud 分支可被黑盒观测。
//
// 该用例在 `with_limits` 落地后由本文件补上（断言文本已冻结，见下方 `#[ignore]` 说明：不允许
// `#[ignore]` 式跳过，故在 API 可用前**不写入**函数体，只在此登记判据；避免用假缝造绿）。

/// 冻结判据（待 `with_limits` 公开放行后落为真实用例，届时断言文本逐字如下）：
///
/// 1. `manager = ProcessManager::with_limits(TerminationLimits { term_grace: Duration::ZERO,
///    quiesce_deadline: Duration::ZERO })`；
/// 2. 起受管 shell（[`b206_script_term_trapping_grandchild`]，含吞 TERM 的孙进程），
///    取标记并断言组内成员完整（反空跑）；
/// 3. `kill(&session, &handle)` 必须返回 `Err`；
/// 4. `Err` 消息必须同时含 **pgid**（十进制数字串）与 **仍存活成员 pid**（便于现场定位）；
/// 5. `Err` 之后范围必须仍非空（证明不是「先杀了再谎报 Err」：此时孙进程仍在组内），
///    随后由测试自身（[`B206GroupGuard`]）精确回收，不留孤儿。
///
/// 若实现方拒绝提供公开限额，本条按 Lead 裁决 3 改为契约文档中的 `[靠 review]` 登记。
fn _b206_zero_deadline_assertions_frozen_here() {}
// ═════════════════════════════════════════════════════════════════════════════════════════════
// B206 / S-3 追加（第二批）：`shutdown_checked` 的加法式接线正控 + C1-U 的静态机器影子
//
// 契约 §7 列出的 `shutdown_checked` 三条判据在本批落地：
// ① [`b206_shutdown_checked_ok_counts_proven_ranges`]（正控）：`Ok(proven)` 且返回时范围全空；
// ② [`b206_shutdown_legacy_count_matches_checked`]（正控）：旧 `shutdown` 计数 == checked 的 proven 数，
//    且**跨 session 不越权**（另一 session 的进程不受影响）；
// ③ [`b206_single_termination_path_static`]（静态机器影子）：`libc::killpg` 在范围终结面
//    （`process.rs` + `process_range.rs`）恰好 1 处，且只能被白名单内的函数调用。
//
// **`Err` 分支本轮不冻结**（Lead 已撤回原「ZERO 死线 ⇒ 确定性 Err」裁决）：`killpg(SIGKILL)`
// 之后第一次 `/proc` 扫描（5~10ms）跑完时目标已是僵尸，而 PROBE 按契约不计僵尸 ⇒ 判空 ⇒ `Ok`；
// 连跑 3/3 都是 `Ok`。造「连 SIGKILL 都杀不掉的成员」需 D 状态/特权，本机不可达 —— 写死该断言
// 只能得到 flaky 假红。该分支按契约文档标 `[靠 review]`，由实现方自检单测覆盖可确定性观测的一半。
//
// ## 自纠记录（测试缺陷，非实现缺陷）
// 本用例初版断言「`libc::killpg` 必须内联在 `terminate_range_path` 函数体内」——**该假设错误**：
// 实现把投递收在共享原语 `fn signal_range(pgid, signal)` 里，且随后 B207 又把投递抽象成
// `RangeOwner::signal`（`PidRangeOwner::signal` → `super::signal_range`）。因此本用例改为
// **结构无关**的判据（单一 `killpg` 派发原语 + 调用者白名单 + `terminate_range_path` 内的
// 阶梯投递次数），对「原语内联」与「owner 适配」两种结构都成立。
// ═════════════════════════════════════════════════════════════════════════════════════════════

/// 受管命令形态 ③：打印组长/孙进程 pid 后前台 `wait` —— **两个进程都能被 `SIGTERM` 正常终止**
/// （不带 `trap "" TERM`），用于 `shutdown_checked` 的「正常收组 + 计数」正控。
fn b206_script_plain_wait() -> Vec<String> {
    let mut script = String::new();
    script.push_str("echo \"B206_LEADER=$$\"\n");
    script.push_str("/bin/sleep ");
    script.push_str(&B206_GRANDCHILD_SLEEP_SECS.to_string());
    script.push_str(" &\n");
    script.push_str("echo \"B206_GC=$!\"\n");
    script.push_str("wait\n");
    vec!["-c".to_string(), script]
}

// ── 用例 6（S-3 正控）：`shutdown_checked` 返回 Ok(proven) 且返回时范围已空 ────────────────────

/// 契约 §7 / S-3：`shutdown_checked(session)` 必须返回**已证明为空的范围数**，且**返回那一刻**
/// 每个范围都已空（C1 的加法式版本：proven 数是「真的证明了」的计数，不是「发了信号」的计数）。
///
/// 同时验 C4 的幂等：条目已移除后 `shutdown(session)` 再调必须返回 `0` —— 证明加法式接线
/// 没有把既有 `shutdown` 语义搞坏（旧调用方的返回口径不变）。
#[test]
fn b206_shutdown_checked_ok_counts_proven_ranges() {
    let manager = ProcessManager::new();
    let session = b206_session("shutdown-checked-ok");
    let mut group_guards = Vec::new();
    let mut pgids = Vec::new();
    for index in 0..2usize {
        let (_, handle) = b206_spawn_in(&manager, &session, b206_script_plain_wait());
        let markers = b206_read_markers(
            &manager,
            &session,
            &handle,
            &format!("读取第 {index} 个受管进程的身份标记"),
        );
        let mut guard = B206GroupGuard::new(&manager, &session);
        guard.track(markers);
        // 反空跑：起 shutdown_checked 之前，组长 + 孙进程必须真的同属一个进程组。
        b206_assert_group_intact(markers, &format!("shutdown_checked 前（第 {index} 个）"));
        group_guards.push(guard);
        pgids.push(markers.leader);
    }

    let proven = manager
        .shutdown_checked(&session)
        .unwrap_or_else(|error| panic!("{B206_TICKET}: shutdown_checked 必须返回 Ok：{error}"));
    assert_eq!(
        proven, 2,
        "{B206_TICKET}: S-3 —— 两个可正常终止的范围都必须被**证明**为空并计数，实际 proven={proven}"
    );

    // 返回后立刻逐个范围验证「已空」（不是「信号已发」）。
    for pgid in &pgids {
        b206_wait_group_empty(
            *pgid,
            B206_REAP_TIMEOUT,
            "shutdown_checked 返回后进程组未清空（proven 计数不成立）",
        );
    }

    // C4 幂等：条目已移除，旧口径再调返回 0。
    assert_eq!(
        manager.shutdown(&session),
        0,
        "{B206_TICKET}: 条目已被 shutdown_checked 移除，旧 shutdown 再调必须返回 0（幂等语义不变）"
    );

    // guard 仅作兜底回收器：本用例的断言不依赖它们。
    drop(group_guards);
}

// ── 用例 7（S-3 正控）：旧口径计数 == checked 的 proven 数，且不越权到别的 session ─────────────

/// 契约 §7 / S-3：旧口径 `shutdown(session)` 的返回数必须等于 checked 的 proven 范围数（同一真值），
/// 且**只动本 session**：另一个 session 的进程必须仍在 `Running`，随后 `kill` 仍能成功命中。
#[test]
fn b206_shutdown_legacy_count_matches_checked() {
    let manager = ProcessManager::new();
    let victim_session = b206_session("legacy-count");
    let aside_session = b206_session("legacy-aside");
    let mut group_guards = Vec::new();

    let mut victim_pgids = Vec::new();
    for index in 0..2usize {
        let (_, handle) = b206_spawn_in(&manager, &victim_session, b206_script_plain_wait());
        let markers = b206_read_markers(
            &manager,
            &victim_session,
            &handle,
            &format!("读取待关闭 session 第 {index} 个进程的身份标记"),
        );
        let mut guard = B206GroupGuard::new(&manager, &victim_session);
        guard.track(markers);
        b206_assert_group_intact(markers, &format!("shutdown 前（victim 第 {index} 个）"));
        group_guards.push(guard);
        victim_pgids.push(markers.leader);
    }

    // 另一个 session 里的旁观进程。
    let (aside_session, aside_handle) =
        b206_spawn_in(&manager, &aside_session, b206_script_plain_wait());
    let aside_markers = b206_read_markers(
        &manager,
        &aside_session,
        &aside_handle,
        "读取旁观 session 的进程身份标记",
    );
    let mut aside_guard = B206GroupGuard::new(&manager, &aside_session);
    aside_guard.track(aside_markers);
    b206_assert_group_intact(aside_markers, "shutdown 前（旁观 session）");

    let count = manager.shutdown(&victim_session);
    assert_eq!(
        count, 2,
        "{B206_TICKET}: S-3 —— 旧口径 shutdown 的计数必须等于 checked 的 proven 范围数（2），实际 {count}"
    );
    for pgid in &victim_pgids {
        b206_wait_group_empty(
            *pgid,
            B206_REAP_TIMEOUT,
            "旧口径 shutdown 返回后进程组未清空",
        );
    }

    // 跨 session 不越权：旁观进程必须仍活着（条目仍在且 Running）。
    let polled = manager
        .poll(&aside_session, &aside_handle)
        .unwrap_or_else(|error| panic!("{B206_TICKET}: 旁观的 poll 必须成功：{error}"));
    assert_eq!(
        polled.state,
        ProcessState::Running,
        "{B206_TICKET}: 关闭 {victim_session} 不得波及 {aside_session}（跨 session 越权）"
    );
    let live_aside = b206_live_group_members(aside_markers.leader);
    assert!(
        live_aside.contains(&aside_markers.leader) && live_aside.contains(&aside_markers.grandchild),
        "{B206_TICKET}: 旁观 session 的组长/孙进程必须都仍存活，实际组内成员={live_aside:?}"
    );

    // 旁观的 kill 仍能成功（证明句柄没被 shutdown 顺手回收）。
    manager
        .kill(&aside_session, &aside_handle)
        .unwrap_or_else(|error| panic!("{B206_TICKET}: 旁观 session 的 kill 必须仍成功：{error}"));
    b206_wait_group_empty(
        aside_markers.leader,
        B206_REAP_TIMEOUT,
        "旁观 session 的 kill 返回后进程组未清空",
    );

    drop(group_guards);
    drop(aside_guard);
}

// ── 用例 8（静态机器影子）：C1-U「唯一终止路径 / 唯一派发点」护栏 ──────────────────────────────

/// 范围终结面的两个源文件（相对 crate 根）。
const B206_PROCESS_RS: &str = "/src/agent/process.rs";
const B206_PROCESS_RANGE_RS: &str = "/src/agent/process_range.rs";

/// 只读实现源文件（静态机器影子用；不启动任何进程）。
fn b206_read_source(relative: &str) -> String {
    let path = format!("{}{relative}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{B206_TICKET}: 必须能只读实现源文件 {path}：{error}"))
}

/// 是否为注释/属性行（`//`、`///`、`//!`、`#[...]`、`#![...]`）——静态影子只看**代码行**。
fn b206_static_is_comment(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("//") || trimmed.starts_with('#')
}

/// 生产代码区上界（排除文件末尾的 `#[cfg(test)] mod tests`；没有该区则返回全文长度）。
fn b206_static_production_end(lines: &[&str]) -> usize {
    lines
        .iter()
        .position(|line| line.trim_end() == "mod tests {")
        .unwrap_or(lines.len())
}

/// 某一行所属的函数名（向上找最近的 `fn <name>(` 定义行）。
fn b206_static_enclosing_fn(lines: &[&str], line_no: usize) -> String {
    for candidate in lines[..line_no].iter().rev() {
        let trimmed = candidate.trim_start();
        if let Some(rest) = trimmed.strip_prefix("fn ") {
            if let Some(name) = rest.split(['(', '<']).next() {
                return name.trim().to_string();
            }
        }
    }
    panic!("{B206_TICKET}: 第 {line_no} 行找不到所属函数（静态影子需要它来判定派发面）");
}

/// C1-U 的机械护栏（**结构无关**）：唯一信号派发原语 + 唯一终止路径 + 白名单调用者。
///
/// 不启动任何进程；判据全部来自只读源文件的**代码行**：
/// 1. `libc::killpg` 在范围终结面（`process.rs` + `process_range.rs`）合计**恰好 1 处**；
/// 2. 该处所属函数必须是 `signal_range`，且 `fn signal_range` 定义**恰好 1 处**；
/// 3. 生产代码里调用该原语的函数只允许是 `terminate_range_path`（唯一终止路径）、
///    `start_inner`（spawn 失败兜底，收的是**尚未发布**的子进程）、`signal`（`PidRangeOwner`
///    适配器，只做口径映射）；任何**其它**函数调用即「第二条派发路径」⇒ 红；
/// 4. `fn terminate_range_path` 定义恰好 1 处，且其函数体内至少投递 2 枪（`SIGTERM` + `SIGKILL`）；
/// 5. `pub fn shutdown_checked` 恰好 1 处，`self.terminate(` 入口 ≥ 2（kill 侧与 shutdown 侧）；
/// 6. `[B206][shutdown-not-quiescent]` 在代码行恰好 1 处；旧前缀
///    `[process] shutdown termination error` 零命中。
#[test]
fn b206_single_termination_path_static() {
    let process_src = b206_read_source(B206_PROCESS_RS);
    let range_src = b206_read_source(B206_PROCESS_RANGE_RS);
    let process_lines: Vec<&str> = process_src.lines().collect();
    let range_lines: Vec<&str> = range_src.lines().collect();
    let surfaces: [(&str, &Vec<&str>); 2] = [
        (B206_PROCESS_RS, &process_lines),
        (B206_PROCESS_RANGE_RS, &range_lines),
    ];

    // 1. 唯一派发点（范围终结面内 `libc::killpg` 合计恰好 1 处）。
    let mut killpg_sites: Vec<(&str, usize)> = Vec::new();
    for (relative, lines) in surfaces.iter() {
        for (index, line) in lines.iter().enumerate() {
            if b206_static_is_comment(line) {
                continue;
            }
            if line.contains("libc::killpg") {
                killpg_sites.push((relative, index + 1));
            }
        }
    }
    assert_eq!(
        killpg_sites.len(),
        1,
        "{B206_TICKET}/C1-U: 范围终结面内 `libc::killpg` 必须恰好 1 处（唯一派发点），实际 {} 处：{:?}",
        killpg_sites.len(),
        killpg_sites
    );
    let (killpg_file, killpg_line) = killpg_sites[0];
    let killpg_lines = if killpg_file == B206_PROCESS_RS {
        &process_lines
    } else {
        &range_lines
    };

    // 2. 派发原语唯一：`libc::killpg` 所在的函数名必须是 `signal_range`，且该函数定义唯一。
    let dispatch_fn = b206_static_enclosing_fn(killpg_lines, killpg_line);
    assert_eq!(
        dispatch_fn, "signal_range",
        "{B206_TICKET}/C1-U: `libc::killpg` 必须收在共享派发原语 `signal_range` 内，实际所属函数 `{dispatch_fn}`（{killpg_file}:{killpg_line}）"
    );
    let mut dispatch_definitions: Vec<(&str, usize)> = Vec::new();
    for (relative, lines) in surfaces.iter() {
        for (index, line) in lines.iter().enumerate() {
            if b206_static_is_comment(line) {
                continue;
            }
            if line.trim_start().starts_with("fn signal_range(") {
                dispatch_definitions.push((relative, index + 1));
            }
        }
    }
    assert_eq!(
        dispatch_definitions.len(),
        1,
        "{B206_TICKET}/C1-U: `fn signal_range` 必须恰好 1 处定义，实际 {dispatch_definitions:?}"
    );

    // 3. 调用者白名单：生产代码里只有这三类函数允许投递信号。
    let allowed_callers = ["terminate_range_path", "start_inner", "signal"];
    let mut call_sites: Vec<(&str, usize, String)> = Vec::new();
    for (relative, lines) in surfaces.iter() {
        let production_end = b206_static_production_end(lines);
        for (index, line) in lines.iter().enumerate().take(production_end) {
            if b206_static_is_comment(line) {
                continue;
            }
            if line.trim_start().starts_with("fn signal_range(") {
                continue; // 定义行本身
            }
            if !line.contains("signal_range(") {
                continue;
            }
            let line_no = index + 1;
            let caller = b206_static_enclosing_fn(lines, line_no);
            assert!(
                allowed_callers.contains(&caller.as_str()),
                "{B206_TICKET}/C1-U: {relative}:{line_no} 在 `{caller}` 里直接调用信号派发原语 \
                 `signal_range` —— 生产代码只允许 `terminate_range_path`（唯一终止路径）/\
                 `start_inner`（spawn 失败兜底）/`signal`（PidRangeOwner 口径适配）调用它；\
                 此处等于新增了第二条信号派发路径"
            );
            call_sites.push((relative, line_no, caller));
        }
    }
    assert!(
        call_sites.len() >= 2,
        "{B206_TICKET}/C1-U: 派发原语至少要有 2 处生产调用点（PidRangeOwner 适配 + spawn 失败兜底），\
         实际 {call_sites:?}"
    );

    // 4. 唯一终止路径：定义唯一，且函数体内至少投递 2 枪（阶梯 SIGTERM + SIGKILL）。
    let terminate_defs: Vec<usize> = process_lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !b206_static_is_comment(line))
        .filter(|(_, line)| line.contains("fn terminate_range_path("))
        .map(|(index, _)| index + 1)
        .collect();
    assert_eq!(
        terminate_defs.len(),
        1,
        "{B206_TICKET}/C1-U: `fn terminate_range_path` 必须恰好 1 处定义，实际 {terminate_defs:?}"
    );
    let terminate_def_line = terminate_defs[0];
    // 顶层函数体结束 = 定义之后第一条**顶格** `}`（rustfmt 风格；函数内部的 `}` 都有缩进）。
    let terminate_body_end = process_lines
        .iter()
        .enumerate()
        .skip(terminate_def_line)
        .find(|(_, line)| line.trim_end() == "}")
        .map(|(index, _)| index + 1)
        .unwrap_or_else(|| {
            panic!("{B206_TICKET}/C1-U: 找不到 `terminate_range_path` 的函数体结束行")
        });
    let ladder_shots = process_lines[(terminate_def_line)..terminate_body_end]
        .iter()
        .filter(|line| !b206_static_is_comment(line))
        .filter(|line| line.contains("owner.signal("))
        .count();
    assert!(
        ladder_shots >= 2,
        "{B206_TICKET}/C1-U: 唯一终止路径内必须至少投递 2 枪（SIGTERM 一枪、必要时 SIGKILL 一枪），\
         实际 `owner.signal(` 命中 {ladder_shots} 处（函数体 {terminate_def_line}..{terminate_body_end}）"
    );

    // 5. 加法式 API 唯一 + 多入口汇入同一终止路径。
    let checked_defs = process_lines
        .iter()
        .filter(|line| !b206_static_is_comment(line))
        .filter(|line| line.contains("pub fn shutdown_checked("))
        .count();
    assert_eq!(
        checked_defs, 1,
        "{B206_TICKET}/S-3: `pub fn shutdown_checked` 必须恰好 1 处，实际 {checked_defs}"
    );
    let terminate_call_sites = process_lines
        .iter()
        .filter(|line| !b206_static_is_comment(line))
        .filter(|line| line.contains("self.terminate("))
        .count();
    assert!(
        terminate_call_sites >= 2,
        "{B206_TICKET}/C1-U: `self.terminate(` 入口必须 ≥ 2（kill 侧与 shutdown 侧各一），实际 {terminate_call_sites}"
    );

    // 6. fail-loud 令牌唯一 + 旧「发了信号就当成功」前缀消失。
    let not_quiescent_code_lines: Vec<usize> = process_lines
        .iter()
        .enumerate()
        .filter(|(_, line)| !b206_static_is_comment(line))
        .filter(|(_, line)| line.contains("[B206][shutdown-not-quiescent]"))
        .map(|(index, _)| index + 1)
        .collect();
    assert_eq!(
        not_quiescent_code_lines.len(),
        1,
        "{B206_TICKET}: `[B206][shutdown-not-quiescent]` 在代码行必须恰好 1 处，实际 {} 处，行号={not_quiescent_code_lines:?}",
        not_quiescent_code_lines.len()
    );
    assert!(
        !process_src.contains("[process] shutdown termination error"),
        "{B206_TICKET}: 旧日志前缀 `[process] shutdown termination error` 必须零命中（已改为 fail-loud）"
    );
}
